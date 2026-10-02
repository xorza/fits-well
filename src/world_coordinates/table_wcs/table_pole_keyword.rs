//! The two celestial-pole keywords, in their image and Table-22 spellings.

/// One of the two celestial-pole keywords, `LONPOLEa` or `LATPOLEa`.
#[derive(Debug, Clone, Copy)]
pub(super) enum TablePoleKeyword {
    Longitude,
    Latitude,
}

impl TablePoleKeyword {
    pub(super) const BOTH: [TablePoleKeyword; 2] =
        [TablePoleKeyword::Longitude, TablePoleKeyword::Latitude];

    /// The Table-22 column-indexed root.
    pub(super) fn table_root(self) -> &'static str {
        match self {
            TablePoleKeyword::Longitude => "LONP",
            TablePoleKeyword::Latitude => "LATP",
        }
    }

    /// The image-header root this translates to.
    pub(super) fn image_root(self) -> &'static str {
        match self {
            TablePoleKeyword::Longitude => "LONPOLE",
            TablePoleKeyword::Latitude => "LATPOLE",
        }
    }
}
