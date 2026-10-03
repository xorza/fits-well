//! [`TimeAxisKind`]: the time-related WCS axis types.

use crate::time_coordinates::time_scale::TimeScale;
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
    /// Classify a `CTYPE` as a time-related axis (§9.6), or `None` if it is not one.
    pub(crate) fn from_ctype(ctype: &str) -> Option<TimeAxisKind> {
        let head = Ctype::parse(ctype).head;
        if head.eq_ignore_ascii_case("TIME") {
            return Some(TimeAxisKind::Time);
        }
        match head {
            "PHASE" => Some(TimeAxisKind::Phase),
            "TIMELAG" => Some(TimeAxisKind::Timelag),
            "FREQUENCY" => Some(TimeAxisKind::Frequency),
            _ => head
                .parse::<TimeScale>()
                .ok()
                .and_then(|scale| scale.kind())
                .map(|_| TimeAxisKind::Time),
        }
    }
}
