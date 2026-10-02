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
    pub(super) const ALL: [TableAxisKeyword; 6] = [
        TableAxisKeyword::Type,
        TableAxisKeyword::Unit,
        TableAxisKeyword::ReferenceValue,
        TableAxisKeyword::Increment,
        TableAxisKeyword::ReferencePoint,
        TableAxisKeyword::Rotation,
    ];

    /// The Table-22 root for this family, or `None` where an alternate description
    /// has no spelling for it (`CROTA` has no shortened alternate form).
    pub(super) fn table_root(self, alternate: bool) -> Option<&'static str> {
        match (self, alternate) {
            (TableAxisKeyword::Type, false) => Some("CTYP"),
            (TableAxisKeyword::Type, true) => Some("CTY"),
            (TableAxisKeyword::Unit, false) => Some("CUNI"),
            (TableAxisKeyword::Unit, true) => Some("CUN"),
            (TableAxisKeyword::ReferenceValue, false) => Some("CRVL"),
            (TableAxisKeyword::ReferenceValue, true) => Some("CRV"),
            (TableAxisKeyword::Increment, false) => Some("CDLT"),
            (TableAxisKeyword::Increment, true) => Some("CDE"),
            (TableAxisKeyword::ReferencePoint, false) => Some("CRPX"),
            (TableAxisKeyword::ReferencePoint, true) => Some("CRP"),
            (TableAxisKeyword::Rotation, false) => Some("CROT"),
            (TableAxisKeyword::Rotation, true) => None,
        }
    }

    /// The image-header root this family translates to.
    pub(super) fn image_root(self) -> &'static str {
        match self {
            TableAxisKeyword::Type => "CTYPE",
            TableAxisKeyword::Unit => "CUNIT",
            TableAxisKeyword::ReferenceValue => "CRVAL",
            TableAxisKeyword::Increment => "CDELT",
            TableAxisKeyword::ReferencePoint => "CRPIX",
            TableAxisKeyword::Rotation => "CROTA",
        }
    }
}
