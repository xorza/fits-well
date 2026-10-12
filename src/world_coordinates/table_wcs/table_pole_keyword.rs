//! The two celestial-pole keywords, in their image and Table-22 spellings.

/// One of the two celestial-pole keywords, `LONPOLEa` or `LATPOLEa`.
#[derive(Debug, Clone, Copy)]
pub(super) enum TablePoleKeyword {
    Longitude,
    Latitude,
}

impl TablePoleKeyword {
    pub(super) const BOTH: [Self; 2] = [Self::Longitude, Self::Latitude];

    /// The Table-22 column-indexed root.
    pub(super) const fn table_root(self) -> &'static str {
        match self {
            Self::Longitude => "LONP",
            Self::Latitude => "LATP",
        }
    }

    /// The image-header root this translates to.
    pub(super) const fn image_root(self) -> &'static str {
        match self {
            Self::Longitude => "LONPOLE",
            Self::Latitude => "LATPOLE",
        }
    }
}
