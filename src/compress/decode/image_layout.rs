//! What a compressed `BINTABLE`'s header says the original image is.

use crate::bitpix::Bitpix;
use crate::compress::ImageCodec;
use crate::data::scaling::Scaling;
use crate::data::shape_product;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

/// The original image a tiled-compression `BINTABLE` encodes, from its `Z`-prefixed
/// header keywords alone: element type, axis lengths, codec, and physical scaling.
#[derive(Debug)]
pub(super) struct ImageLayout {
    pub(super) bitpix: Bitpix,
    pub(super) dims: Vec<usize>,
    pub(super) total: usize,
    pub(super) codec: ImageCodec,
    pub(super) scaling: Scaling,
}

impl ImageLayout {
    pub(super) fn from_header(header: &Header) -> Result<ImageLayout> {
        if header.get_logical("ZIMAGE")? != Some(true) {
            return Err(FitsError::NotCompressedImage);
        }
        let bitpix = Bitpix::from_code(
            header
                .get_integer("ZBITPIX")?
                .ok_or(FitsError::MissingKeyword { name: "ZBITPIX" })?,
        )?;
        let codec = ImageCodec::parse(
            header
                .get_text("ZCMPTYPE")?
                .ok_or(FitsError::MissingKeyword { name: "ZCMPTYPE" })?,
        )?;
        let znaxis = header
            .get_integer("ZNAXIS")?
            .ok_or(FitsError::MissingKeyword { name: "ZNAXIS" })?;
        if !(0..=999).contains(&znaxis) {
            return Err(FitsError::KeywordOutOfRange { name: "ZNAXIS" });
        }
        let dims = read_axes(header, znaxis as usize)?;
        if codec == ImageCodec::Hcompress1 && dims.len() != 2 {
            return Err(FitsError::UnsupportedCompression {
                name: "HCOMPRESS_1 requires a two-dimensional image".to_string(),
            });
        }
        let total = shape_product(&dims)?;
        Ok(ImageLayout {
            bitpix,
            dims,
            total,
            codec,
            scaling: header.scaling()?,
        })
    }

    /// The `ZTILEn` tile extents, one per image axis. `ZTILE1` defaults to a whole
    /// row and the higher axes to one, so an absent tiling is row-at-a-time.
    pub(super) fn tile_shape(&self, header: &Header) -> Result<Vec<usize>> {
        (1..=self.dims.len())
            .map(|i| -> Result<usize> {
                let default = if i == 1 { self.dims[0].max(1) } else { 1 };
                match header.get_integer(key!("ZTILE{i}").as_str())? {
                    Some(value) => usize::try_from(value)
                        .ok()
                        .filter(|&value| value > 0)
                        .ok_or(FitsError::KeywordOutOfRange { name: "ZTILEn" }),
                    None => Ok(default),
                }
            })
            .collect()
    }
}

/// Read the `ZNAXIS1..ZNAXISn` integer axis lengths.
fn read_axes(header: &Header, n: usize) -> Result<Vec<usize>> {
    (1..=n)
        .map(|i| header.required_usize(key!("ZNAXIS{i}").as_str(), "ZNAXISn"))
        .collect()
}
