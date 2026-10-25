//! [`TileIndexes`]: the tile index of each compressed image a reader opened.

use crate::allocation;
use crate::bintable::schema::Schema;
use crate::compression::decode::tile_index::TileIndex;
use crate::fits_error::Result;

/// The tile indexes of the compressed images a reader opened, a slot for each HDU up to the
/// last one opened: each read when its image first opens and kept for every later handle on
/// it, so a handle on one image never makes another read its index again.
#[derive(Debug, Default)]
pub(crate) struct TileIndexes {
    slots: Vec<Option<TileIndex>>,
}

/// The index of the image a handle opened, and the heap of every index the reader holds,
/// which stays as it is while the handle borrows them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoadedIndex<'a> {
    pub(crate) index: &'a TileIndex,
    pub(crate) indexes_bytes: usize,
}

impl TileIndexes {
    /// The index of the container of HDU `hdu`, which `schema` describes and `read_rows`
    /// appends to the buffer it is given: read now unless it is held already.
    pub(crate) fn load(
        &mut self,
        hdu: usize,
        schema: &Schema,
        read_rows: impl FnOnce(usize, &mut Vec<u8>) -> Result<()>,
    ) -> Result<LoadedIndex<'_>> {
        if self.slots.len() <= hdu {
            let additional = hdu + 1 - self.slots.len();
            allocation::try_reserve(&mut self.slots, additional)?;
            self.slots.resize_with(hdu + 1, || None);
        }
        let index = match self.slots[hdu].take() {
            Some(index) => index,
            None => TileIndex::read(schema, read_rows)?,
        };
        let indexes_bytes = self.heap_bytes() + index.heap_bytes();
        Ok(LoadedIndex {
            index: self.slots[hdu].insert(index),
            indexes_bytes,
        })
    }

    /// The heap the indexes hold, with their own slots.
    fn heap_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Option<TileIndex>>()
            + self
                .slots
                .iter()
                .flatten()
                .map(TileIndex::heap_bytes)
                .sum::<usize>()
    }
}
