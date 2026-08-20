use std::ops::Range;

use crate::error::FitsError;
use crate::error::Result;
use crate::table_impl::tform_kind::TformKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PqDescriptor {
    pub(crate) count: usize,
    pub(crate) offset: usize,
}

impl PqDescriptor {
    pub(super) const EMPTY: PqDescriptor = PqDescriptor {
        count: 0,
        offset: 0,
    };

    pub(crate) fn decode(bytes: &[u8], wide: bool) -> Result<PqDescriptor> {
        let expected = if wide { 16 } else { 8 };
        if bytes.len() != expected {
            return Err(FitsError::DataSizeMismatch {
                expected,
                got: bytes.len(),
            });
        }
        let (count, offset) = if wide {
            (
                i64::from_be_bytes(bytes[..8].try_into().unwrap()),
                i64::from_be_bytes(bytes[8..].try_into().unwrap()),
            )
        } else {
            (
                i32::from_be_bytes(bytes[..4].try_into().unwrap()) as i64,
                i32::from_be_bytes(bytes[4..].try_into().unwrap()) as i64,
            )
        };
        Ok(PqDescriptor {
            count: nonnegative_field("count", count)?,
            offset: nonnegative_field("offset", offset)?,
        })
    }

    pub(crate) fn heap_range(
        self,
        element_kind: TformKind,
        heap_start: usize,
        heap_end: usize,
    ) -> Result<Range<usize>> {
        let byte_len = payload_len(element_kind, self.count)?;
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

pub(super) fn payload_len(element_kind: TformKind, count: usize) -> Result<usize> {
    if element_kind == TformKind::Bit {
        Ok(count.div_ceil(8))
    } else {
        count
            .checked_mul(element_kind.elem_size())
            .ok_or(FitsError::DataUnitOverflow)
    }
}

fn nonnegative_field(field: &'static str, value: i64) -> Result<usize> {
    if value < 0 {
        return Err(FitsError::InvalidPqDescriptor { field, value });
    }
    usize::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::error::FitsError;

    #[derive(Debug)]
    pub(crate) struct MalformedDescriptorCase {
        name: &'static str,
        pub(crate) wide: bool,
        pub(crate) bytes: Vec<u8>,
        field: &'static str,
        value: i64,
    }

    impl MalformedDescriptorCase {
        pub(crate) fn assert_error(&self, error: FitsError) {
            assert!(
                matches!(
                    error,
                    FitsError::InvalidPqDescriptor { field, value }
                        if field == self.field && value == self.value
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
                i32::MIN as i64,
            ),
        ] {
            let mut bytes = Vec::with_capacity(8);
            bytes.extend_from_slice(&count.to_be_bytes());
            bytes.extend_from_slice(&offset.to_be_bytes());
            cases.push(MalformedDescriptorCase {
                name,
                wide: false,
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
                wide: true,
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
