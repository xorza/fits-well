//! Float-image dequantization inputs (§10.2) and the per-tile parameters they yield.

use crate::bintable::table_view::TableView;
use crate::compress::decode::float_sample::FloatSample;
use crate::compress::decode::tile_value::{self, TileValue};
use crate::compress::quantization::DitherMethod;
use crate::compress::quantize::{DitherSequence, ZERO_VALUE};
use crate::compress::tile_codec::TileCodec;
use crate::error::CompressionFeature;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::{Header, keywords};

/// What a float image's header says of its quantization, read once for every decode:
/// `ZQUANTIZ`, `ZDITHER0`, and the `ZSCALE` and `ZZERO` keywords that fix a parameter
/// for every tile.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct FloatKeywords {
    quantiz: Quantiz,
    zdither0: i64,
    zscale: Option<f64>,
    zzero: Option<f64>,
}

/// A `ZQUANTIZ` value.
#[derive(Debug, Clone, PartialEq)]
enum Quantiz {
    /// `'NONE'`: the floats are stored losslessly.
    None,
    Method(DitherMethod),
    /// A value this crate does not model, refused only when the image is quantized.
    Unknown(String),
}

/// How a float image's tiles hold their values (§10.2). An integer image has none.
#[derive(Debug)]
pub(super) enum FloatQuantization {
    /// No `ZSCALE` and no `ZZERO`, as a keyword or a column, or `ZQUANTIZ = 'NONE'`: each
    /// primary cell holds the floats themselves, gzip'd, as cfitsio reads it whatever
    /// `ZQUANTIZ` says (`imcompress.c`, `imcomp_get_compressed_image_par`).
    /// The floats gzip'd with a codec that [`TileCodec::stores_lossless_floats`].
    Lossless(TileCodec),
    Quantized(Quantized),
}

/// How each tile's values come back: as stored floats, or dequantized.
#[derive(Debug, Clone, Copy)]
pub(super) enum FloatEncoding {
    Lossless(TileCodec),
    Quantized(Dequant),
}

/// The quantization inputs: the dither method and seed, and the per-tile `ZSCALE`,
/// `ZZERO` and `ZBLANK`.
#[derive(Debug)]
pub(super) struct Quantized {
    method: DitherMethod,
    zdither0: i64,
    zscale: TileValue<f64>,
    zzero: TileValue<f64>,
    zblank: TileValue<i64>,
}

/// Per-tile float dequantization parameters (§10.2): `physical = zero + scale·I`,
/// the dither method/seed, and the integer null sentinel.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dequant {
    pub(crate) scale: f64,
    pub(crate) zero: f64,
    pub(crate) method: DitherMethod,
    pub(crate) irow: i64,
    pub(crate) zblank: Option<i64>,
}

impl FloatKeywords {
    pub(super) fn read(header: &Header) -> Result<Self> {
        let quantiz = match header.get_text(keywords::ZQUANTIZ)?.unwrap_or("NO_DITHER") {
            "NONE" => Quantiz::None,
            name => DitherMethod::parse(name)
                .map_or_else(|| Quantiz::Unknown(name.to_string()), Quantiz::Method),
        };
        let zdither0 = header.get_integer(keywords::ZDITHER0)?.unwrap_or(1);
        if matches!(quantiz, Quantiz::Method(method) if method.dithered())
            && !(1..=10_000).contains(&zdither0)
        {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::ZDITHER0,
            });
        }
        Ok(Self {
            quantiz,
            zdither0,
            zscale: header.get_real(keywords::ZSCALE)?,
            zzero: header.get_real(keywords::ZZERO)?,
        })
    }
}

impl FloatKeywords {
    /// The quantization these keywords, the quantized null `zblank` and the container
    /// `table` declare, the tiles stored with `codec`.
    pub(super) fn quantization(
        &self,
        zblank: TileValue<i64>,
        table: TableView<'_>,
        codec: TileCodec,
    ) -> Result<FloatQuantization> {
        let zscale = TileValue::new(self.zscale, tile_value::real_column(table, "ZSCALE")?);
        let zzero = TileValue::new(self.zzero, tile_value::real_column(table, "ZZERO")?);
        if self.quantiz == Quantiz::None || (zscale.is_absent() && zzero.is_absent()) {
            if !codec.stores_lossless_floats() {
                return Err(FitsError::UnsupportedCompression {
                    feature: CompressionFeature::LosslessFloatCodec {
                        codec: codec.name(),
                    },
                });
            }
            return Ok(FloatQuantization::Lossless(codec));
        }
        // A float image's samples cannot be reconstructed without reproducing its
        // dither exactly.
        let method = match &self.quantiz {
            Quantiz::Method(method) => *method,
            Quantiz::Unknown(name) => {
                return Err(FitsError::UnsupportedCompression {
                    feature: CompressionFeature::Quantization { name: name.clone() },
                });
            }
            Quantiz::None => unreachable!("a lossless image returned above"),
        };
        Ok(FloatQuantization::Quantized(Quantized {
            method,
            zdither0: self.zdither0,
            zscale,
            zzero,
            zblank,
        }))
    }
}

impl FloatQuantization {
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

impl Quantized {
    /// The dequantization parameters for one tile: a scale of 1 and a zero of 0 where
    /// neither the keyword nor the column gives one, as cfitsio defaults them.
    fn dequant(&self, table_row: usize, tile_row: usize) -> Dequant {
        Dequant {
            scale: self.zscale.at(table_row).unwrap_or(1.0),
            zero: self.zzero.at(table_row).unwrap_or(0.0),
            method: self.method,
            irow: i64::try_from(tile_row).expect("a tile row fits i64") + self.zdither0,
            zblank: self.zblank.at(table_row),
        }
    }
}

impl Dequant {
    /// `out`, cleared, filled with the floats of the quantized `ints` (cfitsio
    /// `unquantize_i4r4`): `(i − dither + 0.5)·scale + zero`, or `i·scale + zero` with no
    /// dither. An `i` equal to `zblank` is `NaN`, and `SUBTRACTIVE_DITHER_2`'s
    /// [`ZERO_VALUE`] is exactly 0. The dither cursor advances on every pixel.
    pub(crate) fn apply<F: FloatSample>(&self, ints: &[i64], out: &mut Vec<F>) {
        let Self { scale, zero, .. } = *self;
        out.clear();
        out.reserve_exact(ints.len());
        match self.method {
            DitherMethod::None => self.fill(ints, out, |i| scale * i as f64 + zero),
            DitherMethod::Subtractive1 => {
                let mut dither = DitherSequence::new(self.irow);
                self.fill(ints, out, |i| {
                    (i as f64 - dither.next() + 0.5) * scale + zero
                });
            }
            DitherMethod::Subtractive2 => {
                let mut dither = DitherSequence::new(self.irow);
                self.fill(ints, out, |i| {
                    let r = dither.next();
                    if i == i64::from(ZERO_VALUE) {
                        0.0
                    } else {
                        (i as f64 - r + 0.5) * scale + zero
                    }
                });
            }
        }
    }

    /// `out` extended with `value` of each of `ints`, `NaN` where `ints` holds `zblank`.
    fn fill<F: FloatSample>(
        &self,
        ints: &[i64],
        out: &mut Vec<F>,
        mut value: impl FnMut(i64) -> f64,
    ) {
        match self.zblank {
            None => out.extend(ints.iter().map(|&i| F::from_f64(value(i)))),
            Some(zblank) => out.extend(ints.iter().map(|&i| {
                let value = value(i);
                if i == zblank {
                    F::NAN
                } else {
                    F::from_f64(value)
                }
            })),
        }
    }
}
