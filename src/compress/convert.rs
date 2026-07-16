//! Byte/type conversions shared across the image and table compression paths:
//! gathering a tile's pixels, widening to/narrowing from the `i64`/`f64` the codecs
//! work in, big-endian (de)serialization, and tile-cell accessors.

use crate::allocation;
use crate::bitpix::Bitpix;
use crate::data::ImageData;
use crate::endian;
use crate::error::FitsError;
use crate::error::Result;
use crate::table::TformKind;
use crate::table::VlaCell;

/// Append `src` widened to `i64` to `out` — the repeated integer-widening arm of the
/// gather/cell helpers (`T` is one of `u8`/`i16`/`i32`/`i64`, all lossless to `i64`).
pub(crate) fn widen_i64<T: Copy + Into<i64>>(src: &[T], out: &mut Vec<i64>) {
    out.extend(src.iter().map(|&x| x.into()));
}

/// Gather a tile's integer pixels straight from the typed source into `out`,
/// widening to `i64` — so integer encoding never materializes a whole-image `i64`
/// buffer. Float sources yield nothing (they take the quantized float path).
pub(crate) fn gather_i64(
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
pub(crate) fn gather_f64(
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
pub(crate) fn i64_to_be_into(vals: &[i64], bitpix: Bitpix, out: &mut Vec<u8>) {
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
pub(crate) fn i64_to_be(vals: &[i64], bitpix: Bitpix) -> Vec<u8> {
    let mut out = Vec::new();
    i64_to_be_into(vals, bitpix, &mut out);
    out
}

/// Pack native-width quantized integers directly into reusable big-endian storage.
pub(crate) fn i32_to_be_into(vals: &[i32], out: &mut Vec<u8>) {
    out.clear();
    endian::extend_be(out, vals, i32::to_be_bytes);
}

/// Encode `f64` values as big-endian `bitpix`-width floats into reusable storage.
pub(crate) fn float_to_be_into(vals: &[f64], bitpix: Bitpix, out: &mut Vec<u8>) {
    out.clear();
    match bitpix {
        Bitpix::F32 => endian::extend_be(out, vals, |value| (value as f32).to_be_bytes()),
        Bitpix::F64 => endian::extend_be(out, vals, f64::to_be_bytes),
        _ => unreachable!("float_to_be_into requires a float bitpix"),
    }
}

/// Decode a big-endian buffer of `bitpix` integers into widened `i64` values in `out`
/// (cleared first). Single pass — no intermediate narrowed `Vec`; the
/// `from_be_bytes` + `as i64` closure inlines and vectorizes like `decode_be`.
pub(crate) fn be_to_i64_into(bytes: &[u8], bitpix: Bitpix, out: &mut Vec<i64>) {
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

/// Decode a big-endian buffer of `bitpix` floats into `f64` in `out`, widening in one
/// pass.
pub(crate) fn be_floats_into(bytes: &[u8], bitpix: Bitpix, out: &mut Vec<f64>) {
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

pub(crate) fn cell_to_i64_into(cell: VlaCell<'_>, out: &mut Vec<i64>) {
    match cell.element_type {
        TformKind::Byte => be_to_i64_into(cell.bytes, Bitpix::U8, out),
        TformKind::I16 => be_to_i64_into(cell.bytes, Bitpix::I16, out),
        TformKind::I32 => be_to_i64_into(cell.bytes, Bitpix::I32, out),
        TformKind::I64 => be_to_i64_into(cell.bytes, Bitpix::I64, out),
        _ => out.clear(),
    }
}

pub(crate) fn cell_to_f64_into(cell: VlaCell<'_>, zbitpix: Bitpix, out: &mut Vec<f64>) {
    match cell.element_type {
        TformKind::F32 => be_floats_into(cell.bytes, Bitpix::F32, out),
        TformKind::F64 => be_floats_into(cell.bytes, Bitpix::F64, out),
        TformKind::Byte => be_floats_into(cell.bytes, zbitpix, out),
        _ => out.clear(),
    }
}

pub(crate) fn byte_cell<'a>(cell: VlaCell<'a>) -> Result<&'a [u8]> {
    match cell.element_type {
        TformKind::Byte => Ok(cell.bytes),
        _ => Err(FitsError::UnsupportedCompression {
            name: "compressed cell is not a byte array".to_string(),
        }),
    }
}

pub(crate) fn plio_cell<'a>(cell: VlaCell<'a>) -> Result<&'a [u8]> {
    (cell.element_type == TformKind::I16)
        .then_some(cell.bytes)
        .ok_or_else(|| FitsError::UnsupportedCompression {
            name: "PLIO_1 data is not an i16 list".to_string(),
        })
}

pub(crate) fn bytepix_to_bitpix(bytepix: usize) -> Bitpix {
    match bytepix {
        1 => Bitpix::U8,
        2 => Bitpix::I16,
        8 => Bitpix::I64,
        _ => Bitpix::I32,
    }
}

/// A zeroed typed sample buffer of `len` elements. Parallel decode narrows into
/// per-tile buffers before scattering, so there is no whole-image `i64` or `f64`
/// intermediate. `len` comes from untrusted dimension keywords, so allocation is
/// fallible.
pub(crate) fn zeroed_samples(bitpix: Bitpix, len: usize) -> Result<ImageData> {
    Ok(match bitpix {
        Bitpix::U8 => ImageData::U8(allocation::try_zeroed(0u8, len)?),
        Bitpix::I16 => ImageData::I16(allocation::try_zeroed(0i16, len)?),
        Bitpix::I32 => ImageData::I32(allocation::try_zeroed(0i32, len)?),
        Bitpix::I64 => ImageData::I64(allocation::try_zeroed(0i64, len)?),
        Bitpix::F32 => ImageData::F32(allocation::try_zeroed(0.0f32, len)?),
        Bitpix::F64 => ImageData::F64(allocation::try_zeroed(0.0f64, len)?),
    })
}
