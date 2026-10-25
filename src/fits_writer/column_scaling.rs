//! [`ColumnScaling`]: the `TSCALn` and `TZEROn` a written table column declares.

use crate::fits_error::KeywordDefect;
use crate::fits_error::{FitsError, Result};
use crate::header_model::keywords;

/// The `TSCALn` and `TZEROn` a written column declares (§7.2.2, §7.3.2): its cells hold
/// the stored values, and a reader recovers `TZEROn + TSCALn × stored`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ColumnScaling {
    pub(super) tscale: f64,
    pub(super) tzero: f64,
}

/// Whether a column's values take `TSCALn` and `TZEROn` (§7.2.5, §7.3.2): numbers do,
/// and text, logical and bit values do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scalable {
    Numeric,
    Unscaled,
}

impl ColumnScaling {
    /// Errors when column `n`, whose values `scalable` describes, takes no scaling, or
    /// when a value is not finite.
    pub(super) fn validate(self, scalable: Scalable, n: usize) -> Result<()> {
        for (value, keyword) in [
            (self.tscale, keywords::TSCAL),
            (self.tzero, keywords::TZERO),
        ] {
            let defect = if scalable == Scalable::Unscaled {
                KeywordDefect::Structure
            } else if !value.is_finite() {
                KeywordDefect::Range
            } else {
                continue;
            };
            return Err(FitsError::InvalidKeywordValue {
                keyword: keyword.indexed(n)?,
                defect,
            });
        }
        Ok(())
    }
}
