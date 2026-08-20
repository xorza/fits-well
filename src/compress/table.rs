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
use crate::compress::Compression;
use crate::compress::ImageCodec;
use crate::compress::convert;
use crate::compress::gzip;
use crate::compress::map_tiles;
use crate::compress::rice;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::header::value;
use crate::keyword;
use crate::keyword::key;
use crate::table_impl::BinTable;
use crate::table_impl::descriptor::PqDescriptor;
use crate::table_impl::tform::Tform;
use crate::table_impl::tform_kind::TformKind;

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

/// Resolved per-column layout used by both directions.
#[derive(Debug)]
struct ColMeta {
    kind: TformKind,
    vla_elem: Option<TformKind>,
    /// Element width in bytes (the `t` size, e.g. 2 for `I`).
    elem_size: usize,
    /// Number of elements per row (`repeat`).
    repeat: usize,
    /// Bytes per row for this column (`repeat × elem_size`).
    width: usize,
    /// Byte offset of this column within a row.
    offset: usize,
    algo: Algo,
    gzip_level: u32,
}

#[derive(Debug)]
struct BoundTable<'a> {
    rows: &'a [u8],
    pcount: usize,
}

impl ColMeta {
    fn is_vla(&self) -> bool {
        self.vla_elem.is_some()
    }

    fn is_stored_vla(&self) -> bool {
        self.is_vla() && self.width != 0
    }

    fn compression_kind(&self) -> TformKind {
        self.vla_elem.unwrap_or(self.kind)
    }

    /// GZIP_2 byte-shuffle width: the element size for the multi-byte numeric
    /// types cfitsio shuffles (`I`/`J`/`E`/`K`/`D`), else 1 (no shuffle).
    fn shuffle_width(&self) -> usize {
        match self.compression_kind() {
            TformKind::I16 | TformKind::I32 | TformKind::F32 | TformKind::I64 | TformKind::F64 => {
                self.elem_size
            }
            _ => 1,
        }
    }

    /// `RICE_1` pixel width (`B`=1, `I`=2, `J`=4); other types can't use Rice.
    fn rice_bytepix(&self) -> Option<usize> {
        match self.compression_kind() {
            TformKind::Byte => Some(1),
            TformKind::I16 => Some(2),
            TformKind::I32 => Some(4),
            TformKind::I64 => Some(8),
            _ => None,
        }
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

/// Build per-column metadata from a column's `Tform`, its byte offset, and the
/// chosen algorithm.
fn col_meta(tform: &Tform, offset: usize, algo: Algo, gzip_level: u32) -> Result<ColMeta> {
    let compression_kind = tform.vla_elem.unwrap_or(tform.kind);
    let elem_size = compression_kind.elem_size();
    // Bit columns pack `repeat` bits into bytes; the in-row width is the byte_width.
    let width = tform.byte_width();
    let repeat = if width == 0 { 0 } else { width / elem_size };
    Ok(ColMeta {
        kind: tform.kind,
        vla_elem: tform.vla_elem,
        elem_size,
        repeat,
        width,
        offset,
        algo: pick_algo(compression_kind, algo),
        gzip_level,
    })
}

/// Compress a `BINTABLE` into a `ZTABLE` container. `rows_per_tile`
/// is the tile height (clamped to `[1, nrows]`); `default_algo` applies to every
/// column. Returns the compressed header and its data unit (Q descriptors + heap).
pub(crate) fn compress_table(
    header: &Header,
    table: &BinTable,
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
    let metadata = table.metadata();
    let ncols = metadata.columns.len();
    let nrows = metadata.nrows;
    let naxis1 = table.schema.row_len;
    let raw = bound.rows;

    let metas: Vec<ColMeta> = metadata
        .columns
        .iter()
        .map(|c| col_meta(&c.tform, c.byte_offset, default_algo, gzip_level))
        .collect::<Result<_>>()?;
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
                let mut arrays = Vec::with_capacity(rows);
                for row in 0..rows {
                    let off = (r0 + row) * naxis1 + m.offset;
                    descriptors.extend_from_slice(&raw[off..off + m.width]);
                    let cell = vla.cell(r0 + row)?;
                    let compressed = compress_payload(m, cell.bytes, cell.element_count, scratch)?;
                    arrays.push(if compressed.len() < cell.bytes.len() {
                        compressed
                    } else {
                        cell.bytes.to_vec()
                    });
                }
                return Ok(EncodedColumn::Variable {
                    descriptors,
                    arrays,
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
            let column_bytes = std::mem::take(&mut scratch.column);
            let compressed = compress_payload(
                m,
                &column_bytes,
                rows.checked_mul(m.repeat)
                    .ok_or(FitsError::DataUnitOverflow)?,
                scratch,
            );
            scratch.column = column_bytes;
            Ok(EncodedColumn::Fixed(compressed?))
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
                for (row, mut array) in arrays.into_iter().enumerate() {
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
                    out.append(&mut array);
                }
                combined.extend_from_slice(&descriptors);
                gzip::gzip_encode(&combined, gzip::DEFAULT_GZIP_LEVEL)
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
    h.rename_keywords(&[
        ("THEAP", "ZTHEAP"),
        ("CHECKSUM", "ZHECKSUM"),
        ("DATASUM", "ZDATASUM"),
    ]);
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
pub(crate) fn uncompress_table(header: &Header, table: &BinTable) -> Result<HduParts> {
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
        let m = col_meta(&tform, offset, algo, gzip::DEFAULT_GZIP_LEVEL)?;
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
    let metadata = table.metadata();
    if metadata.nrows != nchunks {
        return Err(FitsError::DataSizeMismatch {
            expected: nchunks,
            got: metadata.nrows,
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
    h.remove_where(|keyword| {
        matches!(
            keyword,
            "ZTABLE"
                | "ZTILELEN"
                | "ZNAXIS1"
                | "ZNAXIS2"
                | "ZPCOUNT"
                | "THEAP"
                | "CHECKSUM"
                | "DATASUM"
        ) || indexed_compression_key(keyword, "ZFORM", ncols)
            || indexed_compression_key(keyword, "ZCTYP", ncols)
    });
    h.rename_keywords(&[
        ("ZTHEAP", "THEAP"),
        ("ZHECKSUM", "CHECKSUM"),
        ("ZDATASUM", "DATASUM"),
    ]);
    Ok(HduParts {
        header: h,
        data: out,
    })
}

fn indexed_compression_key(keyword: &str, prefix: &str, ncols: usize) -> bool {
    keyword::index(keyword, prefix).is_some_and(|column| (1..=ncols).contains(&column))
}

fn bind_table<'a>(header: &Header, table: &'a BinTable) -> Result<BoundTable<'a>> {
    let metadata = table.metadata();
    let xtension = header
        .get_text("XTENSION")?
        .ok_or(FitsError::MissingKeyword { name: "XTENSION" })?;
    if xtension != "BINTABLE" {
        return Err(metadata_mismatch("XTENSION"));
    }
    for (keyword, expected) in [
        ("BITPIX", 8usize),
        ("NAXIS", 2),
        ("NAXIS1", table.schema.row_len),
        ("NAXIS2", metadata.nrows),
        ("GCOUNT", 1),
        ("TFIELDS", metadata.columns.len()),
    ] {
        if header.required_usize(keyword, keyword)? != expected {
            return Err(metadata_mismatch(keyword));
        }
    }
    validate_table_field_count(metadata.columns.len())?;

    let mut row_width = 0usize;
    for (index, column) in metadata.columns.iter().enumerate() {
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
    if row_width != table.schema.row_len {
        return Err(FitsError::RowWidthMismatch {
            computed: row_width,
            declared: table.schema.row_len,
        });
    }

    let rows = table.raw_rows()?;
    if table.schema.heap_offset < rows.len()
        || table.schema.heap_end < table.schema.heap_offset
        || table.schema.heap_end < rows.len()
    {
        return Err(metadata_mismatch("THEAP"));
    }
    let pcount = table.schema.heap_end - rows.len();
    if header.required_usize("PCOUNT", "PCOUNT")? != pcount {
        return Err(metadata_mismatch("PCOUNT"));
    }
    let theap = match header.get_integer("THEAP")? {
        Some(value) => {
            usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { name: "THEAP" })?
        }
        None => rows.len(),
    };
    if theap != table.schema.heap_offset {
        return Err(metadata_mismatch("THEAP"));
    }
    Ok(BoundTable { rows, pcount })
}

fn reject_compression_metadata(header: &Header) -> Result<()> {
    for entry in header.iter() {
        if matches!(
            entry.keyword,
            "ZTABLE"
                | "ZTILELEN"
                | "ZNAXIS1"
                | "ZNAXIS2"
                | "ZPCOUNT"
                | "ZTHEAP"
                | "ZHECKSUM"
                | "ZDATASUM"
        ) || indexed_compression_key(entry.keyword, "ZFORM", 999)
            || indexed_compression_key(entry.keyword, "ZCTYP", 999)
        {
            return Err(metadata_mismatch(entry.keyword));
        }
    }
    Ok(())
}

fn metadata_mismatch(name: impl Into<String>) -> FitsError {
    FitsError::TableMetadataMismatch { name: name.into() }
}

/// Compress one tile's column-major raw bytes per the column's algorithm.
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
        arrays: Vec<Vec<u8>>,
    },
}

fn compress_payload(
    m: &ColMeta,
    bytes: &[u8],
    element_count: usize,
    scratch: &mut TableEncodeScratch,
) -> Result<Vec<u8>> {
    Ok(match m.algo {
        Algo::Gzip1 => gzip::gzip_encode(bytes, m.gzip_level),
        Algo::Gzip2 => {
            gzip::gzip2_encode(bytes, m.shuffle_width(), m.gzip_level, &mut scratch.gzip)
        }
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix().ok_or(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", m.compression_kind().code()),
            })?;
            convert::be_to_i64_into(
                bytes,
                convert::bytepix_to_bitpix(bytepix),
                &mut scratch.ints,
            );
            if scratch.ints.len() != element_count {
                return Err(FitsError::DataSizeMismatch {
                    expected: element_count,
                    got: scratch.ints.len(),
                });
            }
            rice::rice_encode(&scratch.ints, bytepix, 32, &mut scratch.rice)
        }
        Algo::NoCompress => bytes.to_vec(),
    })
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
    fn validate_vla_layout(
        &self,
        table: &BinTable,
        descriptors: &[u8],
        layout: VlaLayout,
        m: &ColMeta,
    ) -> std::result::Result<(), VlaLayoutError> {
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
                    .heap_range(
                        TformKind::Byte,
                        table.schema.heap_offset,
                        table.schema.heap_end,
                    )
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
        table: &BinTable,
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
        table: &BinTable,
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
            let bytepix = m.rice_bytepix().ok_or(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", m.compression_kind().code()),
            })?;
            rice::rice_decode_into(bytes, element_count, bytepix, 32, &mut scratch.ints)?;
            convert::i64_to_be_into(
                &scratch.ints,
                convert::bytepix_to_bitpix(bytepix),
                &mut scratch.vla,
            );
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
            let bytepix = m.rice_bytepix().ok_or(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", m.kind.code()),
            })?;
            let nelem = rows
                .checked_mul(m.repeat)
                .ok_or(FitsError::DataUnitOverflow)?;
            rice::rice_decode_into(bytes, nelem, bytepix, 32, &mut scratch.ints)?;
            convert::i64_to_be_into(
                &scratch.ints,
                convert::bytepix_to_bitpix(bytepix),
                &mut scratch.bytes,
            );
        }
        Algo::NoCompress => {
            scratch.bytes.clear();
            scratch.bytes.extend_from_slice(bytes);
        }
    }
    if scratch.bytes.len() != expect {
        return Err(FitsError::UnsupportedCompression {
            name: "decompressed column size mismatch".to_string(),
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
