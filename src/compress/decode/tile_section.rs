//! The part of a tiled image a section decode covers.

use std::ops::Range;

/// The part of a tiled image a section decode covers: the requested `ranges`, the
/// `shape` they select, and the table rows of the tiles they intersect, in the order
/// the section's table holds them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TileSection<'s> {
    pub(crate) ranges: &'s [Range<usize>],
    pub(crate) shape: &'s [usize],
    pub(crate) rows: &'s [usize],
}
