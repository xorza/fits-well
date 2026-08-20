//! Float-image dequantization inputs (§10.2) and the per-tile parameters they yield.

use crate::compress::DitherMethod;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::BinTable;
use crate::table_impl::column_data::ColumnData;

/// The float-quantization inputs (§10.2): the dither method and seed, and the
/// per-tile `ZSCALE`/`ZZERO`/`ZBLANK` metadata columns. An integer image parses
/// these too but never consults them.
#[derive(Debug)]
pub(super) struct FloatQuantization {
    method: DitherMethod,
    zdither0: i64,
    zblank_keyword: Option<i64>,
    zblank_column: Option<Vec<i64>>,
    zscale: Option<Vec<f64>>,
    zzero: Option<Vec<f64>>,
}

/// Per-tile float dequantization parameters (§10.2): `physical = zero + scale·I`,
/// the dither method/seed, and the integer null sentinel.
#[derive(Debug)]
pub(super) struct Dequant {
    pub(super) scale: f64,
    pub(super) zero: f64,
    pub(super) method: DitherMethod,
    pub(super) irow: i64,
    pub(super) zblank: Option<i64>,
}

impl FloatQuantization {
    pub(super) fn read(
        header: &Header,
        table: &BinTable,
        is_float: bool,
    ) -> Result<FloatQuantization> {
        let quantiz = header.get_text("ZQUANTIZ")?.unwrap_or("NO_DITHER");
        let method = match DitherMethod::parse(quantiz) {
            Some(method) => method,
            // A float image's samples cannot be reconstructed without reproducing its
            // dither exactly; an integer image ignores `ZQUANTIZ` altogether.
            None if is_float => {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("float quantization {quantiz}"),
                });
            }
            None => DitherMethod::None,
        };
        let zdither0 = header.get_integer("ZDITHER0")?.unwrap_or(1);
        if !(1..=10_000).contains(&zdither0) {
            return Err(FitsError::KeywordOutOfRange { name: "ZDITHER0" });
        }
        Ok(FloatQuantization {
            method,
            zdither0,
            zblank_keyword: header.get_integer("ZBLANK")?,
            zblank_column: read_i64_column(table, "ZBLANK")?,
            zscale: read_f64_column(table, "ZSCALE")?,
            zzero: read_f64_column(table, "ZZERO")?,
        })
    }

    /// The dequantization parameters for one tile.
    pub(super) fn dequant(&self, table_row: usize, tile_row: usize) -> Dequant {
        Dequant {
            scale: column_at(&self.zscale, table_row).unwrap_or(1.0),
            zero: column_at(&self.zzero, table_row).unwrap_or(0.0),
            method: self.method,
            irow: tile_row as i64 + self.zdither0,
            zblank: column_at(&self.zblank_column, table_row).or(self.zblank_keyword),
        }
    }
}

/// Decode a named per-tile metadata column, or `None` when the table does not carry
/// it — every such column is optional, so absence is not an error.
fn read_tile_metadata(table: &BinTable, name: &str) -> Result<Option<ColumnData>> {
    match table.column_index(name) {
        Some(index) => Ok(Some(table.column_by_idx(index)?.raw()?)),
        None => Ok(None),
    }
}

/// Read a per-tile `f64` column (e.g. `ZSCALE`/`ZZERO`), or `None` if absent.
fn read_f64_column(table: &BinTable, name: &str) -> Result<Option<Vec<f64>>> {
    let Some(data) = read_tile_metadata(table, name)? else {
        return Ok(None);
    };
    match data {
        ColumnData::F64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "f64 column",
        }),
    }
}

/// Read a per-tile integer column (e.g. a `ZBLANK` column), widening any integer
/// `TFORM` to `i64`, or `None` if absent.
fn read_i64_column(table: &BinTable, name: &str) -> Result<Option<Vec<i64>>> {
    let Some(data) = read_tile_metadata(table, name)? else {
        return Ok(None);
    };
    match data {
        ColumnData::Bytes(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I16(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I32(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "integer column",
        }),
    }
}

fn column_at<T: Copy>(col: &Option<Vec<T>>, t: usize) -> Option<T> {
    col.as_ref().and_then(|v| v.get(t).copied())
}
