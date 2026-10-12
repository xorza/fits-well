//! The two linear-transform keyword families, in their image and Table-22 spellings.

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

    /// The pixel-list root, in its full or (for alternate descriptions) abbreviated
    /// spelling.
    pub(super) const fn pixel_root(self, abbreviated: bool) -> &'static str {
        match (self, abbreviated) {
            (Self::Pc, false) => "TPC",
            (Self::Pc, true) => "TP",
            (Self::Cd, false) => "TCD",
            (Self::Cd, true) => "TC",
        }
    }
}
