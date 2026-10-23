//! The two linear-transform keyword families, in their image and Table-22 spellings.

use crate::world_coordinates::table_wcs::keyword_spelling::KeywordSpelling;

/// The linear-transform convention a keyword belongs to: `PCi_ja` or `CDi_ja`.
#[derive(Debug, Clone, Copy)]
pub(super) enum TableMatrixKeyword {
    Pc,
    Cd,
}

impl TableMatrixKeyword {
    /// The image-header root, also used verbatim by the vector-cell form (`ijPCna`).
    pub(super) const fn root(self) -> &'static str {
        match self {
            Self::Pc => "PC",
            Self::Cd => "CD",
        }
    }

    /// The pixel-list root, in `spelling`.
    pub(super) const fn pixel_root(self, spelling: KeywordSpelling) -> &'static str {
        match (self, spelling) {
            (Self::Pc, KeywordSpelling::Full) => "TPC",
            (Self::Pc, KeywordSpelling::Short) => "TP",
            (Self::Cd, KeywordSpelling::Full) => "TCD",
            (Self::Cd, KeywordSpelling::Short) => "TC",
        }
    }
}
