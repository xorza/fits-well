//! What an image HDU's header says its array is.

use crate::bitpix::Bitpix;
use crate::data::ImageMetadata;
use crate::data::scaling::Scaling;
use crate::data::shape_product;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::header::Header;
use crate::keyword::key;

/// The array an image HDU holds, as its header declares it: the axis lengths, the
/// stored element type and the physical scaling. For a tiled-compressed image these
/// are the `ZNAXISn` and `ZBITPIX` of the image it encodes, not of its `BINTABLE`
/// container.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageGeometry {
    pub(crate) shape: Vec<usize>,
    pub(crate) bitpix: Bitpix,
    pub(crate) scaling: Scaling,
}

impl ImageGeometry {
    /// The geometry of an HDU of `kind`. Errors with [`FitsError::NotAnImage`] for a
    /// table, random groups or an unmodelled extension, and with
    /// [`FitsError::ImageHasGroups`] for a plain image with group structure.
    pub(crate) fn from_header(header: &Header, kind: HduKind) -> Result<ImageGeometry> {
        match kind {
            HduKind::Primary | HduKind::Image => {
                // §4.3: a plain image array has no group structure, so reserved
                // extension / random-groups counts cannot qualify its samples.
                if header.pcount()? != 0 || header.gcount()? != 1 {
                    return Err(FitsError::ImageHasGroups);
                }
                ImageGeometry::new(header, header.axes()?, header.bitpix()?)
            }
            HduKind::CompressedImage => ImageGeometry::compressed(header),
            HduKind::AsciiTable
            | HduKind::BinTable
            | HduKind::CompressedTable
            | HduKind::RandomGroups
            | HduKind::Other => Err(FitsError::NotAnImage),
        }
    }

    /// The image a tiled-compression `BINTABLE` encodes, from its `ZNAXISn` and
    /// `ZBITPIX` (§10.1.1).
    fn compressed(header: &Header) -> Result<ImageGeometry> {
        let bitpix = Bitpix::from_code(
            header
                .get_integer("ZBITPIX")?
                .ok_or(FitsError::MissingKeyword { name: "ZBITPIX" })?,
        )?;
        let rank = header
            .get_integer("ZNAXIS")?
            .ok_or(FitsError::MissingKeyword { name: "ZNAXIS" })?;
        let rank = usize::try_from(rank)
            .ok()
            .filter(|rank| *rank <= 999)
            .ok_or(FitsError::KeywordOutOfRange { name: "ZNAXIS" })?;
        let shape = (1..=rank)
            .map(|axis| header.required_usize(key!("ZNAXIS{axis}").as_str(), "ZNAXISn"))
            .collect::<Result<_>>()?;
        ImageGeometry::new(header, shape, bitpix)
    }

    /// Errors when the sample count does not fit a `usize`, so [`Self::len`] never has to.
    fn new(header: &Header, shape: Vec<usize>, bitpix: Bitpix) -> Result<ImageGeometry> {
        shape_product(&shape)?;
        Ok(ImageGeometry {
            shape,
            bitpix,
            scaling: header.scaling()?,
        })
    }

    /// The number of samples: 0 for an image with no axes or a zero-length axis.
    pub(crate) fn len(&self) -> usize {
        shape_product(&self.shape).expect("`from_header` checked the sample count")
    }

    /// The sample bytes the image occupies, stored or decompressed.
    pub(crate) fn byte_len(&self) -> Result<usize> {
        self.len()
            .checked_mul(self.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)
    }

    pub(crate) fn metadata(&self) -> ImageMetadata<'_> {
        ImageMetadata {
            shape: &self.shape,
            bitpix: self.bitpix,
            scaling: self.scaling,
        }
    }
}
