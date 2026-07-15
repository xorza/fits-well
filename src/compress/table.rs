//! Tiled table compression (§10.3) — a port of cfitsio's `fits_compress_table`/
//! `fits_uncompress_table` for fixed-width `BINTABLE` columns.
//!
//! The table is split into row-tiles of `ZTILELEN` rows. Within a tile each
//! column is transposed to column-major order and compressed independently with
//! its `ZCTYPn` codec (`GZIP_1`/`GZIP_2`/`RICE_1`). The compressed table is itself
//! a `BINTABLE` with `ZTABLE = T`: one row per tile, one `1QB` variable-length
//! byte column per original column, the compressed bytes living in the heap. The
//! original `TFORMn`/`NAXIS1`/`NAXIS2`/`PCOUNT` are preserved as
//! `ZFORMn`/`ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT`.
//!
//! Variable-length (`P`/`Q`) source columns are not supported and are rejected.

use super::DisjointSlice;
use super::HduParts;
use super::convert;
use super::gzip;
use super::map_tiles;
use super::rice;
use super::try_for_each_tile;
use crate::allocation;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table::BinTable;
use crate::table::Tform;
use crate::table::TformKind;

/// Per-column compression algorithm (`ZCTYPn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Algo {
    Gzip1,
    Gzip2,
    Rice1,
}

impl Algo {
    fn name(self) -> &'static str {
        match self {
            Algo::Gzip1 => "GZIP_1",
            Algo::Gzip2 => "GZIP_2",
            Algo::Rice1 => "RICE_1",
        }
    }

    fn parse(s: &str) -> Result<Algo> {
        match s {
            "GZIP_1" => Ok(Algo::Gzip1),
            "GZIP_2" => Ok(Algo::Gzip2),
            "RICE_1" => Ok(Algo::Rice1),
            other => Err(FitsError::UnsupportedCompression {
                name: format!("table column codec {other}"),
            }),
        }
    }
}

/// Resolved per-column layout used by both directions.
#[derive(Debug)]
struct ColMeta {
    kind: TformKind,
    /// Element width in bytes (the `t` size, e.g. 2 for `I`).
    elem_size: usize,
    /// Number of elements per row (`repeat`).
    repeat: usize,
    /// Bytes per row for this column (`repeat × elem_size`).
    width: usize,
    /// Byte offset of this column within a row.
    offset: usize,
    algo: Algo,
}

impl ColMeta {
    /// GZIP_2 byte-shuffle width: the element size for the multi-byte numeric
    /// types cfitsio shuffles (`I`/`J`/`E`/`K`/`D`), else 1 (no shuffle).
    fn shuffle_width(&self) -> usize {
        match self.kind {
            TformKind::I16 | TformKind::I32 | TformKind::F32 | TformKind::I64 | TformKind::F64 => {
                self.elem_size
            }
            _ => 1,
        }
    }

    /// `RICE_1` pixel width (`B`=1, `I`=2, `J`=4); other types can't use Rice.
    fn rice_bytepix(&self) -> Option<usize> {
        match self.kind {
            TformKind::Byte => Some(1),
            TformKind::I16 => Some(2),
            TformKind::I32 => Some(4),
            _ => None,
        }
    }
}

/// Clamp a requested algorithm to one valid for the column type, mirroring
/// cfitsio's per-type sanity overrides.
fn pick_algo(kind: TformKind, requested: Algo) -> Algo {
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
        // `col_meta` rejects array-descriptor columns before `pick_algo` is reached.
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
            unreachable!("variable-length columns are rejected before algo selection")
        }
    }
}

/// Build per-column metadata from a column's `Tform`, its byte offset, and the
/// chosen algorithm. Rejects variable-length columns.
fn col_meta(tform: &Tform, offset: usize, algo: Algo) -> Result<ColMeta> {
    if matches!(tform.kind, TformKind::ArrayDesc32 | TformKind::ArrayDesc64) {
        return Err(FitsError::UnsupportedCompression {
            name: "variable-length column in a compressed table".to_string(),
        });
    }
    let elem_size = tform.kind.elem_size();
    // Bit columns pack `repeat` bits into bytes; the in-row width is the byte_width.
    let width = tform.byte_width();
    let repeat = if width == 0 { 0 } else { width / elem_size };
    Ok(ColMeta {
        kind: tform.kind,
        elem_size,
        repeat,
        width,
        offset,
        algo: pick_algo(tform.kind, algo),
    })
}

/// Compress a fixed-width `BINTABLE` into a `ZTABLE` container. `rows_per_tile`
/// is the tile height (clamped to `[1, nrows]`); `default_algo` applies to every
/// column. Returns the compressed header and its data unit (Q descriptors + heap).
pub(crate) fn compress_table(
    header: &Header,
    table: &BinTable,
    rows_per_tile: usize,
    default_algo: &str,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let default_algo = Algo::parse(default_algo)?;
    let ncols = table.columns.len();
    let nrows = table.nrows;
    let naxis1 = table.row_len;
    let raw = table.raw_rows();

    let metas: Vec<ColMeta> = table
        .columns
        .iter()
        .map(|c| col_meta(&c.tform, c.byte_offset, default_algo))
        .collect::<Result<_>>()?;

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
    let comps = map_tiles(tile_count, Vec::<u8>::new, |cm, i| -> Result<Vec<u8>> {
        let chunk = i / ncols;
        let m = &metas[i % ncols];
        let r0 = chunk * rpt;
        let rows = rpt.min(nrows - r0);
        // Transpose: gather this column's bytes across the tile's rows.
        cm.clear();
        let cell_len = rows
            .checked_mul(m.width)
            .ok_or(FitsError::DataUnitOverflow)?;
        allocation::try_reserve_exact(cm, cell_len)?;
        for r in 0..rows {
            let off = (r0 + r) * naxis1 + m.offset;
            cm.extend_from_slice(&raw[off..off + m.width]);
        }
        compress_column(cm, m)
    })?;

    let heap_len = comps.iter().try_fold(0usize, |len, comp| {
        len.checked_add(comp.len())
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    out.clear();
    let descriptor_bytes = tile_count
        .checked_mul(16)
        .ok_or(FitsError::DataUnitOverflow)?;
    let output_len = descriptor_bytes
        .checked_add(heap_len)
        .ok_or(FitsError::DataUnitOverflow)?;
    allocation::try_reserve_exact(out, output_len)?;
    out.resize(descriptor_bytes, 0);
    for (tile, mut comp) in comps.into_iter().enumerate() {
        let offset = out.len() - descriptor_bytes;
        write_pq_descriptor(
            &mut out[tile * 16..tile * 16 + 16],
            true,
            comp.len() as u64,
            offset as u64,
        )?;
        out.append(&mut comp);
    }

    // Header: copy the original, then layer on the Z* keywords.
    let mut h = header.clone();
    let orig_pcount = header.get_integer("PCOUNT").unwrap_or(0);
    h.set("ZTABLE", true)
        .comment("ZTABLE", "this is a compressed table");
    h.set("ZTILELEN", fits_i64(rpt)?);
    h.set("ZNAXIS1", fits_i64(naxis1)?);
    h.set("ZNAXIS2", fits_i64(nrows)?);
    h.set("ZPCOUNT", orig_pcount);
    for (ci, m) in metas.iter().enumerate() {
        let n = ci + 1;
        let zform = header
            .get_text(key!("TFORM{n}").as_str())
            .unwrap_or("")
            .to_string();
        h.set(key!("ZFORM{n}").as_str(), zform);
        h.set(key!("TFORM{n}").as_str(), "1QB");
        h.set(key!("ZCTYP{n}").as_str(), m.algo.name());
    }
    h.set("NAXIS1", fits_i64(compressed_row_len)?);
    h.set("NAXIS2", fits_i64(nchunks)?);
    h.set("PCOUNT", fits_i64(heap_len)?);
    h.set("GCOUNT", 1);
    Ok(h)
}

/// Uncompress a `ZTABLE` container back into its original fixed-width `BINTABLE`.
/// Returns the restored header and row-major data unit.
pub(crate) fn uncompress_table(header: &Header, table: &BinTable) -> Result<HduParts> {
    if header.get_logical("ZTABLE") != Some(true) {
        return Err(FitsError::NotCompressedTable);
    }
    let naxis1 = req_usize(header, "ZNAXIS1")?;
    let nrows = req_usize(header, "ZNAXIS2")?;
    let zpcount = optional_nonnegative(header, "ZPCOUNT")?;
    let mut rpt = req_positive_usize(header, "ZTILELEN")?;
    if rpt > nrows {
        rpt = nrows.max(1);
    }
    let ncols = req_usize(header, "TFIELDS")?;
    if ncols > 999 {
        return Err(FitsError::KeywordOutOfRange { name: "TFIELDS" });
    }

    // Resolve each column's original form and codec.
    let mut metas = Vec::with_capacity(ncols);
    let mut zforms = Vec::with_capacity(ncols);
    let mut offset = 0;
    for n in 1..=ncols {
        let zform = header
            .get_text(key!("ZFORM{n}").as_str())
            .ok_or(FitsError::MissingKeyword { name: "ZFORMn" })?
            .to_string();
        let tform = Tform::parse(&zform)?;
        let algo = match header.get_text(key!("ZCTYP{n}").as_str()) {
            Some(s) => Algo::parse(s)?,
            None => Algo::Gzip2, // cfitsio's default when ZCTYPn is absent
        };
        let m = col_meta(&tform, offset, algo)?;
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

    // `ZNAXIS2 · ZNAXIS1` from untrusted header values (`nrows` is unbounded):
    // guard the product up front — before reading any tile — so it can't wrap to a
    // too-small output buffer.
    let total = nrows
        .checked_mul(naxis1)
        .ok_or(FitsError::DataUnitOverflow)?;

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
    let sink = DisjointSlice::new(&mut out);
    try_for_each_tile(
        tile_count,
        TableDecodeScratch::default,
        |scratch, i| -> Result<()> {
            let chunk = i / ncols;
            let column = i % ncols;
            let m = &metas[column];
            let r0 = chunk * rpt;
            let rows = rpt.min(nrows - r0);
            let cell = cells[column].cell(chunk)?;
            decompress_column_into(convert::byte_cell(cell)?, m, rows, scratch)?;
            // SAFETY: chunks partition rows and column metadata partitions each row,
            // so every tile writes a distinct in-bounds byte range.
            unsafe { scatter_disjoint(&sink, &scratch.bytes, r0, rows, naxis1, m) };
            Ok(())
        },
    )?;

    // Restore the original header: drop the Z* keywords, reinstate NAXIS/PCOUNT.
    let mut h = header.clone();
    h.set("NAXIS1", fits_i64(naxis1)?);
    h.set("NAXIS2", fits_i64(nrows)?);
    h.set("PCOUNT", zpcount);
    for (n, zform) in zforms.iter().enumerate() {
        h.set(key!("TFORM{}", n + 1).as_str(), zform.clone());
        h.remove(key!("ZFORM{}", n + 1).as_str());
        h.remove(key!("ZCTYP{}", n + 1).as_str());
    }
    for key in [
        "ZTABLE", "ZTILELEN", "ZNAXIS1", "ZNAXIS2", "ZPCOUNT", "ZHEAPPTR",
    ] {
        h.remove(key);
    }
    Ok(HduParts {
        header: h,
        data: out,
    })
}

/// Compress one tile's column-major raw bytes per the column's algorithm.
fn compress_column(cm: &[u8], m: &ColMeta) -> Result<Vec<u8>> {
    Ok(match m.algo {
        Algo::Gzip1 => gzip::gzip_encode(cm, gzip::DEFAULT_GZIP_LEVEL),
        Algo::Gzip2 => gzip::gzip_encode(
            &gzip::shuffle_bytes(cm, m.shuffle_width()),
            gzip::DEFAULT_GZIP_LEVEL,
        ),
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix().ok_or(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", m.kind.code()),
            })?;
            rice::rice_encode(
                &convert::be_to_i64(cm, convert::bytepix_to_bitpix(bytepix)),
                bytepix,
                32,
            )
        }
    })
}

#[derive(Debug, Default)]
struct TableDecodeScratch {
    bytes: Vec<u8>,
    ints: Vec<i64>,
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
        Algo::Gzip1 => scratch.bytes = gzip::gunzip(bytes, expect)?,
        Algo::Gzip2 => {
            scratch.bytes = gzip::unshuffle_bytes(&gzip::gunzip(bytes, expect)?, m.shuffle_width())
        }
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
    }
    if scratch.bytes.len() != expect {
        return Err(FitsError::UnsupportedCompression {
            name: "decompressed column size mismatch".to_string(),
        });
    }
    Ok(())
}

unsafe fn scatter_disjoint(
    sink: &DisjointSlice<u8>,
    bytes: &[u8],
    r0: usize,
    rows: usize,
    row_len: usize,
    m: &ColMeta,
) {
    assert_eq!(bytes.len(), rows * m.width, "decompressed column size");
    for row in 0..rows {
        let offset = (r0 + row) * row_len + m.offset;
        // SAFETY: row chunks and column metadata partition the output, so this
        // in-bounds range does not overlap any concurrently accessed range.
        unsafe { sink.copy_from_slice(offset, &bytes[row * m.width..(row + 1) * m.width]) };
    }
}

fn req_int(header: &Header, key: &'static str) -> Result<i64> {
    header
        .get_integer(key)
        .ok_or(FitsError::MissingKeyword { name: key })
}

fn req_usize(header: &Header, key: &'static str) -> Result<usize> {
    usize::try_from(req_int(header, key)?).map_err(|_| FitsError::KeywordOutOfRange { name: key })
}

fn req_positive_usize(header: &Header, key: &'static str) -> Result<usize> {
    let value = req_usize(header, key)?;
    if value == 0 {
        return Err(FitsError::KeywordOutOfRange { name: key });
    }
    Ok(value)
}

fn optional_nonnegative(header: &Header, key: &'static str) -> Result<i64> {
    match header.get_integer(key) {
        Some(value) if value < 0 => Err(FitsError::KeywordOutOfRange { name: key }),
        Some(value) => Ok(value),
        None => Ok(0),
    }
}

fn fits_i64(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}
