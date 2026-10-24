//! The float types a float image's tiles decode in.

use crate::bintable::element_kind::ElementKind;
use crate::endian;

/// A float image's stored type, which its tiles decode in: a value that needs no
/// arithmetic keeps every bit, a NaN's payload included, and a dequantized one is
/// computed in `f64` and rounded once, as cfitsio's `unquantize` does.
pub(crate) trait FloatSample: Copy + Send + Sync {
    const NAN: Self;
    /// The bytes of one value.
    const SIZE: usize;
    /// The cell element type that holds this type as is.
    const KIND: ElementKind;

    fn from_f64(value: f64) -> Self;

    /// `out`, cleared, filled with the big-endian values of this type `bytes` holds.
    fn decode_be(bytes: &[u8], out: &mut Vec<Self>);

    /// `out`, cleared, filled with the big-endian values of the other float width `bytes`
    /// holds, converted as cfitsio converts a column it reads.
    fn decode_other_be(bytes: &[u8], out: &mut Vec<Self>);
}

impl FloatSample for f32 {
    const NAN: Self = Self::NAN;
    const SIZE: usize = 4;
    const KIND: ElementKind = ElementKind::F32;

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the one rounding of a value computed in f64"
    )]
    fn from_f64(value: f64) -> Self {
        value as Self
    }

    fn decode_be(bytes: &[u8], out: &mut Vec<Self>) {
        endian::decode_be_into(bytes, out, Self::from_be_bytes);
    }

    fn decode_other_be(bytes: &[u8], out: &mut Vec<Self>) {
        endian::decode_be_into(bytes, out, |b| Self::from_f64(f64::from_be_bytes(b)));
    }
}

impl FloatSample for f64 {
    const NAN: Self = Self::NAN;
    const SIZE: usize = 8;
    const KIND: ElementKind = ElementKind::F64;

    fn from_f64(value: f64) -> Self {
        value
    }

    fn decode_be(bytes: &[u8], out: &mut Vec<Self>) {
        endian::decode_be_into(bytes, out, Self::from_be_bytes);
    }

    fn decode_other_be(bytes: &[u8], out: &mut Vec<Self>) {
        endian::decode_be_into(bytes, out, |b| Self::from(f32::from_be_bytes(b)));
    }
}
