//! [`TimeAxisKind`]: the time-related WCS axis types.

use crate::time_coordinates::scale::Scale;

/// A time-related WCS axis type (§9.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeAxisKind {
    /// `'TIME'` or a time-scale name — an absolute time axis (→ MJD).
    Time,
    /// `'PHASE'` — phase folded on a period (`CPERIia`, zero `CZPHSia`).
    Phase,
    /// `'TIMELAG'` — a correlation/cross-spectral time lag.
    Timelag,
    /// `'FREQUENCY'` — a frequency axis.
    Frequency,
}

impl TimeAxisKind {
    /// Classify a `CTYPE` as a time-related axis (§9.6), or `None` if it is not one. The
    /// head matches with no regard to case, as a time-scale name does.
    pub(crate) fn from_ctype(ctype: &str) -> Option<Self> {
        let head = Self::head(ctype);
        let named = |name: &str| head.eq_ignore_ascii_case(name);
        if named("TIME") {
            Some(Self::Time)
        } else if named("PHASE") {
            Some(Self::Phase)
        } else if named("TIMELAG") {
            Some(Self::Timelag)
        } else if named("FREQUENCY") {
            Some(Self::Frequency)
        } else {
            head.parse::<Scale>()
                .ok()
                .and_then(|scale| scale.kind())
                .map(|_| Self::Time)
        }
    }

    /// The head of `ctype`: the name before its first `-`, blanks trimmed.
    pub(crate) fn head(ctype: &str) -> &str {
        ctype.split('-').next().unwrap_or("").trim()
    }
}

#[cfg(test)]
mod tests {

    use crate::time_coordinates::time_axis_kind::TimeAxisKind;

    #[test]
    fn classifies_time_related_axes() {
        use TimeAxisKind::*;
        assert_eq!(TimeAxisKind::from_ctype("TIME"), Some(Time));
        assert_eq!(TimeAxisKind::from_ctype("time"), Some(Time));
        assert_eq!(TimeAxisKind::from_ctype("UTC"), Some(Time)); // a scale name is a time axis
        assert_eq!(TimeAxisKind::from_ctype("PHASE"), Some(Phase));
        assert_eq!(TimeAxisKind::from_ctype("TIMELAG"), Some(Timelag));
        assert_eq!(TimeAxisKind::from_ctype("FREQUENCY"), Some(Frequency));
        // Every head matches with no regard to case, as `TIME` and the scale names do.
        assert_eq!(TimeAxisKind::from_ctype("phase"), Some(Phase));
        assert_eq!(TimeAxisKind::from_ctype("TimeLag"), Some(Timelag));
        assert_eq!(TimeAxisKind::from_ctype("frequency"), Some(Frequency));
        assert_eq!(TimeAxisKind::from_ctype("tai-LOG"), Some(Time));
        assert_eq!(TimeAxisKind::from_ctype("RA---TAN"), None);
    }
}
