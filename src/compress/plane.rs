//! The integer and float sample planes, kept apart by type so that a codec routine
//! for one plane cannot be handed the other.

use crate::bintable::tform_kind::TformKind;
use crate::bitpix::Bitpix;
use crate::image_model::data::Data;

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
    pub(super) const fn from_bytepix(bytepix: usize) -> Option<Self> {
        match bytepix {
            1 => Some(Self::U8),
            2 => Some(Self::I16),
            4 => Some(Self::I32),
            8 => Some(Self::I64),
            _ => None,
        }
    }

    /// The integer a table column of `kind` stores, or `None` for any other kind.
    pub(super) const fn from_tform(kind: TformKind) -> Option<Self> {
        match kind {
            TformKind::Byte => Some(Self::U8),
            TformKind::I16 => Some(Self::I16),
            TformKind::I32 => Some(Self::I32),
            TformKind::I64 => Some(Self::I64),
            _ => None,
        }
    }

    pub(super) const fn bitpix(self) -> Bitpix {
        match self {
            Self::U8 => Bitpix::U8,
            Self::I16 => Bitpix::I16,
            Self::I32 => Bitpix::I32,
            Self::I64 => Bitpix::I64,
        }
    }

    pub(super) const fn elem_size(self) -> usize {
        self.bitpix().elem_size()
    }
}

impl FloatBitpix {
    pub(super) const fn bitpix(self) -> Bitpix {
        match self {
            Self::F32 => Bitpix::F32,
            Self::F64 => Bitpix::F64,
        }
    }

    pub(super) const fn elem_size(self) -> usize {
        self.bitpix().elem_size()
    }
}

impl Plane {
    pub(super) const fn of(bitpix: Bitpix) -> Self {
        match bitpix {
            Bitpix::U8 => Self::Int(IntBitpix::U8),
            Bitpix::I16 => Self::Int(IntBitpix::I16),
            Bitpix::I32 => Self::Int(IntBitpix::I32),
            Bitpix::I64 => Self::Int(IntBitpix::I64),
            Bitpix::F32 => Self::Float(FloatBitpix::F32),
            Bitpix::F64 => Self::Float(FloatBitpix::F64),
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
    pub(super) fn of(data: &Data) -> Samples<'_> {
        match data {
            Data::U8(values) => Samples::Int(IntSamples::U8(values)),
            Data::I16(values) => Samples::Int(IntSamples::I16(values)),
            Data::I32(values) => Samples::Int(IntSamples::I32(values)),
            Data::I64(values) => Samples::Int(IntSamples::I64(values)),
            Data::F32(values) => Samples::Float(FloatSamples::F32(values)),
            Data::F64(values) => Samples::Float(FloatSamples::F64(values)),
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
