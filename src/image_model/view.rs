//! [`View`]: a borrowed image array, and [`Borrowed`], the view
//! plus the shape it is read with.

use crate::bitpix::Bitpix;
use crate::image_model::Metadata;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use std::ops::Range;

/// A borrowed, host-endian view of FITS array samples, tagged by `BITPIX` — the
/// zero-/low-copy counterpart to the owned [`Data`]. It is returned by
/// [`Image::stored`](crate::image_model::Image::stored), [`crate::FitsReader::read_image_view`], and
/// [`crate::io::RandomGroupView`] to expose stored samples without copying.
/// Match it exactly like [`Data`]. A reader image view borrows reused decode
/// scratch (or the source bytes for `BITPIX = 8`) and therefore lasts only until
/// the next read; a random-group view borrows its owning [`crate::io::RandomGroups`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View<'a> {
    U8(&'a [u8]),
    I16(&'a [i16]),
    I32(&'a [i32]),
    I64(&'a [i64]),
    F32(&'a [f32]),
    F64(&'a [f64]),
}

/// A scratch-backed image read: the shape and scaling paired with a borrowed,
/// host-endian sample view whose length the shape fixes.
#[derive(Debug)]
pub struct Borrowed<'a> {
    pub(crate) shape: &'a [usize],
    pub(crate) scaling: Scaling,
    pub(crate) samples: View<'a>,
}

impl<'a> Borrowed<'a> {
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
}

impl View<'_> {
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
    pub(crate) fn slice(self, range: Range<usize>) -> Self {
        match self {
            View::U8(values) => View::U8(&values[range]),
            View::I16(values) => View::I16(&values[range]),
            View::I32(values) => View::I32(&values[range]),
            View::I64(values) => View::I64(&values[range]),
            View::F32(values) => View::F32(&values[range]),
            View::F64(values) => View::F64(&values[range]),
        }
    }

    /// Copy this borrowed view into the matching owned [`Data`] variant.
    pub fn to_owned_data(&self) -> Data {
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
