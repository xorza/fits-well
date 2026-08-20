//! [`ImageView`]: a borrowed image array, and [`BorrowedImage`], the view
//! plus the shape it is read with.

use crate::bitpix::Bitpix;
use crate::data::ImageMetadata;
use crate::data::image_data::ImageData;
use crate::data::scaling::Scaling;

/// A borrowed, host-endian view of FITS array samples, tagged by `BITPIX` — the
/// zero-/low-copy counterpart to the owned [`ImageData`]. It is returned by
/// [`Image::stored`](crate::data::Image::stored), [`crate::FitsReader::read_image_view`], and
/// [`crate::io::RandomGroupView`] to expose stored samples without copying.
/// Match it exactly like [`ImageData`]. A reader image view borrows reused decode
/// scratch (or the source bytes for `BITPIX = 8`) and therefore lasts only until
/// the next read; a random-group view borrows its owning [`crate::io::RandomGroups`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageView<'a> {
    U8(&'a [u8]),
    I16(&'a [i16]),
    I32(&'a [i32]),
    I64(&'a [i64]),
    F32(&'a [f32]),
    F64(&'a [f64]),
}

/// A scratch-backed image read: owned geometry and scaling paired with a borrowed,
/// host-endian sample view.
#[derive(Debug)]
pub struct BorrowedImage<'a> {
    pub shape: Vec<usize>,
    pub scaling: Scaling,
    pub samples: ImageView<'a>,
}

impl BorrowedImage<'_> {
    /// The image geometry, stored element type, and physical-value scaling.
    pub fn metadata(&self) -> ImageMetadata<'_> {
        ImageMetadata {
            shape: &self.shape,
            bitpix: self.samples.bitpix(),
            scaling: self.scaling,
        }
    }
}

impl ImageView<'_> {
    /// The `BITPIX` element kind backing this view.
    pub fn bitpix(&self) -> Bitpix {
        match self {
            ImageView::U8(_) => Bitpix::U8,
            ImageView::I16(_) => Bitpix::I16,
            ImageView::I32(_) => Bitpix::I32,
            ImageView::I64(_) => Bitpix::I64,
            ImageView::F32(_) => Bitpix::F32,
            ImageView::F64(_) => Bitpix::F64,
        }
    }

    /// Number of samples in the view.
    pub fn len(&self) -> usize {
        match self {
            ImageView::U8(v) => v.len(),
            ImageView::I16(v) => v.len(),
            ImageView::I32(v) => v.len(),
            ImageView::I64(v) => v.len(),
            ImageView::F32(v) => v.len(),
            ImageView::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copy this borrowed view into the matching owned [`ImageData`] variant.
    pub fn to_owned_data(&self) -> ImageData {
        match self {
            ImageView::U8(values) => ImageData::U8(values.to_vec()),
            ImageView::I16(values) => ImageData::I16(values.to_vec()),
            ImageView::I32(values) => ImageData::I32(values.to_vec()),
            ImageView::I64(values) => ImageData::I64(values.to_vec()),
            ImageView::F32(values) => ImageData::F32(values.to_vec()),
            ImageView::F64(values) => ImageData::F64(values.to_vec()),
        }
    }
}
