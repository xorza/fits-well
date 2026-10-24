use std::ops::Range;

use crate::bintable::element_kind::ElementKind;
use crate::error::FitsError;
use crate::error::Result;
use crate::error::SizeUnit;

/// A variable-length array descriptor's integers: `P`'s two 32-bit ones, or `Q`'s two 64-bit
/// ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorWidth {
    /// 32-bit integers, of a `P` column.
    P,
    /// 64-bit integers, of a `Q` column.
    Q,
}

impl DescriptorWidth {
    /// The narrower descriptor that addresses a heap of `heap_len` bytes and cells of up to
    /// `max_count` elements: `Q` once either is past the signed 32-bit range (§10.1.3).
    #[cfg(feature = "compression")]
    pub(crate) const fn addressing(heap_len: usize, max_count: usize) -> Self {
        if heap_len > i32::MAX as usize || max_count > i32::MAX as usize {
            Self::Q
        } else {
            Self::P
        }
    }

    /// The bytes a descriptor takes.
    pub(crate) const fn bytes(self) -> usize {
        match self {
            Self::P => 8,
            Self::Q => 16,
        }
    }

    /// The `TFORM` letter.
    pub(crate) const fn letter(self) -> char {
        match self {
            Self::P => 'P',
            Self::Q => 'Q',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PqDescriptor {
    pub(crate) count: usize,
    pub(crate) offset: usize,
}

impl PqDescriptor {
    pub(crate) const EMPTY: Self = Self {
        count: 0,
        offset: 0,
    };

    pub(crate) fn decode(bytes: &[u8], width: DescriptorWidth) -> Result<Self> {
        let expected = width.bytes();
        if bytes.len() != expected {
            return Err(SizeUnit::Bytes.mismatch(expected, bytes.len()));
        }
        let (count, offset) = match width {
            DescriptorWidth::Q => (
                i64::from_be_bytes(bytes[..8].try_into().unwrap()),
                i64::from_be_bytes(bytes[8..].try_into().unwrap()),
            ),
            DescriptorWidth::P => (
                i64::from(i32::from_be_bytes(bytes[..4].try_into().unwrap())),
                i64::from(i32::from_be_bytes(bytes[4..].try_into().unwrap())),
            ),
        };
        let count = nonnegative_field("count", count)?;
        // §7.3.5: the offset of an array of no elements is undefined.
        let offset = if count == 0 {
            0
        } else {
            nonnegative_field("offset", offset)?
        };
        Ok(Self { count, offset })
    }

    /// Errors with [`FitsError::DescriptorOutOfRange`] when the signed integers of
    /// `width` cannot hold the count or the offset.
    pub(crate) fn validate(self, width: DescriptorWidth) -> Result<()> {
        self.integers(width).map(|_| ())
    }

    /// Write this descriptor into `slot` as the big-endian count then offset of `width`
    /// (§7.3.5), after [`PqDescriptor::validate`].
    pub(crate) fn write(self, width: DescriptorWidth, slot: &mut [u8]) -> Result<()> {
        debug_assert_eq!(slot.len(), width.bytes(), "a descriptor's slot");
        match self.integers(width)? {
            DescriptorIntegers::P(count, offset) => {
                slot[..4].copy_from_slice(&count.to_be_bytes());
                slot[4..].copy_from_slice(&offset.to_be_bytes());
            }
            DescriptorIntegers::Q(count, offset) => {
                slot[..8].copy_from_slice(&count.to_be_bytes());
                slot[8..].copy_from_slice(&offset.to_be_bytes());
            }
        }
        Ok(())
    }

    fn integers(self, width: DescriptorWidth) -> Result<DescriptorIntegers> {
        let overflow = |_| FitsError::DescriptorOutOfRange {
            width,
            count: self.count,
            offset: self.offset,
        };
        Ok(match width {
            DescriptorWidth::P => DescriptorIntegers::P(
                i32::try_from(self.count).map_err(overflow)?,
                i32::try_from(self.offset).map_err(overflow)?,
            ),
            DescriptorWidth::Q => DescriptorIntegers::Q(
                i64::try_from(self.count).map_err(overflow)?,
                i64::try_from(self.offset).map_err(overflow)?,
            ),
        })
    }

    pub(crate) fn heap_range(
        self,
        element_kind: ElementKind,
        heap_start: usize,
        heap_end: usize,
    ) -> Result<Range<usize>> {
        let byte_len = element_kind
            .payload_len(self.count)
            .ok_or(FitsError::DataUnitOverflow)?;
        if byte_len == 0 {
            return Ok(heap_start..heap_start);
        }
        let start = heap_start
            .checked_add(self.offset)
            .ok_or(FitsError::DataUnitOverflow)?;
        let end = start
            .checked_add(byte_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        if end > heap_end {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(start..end)
    }
}

/// A descriptor's count then offset, as the signed integers of its width.
#[derive(Debug, Clone, Copy)]
enum DescriptorIntegers {
    P(i32, i32),
    Q(i64, i64),
}

fn nonnegative_field(field: &'static str, value: i64) -> Result<usize> {
    if value < 0 {
        return Err(FitsError::InvalidPqDescriptor { field, value });
    }
    usize::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::descriptor::DescriptorWidth;
    use crate::error::FitsError;

    #[derive(Debug)]
    pub(crate) struct MalformedDescriptorCase {
        name: &'static str,
        pub(crate) width: DescriptorWidth,
        pub(crate) bytes: Vec<u8>,
        field: &'static str,
        value: i64,
    }

    impl MalformedDescriptorCase {
        pub(crate) fn assert_error(&self, error: &FitsError) {
            assert!(
                matches!(
                    error,
                    FitsError::InvalidPqDescriptor { field, value }
                        if *field == self.field && *value == self.value
                ),
                "{}: {error:?}",
                self.name
            );
        }
    }

    pub(crate) fn malformed_descriptor_cases() -> Vec<MalformedDescriptorCase> {
        let mut cases = Vec::new();
        for (name, count, offset, field, value) in [
            ("P negative count", -1i32, 0i32, "count", -1i64),
            ("P negative offset", 1, -1, "offset", -1),
            (
                "P one past signed maximum",
                i32::MIN,
                0,
                "count",
                i64::from(i32::MIN),
            ),
        ] {
            let mut bytes = Vec::with_capacity(8);
            bytes.extend_from_slice(&count.to_be_bytes());
            bytes.extend_from_slice(&offset.to_be_bytes());
            cases.push(MalformedDescriptorCase {
                name,
                width: DescriptorWidth::P,
                bytes,
                field,
                value,
            });
        }
        for (name, count, offset, field, value) in [
            ("Q negative count", -1i64, 0i64, "count", -1i64),
            ("Q negative offset", 1, -1, "offset", -1),
            ("Q one past signed maximum", i64::MIN, 0, "count", i64::MIN),
        ] {
            let mut bytes = Vec::with_capacity(16);
            bytes.extend_from_slice(&count.to_be_bytes());
            bytes.extend_from_slice(&offset.to_be_bytes());
            cases.push(MalformedDescriptorCase {
                name,
                width: DescriptorWidth::Q,
                bytes,
                field,
                value,
            });
        }
        cases
    }
}

#[cfg(test)]
mod tests;
