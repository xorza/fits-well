//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. This
//! module parses that structure into [`Column`] descriptors; decoding goes through
//! a [`ColumnReader`] (from [`BinTable::column_by_idx`] / [`BinTable::column_by_name`]),
//! whose methods yield typed [`ColumnData`] ([`ColumnReader::raw`]), the
//! `TSCALn`/`TZEROn` physical plane ([`ColumnReader::physical`]), and `P`/`Q`
//! variable-length arrays out of the heap ([`ColumnReader::vla`]), including
//! their numeric, complex, and exact unsigned physical views.

pub(crate) mod descriptor;

use std::ops::Index;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::view::BitView;
use num_complex::Complex;

use crate::bitpix::Bitpix;
use crate::column;
use crate::column::Named;
use crate::data::Scaling;
use crate::data::UnsignedData;
use crate::data::UnsignedKind;
use crate::endian::decode_be_cells;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::keyword::key;
use crate::table_impl::descriptor::PqDescriptor;

/// The element type of a binary-table column, from the letter of its `TFORMn`
/// code (Table 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TformKind {
    /// `L` — logical (one ASCII `T`/`F` byte per element).
    Logical,
    /// `X` — bit array (`repeat` bits packed into `ceil(repeat/8)` bytes).
    Bit,
    /// `B` — unsigned byte.
    Byte,
    /// `I` — 16-bit integer.
    I16,
    /// `J` — 32-bit integer.
    I32,
    /// `K` — 64-bit integer.
    I64,
    /// `A` — character (a `repeat`-length string per row).
    Char,
    /// `E` — single-precision float.
    F32,
    /// `D` — double-precision float.
    F64,
    /// `C` — single-precision complex (real, imaginary).
    ComplexF32,
    /// `M` — double-precision complex.
    ComplexF64,
    /// `P` — 32-bit variable-length-array descriptor (into the heap).
    ArrayDesc32,
    /// `Q` — 64-bit variable-length-array descriptor.
    ArrayDesc64,
}

impl TformKind {
    fn from_code(code: u8) -> Option<TformKind> {
        Some(match code {
            b'L' => TformKind::Logical,
            b'X' => TformKind::Bit,
            b'B' => TformKind::Byte,
            b'I' => TformKind::I16,
            b'J' => TformKind::I32,
            b'K' => TformKind::I64,
            b'A' => TformKind::Char,
            b'E' => TformKind::F32,
            b'D' => TformKind::F64,
            b'C' => TformKind::ComplexF32,
            b'M' => TformKind::ComplexF64,
            b'P' => TformKind::ArrayDesc32,
            b'Q' => TformKind::ArrayDesc64,
            _ => return None,
        })
    }

    /// The `TFORMn` letter for this kind.
    pub fn code(self) -> char {
        match self {
            TformKind::Logical => 'L',
            TformKind::Bit => 'X',
            TformKind::Byte => 'B',
            TformKind::I16 => 'I',
            TformKind::I32 => 'J',
            TformKind::I64 => 'K',
            TformKind::Char => 'A',
            TformKind::F32 => 'E',
            TformKind::F64 => 'D',
            TformKind::ComplexF32 => 'C',
            TformKind::ComplexF64 => 'M',
            TformKind::ArrayDesc32 => 'P',
            TformKind::ArrayDesc64 => 'Q',
        }
    }

    /// Bytes per element. For `X` this is the per-*bit* size (1) — use
    /// [`Tform::byte_width`] for a column's true in-row width.
    pub(crate) fn elem_size(self) -> usize {
        match self {
            TformKind::Logical | TformKind::Bit | TformKind::Byte | TformKind::Char => 1,
            TformKind::I16 => 2,
            TformKind::I32 | TformKind::F32 => 4,
            TformKind::I64 | TformKind::F64 | TformKind::ComplexF32 | TformKind::ArrayDesc32 => 8,
            TformKind::ComplexF64 | TformKind::ArrayDesc64 => 16,
        }
    }
}

/// A parsed `TFORMn` value: a repeat count, an element kind, and (for the `P`/`Q`
/// variable-length-array descriptors) the kind of the array elements in the heap.
/// The `rTa` form's trailing `(emax)` size hint is not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tform {
    pub repeat: usize,
    pub kind: TformKind,
    /// For `P`/`Q` columns, the element kind of the heap array (the `t` in
    /// `rPt(emax)`); `None` for fixed-width columns.
    pub vla_elem: Option<TformKind>,
}

impl Tform {
    /// Parse a `TFORMn` value such as `"8A"`, `"3D"`, `"1J"`, `"E"`, or `"1PE(5)"`.
    pub fn parse(value: &str) -> Result<Tform> {
        let s = value.trim();
        let invalid = || FitsError::InvalidTform {
            tform: value.to_string(),
        };
        let pos = s
            .bytes()
            .position(|b| b.is_ascii_alphabetic())
            .ok_or_else(invalid)?;
        let repeat = if pos == 0 {
            1
        } else {
            s[..pos].parse().map_err(|_| invalid())?
        };
        let bytes = s.as_bytes();
        let kind = TformKind::from_code(bytes[pos]).ok_or_else(invalid)?;
        // A P/Q descriptor is followed by its heap element-type letter (`rPt`) and
        // may carry one complete `(emax)` hint. Fixed formats end after their code.
        let vla_elem = if matches!(kind, TformKind::ArrayDesc32 | TformKind::ArrayDesc64) {
            let elem = bytes.get(pos + 1).copied().ok_or_else(invalid)?;
            // §6.3: a `P`/`Q` descriptor's repeat count is restricted to 0 or 1.
            if repeat > 1 {
                return Err(invalid());
            }
            let elem = TformKind::from_code(elem).ok_or_else(invalid)?;
            if matches!(elem, TformKind::ArrayDesc32 | TformKind::ArrayDesc64) {
                return Err(invalid());
            }
            let suffix = &s[pos + 2..];
            if !suffix.is_empty() {
                let hint = suffix
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .filter(|value| {
                        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .ok_or_else(invalid)?;
                hint.parse::<usize>().map_err(|_| invalid())?;
            }
            Some(elem)
        } else {
            if pos + 1 != s.len() {
                return Err(invalid());
            }
            None
        };
        Ok(Tform {
            repeat,
            kind,
            vla_elem,
        })
    }

    /// The number of bytes this column occupies in every row.
    pub fn byte_width(self) -> usize {
        match self.kind {
            TformKind::Bit => self.repeat.div_ceil(8),
            // Saturating: an absurd `repeat` from a hostile `TFORMn` saturates to
            // `usize::MAX` rather than wrapping to a small width that could slip
            // past the row-width check in `from_data`.
            _ => self.repeat.saturating_mul(self.kind.elem_size()),
        }
    }
}

/// One column of a binary table: its `TFORMn` format, optional name/unit, the
/// `TSCALn`/`TZEROn`/`TNULLn` metadata, and its byte offset within a row.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub tform: Tform,
    /// `TSCALn` (default 1.0); applied by [`ColumnReader::physical`].
    pub tscale: f64,
    /// `TZEROn` (default 0.0); applied by [`ColumnReader::physical`].
    pub tzero: f64,
    /// `TNULLn`, the integer value denoting an undefined element, if declared.
    pub tnull: Option<i64>,
    /// `TDIMn` array shape (e.g. `'(4,4)'` → `[4, 4]`), if declared — reshapes the
    /// `repeat` elements of each row into a multidimensional array (§7.3.2).
    pub tdim: Option<Vec<usize>>,
    /// Raw `TDISPn` display recommendation (§7.3.4), if declared.
    pub tdisp: Option<String>,
    /// Byte offset of this column from the start of a row.
    pub byte_offset: usize,
}

/// One binary-table `A` field with its stored bytes preserved exactly.
///
/// [`CharacterField::members`] stops at the first NUL, while [`CharacterField::bytes`]
/// retains the terminator, undefined bytes after it, and all trailing spaces. A NUL
/// in the first byte is therefore distinguishable from an empty or all-space field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterField {
    pub bytes: Vec<u8>,
}

impl CharacterField {
    pub fn new(bytes: impl Into<Vec<u8>>) -> CharacterField {
        CharacterField {
            bytes: bytes.into(),
        }
    }

    /// The defined character members, ending immediately before the first NUL.
    pub fn members(&self) -> &[u8] {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.bytes.len());
        &self.bytes[..end]
    }

    /// Whether this is the FITS null string, identified by an initial NUL.
    pub fn is_null(&self) -> bool {
        self.bytes.first() == Some(&0)
    }

    /// Construct the shortest stored representation of a FITS null string.
    pub fn null() -> CharacterField {
        CharacterField { bytes: vec![0] }
    }
}

impl From<&str> for CharacterField {
    fn from(value: &str) -> CharacterField {
        CharacterField::new(value.as_bytes().to_vec())
    }
}

impl From<String> for CharacterField {
    fn from(value: String) -> CharacterField {
        CharacterField::new(value.into_bytes())
    }
}

impl From<Vec<u8>> for CharacterField {
    fn from(value: Vec<u8>) -> CharacterField {
        CharacterField::new(value)
    }
}

/// A decoded column, flattened across all rows in row order. For array columns
/// (`repeat > 1`) each row contributes `repeat` consecutive elements; binary `A`
/// contributes one exact [`CharacterField`] per row. Values are raw (big-endian
/// decoded but not `TSCALn`/`TZEROn`-scaled).
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// `L` — `Some(true)`/`Some(false)`, or `None` for the `0x00` null value (§7.3.3).
    Logical(Vec<Option<bool>>),
    /// `B` (bytes) and `X` (packed bits).
    Bytes(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    ComplexF32(Vec<Complex<f32>>),
    ComplexF64(Vec<Complex<f64>>),
    /// Binary-table `A` — one exact field per row for fixed columns; a VLA row uses
    /// zero fields for an empty descriptor or one field containing its heap bytes.
    Character(Vec<CharacterField>),
}

impl ColumnData {
    /// Total element count across all rows (the backing `Vec`'s length).
    pub fn element_count(&self) -> usize {
        match self {
            ColumnData::Logical(v) => v.len(),
            ColumnData::Bytes(v) => v.len(),
            ColumnData::I16(v) => v.len(),
            ColumnData::I32(v) => v.len(),
            ColumnData::I64(v) => v.len(),
            ColumnData::F32(v) => v.len(),
            ColumnData::F64(v) => v.len(),
            ColumnData::ComplexF32(v) => v.len(),
            ColumnData::ComplexF64(v) => v.len(),
            ColumnData::Character(v) => v.len(),
        }
    }
}

/// A binary table's structure plus its data unit.
#[derive(Debug, Clone)]
pub struct BinTable {
    /// Everything the header alone determines — the same [`TableSchema`] that
    /// [`BinTable::schema`] parses, held rather than re-derived.
    pub(crate) schema: TableSchema,
    /// The whole data unit (the `nrows * row_len` main table, then the heap and
    /// block fill). Fixed-width reads index the main-table prefix; `P`/`Q` columns
    /// follow their descriptors into the heap.
    bytes: Vec<u8>,
}

/// Immutable row and column metadata for a parsed binary table.
#[derive(Debug, Clone, Copy)]
pub struct BinTableMetadata<'a> {
    /// Number of rows in the table.
    pub nrows: usize,
    /// Validated column descriptors in `TFIELDS` order.
    pub columns: &'a [Column],
}

/// Binary-table schema parsed entirely from the header, without reading the data
/// unit. Byte offsets are relative to the start of the data unit.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub nrows: usize,
    /// Byte width of one row (`NAXIS1`).
    pub row_len: usize,
    pub heap_offset: usize,
    pub heap_end: usize,
    pub columns: Vec<Column>,
}

impl TableSchema {
    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7. Resolvable from the header alone, so a caller
    /// holding only a [`crate::FitsReader::table_schema`] can locate a column
    /// without reading the data unit.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        column::index_of(&self.columns, name)
    }

    /// [`TableSchema::column_index`], reporting an absent column as
    /// [`FitsError::ColumnNotFound`].
    pub(crate) fn column_index_checked(&self, name: &str) -> Result<usize> {
        column::checked_index_of(&self.columns, name)
    }
}

impl Named for Column {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl BinTable {
    /// Borrow the table's validated row count and column descriptors.
    pub fn metadata(&self) -> BinTableMetadata<'_> {
        BinTableMetadata {
            nrows: self.schema.nrows,
            columns: &self.schema.columns,
        }
    }

    /// Parse the complete binary-table schema without touching its data unit.
    pub fn schema(header: &Header) -> Result<TableSchema> {
        let row_len = header.required_usize("NAXIS1", "NAXIS1")?;
        let nrows = header.required_usize("NAXIS2", "NAXIS2")?;
        // §7.3.1: `0 ≤ TFIELDS ≤ 999` — also a guard, since `tfields` sizes the
        // column `Vec` and drives the `TFORMn` loop (an absurd value would abort).
        let tfields = header.required_usize("TFIELDS", "TFIELDS")?;
        validate_table_field_count(tfields)?;

        let mut columns = Vec::with_capacity(tfields);
        let mut offset = 0;
        for n in 1..=tfields {
            let tform_value = header.required_text(key!("TFORM{n}").as_str(), "TFORMn")?;
            let tform = Tform::parse(tform_value)?;
            let tdim = header
                .get_text(key!("TDIM{n}").as_str())?
                .map(parse_tdim)
                .transpose()?;
            // A fixed column's cell holds exactly `repeat` elements; a `P`/`Q` cell's
            // count is per-row and is checked as each row's descriptor is read.
            let is_vla = matches!(tform.kind, TformKind::ArrayDesc32 | TformKind::ArrayDesc64);
            if let Some(dims) = &tdim
                && !is_vla
            {
                validate_tdim_extent(dims, tform.repeat)?;
            }
            columns.push(Column {
                name: header
                    .get_text(key!("TTYPE{n}").as_str())?
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(key!("TUNIT{n}").as_str())?
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                tform,
                tscale: header.get_real(key!("TSCAL{n}").as_str())?.unwrap_or(1.0),
                tzero: header.get_real(key!("TZERO{n}").as_str())?.unwrap_or(0.0),
                tnull: header.get_integer(key!("TNULL{n}").as_str())?,
                tdim,
                tdisp: header
                    .get_text(key!("TDISP{n}").as_str())?
                    .map(str::to_string),
                byte_offset: offset,
            });
            offset = offset.saturating_add(tform.byte_width());
        }
        if offset != row_len {
            return Err(FitsError::RowWidthMismatch {
                computed: offset,
                declared: row_len,
            });
        }

        // `nrows · row_len` from untrusted axes: check once (guards a 32-bit-usize
        // overflow that `data_extent`'s u64 math wouldn't catch) and reuse.
        let main_table = nrows.checked_mul(row_len).ok_or(FitsError::UnexpectedEof)?;
        let heap_offset = header.optional_usize("THEAP", "THEAP", main_table)?;
        // §6.6: the heap follows the main table, so THEAP must be ≥ its size.
        if heap_offset < main_table {
            return Err(FitsError::KeywordOutOfRange { name: "THEAP" });
        }
        // PCOUNT counts the gap-plus-heap bytes after the main table, so the real
        // heap ends here — anything past it is block fill (§6.6).
        let pcount = header.optional_usize("PCOUNT", "PCOUNT", 0)?;
        let heap_end = main_table
            .checked_add(pcount)
            .ok_or(FitsError::UnexpectedEof)?;
        if heap_offset > heap_end {
            return Err(FitsError::KeywordOutOfRange { name: "THEAP" });
        }
        Ok(TableSchema {
            nrows,
            row_len,
            heap_offset,
            heap_end,
            columns,
        })
    }

    /// Build a table from its header and owned data unit (`data` is the main
    /// table followed by the optional heap, as returned by the reader).
    pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<BinTable> {
        let schema = BinTable::schema(header)?;
        if data.len() < schema.heap_end {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(BinTable {
            schema,
            bytes: data,
        })
    }

    /// The fixed-width main table (`nrows × NAXIS1` bytes), excluding the heap.
    #[cfg(feature = "compression")]
    pub(crate) fn raw_rows(&self) -> Result<&[u8]> {
        let len = self
            .schema
            .nrows
            .checked_mul(self.schema.row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.bytes.get(..len).ok_or(FitsError::DataSizeMismatch {
            expected: len,
            got: self.bytes.len(),
        })
    }

    pub(crate) fn pq_payload(
        &self,
        descriptor: PqDescriptor,
        element_kind: TformKind,
    ) -> Result<&[u8]> {
        let range =
            descriptor.heap_range(element_kind, self.schema.heap_offset, self.schema.heap_end)?;
        Ok(&self.bytes[range])
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.schema.column_index(name)
    }

    /// A reader handle for the column at `index`. Decode through it — [`ColumnReader`]
    /// exposes `raw`/`physical`/`unsigned`/`complex`/`bits` and the `vla*` variants —
    /// without re-passing the column descriptor. Errors with
    /// [`FitsError::ColumnIndexOutOfBounds`] for a bad index.
    pub fn column_by_idx(&self, index: usize) -> Result<ColumnReader<'_>> {
        column::validate_index(index, self.schema.columns.len())?;
        Ok(ColumnReader { table: self, index })
    }

    /// A reader handle for the column named `name` (`TTYPEn`, case-insensitive, §6.7).
    /// Errors with [`FitsError::ColumnNotFound`] if no such column exists.
    pub fn column_by_name(&self, name: &str) -> Result<ColumnReader<'_>> {
        let index = self.schema.column_index_checked(name)?;
        Ok(ColumnReader { table: self, index })
    }

    /// The raw bytes of column `col` in row `r`.
    fn cell(&self, col: &Column, r: usize) -> &[u8] {
        let start = r * self.schema.row_len + col.byte_offset;
        &self.bytes[start..start + col.tform.byte_width()]
    }

    fn pq_descriptor(&self, col: &Column, row: usize) -> Result<PqDescriptor> {
        if col.tform.repeat == 0 {
            Ok(PqDescriptor::EMPTY)
        } else {
            PqDescriptor::decode(
                self.cell(col, row),
                col.tform.kind == TformKind::ArrayDesc64,
            )
        }
    }

    fn cells<'a>(&'a self, col: &'a Column) -> impl ExactSizeIterator<Item = &'a [u8]> + 'a {
        (0..self.schema.nrows).map(move |row| self.cell(col, row))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VlaCell<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) element_count: usize,
    pub(crate) element_type: TformKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VlaColumn<'a> {
    table: &'a BinTable,
    index: usize,
    element_type: TformKind,
}

/// A handle to one column of a [`BinTable`], from [`BinTable::column_by_idx`] or
/// [`BinTable::column_by_name`]. Decode through it without re-passing the column
/// descriptor: [`raw`](Self::raw) for the typed values, [`physical`](Self::physical)
/// for the scaled `f64` plane, [`unsigned`](Self::unsigned)/[`complex`](Self::complex)/
/// [`bits`](Self::bits) for the special kinds, and [`vla`](Self::vla) (+
/// [`vla_physical`](Self::vla_physical)/[`vla_unsigned`](Self::vla_unsigned)/
/// [`vla_complex`](Self::vla_complex)/[`vla_bits`](Self::vla_bits)) for variable-length
/// `P`/`Q` columns. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub struct ColumnReader<'a> {
    table: &'a BinTable,
    index: usize,
}

impl<'a> ColumnReader<'a> {
    /// The column's [`Column`] descriptor — name, `TFORMn`, `TSCALn`/`TZEROn`/`TNULLn`,
    /// `TDIMn`, `TDISPn`.
    pub fn descriptor(&self) -> &'a Column {
        &self.table.schema.columns[self.index]
    }

    /// The column descriptor, rejecting a variable-length (`P`/`Q`) column — the
    /// shared precondition of every fixed-width decode. Those columns carry heap
    /// descriptors rather than values, so they go through [`ColumnReader::vla`].
    fn fixed_descriptor(&self) -> Result<&'a Column> {
        let col = self.descriptor();
        if matches!(
            col.tform.kind,
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64
        ) {
            return Err(FitsError::VariableLengthColumn {
                code: col.tform.kind.code(),
            });
        }
        Ok(col)
    }

    /// Decode a fixed-width column into a typed, row-flattened [`ColumnData`]: `A` is
    /// one exact [`CharacterField`] per row, every other fixed kind decodes from the
    /// concatenated cell bytes. Variable-length (`P`/`Q`) columns error here — use
    /// [`ColumnReader::vla`].
    pub fn raw(&self) -> Result<ColumnData> {
        let col = self.fixed_descriptor()?;
        Ok(decode_fixed_column(self.table, col))
    }

    /// The numeric column scaled to its physical `f64` plane: `TZEROn + TSCALn × raw`,
    /// mapping integers equal to `TNULLn` to `NaN`. Errors for the non-numeric kinds
    /// (`A`/`L`/`X`/`C`/`M`) and variable-length columns.
    pub fn physical(&self) -> Result<Vec<f64>> {
        let col = self.fixed_descriptor()?;
        physical_cells(
            self.table.cells(col),
            self.table.schema.nrows * col.tform.repeat,
            col.tform.kind,
            col.tscale,
            col.tzero,
            col.tnull,
        )
    }

    /// Exact typed integers when the column uses the FITS unsigned (or signed-byte)
    /// convention — `TSCALn == 1`, no `TNULLn`, `TZEROn` the matching sign-bit offset
    /// on a `B`/`I`/`J`/`K` column — without the `f64` rounding of
    /// [`physical`](Self::physical). `Ok(None)` for any other column; errors only for a
    /// variable-length column. Mirrors [`crate::image::Image::unsigned`].
    pub fn unsigned(&self) -> Result<Option<UnsignedData>> {
        let col = self.fixed_descriptor()?;
        let Some(kind) = unsigned_kind(col.tform.kind, col.tscale, col.tzero, col.tnull) else {
            return Ok(None);
        };
        Ok(Some(UnsignedData::from_be_cells(
            self.table.cells(col),
            self.table.schema.nrows * col.tform.repeat,
            kind,
        )))
    }

    /// A `C`/`M` complex column as [`Complex<f64>`] values, applying `TSCALn` to both
    /// components and `TZEROn` to the real component (§7.3.2). Errors on non-complex columns.
    pub fn complex(&self) -> Result<Vec<Complex<f64>>> {
        let col = self.descriptor();
        complex_cells(
            self.table.cells(col),
            self.table.schema.nrows * col.tform.repeat,
            col.tform.kind,
            col.tscale,
            col.tzero,
        )
    }

    /// An `X` (bit-array) column as a borrowed 2-D [`BitColumn`] — `nrows × repeat`
    /// bits viewed in place over the data unit, MSB-first (bit 0 is the MSB of the
    /// first byte, §7.3.2), with no per-row allocation. Errors on any non-`X` column.
    pub fn bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        if col.tform.kind != TformKind::Bit {
            return Err(FitsError::NotABitColumn {
                code: col.tform.kind.code(),
            });
        }
        Ok(BitColumn {
            table: self.table,
            index: self.index,
        })
    }

    /// Decode a variable-length (`P`/`Q`) column: one [`ColumnData`] per row, each
    /// holding that row's heap array (which may be empty). Errors for fixed-width
    /// columns.
    pub fn vla(&self) -> Result<Vec<ColumnData>> {
        let column = self.vla_column()?;
        self.map_vla_rows(column, |cell| {
            Ok(decode_array(cell.element_type, cell.bytes))
        })
    }

    /// Scale each row of a `P`/`Q` column to its physical plane: `TZEROn + TSCALn ×
    /// element`, mapping integers equal to `TNULLn` to `NaN` (§6.4 — scaling applies to
    /// the heap values). Errors for fixed-width or non-numeric-heap columns.
    pub fn vla_physical(&self) -> Result<Vec<Vec<f64>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        self.map_vla_rows(column, |cell| {
            physical_cells(
                std::iter::once(cell.bytes),
                cell.element_count,
                cell.element_type,
                col.tscale,
                col.tzero,
                col.tnull,
            )
        })
    }

    /// Exact typed integers for each row of a `P`/`Q` heap array using the FITS
    /// unsigned (or signed-byte) convention. The outer vector is table rows; each
    /// row's [`UnsignedData`] owns its jagged array. Returns `Ok(None)` unless the
    /// heap type and `TSCALn`/`TZEROn`/`TNULLn` metadata form that convention.
    /// Errors for fixed-width columns.
    pub fn vla_unsigned(&self) -> Result<Option<Vec<UnsignedData>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        let Some(kind) = unsigned_kind(column.element_type, col.tscale, col.tzero, col.tnull)
        else {
            return Ok(None);
        };
        self.map_vla_rows(column, |cell| {
            Ok(UnsignedData::from_be_cells(
                std::iter::once(cell.bytes),
                cell.element_count,
                kind,
            ))
        })
        .map(Some)
    }

    /// Each row of a `PC`/`QC`/`PM`/`QM` heap array as [`Complex<f64>`], applying
    /// `TSCALn` to both components and `TZEROn` to only the real component (§7.3.2).
    /// Empty descriptors produce empty rows. Errors for fixed-width or non-complex
    /// heap columns.
    pub fn vla_complex(&self) -> Result<Vec<Vec<Complex<f64>>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        let kind = column.element_type;
        if !matches!(kind, TformKind::ComplexF32 | TformKind::ComplexF64) {
            return Err(FitsError::NotAComplexColumn { code: kind.code() });
        }
        self.map_vla_rows(column, |cell| {
            complex_cells(
                std::iter::once(cell.bytes),
                cell.element_count,
                kind,
                col.tscale,
                col.tzero,
            )
        })
    }

    /// A variable-length `X` (`1PX`/`1QX`) column as a borrowed 2-D [`BitColumn`],
    /// MSB-first (§7.3.2/§7.3.5 — the descriptor's element count is the bit count). The
    /// rows are *jagged* (each its own length), so [`BitColumn::row`]`(r).len()` gives a
    /// row's width. Errors on any non-bit VLA.
    pub fn vla_bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32, Some(TformKind::Bit))
            | (TformKind::ArrayDesc64, Some(TformKind::Bit)) => {}
            _ => {
                return Err(FitsError::NotABitColumn {
                    code: col.tform.kind.code(),
                });
            }
        };
        // Validate every row's heap span up front (no allocation) so [`BitColumn::row`]
        // can resolve a row lazily and infallibly — the only place an overrun surfaces.
        for r in 0..self.table.schema.nrows {
            let descriptor = self.table.pq_descriptor(col, r)?;
            validate_vla_tdim(col, descriptor.count)?;
            self.table.pq_payload(descriptor, TformKind::Bit)?;
        }
        Ok(BitColumn {
            table: self.table,
            index: self.index,
        })
    }

    fn vla_element_type(&self) -> Result<TformKind> {
        let col = self.descriptor();
        match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32 | TformKind::ArrayDesc64, Some(element_type)) => {
                Ok(element_type)
            }
            _ => Err(FitsError::NotAVla {
                code: col.tform.kind.code(),
            }),
        }
    }

    pub(crate) fn vla_column(&self) -> Result<VlaColumn<'a>> {
        Ok(VlaColumn {
            table: self.table,
            index: self.index,
            element_type: self.vla_element_type()?,
        })
    }

    fn map_vla_rows<T>(
        &self,
        column: VlaColumn<'a>,
        decode: impl Fn(VlaCell<'a>) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut rows = Vec::with_capacity(self.table.schema.nrows);
        for row in 0..self.table.schema.nrows {
            rows.push(decode(column.cell(row)?)?);
        }
        Ok(rows)
    }
}

impl<'a> VlaColumn<'a> {
    pub(crate) fn cell(&self, row: usize) -> Result<VlaCell<'a>> {
        if row >= self.table.schema.nrows {
            return Err(FitsError::UnexpectedEof);
        }
        let col = &self.table.schema.columns[self.index];
        let descriptor = self.table.pq_descriptor(col, row)?;
        validate_vla_tdim(col, descriptor.count)?;
        let bytes = self.table.pq_payload(descriptor, self.element_type)?;
        Ok(VlaCell {
            bytes,
            element_count: descriptor.count,
            element_type: self.element_type,
        })
    }
}

/// A binary table's `X` (bit-array) column as a borrowed, 2-D bit view — from
/// [`ColumnReader::bits`] (rectangular, `nrows × repeat`) or [`ColumnReader::vla_bits`]
/// (jagged `PX`/`QX`). Bits are MSB-first (§7.3.2) and viewed in place over the data
/// unit (zero-copy), so this borrows the table and can't outlive it.
///
/// Index a row (`flags[row]` → a [`BitSlice`]), a bit by nesting (`flags[row][col]`)
/// or by cell (`flags[(row, col)]`), reach for the checked [`get`](Self::get), or take
/// a row with the source lifetime via [`row`](Self::row). Rows are full `bitvec`
/// slices — `count_ones()`, `iter_ones()`, `.to_bitvec()` to own, etc.
///
/// ```ignore
/// let flags = table.column_by_name("DQ")?.bits()?;
/// let bit = flags[(row, 3)];             // bool (panics out of range)
/// let bit = flags[row][3];               // same, via the row slice
/// let bit = flags.get(row, 3);           // Option<bool> (checked)
/// let set = flags[row].count_ones();     // bitvec ops on the row
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BitColumn<'a> {
    table: &'a BinTable,
    index: usize,
}

impl<'a> BitColumn<'a> {
    /// The number of rows.
    pub fn nrows(&self) -> usize {
        self.table.schema.nrows
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.table.schema.nrows == 0
    }

    /// Row `r`'s bits as a borrowed [`BitSlice`], MSB-first — resolved on demand from
    /// the data unit (no per-row storage). Index it (`row[c]`), iterate it, or
    /// `.to_bitvec()` to own it. Panics if `r >= nrows()`.
    pub fn row(&self, r: usize) -> &'a BitSlice<u8, Msb0> {
        assert!(
            r < self.table.schema.nrows,
            "row {r} out of bounds ({} rows)",
            self.table.schema.nrows
        );
        let col = &self.table.schema.columns[self.index];
        if col.tform.kind == TformKind::Bit {
            // Fixed `rX`: the row's cell, truncated to `repeat` bits.
            &self.table.cell(col, r).view_bits::<Msb0>()[..col.tform.repeat]
        } else {
            // Variable-length `PX`/`QX`: follow the descriptor into the heap. The span
            // was bounds-checked by `vla_bits`, so the lookup can't fail here.
            let descriptor = self
                .table
                .pq_descriptor(col, r)
                .expect("vla_bits validated every descriptor");
            if descriptor.count == 0 {
                return self.table.bytes[..0].view_bits::<Msb0>();
            }
            let cell = self
                .table
                .pq_payload(descriptor, TformKind::Bit)
                .expect("vla_bits validated every heap span");
            &cell.view_bits::<Msb0>()[..descriptor.count]
        }
    }

    /// The bit at `(row, col)`, MSB-first — `None` if either index is out of range.
    pub fn get(&self, row: usize, col: usize) -> Option<bool> {
        if row >= self.table.schema.nrows {
            return None;
        }
        let bits = self.row(row);
        (col < bits.len()).then(|| bits[col])
    }

    /// Iterate the rows, each a borrowed [`BitSlice`], resolved on demand.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a BitSlice<u8, Msb0>> + '_ {
        (0..self.table.schema.nrows).map(move |r| self.row(r))
    }
}

/// `bits[row]` is row `row`'s [`BitSlice`] (panics out of range, like slice indexing);
/// `bits[row][col]` is the bit. Use [`BitColumn::get`] for the checked element.
impl Index<usize> for BitColumn<'_> {
    type Output = BitSlice<u8, Msb0>;

    fn index(&self, row: usize) -> &BitSlice<u8, Msb0> {
        self.row(row)
    }
}

/// `bits[(row, col)]` is the bit at that cell (panics out of range) — the matrix-style
/// counterpart of [`BitColumn::get`].
impl Index<(usize, usize)> for BitColumn<'_> {
    type Output = bool;

    fn index(&self, (row, col): (usize, usize)) -> &bool {
        &self.row(row)[col]
    }
}

/// Parse a `TDIMn` value `'(d1,d2,…)'` into axis lengths (fastest-varying first).
fn parse_tdim(value: &str) -> Result<Vec<usize>> {
    let invalid = || FitsError::KeywordOutOfRange { name: "TDIMn" };
    let inner = value
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(invalid)?;
    let dims: Vec<usize> = inner
        .split(',')
        .map(|value| value.trim().parse::<usize>().map_err(|_| invalid()))
        .collect::<Result<_>>()?;
    validate_tdim_shape(&dims)?;
    Ok(dims)
}

/// A `TDIMn` shape must name at least one axis and no zero-length one — the rule
/// both the parsed (`TDIMn` card) and supplied (writer) shapes obey.
fn validate_tdim_shape(dims: &[usize]) -> Result<()> {
    if dims.is_empty() || dims.contains(&0) {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
    Ok(())
}

/// §7.3.2: a `TDIMn` shape may describe fewer elements than the cell holds (trailing
/// elements beyond the declared view are permitted) but never more.
fn validate_tdim_extent(dims: &[usize], element_count: usize) -> Result<()> {
    let product = dims
        .iter()
        .try_fold(1usize, |product, &len| product.checked_mul(len))
        .ok_or(FitsError::KeywordOutOfRange { name: "TDIMn" })?;
    if product > element_count {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
    Ok(())
}

/// The extent rule as it applies to a `P`/`Q` heap array: an empty descriptor
/// carries no elements to reshape, so the declared shape is simply not applied to
/// that row.
fn validate_vla_tdim_extent(dims: &[usize], element_count: usize) -> Result<()> {
    if element_count == 0 {
        return Ok(());
    }
    validate_tdim_extent(dims, element_count)
}

/// Both rules for a *caller-supplied* fixed-width `TDIMn`: the shape is well-formed
/// and describes no more elements than the cell holds.
///
/// The read path applies only the extent rule, because a shape that came off a
/// `TDIMn` card was already checked for well-formedness by [`parse_tdim`]; the
/// writer's shape arrives straight from the caller and so needs both.
pub(crate) fn validate_declared_tdim(shape: Option<&[usize]>, element_count: usize) -> Result<()> {
    let Some(shape) = shape else {
        return Ok(());
    };
    validate_tdim_shape(shape)?;
    validate_tdim_extent(shape, element_count)
}

/// [`validate_declared_tdim`] for a `P`/`Q` row, which skips the extent rule on an
/// empty heap array.
pub(crate) fn validate_declared_vla_tdim(
    shape: Option<&[usize]>,
    element_count: usize,
) -> Result<()> {
    let Some(shape) = shape else {
        return Ok(());
    };
    validate_tdim_shape(shape)?;
    validate_vla_tdim_extent(shape, element_count)
}

/// The `TDIMn` extent check for a parsed column's `P`/`Q` heap array.
fn validate_vla_tdim(col: &Column, element_count: usize) -> Result<()> {
    match &col.tdim {
        Some(dims) => validate_vla_tdim_extent(dims, element_count),
        None => Ok(()),
    }
}

fn decode_scalar_cells<'a>(
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    kind: TformKind,
) -> ColumnData {
    match kind {
        TformKind::Logical => {
            ColumnData::Logical(decode_be_cells(cells, capacity, |[byte]| match byte {
                b'T' => Some(true),
                b'F' => Some(false),
                // 0x00 (or any non-T/F byte) is the §7.3.3 undefined value.
                _ => None,
            }))
        }
        TformKind::Byte | TformKind::Bit => {
            ColumnData::Bytes(decode_be_cells(cells, capacity, |[byte]| byte))
        }
        TformKind::I16 => ColumnData::I16(decode_be_cells(cells, capacity, i16::from_be_bytes)),
        TformKind::I32 => ColumnData::I32(decode_be_cells(cells, capacity, i32::from_be_bytes)),
        TformKind::I64 => ColumnData::I64(decode_be_cells(cells, capacity, i64::from_be_bytes)),
        TformKind::F32 => ColumnData::F32(decode_be_cells(cells, capacity, f32::from_be_bytes)),
        TformKind::F64 => ColumnData::F64(decode_be_cells(cells, capacity, f64::from_be_bytes)),
        TformKind::ComplexF32 => {
            ColumnData::ComplexF32(decode_be_cells(cells, capacity, |bytes: [u8; 8]| Complex {
                re: f32::from_be_bytes(bytes[..4].try_into().unwrap()),
                im: f32::from_be_bytes(bytes[4..].try_into().unwrap()),
            }))
        }
        TformKind::ComplexF64 => {
            ColumnData::ComplexF64(decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
                Complex {
                    re: f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                    im: f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                }
            }))
        }
        TformKind::Char | TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
            unreachable!("character and descriptor cells are resolved by the caller")
        }
    }
}

fn decode_fixed_cells<'a>(
    cells: impl ExactSizeIterator<Item = &'a [u8]>,
    row_count: usize,
    col: &Column,
) -> ColumnData {
    match col.tform.kind {
        // One exact field per row, padding and all — the row *is* the field.
        TformKind::Char => ColumnData::Character(
            cells
                .map(|cell| CharacterField::new(cell.to_vec()))
                .collect(),
        ),
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
            unreachable!("raw rejects VLA columns before fixed decode")
        }
        // `X` packs `repeat` bits into `byte_width` bytes, so its element count is the
        // byte width rather than the repeat.
        kind @ (TformKind::Byte | TformKind::Bit) => {
            decode_scalar_cells(cells, row_count * col.tform.byte_width(), kind)
        }
        kind => decode_scalar_cells(cells, row_count * col.tform.repeat, kind),
    }
}

fn decode_fixed_column(table: &BinTable, col: &Column) -> ColumnData {
    decode_fixed_cells(table.cells(col), table.schema.nrows, col)
}

pub(crate) fn decode_fixed_cell(col: &Column, bytes: &[u8]) -> ColumnData {
    debug_assert_eq!(bytes.len(), col.tform.byte_width());
    decode_fixed_cells(std::iter::once(bytes), 1, col)
}

/// Whether a column's `TSCALn`/`TZEROn`/`TNULLn` realize a FITS unsigned (or
/// signed-byte) convention, and which one. A table column's three keywords mean
/// exactly what an image's `BSCALE`/`BZERO`/`BLANK` do, so this only maps the
/// column's stored type onto the matching `BITPIX` and hands the trio to
/// [`Scaling::unsigned_kind`], which resolves the convention for both paths.
fn unsigned_kind(
    kind: TformKind,
    tscale: f64,
    tzero: f64,
    tnull: Option<i64>,
) -> Option<UnsignedKind> {
    let bitpix = match kind {
        TformKind::Byte => Bitpix::U8,
        TformKind::I16 => Bitpix::I16,
        TformKind::I32 => Bitpix::I32,
        TformKind::I64 => Bitpix::I64,
        _ => return None,
    };
    Scaling {
        bscale: tscale,
        bzero: tzero,
        blank: tnull,
    }
    .unsigned_kind(bitpix)
}

fn complex_cells<'a>(
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    kind: TformKind,
    tscale: f64,
    tzero: f64,
) -> Result<Vec<Complex<f64>>> {
    let scale = |re: f64, im: f64| Complex {
        re: tzero + tscale * re,
        im: tscale * im,
    };
    match kind {
        TformKind::ComplexF32 => Ok(decode_be_cells(cells, capacity, |bytes: [u8; 8]| {
            scale(
                f32::from_be_bytes(bytes[..4].try_into().unwrap()) as f64,
                f32::from_be_bytes(bytes[4..].try_into().unwrap()) as f64,
            )
        })),
        TformKind::ComplexF64 => Ok(decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
            scale(
                f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                f64::from_be_bytes(bytes[8..].try_into().unwrap()),
            )
        })),
        _ => Err(FitsError::NotAComplexColumn { code: kind.code() }),
    }
}

fn physical_cells<'a>(
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    kind: TformKind,
    tscale: f64,
    tzero: f64,
    tnull: Option<i64>,
) -> Result<Vec<f64>> {
    let scale = |x: f64| tzero + tscale * x;
    let scaled_int = |xi: i64| {
        if tnull == Some(xi) {
            f64::NAN
        } else {
            scale(xi as f64)
        }
    };
    Ok(match kind {
        TformKind::Byte => decode_be_cells(cells, capacity, |[x]| scaled_int(x as i64)),
        TformKind::I16 => decode_be_cells(cells, capacity, |bytes| {
            scaled_int(i16::from_be_bytes(bytes) as i64)
        }),
        TformKind::I32 => decode_be_cells(cells, capacity, |bytes| {
            scaled_int(i32::from_be_bytes(bytes) as i64)
        }),
        TformKind::I64 => decode_be_cells(cells, capacity, |bytes| {
            scaled_int(i64::from_be_bytes(bytes))
        }),
        TformKind::F32 => decode_be_cells(cells, capacity, |bytes| {
            scale(f32::from_be_bytes(bytes) as f64)
        }),
        TformKind::F64 => {
            decode_be_cells(cells, capacity, |bytes| scale(f64::from_be_bytes(bytes)))
        }
        _ => return Err(FitsError::NonNumericColumn { code: kind.code() }),
    })
}

/// Decode `bytes` as one contiguous run of `kind` elements — a `P`/`Q` row's heap
/// array. The scalar kinds share [`decode_scalar_cells`] with the fixed-width read;
/// only the two run-specific kinds are resolved here.
fn decode_array(kind: TformKind, bytes: &[u8]) -> ColumnData {
    match kind {
        // The whole run is one field: an empty descriptor yields no field at all.
        TformKind::Char if bytes.is_empty() => ColumnData::Character(Vec::new()),
        TformKind::Char => ColumnData::Character(vec![CharacterField::new(bytes.to_vec())]),
        // A heap element can't itself be a descriptor; keep the raw bytes.
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => ColumnData::Bytes(bytes.to_vec()),
        kind => decode_scalar_cells(std::iter::once(bytes), bytes.len() / kind.elem_size(), kind),
    }
}

pub(crate) fn decode_vla_cell(
    col: &Column,
    bytes: &[u8],
    element_count: usize,
) -> Result<ColumnData> {
    validate_vla_tdim(col, element_count)?;
    let element_type = col
        .tform
        .vla_elem
        .expect("validated VLA format carries an element type");
    let expected_len = descriptor::payload_len(element_type, element_count)?;
    debug_assert_eq!(bytes.len(), expected_len);
    Ok(decode_array(element_type, bytes))
}

#[cfg(all(test, feature = "compression"))]
pub(crate) mod internals {
    use crate::table_impl::{BinTable, TformKind};

    pub(crate) fn set_column_kind(table: &mut BinTable, column: usize, kind: TformKind) {
        table.schema.columns[column].tform.kind = kind;
    }
}

#[cfg(test)]
mod tests;
