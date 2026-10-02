//! The two linear-transform keyword families, in their image and Table-22 spellings.

/// The linear-transform convention a keyword belongs to: `PCi_ja` or `CDi_ja`.
#[derive(Debug, Clone, Copy)]
pub(super) enum TableMatrixKeyword {
    Pc,
    Cd,
}

impl TableMatrixKeyword {
    /// The image-header root, also used verbatim by the vector-cell form (`ijPCna`).
    pub(super) fn root(self) -> &'static str {
        match self {
            TableMatrixKeyword::Pc => "PC",
            TableMatrixKeyword::Cd => "CD",
        }
    }

    /// The pixel-list root, in its full or (for alternate descriptions) abbreviated
    /// spelling.
    pub(super) fn pixel_root(self, abbreviated: bool) -> &'static str {
        match (self, abbreviated) {
            (TableMatrixKeyword::Pc, false) => "TPC",
            (TableMatrixKeyword::Pc, true) => "TP",
            (TableMatrixKeyword::Cd, false) => "TCD",
            (TableMatrixKeyword::Cd, true) => "TC",
        }
    }
}
