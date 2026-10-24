//! [`TileIndexes`]: the tile index of each compressed image a reader opened.

use crate::allocation;
use crate::bintable::schema::Schema;
use crate::compression::decode::tile_index::TileIndex;
use crate::fits_error::Result;

/// The tile indexes of the compressed images a reader opened, one for each HDU: each read
/// when its image first opens and kept for every later handle on it, so a handle on one
/// image never makes another read its index again.
#[derive(Debug, Default)]
pub(crate) struct TileIndexes {
    indexes: Vec<TileIndex>,
}

impl TileIndexes {
    /// Hold the index of the container `schema` describes, the data unit at `key`, which
    /// `read_rows` appends to the buffer it is given, unless it is held already.
    pub(crate) fn load(
        &mut self,
        key: u64,
        schema: &Schema,
        read_rows: impl FnOnce(usize, &mut Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        if self.indexes.iter().any(|index| index.key() == key) {
            return Ok(());
        }
        let index = TileIndex::read(key, schema, read_rows)?;
        allocation::try_reserve(&mut self.indexes, 1)?;
        self.indexes.push(index);
        Ok(())
    }

    /// The heap the indexes hold, with their own slots.
    pub(crate) fn heap_bytes(&self) -> usize {
        self.indexes.capacity() * size_of::<TileIndex>()
            + self
                .indexes
                .iter()
                .map(TileIndex::heap_bytes)
                .sum::<usize>()
    }

    /// The index of the container at data offset `key`, which its image's open loaded.
    pub(crate) fn get(&self, key: u64) -> &TileIndex {
        self.indexes
            .iter()
            .find(|index| index.key() == key)
            .expect("a compressed image's open loads its index")
    }
}
