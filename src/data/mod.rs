//! Typed data model (partial scaffold).
//!
//! FITS exposes data on two planes: a zero-copy *raw* plane (the stored,
//! big-endian samples) and a *physical* plane (`BZERO + BSCALE × stored`). The
//! bulk decode path that fills these from a [`crate::FitsReader`] data unit —
//! the SIMD/parallel endian-swap and scaling — is the next layer to build. The
//! types here are its target; the scaling map is already modelled and tested.

use crate::bitpix::Bitpix;
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
            Bitpix::I16 => ImageData::I16(from_be(bytes, i16::from_be_bytes)),
            Bitpix::I32 => ImageData::I32(from_be(bytes, i32::from_be_bytes)),
            Bitpix::I64 => ImageData::I64(from_be(bytes, i64::from_be_bytes)),
            Bitpix::F32 => ImageData::F32(from_be(bytes, f32::from_be_bytes)),
            Bitpix::F64 => ImageData::F64(from_be(bytes, f64::from_be_bytes)),
        }
    }

    /// Encode the samples to a big-endian byte buffer — the inverse of
    /// [`ImageData::decode`]. This is the unpadded data unit; the writer pads it
    /// to the 2880-byte block grid.
    pub(crate) fn encode(&self) -> Vec<u8> {
        match self {
            ImageData::U8(v) => v.clone(),
            ImageData::I16(v) => to_be(v, i16::to_be_bytes),
            ImageData::I32(v) => to_be(v, i32::to_be_bytes),
            ImageData::I64(v) => to_be(v, i64::to_be_bytes),
            ImageData::F32(v) => to_be(v, f32::to_be_bytes),
            ImageData::F64(v) => to_be(v, f64::to_be_bytes),
        }
    }
}

/// Decode a packed big-endian buffer into host-endian values of a fixed-width
/// type. `chunks_exact` consumes every element; any partial tail is impossible
/// because the caller passes whole elements only.
fn from_be<const N: usize, T>(bytes: &[u8], conv: fn([u8; N]) -> T) -> Vec<T> {
    bytes
        .chunks_exact(N)
        .map(|c| conv(c.try_into().expect("chunks_exact yields N-byte arrays")))
        .collect()
}

/// Pack fixed-width values into a big-endian byte buffer.
fn to_be<const N: usize, T: Copy>(values: &[T], conv: fn(T) -> [u8; N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * N);
    for &v in values {
        out.extend_from_slice(&conv(v));
    }
    out
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
    pub fn from_header(header: &Header) -> Scaling {
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
