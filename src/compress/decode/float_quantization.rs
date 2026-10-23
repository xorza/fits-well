//! Float-image dequantization inputs (§10.2) and the per-tile parameters they yield.

use crate::bintable::column_data::ColumnData;
use crate::bintable::table_view::TableView;
use crate::compress::ImageCodec;
use crate::compress::quantization::DitherMethod;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::{Header, keywords};

/// How a float image's tiles hold their values (§10.2). An integer image has none.
#[derive(Debug)]
pub(super) enum FloatQuantization {
    /// No `ZSCALE` and no `ZZERO` column, or `ZQUANTIZ = 'NONE'`: each primary cell
    /// holds the floats themselves, gzip'd, as cfitsio reads it whatever `ZQUANTIZ`
    /// says (`imcompress.c`, `imcomp_get_compressed_image_par`).
    Lossless(LosslessCodec),
    Quantized(Quantized),
}

/// The codec of a lossless float image: cfitsio writes one with no other
/// (`imcomp_init_table`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LosslessCodec {
    Gzip1,
    Gzip2,
}

/// How each tile's values come back: as stored floats, or dequantized.
#[derive(Debug, Clone, Copy)]
pub(super) enum FloatEncoding {
    Lossless(LosslessCodec),
    Quantized(Dequant),
}

/// The quantization inputs: the dither method and seed, and the per-tile
/// `ZSCALE`/`ZZERO`/`ZBLANK` metadata columns.
#[derive(Debug)]
pub(super) struct Quantized {
    method: DitherMethod,
    zdither0: i64,
    zblank_keyword: Option<i64>,
    zblank_column: Option<Vec<i64>>,
    zscale: Option<Vec<f64>>,
    zzero: Option<Vec<f64>>,
}

/// Per-tile float dequantization parameters (§10.2): `physical = zero + scale·I`,
/// the dither method/seed, and the integer null sentinel.
#[derive(Debug, Clone, Copy)]
pub(super) struct Dequant {
    pub(super) scale: f64,
    pub(super) zero: f64,
    pub(super) method: DitherMethod,
    pub(super) irow: i64,
    pub(super) zblank: Option<i64>,
}

impl FloatQuantization {
    /// The quantization a float image's `header` and `table` declare, its tiles stored
    /// with `codec`.
    pub(super) fn read(header: &Header, table: TableView<'_>, codec: ImageCodec) -> Result<Self> {
        let zscale = read_f64_column(table, "ZSCALE")?;
        let zzero = read_f64_column(table, "ZZERO")?;
        let quantiz = header.get_text(keywords::ZQUANTIZ)?.unwrap_or("NO_DITHER");
        if quantiz == "NONE" || (zscale.is_none() && zzero.is_none()) {
            return Ok(Self::Lossless(LosslessCodec::of(codec)?));
        }
        // A float image's samples cannot be reconstructed without reproducing its
        // dither exactly.
        let method =
            DitherMethod::parse(quantiz).ok_or_else(|| FitsError::UnsupportedCompression {
                name: format!("float quantization {quantiz}"),
            })?;
        let zdither0 = header.get_integer(keywords::ZDITHER0)?.unwrap_or(1);
        if !(1..=10_000).contains(&zdither0) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::ZDITHER0,
            });
        }
        Ok(Self::Quantized(Quantized {
            method,
            zdither0,
            zblank_keyword: header.get_integer(keywords::ZBLANK)?,
            zblank_column: read_i64_column(table, "ZBLANK")?,
            zscale,
            zzero,
        }))
    }

    /// How one tile's values come back.
    pub(super) fn encoding(&self, table_row: usize, tile_row: usize) -> FloatEncoding {
        match self {
            Self::Lossless(codec) => FloatEncoding::Lossless(*codec),
            Self::Quantized(quantized) => {
                FloatEncoding::Quantized(quantized.dequant(table_row, tile_row))
            }
        }
    }
}

impl LosslessCodec {
    fn of(codec: ImageCodec) -> Result<Self> {
        match codec {
            ImageCodec::Gzip1 => Ok(Self::Gzip1),
            ImageCodec::Gzip2 => Ok(Self::Gzip2),
            ImageCodec::Rice1
            | ImageCodec::Plio1
            | ImageCodec::Hcompress1
            | ImageCodec::NoCompress => Err(FitsError::UnsupportedCompression {
                name: format!("{} for a lossless float image", codec.name()),
            }),
        }
    }
}

impl Quantized {
    /// The dequantization parameters for one tile.
    fn dequant(&self, table_row: usize, tile_row: usize) -> Dequant {
        Dequant {
            scale: column_at(self.zscale.as_deref(), table_row).unwrap_or(1.0),
            zero: column_at(self.zzero.as_deref(), table_row).unwrap_or(0.0),
            method: self.method,
            irow: i64::try_from(tile_row).expect("a tile row fits i64") + self.zdither0,
            zblank: column_at(self.zblank_column.as_deref(), table_row).or(self.zblank_keyword),
        }
    }
}

/// Decode a named per-tile metadata column, or `None` when the table does not carry
/// it — every such column is optional, so absence is not an error.
fn read_tile_metadata(table: TableView<'_>, name: &str) -> Result<Option<ColumnData>> {
    match table.column_index(name) {
        Some(index) => Ok(Some(table.column(index)?.raw()?)),
        None => Ok(None),
    }
}

/// Read a per-tile `f64` column (e.g. `ZSCALE`/`ZZERO`), or `None` if absent.
fn read_f64_column(table: TableView<'_>, name: &str) -> Result<Option<Vec<f64>>> {
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
fn read_i64_column(table: TableView<'_>, name: &str) -> Result<Option<Vec<i64>>> {
    let Some(data) = read_tile_metadata(table, name)? else {
        return Ok(None);
    };
    match data {
        ColumnData::U8(v) => Ok(Some(v.iter().map(|&x| i64::from(x)).collect())),
        ColumnData::I16(v) => Ok(Some(v.iter().map(|&x| i64::from(x)).collect())),
        ColumnData::I32(v) => Ok(Some(v.iter().map(|&x| i64::from(x)).collect())),
        ColumnData::I64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "integer column",
        }),
    }
}

fn column_at<T: Copy>(col: Option<&[T]>, t: usize) -> Option<T> {
    let v = col?;
    v.get(t).copied()
}
