//! Header and data-unit serialization.
//!
//! The high-level writers — [`FitsWriter::write_image`], `write_table`,
//! `write_ascii_table`, and the compressed forms — synthesize the mandatory header
//! and emit the data unit through one preflighted HDU transaction (which pads to the
//! block grid and embeds `CHECKSUM`/`DATASUM` when enabled), assembling each unit in
//! the writer's reused `scratch`. [`FitsWriter::write_raw_hdu`] is the low-level
//! escape hatch for callers supplying a complete header and already-encoded data
//! unit themselves.

use std::io::{Seek, SeekFrom, Write};
use std::mem;

use crate::block;
use crate::block::{BLOCK_SIZE, SPACE_FILL, ZERO_FILL};
use crate::checksum::running::Running;
#[cfg(feature = "compression")]
use crate::compression::encoded_unit::EncodedUnit;
#[cfg(feature = "compression")]
use crate::compression::{CompressionConfig, encode};
use crate::fits_error::{FitsError, Result, SizeUnit};
use crate::fits_writer::ascii_builder::AsciiBuilder;
use crate::fits_writer::builder::{Builder, LaidOut, TableLayout};
use crate::fits_writer::image_writer::ImageWriter;
use crate::hdu::hdu_kind::{HduKind, HduPosition};
use crate::hdu::prologue::{Opening, Prologue};
use crate::header_model::{Header, Signing, Space, keywords};
use crate::image_model::bitpix::Bitpix;
use crate::image_model::view::{Borrowed, View};
use crate::image_model::{Metadata, check_sample_count};
use crate::reserved_keywords;

pub(crate) mod ascii_builder;
#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod builder;
mod column_scaling;
pub(crate) mod image_writer;

/// Writes FITS HDUs to a byte sink. The first HDU written becomes the primary
/// array; subsequent images/tables are written as extensions.
#[derive(Debug)]
pub struct FitsWriter<W> {
    sink: W,
    state: WriterState,
    form: HeaderForm,
    /// Reused buffer the data unit is assembled into before padding + writing, so
    /// writing many HDUs allocates no per-call staging. Each high-level write
    /// `clear`s it, builds the unit, and hands it to the HDU commit path.
    scratch: Vec<u8>,
    /// Reused block-padded header serialization, kept separate because checksum
    /// generation needs the header and data bytes alive at the same time.
    header_scratch: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterState {
    Empty,
    Active,
    Failed,
}

/// What every header the writer renders carries besides its records: the checksums,
/// and the blank records before `END`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeaderForm {
    pub(crate) checksums: Checksums,
    pub(crate) spare_records: usize,
}

/// Whether each HDU carries `DATASUM` and `CHECKSUM` (§J).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Checksums {
    Off,
    On,
}

/// The data unit an HDU write commits, before its block fill.
#[derive(Debug, Clone, Copy)]
enum Body<'a> {
    /// Bytes already encoded: a caller's raw unit, a table, a compressed container.
    Bytes(&'a [u8]),
    /// Samples, encoded to big-endian in bounded chunks as they are summed and written.
    Samples(View<'a>),
    /// A binary table, encoded in bounded chunks as it is summed and written.
    Table {
        table: &'a Builder,
        layout: &'a TableLayout,
    },
    /// A compressed container's main-table rows, then its heap's cells in order.
    #[cfg(feature = "compression")]
    Parts { rows: &'a [u8], heap: &'a [Vec<u8>] },
}

/// The samples a body encodes at a time: a bounded scratch, whatever the image's size.
const CHUNK_SAMPLES: usize = 1 << 17;

/// The bytes a body gathers to before one write: a binary table's bands of rows and its
/// arrays, a compressed body's small cells.
const CHUNK_BYTES: usize = 1 << 20;

/// The block fill of each kind of data unit (§3.3.2, §7.2.3).
const SPACES: [u8; BLOCK_SIZE] = [SPACE_FILL; BLOCK_SIZE];
const ZEROS: [u8; BLOCK_SIZE] = [ZERO_FILL; BLOCK_SIZE];

impl<W: Write> FitsWriter<W> {
    pub const fn new(sink: W) -> Self {
        Self {
            sink,
            state: WriterState::Empty,
            form: HeaderForm {
                checksums: Checksums::Off,
                spare_records: 0,
            },
            scratch: Vec::new(),
            header_scratch: Vec::new(),
        }
    }

    /// A writer that continues a file whose primary is written, at the sink's position.
    pub(crate) const fn continuing(sink: W) -> Self {
        let mut writer = Self::new(sink);
        writer.state = WriterState::Active;
        writer
    }

    /// Enable `DATASUM`/`CHECKSUM` integrity keywords on every HDU written through
    /// this writer (§J), including [`FitsWriter::write_raw_hdu`].
    #[must_use]
    pub const fn with_checksums(mut self) -> Self {
        self.form.checksums = Checksums::On;
        self
    }

    /// Write at least `records` blank records before each header's `END`, so a later
    /// [`FitsEditor::replace_header`](crate::FitsEditor::replace_header) with more
    /// records fits in place, as cfitsio's `ffhdef` reserves space.
    #[must_use]
    pub const fn with_spare_records(mut self, records: usize) -> Self {
        self.form.spare_records = records;
        self
    }

    /// Write one complete raw HDU after validating the header's role and exact
    /// unpadded data length. The block fill is derived from the HDU kind: spaces
    /// for an ASCII table and NULs for every other data unit. An extension header
    /// written first follows an empty primary, as a table written first does.
    pub fn write_raw_hdu(&mut self, header: &Header, raw: &[u8]) -> Result<()> {
        // No template: this is the escape hatch for a caller supplying a complete
        // header, so there is nothing to merge into it.
        self.finish_hdu(header, None, Body::Bytes(raw))
    }

    /// Where the next HDU goes: the primary of an empty file, an extension after it.
    /// Errors with [`FitsError::WriterFailed`] after a failed write.
    const fn next_position(&self) -> Result<HduPosition> {
        match self.state {
            WriterState::Empty => Ok(HduPosition::Primary),
            WriterState::Active => Ok(HduPosition::Extension),
            WriterState::Failed => Err(FitsError::WriterFailed),
        }
    }

    /// [`FitsError::WriterFailed`] after a failed write.
    const fn ensure_usable(&self) -> Result<()> {
        match self.state {
            WriterState::Failed => Err(FitsError::WriterFailed),
            WriterState::Empty | WriterState::Active => Ok(()),
        }
    }

    /// Write `bytes` to the sink. Every byte the writer writes passes here: a failed
    /// write poisons the writer, and a poisoned one writes nothing more.
    fn emit(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_usable()?;
        if let Err(error) = self.sink.write_all(bytes) {
            self.state = WriterState::Failed;
            return Err(FitsError::Io(error));
        }
        Ok(())
    }

    /// Mark the writer unusable: a stream dropped before its finish leaves a partial HDU.
    const fn poison(&mut self) {
        self.state = WriterState::Failed;
    }

    /// Commit one HDU: merge the caller's informational `template` into `header`, then
    /// render everything fallible before the first byte, then write the empty primary an
    /// extension written first needs, the header, `body` and its fill.
    ///
    /// The template merge lives here rather than in each header builder, so every
    /// write that commits an HDU applies it and none of the builders has to remember
    /// to. The one write that does not commit through here is [`ImageWriter`], which
    /// writes its header up front and rewrites it at `finish`; it merges its own.
    ///
    /// Errors with [`FitsError::DataSizeMismatch`] when the header's data unit is not
    /// `body`'s length, which only a raw write's caller can give.
    fn finish_hdu(
        &mut self,
        header: &Header,
        template: Option<&Header>,
        body: Body<'_>,
    ) -> Result<()> {
        let next = self.next_position()?;
        let merged;
        let header = match template {
            Some(template) => {
                let mut with_template = header.clone();
                merge_header_template(&mut with_template, template);
                merged = with_template;
                &merged
            }
            None => header,
        };
        let position = placed(header, next)?;
        let kind = HduKind::from_header(header, position)?;
        let data_bytes = kind.data_bytes(header)?;
        let body_len = body.byte_len()?;
        if u64::try_from(body_len).ok() != Some(data_bytes) {
            let expected = usize::try_from(data_bytes)
                .map_err(|_| FitsError::DataUnitTooLarge { bytes: data_bytes })?;
            return Err(SizeUnit::Bytes.mismatch(expected, body_len));
        }
        let fill = if kind == HduKind::AsciiTable {
            &SPACES
        } else {
            &ZEROS
        };
        let fill = &fill[..block::padding(body_len)];
        let signing = match self.form.checksums {
            Checksums::Off => Signing::Unsigned,
            Checksums::On => {
                let mut sum = Running::new();
                self.sum_body(body, &mut sum);
                sum.update(fill);
                Signing::WithDataSum(sum.finish())
            }
        };
        let space = Space::Spare(self.form.spare_records);
        let primary = if next == HduPosition::Primary && position == HduPosition::Extension {
            let signing = match self.form.checksums {
                Checksums::Off => Signing::Unsigned,
                Checksums::On => Signing::WithDataSum(0),
            };
            let mut primary = Vec::new();
            empty_primary_header()?.render_into(signing, space, &mut primary)?;
            Some(primary)
        } else {
            None
        };
        let mut header_bytes = mem::take(&mut self.header_scratch);
        let written = header
            .render_into(signing, space, &mut header_bytes)
            .and_then(|()| {
                if let Some(primary) = &primary {
                    self.emit(primary)?;
                }
                self.emit(&header_bytes)?;
                self.emit_body(body)?;
                self.emit(fill)
            });
        self.header_scratch = header_bytes;
        written?;
        self.state = WriterState::Active;
        Ok(())
    }

    /// Add the bytes of `body` to `sum`.
    fn sum_body(&mut self, body: Body<'_>, sum: &mut Running) {
        match body {
            Body::Bytes(bytes) => sum.update(bytes),
            #[cfg(feature = "compression")]
            Body::Parts { rows, heap } => {
                sum.update(rows);
                for cell in heap {
                    sum.update(cell);
                }
            }
            Body::Samples(samples) => self
                .each_chunk(samples, |_, chunk| {
                    sum.update(chunk);
                    Ok(())
                })
                .expect("a sum fails on no chunk"),
            Body::Table { table, layout } => {
                let mut chunk = mem::take(&mut self.scratch);
                table
                    .encode_chunks(layout, CHUNK_BYTES, &mut chunk, |bytes| {
                        sum.update(bytes);
                        Ok(())
                    })
                    .expect("the layout checked each descriptor, and a sum fails on no chunk");
                self.scratch = chunk;
            }
        }
    }

    /// Write the bytes of `body`.
    fn emit_body(&mut self, body: Body<'_>) -> Result<()> {
        match body {
            Body::Bytes(bytes) => self.emit(bytes),
            #[cfg(feature = "compression")]
            Body::Parts { rows, heap } => self.emit_parts(rows, heap),
            Body::Samples(samples) => self.emit_samples(samples, None),
            Body::Table { table, layout } => {
                let mut chunk = mem::take(&mut self.scratch);
                let written =
                    table.encode_chunks(layout, CHUNK_BYTES, &mut chunk, |bytes| self.emit(bytes));
                self.scratch = chunk;
                written
            }
        }
    }

    /// Write `rows`, then each cell of `heap`: small cells gather in the scratch buffer to
    /// [`CHUNK_BYTES`] bytes before one write, so a heap of many small tiles makes few
    /// writes, and a large cell goes out as it is.
    #[cfg(feature = "compression")]
    fn emit_parts(&mut self, rows: &[u8], heap: &[Vec<u8>]) -> Result<()> {
        let mut chunk = mem::take(&mut self.scratch);
        chunk.clear();
        let mut result = Ok(());
        for part in std::iter::once(rows).chain(heap.iter().map(Vec::as_slice)) {
            if chunk.len() + part.len() > CHUNK_BYTES && !chunk.is_empty() {
                result = self.emit(&chunk);
                chunk.clear();
            }
            if result.is_ok() {
                if part.len() >= CHUNK_BYTES {
                    result = self.emit(part);
                } else {
                    chunk.extend_from_slice(part);
                }
            }
            if result.is_err() {
                break;
            }
        }
        if result.is_ok() && !chunk.is_empty() {
            result = self.emit(&chunk);
        }
        self.scratch = chunk;
        result
    }

    /// Encode `samples` to big-endian in bounded chunks, add each to `sum` when given, and
    /// write it.
    fn emit_samples(&mut self, samples: View<'_>, mut sum: Option<&mut Running>) -> Result<()> {
        self.each_chunk(samples, |writer, chunk| {
            if let Some(sum) = sum.as_deref_mut() {
                sum.update(chunk);
            }
            writer.emit(chunk)
        })
    }

    /// Encode `samples` to big-endian in chunks of `scratch`, of `CHUNK_SAMPLES` each, and
    /// give each to `each` until one fails.
    fn each_chunk(
        &mut self,
        samples: View<'_>,
        mut each: impl FnMut(&mut Self, &[u8]) -> Result<()>,
    ) -> Result<()> {
        let mut chunk = mem::take(&mut self.scratch);
        let mut result = Ok(());
        for start in (0..samples.len()).step_by(CHUNK_SAMPLES) {
            chunk.clear();
            let end = (start + CHUNK_SAMPLES).min(samples.len());
            samples.slice(start..end).encode_into(&mut chunk);
            result = each(self, &chunk);
            if result.is_err() {
                break;
            }
        }
        self.scratch = chunk;
        result
    }

    /// Render `header` with `signing` and write it.
    fn emit_header(&mut self, header: &Header, signing: Signing) -> Result<()> {
        let mut header_bytes = mem::take(&mut self.header_scratch);
        let written = header
            .render_into(
                signing,
                Space::Spare(self.form.spare_records),
                &mut header_bytes,
            )
            .and_then(|()| self.emit(&header_bytes));
        self.header_scratch = header_bytes;
        written
    }

    /// Write the compressed HDU that `encode` builds, as its parts: the container rows and
    /// the heap cells.
    #[cfg(feature = "compression")]
    fn write_unit(
        &mut self,
        encode: impl FnOnce() -> Result<EncodedUnit>,
        template: Option<&Header>,
    ) -> Result<()> {
        self.next_position()?;
        let unit = encode()?;
        self.finish_hdu(
            &unit.header,
            template,
            Body::Parts {
                rows: &unit.rows,
                heap: &unit.heap,
            },
        )
    }

    /// Write a table HDU whose data unit `encode` writes into the scratch, giving the
    /// header that declares it.
    fn write_encoded(
        &mut self,
        encode: impl FnOnce(&mut Vec<u8>) -> Result<Header>,
        template: Option<&Header>,
    ) -> Result<()> {
        self.next_position()?;
        let mut data = mem::take(&mut self.scratch);
        let written = encode(&mut data)
            .and_then(|header| self.finish_hdu(&header, template, Body::Bytes(&data)));
        self.scratch = data;
        written
    }

    /// Consume the writer and return the underlying sink. HDUs are written eagerly,
    /// so an unbuffered sink (e.g. a `File`) holds the complete file. This does **not**
    /// flush: if the sink is a `BufWriter`, flush it (or rely on its `Drop`) before
    /// trusting the bytes, and check the flush result if you need write errors surfaced.
    pub fn into_inner(self) -> W {
        self.sink
    }

    /// Write an ASCII table as a `TABLE` extension (a dataless primary is written
    /// first if needed). Columns are packed left-to-right with no gaps; data is
    /// space-padded per §7.2.3. The non-structural cards of `header` are kept; the
    /// table-layout and checksum cards are generated from `table`.
    pub fn write_ascii_table(
        &mut self,
        table: &AsciiBuilder,
        header: Option<&Header>,
    ) -> Result<()> {
        self.write_encoded(|data| table.encode(data), header)
    }

    /// Write a binary table as a `BINTABLE` extension. A dataless primary HDU is
    /// written automatically first if nothing has been written yet (a table can
    /// never be the primary HDU). Fixed-width and variable-length (`P`/`Q`) columns
    /// are both supported, including jagged `PX`/`QX` bit arrays — VLA columns
    /// write a heap after the main table. The non-structural cards of `header` are
    /// kept; the table-layout and checksum cards are generated from `table`.
    pub fn write_table(&mut self, table: &Builder, header: Option<&Header>) -> Result<()> {
        self.next_position()?;
        let LaidOut {
            header: table_header,
            layout,
        } = table.laid_out()?;
        let body = Body::Table {
            table,
            layout: &layout,
        };
        self.finish_hdu(&table_header, header, body)
    }

    /// Write `image` as the primary HDU (first call) or an `IMAGE` extension
    /// (later calls). The mandatory header is synthesized (`SIMPLE`/`XTENSION`,
    /// `BITPIX`, `NAXISn`, plus `BSCALE`/`BZERO`/`BLANK` when scaling is
    /// non-trivial), followed by the big-endian data unit. The non-structural cards
    /// of `header` are kept; the image-layout and checksum cards are generated from
    /// `image`. An owned [`Image`](crate::image::Image) lends one with `view`.
    pub fn write_image(&mut self, image: Borrowed<'_>, header: Option<&Header>) -> Result<()> {
        debug_assert!(
            check_sample_count(image.shape, image.samples.len()).is_ok(),
            "a borrowed image's shape fixes its sample count"
        );
        let image_header = image_writer::image_header(image.metadata(), self.next_position()?)?;
        self.finish_hdu(&image_header, header, Body::Samples(image.samples))
    }

    /// Write a primary HDU with no data (`NAXIS = 0`) that carries the non-structural
    /// cards of `header`. Errors with [`FitsError::PrimaryAlreadyWritten`] after any
    /// other HDU. A table or a compressed image written first gets an empty primary
    /// without this call.
    pub fn write_empty_primary(&mut self, header: Option<&Header>) -> Result<()> {
        if self.next_position()? != HduPosition::Primary {
            return Err(FitsError::PrimaryAlreadyWritten);
        }
        self.finish_hdu(&empty_primary_header()?, header, Body::Bytes(&[]))
    }

    /// Write `image` as a tiled-compressed `BINTABLE` extension (§10.1), with the
    /// codec, tiles and quantization of `config`. Requires the `compression` feature.
    /// Integer images support `GZIP_1`/`GZIP_2`/`RICE_1`/`PLIO_1`/`HCOMPRESS_1`; float
    /// images are quantized with the dither method `config` selects
    /// (`SUBTRACTIVE_DITHER_1` by default) and compressed with `GZIP_1`/`GZIP_2`/
    /// `RICE_1`/`HCOMPRESS_1`, or stored with `NOCOMPRESS`; with `Quantization::NONE`
    /// they are stored losslessly with `GZIP_1`/`GZIP_2`. `HCOMPRESS_1` needs a 2-D tile
    /// shape, and every `PLIO_1` mask sample must be in the lossless `0..=0xFF_FFFF`
    /// domain. The non-structural cards of `header` are kept; the container,
    /// compression, image-layout and checksum cards are generated from `image` and
    /// `config`.
    #[cfg(feature = "compression")]
    pub fn write_compressed_image(
        &mut self,
        image: Borrowed<'_>,
        config: &CompressionConfig,
        header: Option<&Header>,
    ) -> Result<()> {
        self.write_unit(|| encode::compress_image(image, config), header)
    }

    /// Write `table` as a tiled-compressed table (§10.3): the table [`Self::write_table`]
    /// writes, compressed in tiles of the rows `config` sets with its codec on every
    /// column (`GZIP_1`/`GZIP_2`/`RICE_1`, each clamped to one its type takes, so that
    /// cfitsio reads every column). Fixed-width and P/Q variable-length columns are
    /// supported. Errors when `config` has more than one tile axis or a quantization,
    /// or is `Compression::None`, which `write_table` is. The non-structural
    /// cards of `header` are kept. Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn write_compressed_table(
        &mut self,
        table: &Builder,
        config: &CompressionConfig,
        header: Option<&Header>,
    ) -> Result<()> {
        self.write_unit(|| table.compress(config, header), None)
    }
}

impl<W: Write + Seek> FitsWriter<W> {
    /// The sink's position, where the next byte lands.
    fn stream_position(&mut self) -> Result<u64> {
        Ok(self.sink.stream_position()?)
    }

    /// Render a stream's header again with `signing` where it was written, then return to
    /// the end of what is written. A failed seek or write poisons the writer.
    fn rewrite_header(&mut self, header: &StreamHeader, signing: Signing) -> Result<()> {
        let rewritten = (|| {
            let end = self.sink.stream_position()?;
            self.sink.seek(SeekFrom::Start(header.offset))?;
            self.emit_header(&header.header, signing)?;
            self.sink.seek(SeekFrom::Start(end))?;
            Ok(())
        })();
        if rewritten.is_err() {
            self.poison();
        }
        rewritten
    }

    /// Write the header of a stream at the sink's position, signed with placeholders when
    /// the writer writes checksums, and give it with that position.
    fn start_stream(&mut self, header: Header) -> Result<StreamHeader> {
        let offset = self.stream_position()?;
        let signing = match self.form.checksums {
            Checksums::Off => Signing::Unsigned,
            Checksums::On => Signing::Placeholder,
        };
        self.emit_header(&header, signing)?;
        self.state = WriterState::Active;
        Ok(StreamHeader { header, offset })
    }

    /// End a stream of `data_bytes` whose bytes `sum` holds: write the block fill, and
    /// sign its header again when the writer writes checksums.
    fn finish_stream(
        &mut self,
        header: &StreamHeader,
        data_bytes: usize,
        sum: &mut Running,
    ) -> Result<()> {
        let fill = &ZEROS[..block::padding(data_bytes)];
        self.emit(fill)?;
        sum.update(fill);
        if self.form.checksums == Checksums::On {
            self.rewrite_header(header, Signing::WithDataSum(sum.finish()))?;
        }
        Ok(())
    }

    /// Begin a large image write of `layout`: its shape, `BITPIX` and scaling. The
    /// returned stream must receive exactly the axis-product sample count and be
    /// finished successfully. The non-structural cards of `header` are kept; the
    /// structural and checksum cards are generated.
    pub fn stream_image(
        &mut self,
        layout: Metadata<'_>,
        header: Option<&Header>,
    ) -> Result<ImageWriter<'_, W>> {
        ImageWriter::new(self, layout, header)
    }
}

/// A stream's header and the offset it is written at, which `finish` signs again.
#[derive(Debug)]
struct StreamHeader {
    header: Header,
    offset: u64,
}

/// Where `header` goes, when the next HDU is at `next`: an extension header written first
/// goes after an empty primary, as a table or a compressed image does.
fn placed(header: &Header, next: HduPosition) -> Result<HduPosition> {
    if next == HduPosition::Primary && header.get(keywords::XTENSION)?.is_some() {
        Ok(HduPosition::Extension)
    } else {
        Ok(next)
    }
}

/// A dataless primary HDU (`NAXIS = 0`), written before extensions when the
/// caller's first HDU is itself an extension.
fn empty_primary_header() -> Result<Header> {
    Prologue {
        opening: Opening::Primary,
        bitpix: Bitpix::U8,
        axes: &[],
        pcount: 0,
    }
    .header()
}

impl Body<'_> {
    /// The bytes of the unit, before its fill.
    fn byte_len(&self) -> Result<usize> {
        match self {
            Self::Bytes(bytes) => Ok(bytes.len()),
            #[cfg(feature = "compression")]
            Self::Parts { rows, heap } => heap
                .iter()
                .try_fold(rows.len(), |len, cell| len.checked_add(cell.len()))
                .ok_or(FitsError::DataUnitOverflow),
            Self::Samples(samples) => samples
                .len()
                .checked_mul(samples.bitpix().elem_size())
                .ok_or(FitsError::DataUnitOverflow),
            Self::Table { layout, .. } => Ok(layout.data_len()),
        }
    }
}

/// Reconcile one column's implied row count with the table's — the inference rule
/// [`Builder`] and [`AsciiBuilder`] share: the first column to imply
/// a count fixes it, every later column must agree, and a column that implies
/// nothing at all is acceptable only once the count is already known.
///
/// `inferred` is `None` only for a binary column whose payload determines no count
/// (an empty or zero-width one). An ASCII column is always scalar, so its row count
/// is its value count and it never reaches the undetermined case.
fn accept_row_count(
    row_count: &mut Option<usize>,
    column: &str,
    inferred: Option<usize>,
) -> Result<()> {
    match (*row_count, inferred) {
        (Some(expected), Some(got)) if expected != got => Err(FitsError::TableRowCountMismatch {
            column: column.to_string(),
            expected,
            got,
        }),
        (None, Some(rows)) => {
            *row_count = Some(rows);
            Ok(())
        }
        (None, None) => Err(FitsError::TableRowCountUndetermined {
            column: column.to_string(),
        }),
        _ => Ok(()),
    }
}

/// [`accept_row_count`] holds every column a builder takes to the table's row count.
const ROW_COUNT_FIXED: &str = "the table builder fixes every column's row count";

/// Append the records of `template` that `header`'s writer does not generate.
fn merge_header_template(header: &mut Header, template: &Header) {
    header.append_filtered_from(template, |keyword| {
        !reserved_keywords::is_generated(keyword)
    });
}

#[cfg(feature = "internals")]
pub(crate) mod internals {
    use crate::fits_writer::{FitsWriter, WriterState};

    impl<W> FitsWriter<W> {
        /// Whether the writer has written nothing, so its next write makes the primary.
        pub(crate) const fn is_empty(&self) -> bool {
            matches!(self.state, WriterState::Empty)
        }
    }
}

#[cfg(test)]
mod tests;
