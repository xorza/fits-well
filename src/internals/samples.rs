//! Sample planes for tests: every `BITPIX`, a closed-form ramp of each, and bit-exact
//! comparison.

use crate::bitpix::Bitpix;
use crate::image_model::data::Data;
use std::ops::{Range, RangeInclusive};

/// Every `BITPIX`.
pub(crate) const BITPIXES: [Bitpix; 6] = [
    Bitpix::U8,
    Bitpix::I16,
    Bitpix::I32,
    Bitpix::I64,
    Bitpix::F32,
    Bitpix::F64,
];

/// The values a stored integer of `bitpix` holds, or `None` for a float type.
pub(crate) fn integer_range(bitpix: Bitpix) -> Option<RangeInclusive<i64>> {
    match bitpix {
        Bitpix::U8 => Some(0..=i64::from(u8::MAX)),
        Bitpix::I16 => Some(i64::from(i16::MIN)..=i64::from(i16::MAX)),
        Bitpix::I32 => Some(i64::from(i32::MIN)..=i64::from(i32::MAX)),
        Bitpix::I64 => Some(i64::MIN..=i64::MAX),
        Bitpix::F32 | Bitpix::F64 => None,
    }
}

/// Scale and zero pairs that are not finite: a NaN or infinite scale, the second with a
/// NaN zero too, and a NaN or infinite zero after a finite scale.
pub(crate) const NONFINITE: [[f64; 2]; 4] = [
    [f64::NAN, 0.0],
    [f64::INFINITY, f64::NAN],
    [1.0, f64::NAN],
    [1.0, f64::NEG_INFINITY],
];

/// `count` samples of `bitpix`: sample `i` is `i` (u8), `i − 30` (i16), `1000·i − 7` (i32),
/// `−50·i` (i64), `0.5·i` (f32) or `−0.25·i` (f64), so every type holds distinct values of
/// both signs where it can, each exact.
pub(crate) fn ramp(bitpix: Bitpix, count: usize) -> Data {
    debug_assert!(count <= 256, "a u8 ramp of {count} samples wraps");
    let index = 0..count;
    match bitpix {
        Bitpix::U8 => Data::U8(index.map(|i| u8::try_from(i).unwrap()).collect()),
        Bitpix::I16 => Data::I16(index.map(|i| i16::try_from(i).unwrap() - 30).collect()),
        Bitpix::I32 => Data::I32(
            index
                .map(|i| i32::try_from(i).unwrap() * 1000 - 7)
                .collect(),
        ),
        Bitpix::I64 => Data::I64(index.map(|i| i64::try_from(i).unwrap() * -50).collect()),
        Bitpix::F32 => Data::F32(
            index
                .map(|i| f32::from(u8::try_from(i).unwrap()) * 0.5)
                .collect(),
        ),
        Bitpix::F64 => Data::F64(
            index
                .map(|i| f64::from(u8::try_from(i).unwrap()) * -0.25)
                .collect(),
        ),
    }
}

/// The flat indices of the section `ranges` of an image of `shape`, in order, axis 0
/// fastest.
pub(crate) fn section_indices(shape: &[usize], ranges: &[Range<usize>]) -> Vec<usize> {
    debug_assert_eq!(shape.len(), ranges.len());
    let mut idxs = vec![0];
    let mut stride = 1;
    for (length, range) in shape.iter().zip(ranges) {
        let inner = idxs;
        idxs = range
            .clone()
            .flat_map(|i| inner.iter().map(move |&base| base + i * stride))
            .collect();
        stride *= length;
    }
    idxs
}

/// The samples of `data` at `idxs`, in order.
pub(crate) fn gather(data: &Data, idxs: &[usize]) -> Data {
    fn pick<T: Copy>(values: &[T], idxs: &[usize]) -> Vec<T> {
        idxs.iter().map(|&i| values[i]).collect()
    }
    match data {
        Data::U8(values) => Data::U8(pick(values, idxs)),
        Data::I16(values) => Data::I16(pick(values, idxs)),
        Data::I32(values) => Data::I32(pick(values, idxs)),
        Data::I64(values) => Data::I64(pick(values, idxs)),
        Data::F32(values) => Data::F32(pick(values, idxs)),
        Data::F64(values) => Data::F64(pick(values, idxs)),
    }
}

/// The bit pattern of each sample of `data`, zero-extended.
fn bits(data: &Data) -> Vec<u64> {
    match data {
        Data::U8(values) => values.iter().map(|&v| u64::from(v)).collect(),
        Data::I16(values) => values
            .iter()
            .map(|&v| u64::from(v.cast_unsigned()))
            .collect(),
        Data::I32(values) => values
            .iter()
            .map(|&v| u64::from(v.cast_unsigned()))
            .collect(),
        Data::I64(values) => values.iter().map(|&v| v.cast_unsigned()).collect(),
        Data::F32(values) => values.iter().map(|&v| u64::from(v.to_bits())).collect(),
        Data::F64(values) => values.iter().map(|&v| v.to_bits()).collect(),
    }
}

/// Assert that `actual` and `expected` hold one type and the same bits, so a NaN's payload
/// counts.
#[track_caller]
pub(crate) fn assert_same_bits(actual: &Data, expected: &Data, label: &str) {
    assert_eq!(actual.bitpix(), expected.bitpix(), "{label}: sample types");
    assert_eq!(bits(actual), bits(expected), "{label}");
}

/// The bit pattern of each sample of `data`, `None` for a NaN, so any NaN matches any other.
#[cfg(feature = "compression")]
pub(crate) fn nan_blind_bits(data: &Data) -> Vec<Option<u64>> {
    let is_nan = |i: usize| match data {
        Data::F32(values) => values[i].is_nan(),
        Data::F64(values) => values[i].is_nan(),
        Data::U8(_) | Data::I16(_) | Data::I32(_) | Data::I64(_) => false,
    };
    bits(data)
        .into_iter()
        .enumerate()
        .map(|(i, bits)| (!is_nan(i)).then_some(bits))
        .collect()
}
