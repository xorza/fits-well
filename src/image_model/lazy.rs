//! [`Lazy`]: an image read straight from a source, over borrowed or
//! owned bytes.

use crate::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::sample_type::SampleType;
use crate::image_model::scaling::Scaling;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::{Metadata, physical_from_be, unsigned_from_be};

/// An image read from an HDU, in whichever form the reader could give cheaply —
/// returned by [`crate::FitsReader::read_image`] for *both* plain and tiled-
/// compressed images, so callers needn't know which they have. Carries the shape,
/// `BITPIX`, and [`Scaling`]; the pixels are exposed lazily through [`decode`],
/// [`u8`], [`physical`], and [`unsigned`].
///
/// A **plain** image borrows the data unit's big-endian bytes in place (zero-copy);
/// a **compressed** one (`ZIMAGE`) holds the reconstructed host-endian samples it had
/// to decompress. The accessors paper over the difference — e.g. [`u8`] is the
/// zero-copy `BITPIX = 8` plane either way — so you only reach for [`raw_bytes`] when
/// you specifically want the undecoded on-disk bytes (plain images only).
///
/// [`decode`]: Lazy::decode
/// [`u8`]: Lazy::u8
/// [`physical`]: Lazy::physical
/// [`unsigned`]: Lazy::unsigned
/// [`raw_bytes`]: Lazy::raw_bytes
#[derive(Debug)]
pub struct Lazy<'a> {
    pub(crate) shape: &'a [usize],
    pub(crate) scaling: Scaling,
    data: ImageBytes<'a>,
}

/// The two forms a [`Lazy`]'s pixels can take, by how it was read.
#[derive(Debug)]
enum ImageBytes<'a> {
    /// Plain image: the data unit's big-endian on-disk bytes, viewed in place over
    /// the source (or the reader's reused scratch for a seeking source).
    Raw { bytes: &'a [u8], bitpix: Bitpix },
    /// Compressed image (`ZIMAGE`): pixels reconstructed into an owned, host-endian
    /// buffer (only the `compression` feature ever builds this).
    #[cfg_attr(
        not(feature = "compression"),
        expect(dead_code, reason = "only the compression feature decodes an image")
    )]
    Decoded(Data),
}

impl<'a> Lazy<'a> {
    /// A plain image over borrowed big-endian bytes.
    pub(crate) const fn raw(
        shape: &'a [usize],
        bitpix: Bitpix,
        scaling: Scaling,
        bytes: &'a [u8],
    ) -> Self {
        Lazy {
            shape,
            scaling,
            data: ImageBytes::Raw { bytes, bitpix },
        }
    }

    /// A compressed image over its reconstructed, host-endian samples.
    #[cfg(feature = "compression")]
    pub(crate) const fn decoded(samples: Data, shape: &'a [usize], scaling: Scaling) -> Self {
        Lazy {
            shape,
            scaling,
            data: ImageBytes::Decoded(samples),
        }
    }

    /// The image geometry, stored element type, and physical-value scaling.
    pub fn metadata(&self) -> Metadata<'_> {
        Metadata {
            shape: self.shape,
            bitpix: self.bitpix(),
            scaling: self.scaling,
        }
    }

    /// The stored element type. For reconstructed compressed images this is
    /// derived from the owned sample buffer, so it cannot disagree with the data.
    pub fn bitpix(&self) -> Bitpix {
        match &self.data {
            ImageBytes::Raw { bitpix, .. } => *bitpix,
            ImageBytes::Decoded(samples) => samples.bitpix(),
        }
    }

    /// Consume the image and return its host-endian samples. Plain-image bytes are
    /// decoded into an owned buffer; a compressed image moves out its decoded plane.
    pub fn decode(self) -> Data {
        match self.data {
            ImageBytes::Raw { bytes, bitpix } => Data::decode(bytes, bitpix),
            ImageBytes::Decoded(samples) => samples,
        }
    }

    /// The samples as a borrowed `&[u8]` when no byte-swap is needed (`BITPIX = 8`):
    /// a plain image's borrowed on-disk bytes, or a compressed image's decoded `u8`
    /// buffer. `None` for multi-byte element types — use [`Lazy::decode`].
    pub fn u8(&self) -> Option<&[u8]> {
        match &self.data {
            ImageBytes::Raw {
                bytes,
                bitpix: Bitpix::U8,
            } => Some(bytes),
            ImageBytes::Decoded(Data::U8(v)) => Some(v),
            _ => None,
        }
    }

    /// The undecoded big-endian on-disk bytes — `Some` only for a **plain** image
    /// (zero-copy borrow); `None` for a compressed one, whose pixels were
    /// reconstructed and have no on-disk byte form. Use [`Lazy::decode`] for the
    /// samples regardless of form.
    pub const fn raw_bytes(&self) -> Option<&[u8]> {
        match &self.data {
            ImageBytes::Raw { bytes, .. } => Some(bytes),
            ImageBytes::Decoded(_) => None,
        }
    }

    /// The physical-plane values: `BZERO + BSCALE × sample`, `BLANK` → `NaN` (§3.4).
    pub fn physical(&self) -> Vec<f64> {
        match &self.data {
            ImageBytes::Raw { bytes, bitpix } => physical_from_be(bytes, *bitpix, &self.scaling),
            ImageBytes::Decoded(samples) => samples.physical_as::<f64>(&self.scaling),
        }
    }

    /// The physical plane narrowed to `f32` in a single pass — the compact, lossy
    /// counterpart to [`physical`](Lazy::physical). The scaling is still evaluated
    /// in `f64` (so each value is the correctly-rounded `f32`), but only one `Vec<f32>`
    /// is allocated rather than a `Vec<f64>` the caller then re-walks to narrow. Prefer
    /// it when the consumer wants `f32` regardless (display, GPU upload, `f32`
    /// pipelines); use [`physical`](Lazy::physical) when you need double precision —
    /// e.g. large `BITPIX = 64` integers or fine `BSCALE`/`BZERO` past `f32`'s range.
    pub fn physical_f32(&self) -> Vec<f32> {
        match &self.data {
            ImageBytes::Raw { bytes, bitpix } => physical_from_be(bytes, *bitpix, &self.scaling),
            ImageBytes::Decoded(samples) => samples.physical_as::<f32>(&self.scaling),
        }
    }

    /// Exact typed integers when the scaling is the FITS unsigned (or signed-byte)
    /// convention; `None` otherwise — same rule as [`Image::unsigned`](crate::image_model::Image::unsigned).
    pub fn unsigned(&self) -> Option<UnsignedData> {
        match &self.data {
            ImageBytes::Raw { bytes, bitpix } => unsigned_from_be(bytes, *bitpix, &self.scaling),
            ImageBytes::Decoded(samples) => samples.unsigned(&self.scaling),
        }
    }

    /// The effective element type these samples represent, resolving the unsigned and
    /// signed-byte conventions from `BITPIX` + [`Scaling`] without decoding the pixels.
    pub fn sample_type(&self) -> SampleType {
        SampleType::from_scaling(self.bitpix(), &self.scaling)
    }
}
