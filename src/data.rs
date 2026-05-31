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
}

/// An N-dimensional image: a flat, Fortran-ordered buffer (axis 0 varies
/// fastest) plus the axis lengths from `NAXISn`.
#[derive(Debug, Clone)]
pub struct Image {
    pub shape: Vec<usize>,
    pub samples: ImageData,
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
