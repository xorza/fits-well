//! [`ReferencePosition`]: where a FITS time coordinate is measured.

/// The spatial location at which a FITS time coordinate is valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferencePosition {
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

impl ReferencePosition {
    pub(super) fn parse(value: &str) -> Self {
        match value {
            value if value.starts_with("TOP") => Self::Topocenter,
            value if value.starts_with("GEO") => Self::Geocenter,
            value if value.starts_with("BAR") => Self::Barycenter,
            value if value.starts_with("REL") => Self::Relocatable,
            value if value.starts_with("CUS") => Self::Custom,
            value if value.starts_with("HEL") => Self::Heliocenter,
            value if value.starts_with("GAL") => Self::GalacticCenter,
            value if value.starts_with("EMB") => Self::EarthMoonBarycenter,
            value if value.starts_with("MER") => Self::Mercury,
            value if value.starts_with("VEN") => Self::Venus,
            value if value.starts_with("MAR") => Self::Mars,
            value if value.starts_with("JUP") => Self::Jupiter,
            value if value.starts_with("SAT") => Self::Saturn,
            value if value.starts_with("URA") => Self::Uranus,
            value if value.starts_with("NEP") => Self::Neptune,
            value => Self::Other(value.to_string()),
        }
    }
}
