//! [`ColumnScaling`]: the `TSCALn` and `TZEROn` a written table column declares.

use crate::error::{FitsError, Result};
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
        let refused = |value: f64| scalable == Scalable::Unscaled || !value.is_finite();
        if refused(self.tscale) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::TSCAL.indexed(n)?,
            });
        }
        if refused(self.tzero) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::TZERO.indexed(n)?,
            });
        }
        Ok(())
    }
}
