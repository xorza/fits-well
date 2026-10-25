//! What the read-bytes tests share: the images they write, and the run of views they
//! measure against the counting allocator's live bytes and peak.

use std::io::Cursor;
use std::ops::Range;

use fits_well::compress::{Compression, CompressionConfig, Dither, Hcompress, Quantization};
use fits_well::image::{Data, Image, Region, Scaling};
use fits_well::internals::INFLATE_STATE_BYTES;
use fits_well::io::{BLOCK_SIZE, SliceSource, StreamSource};
use fits_well::{FitsReader, FitsWriter, ImageReader, ViewBytes};

use crate::support::counting::ALLOCATOR;

/// The image every case writes: 61 × 47, so no tiling divides it, of values that keep
/// each codec's tiles of unequal sizes.
const SHAPE: [usize; 2] = [61, 47];

fn integers() -> Vec<i16> {
    (0..SHAPE[0] * SHAPE[1])
        .map(|i| ((i * 7919 + 13) % 2001) as i16 - 1000)
        .collect()
}

fn mask() -> Vec<i32> {
    (0..SHAPE[0] * SHAPE[1])
        .map(|i| i32::from(i % 13 < 4))
        .collect()
}

fn floats() -> Vec<f32> {
    (0..SHAPE[0] * SHAPE[1])
        .map(|i| (i as f32 * 0.37).sin() * 100.0)
        .collect()
}

/// A file of one tiled-compressed image of `samples`, in `tiles`, through `compression`;
/// a float image quantized as `quantization` says.
fn written(
    samples: Data,
    tiles: [usize; 2],
    compression: Compression,
    quantization: Option<Quantization>,
) -> Vec<u8> {
    let image = Image::new(SHAPE, samples, Scaling::IDENTITY).unwrap();
    let mut config = CompressionConfig::new(compression).with_tiles(tiles);
    if let Some(quantization) = quantization {
        config = config.with_quantization(quantization);
    }
    let mut writer = FitsWriter::new(Vec::new());
    writer.write_empty_primary(None).unwrap();
    writer
        .write_compressed_image(image.view(), &config, None)
        .unwrap();
    writer.into_inner()
}

/// One case: its name, its file, and what its tile sets inflate with.
#[derive(Debug)]
pub(crate) struct Case {
    name: String,
    bytes: Vec<u8>,
    inflate: Inflate,
}

/// How a case's tile sets meet the inflate state the figure counts for a container with a
/// gzip column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inflate {
    /// No gzip column: the figure counts none.
    None,
    /// A gzip column no tile uses: the figure counts the bound, and no set makes one.
    Unused,
    /// Gzip tiles: the figure counts the bound, and a set makes the state its first meets.
    Used,
}

/// Every case's file: row tiles, square tiles and one tile, each through every codec, for
/// an integer, a quantized float and a lossless float image.
pub(crate) fn cases() -> Vec<Case> {
    let tilings = [("row", [SHAPE[0], 1]), ("square", [16, 16]), ("one", SHAPE)];
    let quantized = Quantization::relative(4.0, Dither::default()).unwrap();
    let mut cases = Vec::new();
    for (tiling, tiles) in tilings {
        for (codec, compression) in [
            ("GZIP_1", Compression::GZIP),
            ("GZIP_2", Compression::GZIP_SHUFFLED),
            ("RICE_1", Compression::Rice),
            ("HCOMPRESS_1", Compression::Hcompress(Hcompress::default())),
            ("NOCOMPRESS", Compression::None),
        ] {
            if tiling == "row" && codec == "HCOMPRESS_1" {
                // HCOMPRESS takes no tile a row high.
                continue;
            }
            cases.push(Case {
                name: format!("{tiling} {codec} i16"),
                bytes: written(integers().into(), tiles, compression, None),
                inflate: if codec.starts_with("GZIP") {
                    Inflate::Used
                } else {
                    Inflate::None
                },
            });
        }
        cases.push(Case {
            name: format!("{tiling} PLIO_1 i32"),
            bytes: written(mask().into(), tiles, Compression::Plio, None),
            inflate: Inflate::None,
        });
        // A quantized image's container has a gzip column, for the tiles it stores
        // losslessly, which these are not.
        cases.push(Case {
            name: format!("{tiling} RICE_1 f32"),
            bytes: written(floats().into(), tiles, Compression::Rice, Some(quantized)),
            inflate: Inflate::Unused,
        });
        let lossless = Some(Quantization::NONE);
        cases.push(Case {
            name: format!("{tiling} GZIP_1 lossless f32"),
            bytes: written(floats().into(), tiles, Compression::GZIP, lossless),
            inflate: Inflate::Used,
        });
    }
    cases
}

/// The regions a run views in turn, in two orders. Chunks of 16 whole rows, as a chunked
/// reader takes them, then the whole image, then a section that touches more tiles than a
/// chunk: the buffers the reader keeps grow twice, and a stream's copy of the whole unit
/// comes after the largest decode, so the figure, which adds each buffer's largest and the
/// largest decode, is above the peak. The same, the whole image first and the section
/// next: each buffer grows once, before the largest decode, which the section's is.
fn orders() -> [[Vec<Range<usize>>; 5]; 2] {
    let rows = |rows: Range<usize>| vec![0..SHAPE[0], rows];
    let section = vec![5..40, 7..30];
    [
        [
            rows(0..16),
            rows(16..32),
            rows(32..47),
            Vec::new(),
            section.clone(),
        ],
        [Vec::new(), section, rows(0..16), rows(16..32), rows(32..47)],
    ]
}

/// Open the image of each case's file, from memory and from a stream, view the regions of
/// each of [`orders`] in turn, and, when `measured`, hold the peak above what the reader
/// held before the image opened to `view_bytes`: never past it. On one worker, which
/// makes the one tile set the figure counts, and in the order that grows each buffer
/// once, the figure is the peak, the room the inflate state's bound leaves for a case
/// whose container has a gzip column — all of it where no tile inflates — and for a
/// stream at most the header block its staging buffer held.
/// An empty region list stands for the whole image. The first pass warms what a process
/// makes once — the pool's workers, the dither table.
pub(crate) fn check(cases: &[Case]) {
    #[cfg(feature = "parallel")]
    let workers = rayon::current_num_threads();
    #[cfg(not(feature = "parallel"))]
    let workers = 1;
    let before = ALLOCATOR.live();
    let inflate = flate2::Decompress::new(false);
    let room = INFLATE_STATE_BYTES as isize - (ALLOCATOR.live() - before);
    drop(inflate);
    let mut words = Vec::with_capacity(SHAPE[0] * SHAPE[1]);
    for measured in [false, true] {
        for case in cases {
            for (order, regions) in orders().iter().enumerate() {
                for stream in [false, true] {
                    let held = measure(&case.bytes, stream, regions, &mut words);
                    if !measured {
                        continue;
                    }
                    let source = if stream { "stream" } else { "memory" };
                    let name = format!("{} from {source}, order {order}", case.name);
                    assert!(
                        held.finish_peak <= held.finish_figure,
                        "{name}: the finish held {} B at its peak, past its {} B",
                        held.finish_peak,
                        held.finish_figure
                    );
                    assert!(
                        held.peak <= held.figure,
                        "{name}: held {} B at its peak, past its {} B",
                        held.peak,
                        held.figure
                    );
                    if workers == 1 && order == 1 {
                        let room = match case.inflate {
                            Inflate::None => 0,
                            Inflate::Unused => INFLATE_STATE_BYTES as isize,
                            Inflate::Used => room,
                        };
                        let over = held.figure - held.peak - room;
                        // A stream's staging buffer holds the header block the scan read,
                        // which it holds beside its new block while it grows for the whole
                        // image: the figure counts it beside the buffers that grow later.
                        let block = if stream { BLOCK_SIZE as isize } else { 0 };
                        assert!(
                            (0..=block).contains(&over),
                            "{name}: {over} B over the peak, past {block} B"
                        );
                    }
                }
            }
        }
    }
}

/// What a run of views held above what the reader held before the image opened, and
/// what `view_bytes` said it would; and what the finish of a sum that holds nothing of the
/// views added at its peak, and what `finish_bytes` said it would.
#[derive(Debug)]
struct Held {
    peak: isize,
    figure: isize,
    finish_peak: isize,
    finish_figure: isize,
}

/// View `regions` in turn of the image of `bytes`, read from a stream or from memory.
fn measure(
    bytes: &[u8],
    stream: bool,
    regions: &[Vec<Range<usize>>],
    words: &mut Vec<u64>,
) -> Held {
    if stream {
        let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
        let before = ALLOCATOR.live();
        views(reader.image(1).unwrap(), before, regions, words)
    } else {
        let mut reader = FitsReader::from_bytes(bytes).unwrap();
        let before = ALLOCATOR.live();
        views(reader.image(1).unwrap(), before, regions, words)
    }
}

fn views(
    mut image: impl Views,
    before: isize,
    regions: &[Vec<Range<usize>>],
    words: &mut Vec<u64>,
) -> Held {
    let figure = image.figure(regions);
    let _ = ALLOCATOR.restart_peak();
    for ranges in regions {
        image.view_region(ranges, words);
    }
    let peak = ALLOCATOR.peak() - before;
    let finish_figure = isize::try_from(image.finish_figure()).unwrap();
    let before_finish = ALLOCATOR.restart_peak();
    image.finish();
    Held {
        peak,
        figure: isize::try_from(figure.kept + figure.handle).unwrap(),
        finish_peak: ALLOCATOR.peak() - before_finish,
        finish_figure,
    }
}

/// The region `ranges` stands for: the whole image for none.
fn region(ranges: &[Range<usize>]) -> Region<'_> {
    if ranges.is_empty() {
        Region::All
    } else {
        Region::from(ranges)
    }
}

/// The calls [`views`] makes, on a reader of either source.
trait Views {
    /// [`ImageReader::view_bytes`] of `regions`.
    fn figure(&self, regions: &[Vec<Range<usize>>]) -> ViewBytes;
    /// [`ImageReader::view`] of `ranges`.
    fn view_region(&mut self, ranges: &[Range<usize>], words: &mut Vec<u64>);
    /// [`ImageReader::finish_bytes`].
    fn finish_figure(&self) -> u64;
    /// [`ImageReader::finish`].
    fn finish(self);
}

impl Views for ImageReader<'_, SliceSource<'_>> {
    fn figure(&self, regions: &[Vec<Range<usize>>]) -> ViewBytes {
        self.view_bytes(regions.iter().map(|ranges| region(ranges)))
            .unwrap()
    }

    fn view_region(&mut self, ranges: &[Range<usize>], words: &mut Vec<u64>) {
        self.view(region(ranges), words).unwrap();
    }

    fn finish_figure(&self) -> u64 {
        self.finish_bytes()
    }

    fn finish(self) {
        ImageReader::finish(self).unwrap();
    }
}

impl Views for ImageReader<'_, StreamSource<Cursor<&[u8]>>> {
    fn figure(&self, regions: &[Vec<Range<usize>>]) -> ViewBytes {
        self.view_bytes(regions.iter().map(|ranges| region(ranges)))
            .unwrap()
    }

    fn view_region(&mut self, ranges: &[Range<usize>], words: &mut Vec<u64>) {
        self.view(region(ranges), words).unwrap();
    }

    fn finish_figure(&self) -> u64 {
        self.finish_bytes()
    }

    fn finish(self) {
        ImageReader::finish(self).unwrap();
    }
}
