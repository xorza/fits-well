//! [`PhaseAxis`]: the §9.6 `'PHASE'` axis folding parameters.

/// The §9.6 `'PHASE'` axis folding parameters: the zero-phase reference time
/// `CZPHSia` and the period `CPERIia`, in `TIMEUNIT` relative to the time
/// reference (the `TSTART` convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseAxis {
    pub zero_phase: f64,
    /// A constant non-zero `CPERI` value; `None` means the period is undefined or
    /// varies with time.
    pub period: Option<f64>,
}
