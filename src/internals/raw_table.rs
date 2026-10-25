//! Binary tables laid out by hand for tests and benches: the header's `TFORMn`, `NAXIS1`,
//! `PCOUNT` and `TTYPEn`, and every `P` descriptor, follow from the cells.

use crate::bintable::internals::table_header;
use crate::header_model::{Header, keywords};

/// One column: its optional `TTYPEn` and its cells, one per row.
#[derive(Debug)]
struct RawColumn {
    ttype: Option<String>,
    cells: Cells,
}

#[derive(Debug)]
enum Cells {
    /// Fixed cells of `tform`, their bytes in the rows.
    Fixed { tform: String, rows: Vec<Vec<u8>> },
    /// `1P` descriptors of elements of `code`, each row's bytes in the heap.
    Heap { code: char, rows: Vec<Vec<u8>> },
}

/// A binary table of `row_count` rows, built a column at a time.
#[derive(Debug)]
pub(crate) struct RawTable {
    row_count: usize,
    columns: Vec<RawColumn>,
}

impl RawTable {
    pub(crate) const fn new(row_count: usize) -> Self {
        Self {
            row_count,
            columns: Vec::new(),
        }
    }

    /// A fixed column `ttype` of `tform` whose cells, one per row, hold `rows`.
    #[must_use]
    pub(crate) fn fixed(mut self, ttype: Option<&str>, tform: &str, rows: &[&[u8]]) -> Self {
        self.push(
            ttype,
            Cells::Fixed {
                tform: tform.to_string(),
                rows: rows.iter().map(|row| row.to_vec()).collect(),
            },
        );
        self
    }

    /// A `1P{code}` column `ttype` whose heap arrays, one per row, hold `rows`.
    #[must_use]
    pub(crate) fn heap(mut self, ttype: Option<&str>, code: char, rows: &[&[u8]]) -> Self {
        let size = element_size(code);
        for row in rows {
            assert!(row.len().is_multiple_of(size), "a partial {code} element");
        }
        self.push(
            ttype,
            Cells::Heap {
                code,
                rows: rows.iter().map(|row| row.to_vec()).collect(),
            },
        );
        self
    }

    fn push(&mut self, ttype: Option<&str>, cells: Cells) {
        let rows = match &cells {
            Cells::Fixed { rows, .. } | Cells::Heap { rows, .. } => rows.len(),
        };
        assert_eq!(rows, self.row_count, "a cell for each row");
        self.columns.push(RawColumn {
            ttype: ttype.map(str::to_string),
            cells,
        });
    }

    /// The header: the structure, each `TFORMn` and `TTYPEn`, and `PCOUNT` for the heap.
    pub(crate) fn header(&self) -> Header {
        let tforms: Vec<String> = self
            .columns
            .iter()
            .map(|column| match &column.cells {
                Cells::Fixed { tform, .. } => tform.clone(),
                Cells::Heap { code, rows } => {
                    let longest = rows.iter().map(Vec::len).max().unwrap_or(0);
                    format!("1P{code}({})", longest / element_size(*code))
                }
            })
            .collect();
        let tforms: Vec<&str> = tforms.iter().map(String::as_str).collect();
        let mut header = table_header(self.row_len(), self.row_count, &tforms);
        header.set_internal(
            keywords::PCOUNT,
            i64::try_from(self.heap_bytes().len()).unwrap(),
        );
        for (index, column) in self.columns.iter().enumerate() {
            if let Some(ttype) = &column.ttype {
                header.set_internal(keywords::TTYPE.indexed(index + 1).unwrap(), ttype.as_str());
            }
        }
        header
    }

    /// The data unit: the rows, each column's cell in turn, then the heap.
    pub(crate) fn data(&self) -> Vec<u8> {
        let mut data = Vec::new();
        let mut offsets: Vec<usize> = vec![0; self.columns.len()];
        let mut heap_offset = 0;
        // The heap holds each column's arrays in row order, column after column.
        for (index, column) in self.columns.iter().enumerate() {
            offsets[index] = heap_offset;
            if let Cells::Heap { rows, .. } = &column.cells {
                heap_offset += rows.iter().map(Vec::len).sum::<usize>();
            }
        }
        for row in 0..self.row_count {
            for (index, column) in self.columns.iter().enumerate() {
                match &column.cells {
                    Cells::Fixed { rows, .. } => data.extend_from_slice(&rows[row]),
                    Cells::Heap { code, rows } => {
                        let count = rows[row].len() / element_size(*code);
                        data.extend_from_slice(&i32::try_from(count).unwrap().to_be_bytes());
                        let at = offsets[index];
                        data.extend_from_slice(&i32::try_from(at).unwrap().to_be_bytes());
                        offsets[index] += rows[row].len();
                    }
                }
            }
        }
        data.extend_from_slice(&self.heap_bytes());
        data
    }

    fn row_len(&self) -> usize {
        self.columns
            .iter()
            .map(|column| match &column.cells {
                Cells::Fixed { rows, .. } => rows.first().map_or(0, Vec::len),
                Cells::Heap { .. } => 8,
            })
            .sum()
    }

    fn heap_bytes(&self) -> Vec<u8> {
        self.columns
            .iter()
            .filter_map(|column| match &column.cells {
                Cells::Heap { rows, .. } => Some(rows.concat()),
                Cells::Fixed { .. } => None,
            })
            .flatten()
            .collect()
    }
}

/// The bytes of one heap element of the `TFORMn` letter `code`.
fn element_size(code: char) -> usize {
    match code {
        'B' | 'L' | 'A' => 1,
        'I' => 2,
        'J' | 'E' => 4,
        'K' | 'D' | 'C' => 8,
        'M' => 16,
        other => panic!("no heap element of code {other}"),
    }
}
