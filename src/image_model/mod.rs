//! Typed image data model.
//!
//! FITS exposes image data on two planes: a *raw* plane (the stored samples) and a
//! *physical* plane (`BZERO + BSCALE × raw`). The stored samples are big-endian, so
//! [`Data::decode`] swaps a data unit into an owned, host-endian [`Data`]
//! and [`View::encode_into`] writes them back. When no swap is needed
//! (`BITPIX = 8`) an in-memory reader can skip even that copy and borrow the data
//! unit in place — see [`crate::ImageReader::view`]. The per-element swap loops are
//! memory-bandwidth-bound, so they lean on autovectorization rather than threads
//! (the thread-parallel layer is the compute-bound tiled codecs in the `compress`
//! module, not this path).

#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod data;
pub(crate) mod physical_sample;
pub(crate) mod region;
pub(crate) mod sample_type;
pub(crate) mod scaling;
pub(crate) mod unsigned_data;
pub(crate) mod view;

use crate::bitpix::Bitpix;
use crate::endian::decode_be_into_slice;
use crate::error::FitsError;
use crate::error::Ranked;
use crate::error::Result;
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::{Borrowed, View};
use crate::words;
use std::ops::Range;

/// Element count for an N-d `shape`: the product of the axis lengths, or `0` for
/// an empty shape (`NAXIS = 0` ⇒ no data, not the empty-product `1`).
pub(crate) fn shape_product(shape: &[usize]) -> Result<usize> {
    if shape.is_empty() || shape.contains(&0) {
        Ok(0)
    } else {
        shape
            .iter()
            .try_fold(1usize, |acc, &len| acc.checked_mul(len))
            .ok_or(FitsError::DataUnitOverflow)
    }
}

/// `shape`'s sample count, which must be `samples`.
pub(crate) fn check_sample_count(shape: &[usize], samples: usize) -> Result<usize> {
    let expected = shape_product(shape)?;
    if samples != expected {
        return Err(FitsError::DataSizeMismatch {
            expected,
            got: samples,
        });
    }
    Ok(expected)
}

/// Validate a zero-based, half-open N-d region against `shape` and write the
/// selected extent per axis to `selected`. The region must have the image's rank, and
/// each range must be ordered and within its axis. Shared by the plain-image section
/// reader and the tiled-compressed one, which apply the identical rule to the same
/// geometry.
pub(crate) fn validate_image_region(
    ranges: &[Range<usize>],
    shape: &[usize],
    selected: &mut Vec<usize>,
) -> Result<()> {
    if ranges.len() != shape.len() {
        return Err(FitsError::RankMismatch {
            ranked: Ranked::ImageRegion,
            expected: shape.len(),
            got: ranges.len(),
        });
    }
    selected.clear();
    selected.reserve_exact(shape.len());
    for (axis, (range, &len)) in ranges.iter().zip(shape).enumerate() {
        if range.start > range.end || range.end > len {
            return Err(FitsError::ImageRegionOutOfBounds {
                axis,
                start: range.start,
                end: range.end,
                len,
            });
        }
        selected.push(range.end - range.start);
    }
    Ok(())
}

/// Append the physical plane of a borrowed sample range to `out`: `BZERO + BSCALE ×
/// sample`, with integer samples equal to `BLANK` mapping to `NaN` (§3.4). The
/// `BITPIX` match runs once for the whole view rather than once per element, so a
/// caller wanting a sub-range (a random group's parameters or array) slices the view,
/// not the loop.
pub(crate) fn extend_physical<O: PhysicalSample>(
    view: View<'_>,
    scaling: &Scaling,
    out: &mut Vec<O>,
) {
    out.reserve(view.len());
    match view {
        View::U8(v) => out.extend(v.iter().map(|&x| O::scaled_integer(i64::from(x), scaling))),
        View::I16(v) => out.extend(v.iter().map(|&x| O::scaled_integer(i64::from(x), scaling))),
        View::I32(v) => out.extend(v.iter().map(|&x| O::scaled_integer(i64::from(x), scaling))),
        View::I64(v) => out.extend(v.iter().map(|&x| O::scaled_integer(x, scaling))),
        View::F32(v) => out.extend(v.iter().map(|&x| O::scaled(f64::from(x), scaling))),
        View::F64(v) => out.extend(v.iter().map(|&x| O::scaled(x, scaling))),
    }
}

/// [`extend_physical`] into a new `Vec`.
pub(crate) fn physical_view<O: PhysicalSample>(view: View<'_>, scaling: &Scaling) -> Vec<O> {
    let mut out = Vec::new();
    extend_physical(view, scaling, &mut out);
    out
}

/// Assert the buffer holds a whole number of `bitpix` elements — an invariant
/// between `data_extent`'s sizing and the declared axes, not a data-driven failure.
fn assert_whole_elements(bytes: &[u8], bitpix: Bitpix) {
    assert_eq!(
        bytes.len() % bitpix.elem_size(),
        0,
        "data length must be a whole number of {bitpix:?} elements"
    );
}

/// [`extend_physical`] straight from the big-endian stored bytes, in one pass: no
/// host-endian copy of the samples is made.
pub(crate) fn extend_physical_be<O: PhysicalSample>(
    bytes: &[u8],
    bitpix: Bitpix,
    scaling: &Scaling,
    out: &mut Vec<O>,
) {
    assert_whole_elements(bytes, bitpix);
    out.reserve(bytes.len() / bitpix.elem_size());
    let integer = |raw: i64| O::scaled_integer(raw, scaling);
    match bitpix {
        Bitpix::U8 => out.extend(bytes.iter().map(|&x| integer(i64::from(x)))),
        Bitpix::I16 => out.extend(
            (bytes.as_chunks().0.iter()).map(|&b| integer(i64::from(i16::from_be_bytes(b)))),
        ),
        Bitpix::I32 => out.extend(
            (bytes.as_chunks().0.iter()).map(|&b| integer(i64::from(i32::from_be_bytes(b)))),
        ),
        Bitpix::I64 => {
            out.extend((bytes.as_chunks().0.iter()).map(|&b| integer(i64::from_be_bytes(b))));
        }
        Bitpix::F32 => out.extend(
            (bytes.as_chunks().0.iter())
                .map(|&b| O::scaled(f64::from(f32::from_be_bytes(b)), scaling)),
        ),
        Bitpix::F64 => out.extend(
            (bytes.as_chunks().0.iter()).map(|&b| O::scaled(f64::from_be_bytes(b), scaling)),
        ),
    }
}

/// Byte-swap big-endian image bytes `src` into `words` — a `u64`-backed (8-byte-
/// aligned) reused scratch, resized to fit — so [`view_words`] can hand back typed
/// `&[T]` slices over the result. `bitpix` must not be `U8` (that needs no swap; the
/// reader borrows the source bytes directly).
pub(crate) fn swap_into_words(src: &[u8], bitpix: Bitpix, words: &mut Vec<u64>) {
    let count = src.len() / bitpix.elem_size();
    words.resize(src.len().div_ceil(8), 0);
    unsafe {
        match bitpix {
            Bitpix::I16 => {
                decode_be_into_slice(src, words::samples_mut(words, count), i16::from_be_bytes);
            }
            Bitpix::I32 => {
                decode_be_into_slice(src, words::samples_mut(words, count), i32::from_be_bytes);
            }
            Bitpix::I64 => {
                decode_be_into_slice(src, words::samples_mut(words, count), i64::from_be_bytes);
            }
            Bitpix::F32 => {
                decode_be_into_slice(src, words::samples_mut(words, count), f32::from_be_bytes);
            }
            Bitpix::F64 => {
                decode_be_into_slice(src, words::samples_mut(words, count), f64::from_be_bytes);
            }
            Bitpix::U8 => unreachable!("U8 is handled by the caller, never swapped"),
        }
    }
}

/// Reinterpret the first `nbytes` of a `u64`-backed host-endian scratch (written by
/// [`swap_into_words`] or the compressed-image decoder) as a typed [`View`].
/// `nbytes` is a whole number of
/// `bitpix` elements and `<= words.len() * 8`.
pub(crate) fn view_words(words: &[u64], bitpix: Bitpix, nbytes: usize) -> View<'_> {
    let count = nbytes / bitpix.elem_size();
    unsafe {
        match bitpix {
            Bitpix::U8 => View::U8(words::samples(words, count)),
            Bitpix::I16 => View::I16(words::samples(words, count)),
            Bitpix::I32 => View::I32(words::samples(words, count)),
            Bitpix::I64 => View::I64(words::samples(words, count)),
            Bitpix::F32 => View::F32(words::samples(words, count)),
            Bitpix::F64 => View::F64(words::samples(words, count)),
        }
    }
}

/// The `BZERO`/`TZEROn` offsets that realize the FITS unsigned-integer convention:
/// a sign-bit flip (`2^(n-1)`), exactly representable as `f64`. Shared by the image
/// (`BZERO`) and binary-table (`TZEROn`) unsigned paths.
pub(crate) const I8_OFFSET: f64 = -128.0; // −2⁷
pub(crate) const U16_OFFSET: f64 = 32_768.0; // 2¹⁵
pub(crate) const U32_OFFSET: f64 = 2_147_483_648.0; // 2³¹
pub(crate) const U64_OFFSET_INTEGER: u64 = 1_u64 << 63;
pub(crate) const U64_OFFSET: f64 = U64_OFFSET_INTEGER as f64;

/// The same offsets in the integer domain. Stored and physical differ by exactly the
/// sign bit, so the conversion is a XOR — and, being its own inverse, the identical
/// mask serves both reading a stored value and storing a physical one. Each mask is
/// written once here rather than inline at every conversion.
const I8_SIGN: u8 = 0x80;
const U16_SIGN: u16 = 0x8000;
const U32_SIGN: u32 = 0x8000_0000;
const U64_SIGN: u64 = U64_OFFSET_INTEGER;

const fn flip_i8(stored: u8) -> i8 {
    (stored ^ I8_SIGN).cast_signed()
}
const fn flip_u16(stored: i16) -> u16 {
    stored.cast_unsigned() ^ U16_SIGN
}
const fn flip_u32(stored: i32) -> u32 {
    stored.cast_unsigned() ^ U32_SIGN
}
const fn flip_u64(stored: i64) -> u64 {
    stored.cast_unsigned() ^ U64_SIGN
}

const fn store_i8(value: i8) -> u8 {
    value.cast_unsigned() ^ I8_SIGN
}
const fn store_u16(value: u16) -> i16 {
    (value ^ U16_SIGN).cast_signed()
}
const fn store_u32(value: u32) -> i32 {
    (value ^ U32_SIGN).cast_signed()
}
const fn store_u64(value: u64) -> i64 {
    (value ^ U64_SIGN).cast_signed()
}

/// An N-dimensional image: a flat, Fortran-ordered buffer (axis 0 varies
/// fastest), the axis lengths from `NAXISn`, and the scaling map that turns its
/// stored (raw) samples into physical values.
#[derive(Debug, Clone)]
pub struct Image {
    pub(crate) shape: Vec<usize>,
    pub(crate) samples: Data,
    pub(crate) scaling: Scaling,
}

/// An immutable view of an image's geometry and stored representation metadata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metadata<'a> {
    pub shape: &'a [usize],
    pub bitpix: Bitpix,
    pub scaling: Scaling,
}

impl Image {
    /// An image of `shape` over `samples`, any typed vector [`Data`] accepts, with
    /// `scaling` ([`Scaling::IDENTITY`] for none), as [`Borrowed::new`] builds a view.
    pub fn new(
        shape: impl Into<Vec<usize>>,
        samples: impl Into<Data>,
        scaling: Scaling,
    ) -> Result<Self> {
        let shape = shape.into();
        let samples = samples.into();
        Borrowed::new(&shape, samples.view(), scaling)?;
        Ok(Self {
            shape,
            samples,
            scaling,
        })
    }

    /// The image, borrowed: its geometry, scaling and host-endian stored samples, and
    /// the conversions from them.
    pub fn view(&self) -> Borrowed<'_> {
        Borrowed {
            shape: &self.shape,
            scaling: self.scaling,
            samples: self.samples.view(),
        }
    }

    /// Consume the image and return its host-endian stored samples.
    pub fn into_samples(self) -> Data {
        self.samples
    }

    /// Build an image storing a `u16` buffer via the FITS unsigned convention
    /// (`BITPIX = 16`, `BZERO = 2¹⁵`, `BSCALE = 1`) — the inverse of
    /// [`Borrowed::unsigned`]. The writer emits the `BZERO` keyword so it round-trips.
    pub fn from_u16(shape: impl Into<Vec<usize>>, data: &[u16]) -> Result<Self> {
        Self::offset_image(
            shape,
            Data::I16(data.iter().copied().map(store_u16).collect()),
            U16_OFFSET,
        )
    }

    /// Build an image storing a `u32` buffer (`BITPIX = 32`, `BZERO = 2³¹`).
    pub fn from_u32(shape: impl Into<Vec<usize>>, data: &[u32]) -> Result<Self> {
        Self::offset_image(
            shape,
            Data::I32(data.iter().copied().map(store_u32).collect()),
            U32_OFFSET,
        )
    }

    /// Build an image storing a `u64` buffer (`BITPIX = 64`, `BZERO = 2⁶³`).
    pub fn from_u64(shape: impl Into<Vec<usize>>, data: &[u64]) -> Result<Self> {
        Self::offset_image(
            shape,
            Data::I64(data.iter().copied().map(store_u64).collect()),
            U64_OFFSET,
        )
    }

    /// Build an image storing a signed-`i8` buffer (`BITPIX = 8`, `BZERO = -128`).
    pub fn from_i8(shape: impl Into<Vec<usize>>, data: &[i8]) -> Result<Self> {
        Self::offset_image(
            shape,
            Data::U8(data.iter().copied().map(store_i8).collect()),
            I8_OFFSET,
        )
    }

    fn offset_image(shape: impl Into<Vec<usize>>, samples: Data, bzero: f64) -> Result<Self> {
        Self::new(
            shape,
            samples,
            Scaling {
                bscale: 1.0,
                bzero,
                blank: None,
            },
        )
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::image_model::Image;
    use crate::image_model::physical_sample::PhysicalSample;

    /// The physical plane of `image`, in a new buffer.
    pub(crate) fn physical<T: PhysicalSample>(image: &Image) -> Vec<T> {
        let mut out = Vec::new();
        image.view().physical_into(&mut out);
        out
    }
}

#[cfg(test)]
mod tests;
