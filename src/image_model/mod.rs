//! Typed image data model.
//!
//! FITS exposes image data on two planes: a *raw* plane (the stored samples) and a
//! *physical* plane (`BZERO + BSCALE × raw`). The stored samples are big-endian, so
//! a read swaps a data unit into an owned, host-endian [`Data`]
//! and [`View::encode_into`](crate::image_model::view::View::encode_into) writes them back. When no swap is needed
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
pub(crate) mod sample;
pub(crate) mod sample_type;
pub(crate) mod scaling;
pub(crate) mod unsigned_data;
pub(crate) mod unsigned_view;
pub(crate) mod view;

use crate::bitpix::Bitpix;
use crate::fits_error::FitsError;
use crate::fits_error::Ranked;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use crate::image_model::data::Data;
use crate::image_model::sample_type::SampleType;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::Borrowed;
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
        return Err(SizeUnit::Samples.mismatch(expected, samples));
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

/// An image taken apart: its axis lengths, its stored samples and their scaling.
#[derive(Debug, Clone)]
pub struct Parts {
    pub shape: Vec<usize>,
    pub samples: Data,
    pub scaling: Scaling,
}

/// An immutable view of an image's geometry and stored representation metadata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metadata<'a> {
    pub shape: &'a [usize],
    pub bitpix: Bitpix,
    pub scaling: Scaling,
}

impl Metadata<'_> {
    /// The effective element type the stored samples represent, resolving the unsigned
    /// and signed-byte conventions from `BITPIX` and the [`Scaling`].
    pub fn sample_type(&self) -> SampleType {
        SampleType::from_scaling(self.bitpix, self.scaling)
    }
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

    /// An image a read built from the geometry its header gave, which checked the
    /// scaling and fixed the sample count, so neither is checked again.
    pub(crate) fn of_geometry(shape: Vec<usize>, samples: Data, scaling: Scaling) -> Self {
        debug_assert!(
            check_sample_count(&shape, samples.len()).is_ok()
                && scaling.validate(samples.bitpix()).is_ok(),
            "a read's geometry fixes the samples and checked the scaling"
        );
        Self {
            shape,
            samples,
            scaling,
        }
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

    /// The axis lengths, fastest axis first.
    pub const fn shape(&self) -> &[usize] {
        self.shape.as_slice()
    }

    /// The host-endian stored samples.
    pub const fn samples(&self) -> &Data {
        &self.samples
    }

    /// The map from the stored samples to physical values.
    pub const fn scaling(&self) -> Scaling {
        self.scaling
    }

    /// Consume the image and return its host-endian stored samples.
    pub fn into_samples(self) -> Data {
        self.samples
    }

    /// Consume the image and return its parts, with no copy of the samples.
    pub fn into_parts(self) -> Parts {
        let Self {
            shape,
            samples,
            scaling,
        } = self;
        Parts {
            shape,
            samples,
            scaling,
        }
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
