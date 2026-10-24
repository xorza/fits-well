//! The plane a tile is reconstructed in before it lands in the stored type.

use crate::compress::decode::float_quantization::FloatEncoding;
use crate::compress::decode::float_sample::FloatSample;
use crate::compress::decode::null_mask::NullMask;
use crate::compress::decode::tile_decoder::{Overshoot, TileDecoder};
use crate::compress::decode::tile_scratch_set::TileScratchSet;
use crate::compress::decode::tile_sources::TileSource;
use crate::error::Result;

/// What one tile's decode reads: the image's decoder and nulls, the tile's source cell
/// and table row, and, for a float image, how its values come back.
#[derive(Debug, Clone, Copy)]
pub(super) struct TileJob<'a> {
    pub(super) decoder: &'a TileDecoder,
    pub(super) nulls: &'a NullMask<'a>,
    pub(super) source: TileSource<'a>,
    pub(super) table_row: usize,
    /// `Some` exactly for a float image.
    pub(super) encoding: Option<FloatEncoding>,
}

/// The plane a tile decodes into: `i64` for an integer image, whose values fit the
/// stored type afterwards, and the stored type itself for a float one. The two differ in
/// how a tile is reconstructed and how its nulls are applied, so selecting the plane by
/// type makes that split once, rather than re-testing `ZBITPIX.is_float()` at every
/// dispatch site.
pub(super) trait WidePlane: Copy + Send {
    /// Decode the tile of `job` into `scratch`, and say what a value outside the stored
    /// type's range means for it.
    fn decode_tile(job: TileJob<'_>, scratch: &mut TileScratchSet<Self>) -> Result<Overshoot>;
}

impl WidePlane for i64 {
    fn decode_tile(job: TileJob<'_>, scratch: &mut TileScratchSet<Self>) -> Result<Overshoot> {
        let overshoot = job.decoder.decode_tile_into(job.source, scratch)?;
        job.nulls.apply_integer(job.table_row, scratch)?;
        Ok(overshoot)
    }
}

impl WidePlane for f32 {
    fn decode_tile(job: TileJob<'_>, scratch: &mut TileScratchSet<Self>) -> Result<Overshoot> {
        decode_float_tile(job, scratch)
    }
}

impl WidePlane for f64 {
    fn decode_tile(job: TileJob<'_>, scratch: &mut TileScratchSet<Self>) -> Result<Overshoot> {
        decode_float_tile(job, scratch)
    }
}

/// A float tile never overshoots: a float takes any value its stored type holds.
fn decode_float_tile<F: FloatSample>(
    job: TileJob<'_>,
    scratch: &mut TileScratchSet<F>,
) -> Result<Overshoot> {
    let encoding = job
        .encoding
        .expect("only a float image decodes in a float plane");
    job.decoder
        .decode_float_tile_into(job.source, encoding, scratch)?;
    job.nulls.apply_float(job.table_row, scratch)?;
    Ok(Overshoot::Refuse)
}
