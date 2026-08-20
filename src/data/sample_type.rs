//! [`SampleType`]: which of the six FITS sample types an array holds.

use crate::bitpix::Bitpix;
use crate::data::scaling::Scaling;
use crate::data::{U16_OFFSET, U32_OFFSET, U64_OFFSET};

/// Which exact-integer realization of the FITS sign-bit-offset conventions a stored
/// type carries — effectively the tag of [`UnsignedData`], and the single thing both
/// the image (`BZERO`) and binary-table (`TZEROn`) paths must resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnsignedKind {
    I8,
    U16,
    U32,
    U64,
}

/// The effective element type of an image's *physical* samples — the analogue of
/// cfitsio's image "equivalent type". `BITPIX` records only the stored width and
/// signedness; the FITS unsigned and signed-byte conventions then layer a `BZERO`
/// offset on top (`BSCALE == 1` with `BZERO = 2^(n-1)`, or `BZERO = -128` for signed
/// bytes), so the values actually mean an unsigned (or signed-byte) integer. This
/// enum is what [`ReadImage::physical`](crate::data::read_image::ReadImage::physical) / [`ReadImage::unsigned`](crate::data::read_image::ReadImage::unsigned) yield, resolved up
/// front from `BITPIX` + [`Scaling`] without touching the pixels — so a caller can
/// pick a code path (e.g. a per-type normalization range) without re-deriving the
/// `BZERO` convention itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleType {
    /// `BITPIX = 8`, `BZERO = -128`: a signed byte.
    I8,
    /// `BITPIX = 8`: an unsigned byte (the FITS default for `BITPIX = 8`).
    U8,
    /// `BITPIX = 16`, no unsigned offset.
    I16,
    /// `BITPIX = 16`, `BZERO = 2¹⁵`.
    U16,
    /// `BITPIX = 32`, no unsigned offset.
    I32,
    /// `BITPIX = 32`, `BZERO = 2³¹`.
    U32,
    /// `BITPIX = 64`, no unsigned offset.
    I64,
    /// `BITPIX = 64`, `BZERO = 2⁶³`.
    U64,
    /// `BITPIX = -32`.
    F32,
    /// `BITPIX = -64`.
    F64,
}

impl SampleType {
    /// Resolve the effective type from the stored `BITPIX` and its [`Scaling`].
    ///
    /// A signed integer `BITPIX` whose scaling is exactly the unsigned (or signed-byte)
    /// convention — `BSCALE == 1` and `BZERO` the matching sign-bit offset — resolves
    /// to the corresponding unsigned (or `I8`) type; any other scaling leaves the
    /// stored type as-is. `BLANK` does not affect the classification: it marks null
    /// samples *within* a type, not the type itself.
    pub fn from_scaling(bitpix: Bitpix, scaling: &Scaling) -> SampleType {
        let offset = scaling.bscale == 1.0;
        match bitpix {
            Bitpix::U8 if offset && scaling.bzero == -128.0 => SampleType::I8,
            Bitpix::U8 => SampleType::U8,
            Bitpix::I16 if offset && scaling.bzero == U16_OFFSET => SampleType::U16,
            Bitpix::I16 => SampleType::I16,
            Bitpix::I32 if offset && scaling.bzero == U32_OFFSET => SampleType::U32,
            Bitpix::I32 => SampleType::I32,
            Bitpix::I64 if offset && scaling.bzero == U64_OFFSET => SampleType::U64,
            Bitpix::I64 => SampleType::I64,
            Bitpix::F32 => SampleType::F32,
            Bitpix::F64 => SampleType::F64,
        }
    }

    /// `true` for `U8`/`U16`/`U32`/`U64`.
    pub fn is_unsigned(self) -> bool {
        matches!(
            self,
            SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::U64
        )
    }

    /// `true` for `F32`/`F64`.
    pub fn is_float(self) -> bool {
        matches!(self, SampleType::F32 | SampleType::F64)
    }

    /// `true` for every integer variant (signed or unsigned).
    pub fn is_integer(self) -> bool {
        !self.is_float()
    }

    /// The exact-integer realization this type denotes, or `None` when no sign-bit
    /// offset is in play. `U8` is deliberately absent: a `BITPIX = 8` sample is
    /// natively unsigned, so there is nothing to recover.
    pub(crate) fn unsigned_kind(self) -> Option<UnsignedKind> {
        match self {
            SampleType::I8 => Some(UnsignedKind::I8),
            SampleType::U16 => Some(UnsignedKind::U16),
            SampleType::U32 => Some(UnsignedKind::U32),
            SampleType::U64 => Some(UnsignedKind::U64),
            _ => None,
        }
    }
}
