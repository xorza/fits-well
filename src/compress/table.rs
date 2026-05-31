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

use super::gzip;
use super::rice;
use crate::endian::decode_be;
use crate::endian::encode_be;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table::BinTable;
use crate::table::ColumnData;
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
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => requested,
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
    let elem_size = tform_elem_size(tform.kind);
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

/// In-row element size in bytes for a column kind (`A`/`X` count as 1).
fn tform_elem_size(kind: TformKind) -> usize {
    match kind {
        TformKind::Logical | TformKind::Bit | TformKind::Byte | TformKind::Char => 1,
        TformKind::I16 => 2,
        TformKind::I32 | TformKind::F32 => 4,
        TformKind::I64 | TformKind::F64 | TformKind::ComplexF32 => 8,
        TformKind::ComplexF64 => 16,
        TformKind::ArrayDesc32 => 8,
        TformKind::ArrayDesc64 => 16,
    }
}

/// Compress a fixed-width `BINTABLE` into a `ZTABLE` container. `rows_per_tile`
/// is the tile height (clamped to `[1, nrows]`); `default_algo` applies to every
/// column. Returns the compressed header and its data unit (Q descriptors + heap).
pub(crate) fn compress_table(
    header: &Header,
    table: &BinTable,
    rows_per_tile: usize,
    default_algo: &str,
) -> Result<(Header, Vec<u8>)> {
    let default_algo = Algo::parse(default_algo)?;
    let ncols = table.columns.len();
    let nrows = table.nrows;
    let naxis1 = table.row_width();
    let raw = table.raw_rows();

    let metas: Vec<ColMeta> = table
        .columns
        .iter()
        .map(|c| col_meta(&c.tform, c.byte_offset, default_algo))
        .collect::<Result<_>>()?;

    let rpt = rows_per_tile.clamp(1, nrows.max(1));
    let nchunks = nrows.div_ceil(rpt);

    // Per (chunk, column) Q descriptor (nelem, heap offset), and the heap.
    let mut descriptors = vec![(0u64, 0u64); nchunks * ncols];
    let mut heap: Vec<u8> = Vec::new();
    for chunk in 0..nchunks {
        let r0 = chunk * rpt;
        let rows = rpt.min(nrows - r0);
        for (ci, m) in metas.iter().enumerate() {
            // Transpose: gather this column's bytes across the tile's rows.
            let mut cm = Vec::with_capacity(rows * m.width);
            for r in 0..rows {
                let off = (r0 + r) * naxis1 + m.offset;
                cm.extend_from_slice(&raw[off..off + m.width]);
            }
            let comp = compress_column(&cm, m)?;
            descriptors[chunk * ncols + ci] = (comp.len() as u64, heap.len() as u64);
            heap.extend_from_slice(&comp);
        }
    }

    // Data unit: nchunks rows of ncols 16-byte Q descriptors, then the heap.
    let mut data = Vec::with_capacity(nchunks * ncols * 16 + heap.len());
    for &(nelem, off) in &descriptors {
        data.extend_from_slice(&(nelem as i64).to_be_bytes());
        data.extend_from_slice(&(off as i64).to_be_bytes());
    }
    data.extend_from_slice(&heap);

    // Header: copy the original, then layer on the Z* keywords.
    let mut h = header.clone();
    let orig_pcount = header.get_integer("PCOUNT").unwrap_or(0);
    h.set("ZTABLE", true)
        .comment("ZTABLE", "this is a compressed table");
    h.set("ZTILELEN", rpt as i64);
    h.set("ZNAXIS1", naxis1 as i64);
    h.set("ZNAXIS2", nrows as i64);
    h.set("ZPCOUNT", orig_pcount);
    for (ci, m) in metas.iter().enumerate() {
        let n = ci + 1;
        let zform = header
            .get_text(&format!("TFORM{n}"))
            .unwrap_or("")
            .to_string();
        h.set(&format!("ZFORM{n}"), zform);
        h.set(&format!("TFORM{n}"), "1QB");
        h.set(&format!("ZCTYP{n}"), m.algo.name());
    }
    h.set("NAXIS1", (ncols * 16) as i64);
    h.set("NAXIS2", nchunks as i64);
    h.set("PCOUNT", heap.len() as i64);
    h.set("GCOUNT", 1);
    Ok((h, data))
}

/// Uncompress a `ZTABLE` container back into its original fixed-width `BINTABLE`.
/// Returns the restored header and row-major data unit.
pub(crate) fn uncompress_table(header: &Header, table: &BinTable) -> Result<(Header, Vec<u8>)> {
    if header.get_logical("ZTABLE") != Some(true) {
        return Err(FitsError::NotCompressedTable);
    }
    let naxis1 = req_int(header, "ZNAXIS1")? as usize;
    let nrows = req_int(header, "ZNAXIS2")? as usize;
    let zpcount = header.get_integer("ZPCOUNT").unwrap_or(0);
    let mut rpt = req_int(header, "ZTILELEN")?.max(1) as usize;
    if rpt > nrows {
        rpt = nrows.max(1);
    }
    let ncols = req_int(header, "TFIELDS")? as usize;

    // Resolve each column's original form and codec.
    let mut metas = Vec::with_capacity(ncols);
    let mut zforms = Vec::with_capacity(ncols);
    let mut offset = 0;
    for n in 1..=ncols {
        let zform = header
            .get_text(&format!("ZFORM{n}"))
            .ok_or(FitsError::MissingKeyword { name: "ZFORMn" })?
            .to_string();
        let tform = Tform::parse(&zform)?;
        let algo = match header.get_text(&format!("ZCTYP{n}")) {
            Some(s) => Algo::parse(s)?,
            None => Algo::Gzip2, // cfitsio's default when ZCTYPn is absent
        };
        let m = col_meta(&tform, offset, algo)?;
        offset += m.width;
        zforms.push(zform);
        metas.push(m);
    }
    if offset != naxis1 {
        return Err(FitsError::RowWidthMismatch {
            computed: offset,
            declared: naxis1,
        });
    }

    let nchunks = nrows.div_ceil(rpt.max(1));
    // Each column's per-chunk compressed cells.
    let cells: Vec<Vec<ColumnData>> = (0..ncols)
        .map(|ci| table.read_vla_column(ci))
        .collect::<Result<_>>()?;

    let mut out = vec![0u8; nrows * naxis1];
    for chunk in 0..nchunks {
        let r0 = chunk * rpt;
        let rows = rpt.min(nrows - r0);
        for (ci, m) in metas.iter().enumerate() {
            let cell = cells[ci].get(chunk).ok_or(FitsError::UnexpectedEof)?;
            let bytes = match cell {
                ColumnData::Bytes(b) => b.as_slice(),
                _ => {
                    return Err(FitsError::UnsupportedCompression {
                        name: "compressed table cell is not a byte array".to_string(),
                    });
                }
            };
            let cm = decompress_column(bytes, m, rows)?;
            // Transpose back: scatter column-major bytes into the output rows.
            for r in 0..rows {
                let dst = (r0 + r) * naxis1 + m.offset;
                out[dst..dst + m.width].copy_from_slice(&cm[r * m.width..(r + 1) * m.width]);
            }
        }
    }

    // Restore the original header: drop the Z* keywords, reinstate NAXIS/PCOUNT.
    let mut h = header.clone();
    h.set("NAXIS1", naxis1 as i64);
    h.set("NAXIS2", nrows as i64);
    h.set("PCOUNT", zpcount);
    for (n, zform) in zforms.iter().enumerate() {
        h.set(&format!("TFORM{}", n + 1), zform.clone());
        h.remove(&format!("ZFORM{}", n + 1));
        h.remove(&format!("ZCTYP{}", n + 1));
    }
    for key in [
        "ZTABLE", "ZTILELEN", "ZNAXIS1", "ZNAXIS2", "ZPCOUNT", "ZHEAPPTR",
    ] {
        h.remove(key);
    }
    Ok((h, out))
}

/// Compress one tile's column-major raw bytes per the column's algorithm.
fn compress_column(cm: &[u8], m: &ColMeta) -> Result<Vec<u8>> {
    Ok(match m.algo {
        Algo::Gzip1 => gzip::gzip_encode(cm),
        Algo::Gzip2 => gzip::gzip_encode(&gzip::shuffle_bytes(cm, m.shuffle_width())),
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix().ok_or(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", m.kind.code()),
            })?;
            rice::rice_encode(&be_to_i64(cm, bytepix), bytepix, 32)
        }
    })
}

/// Decompress one tile's column cell back to `rows × width` column-major bytes.
fn decompress_column(bytes: &[u8], m: &ColMeta, rows: usize) -> Result<Vec<u8>> {
    let cm = match m.algo {
        Algo::Gzip1 => gzip::gunzip(bytes)?,
        Algo::Gzip2 => gzip::unshuffle_bytes(&gzip::gunzip(bytes)?, m.shuffle_width()),
        Algo::Rice1 => {
            let bytepix = m.rice_bytepix().ok_or(FitsError::UnsupportedCompression {
                name: format!("RICE_1 on a {} column", m.kind.code()),
            })?;
            let nelem = rows * m.repeat;
            i64_to_be(&rice::rice_decode(bytes, nelem, bytepix, 32), bytepix)
        }
    };
    if cm.len() != rows * m.width {
        return Err(FitsError::UnsupportedCompression {
            name: "decompressed column size mismatch".to_string(),
        });
    }
    Ok(cm)
}

/// Decode big-endian integers of `bytepix` bytes into `i64` values (signed).
fn be_to_i64(bytes: &[u8], bytepix: usize) -> Vec<i64> {
    match bytepix {
        1 => bytes.iter().map(|&b| b as i8 as i64).collect(),
        2 => decode_be(bytes, i16::from_be_bytes)
            .iter()
            .map(|&x| x as i64)
            .collect(),
        _ => decode_be(bytes, i32::from_be_bytes)
            .iter()
            .map(|&x| x as i64)
            .collect(),
    }
}

/// Encode `i64` values as big-endian integers of `bytepix` bytes.
fn i64_to_be(vals: &[i64], bytepix: usize) -> Vec<u8> {
    match bytepix {
        1 => vals.iter().map(|&v| v as u8).collect(),
        2 => encode_be(
            &vals.iter().map(|&v| v as i16).collect::<Vec<_>>(),
            i16::to_be_bytes,
        ),
        _ => encode_be(
            &vals.iter().map(|&v| v as i32).collect::<Vec<_>>(),
            i32::to_be_bytes,
        ),
    }
}

fn req_int(header: &Header, key: &'static str) -> Result<i64> {
    header
        .get_integer(key)
        .ok_or(FitsError::MissingKeyword { name: key })
}
