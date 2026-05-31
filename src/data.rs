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
mod tests {
    use super::*;
    use crate::block::CARD_SIZE;

    fn header(lines: &[&str]) -> Header {
        let mut buf = Vec::new();
        for line in lines {
            let mut card = [b' '; CARD_SIZE];
            card[..line.len()].copy_from_slice(line.as_bytes());
            buf.extend_from_slice(&card);
        }
        let mut end = [b' '; CARD_SIZE];
        end[..3].copy_from_slice(b"END");
        buf.extend_from_slice(&end);
        Header::parse(&buf).unwrap()
    }

    fn image(samples: ImageData, scaling: Scaling) -> Image {
        Image {
            shape: vec![samples.len()],
            samples,
            scaling,
        }
    }

    #[test]
    fn decodes_big_endian_integers_and_floats() {
        // i16: 0x0001=1, 0xFFFF=-1, 0x8000=-32768 (the unsigned-u16 min sentinel).
        assert_eq!(
            ImageData::decode(&[0x00, 0x01, 0xFF, 0xFF, 0x80, 0x00], Bitpix::I16),
            ImageData::I16(vec![1, -1, -32768])
        );
        // BITPIX=8 is raw bytes, no byte order.
        assert_eq!(
            ImageData::decode(&[1, 2, 3], Bitpix::U8),
            ImageData::U8(vec![1, 2, 3])
        );
        // i32 0x00000100 = 256.
        assert_eq!(
            ImageData::decode(&[0, 0, 1, 0], Bitpix::I32),
            ImageData::I32(vec![256])
        );
        // i64 = 5.
        assert_eq!(
            ImageData::decode(&[0, 0, 0, 0, 0, 0, 0, 5], Bitpix::I64),
            ImageData::I64(vec![5])
        );
        // f32 1.0 = 0x3F800000, f64 1.0 = 0x3FF0000000000000.
        assert_eq!(
            ImageData::decode(&[0x3F, 0x80, 0x00, 0x00], Bitpix::F32),
            ImageData::F32(vec![1.0])
        );
        assert_eq!(
            ImageData::decode(&[0x3F, 0xF0, 0, 0, 0, 0, 0, 0], Bitpix::F64),
            ImageData::F64(vec![1.0])
        );
    }

    #[test]
    fn physical_applies_scaling_and_maps_blank_to_nan() {
        // 10 -> 5 + 2·10 = 25 ; 20 == BLANK -> NaN ; -5 -> 5 + 2·-5 = -5
        let img = image(
            ImageData::I16(vec![10, 20, -5]),
            Scaling {
                bscale: 2.0,
                bzero: 5.0,
                blank: Some(20),
            },
        );
        let phys = img.physical();
        assert_eq!(phys[0], 25.0);
        assert!(phys[1].is_nan());
        assert_eq!(phys[2], -5.0);
    }

    #[test]
    fn physical_realizes_unsigned_16_bit_via_the_bzero_offset() {
        // u16 trick: signed-16 storage with BSCALE=1, BZERO=32768.
        // -32768 -> 0, 0 -> 32768, 32767 -> 65535.
        let img = image(
            ImageData::I16(vec![-32768, 0, 32767]),
            Scaling {
                bscale: 1.0,
                bzero: 32768.0,
                blank: None,
            },
        );
        assert_eq!(img.physical(), vec![0.0, 32768.0, 65535.0]);
    }

    #[test]
    fn float_physical_scales_and_passes_nan_through() {
        let img = image(
            ImageData::F32(vec![1.5, f32::NAN]),
            Scaling {
                bscale: 10.0,
                bzero: 1.0,
                blank: None,
            },
        );
        let phys = img.physical();
        assert_eq!(phys[0], 16.0); // 1 + 10·1.5
        assert!(phys[1].is_nan());
    }

    #[test]
    fn image_data_reports_its_bitpix() {
        assert_eq!(ImageData::U8(vec![]).bitpix(), Bitpix::U8);
        assert_eq!(ImageData::I16(vec![]).bitpix(), Bitpix::I16);
        assert_eq!(ImageData::I32(vec![]).bitpix(), Bitpix::I32);
        assert_eq!(ImageData::I64(vec![]).bitpix(), Bitpix::I64);
        assert_eq!(ImageData::F32(vec![]).bitpix(), Bitpix::F32);
        assert_eq!(ImageData::F64(vec![]).bitpix(), Bitpix::F64);
    }

    #[test]
    fn scaling_defaults_to_the_identity_map() {
        let s = Scaling::from_header(&header(&["SIMPLE  = T"]));
        assert_eq!(
            s,
            Scaling {
                bscale: 1.0,
                bzero: 0.0,
                blank: None
            }
        );
        assert!(s.is_identity());
    }

    #[test]
    fn scaling_reads_explicit_keywords() {
        let s = Scaling::from_header(&header(&[
            "BSCALE  = 2.5",
            "BZERO   = -1000.0",
            "BLANK   = -32768",
        ]));
        assert_eq!(
            s,
            Scaling {
                bscale: 2.5,
                bzero: -1000.0,
                blank: Some(-32768)
            }
        );
        assert!(!s.is_identity());
    }

    #[test]
    fn unsigned_16_bit_offset_is_not_an_identity_map() {
        // The unsigned-u16 trick: BSCALE=1, BZERO=32768.
        let s = Scaling::from_header(&header(&["BSCALE  = 1", "BZERO   = 32768"]));
        assert_eq!(s.bzero, 32768.0);
        assert!(!s.is_identity());
    }
}
