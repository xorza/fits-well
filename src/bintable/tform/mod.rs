//! A parsed `TFORMn` column format.

use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::element_kind::ElementKind;
use crate::error::FitsError;
use crate::error::Result;
use std::str::FromStr;

/// A parsed `TFORMn` value (§7.3.1): a repeat count, the type of the elements, and
/// where they are stored, in the row or in the heap. The trailing characters of the
/// `rTa` form, a `P`/`Q` column's `(emax)` among them, are not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tform {
    repeat: usize,
    element: ElementKind,
    storage: Storage,
}

/// Where a column's elements are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// `repeat` elements in each row's cell.
    Fixed,
    /// A heap array for each row, which the row's `P` or `Q` descriptor addresses
    /// (§7.3.5); `repeat` is 0 or 1.
    Array(DescriptorWidth),
}

impl Tform {
    /// The repeat count: the elements of a fixed-width cell, or for an array column
    /// its descriptors, 0 or 1.
    pub const fn repeat(self) -> usize {
        self.repeat
    }

    pub const fn element(self) -> ElementKind {
        self.element
    }

    pub const fn storage(self) -> Storage {
        self.storage
    }

    /// The letter of the stored cell: the element's for a fixed-width column, `P` or
    /// `Q` for an array column.
    pub const fn code(self) -> char {
        match self.storage {
            Storage::Fixed => self.element.code(),
            Storage::Array(width) => width.letter(),
        }
    }

    /// The descriptor width of an array column; `None` for a fixed-width one.
    pub const fn descriptor_width(self) -> Option<DescriptorWidth> {
        match self.storage {
            Storage::Fixed => None,
            Storage::Array(width) => Some(width),
        }
    }

    /// The number of bytes this column occupies in every row.
    pub const fn byte_width(self) -> usize {
        match self.storage {
            // Saturating: an absurd `repeat` from a hostile `TFORMn` saturates to
            // `usize::MAX` rather than wrapping to a small width that could slip
            // past the row-width check in `Schema::parse`.
            Storage::Fixed => match self.element.payload_len(self.repeat) {
                Some(len) => len,
                None => usize::MAX,
            },
            Storage::Array(width) => self.repeat * width.bytes(),
        }
    }
}

/// Parse a `TFORMn` value such as `"8A"`, `"3D"`, `"1J"`, `"E"`, or `"1PE(5)"`.
impl FromStr for Tform {
    type Err = FitsError;

    fn from_str(value: &str) -> Result<Self> {
        let s = value.trim();
        let invalid = || FitsError::InvalidTform {
            tform: value.to_string(),
        };
        let pos = s
            .bytes()
            .position(|b| b.is_ascii_alphabetic())
            .ok_or_else(invalid)?;
        let repeat = if pos == 0 {
            1
        } else {
            s[..pos].parse().map_err(|_| invalid())?
        };
        let bytes = s.as_bytes();
        // A P/Q descriptor is followed by its heap element-type letter (`rPt`). The
        // characters after a code, or after `t`, such as `(emax)`, are undefined
        // (§7.3.1, §7.3.5) and ignored, as cfitsio and astropy ignore them.
        let width = match bytes[pos] {
            b'P' => Some(DescriptorWidth::P),
            b'Q' => Some(DescriptorWidth::Q),
            _ => None,
        };
        let (element, storage) = match width {
            Some(width) => {
                // §6.3: a `P`/`Q` descriptor's repeat count is restricted to 0 or 1.
                if repeat > 1 {
                    return Err(invalid());
                }
                let code = bytes.get(pos + 1).copied().ok_or_else(invalid)?;
                (
                    ElementKind::from_code(code).ok_or_else(invalid)?,
                    Storage::Array(width),
                )
            }
            None => (
                ElementKind::from_code(bytes[pos]).ok_or_else(invalid)?,
                Storage::Fixed,
            ),
        };
        Ok(Self {
            repeat,
            element,
            storage,
        })
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::element_kind::ElementKind;
    use crate::bintable::tform::{Storage, Tform};

    /// A `TFORMn` value spelled out field by field, for tests asserting what a
    /// parsed or schema-derived format resolved to.
    pub(crate) const fn tform(repeat: usize, element: ElementKind, storage: Storage) -> Tform {
        Tform {
            repeat,
            element,
            storage,
        }
    }
}

#[cfg(test)]
mod tests;
