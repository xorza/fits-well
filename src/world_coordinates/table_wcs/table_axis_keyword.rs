//! The per-axis `CTYPE`-family keywords, in their image and Table-22 spellings.

/// One of the six per-axis keyword families a WCS axis is described by. Table 22
/// gives each a shortened column-indexed spelling — and a second, further shortened
/// one for alternate descriptions, where the suffix letter costs a character.
#[derive(Debug, Clone, Copy)]
pub(super) enum TableAxisKeyword {
    Type,
    Unit,
    ReferenceValue,
    Increment,
    ReferencePoint,
    Rotation,
}

impl TableAxisKeyword {
    pub(super) const ALL: [Self; 6] = [
        Self::Type,
        Self::Unit,
        Self::ReferenceValue,
        Self::Increment,
        Self::ReferencePoint,
        Self::Rotation,
    ];

    /// The Table-22 root for this family, or `None` where an alternate description
    /// has no spelling for it (`CROTA` has no shortened alternate form).
    pub(super) const fn table_root(self, alternate: bool) -> Option<&'static str> {
        match (self, alternate) {
            (Self::Type, false) => Some("CTYP"),
            (Self::Type, true) => Some("CTY"),
            (Self::Unit, false) => Some("CUNI"),
            (Self::Unit, true) => Some("CUN"),
            (Self::ReferenceValue, false) => Some("CRVL"),
            (Self::ReferenceValue, true) => Some("CRV"),
            (Self::Increment, false) => Some("CDLT"),
            (Self::Increment, true) => Some("CDE"),
            (Self::ReferencePoint, false) => Some("CRPX"),
            (Self::ReferencePoint, true) => Some("CRP"),
            (Self::Rotation, false) => Some("CROT"),
            (Self::Rotation, true) => None,
        }
    }

    /// The image-header root this family translates to.
    pub(super) const fn image_root(self) -> &'static str {
        match self {
            Self::Type => "CTYPE",
            Self::Unit => "CUNIT",
            Self::ReferenceValue => "CRVAL",
            Self::Increment => "CDELT",
            Self::ReferencePoint => "CRPIX",
            Self::Rotation => "CROTA",
        }
    }
}
