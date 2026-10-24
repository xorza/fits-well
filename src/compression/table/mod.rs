//! Tiled table compression (§10.3) — a port of cfitsio's `fits_compress_table`/
//! `fits_uncompress_table`.
//!
//! The table is split into row-tiles of `ZTILELEN` rows. Within a tile each
//! column is transposed to column-major order and compressed independently with
//! its `ZCTYPn` codec (`GZIP_1`/`GZIP_2`/`RICE_1`/`NOCOMPRESS`). The compressed
//! table is itself a `BINTABLE` with `ZTABLE = T`: one row per tile, one `1QB`
//! variable-length byte column per original column, the compressed bytes living
//! in the heap. The original `TFORMn`/`NAXIS1`/`NAXIS2`/`PCOUNT` are preserved
//! as `ZFORMn`/`ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT`.

use crate::allocation;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::element_kind::ElementKind;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
use crate::bintable::tform::Tform;
use crate::bintable::vla_column::VlaColumn;
use crate::column::{MAX_TABLE_FIELDS, validate_table_field_count};
use crate::compression::Compression;
use crate::compression::CompressionConfig;
use crate::compression::convert;
use crate::compression::encoded_unit::EncodedUnit;
use crate::compression::gzip;
use crate::compression::map_tiles;
use crate::compression::plane::IntBitpix;
use crate::compression::rice;
use crate::compression::tile_codec::{CodecScratch, Tile, TileCodec, TileSamples};
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use crate::fits_error::{CompressionFeature, CompressionParameter};
use crate::header_model::record_name::RecordName;
use crate::header_model::{Header, keywords, value};
use crate::ragged::Ragged;
use crate::reserved_keywords;
use std::ops::Range;

/// Per-column compression algorithm (`ZCTYPn`).
/// One column's layout and codec, used by both directions.
#[derive(Debug)]
struct ColMeta {
    tform: Tform,
    /// Bytes per row for this column — the descriptor for a `P`/`Q` column.
    width: usize,
    /// Byte offset of this column within a row.
    offset: usize,
    /// `GZIP_1`, `GZIP_2`, `RICE_1` or `NOCOMPRESS`, valid for the column's type:
    /// [`ColMeta::chosen`] clamps a request and [`ColMeta::declared`] refuses an invalid
    /// `ZCTYPn`.
    codec: TileCodec,
}

impl ColMeta {
    /// Copy the decompressed cells `column` into their place in each row of `out`, a
    /// tile's rows `row_len` bytes each.
    fn scatter(&self, column: &[u8], out: &mut [u8], row_len: usize) {
        let rows = out.len() / row_len;
        debug_assert_eq!(out.len(), rows * row_len, "whole rows");
        debug_assert_eq!(column.len(), rows * self.width, "decompressed column size");
        for (row, start) in out
            .chunks_exact_mut(row_len)
            .zip((0..rows).map(|row| row * self.width))
        {
            row[self.offset..self.offset + self.width]
                .copy_from_slice(&column[start..start + self.width]);
        }
    }

    /// Compress one tile's column-major raw bytes per the column's codec, and lend the
    /// stream from `scratch`.
    fn compress<'s>(&self, bytes: &[u8], scratch: &'s mut CodecScratch) -> Result<&'s [u8]> {
        self.codec.encode(
            self.tile(&[self.element_count(bytes.len())]),
            TileSamples::Bytes(bytes),
            scratch,
        )
    }

    /// The column `tform` at `offset`, compressed with `requested` clamped to a
    /// codec valid for its type, as cfitsio does when it writes.
    fn chosen(tform: &Tform, offset: usize, requested: TileCodec) -> Self {
        let mut meta = Self::declared_unchecked(tform, offset, requested);
        meta.codec = meta.with_rice_width(pick_codec(meta.compression_kind(), requested));
        meta
    }

    /// The column `tform` at `offset` with the codec its `ZCTYPn` declares. Errors
    /// when that codec cannot encode the column's type (`RICE_1` takes only `B`, `I`
    /// and `J` values).
    const fn declared(tform: &Tform, offset: usize, codec: TileCodec) -> Result<Self> {
        let mut meta = Self::declared_unchecked(tform, offset, codec);
        if matches!(codec, TileCodec::Rice(_)) && meta.rice_width().is_none() {
            return Err(FitsError::UnsupportedCompression {
                feature: CompressionFeature::RiceColumn {
                    code: meta.compression_kind().code(),
                },
            });
        }
        meta.codec = meta.with_rice_width(codec);
        Ok(meta)
    }

    const fn declared_unchecked(tform: &Tform, offset: usize, codec: TileCodec) -> Self {
        Self {
            tform: *tform,
            width: tform.byte_width(),
            offset,
            codec,
        }
    }

    /// `codec`, its `RICE_1` samples as wide as the column's.
    const fn with_rice_width(&self, codec: TileCodec) -> TileCodec {
        match (codec, self.rice_width()) {
            (TileCodec::Rice(rice::Codec { blocksize, .. }), Some(bytepix)) => {
                TileCodec::Rice(rice::Codec { blocksize, bytepix })
            }
            (codec, _) => codec,
        }
    }

    /// One tile of `count` of the column's elements as its codec sees them, typed by
    /// [`Self::sample_width`].
    const fn tile<'c>(&self, count: &'c [usize; 1]) -> Tile<'c> {
        Tile {
            shape: count,
            width: self.sample_width(),
            blank: None,
        }
    }

    /// The integer type the codec reads the column's values as: `RICE_1`'s pixel width,
    /// and the `GZIP_2` shuffle width for the multi-byte numeric types cfitsio shuffles
    /// (`I`/`J`/`E`/`K`/`D`); 1 byte for any other.
    const fn sample_width(&self) -> IntBitpix {
        match self.compression_kind() {
            ElementKind::I16 => IntBitpix::I16,
            ElementKind::I32 | ElementKind::F32 => IntBitpix::I32,
            ElementKind::I64 | ElementKind::F64 => IntBitpix::I64,
            _ => IntBitpix::U8,
        }
    }

    /// The column's elements in `bytes`.
    const fn element_count(&self, bytes: usize) -> usize {
        bytes / self.sample_width().elem_size()
    }

    const fn is_vla(&self) -> bool {
        self.tform.descriptor_width().is_some()
    }

    const fn is_stored_vla(&self) -> bool {
        self.is_vla() && self.width != 0
    }

    /// The descriptors of an array column.
    const fn descriptor_width(&self) -> DescriptorWidth {
        self.tform
            .descriptor_width()
            .expect("descriptors are read from an array column")
    }

    /// The type of the values that are compressed: a fixed column's cells, or an array
    /// column's heap elements.
    const fn compression_kind(&self) -> ElementKind {
        self.tform.element()
    }

    /// `RICE_1` pixel width (`B`, `I` and `J`); other types can't use Rice.
    const fn rice_width(&self) -> Option<IntBitpix> {
        match self.compression_kind() {
            ElementKind::U8 => Some(IntBitpix::U8),
            ElementKind::I16 => Some(IntBitpix::I16),
            ElementKind::I32 => Some(IntBitpix::I32),
            _ => None,
        }
    }
}

/// Clamp a requested codec to one valid for the column type, mirroring cfitsio's
/// per-type sanity overrides; a gzip codec keeps the requested level.
fn pick_codec(kind: ElementKind, requested: TileCodec) -> TileCodec {
    debug_assert!(
        requested != TileCodec::NoCompress,
        "the writer refuses NOCOMPRESS"
    );
    let level = match requested {
        TileCodec::Gzip1 { level } | TileCodec::Gzip2 { level } => level,
        _ => gzip::DEFAULT_GZIP_LEVEL,
    };
    match kind {
        // RICE_1 takes no logical, bit, text or complex value, and cfitsio's reader
        // unshuffles GZIP_2 only for I, J, K, E and D (`fits_uncompress_table`).
        ElementKind::Logical
        | ElementKind::Bit
        | ElementKind::Character
        | ElementKind::ComplexF32
        | ElementKind::ComplexF64 => TileCodec::Gzip1 { level },
        ElementKind::F32 | ElementKind::F64 | ElementKind::I64 => match requested {
            TileCodec::Gzip1 { .. } => TileCodec::Gzip1 { level },
            _ => TileCodec::Gzip2 { level },
        },
        // A shuffle of 1-byte values changes nothing, and cfitsio refuses it.
        ElementKind::U8 if matches!(requested, TileCodec::Gzip2 { .. }) => {
            TileCodec::Gzip1 { level }
        }
        ElementKind::I16 | ElementKind::I32 | ElementKind::U8 => requested,
    }
}

/// The cells of a table being compressed, read by column: what a tile of each column is
/// made from. A table built for a write gives them with no row-major copy of itself.
pub(crate) trait TableCells: Sync {
    /// Append the stored bytes of the fixed-width `column`'s cells in `rows` to `out`.
    fn fixed(&self, column: usize, rows: Range<usize>, out: &mut Vec<u8>);

    /// Append the stored bytes of the variable-length `column`'s array at `row` to
    /// `out`, and give the descriptor the uncompressed table holds for it.
    fn array(&self, column: usize, row: usize, out: &mut Vec<u8>) -> PqDescriptor;
}

/// Compress the `BINTABLE` that `header` describes, its cells from `cells`, into a
/// `ZTABLE` container, with the tile height and the codec of every column from
/// `config`: its header and its data unit, the Q descriptors and the heap.
pub(crate) fn compress_table(
    header: &Header,
    cells: &impl TableCells,
    config: &CompressionConfig,
) -> Result<EncodedUnit> {
    let requested = config.compression.tile_codec(IntBitpix::I32, 0);
    let gzip_level = match requested {
        TileCodec::Gzip1 { level } | TileCodec::Gzip2 { level } => level,
        _ => gzip::DEFAULT_GZIP_LEVEL,
    };
    match config.compression {
        Compression::Gzip(_) | Compression::Rice => {}
        Compression::None => {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::TableWithoutCodec,
            });
        }
        other => {
            return Err(FitsError::UnsupportedCompression {
                feature: CompressionFeature::TableCodec {
                    codec: other.name(),
                },
            });
        }
    }
    let descriptor_codec = TileCodec::Gzip1 { level: gzip_level };
    debug_assert!(
        !has_compression_keyword(header),
        "a written table's header holds no compression keyword"
    );
    let schema = Schema::parse(header)?;
    let ncols = schema.columns.len();
    let nrows = schema.extent.row_count;
    let naxis1 = schema.extent.row_len;
    let pcount = header.require_usize(keywords::PCOUNT)?;
    let rpt = config.table_tile_rows(nrows, naxis1)?;
    let metas: Vec<ColMeta> = schema
        .columns
        .iter()
        .map(|c| ColMeta::chosen(&c.tform, c.byte_offset, requested))
        .collect();

    let nchunks = nrows.div_ceil(rpt);
    let compressed_row_len = ncols.checked_mul(16).ok_or(FitsError::DataUnitOverflow)?;
    let tile_count = nchunks
        .checked_mul(ncols)
        .ok_or(FitsError::DataUnitOverflow)?;

    // Compress each (chunk, column) tile independently — the compute-bound step,
    // parallel under the `parallel` feature, indexed `chunk * ncols + ci` so the
    // results land in the same flat order the descriptor rows expect. The reused
    // per-worker buffer holds the column's transposed bytes.
    let comps = map_tiles(
        tile_count,
        TableEncodeScratch::default,
        |scratch, i| -> Result<EncodedColumn> {
            let chunk = i / ncols;
            let column = i % ncols;
            let m = &metas[column];
            let r0 = chunk * rpt;
            let rows = rpt.min(nrows - r0);
            if m.is_stored_vla() {
                let width = m.descriptor_width();
                let mut descriptors = allocation::try_zeroed(
                    0u8,
                    rows.checked_mul(m.width)
                        .ok_or(FitsError::DataUnitOverflow)?,
                )?;
                let mut values = Vec::new();
                let mut ends = Vec::with_capacity(rows);
                let array = &mut scratch.array;
                for row in 0..rows {
                    array.clear();
                    let descriptor = cells.array(column, r0 + row, array);
                    descriptor
                        .write(width, &mut descriptors[row * m.width..(row + 1) * m.width])?;
                    // An array the codec does not shrink is stored as is, as cfitsio — the
                    // reference implementation of §10.3 — stores it; a stored length equal
                    // to the raw length is then what marks it raw, and cannot be a stream.
                    let stream = m.compress(array, &mut scratch.codec)?;
                    values.extend_from_slice(if stream.len() < array.len() {
                        stream
                    } else {
                        array
                    });
                    ends.push(values.len());
                }
                return Ok(EncodedColumn::Variable {
                    descriptors,
                    arrays: Ragged::<Vec<u8>>::new(values, ends),
                });
            }
            scratch.column.clear();
            let cell_len = rows
                .checked_mul(m.width)
                .ok_or(FitsError::DataUnitOverflow)?;
            scratch.column.reserve_exact(cell_len);
            cells.fixed(column, r0..r0 + rows, &mut scratch.column);
            debug_assert_eq!(scratch.column.len(), cell_len, "a tile's whole cells");
            let compressed = m.compress(&scratch.column, &mut scratch.codec)?.to_vec();
            Ok(EncodedColumn::Fixed(compressed))
        },
    )?;

    // The heap holds the fixed columns' cells, then every array, then the arrays'
    // descriptor blocks, so the arrays' offsets are prefix sums of lengths known before
    // any block is compressed, and each block gzips on its own, in parallel, rather than
    // inside a serial walk that a block's compressed length would hold up.
    let fixed_len = comps.iter().try_fold(0usize, |len, comp| match comp {
        EncodedColumn::Fixed(bytes) => len.checked_add(bytes.len()),
        EncodedColumn::Variable { .. } => Some(len),
    });
    let fixed_len = fixed_len.ok_or(FitsError::DataUnitOverflow)?;
    let mut array_starts = Vec::with_capacity(tile_count);
    let mut arrays_end = fixed_len;
    for comp in &comps {
        array_starts.push(arrays_end);
        if let EncodedColumn::Variable { arrays, .. } = comp {
            arrays_end = arrays_end
                .checked_add(arrays.values().len())
                .ok_or(FitsError::DataUnitOverflow)?;
        }
    }
    let blocks = map_tiles(
        tile_count,
        CodecScratch::default,
        |scratch, tile| -> Result<Vec<u8>> {
            let EncodedColumn::Variable {
                descriptors,
                arrays,
            } = &comps[tile]
            else {
                return Ok(Vec::new());
            };
            let compressed_len = arrays
                .len()
                .checked_mul(16)
                .ok_or(FitsError::DataUnitOverflow)?;
            // The original descriptors, then those of their compressed arrays, as
            // cfitsio writes them, the reverse of the order FITS 4.0 §10.3.5 lists.
            let original_len = descriptors.len();
            let mut combined = allocation::try_zeroed(
                0u8,
                original_len
                    .checked_add(compressed_len)
                    .ok_or(FitsError::DataUnitOverflow)?,
            )?;
            combined[..original_len].copy_from_slice(descriptors);
            let compressed = combined[original_len..].as_chunks_mut::<16>().0;
            for (row, slot) in compressed.iter_mut().enumerate() {
                let range = arrays.range(row);
                if range.is_empty() {
                    continue;
                }
                PqDescriptor {
                    count: range.len(),
                    offset: array_starts[tile] + range.start,
                }
                .write(DescriptorWidth::Q, slot)?;
            }
            Ok(descriptor_codec
                .encode(
                    byte_tile(&[combined.len()]),
                    TileSamples::Bytes(&combined),
                    scratch,
                )?
                .to_vec())
        },
    )?;
    let descriptor_bytes = tile_count
        .checked_mul(16)
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut rows = allocation::try_zeroed(0u8, descriptor_bytes)?;
    let mut fixed = Vec::with_capacity(tile_count);
    let mut arrays = Vec::new();
    let mut variable = Vec::new();
    let (mut fixed_end, mut blocks_end) = (0usize, arrays_end);
    for ((slot, comp), block) in rows
        .as_chunks_mut::<16>()
        .0
        .iter_mut()
        .zip(comps)
        .zip(blocks)
    {
        let (cell, offset) = match comp {
            EncodedColumn::Fixed(bytes) => {
                let offset = fixed_end;
                fixed_end += bytes.len();
                fixed.push(bytes);
                (fixed.last().expect("pushed"), offset)
            }
            EncodedColumn::Variable {
                arrays: tile_arrays,
                ..
            } => {
                arrays.push(tile_arrays.into_values());
                let offset = blocks_end;
                blocks_end = blocks_end
                    .checked_add(block.len())
                    .ok_or(FitsError::DataUnitOverflow)?;
                variable.push(block);
                (variable.last().expect("pushed"), offset)
            }
        };
        PqDescriptor {
            count: cell.len(),
            offset,
        }
        .write(DescriptorWidth::Q, slot)?;
    }
    debug_assert_eq!(fixed_end, fixed_len, "the fixed cells precede the arrays");
    let heap_len = blocks_end;
    let mut heap = fixed;
    heap.extend(arrays);
    heap.extend(variable);

    // Header: copy the original, then layer on the Z* keywords.
    let mut h = header.clone();
    h.set_internal(keywords::ZTABLE, true)
        .set_comment_internal(keywords::ZTABLE, "this is a compressed table");
    h.set_internal(keywords::ZTILELEN, value::fits_i64(rpt)?);
    h.set_internal(keywords::ZNAXIS.indexed(1)?, value::fits_i64(naxis1)?);
    h.set_internal(keywords::ZNAXIS.indexed(2)?, value::fits_i64(nrows)?);
    h.set_internal(keywords::ZPCOUNT, value::fits_i64(pcount)?);
    for (ci, m) in metas.iter().enumerate() {
        let n = ci + 1;
        let zform = header
            .get_text(keywords::TFORM.indexed(n)?)?
            .unwrap_or("")
            .to_string();
        h.set_internal(keywords::ZFORM.indexed(n)?, zform);
        h.set_internal(keywords::TFORM.indexed(n)?, "1QB");
        h.set_internal(keywords::ZCTYP.indexed(n)?, m.codec.name());
    }
    h.set_internal(
        keywords::NAXIS.indexed(1)?,
        value::fits_i64(compressed_row_len)?,
    );
    h.set_internal(keywords::NAXIS.indexed(2)?, value::fits_i64(nchunks)?);
    h.set_internal(keywords::PCOUNT, value::fits_i64(heap_len)?);
    h.set_internal(keywords::GCOUNT, 1);
    Ok(EncodedUnit {
        header: h,
        rows,
        heap,
    })
}

/// A tiled-compressed table's layout, read once from its container's header: the table
/// it restores to, and where and how each tile column is stored.
#[derive(Debug)]
pub(crate) struct TiledTable {
    /// The restored table's header, which the tests of the restore read.
    #[cfg(test)]
    pub(crate) header: Header,
    /// The restored table's schema: what every read of the table gives.
    pub(crate) schema: Schema,
    /// `ZTILELEN`, cut to `1..=nrows`.
    rows_per_tile: usize,
    metas: Vec<ColMeta>,
    /// Bytes `ZTHEAP` leaves between the main table and the heap.
    heap_gap: usize,
    /// The restored data unit's length: main table, gap and heap.
    data_len: usize,
}

impl TiledTable {
    /// The layout `header`, a `ZTABLE` container's, declares, checked once, so no decode
    /// checks it again.
    pub(crate) fn new(header: &Header) -> Result<Self> {
        debug_assert_eq!(header.get_logical(keywords::ZTABLE).unwrap(), Some(true));
        let naxis1 = header.require_usize(keywords::ZNAXIS.indexed(1)?)?;
        let nrows = header.require_usize(keywords::ZNAXIS.indexed(2)?)?;
        let zpcount = header.require_usize(keywords::ZPCOUNT)?;
        let original_bytes = nrows
            .checked_mul(naxis1)
            .ok_or(FitsError::DataUnitOverflow)?;
        let original_heap_offset = match header.get_integer(keywords::ZTHEAP)? {
            Some(ztheap) => usize::try_from(ztheap).map_err(|_| FitsError::KeywordOutOfRange {
                keyword: keywords::ZTHEAP,
            })?,
            None => original_bytes,
        };
        let data_len = original_bytes
            .checked_add(zpcount)
            .ok_or(FitsError::DataUnitOverflow)?;
        if !(original_bytes..=data_len).contains(&original_heap_offset) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::ZTHEAP,
            });
        }
        header.get_text(keywords::ZHECKSUM)?;
        header.get_text(keywords::ZDATASUM)?;
        let mut rows_per_tile = header.require_usize(keywords::ZTILELEN)?;
        // A zero tile height would make the row-tile count diverge; §10.3 requires ≥ 1.
        if rows_per_tile == 0 {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::ZTILELEN,
            });
        }
        rows_per_tile = rows_per_tile.min(nrows.max(1));
        let ncols = header.require_usize(keywords::TFIELDS)?;
        validate_table_field_count(ncols)?;

        // Resolve each column's original form and codec.
        let mut metas = Vec::with_capacity(ncols);
        let mut zforms = Vec::with_capacity(ncols);
        let mut offset = 0;
        for n in 1..=ncols {
            let zform = header
                .require_text(keywords::ZFORM.indexed(n)?)?
                .to_string();
            let tform = zform.parse::<Tform>()?;
            let codec = match header.get_text(keywords::ZCTYP.indexed(n)?)? {
                Some(name) => column_codec(name)?,
                // cfitsio's default when ZCTYPn is absent.
                None => TileCodec::Gzip2 {
                    level: gzip::DEFAULT_GZIP_LEVEL,
                },
            };
            let m = ColMeta::declared(&tform, offset, codec)?;
            offset = offset
                .checked_add(m.width)
                .ok_or(FitsError::DataUnitOverflow)?;
            zforms.push(zform);
            metas.push(m);
        }
        if offset != naxis1 {
            return Err(FitsError::RowWidthMismatch {
                computed: offset,
                declared: naxis1,
            });
        }

        // Restore the original header: drop the Z* keywords, reinstate NAXIS/PCOUNT.
        let mut restored = header.clone();
        restored.set_internal(keywords::NAXIS.indexed(1)?, value::fits_i64(naxis1)?);
        restored.set_internal(keywords::NAXIS.indexed(2)?, value::fits_i64(nrows)?);
        restored.set_internal(keywords::PCOUNT, value::fits_i64(zpcount)?);
        for (n, zform) in zforms.into_iter().enumerate() {
            restored.set_internal(keywords::TFORM.indexed(n + 1)?, zform);
        }
        // The container's own heap pointer and checksums give way to the table's.
        restored.remove_where(|keyword| {
            reserved_keywords::is_table_compression(keyword, ncols)
                || reserved_keywords::TABLE_PRESERVED
                    .iter()
                    .any(|&(table, _)| table == keyword)
        });
        restored.rename_keywords(
            &reserved_keywords::TABLE_PRESERVED.map(|(table, container)| (container, table)),
        );
        let schema = Schema::parse(&restored)?;
        let nchunks = nrows.div_ceil(rows_per_tile);
        let container_rows = header.require_usize(keywords::NAXIS.indexed(2)?)?;
        if container_rows != nchunks {
            return Err(SizeUnit::Rows.mismatch(nchunks, container_rows));
        }
        Ok(Self {
            #[cfg(test)]
            header: restored,
            schema,
            rows_per_tile,
            metas,
            heap_gap: original_heap_offset - original_bytes,
            data_len,
        })
    }

    /// Whether a tile holds arrays of the table's heap, whose compressed descriptors
    /// address the container's heap: a decode then reads the whole container.
    pub(crate) fn has_arrays(&self) -> bool {
        self.metas.iter().any(ColMeta::is_stored_vla)
    }

    /// The row tiles, the container's rows, that hold `rows`.
    pub(crate) fn tiles(&self, rows: &Range<usize>) -> Range<usize> {
        if rows.is_empty() {
            return 0..0;
        }
        rows.start / self.rows_per_tile..rows.end.div_ceil(self.rows_per_tile)
    }

    /// The tiles that hold `rows`, a range of the restored table's rows, decoded with the
    /// heap arrays they reference and no other, of the `columns` selected (every column
    /// when `None`). Every byte of a column not selected is zero.
    pub(crate) fn decode(
        &self,
        container_tiles: ContainerTiles<'_>,
        rows: &Range<usize>,
        columns: Option<&[bool]>,
    ) -> Result<DecodedRows> {
        let ContainerTiles {
            view: container,
            first_tile,
        } = container_tiles;
        let nrows = self.schema.extent.row_count;
        let naxis1 = self.schema.extent.row_len;
        let rpt = self.rows_per_tile;
        debug_assert!(rows.end <= nrows, "a validated row range");
        debug_assert!(columns.is_none_or(|columns| columns.len() == self.metas.len()));
        let tiles = self.tiles(rows);
        let first_row = if tiles.is_empty() {
            rows.start
        } else {
            tiles.start * rpt
        };
        let row_count = (tiles.end * rpt).min(nrows).saturating_sub(first_row);
        let cells: Vec<_> = (0..self.metas.len())
            .map(|ci| container.column(ci)?.vla_column())
            .collect::<Result<_>>()?;
        debug_assert!(
            tiles.is_empty()
                || (first_tile <= tiles.start
                    && tiles.end <= first_tile + container.extent.row_count),
            "the container holds the tiles"
        );
        let main_len = row_count * naxis1;
        let mut out = allocation::try_zeroed(0u8, main_len)?;
        let chunk_len = rpt.checked_mul(naxis1).ok_or(FitsError::DataUnitOverflow)?;
        let context = TileDecode {
            table: self,
            tiles: container_tiles,
            cells: &cells,
            columns,
        };
        if self.has_arrays() {
            // Each tile restores its rows and stages its own heap, with its descriptors
            // offset from that heap's start; the heaps then join in tile order, and each
            // tile's descriptors land offset by the bytes of the heaps before it.
            let restore = |state: &mut TileRestore, chunk: usize, main: &mut [u8]| {
                context.restore_tile(chunk, state, main)
            };
            let first = tiles.start;
            let main = &mut out[..main_len];
            #[cfg(feature = "parallel")]
            let heaps: Vec<TileHeap> = {
                use rayon::prelude::*;
                main.par_chunks_mut(chunk_len)
                    .enumerate()
                    .map_init(TileRestore::default, |state, (chunk, main)| {
                        restore(state, first + chunk, main)
                    })
                    .collect::<Result<_>>()?
            };
            #[cfg(not(feature = "parallel"))]
            let heaps: Vec<TileHeap> = {
                let mut state = TileRestore::default();
                main.chunks_mut(chunk_len)
                    .enumerate()
                    .map(|(chunk, main)| restore(&mut state, first + chunk, main))
                    .collect::<Result<_>>()?
            };
            let mut base = 0usize;
            for (main, heap) in out[..main_len].chunks_mut(chunk_len).zip(&heaps) {
                context.place_descriptors(main, heap, base)?;
                base = base
                    .checked_add(heap.bytes.len())
                    .ok_or(FitsError::DataUnitOverflow)?;
            }
            allocation::try_reserve(&mut out, base)?;
            for heap in &heaps {
                out.extend_from_slice(&heap.bytes);
            }
        } else if main_len != 0 {
            let first = tiles.start;
            let main = &mut out[..main_len];
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;

                main.par_chunks_mut(chunk_len)
                    .enumerate()
                    .try_for_each_init(CodecScratch::default, |scratch, (chunk, out)| {
                        context.decode_fixed(first + chunk, scratch, out)
                    })?;
            }
            #[cfg(not(feature = "parallel"))]
            {
                let mut scratch = CodecScratch::default();
                for (chunk, out) in main.chunks_mut(chunk_len).enumerate() {
                    context.decode_fixed(first + chunk, &mut scratch, out)?;
                }
            }
        }
        Ok(DecodedRows {
            first_row,
            schema: self.schema.compacted(row_count, out.len()),
            data: out,
        })
    }
}

/// The container rows a decode reads: the tiles from `first_tile` on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContainerTiles<'a> {
    pub(crate) view: TableView<'a>,
    pub(crate) first_tile: usize,
}

/// The inputs fixed for one decode: the tiled table, the container rows that hold its
/// tiles, the container's cells, and the columns it restores, every column when `None`.
#[derive(Debug, Clone, Copy)]
struct TileDecode<'a> {
    table: &'a TiledTable,
    tiles: ContainerTiles<'a>,
    cells: &'a [VlaColumn<'a>],
    columns: Option<&'a [bool]>,
}

impl TileDecode<'_> {
    fn selected(&self, column: usize) -> bool {
        self.columns.is_none_or(|columns| columns[column])
    }

    /// The compressed cell of `column` in tile `chunk`.
    fn cell(&self, column: usize, chunk: usize) -> Result<&[u8]> {
        convert::byte_cell(self.cells[column].cell(chunk - self.tiles.first_tile)?)
    }

    /// Restore tile `chunk` of a table with no arrays into `out`.
    fn decode_fixed(&self, chunk: usize, codecs: &mut CodecScratch, out: &mut [u8]) -> Result<()> {
        let table = self.table;
        let rows = table
            .rows_per_tile
            .min(table.schema.extent.row_count - chunk * table.rows_per_tile);
        for (column, m) in table.metas.iter().enumerate() {
            if !self.selected(column) {
                continue;
            }
            let stream = ColumnStream {
                meta: m,
                bytes: self.cell(column, chunk)?,
            };
            m.scatter(
                stream.decompress_cells(rows, codecs)?,
                out,
                table.schema.extent.row_len,
            );
        }
        Ok(())
    }

    /// Restore tile `chunk`'s fixed columns into `main` and return its heap: the arrays
    /// its rows reference, row by row, each column in turn, as a write and a plain
    /// selection lay them out, with their descriptors offset from the heap's start.
    fn restore_tile(
        &self,
        chunk: usize,
        state: &mut TileRestore,
        main: &mut [u8],
    ) -> Result<TileHeap> {
        let table = self.table;
        let nrows = table.schema.extent.row_count;
        let naxis1 = table.schema.extent.row_len;
        let rows = table.rows_per_tile.min(nrows - chunk * table.rows_per_tile);
        let restore = RestoreChunk {
            container: self.tiles.view,
            rows,
            heap_gap: table.heap_gap,
            heap_len: table.data_len - nrows * naxis1,
        };
        let TileRestore { codecs, staged } = state;
        let mut stored = 0;
        for (column, m) in table.metas.iter().enumerate() {
            if !self.selected(column) {
                continue;
            }
            let stream = ColumnStream {
                meta: m,
                bytes: self.cell(column, chunk)?,
            };
            if m.is_stored_vla() {
                if staged.len() == stored {
                    staged.push(StagedArrays::default());
                }
                let arrays = &mut staged[stored];
                arrays.column = column;
                restore.stage_vla_column(stream, codecs, arrays)?;
                stored += 1;
            } else {
                m.scatter(stream.decompress_cells(rows, codecs)?, main, naxis1);
            }
        }
        let mut heap = TileHeap::default();
        heap.descriptors.reserve_exact(rows * stored);
        for row in 0..rows {
            for arrays in &staged[..stored] {
                let range = arrays.range(row);
                heap.descriptors.push(PqDescriptor {
                    count: arrays.counts[row],
                    offset: heap.bytes.len(),
                });
                allocation::try_reserve(&mut heap.bytes, range.len())?;
                heap.bytes.extend_from_slice(&arrays.bytes[range]);
            }
        }
        Ok(heap)
    }

    /// Write `heap`'s descriptors into the array columns of a tile's restored rows
    /// `main`, offset by `base`, the bytes of the heaps before it.
    fn place_descriptors(&self, main: &mut [u8], heap: &TileHeap, base: usize) -> Result<()> {
        let naxis1 = self.table.schema.extent.row_len;
        let arrays: Vec<&ColMeta> = self
            .table
            .metas
            .iter()
            .enumerate()
            .filter(|&(column, m)| m.is_stored_vla() && self.selected(column))
            .map(|(_, m)| m)
            .collect();
        debug_assert_eq!(
            heap.descriptors.len(),
            main.len() / naxis1 * arrays.len(),
            "one descriptor a row and column"
        );
        let mut descriptors = heap.descriptors.iter();
        for row in main.chunks_exact_mut(naxis1) {
            for m in &arrays {
                let descriptor = descriptors.next().expect("one descriptor a row and column");
                PqDescriptor {
                    offset: descriptor
                        .offset
                        .checked_add(base)
                        .ok_or(FitsError::DataUnitOverflow)?,
                    ..*descriptor
                }
                .write(m.descriptor_width(), &mut row[m.offset..m.offset + m.width])?;
            }
        }
        Ok(())
    }
}

/// One tile's restored heap: the bytes of its arrays, and their descriptors in row
/// order, each column in turn, offset from the heap's start.
#[derive(Debug, Default)]
struct TileHeap {
    bytes: Vec<u8>,
    descriptors: Vec<PqDescriptor>,
}

/// The buffers one worker restores a table tile's arrays through.
#[derive(Debug, Default)]
struct TileRestore {
    codecs: CodecScratch,
    staged: Vec<StagedArrays>,
}

/// The rows of a tiled-compressed table that [`TiledTable::decode`] restores: whole tiles
/// and the heap arrays they reference, laid out as a table of their own.
#[derive(Debug)]
pub(crate) struct DecodedRows {
    /// The row of the restored table that is row 0 here.
    pub(crate) first_row: usize,
    pub(crate) schema: Schema,
    pub(crate) data: Vec<u8>,
}

/// Whether `header` holds a keyword of the table compression or one it moves the
/// table's own to.
fn has_compression_keyword(header: &Header) -> bool {
    header.into_iter().any(|record| {
        matches!(record.name(), Some(RecordName::Keyword(keyword))
            if reserved_keywords::is_table_compression(keyword, MAX_TABLE_FIELDS)
                || reserved_keywords::TABLE_PRESERVED
                    .iter()
                    .any(|&(_, container)| container == keyword))
    })
}

/// The buffers a worker reuses from one table tile to the next.
#[derive(Debug, Default)]
struct TableEncodeScratch {
    column: Vec<u8>,
    array: Vec<u8>,
    codec: CodecScratch,
}

#[derive(Debug)]
enum EncodedColumn {
    Fixed(Vec<u8>),
    Variable {
        descriptors: Vec<u8>,
        arrays: Ragged<Vec<u8>>,
    },
}

/// The codec a `ZCTYPn` spells: §10.3 takes the gzip codecs, `RICE_1` and
/// `NOCOMPRESS` for a column, which the wavelet and the mask codecs have no meaning for.
fn column_codec(name: &str) -> Result<TileCodec> {
    match TileCodec::parse(name) {
        Ok(
            codec @ (TileCodec::Gzip1 { .. }
            | TileCodec::Gzip2 { .. }
            | TileCodec::Rice(_)
            | TileCodec::NoCompress),
        ) => Ok(codec),
        _ => Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::ColumnCodec {
                name: name.to_string(),
            },
        }),
    }
}

/// A tile of `count` bytes, as a descriptor block is.
const fn byte_tile(count: &[usize; 1]) -> Tile<'_> {
    Tile {
        shape: count,
        width: IntBitpix::U8,
        blank: None,
    }
}

/// One row-tile of the table being restored: the slice of the main table its rows
/// occupy, the shared heap they point into, and the geometry both need.
#[derive(Debug)]
struct RestoreChunk<'a> {
    container: TableView<'a>,
    rows: usize,
    /// Bytes `THEAP` leaves between the end of the main table and the heap.
    heap_gap: usize,
    /// The bytes after the main table of the table restored whole, `PCOUNT`: where the
    /// original descriptors may point.
    heap_len: usize,
}

/// One array column's restored arrays of a tile: each row's element count, and its bytes
/// in one buffer, with the tile's decoded descriptor block, reused from tile to tile.
#[derive(Debug, Default)]
struct StagedArrays {
    column: usize,
    counts: Vec<usize>,
    bytes: Vec<u8>,
    ends: Vec<usize>,
    descriptors: Vec<u8>,
}

impl StagedArrays {
    fn range(&self, row: usize) -> Range<usize> {
        let start = if row == 0 { 0 } else { self.ends[row - 1] };
        start..self.ends[row]
    }
}

impl RestoreChunk<'_> {
    /// Restore the tile's arrays of a column from its compressed `cell` into `arrays`.
    /// The cell holds the tile's original descriptors, then the Q descriptors of their
    /// compressed arrays, as cfitsio and nom-tam-fits write and read them.
    fn stage_vla_column(
        &self,
        cell: ColumnStream<'_>,
        codecs: &mut CodecScratch,
        arrays: &mut StagedArrays,
    ) -> Result<()> {
        let m = cell.meta;
        let original_len = self
            .rows
            .checked_mul(m.width)
            .ok_or(FitsError::DataUnitOverflow)?;
        let combined_len = original_len
            .checked_add(
                self.rows
                    .checked_mul(16)
                    .ok_or(FitsError::DataUnitOverflow)?,
            )
            .ok_or(FitsError::DataUnitOverflow)?;
        let descriptors =
            DESCRIPTOR_GZIP.decode_bytes(byte_tile(&[combined_len]), cell.bytes, codecs)?;
        arrays.descriptors.clear();
        arrays.descriptors.extend_from_slice(descriptors);

        let width = m.descriptor_width();
        let element_kind = m.compression_kind();
        arrays.counts.clear();
        arrays.bytes.clear();
        arrays.ends.clear();
        for row in 0..self.rows {
            let original_start = row * m.width;
            let original = PqDescriptor::decode(
                &arrays.descriptors[original_start..original_start + m.width],
                width,
            )?;
            let compressed_start = original_len + row * 16;
            let compressed = PqDescriptor::decode(
                &arrays.descriptors[compressed_start..compressed_start + 16],
                DescriptorWidth::Q,
            )?;
            let original_range = original.heap_range(element_kind, self.heap_gap, self.heap_len)?;
            let expected = original_range.len();
            arrays.counts.push(original.count);
            if expected == 0 {
                if compressed.count != 0 {
                    return Err(SizeUnit::Bytes.mismatch(0, compressed.count));
                }
                arrays.ends.push(arrays.bytes.len());
                continue;
            }
            let stream = ColumnStream {
                meta: m,
                bytes: self.container.pq_payload(compressed, ElementKind::U8)?,
            };
            let array = stream.decompress_array(expected, codecs)?;
            allocation::try_reserve(&mut arrays.bytes, array.len())?;
            arrays.bytes.extend_from_slice(array);
            arrays.ends.push(arrays.bytes.len());
        }
        Ok(())
    }
}

/// A column's compressed bytes, a tile's cells of it or one of its arrays, with the
/// column they restore.
#[derive(Debug, Clone, Copy)]
struct ColumnStream<'a> {
    meta: &'a ColMeta,
    bytes: &'a [u8],
}

impl ColumnStream<'_> {
    /// The `rows` cells of a tile, lent from `codecs`. The inflate is bounded at their
    /// exact size, `rows × width ≤ ZNAXIS2 × ZNAXIS1`, so a crafted cell cannot expand
    /// unbounded.
    fn decompress_cells<'s>(self, rows: usize, codecs: &'s mut CodecScratch) -> Result<&'s [u8]>
    where
        Self: 's,
    {
        let expected = rows
            .checked_mul(self.meta.width)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.decompress(self.meta.codec, expected, codecs)
    }

    /// One array of `expected` bytes, lent from `codecs`. cfitsio stores an array its
    /// codec does not shrink as is, and every stream it does store is shorter than the
    /// array: a length equal to the raw length marks a raw array (cfitsio
    /// `fits_uncompress_table`), §10.3.6 notwithstanding.
    fn decompress_array<'s>(self, expected: usize, codecs: &'s mut CodecScratch) -> Result<&'s [u8]>
    where
        Self: 's,
    {
        let codec = if self.bytes.len() == expected {
            TileCodec::NoCompress
        } else {
            self.meta.codec
        };
        self.decompress(codec, expected, codecs)
    }

    fn decompress<'s>(
        self,
        codec: TileCodec,
        expected: usize,
        codecs: &'s mut CodecScratch,
    ) -> Result<&'s [u8]>
    where
        Self: 's,
    {
        let count = [self.meta.element_count(expected)];
        codec.decode_bytes(self.meta.tile(&count), self.bytes, codecs)
    }
}

/// The codec of a tile's descriptor block, whose level a decode does not read.
const DESCRIPTOR_GZIP: TileCodec = TileCodec::Gzip1 {
    level: gzip::DEFAULT_GZIP_LEVEL,
};

/// The mixed-column table the table-compression tests compress.
#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::column_data::ColumnData;
    use crate::bintable::table_view::TableView;
    use crate::compression::table::{ContainerTiles, TiledTable};
    use crate::fits_error::Result;
    use crate::fits_writer::builder::WriteColumn;
    use crate::header_model::Header;

    /// A restored header and its decompressed data unit.
    #[derive(Debug)]
    pub(crate) struct HduParts {
        pub(crate) header: Header,
        pub(crate) data: Vec<u8>,
    }

    /// The table the container `header` and `table` hold, restored whole.
    pub(crate) fn uncompress_table(header: &Header, table: TableView<'_>) -> Result<HduParts> {
        let tiled = TiledTable::new(header)?;
        let data = tiled
            .decode(
                ContainerTiles {
                    view: table,
                    first_tile: 0,
                },
                &(0..tiled.schema.row_count()),
                None,
            )?
            .data;
        Ok(HduParts {
            header: tiled.header,
            data,
        })
    }

    /// i16, i32, f32, f64 and byte columns, and a repeat-3 i16 vector, over
    /// `row_count` rows of simple formulas with negative values among them; the integer
    /// formulas repeat every 1000 or 10 000 rows, so they stay in range for any count.
    pub(crate) fn mixed_columns(row_count: usize) -> Vec<WriteColumn> {
        vec![
            WriteColumn::fixed(
                "SHORT",
                ColumnData::I16(
                    (0..row_count)
                        .map(|i| i16::try_from(i % 1000).unwrap() * 7 - 30)
                        .collect(),
                ),
                1,
            ),
            WriteColumn::fixed(
                "INT",
                ColumnData::I32(
                    (0..row_count)
                        .map(|i| i32::try_from(i % 10_000).unwrap() * 100_000 - 5)
                        .collect(),
                ),
                1,
            ),
            WriteColumn::fixed(
                "FLT",
                ColumnData::F32((0..row_count).map(|i| i as f32 * 1.5 - 3.25).collect()),
                1,
            ),
            WriteColumn::fixed(
                "DBL",
                ColumnData::F64((0..row_count).map(|i| i as f64 * 0.1).collect()),
                1,
            ),
            WriteColumn::fixed(
                "BYTE",
                ColumnData::U8((0..row_count).map(|i| (i * 3) as u8).collect()),
                1,
            ),
            WriteColumn::fixed(
                "VEC",
                ColumnData::I16(
                    (0..row_count * 3)
                        .map(|i| i16::try_from(i % 10_000).unwrap() * 2)
                        .collect(),
                ),
                3,
            ),
        ]
    }
}

#[cfg(test)]
mod tests;
