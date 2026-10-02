//! Tiled table compression (§10.3) — a port of cfitsio's `fits_compress_table`/
//! `fits_uncompress_table`.
//!
//! The table is split into row-tiles of `ZTILELEN` rows. Within a tile each
//! column is transposed to column-major order and compressed independently with
//! its `ZCTYPn` codec (`GZIP_1`/`GZIP_2`/`RICE_1`/`NOCOMPRESS`). The compressed
//! table is itself a `BINTABLE` with `ZTABLE = T`: one row per tile, one `1QB`
//! variable-length byte column per original column, the compressed bytes living
//! in the heap. The original `TFORMn`/`NAXIS1`/`NAXIS2`/`PCOUNT` are preserved
//! as `ZFORMn`/`ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT`.

use crate::allocation;
use crate::bintable::descriptor::PqDescriptor;
use crate::bintable::table_view::TableView;
use crate::bintable::tform::Tform;
use crate::bintable::tform_kind::TformKind;
use crate::compress::Compression;
use crate::compress::ImageCodec;
use crate::compress::convert;
use crate::compress::gzip;
use crate::compress::map_tiles;
use crate::compress::plane::IntBitpix;
use crate::compress::rice;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::validate_table_field_count;
use crate::header_model::Header;
use crate::header_model::value;
use crate::keyword::key;
use crate::ragged::Ragged;
use crate::reserved_keywords;
use std::mem;
use std::result;

/// Per-column compression algorithm (`ZCTYPn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Algo {
    Gzip1,
    Gzip2,
    Rice1,
    NoCompress,
}

impl Algo {
    /// `Algo` is exactly the subset of [`ImageCodec`] §10.3 permits for a table
    /// column — the wavelet and mask codecs have no column meaning. Keeping it a
    /// distinct type means the per-column match sites cannot be handed one.
    fn image_codec(self) -> ImageCodec {
        match self {
            Algo::Gzip1 => ImageCodec::Gzip1,
            Algo::Gzip2 => ImageCodec::Gzip2,
            Algo::Rice1 => ImageCodec::Rice1,
            Algo::NoCompress => ImageCodec::NoCompress,
        }
    }

    fn name(self) -> &'static str {
        self.image_codec().name()
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "the error is deliberately replaced by the one this context defines"
    )]
    fn parse(s: &str) -> Result<Algo> {
        let invalid = || FitsError::UnsupportedCompression {
            name: format!("table column codec {s}"),
        };
        match ImageCodec::parse(s).map_err(|_| invalid())? {
            ImageCodec::Gzip1 => Ok(Algo::Gzip1),
            ImageCodec::Gzip2 => Ok(Algo::Gzip2),
            ImageCodec::Rice1 => Ok(Algo::Rice1),
            ImageCodec::NoCompress => Ok(Algo::NoCompress),
            ImageCodec::Plio1 | ImageCodec::Hcompress1 => Err(invalid()),
        }
    }
}

/// One column's layout and codec, used by both directions.
#[derive(Debug)]
struct ColMeta {
    kind: TformKind,
    vla_elem: Option<TformKind>,
    /// Element width in bytes of the compressed values (the heap element for a
    /// `P`/`Q` column), e.g. 2 for `I`.
    elem_size: usize,
    /// Bytes per row for this column — the descriptor for a `P`/`Q` column.
    width: usize,
    /// Byte offset of this column within a row.
    offset: usize,
    /// Valid for the column's type: [`ColMeta::chosen`] clamps a request and
    /// [`ColMeta::declared`] refuses an invalid `ZCTYPn`.
    algo: Algo,
}

#[derive(Debug)]
struct BoundTable<'a> {
    rows: &'a [u8],
    pcount: usize,
}

impl ColMeta {
    /// The column `tform` at `offset`, compressed with `requested` clamped to a
    /// codec valid for its type, as cfitsio does when it writes.
    fn chosen(tform: &Tform, offset: usize, requested: Algo) -> ColMeta {
        let mut meta = ColMeta::declared_unchecked(tform, offset, requested);
        meta.algo = pick_algo(meta.compression_kind(), requested);
        meta
    }

    /// The column `tform` at `offset` with the codec its `ZCTYPn` declares. Errors
    /// when that codec cannot encode the column's type (`RICE_1` takes only `B`, `I`
    /// and `J` values).
    fn declared(tform: &Tform, offset: usize, algo: Algo) -> Result<ColMeta> {
        let meta = ColMeta::declared_unchecked(tform, offset, algo);
        if algo == Algo::Rice1 && meta.rice_width().is_none() {
            return Err(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", meta.compression_kind().code()),
            });
        }
        Ok(meta)
    }

    fn declared_unchecked(tform: &Tform, offset: usize, algo: Algo) -> ColMeta {
        ColMeta {
            kind: tform.kind,
            vla_elem: tform.vla_elem,
            elem_size: tform.vla_elem.unwrap_or(tform.kind).elem_size(),
            width: tform.byte_width(),
            offset,
            algo,
        }
    }

    fn is_vla(&self) -> bool {
        self.vla_elem.is_some()
    }

    fn is_stored_vla(&self) -> bool {
        self.is_vla() && self.width != 0
    }

    fn compression_kind(&self) -> TformKind {
        self.vla_elem.unwrap_or(self.kind)
    }

    /// `GZIP_2` byte-shuffle width: the element size for the multi-byte numeric
    /// types cfitsio shuffles (`I`/`J`/`E`/`K`/`D`), else 1 (no shuffle).
    fn shuffle_width(&self) -> usize {
        match self.compression_kind() {
            TformKind::I16 | TformKind::I32 | TformKind::F32 | TformKind::I64 | TformKind::F64 => {
                self.elem_size
            }
            _ => 1,
        }
    }

    /// `RICE_1` pixel width (`B`, `I` and `J`); other types can't use Rice.
    fn rice_width(&self) -> Option<IntBitpix> {
        match self.compression_kind() {
            TformKind::Byte => Some(IntBitpix::U8),
            TformKind::I16 => Some(IntBitpix::I16),
            TformKind::I32 => Some(IntBitpix::I32),
            _ => None,
        }
    }

    /// The `RICE_1` pixel width of a column whose codec is `RICE_1`.
    fn rice_bytepix(&self) -> IntBitpix {
        self.rice_width()
            .expect("both constructors keep RICE_1 to B, I and J columns")
    }
}

/// Clamp a requested algorithm to one valid for the column type, mirroring
/// cfitsio's per-type sanity overrides.
fn pick_algo(kind: TformKind, requested: Algo) -> Algo {
    if requested == Algo::NoCompress {
        return requested;
    }
    match kind {
        // Logical/bit/char/complex always gzip (Rice/shuffle are ill-defined).
        TformKind::Logical
        | TformKind::Bit
        | TformKind::Char
        | TformKind::ComplexF32
        | TformKind::ComplexF64 => {
            if requested == Algo::Gzip2 {
                Algo::Gzip2
            } else {
                Algo::Gzip1
            }
        }
        TformKind::F32 | TformKind::F64 | TformKind::I64 => {
            if requested == Algo::Gzip1 {
                Algo::Gzip1
            } else {
                Algo::Gzip2
            }
        }
        TformKind::I16 | TformKind::I32 | TformKind::Byte => requested,
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
            unreachable!("compression is selected from the VLA element type")
        }
    }
}

/// Compress a `BINTABLE` into a `ZTABLE` container. `rows_per_tile`
/// is the tile height (clamped to `[1, nrows]`); `default_algo` applies to every
/// column. Returns the compressed header and its data unit (Q descriptors + heap).
pub(crate) fn compress_table(
    header: &Header,
    table: TableView<'_>,
    rows_per_tile: usize,
    compression: Compression,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let (default_algo, gzip_level) = match compression {
        Compression::Gzip(config) if config.shuffle => (Algo::Gzip2, config.level),
        Compression::Gzip(config) => (Algo::Gzip1, config.level),
        Compression::Rice => (Algo::Rice1, gzip::DEFAULT_GZIP_LEVEL),
        Compression::None => (Algo::NoCompress, gzip::DEFAULT_GZIP_LEVEL),
        other => {
            return Err(FitsError::UnsupportedCompression {
                name: format!("{} for compressed tables", other.name()),
            });
        }
    };
    let bound = bind_table(header, table)?;
    reject_compression_metadata(header)?;
    let ncols = table.columns.len();
    let nrows = table.nrows;
    let naxis1 = table.row_len;
    let raw = bound.rows;

    let metas: Vec<ColMeta> = table
        .columns
        .iter()
        .map(|c| ColMeta::chosen(&c.tform, c.byte_offset, default_algo))
        .collect();
    let vla_columns = metas
        .iter()
        .enumerate()
        .map(|(index, meta)| {
            meta.is_stored_vla()
                .then(|| table.column_by_idx(index)?.vla_column())
                .transpose()
        })
        .collect::<Result<Vec<_>>>()?;

    let rpt = rows_per_tile.clamp(1, nrows.max(1));
    let nchunks = nrows.div_ceil(rpt);
    let compressed_row_len = ncols.checked_mul(16).ok_or(FitsError::DataUnitOverflow)?;
    let tile_count = nchunks
        .checked_mul(ncols)
        .ok_or(FitsError::DataUnitOverflow)?;

    // Compress each (chunk, column) tile independently — the compute-bound step,
    // parallel under the `parallel` feature, indexed `chunk * ncols + ci` so the
    // results land in the same flat order the descriptor rows expect. The reused
    // per-worker buffer holds the column's transposed bytes.
    let comps = map_tiles(
        tile_count,
        TableEncodeScratch::default,
        |scratch, i| -> Result<EncodedColumn> {
            let chunk = i / ncols;
            let column = i % ncols;
            let m = &metas[column];
            let r0 = chunk * rpt;
            let rows = rpt.min(nrows - r0);
            if let Some(vla) = vla_columns[column] {
                let mut descriptors = Vec::with_capacity(
                    rows.checked_mul(m.width)
                        .ok_or(FitsError::DataUnitOverflow)?,
                );
                let mut values = Vec::new();
                let mut ends = Vec::with_capacity(rows);
                for row in 0..rows {
                    let off = (r0 + row) * naxis1 + m.offset;
                    descriptors.extend_from_slice(&raw[off..off + m.width]);
                    // An array the codec does not shrink is stored as is, as cfitsio — the
                    // reference implementation of §10.3 — stores it; a stored length equal
                    // to the raw length is then what marks it raw, and cannot be a stream.
                    let cell = vla.cell(r0 + row)?;
                    let start = values.len();
                    compress_payload_into(m, cell.bytes, gzip_level, scratch, &mut values);
                    if values.len() - start >= cell.bytes.len() {
                        values.truncate(start);
                        values.extend_from_slice(cell.bytes);
                    }
                    ends.push(values.len());
                }
                return Ok(EncodedColumn::Variable {
                    descriptors,
                    arrays: Ragged::<Vec<u8>>::new(values, ends),
                });
            }
            // Transpose: gather this column's bytes across the tile's rows.
            scratch.column.clear();
            let cell_len = rows
                .checked_mul(m.width)
                .ok_or(FitsError::DataUnitOverflow)?;
            scratch.column.reserve_exact(cell_len);
            for r in 0..rows {
                let off = (r0 + r) * naxis1 + m.offset;
                scratch.column.extend_from_slice(&raw[off..off + m.width]);
            }
            let column_bytes = mem::take(&mut scratch.column);
            let mut compressed = Vec::new();
            compress_payload_into(m, &column_bytes, gzip_level, scratch, &mut compressed);
            scratch.column = column_bytes;
            Ok(EncodedColumn::Fixed(compressed))
        },
    )?;

    out.clear();
    let descriptor_bytes = tile_count
        .checked_mul(16)
        .ok_or(FitsError::DataUnitOverflow)?;
    out.reserve_exact(descriptor_bytes);
    out.resize(descriptor_bytes, 0);
    for (tile, comp) in comps.into_iter().enumerate() {
        let mut cell = match comp {
            EncodedColumn::Fixed(bytes) => bytes,
            EncodedColumn::Variable {
                descriptors,
                arrays,
            } => {
                let compressed_len = arrays
                    .len()
                    .checked_mul(16)
                    .ok_or(FitsError::DataUnitOverflow)?;
                let mut combined = allocation::try_zeroed(0u8, compressed_len)?;
                for (row, array) in arrays.rows().enumerate() {
                    if array.is_empty() {
                        continue;
                    }
                    let offset = out.len() - descriptor_bytes;
                    write_pq_descriptor(
                        &mut combined[row * 16..(row + 1) * 16],
                        true,
                        array.len() as u64,
                        offset as u64,
                    )?;
                    out.extend_from_slice(array);
                }
                combined.extend_from_slice(&descriptors);
                gzip::gzip_encode(&combined, gzip_level)
            }
        };
        let offset = out.len() - descriptor_bytes;
        write_pq_descriptor(
            &mut out[tile * 16..tile * 16 + 16],
            true,
            cell.len() as u64,
            offset as u64,
        )?;
        out.append(&mut cell);
    }
    let heap_len = out.len() - descriptor_bytes;

    // Header: copy the original, then layer on the Z* keywords.
    let mut h = header.clone();
    h.rename_keywords(&reserved_keywords::TABLE_PRESERVED);
    h.set_internal("ZTABLE", true)
        .comment_internal("ZTABLE", "this is a compressed table");
    h.set_internal("ZTILELEN", value::fits_i64(rpt)?);
    h.set_internal("ZNAXIS1", value::fits_i64(naxis1)?);
    h.set_internal("ZNAXIS2", value::fits_i64(nrows)?);
    h.set_internal("ZPCOUNT", value::fits_i64(bound.pcount)?);
    for (ci, m) in metas.iter().enumerate() {
        let n = ci + 1;
        let zform = header
            .get_text(key!("TFORM{n}").as_str())?
            .unwrap_or("")
            .to_string();
        h.set_internal(key!("ZFORM{n}").as_str(), zform);
        h.set_internal(key!("TFORM{n}").as_str(), "1QB");
        h.set_internal(key!("ZCTYP{n}").as_str(), m.algo.name());
    }
    h.set_internal("NAXIS1", value::fits_i64(compressed_row_len)?);
    h.set_internal("NAXIS2", value::fits_i64(nchunks)?);
    h.set_internal("PCOUNT", value::fits_i64(heap_len)?);
    h.set_internal("GCOUNT", 1);
    Ok(h)
}

/// A restored header and its decompressed data unit.
#[derive(Debug)]
pub(crate) struct HduParts {
    pub(crate) header: Header,
    pub(crate) data: Vec<u8>,
}

/// Uncompress a `ZTABLE` container back into its original `BINTABLE`.
/// Returns the restored header and row-major data unit.
#[expect(
    clippy::map_err_ignore,
    reason = "a `TryFromIntError` says only that the value does not fit, which the error it becomes states"
)]
pub(crate) fn uncompress_table(header: &Header, table: TableView<'_>) -> Result<HduParts> {
    if header.get_logical("ZTABLE")? != Some(true) {
        return Err(FitsError::NotCompressedTable);
    }
    bind_table(header, table)?;
    let naxis1 = header.required_usize("ZNAXIS1", "ZNAXIS1")?;
    let nrows = header.required_usize("ZNAXIS2", "ZNAXIS2")?;
    let zpcount = header.required_usize("ZPCOUNT", "ZPCOUNT")?;
    let original_bytes = nrows
        .checked_mul(naxis1)
        .ok_or(FitsError::DataUnitOverflow)?;
    let original_heap_offset = match header.get_integer("ZTHEAP")? {
        Some(ztheap) => {
            usize::try_from(ztheap).map_err(|_| FitsError::KeywordOutOfRange { name: "ZTHEAP" })?
        }
        None => original_bytes,
    };
    let total = original_bytes
        .checked_add(zpcount)
        .ok_or(FitsError::DataUnitOverflow)?;
    if !(original_bytes..=total).contains(&original_heap_offset) {
        return Err(FitsError::TableMetadataMismatch {
            name: "ZTHEAP".to_string(),
        });
    }
    header.get_text("ZHECKSUM")?;
    header.get_text("ZDATASUM")?;
    let mut rpt = header.required_usize("ZTILELEN", "ZTILELEN")?;
    // A zero tile height would make the row-tile count diverge; §10.3 requires ≥ 1.
    if rpt == 0 {
        return Err(FitsError::KeywordOutOfRange { name: "ZTILELEN" });
    }
    if rpt > nrows {
        rpt = nrows.max(1);
    }
    let ncols = header.required_usize("TFIELDS", "TFIELDS")?;
    validate_table_field_count(ncols)?;

    // Resolve each column's original form and codec.
    let mut metas = Vec::with_capacity(ncols);
    let mut zforms = Vec::with_capacity(ncols);
    let mut offset = 0;
    for n in 1..=ncols {
        let zform = header
            .get_text(key!("ZFORM{n}").as_str())?
            .ok_or(FitsError::MissingKeyword { name: "ZFORMn" })?
            .to_string();
        let tform = Tform::parse(&zform)?;
        let algo = match header.get_text(key!("ZCTYP{n}").as_str())? {
            Some(s) => Algo::parse(s)?,
            None => Algo::Gzip2, // cfitsio's default when ZCTYPn is absent
        };
        let m = ColMeta::declared(&tform, offset, algo)?;
        offset = offset
            .checked_add(m.width)
            .ok_or(FitsError::DataUnitOverflow)?;
        zforms.push(zform);
        metas.push(m);
    }
    if offset != naxis1 {
        return Err(FitsError::RowWidthMismatch {
            computed: offset,
            declared: naxis1,
        });
    }

    let nchunks = nrows.div_ceil(rpt);
    let tile_count = nchunks
        .checked_mul(ncols)
        .ok_or(FitsError::DataUnitOverflow)?;
    if table.nrows != nchunks {
        return Err(FitsError::DataSizeMismatch {
            expected: nchunks,
            got: table.nrows,
        });
    }
    let cells: Vec<_> = (0..ncols)
        .map(|ci| table.column_by_idx(ci)?.vla_column())
        .collect::<Result<_>>()?;

    let mut out = allocation::try_zeroed(0u8, total)?;
    let has_vla = metas.iter().any(ColMeta::is_stored_vla);
    if has_vla {
        let (main, heap) = out.split_at_mut(original_bytes);
        let heap_gap = original_heap_offset - original_bytes;
        let mut scratch = TableDecodeScratch::default();
        for chunk in 0..nchunks {
            let rows = rpt.min(nrows - chunk * rpt);
            let chunk_start = chunk
                .checked_mul(rpt)
                .and_then(|row| row.checked_mul(naxis1))
                .ok_or(FitsError::DataUnitOverflow)?;
            let chunk_len = rows
                .checked_mul(naxis1)
                .ok_or(FitsError::DataUnitOverflow)?;
            let mut restore = RestoreChunk {
                main: &mut main[chunk_start..chunk_start + chunk_len],
                heap: &mut *heap,
                rows,
                row_len: naxis1,
                heap_gap,
            };
            for column in 0..ncols {
                let m = &metas[column];
                let cell = cells[column].cell(chunk)?;
                let bytes = convert::byte_cell(cell)?;
                if m.is_stored_vla() {
                    restore.decompress_vla_column(table, bytes, m, &mut scratch)?;
                } else {
                    decompress_column_into(bytes, m, rows, &mut scratch)?;
                    scatter_column(restore.main, &scratch.bytes, rows, naxis1, m);
                }
            }
        }
    } else if tile_count != 0 {
        let chunk_len = rpt.checked_mul(naxis1).ok_or(FitsError::DataUnitOverflow)?;
        let main = &mut out[..original_bytes];
        let decode_chunk =
            |scratch: &mut TableDecodeScratch, chunk: usize, out: &mut [u8]| -> Result<()> {
                let rows = rpt.min(nrows - chunk * rpt);
                for column in 0..ncols {
                    let m = &metas[column];
                    let cell = cells[column].cell(chunk)?;
                    decompress_column_into(convert::byte_cell(cell)?, m, rows, scratch)?;
                    scatter_column(out, &scratch.bytes, rows, naxis1, m);
                }
                Ok(())
            };
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;

            main.par_chunks_mut(chunk_len)
                .enumerate()
                .try_for_each_init(TableDecodeScratch::default, |scratch, (chunk, out)| {
                    decode_chunk(scratch, chunk, out)
                })?;
        }
        #[cfg(not(feature = "parallel"))]
        {
            let mut scratch = TableDecodeScratch::default();
            for (chunk, out) in main.chunks_mut(chunk_len).enumerate() {
                decode_chunk(&mut scratch, chunk, out)?;
            }
        }
    }

    // Restore the original header: drop the Z* keywords, reinstate NAXIS/PCOUNT.
    let mut h = header.clone();
    h.set_internal("NAXIS1", value::fits_i64(naxis1)?);
    h.set_internal("NAXIS2", value::fits_i64(nrows)?);
    h.set_internal("PCOUNT", value::fits_i64(zpcount)?);
    for (n, zform) in zforms.iter().enumerate() {
        h.set_internal(key!("TFORM{}", n + 1).as_str(), zform.clone());
    }
    // The container's own heap pointer and checksums give way to the table's.
    h.remove_where(|keyword| {
        reserved_keywords::is_table_compression(keyword, ncols)
            || reserved_keywords::TABLE_PRESERVED
                .iter()
                .any(|&(table, _)| table == keyword)
    });
    h.rename_keywords(
        &reserved_keywords::TABLE_PRESERVED.map(|(table, container)| (container, table)),
    );
    Ok(HduParts {
        header: h,
        data: out,
    })
}

#[expect(
    clippy::map_err_ignore,
    reason = "a `TryFromIntError` says only that the value does not fit, which the error it becomes states"
)]
fn bind_table<'a>(header: &Header, table: TableView<'a>) -> Result<BoundTable<'a>> {
    let xtension = header
        .get_text("XTENSION")?
        .ok_or(FitsError::MissingKeyword { name: "XTENSION" })?;
    if xtension != "BINTABLE" {
        return Err(metadata_mismatch("XTENSION"));
    }
    for (keyword, expected) in [
        ("BITPIX", 8usize),
        ("NAXIS", 2),
        ("NAXIS1", table.row_len),
        ("NAXIS2", table.nrows),
        ("GCOUNT", 1),
        ("TFIELDS", table.columns.len()),
    ] {
        if header.required_usize(keyword, keyword)? != expected {
            return Err(metadata_mismatch(keyword));
        }
    }
    validate_table_field_count(table.columns.len())?;

    let mut row_width = 0usize;
    for (index, column) in table.columns.iter().enumerate() {
        let n = index + 1;
        if column.byte_offset != row_width {
            return Err(metadata_mismatch(format!("column {n} byte offset")));
        }
        row_width = row_width
            .checked_add(column.tform.byte_width())
            .ok_or(FitsError::DataUnitOverflow)?;
        let keyword = key!("TFORM{n}");
        let tform = header
            .get_text(keyword.as_str())?
            .ok_or(FitsError::MissingKeyword { name: "TFORMn" })?;
        if Tform::parse(tform)? != column.tform {
            return Err(metadata_mismatch(keyword.as_str()));
        }
    }
    if row_width != table.row_len {
        return Err(FitsError::RowWidthMismatch {
            computed: row_width,
            declared: table.row_len,
        });
    }

    let rows = table.raw_rows();
    if table.heap_offset < rows.len()
        || table.heap_end < table.heap_offset
        || table.heap_end < rows.len()
    {
        return Err(metadata_mismatch("THEAP"));
    }
    let pcount = table.heap_end - rows.len();
    if header.required_usize("PCOUNT", "PCOUNT")? != pcount {
        return Err(metadata_mismatch("PCOUNT"));
    }
    let theap = match header.get_integer("THEAP")? {
        Some(value) => {
            usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { name: "THEAP" })?
        }
        None => rows.len(),
    };
    if theap != table.heap_offset {
        return Err(metadata_mismatch("THEAP"));
    }
    Ok(BoundTable { rows, pcount })
}

fn reject_compression_metadata(header: &Header) -> Result<()> {
    for entry in header.iter() {
        if reserved_keywords::is_table_compression(entry.keyword, 999)
            || reserved_keywords::TABLE_PRESERVED
                .iter()
                .any(|&(_, container)| container == entry.keyword)
        {
            return Err(metadata_mismatch(entry.keyword));
        }
    }
    Ok(())
}

fn metadata_mismatch(name: impl Into<String>) -> FitsError {
    FitsError::TableMetadataMismatch { name: name.into() }
}

/// The buffers a worker reuses from one table tile to the next.
#[derive(Debug, Default)]
struct TableEncodeScratch {
    column: Vec<u8>,
    ints: Vec<i64>,
    gzip: gzip::GzipScratch,
    rice: rice::RiceScratch,
}

#[derive(Debug)]
enum EncodedColumn {
    Fixed(Vec<u8>),
    Variable {
        descriptors: Vec<u8>,
        arrays: Ragged<Vec<u8>>,
    },
}

/// Compress one tile's column-major raw bytes per the column's codec, appending
/// the stream to `out`.
fn compress_payload_into(
    m: &ColMeta,
    bytes: &[u8],
    gzip_level: u32,
    scratch: &mut TableEncodeScratch,
    out: &mut Vec<u8>,
) {
    match m.algo {
        Algo::Gzip1 => gzip::gzip_encode_into(bytes, gzip_level, out),
        Algo::Gzip2 => {
            gzip::gzip2_encode_into(bytes, m.shuffle_width(), gzip_level, &mut scratch.gzip, out);
        }
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix();
            debug_assert!(
                bytes.len().is_multiple_of(bytepix.elem_size()),
                "whole Rice pixels"
            );
            convert::be_to_i64_into(bytes, bytepix, &mut scratch.ints);
            rice::rice_encode_into(
                &scratch.ints,
                bytepix,
                rice::BLOCKSIZE,
                &mut scratch.rice,
                out,
            );
        }
        Algo::NoCompress => out.extend_from_slice(bytes),
    }
}

#[derive(Debug, Default)]
struct TableDecodeScratch {
    bytes: Vec<u8>,
    inflated: Vec<u8>,
    ints: Vec<i64>,
    descriptors: Vec<u8>,
    vla: Vec<u8>,
}

/// Which end of a tile's descriptor block holds which set. FITS 4.0 stores the
/// compressed descriptors first; cfitsio-written files reverse them, so a reader has
/// to work out which order it is looking at.
#[derive(Debug, Clone, Copy)]
struct VlaLayout {
    original_start: usize,
    compressed_start: usize,
}

#[derive(Debug)]
enum VlaLayoutError {
    OriginalDescriptor(FitsError),
    Rejected,
}

/// One row-tile of the table being restored: the slice of the main table its rows
/// occupy, the shared heap they point into, and the geometry both need.
#[derive(Debug)]
struct RestoreChunk<'a> {
    main: &'a mut [u8],
    heap: &'a mut [u8],
    rows: usize,
    row_len: usize,
    /// Bytes `THEAP` leaves between the end of the main table and the heap.
    heap_gap: usize,
}

impl RestoreChunk<'_> {
    /// Whether `layout` reads this tile's descriptor block consistently: every
    /// original descriptor must address the restored heap, and must have a compressed
    /// counterpart exactly when it is non-empty.
    #[expect(
        clippy::map_err_ignore,
        reason = "the error is deliberately replaced by the one this context defines"
    )]
    fn validate_vla_layout(
        &self,
        table: TableView<'_>,
        descriptors: &[u8],
        layout: VlaLayout,
        m: &ColMeta,
    ) -> result::Result<(), VlaLayoutError> {
        let wide = m.kind == TformKind::ArrayDesc64;
        let element_kind = m
            .vla_elem
            .expect("VLA column metadata carries its element kind");
        for row in 0..self.rows {
            let original_start = layout.original_start + row * m.width;
            let compressed_start = layout.compressed_start + row * 16;
            let original =
                PqDescriptor::decode(&descriptors[original_start..original_start + m.width], wide)
                    .map_err(VlaLayoutError::OriginalDescriptor)?;
            let compressed =
                PqDescriptor::decode(&descriptors[compressed_start..compressed_start + 16], true)
                    .map_err(|_| VlaLayoutError::Rejected)?;
            let original_range = original
                .heap_range(element_kind, self.heap_gap, self.heap.len())
                .map_err(|_| VlaLayoutError::Rejected)?;
            if original_range.is_empty() {
                if compressed.count != 0 {
                    return Err(VlaLayoutError::Rejected);
                }
                continue;
            }
            if compressed.count == 0
                || compressed
                    .heap_range(TformKind::Byte, table.heap_offset, table.heap_end)
                    .is_err()
            {
                return Err(VlaLayoutError::Rejected);
            }
        }
        Ok(())
    }

    /// Resolve which descriptor order this tile uses. A malformed *original*
    /// descriptor is a property of the data rather than of the order being guessed,
    /// so it is the more informative failure to surface when neither order fits.
    fn resolve_vla_layout(
        &self,
        table: TableView<'_>,
        descriptors: &[u8],
        candidates: [VlaLayout; 2],
        m: &ColMeta,
    ) -> Result<VlaLayout> {
        let mut malformed = None;
        for candidate in candidates {
            match self.validate_vla_layout(table, descriptors, candidate, m) {
                Ok(()) => return Ok(candidate),
                Err(VlaLayoutError::OriginalDescriptor(
                    error @ FitsError::InvalidPqDescriptor { .. },
                )) => {
                    malformed.get_or_insert(error);
                }
                Err(_) => {}
            }
        }
        Err(malformed.unwrap_or(FitsError::UnexpectedEof))
    }

    fn decompress_vla_column(
        &mut self,
        table: TableView<'_>,
        bytes: &[u8],
        m: &ColMeta,
        scratch: &mut TableDecodeScratch,
    ) -> Result<()> {
        let original_len = self
            .rows
            .checked_mul(m.width)
            .ok_or(FitsError::DataUnitOverflow)?;
        let combined_len = original_len
            .checked_add(
                self.rows
                    .checked_mul(16)
                    .ok_or(FitsError::DataUnitOverflow)?,
            )
            .ok_or(FitsError::DataUnitOverflow)?;
        gzip::gunzip_into(bytes, combined_len, &mut scratch.descriptors)?;
        if scratch.descriptors.len() != combined_len {
            return Err(FitsError::DataSizeMismatch {
                expected: combined_len,
                got: scratch.descriptors.len(),
            });
        }

        let standard = VlaLayout {
            original_start: self
                .rows
                .checked_mul(16)
                .ok_or(FitsError::DataUnitOverflow)?,
            compressed_start: 0,
        };
        let cfitsio = VlaLayout {
            original_start: 0,
            compressed_start: original_len,
        };
        let layout =
            self.resolve_vla_layout(table, &scratch.descriptors, [standard, cfitsio], m)?;

        for row in 0..self.rows {
            let start = layout.original_start + row * m.width;
            let descriptor = &scratch.descriptors[start..start + m.width];
            let offset = row * self.row_len + m.offset;
            self.main[offset..offset + m.width].copy_from_slice(descriptor);
        }

        let wide = m.kind == TformKind::ArrayDesc64;
        let element_kind = m
            .vla_elem
            .expect("VLA column metadata carries its element kind");
        for row in 0..self.rows {
            let original_start = layout.original_start + row * m.width;
            let original = PqDescriptor::decode(
                &scratch.descriptors[original_start..original_start + m.width],
                wide,
            )?;
            let compressed_start = layout.compressed_start + row * 16;
            let compressed = PqDescriptor::decode(
                &scratch.descriptors[compressed_start..compressed_start + 16],
                true,
            )?;
            let original_range =
                original.heap_range(element_kind, self.heap_gap, self.heap.len())?;
            let expected = original_range.len();
            if expected == 0 {
                if compressed.count != 0 {
                    return Err(FitsError::DataSizeMismatch {
                        expected: 0,
                        got: compressed.count,
                    });
                }
                continue;
            }
            let stream = table.pq_payload(compressed, TformKind::Byte)?;
            decompress_vla_payload(stream, m, original.count, expected, scratch)?;
            self.heap[original_range].copy_from_slice(&scratch.vla);
        }
        Ok(())
    }
}

fn decompress_vla_payload(
    bytes: &[u8],
    m: &ColMeta,
    element_count: usize,
    expected: usize,
    scratch: &mut TableDecodeScratch,
) -> Result<()> {
    // cfitsio stores an array its codec does not shrink as is, and every stream it does
    // store is shorter than the array: a length equal to the raw length marks a raw array
    // (cfitsio `fits_uncompress_table`), §10.3.6 notwithstanding.
    if bytes.len() == expected || m.algo == Algo::NoCompress {
        if bytes.len() != expected {
            return Err(FitsError::DataSizeMismatch {
                expected,
                got: bytes.len(),
            });
        }
        scratch.vla.clear();
        scratch.vla.extend_from_slice(bytes);
        return Ok(());
    }

    match m.algo {
        Algo::Gzip1 => gzip::gunzip_into(bytes, expected, &mut scratch.vla)?,
        Algo::Gzip2 => gzip::gunzip2_into(
            bytes,
            expected,
            m.shuffle_width(),
            &mut scratch.inflated,
            &mut scratch.vla,
        )?,
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix();
            rice::rice_decode_into(
                bytes,
                element_count,
                bytepix,
                rice::BLOCKSIZE,
                &mut scratch.ints,
            )?;
            convert::i64_to_be_into(&scratch.ints, bytepix, &mut scratch.vla);
        }
        Algo::NoCompress => unreachable!("handled before decompression"),
    }
    if scratch.vla.len() != expected {
        return Err(FitsError::DataSizeMismatch {
            expected,
            got: scratch.vla.len(),
        });
    }
    Ok(())
}

fn decompress_column_into(
    bytes: &[u8],
    m: &ColMeta,
    rows: usize,
    scratch: &mut TableDecodeScratch,
) -> Result<()> {
    // The decompressed column is exactly this many bytes; bound the gzip inflate at it
    // so a crafted cell can't expand unbounded (`rows × width ≤ ZNAXIS2 × ZNAXIS1`,
    // already checked non-overflowing by the caller).
    let expect = rows
        .checked_mul(m.width)
        .ok_or(FitsError::DataUnitOverflow)?;
    match m.algo {
        Algo::Gzip1 => gzip::gunzip_into(bytes, expect, &mut scratch.bytes)?,
        Algo::Gzip2 => gzip::gunzip2_into(
            bytes,
            expect,
            m.shuffle_width(),
            &mut scratch.inflated,
            &mut scratch.bytes,
        )?,
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix();
            let count = expect / bytepix.elem_size();
            rice::rice_decode_into(bytes, count, bytepix, rice::BLOCKSIZE, &mut scratch.ints)?;
            convert::i64_to_be_into(&scratch.ints, bytepix, &mut scratch.bytes);
        }
        Algo::NoCompress => {
            scratch.bytes.clear();
            scratch.bytes.extend_from_slice(bytes);
        }
    }
    if scratch.bytes.len() != expect {
        return Err(FitsError::DataSizeMismatch {
            expected: expect,
            got: scratch.bytes.len(),
        });
    }
    Ok(())
}

fn scatter_column(out: &mut [u8], bytes: &[u8], rows: usize, row_len: usize, m: &ColMeta) {
    debug_assert_eq!(bytes.len(), rows * m.width, "decompressed column size");
    for row in 0..rows {
        let offset = row * row_len + m.offset;
        out[offset..offset + m.width].copy_from_slice(&bytes[row * m.width..(row + 1) * m.width]);
    }
}

/// The mixed-column table the table-compression tests and bench compress.
#[cfg(any(test, feature = "bench"))]
pub(crate) mod internals {
    use crate::bintable::column_data::ColumnData;
    use crate::writer::table::WriteColumn;

    /// i16, i32, f32, f64 and byte columns, and a repeat-3 i16 vector, over `nrows`
    /// rows of simple formulas with negative values among them.
    pub(crate) fn mixed_columns(nrows: usize) -> Vec<WriteColumn> {
        vec![
            WriteColumn::fixed(
                "SHORT",
                ColumnData::I16((0..nrows).map(|i| i as i16 * 7 - 30).collect()),
                1,
            ),
            WriteColumn::fixed(
                "INT",
                ColumnData::I32((0..nrows).map(|i| i as i32 * 100_000 - 5).collect()),
                1,
            ),
            WriteColumn::fixed(
                "FLT",
                ColumnData::F32((0..nrows).map(|i| i as f32 * 1.5 - 3.25).collect()),
                1,
            ),
            WriteColumn::fixed(
                "DBL",
                ColumnData::F64((0..nrows).map(|i| i as f64 * 0.1).collect()),
                1,
            ),
            WriteColumn::fixed(
                "BYTE",
                ColumnData::Bytes((0..nrows).map(|i| (i * 3) as u8).collect()),
                1,
            ),
            WriteColumn::fixed(
                "VEC",
                ColumnData::I16((0..nrows * 3).map(|i| (i * 2) as i16).collect()),
                3,
            ),
        ]
    }
}

#[cfg(test)]
mod tests;
