//! [`View`]: a borrowed image array, and [`Borrowed`], the view plus the shape and
//! scaling it is read with.

use crate::fits_error::Result;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::sample::sealed::Raw;
use crate::image_model::sample_type::SampleType;
use crate::image_model::scaling::Scaling;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::{Image, Metadata, check_sample_count};
use std::ops::Range;

/// A borrowed, host-endian view of FITS array samples, tagged by `BITPIX` — the
/// zero-/low-copy counterpart to the owned [`Data`]. Match it exactly like [`Data`],
/// or take one type's slice with [`View::as_f32`] and its siblings. A view an
/// [`ImageReader`](crate::ImageReader) reads borrows its scratch (or the
/// source bytes for `BITPIX = 8`) and therefore lasts only until the next read; a
/// random-group view borrows its owning [`crate::io::RandomGroups`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View<'a> {
    U8(&'a [u8]),
    I16(&'a [i16]),
    I32(&'a [i32]),
    I64(&'a [i64]),
    F32(&'a [f32]),
    F64(&'a [f64]),
}

/// A borrowed image: its shape and scaling paired with a host-endian sample view whose
/// length the shape fixes. What an [`ImageReader`](crate::ImageReader) reads,
/// and what [`Image::view`] lends.
#[derive(Debug, Clone, Copy)]
pub struct Borrowed<'a> {
    pub(crate) shape: &'a [usize],
    pub(crate) scaling: Scaling,
    pub(crate) samples: View<'a>,
}

impl<'a> Borrowed<'a> {
    /// `samples` read with `shape` and `scaling`. Errors when the shape's product is not
    /// the sample count, or the scaling does not fit the samples' `BITPIX`.
    pub fn new(shape: &'a [usize], samples: View<'a>, scaling: Scaling) -> Result<Self> {
        let view = Self {
            shape,
            scaling,
            samples,
        };
        check_sample_count(shape, samples.len())?;
        scaling.validate(samples.bitpix())?;
        Ok(view)
    }

    /// The axis lengths, fastest first.
    pub const fn shape(&self) -> &'a [usize] {
        self.shape
    }

    pub const fn scaling(&self) -> Scaling {
        self.scaling
    }

    /// The host-endian samples, as many as the shape's product.
    pub const fn samples(&self) -> View<'a> {
        self.samples
    }

    /// The image geometry, stored element type, and physical-value scaling.
    pub const fn metadata(&self) -> Metadata<'a> {
        Metadata {
            shape: self.shape,
            bitpix: self.samples.bitpix(),
            scaling: self.scaling,
        }
    }

    /// Append the physical-plane values to `out`: `BZERO + BSCALE × sample` for every
    /// sample (§3.4), integer samples equal to `BLANK` as `NaN`, float `NaN`/`Inf`
    /// passed through. The arithmetic runs in `f64`, and an `f32` buffer narrows its
    /// result, a second rounding. The unsigned-integer convention falls out for free — a
    /// signed-16 buffer with `BZERO = 32768` gives the `u16` value.
    pub fn physical_into<T: PhysicalSample>(&self, out: &mut Vec<T>) {
        self.samples.extend_physical(&self.scaling, out);
    }

    /// The samples as exact typed integers into `out` when the scaling is precisely a
    /// FITS unsigned-integer (or signed-byte) convention: `BSCALE == 1`, no `BLANK`, and
    /// `BZERO` the matching sign-bit offset; `out`'s storage is reused when it holds that
    /// kind. Unlike [`Self::physical_into`], this is exact for all 64-bit values (no `f64`
    /// rounding past 2⁵³). `false`, and `out` as it was, for any other scaling or element
    /// type.
    pub fn unsigned_into(&self, out: &mut UnsignedData) -> bool {
        let Some(kind) = self.scaling.unsigned_kind(self.samples.bitpix()) else {
            return false;
        };
        out.fill_from_view(self.samples, kind);
        true
    }

    /// The effective element type these samples represent, resolving the unsigned and
    /// signed-byte conventions from the stored `BITPIX` and the [`Scaling`].
    pub fn sample_type(&self) -> SampleType {
        self.metadata().sample_type()
    }

    /// The image, copied into owned storage.
    pub fn to_image(&self) -> Image {
        Image {
            shape: self.shape.to_vec(),
            samples: self.samples.to_data(),
            scaling: self.scaling,
        }
    }
}

impl<'a> View<'a> {
    /// The `BITPIX` element kind backing this view.
    pub const fn bitpix(&self) -> Bitpix {
        match self {
            View::U8(_) => Bitpix::U8,
            View::I16(_) => Bitpix::I16,
            View::I32(_) => Bitpix::I32,
            View::I64(_) => Bitpix::I64,
            View::F32(_) => Bitpix::F32,
            View::F64(_) => Bitpix::F64,
        }
    }

    /// Number of samples in the view.
    pub const fn len(&self) -> usize {
        match self {
            View::U8(v) => v.len(),
            View::I16(v) => v.len(),
            View::I32(v) => v.len(),
            View::I64(v) => v.len(),
            View::F32(v) => v.len(),
            View::F64(v) => v.len(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The samples at `range`.
    ///
    /// # Panics
    /// When `range` runs past the view.
    #[must_use]
    pub fn slice(self, range: Range<usize>) -> Self {
        match self {
            View::U8(values) => View::U8(&values[range]),
            View::I16(values) => View::I16(&values[range]),
            View::I32(values) => View::I32(&values[range]),
            View::I64(values) => View::I64(&values[range]),
            View::F32(values) => View::F32(&values[range]),
            View::F64(values) => View::F64(&values[range]),
        }
    }

    pub const fn as_u8(&self) -> Option<&'a [u8]> {
        match *self {
            Self::U8(values) => Some(values),
            _ => None,
        }
    }

    pub const fn as_i16(&self) -> Option<&'a [i16]> {
        match *self {
            Self::I16(values) => Some(values),
            _ => None,
        }
    }

    pub const fn as_i32(&self) -> Option<&'a [i32]> {
        match *self {
            Self::I32(values) => Some(values),
            _ => None,
        }
    }

    pub const fn as_i64(&self) -> Option<&'a [i64]> {
        match *self {
            Self::I64(values) => Some(values),
            _ => None,
        }
    }

    pub const fn as_f32(&self) -> Option<&'a [f32]> {
        match *self {
            Self::F32(values) => Some(values),
            _ => None,
        }
    }

    pub const fn as_f64(&self) -> Option<&'a [f64]> {
        match *self {
            Self::F64(values) => Some(values),
            _ => None,
        }
    }

    /// Append the samples to `out` in big-endian order — the inverse of a read's
    /// decode. This is the unpadded data unit; the writer pads it to the
    /// 2880-byte block grid. Appends (never clears), so a writer reusing one buffer
    /// across HDUs clears it first and pays no per-image staging allocation.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            View::U8(v) => out.extend_from_slice(v),
            View::I16(v) => i16::extend_be(v, out),
            View::I32(v) => i32::extend_be(v, out),
            View::I64(v) => i64::extend_be(v, out),
            View::F32(v) => f32::extend_be(v, out),
            View::F64(v) => f64::extend_be(v, out),
        }
    }

    /// Append the physical plane of these samples to `out`, as
    /// [`Scaling::extend_physical`] gives it. The `BITPIX` match runs once for the
    /// whole view, so a caller wanting a sub-range slices the view, not the loop.
    pub(crate) fn extend_physical<O: PhysicalSample>(self, scaling: &Scaling, out: &mut Vec<O>) {
        match self {
            View::U8(v) => scaling.extend_physical(v, out),
            View::I16(v) => scaling.extend_physical(v, out),
            View::I32(v) => scaling.extend_physical(v, out),
            View::I64(v) => scaling.extend_physical(v, out),
            View::F32(v) => scaling.extend_physical(v, out),
            View::F64(v) => scaling.extend_physical(v, out),
        }
    }

    /// The sample at `index` as an `f64`.
    ///
    /// # Panics
    /// When `index` is past the view.
    pub(crate) fn get_f64(self, index: usize) -> f64 {
        match self {
            View::U8(v) => v[index].to_f64(),
            View::I16(v) => v[index].to_f64(),
            View::I32(v) => v[index].to_f64(),
            View::I64(v) => v[index].to_f64(),
            View::F32(v) => v[index].to_f64(),
            View::F64(v) => v[index].to_f64(),
        }
    }

    /// Copy this borrowed view into the matching owned [`Data`] variant.
    pub fn to_data(&self) -> Data {
        match self {
            View::U8(values) => Data::U8(values.to_vec()),
            View::I16(values) => Data::I16(values.to_vec()),
            View::I32(values) => Data::I32(values.to_vec()),
            View::I64(values) => Data::I64(values.to_vec()),
            View::F32(values) => Data::F32(values.to_vec()),
            View::F64(values) => Data::F64(values.to_vec()),
        }
    }
}
