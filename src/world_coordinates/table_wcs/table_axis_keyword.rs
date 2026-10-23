//! The per-axis `CTYPE`-family keywords, in their image and Table-22 spellings.

use crate::world_coordinates::table_wcs::keyword_spelling::KeywordSpelling;

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

    /// The Table-22 root for this family in `spelling`, or `None` where it has none (`CROTA`
    /// has no short form).
    pub(super) const fn table_root(self, spelling: KeywordSpelling) -> Option<&'static str> {
        match (self, spelling) {
            (Self::Type, KeywordSpelling::Full) => Some("CTYP"),
            (Self::Type, KeywordSpelling::Short) => Some("CTY"),
            (Self::Unit, KeywordSpelling::Full) => Some("CUNI"),
            (Self::Unit, KeywordSpelling::Short) => Some("CUN"),
            (Self::ReferenceValue, KeywordSpelling::Full) => Some("CRVL"),
            (Self::ReferenceValue, KeywordSpelling::Short) => Some("CRV"),
            (Self::Increment, KeywordSpelling::Full) => Some("CDLT"),
            (Self::Increment, KeywordSpelling::Short) => Some("CDE"),
            (Self::ReferencePoint, KeywordSpelling::Full) => Some("CRPX"),
            (Self::ReferencePoint, KeywordSpelling::Short) => Some("CRP"),
            (Self::Rotation, KeywordSpelling::Full) => Some("CROT"),
            (Self::Rotation, KeywordSpelling::Short) => None,
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
