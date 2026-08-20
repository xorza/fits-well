//! Typed image data model.
//!
//! FITS exposes image data on two planes: a *raw* plane (the stored samples) and a
//! *physical* plane (`BZERO + BSCALE × raw`). The stored samples are big-endian, so
//! [`ImageData::decode`] swaps a data unit into an owned, host-endian [`ImageData`]
//! and [`ImageData::encode_into`] writes them back. When no swap is needed
//! (`BITPIX = 8`, or a big-endian host) an in-memory reader can skip even that copy
//! and borrow the data unit in place — see [`ReadImage`] /
//! [`crate::FitsReader::read_image`]. The per-element swap loops are
//! memory-bandwidth-bound, so they lean on autovectorization rather than threads
//! (the thread-parallel layer is the compute-bound tiled codecs in the `compress`
//! module, not this path).

pub(crate) mod image_data;
pub(crate) mod image_view;
pub(crate) mod physical_out;
pub(crate) mod read_image;
pub(crate) mod sample_type;
pub(crate) mod scaling;
pub(crate) mod unsigned_data;

use crate::bitpix::Bitpix;
use crate::data::image_data::ImageData;
use crate::data::image_view::ImageView;
use crate::data::physical_out::PhysicalOut;
use crate::data::sample_type::SampleType;
use crate::data::scaling::Scaling;
use crate::data::unsigned_data::UnsignedData;
use crate::endian::decode_be;
use crate::endian::decode_be_into_slice;
use crate::error::FitsError;
use crate::error::Ranked;
use crate::error::Result;
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

/// Validate a zero-based, half-open N-d region against `shape` and return the
/// selected extent per axis. The region must have the image's rank, and each range
/// must be ordered and within its axis. Shared by the plain-image section reader and
/// the tiled-compressed one, which apply the identical rule to the same geometry.
pub(crate) fn validate_image_region(
    ranges: &[Range<usize>],
    shape: &[usize],
) -> Result<Vec<usize>> {
    if ranges.len() != shape.len() {
        return Err(FitsError::RankMismatch {
            ranked: Ranked::ImageRegion,
            expected: shape.len(),
            got: ranges.len(),
        });
    }
    let mut selected = Vec::with_capacity(shape.len());
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
    Ok(selected)
}

/// The physical plane of a borrowed sample range: `BZERO + BSCALE × sample`, with
/// integer samples equal to `BLANK` mapping to `NaN` (§3.4). The `BITPIX` match runs
/// once for the whole view rather than once per element, so a caller wanting a
/// sub-range (a random group's parameters or array) slices the view, not the loop.
pub(crate) fn physical_view<O: PhysicalOut>(view: ImageView<'_>, scaling: &Scaling) -> Vec<O> {
    match view {
        ImageView::U8(v) => scale_ints(v, scaling),
        ImageView::I16(v) => scale_ints(v, scaling),
        ImageView::I32(v) => scale_ints(v, scaling),
        ImageView::I64(v) => scale_ints(v, scaling),
        ImageView::F32(v) => v.iter().map(|&x| O::scaled(x as f64, scaling)).collect(),
        ImageView::F64(v) => v.iter().map(|&x| O::scaled(x, scaling)).collect(),
    }
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

fn physical_from_be<O: PhysicalOut>(bytes: &[u8], bitpix: Bitpix, scaling: &Scaling) -> Vec<O> {
    assert_whole_elements(bytes, bitpix);
    match bitpix {
        Bitpix::U8 => bytes
            .iter()
            .map(|&x| O::scaled_integer(x as i64, scaling))
            .collect(),
        Bitpix::I16 => decode_be(bytes, |b| {
            O::scaled_integer(i16::from_be_bytes(b) as i64, scaling)
        }),
        Bitpix::I32 => decode_be(bytes, |b| {
            O::scaled_integer(i32::from_be_bytes(b) as i64, scaling)
        }),
        Bitpix::I64 => decode_be(bytes, |b| O::scaled_integer(i64::from_be_bytes(b), scaling)),
        Bitpix::F32 => decode_be(bytes, |b| O::scaled(f32::from_be_bytes(b) as f64, scaling)),
        Bitpix::F64 => decode_be(bytes, |b| O::scaled(f64::from_be_bytes(b), scaling)),
    }
}

fn unsigned_from_be(bytes: &[u8], bitpix: Bitpix, scaling: &Scaling) -> Option<UnsignedData> {
    assert_whole_elements(bytes, bitpix);
    let kind = scaling.unsigned_kind(bitpix)?;
    Some(UnsignedData::from_be_cells(
        std::iter::once(bytes),
        bytes.len() / bitpix.elem_size(),
        kind,
    ))
}

/// Byte-swap big-endian image bytes `src` into `words` — a `u64`-backed (8-byte-
/// aligned) reused scratch, resized to fit — so [`view_words`] can hand back typed
/// `&[T]` slices over the result. `bitpix` must not be `U8` (that needs no swap; the
/// reader borrows the source bytes directly).
pub(crate) fn swap_into_words(src: &[u8], bitpix: Bitpix, words: &mut Vec<u64>) {
    let count = src.len() / bitpix.elem_size();
    words.resize(src.len().div_ceil(8), 0);
    // SAFETY: `words` was just sized to hold `src.len()` bytes and filled with zeros,
    // which is a valid value for every sample type, so each `samples_mut` view covers
    // initialized storage. `src` is a separate buffer, so the typed slice never
    // aliases it.
    unsafe {
        match bitpix {
            Bitpix::I16 => {
                decode_be_into_slice(src, words::samples_mut(words, count), i16::from_be_bytes)
            }
            Bitpix::I32 => {
                decode_be_into_slice(src, words::samples_mut(words, count), i32::from_be_bytes)
            }
            Bitpix::I64 => {
                decode_be_into_slice(src, words::samples_mut(words, count), i64::from_be_bytes)
            }
            Bitpix::F32 => {
                decode_be_into_slice(src, words::samples_mut(words, count), f32::from_be_bytes)
            }
            Bitpix::F64 => {
                decode_be_into_slice(src, words::samples_mut(words, count), f64::from_be_bytes)
            }
            Bitpix::U8 => unreachable!("U8 is handled by the caller, never swapped"),
        }
    }
}

/// Reinterpret the first `nbytes` of a `u64`-backed host-endian scratch (written by
/// [`swap_into_words`] or the compressed-image decoder) as a typed [`ImageView`].
/// `nbytes` is a whole number of
/// `bitpix` elements and `<= words.len() * 8`.
pub(crate) fn view_words(words: &[u64], bitpix: Bitpix, nbytes: usize) -> ImageView<'_> {
    let count = nbytes / bitpix.elem_size();
    // SAFETY: `swap_into_words` (or the compressed-image decoder) wrote all `nbytes`
    // = `count` elements of host-endian samples, so every viewed element is
    // initialized. Alignment and bit-pattern validity are `words::samples`'s to
    // uphold, not this caller's.
    unsafe {
        match bitpix {
            Bitpix::U8 => ImageView::U8(words::samples(words, count)),
            Bitpix::I16 => ImageView::I16(words::samples(words, count)),
            Bitpix::I32 => ImageView::I32(words::samples(words, count)),
            Bitpix::I64 => ImageView::I64(words::samples(words, count)),
            Bitpix::F32 => ImageView::F32(words::samples(words, count)),
            Bitpix::F64 => ImageView::F64(words::samples(words, count)),
        }
    }
}

/// The `BZERO`/`TZEROn` offsets that realize the FITS unsigned-integer convention:
/// a sign-bit flip (`2^(n-1)`), exactly representable as `f64`. Shared by the image
/// (`BZERO`) and binary-table (`TZEROn`) unsigned paths.
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
    (stored ^ I8_SIGN) as i8
}
const fn flip_u16(stored: i16) -> u16 {
    (stored as u16) ^ U16_SIGN
}
const fn flip_u32(stored: i32) -> u32 {
    (stored as u32) ^ U32_SIGN
}
const fn flip_u64(stored: i64) -> u64 {
    (stored as u64) ^ U64_SIGN
}

const fn store_i8(value: i8) -> u8 {
    (value as u8) ^ I8_SIGN
}
const fn store_u16(value: u16) -> i16 {
    (value ^ U16_SIGN) as i16
}
const fn store_u32(value: u32) -> i32 {
    (value ^ U32_SIGN) as i32
}
const fn store_u64(value: u64) -> i64 {
    (value ^ U64_SIGN) as i64
}

/// An N-dimensional image: a flat, Fortran-ordered buffer (axis 0 varies
/// fastest), the axis lengths from `NAXISn`, and the scaling map that turns its
/// stored (raw) samples into physical values.
#[derive(Debug, Clone)]
pub struct Image {
    pub(crate) shape: Vec<usize>,
    pub(crate) samples: ImageData,
    pub(crate) scaling: Scaling,
}

/// An immutable view of an image's geometry and stored representation metadata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageMetadata<'a> {
    pub shape: &'a [usize],
    pub bitpix: Bitpix,
    pub scaling: Scaling,
}

impl Image {
    /// Build an identity-scaled image from any typed vector accepted by
    /// [`ImageData`].
    pub fn new(shape: impl Into<Vec<usize>>, samples: impl Into<ImageData>) -> Result<Image> {
        Image::new_scaled(shape, samples, Scaling::IDENTITY)
    }

    /// Build an image with explicit physical-value scaling.
    pub fn new_scaled(
        shape: impl Into<Vec<usize>>,
        samples: impl Into<ImageData>,
        scaling: Scaling,
    ) -> Result<Image> {
        let image = Image {
            shape: shape.into(),
            samples: samples.into(),
            scaling,
        };
        image.validate_geometry()?;
        image.scaling.validate(image.samples.bitpix())?;
        Ok(image)
    }

    /// The image geometry, stored element type, and physical-value scaling.
    pub fn metadata(&self) -> ImageMetadata<'_> {
        ImageMetadata {
            shape: &self.shape,
            bitpix: self.samples.bitpix(),
            scaling: self.scaling,
        }
    }

    /// Borrow the exact host-endian stored sample plane without allowing it to
    /// become inconsistent with the validated geometry.
    pub fn stored(&self) -> ImageView<'_> {
        self.samples.view(0..self.samples.len())
    }

    pub(crate) fn validate_geometry(&self) -> Result<usize> {
        let expected = shape_product(&self.shape)?;
        let got = self.samples.len();
        if got != expected {
            return Err(FitsError::DataSizeMismatch { expected, got });
        }
        Ok(expected)
    }

    /// Build an image storing a `u16` buffer via the FITS unsigned convention
    /// (`BITPIX = 16`, `BZERO = 2¹⁵`, `BSCALE = 1`) — the inverse of
    /// [`Image::unsigned`]. The writer emits the `BZERO` keyword so it round-trips.
    pub fn from_u16(shape: Vec<usize>, data: &[u16]) -> Result<Image> {
        Image::offset_image(
            shape,
            ImageData::I16(data.iter().copied().map(store_u16).collect()),
            U16_OFFSET,
        )
    }

    /// Build an image storing a `u32` buffer (`BITPIX = 32`, `BZERO = 2³¹`).
    pub fn from_u32(shape: Vec<usize>, data: &[u32]) -> Result<Image> {
        Image::offset_image(
            shape,
            ImageData::I32(data.iter().copied().map(store_u32).collect()),
            U32_OFFSET,
        )
    }

    /// Build an image storing a `u64` buffer (`BITPIX = 64`, `BZERO = 2⁶³`).
    pub fn from_u64(shape: Vec<usize>, data: &[u64]) -> Result<Image> {
        Image::offset_image(
            shape,
            ImageData::I64(data.iter().copied().map(store_u64).collect()),
            U64_OFFSET,
        )
    }

    /// Build an image storing a signed-`i8` buffer (`BITPIX = 8`, `BZERO = -128`).
    pub fn from_i8(shape: Vec<usize>, data: &[i8]) -> Result<Image> {
        Image::offset_image(
            shape,
            ImageData::U8(data.iter().copied().map(store_i8).collect()),
            -128.0,
        )
    }

    fn offset_image(shape: Vec<usize>, samples: ImageData, bzero: f64) -> Result<Image> {
        Image::new_scaled(
            shape,
            samples,
            Scaling {
                bscale: 1.0,
                bzero,
                blank: None,
            },
        )
    }

    /// Reinterpret the stored buffer as exact typed integers when the scaling is
    /// precisely a FITS unsigned-integer (or signed-byte) convention: `BSCALE == 1`,
    /// no `BLANK`, and `BZERO` the matching sign-bit offset. Unlike
    /// [`Image::physical`], this is exact for all 64-bit values (no `f64` rounding
    /// past 2⁵³). Returns `None` for any other scaling or element type.
    pub fn unsigned(&self) -> Option<UnsignedData> {
        self.samples.unsigned(&self.scaling)
    }

    /// The physical-plane values: `BZERO + BSCALE × sample` for every sample
    /// (§3.4). Integer samples equal to the `BLANK` sentinel become `NaN`; float
    /// `NaN`/`Inf` pass through. The unsigned-integer convention falls out for
    /// free — e.g. a signed-16 buffer with `BZERO = 32768` yields the `u16` value.
    pub fn physical(&self) -> Vec<f64> {
        self.samples.physical_as::<f64>(&self.scaling)
    }

    /// The physical plane narrowed to `f32` in a single pass — the compact, lossy
    /// counterpart to [`physical`](Image::physical); see [`ReadImage::physical_f32`](crate::data::read_image::ReadImage::physical_f32).
    pub fn physical_f32(&self) -> Vec<f32> {
        self.samples.physical_as::<f32>(&self.scaling)
    }

    /// The effective element type these samples represent, resolving the unsigned and
    /// signed-byte conventions from the stored `BITPIX` + [`Scaling`].
    pub fn sample_type(&self) -> SampleType {
        SampleType::from_scaling(self.samples.bitpix(), &self.scaling)
    }
}

/// Scale an integer sample buffer to the physical plane, mapping the `BLANK`
/// sentinel (a stored integer value) to `NaN`.
fn scale_ints<T, O>(v: &[T], scaling: &Scaling) -> Vec<O>
where
    T: Copy + Into<i64>,
    O: PhysicalOut,
{
    v.iter()
        .map(|&x| O::scaled_integer(x.into(), scaling))
        .collect()
}

#[cfg(test)]
mod tests;
