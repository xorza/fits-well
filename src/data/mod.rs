//! Typed image data model.
//!
//! FITS exposes data on two planes: a zero-copy *raw* plane (the stored,
//! big-endian samples) and a *physical* plane (`BZERO + BSCALE × stored`). The
//! bulk decode/encode path that fills these from a [`crate::FitsReader`] data
//! unit is implemented here: [`ImageData::decode`] swaps a big-endian unit into
//! host-endian samples and [`ImageData::encode_into`] writes them back. Those
//! per-element loops are memory-bandwidth-bound, so they lean on
//! autovectorization rather than threads (the thread-parallel layer is the
//! compute-bound tiled codecs in the `compress` module, not this path).

use crate::bitpix::Bitpix;
use crate::endian::decode_be;
use crate::endian::extend_be;
use crate::header::Header;

/// An owned, host-endian sample buffer, tagged by its `BITPIX` element type.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    U8(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

/// Element count for an N-d `shape`: the product of the axis lengths, or `0` for
/// an empty shape (`NAXIS = 0` ⇒ no data, not the empty-product `1`).
pub(crate) fn shape_product(shape: &[usize]) -> usize {
    if shape.is_empty() {
        0
    } else {
        shape.iter().product()
    }
}

impl ImageData {
    /// The `BITPIX` element kind backing this buffer.
    pub fn bitpix(&self) -> Bitpix {
        match self {
            ImageData::U8(_) => Bitpix::U8,
            ImageData::I16(_) => Bitpix::I16,
            ImageData::I32(_) => Bitpix::I32,
            ImageData::I64(_) => Bitpix::I64,
            ImageData::F32(_) => Bitpix::F32,
            ImageData::F64(_) => Bitpix::F64,
        }
    }

    /// Number of samples in the buffer.
    pub fn len(&self) -> usize {
        match self {
            ImageData::U8(v) => v.len(),
            ImageData::I16(v) => v.len(),
            ImageData::I32(v) => v.len(),
            ImageData::I64(v) => v.len(),
            ImageData::F32(v) => v.len(),
            ImageData::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decode the raw, big-endian data unit into host-endian typed samples.
    /// `bytes` is the unpadded data (a whole number of `bitpix` elements); the
    /// fill past the data range must already be sliced off (see
    /// [`crate::DataUnit::data`]).
    pub(crate) fn decode(bytes: &[u8], bitpix: Bitpix) -> ImageData {
        assert_eq!(
            bytes.len() % bitpix.elem_size(),
            0,
            "data length must be a whole number of {bitpix:?} elements"
        );
        match bitpix {
            Bitpix::U8 => ImageData::U8(bytes.to_vec()),
            Bitpix::I16 => ImageData::I16(decode_be(bytes, i16::from_be_bytes)),
            Bitpix::I32 => ImageData::I32(decode_be(bytes, i32::from_be_bytes)),
            Bitpix::I64 => ImageData::I64(decode_be(bytes, i64::from_be_bytes)),
            Bitpix::F32 => ImageData::F32(decode_be(bytes, f32::from_be_bytes)),
            Bitpix::F64 => ImageData::F64(decode_be(bytes, f64::from_be_bytes)),
        }
    }

    /// Append the samples to `out` in big-endian order — the inverse of
    /// [`ImageData::decode`]. This is the unpadded data unit; the writer pads it
    /// to the 2880-byte block grid. Appends (never clears), so a writer reusing one
    /// buffer across HDUs clears it first and pays no per-image staging allocation.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            ImageData::U8(v) => out.extend_from_slice(v),
            ImageData::I16(v) => extend_be(out, v, i16::to_be_bytes),
            ImageData::I32(v) => extend_be(out, v, i32::to_be_bytes),
            ImageData::I64(v) => extend_be(out, v, i64::to_be_bytes),
            ImageData::F32(v) => extend_be(out, v, f32::to_be_bytes),
            ImageData::F64(v) => extend_be(out, v, f64::to_be_bytes),
        }
    }
}

/// The `BZERO`/`TZEROn` offsets that realize the FITS unsigned-integer convention:
/// a sign-bit flip (`2^(n-1)`), exactly representable as `f64`. Shared by the image
/// (`BZERO`) and binary-table (`TZEROn`) unsigned paths.
pub(crate) const U16_OFFSET: f64 = 32_768.0; // 2¹⁵
pub(crate) const U32_OFFSET: f64 = 2_147_483_648.0; // 2³¹
pub(crate) const U64_OFFSET: f64 = 9_223_372_036_854_775_808.0; // 2⁶³

/// A typed integer realization of the FITS unsigned (and signed-byte) storage
/// conventions — `BSCALE == 1` with `BZERO` the sign-bit offset. Values are exact
/// (no `f64` rounding), recovered by flipping the stored sign bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsignedView {
    /// `BITPIX = 8`, `BZERO = -128`: stored `u8` → `i8`.
    I8(Vec<i8>),
    /// `BITPIX = 16`, `BZERO = 2¹⁵`: stored `i16` → `u16`.
    U16(Vec<u16>),
    /// `BITPIX = 32`, `BZERO = 2³¹`: stored `i32` → `u32`.
    U32(Vec<u32>),
    /// `BITPIX = 64`, `BZERO = 2⁶³`: stored `i64` → `u64`.
    U64(Vec<u64>),
}

impl UnsignedView {
    /// Recover unsigned values from sign-bit-offset storage (the §5.2.5 / Table 19
    /// convention) by flipping the sign bit. Shared by [`Image::unsigned`] and
    /// `ColumnData::unsigned` so the bit math has one definition.
    pub(crate) fn from_signed_byte(stored: &[u8]) -> UnsignedView {
        UnsignedView::I8(stored.iter().map(|&x| (x ^ 0x80) as i8).collect())
    }
    pub(crate) fn from_offset_i16(stored: &[i16]) -> UnsignedView {
        UnsignedView::U16(stored.iter().map(|&x| (x as u16) ^ 0x8000).collect())
    }
    pub(crate) fn from_offset_i32(stored: &[i32]) -> UnsignedView {
        UnsignedView::U32(stored.iter().map(|&x| (x as u32) ^ 0x8000_0000).collect())
    }
    pub(crate) fn from_offset_i64(stored: &[i64]) -> UnsignedView {
        UnsignedView::U64(
            stored
                .iter()
                .map(|&x| (x as u64) ^ 0x8000_0000_0000_0000)
                .collect(),
        )
    }
}

/// An N-dimensional image: a flat, Fortran-ordered buffer (axis 0 varies
/// fastest), the axis lengths from `NAXISn`, and the scaling map that turns its
/// stored (raw) samples into physical values.
#[derive(Debug, Clone)]
pub struct Image {
    pub shape: Vec<usize>,
    pub samples: ImageData,
    pub scaling: Scaling,
}

impl Image {
    /// Build an image storing a `u16` buffer via the FITS unsigned convention
    /// (`BITPIX = 16`, `BZERO = 2¹⁵`, `BSCALE = 1`) — the inverse of
    /// [`Image::unsigned`]. The writer emits the `BZERO` keyword so it round-trips.
    pub fn from_u16(shape: Vec<usize>, data: &[u16]) -> Image {
        Image::offset_image(
            shape,
            ImageData::I16(data.iter().map(|&x| (x ^ 0x8000) as i16).collect()),
            U16_OFFSET,
        )
    }

    /// Build an image storing a `u32` buffer (`BITPIX = 32`, `BZERO = 2³¹`).
    pub fn from_u32(shape: Vec<usize>, data: &[u32]) -> Image {
        Image::offset_image(
            shape,
            ImageData::I32(data.iter().map(|&x| (x ^ 0x8000_0000) as i32).collect()),
            U32_OFFSET,
        )
    }

    /// Build an image storing a `u64` buffer (`BITPIX = 64`, `BZERO = 2⁶³`).
    pub fn from_u64(shape: Vec<usize>, data: &[u64]) -> Image {
        Image::offset_image(
            shape,
            ImageData::I64(
                data.iter()
                    .map(|&x| (x ^ 0x8000_0000_0000_0000) as i64)
                    .collect(),
            ),
            U64_OFFSET,
        )
    }

    /// Build an image storing a signed-`i8` buffer (`BITPIX = 8`, `BZERO = -128`).
    pub fn from_i8(shape: Vec<usize>, data: &[i8]) -> Image {
        Image::offset_image(
            shape,
            ImageData::U8(data.iter().map(|&x| (x as u8) ^ 0x80).collect()),
            -128.0,
        )
    }

    fn offset_image(shape: Vec<usize>, samples: ImageData, bzero: f64) -> Image {
        Image {
            shape,
            samples,
            scaling: Scaling {
                bscale: 1.0,
                bzero,
                blank: None,
            },
        }
    }

    /// Reinterpret the stored buffer as exact typed integers when the scaling is
    /// precisely a FITS unsigned-integer (or signed-byte) convention: `BSCALE == 1`,
    /// no `BLANK`, and `BZERO` the matching sign-bit offset. Unlike
    /// [`Image::physical`], this is exact for all 64-bit values (no `f64` rounding
    /// past 2⁵³). Returns `None` for any other scaling or element type.
    pub fn unsigned(&self) -> Option<UnsignedView> {
        if self.scaling.bscale != 1.0 || self.scaling.blank.is_some() {
            return None;
        }
        let bzero = self.scaling.bzero;
        match &self.samples {
            ImageData::U8(v) if bzero == -128.0 => Some(UnsignedView::from_signed_byte(v)),
            ImageData::I16(v) if bzero == U16_OFFSET => Some(UnsignedView::from_offset_i16(v)),
            ImageData::I32(v) if bzero == U32_OFFSET => Some(UnsignedView::from_offset_i32(v)),
            ImageData::I64(v) if bzero == U64_OFFSET => Some(UnsignedView::from_offset_i64(v)),
            _ => None,
        }
    }

    /// The physical-plane values: `BZERO + BSCALE × sample` for every sample
    /// (§3.4). Integer samples equal to the `BLANK` sentinel become `NaN`; float
    /// `NaN`/`Inf` pass through. The unsigned-integer convention falls out for
    /// free — e.g. a signed-16 buffer with `BZERO = 32768` yields the `u16` value.
    pub fn physical(&self) -> Vec<f64> {
        let Scaling {
            bscale,
            bzero,
            blank,
        } = self.scaling;
        let scale = |x: f64| bzero + bscale * x;
        match &self.samples {
            ImageData::U8(v) => scale_ints(v, blank, scale),
            ImageData::I16(v) => scale_ints(v, blank, scale),
            ImageData::I32(v) => scale_ints(v, blank, scale),
            ImageData::I64(v) => scale_ints(v, blank, scale),
            ImageData::F32(v) => v.iter().map(|&x| scale(x as f64)).collect(),
            ImageData::F64(v) => v.iter().map(|&x| scale(x)).collect(),
        }
    }
}

/// Scale an integer sample buffer to the physical plane, mapping the `BLANK`
/// sentinel (a stored integer value) to `NaN`.
fn scale_ints<T>(v: &[T], blank: Option<i64>, scale: impl Fn(f64) -> f64) -> Vec<f64>
where
    T: Copy + Into<i64>,
{
    v.iter()
        .map(|&x| {
            let xi: i64 = x.into();
            if blank == Some(xi) {
                f64::NAN
            } else {
                scale(xi as f64)
            }
        })
        .collect()
}

/// The linear `BSCALE`/`BZERO` map from a stored value to its physical value,
/// plus the integer `BLANK` sentinel marking undefined pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    pub bscale: f64,
    pub bzero: f64,
    pub blank: Option<i64>,
}

impl Scaling {
    /// The public entry point is [`Header::scaling`](crate::Header::scaling).
    pub(crate) fn from_header(header: &Header) -> Scaling {
        Scaling {
            bscale: header.get_real("BSCALE").unwrap_or(1.0),
            bzero: header.get_real("BZERO").unwrap_or(0.0),
            blank: header.get_integer("BLANK"),
        }
    }

    /// `true` when decoding needs no arithmetic — just an endian swap or copy.
    pub fn is_identity(&self) -> bool {
        self.bscale == 1.0 && self.bzero == 0.0
    }
}

#[cfg(test)]
mod tests;
