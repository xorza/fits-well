//! One-row containers of tiled-compressed images, for tests that build the cells a codec
//! reads: a [`RawTable`] of one row, with the image's `Z*` keywords.

use crate::bintable::Table;
use crate::header_model::{Header, keywords};
use crate::internals::raw_table::RawTable;

/// The container of a tiled-compressed image of one tile row.
#[derive(Debug)]
pub(crate) struct OneTile {
    cmptype: String,
    zbitpix: i64,
    shape: Vec<i64>,
    tile: Vec<i64>,
    raw: RawTable,
}

impl OneTile {
    /// An image of `shape` stored as one tile of the whole shape.
    pub(crate) fn new(cmptype: &str, zbitpix: i64, shape: &[i64]) -> Self {
        Self {
            cmptype: cmptype.to_string(),
            zbitpix,
            shape: shape.to_vec(),
            tile: shape.to_vec(),
            raw: RawTable::new(1),
        }
    }

    /// No `ZTILEn`, so the tiles are the convention's default rows.
    #[must_use]
    pub(crate) fn without_tiles(mut self) -> Self {
        self.tile.clear();
        self
    }

    /// A `1P{code}` column `ttype` whose one cell holds `bytes`.
    #[must_use]
    pub(crate) fn heap(mut self, ttype: &str, code: char, bytes: &[u8]) -> Self {
        self.raw = self.raw.heap(Some(ttype), code, &[bytes]);
        self
    }

    /// A fixed column `ttype` of `tform` whose one cell holds `bytes`.
    #[must_use]
    pub(crate) fn fixed(mut self, ttype: &str, tform: &str, bytes: &[u8]) -> Self {
        self.raw = self.raw.fixed(Some(ttype), tform, &[bytes]);
        self
    }

    pub(crate) fn header(&self) -> Header {
        let mut h = self.raw.header();
        h.set_internal(keywords::ZIMAGE, true)
            .set_internal(keywords::ZCMPTYPE, self.cmptype.as_str())
            .set_internal(keywords::ZBITPIX, self.zbitpix)
            .set_internal(keywords::ZNAXIS, i64::try_from(self.shape.len()).unwrap());
        for (index, &length) in self.shape.iter().enumerate() {
            h.set_internal(format!("ZNAXIS{}", index + 1), length);
        }
        for (index, &length) in self.tile.iter().enumerate() {
            h.set_internal(format!("ZTILE{}", index + 1), length);
        }
        h
    }

    /// The data unit: the row, then the heap.
    pub(crate) fn data(&self) -> Vec<u8> {
        self.raw.data()
    }

    pub(crate) fn table(&self) -> Table {
        Table::from_data(&self.header(), self.data()).unwrap()
    }
}
