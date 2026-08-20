//! The output plane for a decompressed image *section*.

use std::ops::Range;

use crate::compress::decode::image_layout::ImageLayout;
use crate::data::shape_product;
use crate::data::validate_image_region;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::BinTable;

/// A requested sub-region of a tiled image: the whole image's [`ImageLayout`] plus
/// the shape, element count, and byte size of the section itself.
#[derive(Debug)]
pub(super) struct ImageRegionLayout {
    pub(super) image: ImageLayout,
    pub(super) shape: Vec<usize>,
    pub(super) total: usize,
    pub(super) nbytes: usize,
}

impl ImageRegionLayout {
    pub(super) fn new(
        header: &Header,
        table: &BinTable,
        tile_rows: &[usize],
        ranges: &[Range<usize>],
    ) -> Result<ImageRegionLayout> {
        let image = ImageLayout::from_header(header)?;
        let shape = validate_image_region(ranges, &image.dims)?;
        if table.metadata().nrows != tile_rows.len() {
            return Err(FitsError::DataSizeMismatch {
                expected: tile_rows.len(),
                got: table.metadata().nrows,
            });
        }
        let total = shape_product(&shape)?;
        let nbytes = total
            .checked_mul(image.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        Ok(ImageRegionLayout {
            image,
            shape,
            total,
            nbytes,
        })
    }
}
