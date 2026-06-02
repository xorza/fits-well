//! Byte/type conversions shared across the image and table compression paths:
//! gathering a tile's pixels, widening to/narrowing from the `i64`/`f64` the codecs
//! work in, big-endian (de)serialization, and tile-cell accessors.

use crate::bitpix::Bitpix;
use crate::data::ImageData;
use crate::endian::encode_be;
use crate::error::FitsError;
use crate::error::Result;
use crate::table::ColumnData;

/// Append `src` widened to `i64` to `out` — the repeated integer-widening arm of the
/// gather/cell helpers (`T` is one of `u8`/`i16`/`i32`/`i64`, all lossless to `i64`).
pub(super) fn widen_i64<T: Copy + Into<i64>>(src: &[T], out: &mut Vec<i64>) {
    out.extend(src.iter().map(|&x| x.into()));
}

/// Gather a tile's integer pixels straight from the typed source into `out`,
/// widening to `i64` — so integer encoding never materializes a whole-image `i64`
/// buffer. Float sources yield nothing (they take the quantized float path).
pub(super) fn gather_i64(
    samples: &ImageData,
    row_bases: &[usize],
    row_len: usize,
    out: &mut Vec<i64>,
) {
    debug_assert!(!samples.bitpix().is_float(), "gather_i64 on a float source");
    out.clear();
    match samples {
        ImageData::U8(v) => {
            for &b in row_bases {
                widen_i64(&v[b..b + row_len], out);
            }
        }
        ImageData::I16(v) => {
            for &b in row_bases {
                widen_i64(&v[b..b + row_len], out);
            }
        }
        ImageData::I32(v) => {
            for &b in row_bases {
                widen_i64(&v[b..b + row_len], out);
            }
        }
        ImageData::I64(v) => {
            for &b in row_bases {
                out.extend_from_slice(&v[b..b + row_len]);
            }
        }
        _ => {}
    }
}

/// Gather a tile's float pixels straight from the typed source into `out`,
/// widening to `f64` — so float encoding never materializes a whole-image `f64`
/// buffer. Integer sources yield nothing (they take the integer path).
pub(super) fn gather_f64(
    samples: &ImageData,
    row_bases: &[usize],
    row_len: usize,
    out: &mut Vec<f64>,
) {
    debug_assert!(
        samples.bitpix().is_float(),
        "gather_f64 on an integer source"
    );
    out.clear();
    match samples {
        ImageData::F32(v) => {
            for &b in row_bases {
                out.extend(v[b..b + row_len].iter().map(|&x| x as f64));
            }
        }
        ImageData::F64(v) => {
            for &b in row_bases {
                out.extend_from_slice(&v[b..b + row_len]);
            }
        }
        _ => {}
    }
}

/// Narrow + pack `i64` values to big-endian `bitpix`-width integers in `out`, in a
/// single pass (no intermediate narrowed `Vec`). `out` is cleared first, so it can
/// be a reused scratch buffer. Grows once then writes each `N`-byte slot, the
/// vectorizable shape `extend_be` uses.
pub(super) fn i64_to_be_into(vals: &[i64], bitpix: Bitpix, out: &mut Vec<u8>) {
    debug_assert!(!bitpix.is_float(), "i64_to_be_into on a float bitpix");
    out.clear();
    out.resize(vals.len() * bitpix.elem_size(), 0);
    match bitpix {
        Bitpix::U8 => {
            for (slot, &v) in out.iter_mut().zip(vals) {
                *slot = v as u8;
            }
        }
        Bitpix::I16 => {
            for (slot, &v) in out.chunks_exact_mut(2).zip(vals) {
                slot.copy_from_slice(&(v as i16).to_be_bytes());
            }
        }
        Bitpix::I32 => {
            for (slot, &v) in out.chunks_exact_mut(4).zip(vals) {
                slot.copy_from_slice(&(v as i32).to_be_bytes());
            }
        }
        Bitpix::I64 => {
            for (slot, &v) in out.chunks_exact_mut(8).zip(vals) {
                slot.copy_from_slice(&v.to_be_bytes());
            }
        }
        _ => {}
    }
}

/// Owning form of [`i64_to_be_into`], for the few sites that keep the bytes (the
/// `NOCOMPRESS` cell is stored verbatim, so it can't share the reused scratch).
pub(super) fn i64_to_be(vals: &[i64], bitpix: Bitpix) -> Vec<u8> {
    let mut out = Vec::new();
    i64_to_be_into(vals, bitpix, &mut out);
    out
}

/// Encode `f64` values as a big-endian buffer of `bitpix`-width floats.
pub(super) fn float_to_be(vals: &[f64], bitpix: Bitpix) -> Vec<u8> {
    match bitpix {
        Bitpix::F32 => encode_be(
            &vals.iter().map(|&v| v as f32).collect::<Vec<_>>(),
            f32::to_be_bytes,
        ),
        _ => encode_be(vals, f64::to_be_bytes),
    }
}

/// Decode a big-endian buffer of `bitpix` integers into widened `i64` values in `out`
/// (cleared first). Single pass — no intermediate narrowed `Vec`; the
/// `from_be_bytes` + `as i64` closure inlines and vectorizes like `decode_be`.
pub(super) fn be_to_i64_into(bytes: &[u8], bitpix: Bitpix, out: &mut Vec<i64>) {
    debug_assert!(!bitpix.is_float(), "be_to_i64_into on a float bitpix");
    out.clear();
    match bitpix {
        Bitpix::U8 => out.extend(bytes.iter().map(|&b| b as i64)),
        Bitpix::I16 => out.extend(
            bytes
                .chunks_exact(2)
                .map(|c| i16::from_be_bytes(c.try_into().unwrap()) as i64),
        ),
        Bitpix::I32 => out.extend(
            bytes
                .chunks_exact(4)
                .map(|c| i32::from_be_bytes(c.try_into().unwrap()) as i64),
        ),
        Bitpix::I64 => out.extend(
            bytes
                .chunks_exact(8)
                .map(|c| i64::from_be_bytes(c.try_into().unwrap())),
        ),
        Bitpix::F32 | Bitpix::F64 => {} // excluded before this point
    }
}

/// Owning form of [`be_to_i64_into`], for the table path (which keeps the widened
/// values to feed the RICE encoder).
pub(super) fn be_to_i64(bytes: &[u8], bitpix: Bitpix) -> Vec<i64> {
    let mut out = Vec::new();
    be_to_i64_into(bytes, bitpix, &mut out);
    out
}

/// Decode a big-endian buffer of `bitpix` floats into `f64` in `out`, widening in one
/// pass.
pub(super) fn be_floats_into(bytes: &[u8], bitpix: Bitpix, out: &mut Vec<f64>) {
    out.clear();
    match bitpix {
        Bitpix::F32 => out.extend(
            bytes
                .chunks_exact(4)
                .map(|c| f32::from_be_bytes(c.try_into().unwrap()) as f64),
        ),
        Bitpix::F64 => out.extend(
            bytes
                .chunks_exact(8)
                .map(|c| f64::from_be_bytes(c.try_into().unwrap())),
        ),
        _ => {}
    }
}

/// Widen a raw (`UNCOMPRESSED_DATA`) integer tile cell to `i64` values in `out`.
pub(super) fn cell_to_i64_into(cell: &ColumnData, out: &mut Vec<i64>) {
    out.clear();
    match cell {
        ColumnData::Bytes(v) => widen_i64(v, out),
        ColumnData::I16(v) => widen_i64(v, out),
        ColumnData::I32(v) => widen_i64(v, out),
        ColumnData::I64(v) => out.extend_from_slice(v),
        _ => {}
    }
}

/// Widen a raw (`UNCOMPRESSED_DATA`) float tile cell to `f64` in `out`.
pub(super) fn cell_to_f64_into(cell: &ColumnData, zbitpix: Bitpix, out: &mut Vec<f64>) {
    out.clear();
    match cell {
        ColumnData::F32(v) => out.extend(v.iter().map(|&x| x as f64)),
        ColumnData::F64(v) => out.extend_from_slice(v),
        ColumnData::Bytes(b) => be_floats_into(b, zbitpix, out),
        _ => {}
    }
}

pub(super) fn cell_len(cell: &ColumnData) -> usize {
    match cell {
        ColumnData::Bytes(v) => v.len(),
        ColumnData::I16(v) => v.len(),
        ColumnData::I32(v) => v.len(),
        ColumnData::I64(v) => v.len(),
        _ => 0,
    }
}

pub(super) fn as_bytes(cell: &ColumnData) -> Result<&[u8]> {
    match cell {
        ColumnData::Bytes(b) => Ok(b),
        _ => Err(FitsError::UnsupportedCompression {
            name: "compressed cell is not a byte array".to_string(),
        }),
    }
}

pub(super) fn as_i16(cell: &ColumnData) -> Result<&[i16]> {
    match cell {
        ColumnData::I16(v) => Ok(v),
        _ => Err(FitsError::UnsupportedCompression {
            name: "PLIO_1 data is not an i16 list".to_string(),
        }),
    }
}

pub(super) fn bytepix_to_bitpix(bytepix: usize) -> Bitpix {
    match bytepix {
        1 => Bitpix::U8,
        2 => Bitpix::I16,
        8 => Bitpix::I64,
        _ => Bitpix::I32,
    }
}

/// Allocate a zero-filled `Vec<T>` of `len` elements, failing with
/// [`FitsError::DataUnitTooLarge`] instead of aborting the process when `len`
/// (driven by an untrusted `ZNAXISn`/`ZNAXIS2`) is too large for the allocator.
/// `try_reserve_exact` turns the would-be out-of-memory abort into a recoverable
/// error — and also rejects a capacity whose byte size overflows `isize`.
pub(super) fn try_zeroed<T: Clone>(value: T, len: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| FitsError::DataUnitTooLarge {
            bytes: len.saturating_mul(std::mem::size_of::<T>()),
        })?;
    v.resize(len, value);
    Ok(v)
}

/// A zeroed typed sample buffer of `len` elements — the decompression output the
/// tiles scatter into (narrowing as they land), so there is no whole-image `i64`
/// or `f64` intermediate to narrow afterwards. Allocated fallibly (see
/// [`try_zeroed`]): `len` comes from untrusted dimension keywords.
pub(super) fn zeroed_samples(bitpix: Bitpix, len: usize) -> Result<ImageData> {
    Ok(match bitpix {
        Bitpix::U8 => ImageData::U8(try_zeroed(0u8, len)?),
        Bitpix::I16 => ImageData::I16(try_zeroed(0i16, len)?),
        Bitpix::I32 => ImageData::I32(try_zeroed(0i32, len)?),
        Bitpix::I64 => ImageData::I64(try_zeroed(0i64, len)?),
        Bitpix::F32 => ImageData::F32(try_zeroed(0.0f32, len)?),
        Bitpix::F64 => ImageData::F64(try_zeroed(0.0f64, len)?),
    })
}
