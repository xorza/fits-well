//! One-row containers of tiled-compressed images, for tests that build the cells a codec
//! reads: the header's `TFORMn`, `NAXIS1`, `PCOUNT` and every descriptor follow from the
//! cells.

use crate::bintable::Table;
use crate::bintable::internals::table_header;
use crate::header_model::{Header, keywords};

#[derive(Debug)]
enum Cell {
    /// A `1P` descriptor of `count` elements of `code`, its bytes in the heap.
    Heap {
        code: char,
        count: usize,
        bytes: Vec<u8>,
    },
    /// A fixed field of `tform`, its bytes in the row.
    Fixed { tform: String, bytes: Vec<u8> },
}

/// The container of a tiled-compressed image of one tile row.
#[derive(Debug)]
pub(crate) struct OneTile {
    cmptype: String,
    zbitpix: i64,
    shape: Vec<i64>,
    tile: Vec<i64>,
    columns: Vec<(String, Cell)>,
}

impl OneTile {
    /// An image of `shape` stored as one tile of the whole shape.
    pub(crate) fn new(cmptype: &str, zbitpix: i64, shape: &[i64]) -> Self {
        Self {
            cmptype: cmptype.to_string(),
            zbitpix,
            shape: shape.to_vec(),
            tile: shape.to_vec(),
            columns: Vec::new(),
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
        let size = match code {
            'B' => 1,
            'I' => 2,
            'J' | 'E' => 4,
            'K' | 'D' => 8,
            other => panic!("no heap element of code {other}"),
        };
        assert!(
            bytes.len().is_multiple_of(size),
            "{ttype}: a partial element"
        );
        let cell = Cell::Heap {
            code,
            count: bytes.len() / size,
            bytes: bytes.to_vec(),
        };
        self.columns.push((ttype.to_string(), cell));
        self
    }

    /// A fixed column `ttype` of `tform` whose one cell holds `bytes`.
    #[must_use]
    pub(crate) fn fixed(mut self, ttype: &str, tform: &str, bytes: &[u8]) -> Self {
        let cell = Cell::Fixed {
            tform: tform.to_string(),
            bytes: bytes.to_vec(),
        };
        self.columns.push((ttype.to_string(), cell));
        self
    }

    pub(crate) fn header(&self) -> Header {
        let tforms: Vec<String> = self
            .columns
            .iter()
            .map(|(_, cell)| match cell {
                Cell::Heap { code, count, .. } => format!("1P{code}({count})"),
                Cell::Fixed { tform, .. } => tform.clone(),
            })
            .collect();
        let tforms: Vec<&str> = tforms.iter().map(String::as_str).collect();
        let mut h = table_header(self.row().len(), 1, &tforms);
        h.set_internal(
            keywords::PCOUNT,
            i64::try_from(self.heap_bytes().len()).unwrap(),
        );
        for (index, (ttype, _)) in self.columns.iter().enumerate() {
            h.set_internal(format!("TTYPE{}", index + 1), ttype.as_str());
        }
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
        let mut data = self.row();
        data.extend_from_slice(&self.heap_bytes());
        data
    }

    pub(crate) fn table(&self) -> Table {
        Table::from_data(&self.header(), self.data()).unwrap()
    }

    fn row(&self) -> Vec<u8> {
        let mut row = Vec::new();
        let mut offset = 0usize;
        for (_, cell) in &self.columns {
            match cell {
                Cell::Heap { count, bytes, .. } => {
                    row.extend_from_slice(&i32::try_from(*count).unwrap().to_be_bytes());
                    row.extend_from_slice(&i32::try_from(offset).unwrap().to_be_bytes());
                    offset += bytes.len();
                }
                Cell::Fixed { bytes, .. } => row.extend_from_slice(bytes),
            }
        }
        row
    }

    fn heap_bytes(&self) -> Vec<u8> {
        self.columns
            .iter()
            .filter_map(|(_, cell)| match cell {
                Cell::Heap { bytes, .. } => Some(bytes.as_slice()),
                Cell::Fixed { .. } => None,
            })
            .flatten()
            .copied()
            .collect()
    }
}

/// The big-endian bytes of `values`.
pub(crate) fn be<T: Copy, const N: usize>(values: &[T], to_be: impl Fn(T) -> [u8; N]) -> Vec<u8> {
    values.iter().flat_map(|&v| to_be(v)).collect()
}
