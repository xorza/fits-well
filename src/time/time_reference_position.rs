//! [`TimeReferencePosition`]: where a FITS time coordinate is measured.

/// The spatial location at which a FITS time coordinate is valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeReferencePosition {
    Topocenter,
    Geocenter,
    Barycenter,
    Relocatable,
    Custom,
    Heliocenter,
    GalacticCenter,
    EarthMoonBarycenter,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    /// A non-standard value retained exactly as declared.
    Other(String),
}

impl TimeReferencePosition {
    pub(super) fn parse(value: &str) -> TimeReferencePosition {
        match value {
            value if value.starts_with("TOP") => TimeReferencePosition::Topocenter,
            value if value.starts_with("GEO") => TimeReferencePosition::Geocenter,
            value if value.starts_with("BAR") => TimeReferencePosition::Barycenter,
            value if value.starts_with("REL") => TimeReferencePosition::Relocatable,
            value if value.starts_with("CUS") => TimeReferencePosition::Custom,
            value if value.starts_with("HEL") => TimeReferencePosition::Heliocenter,
            value if value.starts_with("GAL") => TimeReferencePosition::GalacticCenter,
            value if value.starts_with("EMB") => TimeReferencePosition::EarthMoonBarycenter,
            value if value.starts_with("MER") => TimeReferencePosition::Mercury,
            value if value.starts_with("VEN") => TimeReferencePosition::Venus,
            value if value.starts_with("MAR") => TimeReferencePosition::Mars,
            value if value.starts_with("JUP") => TimeReferencePosition::Jupiter,
            value if value.starts_with("SAT") => TimeReferencePosition::Saturn,
            value if value.starts_with("URA") => TimeReferencePosition::Uranus,
            value if value.starts_with("NEP") => TimeReferencePosition::Neptune,
            value => TimeReferencePosition::Other(value.to_string()),
        }
    }
}
