//! A parsed `TFORMn` column format.

use crate::error::FitsError;
use crate::error::Result;
use crate::table_impl::tform_kind::TformKind;

/// A parsed `TFORMn` value: a repeat count, an element kind, and (for the `P`/`Q`
/// variable-length-array descriptors) the kind of the array elements in the heap.
/// The `rTa` form's trailing `(emax)` size hint is not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tform {
    pub repeat: usize,
    pub kind: TformKind,
    /// For `P`/`Q` columns, the element kind of the heap array (the `t` in
    /// `rPt(emax)`); `None` for fixed-width columns.
    pub vla_elem: Option<TformKind>,
}

impl Tform {
    /// Parse a `TFORMn` value such as `"8A"`, `"3D"`, `"1J"`, `"E"`, or `"1PE(5)"`.
    pub fn parse(value: &str) -> Result<Tform> {
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
        let kind = TformKind::from_code(bytes[pos]).ok_or_else(invalid)?;
        // A P/Q descriptor is followed by its heap element-type letter (`rPt`) and
        // may carry one complete `(emax)` hint. Fixed formats end after their code.
        let vla_elem = if kind.is_descriptor() {
            let elem = bytes.get(pos + 1).copied().ok_or_else(invalid)?;
            // §6.3: a `P`/`Q` descriptor's repeat count is restricted to 0 or 1.
            if repeat > 1 {
                return Err(invalid());
            }
            let elem = TformKind::from_code(elem).ok_or_else(invalid)?;
            if elem.is_descriptor() {
                return Err(invalid());
            }
            let suffix = &s[pos + 2..];
            if !suffix.is_empty() {
                let hint = suffix
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .filter(|value| {
                        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .ok_or_else(invalid)?;
                hint.parse::<usize>().map_err(|_| invalid())?;
            }
            Some(elem)
        } else {
            if pos + 1 != s.len() {
                return Err(invalid());
            }
            None
        };
        Ok(Tform {
            repeat,
            kind,
            vla_elem,
        })
    }

    /// The number of bytes this column occupies in every row.
    pub fn byte_width(self) -> usize {
        match self.kind {
            TformKind::Bit => self.repeat.div_ceil(8),
            // Saturating: an absurd `repeat` from a hostile `TFORMn` saturates to
            // `usize::MAX` rather than wrapping to a small width that could slip
            // past the row-width check in `BinTable::from_data`.
            _ => self.repeat.saturating_mul(self.kind.elem_size()),
        }
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::table_impl::tform::Tform;
    use crate::table_impl::tform_kind::TformKind;

    /// A `TFORMn` value spelled out field by field, for tests asserting what a
    /// parsed or schema-derived format resolved to.
    pub(crate) fn tform(repeat: usize, kind: TformKind, vla_elem: Option<TformKind>) -> Tform {
        Tform {
            repeat,
            kind,
            vla_elem,
        }
    }
}

#[cfg(test)]
mod tests;
