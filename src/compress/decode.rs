//! Tiled-image decompression (§10.1).
//!
//! Reassemble the per-tile codec output (`COMPRESSED_DATA`, with the
//! `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks) into the full [`Image`],
//! de-quantizing float tiles (`ZSCALE`/`ZZERO`) on the way. The per-codec work lives
//! in the sibling [`gzip`]/[`rice`]/
//! [`plio`]/[`hcompress`] modules;
//! this drives the tile geometry, the
//! fallback-column resolution, and the narrow-and-scatter into the output plane.

use crate::compress;
use crate::compress::convert;
use crate::compress::geometry::TileGeometry;
use crate::compress::geometry::TileScratch;
#[cfg(feature = "parallel")]
use crate::compress::map_tiles;
use crate::compress::{DitherMethod, ImageCodec};
use crate::compress::{gzip, hcompress, plio, quantize, rice};

use crate::allocation;
use crate::bitpix::Bitpix;
use crate::data::BorrowedImage;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::data::shape_product;
use crate::data::validate_image_region;
use crate::data::view_words;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table_impl::BinTable;
use crate::table_impl::ColumnData;
use crate::table_impl::VlaCell;
use crate::table_impl::VlaColumn;
use crate::words;
use std::ops::Range;

#[derive(Debug)]
struct ImageLayout {
    bitpix: Bitpix,
    dims: Vec<usize>,
    total: usize,
    codec: ImageCodec,
    scaling: Scaling,
}

impl ImageLayout {
    fn from_header(header: &Header) -> Result<ImageLayout> {
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
}

/// Everything a tiled image's decode needs that the header and the table's metadata
/// columns determine once, up front — grouped by the concern each part serves rather
/// than held as one flat bag.
#[derive(Debug)]
struct ImageDecodePlan<'a> {
    geometry: TileGeometry,
    context: DecodeCtx,
    sources: TileSources<'a>,
    null_mask: NullMask<'a>,
    quantization: FloatQuantization,
}

impl<'a> ImageDecodePlan<'a> {
    fn new(
        header: &Header,
        table: &'a BinTable,
        layout: &ImageLayout,
    ) -> Result<ImageDecodePlan<'a>> {
        let tiles = read_tile_shape(header, &layout.dims)?;
        let rice = rice::rice_params(header)?;
        let is_float = layout.bitpix.is_float();
        let int_bitpix = if is_float && layout.codec == ImageCodec::Rice1 {
            convert::bytepix_to_bitpix(rice.bytepix)
        } else if is_float {
            Bitpix::I32
        } else {
            layout.bitpix
        };
        Ok(ImageDecodePlan {
            geometry: TileGeometry::new(&layout.dims, &tiles),
            context: DecodeCtx {
                codec: layout.codec,
                zbitpix: layout.bitpix,
                int_bitpix,
                params: CodecParams {
                    blocksize: rice.blocksize,
                    bytepix: rice.bytepix,
                    smooth: hcompress_smooth(header)?,
                },
            },
            sources: TileSources::read(table)?,
            null_mask: NullMask::read(header, table, layout)?,
            quantization: FloatQuantization::read(header, table, is_float)?,
        })
    }
}

/// The three per-tile source columns (§10.1.3): the primary `COMPRESSED_DATA` and
/// the `GZIP_COMPRESSED_DATA` / `UNCOMPRESSED_DATA` fallbacks. Any of them may be
/// absent, and each tile picks the first with a non-empty cell.
#[derive(Debug, Clone, Copy)]
struct TileSources<'a> {
    primary: Option<VlaColumn<'a>>,
    gzip_fallback: Option<VlaColumn<'a>>,
    uncompressed: Option<VlaColumn<'a>>,
}

impl<'a> TileSources<'a> {
    fn read(table: &'a BinTable) -> Result<TileSources<'a>> {
        Ok(TileSources {
            primary: read_tiles(table, "COMPRESSED_DATA")?,
            gzip_fallback: read_tiles(table, "GZIP_COMPRESSED_DATA")?,
            uncompressed: read_tiles(table, "UNCOMPRESSED_DATA")?,
        })
    }

    /// The three candidate cells for one table row.
    fn cells(&self, row: usize) -> Result<TileCells<'a>> {
        Ok(TileCells {
            primary: self.primary.map(|column| column.cell(row)).transpose()?,
            gzip: self
                .gzip_fallback
                .map(|column| column.cell(row))
                .transpose()?,
            uncompressed: self
                .uncompressed
                .map(|column| column.cell(row))
                .transpose()?,
        })
    }
}

/// The optional null-pixel mask and everything applying it needs: the per-tile mask
/// column, the `ZMASKCMP` codec that encodes it, and the `BLANK` value an integer
/// image's masked pixels take (a float image's take `NaN`).
#[derive(Debug, Clone, Copy)]
struct NullMask<'a> {
    column: Option<VlaColumn<'a>>,
    codec: Option<ImageCodec>,
    blank: Option<i64>,
}

impl<'a> NullMask<'a> {
    fn read(header: &Header, table: &'a BinTable, layout: &ImageLayout) -> Result<NullMask<'a>> {
        let column = read_first_tiles(
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
}

/// The float-quantization inputs (§10.2): the dither method and seed, and the
/// per-tile `ZSCALE`/`ZZERO`/`ZBLANK` metadata columns. An integer image parses
/// these too but never consults them.
#[derive(Debug)]
struct FloatQuantization {
    method: DitherMethod,
    zdither0: i64,
    zblank_keyword: Option<i64>,
    zblank_column: Option<Vec<i64>>,
    zscale: Option<Vec<f64>>,
    zzero: Option<Vec<f64>>,
}

impl FloatQuantization {
    fn read(header: &Header, table: &BinTable, is_float: bool) -> Result<FloatQuantization> {
        let quantiz = header.get_text("ZQUANTIZ")?.unwrap_or("NO_DITHER");
        let method = match DitherMethod::parse(quantiz) {
            Some(method) => method,
            // A float image's samples cannot be reconstructed without reproducing its
            // dither exactly; an integer image ignores `ZQUANTIZ` altogether.
            None if is_float => {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("float quantization {quantiz}"),
                });
            }
            None => DitherMethod::None,
        };
        let zdither0 = header.get_integer("ZDITHER0")?.unwrap_or(1);
        if !(1..=10_000).contains(&zdither0) {
            return Err(FitsError::KeywordOutOfRange { name: "ZDITHER0" });
        }
        Ok(FloatQuantization {
            method,
            zdither0,
            zblank_keyword: header.get_integer("ZBLANK")?,
            zblank_column: read_i64_column(table, "ZBLANK")?,
            zscale: read_f64_column(table, "ZSCALE")?,
            zzero: read_f64_column(table, "ZZERO")?,
        })
    }

    /// The dequantization parameters for one tile.
    fn dequant(&self, table_row: usize, tile_row: usize) -> Dequant {
        Dequant {
            scale: column_at(&self.zscale, table_row).unwrap_or(1.0),
            zero: column_at(&self.zzero, table_row).unwrap_or(0.0),
            method: self.method,
            irow: tile_row as i64 + self.zdither0,
            zblank: column_at(&self.zblank_column, table_row).or(self.zblank_keyword),
        }
    }
}

/// Reusable per-worker buffers for one tile: its geometry, the decoded values in
/// their widened plane, the codecs' auxiliary integer buffer (quantized samples for a
/// float plane, the null mask for an integer one), and the codec workspaces.
#[derive(Debug)]
struct TileScratchSet<W> {
    tile: TileScratch,
    values: Vec<W>,
    aux: Vec<i64>,
    codecs: CodecScratch,
}

// Hand-written rather than derived: `Vec<W>` is `Default` for every `W`, so the
// derive's `W: Default` bound would be a fiction.
impl<W> Default for TileScratchSet<W> {
    fn default() -> TileScratchSet<W> {
        TileScratchSet {
            tile: TileScratch::default(),
            values: Vec::new(),
            aux: Vec::new(),
            codecs: CodecScratch::default(),
        }
    }
}

impl<W: WidePlane> TileScratchSet<W> {
    /// Reconstruct one tile into this scratch: resolve its geometry, decode its
    /// samples in the wide plane, and check the count against the header's tiling.
    ///
    /// `tile_row` indexes the image's tile grid (it drives the dither sequence and
    /// the scatter); `table_row` indexes the compressed table. They coincide for a
    /// whole-image decode and diverge for a section, which reads only the rows its
    /// region intersects.
    fn decode(
        &mut self,
        plan: &ImageDecodePlan<'_>,
        table_row: usize,
        tile_row: usize,
    ) -> Result<()> {
        plan.geometry.tile_into(tile_row, &mut self.tile);
        W::decode_tile(plan, table_row, tile_row, self)?;
        ensure_tile_size(self.tile.nelem(), self.values.len())
    }
}

/// The widened plane a tile decodes into before narrowing to the stored type: `i64`
/// for an integer image, `f64` for a quantized float one. The two differ in how a
/// tile is reconstructed and how its nulls are applied, so selecting the plane by
/// type makes that split once — where the old code re-tested `ZBITPIX.is_float()` at
/// every dispatch site and then asserted the buffer agreed.
trait WidePlane: Copy + Send {
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
        decode_one_tile_into(
            &plan.context,
            plan.sources.cells(table_row)?,
            nelem,
            &mut scratch.values,
            &mut scratch.codecs,
        )?;
        apply_integer_null_mask(
            plan,
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
        decode_float_tile_into(
            &plan.context,
            plan.sources.cells(table_row)?,
            nelem,
            plan.quantization.dequant(table_row, tile_row),
            &mut scratch.values,
            &mut scratch.aux,
            &mut scratch.codecs,
        )?;
        apply_float_null_mask(
            plan,
            table_row,
            nelem,
            &mut scratch.values,
            &mut scratch.aux,
            &mut scratch.codecs,
        )
    }
}

/// A stored sample type the decoder scatters into, paired with the plane its tiles
/// decode in. Narrowing happens only as values land in the output plane.
trait DecodeSample: Copy + Send + Sync {
    type Wide: WidePlane;
    fn narrow(wide: Self::Wide) -> Self;
}

impl DecodeSample for u8 {
    type Wide = i64;
    fn narrow(wide: i64) -> u8 {
        wide as u8
    }
}

impl DecodeSample for i16 {
    type Wide = i64;
    fn narrow(wide: i64) -> i16 {
        wide as i16
    }
}

impl DecodeSample for i32 {
    type Wide = i64;
    fn narrow(wide: i64) -> i32 {
        wide as i32
    }
}

impl DecodeSample for i64 {
    type Wide = i64;
    fn narrow(wide: i64) -> i64 {
        wide
    }
}

impl DecodeSample for f32 {
    type Wide = f64;
    fn narrow(wide: f64) -> f32 {
        wide as f32
    }
}

impl DecodeSample for f64 {
    type Wide = f64;
    fn narrow(wide: f64) -> f64 {
        wide
    }
}

#[derive(Debug)]
enum DecodeBuffer<'a> {
    U8(&'a mut [u8]),
    I16(&'a mut [i16]),
    I32(&'a mut [i32]),
    I64(&'a mut [i64]),
    F32(&'a mut [f32]),
    F64(&'a mut [f64]),
}

impl<'a> DecodeBuffer<'a> {
    /// Whether this buffer holds the float plane. Every constructor sizes the buffer
    /// from `ZBITPIX`, so this must agree with the layout the plan was built from.
    fn is_float(&self) -> bool {
        matches!(self, DecodeBuffer::F32(_) | DecodeBuffer::F64(_))
    }

    fn from_samples(samples: &'a mut ImageData) -> DecodeBuffer<'a> {
        match samples {
            ImageData::U8(values) => DecodeBuffer::U8(values),
            ImageData::I16(values) => DecodeBuffer::I16(values),
            ImageData::I32(values) => DecodeBuffer::I32(values),
            ImageData::I64(values) => DecodeBuffer::I64(values),
            ImageData::F32(values) => DecodeBuffer::F32(values),
            ImageData::F64(values) => DecodeBuffer::F64(values),
        }
    }

    fn from_words(words: &'a mut [u64], bitpix: Bitpix, count: usize) -> DecodeBuffer<'a> {
        // SAFETY: the callers resize `words` to `count` zeroed samples before this, and
        // zero is a valid value for every sample type, so the view covers initialized
        // storage. Alignment and bit-pattern validity are `words::samples_mut`'s to
        // uphold, not this caller's.
        unsafe {
            match bitpix {
                Bitpix::U8 => DecodeBuffer::U8(words::samples_mut(words, count)),
                Bitpix::I16 => DecodeBuffer::I16(words::samples_mut(words, count)),
                Bitpix::I32 => DecodeBuffer::I32(words::samples_mut(words, count)),
                Bitpix::I64 => DecodeBuffer::I64(words::samples_mut(words, count)),
                Bitpix::F32 => DecodeBuffer::F32(words::samples_mut(words, count)),
                Bitpix::F64 => DecodeBuffer::F64(words::samples_mut(words, count)),
            }
        }
    }
}

/// Decompress a tiled-image `BINTABLE` into the full [`Image`] it encodes.
pub(crate) fn decompress_image(header: &Header, table: &BinTable) -> Result<Image> {
    let layout = ImageLayout::from_header(header)?;
    let mut samples = convert::zeroed_samples(layout.bitpix, layout.total)?;
    if layout.total != 0 {
        decode_image_into(
            header,
            table,
            &layout,
            DecodeBuffer::from_samples(&mut samples),
        )?;
    }
    Image::new_scaled(layout.dims, samples, layout.scaling)
}

pub(crate) fn decompress_image_into_words<'a>(
    header: &Header,
    table: &BinTable,
    words: &'a mut Vec<u64>,
) -> Result<BorrowedImage<'a>> {
    let layout = ImageLayout::from_header(header)?;
    let nbytes = layout
        .total
        .checked_mul(layout.bitpix.elem_size())
        .ok_or(FitsError::DataUnitOverflow)?;
    allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
    if layout.total != 0 {
        let output = DecodeBuffer::from_words(words, layout.bitpix, layout.total);
        decode_image_into(header, table, &layout, output)?;
    }
    Ok(BorrowedImage {
        shape: layout.dims,
        scaling: layout.scaling,
        samples: view_words(words, layout.bitpix, nbytes),
    })
}

/// Original compressed-table row indices for tiles intersecting `ranges`.
pub(crate) fn compressed_image_tile_rows(
    header: &Header,
    ranges: &[Range<usize>],
) -> Result<Vec<usize>> {
    let layout = ImageLayout::from_header(header)?;
    validate_image_region(ranges, &layout.dims)?;
    if ranges.iter().any(Range::is_empty) || layout.dims.is_empty() {
        return Ok(Vec::new());
    }
    let tiles = read_tile_shape(header, &layout.dims)?;
    let counts: Vec<usize> = layout
        .dims
        .iter()
        .zip(&tiles)
        .map(|(&dim, &tile)| dim.div_ceil(tile))
        .collect();
    let starts: Vec<usize> = ranges
        .iter()
        .zip(&tiles)
        .map(|(range, &tile)| range.start / tile)
        .collect();
    let ends: Vec<usize> = ranges
        .iter()
        .zip(&tiles)
        .map(|(range, &tile)| (range.end - 1) / tile + 1)
        .collect();
    let tile_count = starts
        .iter()
        .zip(&ends)
        .try_fold(1usize, |count, (&start, &end)| {
            count.checked_mul(end - start)
        })
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut coordinates = starts.clone();
    let mut selected = Vec::with_capacity(tile_count);
    for _ in 0..tile_count {
        let mut stride = 1usize;
        let mut index = 0usize;
        for axis in 0..coordinates.len() {
            index = index
                .checked_add(
                    coordinates[axis]
                        .checked_mul(stride)
                        .ok_or(FitsError::DataUnitOverflow)?,
                )
                .ok_or(FitsError::DataUnitOverflow)?;
            stride = stride
                .checked_mul(counts[axis])
                .ok_or(FitsError::DataUnitOverflow)?;
        }
        selected.push(index);
        for axis in 0..coordinates.len() {
            coordinates[axis] += 1;
            if coordinates[axis] < ends[axis] {
                break;
            }
            coordinates[axis] = starts[axis];
        }
    }
    Ok(selected)
}

/// Decompress only the compact table rows in `tile_rows`, scattering their
/// intersections into one scratch-backed image section.
pub(crate) fn decompress_image_section_into_words<'a>(
    header: &Header,
    table: &BinTable,
    tile_rows: &[usize],
    ranges: &[Range<usize>],
    words: &'a mut Vec<u64>,
) -> Result<BorrowedImage<'a>> {
    let region = ImageRegionLayout::new(header, table, tile_rows, ranges)?;
    allocation::try_resize(words, region.nbytes.div_ceil(8), 0)?;
    if region.total != 0 {
        let output = DecodeBuffer::from_words(words, region.image.bitpix, region.total);
        decode_image_section_into(header, table, tile_rows, ranges, &region, output)?;
    }
    Ok(BorrowedImage {
        shape: region.shape,
        scaling: region.image.scaling,
        samples: view_words(words, region.image.bitpix, region.nbytes),
    })
}

pub(crate) fn decompress_image_section(
    header: &Header,
    table: &BinTable,
    tile_rows: &[usize],
    ranges: &[Range<usize>],
) -> Result<Image> {
    let region = ImageRegionLayout::new(header, table, tile_rows, ranges)?;
    let mut samples = convert::zeroed_samples(region.image.bitpix, region.total)?;
    if region.total != 0 {
        let output = DecodeBuffer::from_samples(&mut samples);
        decode_image_section_into(header, table, tile_rows, ranges, &region, output)?;
    }
    Image::new_scaled(region.shape, samples, region.image.scaling)
}

#[derive(Debug)]
struct ImageRegionLayout {
    image: ImageLayout,
    shape: Vec<usize>,
    total: usize,
    nbytes: usize,
}

impl ImageRegionLayout {
    fn new(
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

/// Build the decode plan for `layout` and check it against the buffer the caller
/// sized from the same layout.
///
/// The buffer's plane and `ZBITPIX` cannot disagree — every caller derives both from
/// one [`ImageLayout`] — but the two dispatch paths below select their tile decoder
/// from the plane and their scatter from the buffer, so the pairing is worth stating
/// once rather than at each of them.
fn plan_for<'a>(
    header: &Header,
    table: &'a BinTable,
    layout: &ImageLayout,
    output: &DecodeBuffer<'_>,
) -> Result<ImageDecodePlan<'a>> {
    let plan = ImageDecodePlan::new(header, table, layout)?;
    debug_assert_eq!(
        plan.context.zbitpix.is_float(),
        output.is_float(),
        "the sample buffer is sized from ZBITPIX, so its plane must match"
    );
    Ok(plan)
}

fn decode_image_section_into(
    header: &Header,
    table: &BinTable,
    tile_rows: &[usize],
    ranges: &[Range<usize>],
    region: &ImageRegionLayout,
    output: DecodeBuffer<'_>,
) -> Result<()> {
    debug_assert_ne!(region.total, 0);
    let plan = plan_for(header, table, &region.image, &output)?;
    let shape = &region.shape;
    match output {
        DecodeBuffer::U8(out) => run_decode_region(&plan, ranges, shape, tile_rows, out),
        DecodeBuffer::I16(out) => run_decode_region(&plan, ranges, shape, tile_rows, out),
        DecodeBuffer::I32(out) => run_decode_region(&plan, ranges, shape, tile_rows, out),
        DecodeBuffer::I64(out) => run_decode_region(&plan, ranges, shape, tile_rows, out),
        DecodeBuffer::F32(out) => run_decode_region(&plan, ranges, shape, tile_rows, out),
        DecodeBuffer::F64(out) => run_decode_region(&plan, ranges, shape, tile_rows, out),
    }
}

fn decode_image_into(
    header: &Header,
    table: &BinTable,
    layout: &ImageLayout,
    output: DecodeBuffer<'_>,
) -> Result<()> {
    let plan = plan_for(header, table, layout, &output)?;
    match output {
        DecodeBuffer::U8(out) => run_decode_scatter(&plan, out),
        DecodeBuffer::I16(out) => run_decode_scatter(&plan, out),
        DecodeBuffer::I32(out) => run_decode_scatter(&plan, out),
        DecodeBuffer::I64(out) => run_decode_scatter(&plan, out),
        DecodeBuffer::F32(out) => run_decode_scatter(&plan, out),
        DecodeBuffer::F64(out) => run_decode_scatter(&plan, out),
    }
}

/// Decode every tile and scatter it into the full image plane.
///
/// The two builds share the per-tile decode ([`TileScratchSet::decode`]) but not the
/// hand-off, and deliberately so: a parallel worker cannot scatter into `out`
/// directly, so it must hand back an owned buffer, and narrowing *before* that
/// hand-off is what bounds the memory a wave retains — [`decode_wave_tile_count`]
/// sizes the wave from `size_of::<D>()`, not from the wide plane. The serial build
/// has no hand-off to pay for, so it narrows straight into `out` and allocates
/// nothing per tile.
fn run_decode_scatter<D: DecodeSample>(plan: &ImageDecodePlan<'_>, out: &mut [D]) -> Result<()> {
    let geom = &plan.geometry;
    #[cfg(feature = "parallel")]
    {
        let wave_len = decode_wave_tile_count::<D>(geom);
        let mut scatter = TileScratch::default();
        for wave_start in (0..geom.ntiles()).step_by(wave_len) {
            let count = wave_len.min(geom.ntiles() - wave_start);
            let decoded = map_tiles(
                count,
                TileScratchSet::<D::Wide>::default,
                |scratch, offset| -> Result<Vec<D>> {
                    let tile = wave_start + offset;
                    scratch.decode(plan, tile, tile)?;
                    Ok(scratch.values.iter().copied().map(D::narrow).collect())
                },
            )?;
            for (offset, values) in decoded.iter().enumerate() {
                geom.tile_into(wave_start + offset, &mut scatter);
                // Already narrowed, in the worker.
                scatter_rows(
                    out,
                    &scatter.row_bases,
                    scatter.row_len,
                    values,
                    &std::convert::identity,
                );
            }
        }
        Ok(())
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut scratch = TileScratchSet::<D::Wide>::default();
        for tile in 0..geom.ntiles() {
            scratch.decode(plan, tile, tile)?;
            scatter_rows(
                out,
                &scratch.tile.row_bases,
                scratch.tile.row_len,
                &scratch.values,
                &D::narrow,
            );
        }
        Ok(())
    }
}

#[cfg(feature = "parallel")]
pub(super) fn decode_wave_tile_count<D>(geom: &TileGeometry) -> usize {
    const DECODE_WAVE_BYTES: usize = 4 * 1024 * 1024;

    let payload_bytes = geom
        .max_tile_elements()
        .saturating_mul(std::mem::size_of::<D>());
    let retained_bytes = payload_bytes
        .saturating_add(std::mem::size_of::<Vec<D>>())
        .max(1);
    (DECODE_WAVE_BYTES / retained_bytes).max(1)
}

/// Decode only the tiles intersecting a requested region, scattering each tile's
/// intersection into the section plane. Serial: the tiles are already a sparse subset.
fn run_decode_region<D: DecodeSample>(
    plan: &ImageDecodePlan<'_>,
    ranges: &[Range<usize>],
    selected_shape: &[usize],
    tile_rows: &[usize],
    out: &mut [D],
) -> Result<()> {
    let mut scratch = TileScratchSet::<D::Wide>::default();
    for (table_row, &tile_row) in tile_rows.iter().enumerate() {
        scratch.decode(plan, table_row, tile_row)?;
        scatter_region_tile(
            &scratch.tile,
            ranges,
            selected_shape,
            &scratch.values,
            out,
            &D::narrow,
        );
    }
    Ok(())
}

fn scatter_region_tile<S: Copy, D: Copy>(
    tile: &TileScratch,
    ranges: &[Range<usize>],
    selected_shape: &[usize],
    values: &[S],
    out: &mut [D],
    convert: &impl Fn(S) -> D,
) {
    let x_start = tile.origin[0].max(ranges[0].start);
    let x_end = (tile.origin[0] + tile.tdims[0]).min(ranges[0].end);
    if x_start >= x_end {
        return;
    }
    let width = x_end - x_start;
    for row in 0..tile.row_bases.len() {
        let mut remainder = row;
        let mut output_base = 0usize;
        let mut output_stride = selected_shape[0];
        let mut selected = true;
        for axis in 1..tile.tdims.len() {
            let local = remainder % tile.tdims[axis];
            remainder /= tile.tdims[axis];
            let coordinate = tile.origin[axis] + local;
            if !ranges[axis].contains(&coordinate) {
                selected = false;
                break;
            }
            output_base += (coordinate - ranges[axis].start) * output_stride;
            output_stride *= selected_shape[axis];
        }
        if !selected {
            continue;
        }
        let source = row * tile.row_len + (x_start - tile.origin[0]);
        let destination = output_base + (x_start - ranges[0].start);
        for (slot, &value) in out[destination..destination + width]
            .iter_mut()
            .zip(&values[source..source + width])
        {
            *slot = convert(value);
        }
    }
}

fn scatter_rows<S: Copy, D>(
    out: &mut [D],
    row_bases: &[usize],
    row_len: usize,
    vals: &[S],
    convert: &impl Fn(S) -> D,
) {
    let mut off = 0;
    for &base in row_bases {
        for (d, &v) in out[base..base + row_len]
            .iter_mut()
            .zip(&vals[off..off + row_len])
        {
            *d = convert(v);
        }
        off += row_len;
    }
}

fn read_tile_shape(header: &Header, dims: &[usize]) -> Result<Vec<usize>> {
    (1..=dims.len())
        .map(|i| -> Result<usize> {
            let default = if i == 1 { dims[0].max(1) } else { 1 };
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

fn read_tiles<'a>(table: &'a BinTable, name: &str) -> Result<Option<VlaColumn<'a>>> {
    match table.column_index(name) {
        Some(c) => Ok(Some(table.column_by_idx(c)?.vla_column()?)),
        None => Ok(None),
    }
}

fn read_first_tiles<'a>(table: &'a BinTable, names: &[&str]) -> Result<Option<VlaColumn<'a>>> {
    for &name in names {
        if let Some(column) = read_tiles(table, name)? {
            return Ok(Some(column));
        }
    }
    Ok(None)
}

/// Decode a named per-tile metadata column, or `None` when the table does not carry
/// it — every such column is optional, so absence is not an error.
fn read_tile_metadata(table: &BinTable, name: &str) -> Result<Option<ColumnData>> {
    match table.column_index(name) {
        Some(index) => Ok(Some(table.column_by_idx(index)?.raw()?)),
        None => Ok(None),
    }
}

/// Read a per-tile `f64` column (e.g. `ZSCALE`/`ZZERO`), or `None` if absent.
fn read_f64_column(table: &BinTable, name: &str) -> Result<Option<Vec<f64>>> {
    let Some(data) = read_tile_metadata(table, name)? else {
        return Ok(None);
    };
    match data {
        ColumnData::F64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "f64 column",
        }),
    }
}

/// Read a per-tile integer column (e.g. a `ZBLANK` column), widening any integer
/// `TFORM` to `i64`, or `None` if absent.
fn read_i64_column(table: &BinTable, name: &str) -> Result<Option<Vec<i64>>> {
    let Some(data) = read_tile_metadata(table, name)? else {
        return Ok(None);
    };
    match data {
        ColumnData::Bytes(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I16(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I32(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "integer column",
        }),
    }
}

fn column_at<T: Copy>(col: &Option<Vec<T>>, t: usize) -> Option<T> {
    col.as_ref().and_then(|v| v.get(t).copied())
}

/// One tile's three candidate source cells, read from [`TileSources`]. The tile is
/// decoded from the first non-empty one: the primary `COMPRESSED_DATA` (via
/// `ZCMPTYPE`), else gzip'd `GZIP_COMPRESSED_DATA`, else raw `UNCOMPRESSED_DATA`.
#[derive(Debug, Clone, Copy)]
struct TileCells<'a> {
    primary: Option<VlaCell<'a>>,
    gzip: Option<VlaCell<'a>>,
    uncompressed: Option<VlaCell<'a>>,
}

/// The resolved source for one tile — which non-empty column holds its bytes.
#[derive(Debug)]
enum TileSource<'a> {
    Compressed(VlaCell<'a>),
    Gzip(VlaCell<'a>),
    Uncompressed(VlaCell<'a>),
}

impl<'a> TileCells<'a> {
    /// Pick the first non-empty source: primary `COMPRESSED_DATA`, then the
    /// gzip and uncompressed fallbacks; error if every column's cell is empty.
    fn resolve(self) -> Result<TileSource<'a>> {
        if let Some(c) = self.primary.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Compressed(c))
        } else if let Some(c) = self.gzip.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Gzip(c))
        } else if let Some(c) = self.uncompressed.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Uncompressed(c))
        } else {
            Err(FitsError::UnsupportedCompression {
                name: "empty tile (no compressed or uncompressed data)".to_string(),
            })
        }
    }
}

/// The codec knobs from `ZNAMEi`/`ZVALi`: Rice block size & pixel width, and the
/// HCOMPRESS `SMOOTH` flag.
#[derive(Debug, Clone, Copy)]
struct CodecParams {
    blocksize: usize,
    bytepix: usize,
    smooth: bool,
}

#[derive(Debug, Default)]
struct CodecScratch {
    gzip: gzip::GzipScratch,
    hcompress: hcompress::HcompressScratch,
}

/// Per-tile float dequantization parameters (§10.2): `physical = zero + scale·I`,
/// the dither method/seed, and the integer null sentinel.
#[derive(Debug)]
struct Dequant {
    scale: f64,
    zero: f64,
    method: DitherMethod,
    irow: i64,
    zblank: Option<i64>,
}

/// The decode parameters constant across all of a tiled image's tiles: the codec,
/// the stored/quantized integer bitpix (and float `ZBITPIX`), and the codec knobs.
/// Bundled so the per-tile decode helpers take one context rather than a long
/// parameter list.
#[derive(Debug)]
struct DecodeCtx {
    codec: ImageCodec,
    zbitpix: Bitpix,
    int_bitpix: Bitpix,
    params: CodecParams,
}

fn decode_null_mask_into(
    plan: &ImageDecodePlan<'_>,
    table_row: usize,
    tile_elems: usize,
    out: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<bool> {
    let Some(column) = plan.null_mask.column else {
        return Ok(false);
    };
    let cell = column.cell(table_row)?;
    if cell.element_count == 0 {
        return Ok(false);
    }
    let codec = plan
        .null_mask
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

fn apply_integer_null_mask(
    plan: &ImageDecodePlan<'_>,
    table_row: usize,
    tile_elems: usize,
    values: &mut [i64],
    mask: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<()> {
    if !decode_null_mask_into(plan, table_row, tile_elems, mask, scratch)? {
        return Ok(());
    }
    let blank = plan
        .null_mask
        .blank
        .ok_or(FitsError::MissingKeyword { name: "BLANK" })?;
    fill_masked(values, mask, blank);
    Ok(())
}

fn apply_float_null_mask(
    plan: &ImageDecodePlan<'_>,
    table_row: usize,
    tile_elems: usize,
    values: &mut [f64],
    mask: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<()> {
    if !decode_null_mask_into(plan, table_row, tile_elems, mask, scratch)? {
        return Ok(());
    }
    fill_masked(values, mask, f64::NAN);
    Ok(())
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

fn decode_one_tile_into(
    ctx: &DecodeCtx,
    cols: TileCells,
    tile_elems: usize,
    out: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => decode_tile_cell_into(ctx, cell, tile_elems, out, scratch),
        TileSource::Gzip(cell) => gzip::gzip_tile_into(
            convert::byte_cell(cell)?,
            ctx.int_bitpix,
            tile_elems,
            out,
            &mut scratch.gzip,
        ),
        TileSource::Uncompressed(cell) => {
            convert::cell_to_i64_into(cell, out);
            Ok(())
        }
    }
}

/// Decode one tile of a *float* image into `out`. A primary `COMPRESSED_DATA` cell
/// holds quantized integers (decoded into the reused `ints` buffer, then dequantized
/// as `scale·int + zero`); otherwise the `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA`
/// fallbacks hold the raw float values.
fn decode_float_tile_into(
    ctx: &DecodeCtx,
    cols: TileCells,
    tile_elems: usize,
    dq: Dequant,
    out: &mut Vec<f64>,
    ints: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => {
            // The primary stream holds quantized integers for every float-image codec.
            decode_tile_cell_into(ctx, cell, tile_elems, ints, scratch)?;
            quantize::dequantize_into(ints, dq.scale, dq.zero, dq.method, dq.irow, dq.zblank, out);
            Ok(())
        }
        TileSource::Gzip(cell) => {
            // Raw floats, bounded at the tile's known byte size (`tile_elems` floats).
            let max = tile_elems.saturating_mul(ctx.zbitpix.elem_size());
            gzip::gunzip_into(convert::byte_cell(cell)?, max, &mut scratch.gzip.bytes)?;
            convert::be_floats_into(&scratch.gzip.bytes, ctx.zbitpix, out);
            Ok(())
        }
        TileSource::Uncompressed(cell) => {
            convert::cell_to_f64_into(cell, ctx.zbitpix, out);
            Ok(())
        }
    }
}

/// Decode one tile's primary `COMPRESSED_DATA` cell into `tile_elems` integer values
/// in `out`, per `ZCMPTYPE`. The cell is a byte array except for `PLIO_1` (i16).
fn decode_tile_cell_into(
    ctx: &DecodeCtx,
    cell: VlaCell<'_>,
    tile_elems: usize,
    out: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<()> {
    let params = ctx.params;
    match ctx.codec {
        ImageCodec::Gzip1 => gzip::gzip_tile_into(
            convert::byte_cell(cell)?,
            ctx.int_bitpix,
            tile_elems,
            out,
            &mut scratch.gzip,
        ),
        ImageCodec::Gzip2 => gzip::gzip2_tile_into(
            convert::byte_cell(cell)?,
            ctx.int_bitpix,
            tile_elems,
            out,
            &mut scratch.gzip,
        ),
        ImageCodec::Rice1 => {
            if !matches!(params.bytepix, 1 | 2 | 4 | 8) {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("RICE_1 with BYTEPIX = {}", params.bytepix),
                });
            }
            rice::rice_decode_into(
                convert::byte_cell(cell)?,
                tile_elems,
                params.bytepix,
                params.blocksize,
                out,
            )
        }
        ImageCodec::Plio1 => plio::plio_decode_be_into(convert::plio_cell(cell)?, tile_elems, out),
        ImageCodec::Hcompress1 => hcompress::hcompress_tile_into(
            convert::byte_cell(cell)?,
            params.smooth,
            tile_elems,
            out,
            &mut scratch.hcompress,
        ),
        // §10.4: a tile stored verbatim — the cell is the raw big-endian pixels.
        ImageCodec::NoCompress => {
            let bytes = convert::byte_cell(cell)?;
            let expected = tile_elems
                .checked_mul(ctx.int_bitpix.elem_size())
                .ok_or(FitsError::DataUnitOverflow)?;
            if bytes.len() != expected {
                return Err(FitsError::DataSizeMismatch {
                    expected,
                    got: bytes.len(),
                });
            }
            convert::be_to_i64_into(bytes, ctx.int_bitpix, out);
            Ok(())
        }
    }
}

fn ensure_tile_size(expected: usize, got: usize) -> Result<()> {
    if got != expected {
        return Err(FitsError::DataSizeMismatch { expected, got });
    }
    Ok(())
}

/// HCOMPRESS smoothing flag: the `SMOOTH` `ZVALn` is non-zero (cfitsio applies
/// inverse-transform smoothing to suppress blocking in lossy images).
fn hcompress_smooth(header: &Header) -> Result<bool> {
    for entry in header.iter() {
        let Some(i) = compress::parameter_index(entry.keyword) else {
            continue;
        };
        let Some(name) = header.get_text(entry.keyword)? else {
            continue;
        };
        if name == "SMOOTH" {
            return Ok(header.get_integer(key!("ZVAL{i}").as_str())?.unwrap_or(0) != 0);
        }
    }
    Ok(false)
}

/// Read the `ZNAXIS1..ZNAXISn` integer axis lengths.
fn read_axes(header: &Header, n: usize) -> Result<Vec<usize>> {
    (1..=n)
        .map(|i| header.required_usize(key!("ZNAXIS{i}").as_str(), "ZNAXISn"))
        .collect()
}
