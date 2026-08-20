//! The optional per-tile null-pixel mask (§10.1.2) and how it is applied.

use crate::bitpix::Bitpix;
use crate::compress::ImageCodec;
use crate::compress::decode::ensure_tile_size;
use crate::compress::decode::image_layout::ImageLayout;
use crate::compress::decode::tile_scratch_set::CodecScratch;
use crate::compress::gzip;
use crate::compress::plio;
use crate::compress::rice;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::BinTable;
use crate::table_impl::vla_column::VlaColumn;

/// The optional null-pixel mask and everything applying it needs: the per-tile mask
/// column, the `ZMASKCMP` codec that encodes it, and the `BLANK` value an integer
/// image's masked pixels take (a float image's take `NaN`).
#[derive(Debug, Clone, Copy)]
pub(super) struct NullMask<'a> {
    column: Option<VlaColumn<'a>>,
    codec: Option<ImageCodec>,
    blank: Option<i64>,
}

impl<'a> NullMask<'a> {
    pub(super) fn read(
        header: &Header,
        table: &'a BinTable,
        layout: &ImageLayout,
    ) -> Result<NullMask<'a>> {
        let column = first_column(
            table,
            &[
                "NULL_PIXEL_MASK",
                "NULL_PIXEL_MASK_COLUMN",
                "NULL PIXEL MASK",
            ],
        )?;
        let codec = header
            .get_text("ZMASKCMP")?
            .map(ImageCodec::parse)
            .transpose()?;
        if column.is_some() && codec == Some(ImageCodec::Hcompress1) {
            return Err(FitsError::UnsupportedCompression {
                name: "lossy HCOMPRESS_1 cannot encode a null-pixel mask".to_string(),
            });
        }
        Ok(NullMask {
            column,
            codec,
            blank: layout.scaling.blank,
        })
    }

    /// Replace every masked element of an integer tile with `BLANK`.
    pub(super) fn apply_integer(
        &self,
        table_row: usize,
        tile_elems: usize,
        values: &mut [i64],
        mask: &mut Vec<i64>,
        scratch: &mut CodecScratch,
    ) -> Result<()> {
        if !self.decode_into(table_row, tile_elems, mask, scratch)? {
            return Ok(());
        }
        let blank = self
            .blank
            .ok_or(FitsError::MissingKeyword { name: "BLANK" })?;
        fill_masked(values, mask, blank);
        Ok(())
    }

    /// Replace every masked element of a float tile with `NaN`.
    pub(super) fn apply_float(
        &self,
        table_row: usize,
        tile_elems: usize,
        values: &mut [f64],
        mask: &mut Vec<i64>,
        scratch: &mut CodecScratch,
    ) -> Result<()> {
        if !self.decode_into(table_row, tile_elems, mask, scratch)? {
            return Ok(());
        }
        fill_masked(values, mask, f64::NAN);
        Ok(())
    }

    /// Decode one tile's mask into `out`, reporting whether the tile has one at all.
    fn decode_into(
        &self,
        table_row: usize,
        tile_elems: usize,
        out: &mut Vec<i64>,
        scratch: &mut CodecScratch,
    ) -> Result<bool> {
        let Some(column) = self.column else {
            return Ok(false);
        };
        let cell = column.cell(table_row)?;
        if cell.element_count == 0 {
            return Ok(false);
        }
        let codec = self
            .codec
            .ok_or(FitsError::MissingKeyword { name: "ZMASKCMP" })?;
        match codec {
            ImageCodec::Gzip1 => {
                gzip::gzip_tile_into(cell.bytes, Bitpix::U8, tile_elems, out, &mut scratch.gzip)?
            }
            ImageCodec::Gzip2 => {
                gzip::gzip2_tile_into(cell.bytes, Bitpix::U8, tile_elems, out, &mut scratch.gzip)?
            }
            ImageCodec::Rice1 => rice::rice_decode_into(cell.bytes, tile_elems, 1, 32, out)?,
            ImageCodec::Plio1 => plio::plio_decode_be_into(cell.bytes, tile_elems, out)?,
            ImageCodec::NoCompress => {
                if cell.bytes.len() != tile_elems {
                    return Err(FitsError::DataSizeMismatch {
                        expected: tile_elems,
                        got: cell.bytes.len(),
                    });
                }
                out.clear();
                out.extend(cell.bytes.iter().map(|&value| value as i64));
            }
            ImageCodec::Hcompress1 => unreachable!("rejected while building the decode plan"),
        }
        ensure_tile_size(tile_elems, out.len())?;
        if out.iter().any(|&value| !matches!(value, 0 | 1)) {
            return Err(FitsError::UnsupportedCompression {
                name: "null-pixel mask contains a value other than zero or one".to_string(),
            });
        }
        Ok(true)
    }
}

/// The first of `names` the table carries as a variable-length column — writers
/// disagree on the mask column's spelling, so all three are accepted.
fn first_column<'a>(table: &'a BinTable, names: &[&str]) -> Result<Option<VlaColumn<'a>>> {
    for &name in names {
        if let Some(column) = table.optional_vla_column(name)? {
            return Ok(Some(column));
        }
    }
    Ok(None)
}

/// Overwrite every element the mask marks (§10.1.2: a mask value of 1 is a null)
/// with `fill` — `BLANK` for an integer plane, `NaN` for a float one.
fn fill_masked<T: Copy>(values: &mut [T], mask: &[i64], fill: T) {
    for (value, &masked) in values.iter_mut().zip(mask) {
        if masked == 1 {
            *value = fill;
        }
    }
}
