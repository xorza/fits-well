//! The widened plane a tile is reconstructed in before narrowing to the stored type.

use crate::compress::decode::image_decode_plan::ImageDecodePlan;
use crate::compress::decode::tile_scratch_set::TileScratchSet;
use crate::error::Result;

/// The widened plane a tile decodes into before narrowing to the stored type: `i64`
/// for an integer image, `f64` for a quantized float one. The two differ in how a
/// tile is reconstructed and how its nulls are applied, so selecting the plane by
/// type makes that split once — where the old code re-tested `ZBITPIX.is_float()` at
/// every dispatch site and then asserted the buffer agreed.
pub(super) trait WidePlane: Copy + Send {
    fn decode_tile(
        plan: &ImageDecodePlan<'_>,
        table_row: usize,
        tile_row: usize,
        scratch: &mut TileScratchSet<Self>,
    ) -> Result<()>;
}

impl WidePlane for i64 {
    fn decode_tile(
        plan: &ImageDecodePlan<'_>,
        table_row: usize,
        _tile_row: usize,
        scratch: &mut TileScratchSet<i64>,
    ) -> Result<()> {
        let nelem = scratch.tile.nelem();
        plan.decoder.decode_tile_into(
            plan.sources.cells(table_row)?,
            nelem,
            &mut scratch.values,
            &mut scratch.codecs,
        )?;
        plan.null_mask.apply_integer(
            table_row,
            nelem,
            &mut scratch.values,
            &mut scratch.aux,
            &mut scratch.codecs,
        )
    }
}

impl WidePlane for f64 {
    fn decode_tile(
        plan: &ImageDecodePlan<'_>,
        table_row: usize,
        tile_row: usize,
        scratch: &mut TileScratchSet<f64>,
    ) -> Result<()> {
        let nelem = scratch.tile.nelem();
        plan.decoder.decode_float_tile_into(
            plan.sources.cells(table_row)?,
            nelem,
            plan.quantization.dequant(table_row, tile_row),
            &mut scratch.values,
            &mut scratch.aux,
            &mut scratch.codecs,
        )?;
        plan.null_mask.apply_float(
            table_row,
            nelem,
            &mut scratch.values,
            &mut scratch.aux,
            &mut scratch.codecs,
        )
    }
}
