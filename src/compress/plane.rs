//! The integer and float sample planes, kept apart by type so that a codec routine
//! for one plane cannot be handed the other.

use crate::bintable::tform_kind::TformKind;
use crate::bitpix::Bitpix;
use crate::data::image_data::ImageData;

/// An integer `BITPIX`: the plane the lossless codecs work in, and the one a float
/// image's quantized tiles are stored at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IntBitpix {
    U8,
    I16,
    I32,
    I64,
}

/// A float `BITPIX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FloatBitpix {
    F32,
    F64,
}

/// A `BITPIX` by plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Plane {
    Int(IntBitpix),
    Float(FloatBitpix),
}

impl IntBitpix {
    /// The integer of a `RICE_1` `BYTEPIX`, or `None` for a width Rice does not
    /// define (§10.4.1: 1, 2, 4 or 8).
    pub(super) const fn from_bytepix(bytepix: usize) -> Option<IntBitpix> {
        match bytepix {
            1 => Some(IntBitpix::U8),
            2 => Some(IntBitpix::I16),
            4 => Some(IntBitpix::I32),
            8 => Some(IntBitpix::I64),
            _ => None,
        }
    }

    /// The integer a table column of `kind` stores, or `None` for any other kind.
    pub(super) const fn from_tform(kind: TformKind) -> Option<IntBitpix> {
        match kind {
            TformKind::Byte => Some(IntBitpix::U8),
            TformKind::I16 => Some(IntBitpix::I16),
            TformKind::I32 => Some(IntBitpix::I32),
            TformKind::I64 => Some(IntBitpix::I64),
            _ => None,
        }
    }

    pub(super) const fn bitpix(self) -> Bitpix {
        match self {
            IntBitpix::U8 => Bitpix::U8,
            IntBitpix::I16 => Bitpix::I16,
            IntBitpix::I32 => Bitpix::I32,
            IntBitpix::I64 => Bitpix::I64,
        }
    }

    pub(super) const fn elem_size(self) -> usize {
        match self {
            IntBitpix::U8 => 1,
            IntBitpix::I16 => 2,
            IntBitpix::I32 => 4,
            IntBitpix::I64 => 8,
        }
    }
}

impl FloatBitpix {
    pub(super) const fn bitpix(self) -> Bitpix {
        match self {
            FloatBitpix::F32 => Bitpix::F32,
            FloatBitpix::F64 => Bitpix::F64,
        }
    }

    pub(super) const fn elem_size(self) -> usize {
        match self {
            FloatBitpix::F32 => 4,
            FloatBitpix::F64 => 8,
        }
    }
}

impl Plane {
    pub(super) const fn of(bitpix: Bitpix) -> Plane {
        match bitpix {
            Bitpix::U8 => Plane::Int(IntBitpix::U8),
            Bitpix::I16 => Plane::Int(IntBitpix::I16),
            Bitpix::I32 => Plane::Int(IntBitpix::I32),
            Bitpix::I64 => Plane::Int(IntBitpix::I64),
            Bitpix::F32 => Plane::Float(FloatBitpix::F32),
            Bitpix::F64 => Plane::Float(FloatBitpix::F64),
        }
    }
}

/// An image's samples by plane.
#[derive(Debug, Clone, Copy)]
pub(super) enum Samples<'a> {
    Int(IntSamples<'a>),
    Float(FloatSamples<'a>),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum IntSamples<'a> {
    U8(&'a [u8]),
    I16(&'a [i16]),
    I32(&'a [i32]),
    I64(&'a [i64]),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FloatSamples<'a> {
    F32(&'a [f32]),
    F64(&'a [f64]),
}

impl Samples<'_> {
    pub(super) fn of(data: &ImageData) -> Samples<'_> {
        match data {
            ImageData::U8(values) => Samples::Int(IntSamples::U8(values)),
            ImageData::I16(values) => Samples::Int(IntSamples::I16(values)),
            ImageData::I32(values) => Samples::Int(IntSamples::I32(values)),
            ImageData::I64(values) => Samples::Int(IntSamples::I64(values)),
            ImageData::F32(values) => Samples::Float(FloatSamples::F32(values)),
            ImageData::F64(values) => Samples::Float(FloatSamples::F64(values)),
        }
    }
}

impl IntSamples<'_> {
    pub(super) const fn bitpix(&self) -> IntBitpix {
        match self {
            IntSamples::U8(_) => IntBitpix::U8,
            IntSamples::I16(_) => IntBitpix::I16,
            IntSamples::I32(_) => IntBitpix::I32,
            IntSamples::I64(_) => IntBitpix::I64,
        }
    }
}

impl FloatSamples<'_> {
    pub(super) const fn bitpix(&self) -> FloatBitpix {
        match self {
            FloatSamples::F32(_) => FloatBitpix::F32,
            FloatSamples::F64(_) => FloatBitpix::F64,
        }
    }
}
