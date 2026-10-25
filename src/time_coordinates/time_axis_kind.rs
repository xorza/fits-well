//! [`TimeAxisKind`]: the time-related WCS axis types.

use crate::time_coordinates::scale::Scale;
use crate::world_coordinates::ctype::Ctype;

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
        Self::from_head(Ctype::parse(ctype).head)
    }

    /// [`TimeAxisKind::from_ctype`] of a `CTYPE` whose head is `head`.
    pub(crate) fn from_head(head: &str) -> Option<Self> {
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
}

#[cfg(test)]
mod tests {

    use crate::time_coordinates::time_axis_kind::TimeAxisKind;

    /// Each head names its kind with no regard to case, and a scale name is a time axis.
    #[test]
    fn classifies_time_related_axes() {
        use TimeAxisKind::{Frequency, Phase, Time, Timelag};
        for (ctype, kind) in [
            ("TIME", Some(Time)),
            ("time", Some(Time)),
            ("UTC", Some(Time)),
            ("tai-LOG", Some(Time)),
            ("PHASE", Some(Phase)),
            ("phase", Some(Phase)),
            ("TIMELAG", Some(Timelag)),
            ("TimeLag", Some(Timelag)),
            ("FREQUENCY", Some(Frequency)),
            ("frequency", Some(Frequency)),
            ("RA---TAN", None),
        ] {
            assert_eq!(TimeAxisKind::from_ctype(ctype), kind, "{ctype}");
        }
    }
}
