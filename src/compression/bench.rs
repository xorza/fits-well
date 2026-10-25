//! The codec cases: each codec of the storage table on one tile, its encoder and its decoder,
//! and the quantizer and dequantizer of a float row. The inputs are the scene's, never random
//! bytes, as a codec's speed follows its data. Compressed files are read and written by the
//! read and write cases, beside the plain ones, in every storage.

use std::hint::black_box;

use criterion::Criterion;

use crate::bench::policy::{self, Kind, Scale};
use crate::bench::scene_file::Storage;
use crate::compression::decode::float_quantization::Dequant;
use crate::compression::encode::hcompress_default_rows;
use crate::compression::half_step::assert_within_half_step;
use crate::compression::plane::{FloatSamples, IntBitpix};
use crate::compression::quantization::{Quantization, Spacing};
use crate::compression::quantize::{self, Precision, QuantizeScratch, Quantizer, TileValues};
use crate::compression::tile_codec::{CodecScratch, Tile, TileCodec, TileSamples};
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::internals::scene;
use crate::internals::scene::{HEIGHT, WIDTH};

/// `atomic/compress/<storage>/<input>` and `atomic/decompress/<storage>/<input>`: one tile of
/// each codec's default tiling of the standard frame ([`tile_rows`] rows) through its tile
/// encoder and decoder, into reused buffers. The integer codecs take the light at i16 and the
/// float frame quantized to i32, as a quantized float reaches them; `plio_1` the mask; and
/// `gzip_2_lossless` the float frame's bytes. A decode case first checks that the tile
/// round-trips, as each codec is lossless on its integers. Per sample, in the stored bytes.
///
/// `atomic/quantize/f32` and `atomic/dequantize/f32`: one row of the float frame at the
/// default quantization, to its integers and back, checked once to be within half a step.
/// Per sample.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "compress", Scale::Row);
    for (storage, input) in codec_cases() {
        let rows = tile_rows(storage);
        group.throughput(policy::throughput(WIDTH * rows, input.elem_size()));
        group.bench_function(format!("{}/{}", storage.id(), input.id()), |b| {
            let mut tile = CodecTile::new(storage, input);
            b.iter(|| tile.encode());
        });
    }
    group.finish();

    let mut group = Kind::Atomic.group(c, "decompress", Scale::Row);
    for (storage, input) in codec_cases() {
        let rows = tile_rows(storage);
        group.throughput(policy::throughput(WIDTH * rows, input.elem_size()));
        group.bench_function(format!("{}/{}", storage.id(), input.id()), |b| {
            let mut tile = CodecTile::new(storage, input);
            tile.encode();
            tile.assert_round_trips();
            b.iter(|| tile.decode());
        });
    }
    group.finish();

    let float_id = policy::bitpix_id(Bitpix::F32);
    let mut group = Kind::Atomic.group(c, "quantize", Scale::Row);
    group.throughput(policy::throughput(WIDTH, Bitpix::F32.elem_size()));
    group.bench_function(float_id, |b| {
        let mut row = QuantizedRow::new();
        b.iter(|| black_box(row.quantize()));
    });
    group.finish();

    let mut group = Kind::Atomic.group(c, "dequantize", Scale::Row);
    group.throughput(policy::throughput(WIDTH, Bitpix::F32.elem_size()));
    group.bench_function(float_id, |b| {
        let row = QuantizedRow::new();
        let mut dequantized = row.dequantize();
        row.assert_within_half_step(&dequantized);
        b.iter(|| {
            row.dequant.apply(&row.ints, &mut dequantized);
            black_box(&dequantized);
        });
    });
    group.finish();
}

/// What a codec case codes: a tile of a scene image as the write path hands it to the codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TileInput {
    /// The light, i16.
    Light,
    /// The float frame quantized at the default quantization, i32.
    Quantized,
    /// The defect mask, u8.
    Mask,
    /// The float frame's big-endian bytes, for a lossless float codec.
    FloatBytes,
}

impl TileInput {
    const fn id(self) -> &'static str {
        match self {
            Self::Light => "i16",
            Self::Quantized => "i32",
            Self::Mask => "mask",
            Self::FloatBytes => "f32",
        }
    }

    /// The integer width the codec takes the samples at.
    const fn width(self) -> IntBitpix {
        match self {
            Self::Light => IntBitpix::I16,
            Self::Quantized | Self::FloatBytes => IntBitpix::I32,
            Self::Mask => IntBitpix::U8,
        }
    }

    const fn elem_size(self) -> usize {
        self.width().elem_size()
    }
}

/// Each codec of the storage table and the inputs it takes.
fn codec_cases() -> Vec<(Storage, TileInput)> {
    let mut cases = Vec::new();
    for storage in [
        Storage::Gzip1,
        Storage::Gzip2,
        Storage::Rice1,
        Storage::Hcompress1,
    ] {
        cases.push((storage, TileInput::Light));
        cases.push((storage, TileInput::Quantized));
    }
    cases.push((Storage::Plio1, TileInput::Mask));
    cases.push((Storage::Gzip2Lossless, TileInput::FloatBytes));
    cases
}

/// The rows of one default tile of the standard frame: one, or cfitsio's `HCOMPRESS_1` height.
const fn tile_rows(storage: Storage) -> usize {
    match storage {
        Storage::Hcompress1 => hcompress_default_rows(HEIGHT),
        _ => 1,
    }
}

/// The samples of a tile, as the codec takes them.
#[derive(Debug)]
enum Samples {
    Ints(Vec<i64>),
    Bytes(Vec<u8>),
}

/// One tile of a scene image through the tile codec a write and a read take, and the
/// reused buffers it writes.
#[derive(Debug)]
struct CodecTile {
    codec: TileCodec,
    width: IntBitpix,
    shape: [usize; 2],
    samples: Samples,
    encoded: Vec<u8>,
    scratch: CodecScratch,
}

impl CodecTile {
    fn new(storage: Storage, input: TileInput) -> Self {
        let rows = tile_rows(storage);
        let samples = match input {
            TileInput::Light => match scene::sky(Bitpix::I16, WIDTH, rows, 1).samples {
                Data::I16(values) => Samples::Ints(values.into_iter().map(i64::from).collect()),
                _ => unreachable!("the light is i16"),
            },
            TileInput::Mask => match scene::mask(WIDTH, rows, 1).samples {
                Data::U8(values) => Samples::Ints(values.into_iter().map(i64::from).collect()),
                _ => unreachable!("the mask is u8"),
            },
            TileInput::Quantized => {
                let mut quantizer = QuantizedTile::new(rows);
                quantizer.quantize().expect("the float frame quantizes");
                let ints = quantizer.scratch.ints.iter().map(|&v| i64::from(v));
                Samples::Ints(ints.collect())
            }
            TileInput::FloatBytes => {
                let mut bytes = Vec::new();
                float_rows(rows).view().encode_into(&mut bytes);
                Samples::Bytes(bytes)
            }
        };
        let config = storage.config().expect("a codec storage");
        let codec = config.compression.tile_codec(input.width(), 0);
        Self {
            codec,
            width: input.width(),
            shape: [WIDTH, rows],
            samples,
            encoded: Vec::new(),
            scratch: CodecScratch::default(),
        }
    }

    /// Encodes the tile into [`Self::encoded`], as the tile encoder of `compress_image` does.
    fn encode(&mut self) {
        let samples = match &self.samples {
            Samples::Ints(values) => TileSamples::Ints(values),
            Samples::Bytes(bytes) => TileSamples::Bytes(bytes),
        };
        let stream = self
            .codec
            .encode(tile(&self.shape, self.width), samples, &mut self.scratch)
            .unwrap();
        self.encoded.clear();
        self.encoded.extend_from_slice(stream);
    }

    /// Decodes [`Self::encoded`] into the codec's reused buffers, as the tile decoder of a
    /// read does.
    fn decode(&mut self) {
        let tile = tile(&self.shape, self.width);
        match self.samples {
            Samples::Ints(_) => {
                black_box(
                    self.codec
                        .decode_ints(tile, &self.encoded, &mut self.scratch)
                        .unwrap(),
                );
            }
            Samples::Bytes(_) => {
                black_box(
                    self.codec
                        .decode_bytes(tile, &self.encoded, &mut self.scratch)
                        .unwrap(),
                );
            }
        }
    }

    /// Asserts that [`Self::encoded`] decodes to the tile's samples.
    fn assert_round_trips(&mut self) {
        let tile = tile(&self.shape, self.width);
        match &self.samples {
            Samples::Ints(values) => {
                let decoded = self
                    .codec
                    .decode_ints(tile, &self.encoded, &mut self.scratch)
                    .unwrap();
                assert_eq!(decoded, values, "the codec round-trips the tile");
            }
            Samples::Bytes(bytes) => {
                let decoded = self
                    .codec
                    .decode_bytes(tile, &self.encoded, &mut self.scratch)
                    .unwrap();
                assert_eq!(decoded, bytes.as_slice(), "the codec round-trips the tile");
            }
        }
    }
}

/// A tile of `shape` and `width` with no blank, as the codec takes it.
const fn tile(shape: &[usize; 2], width: IntBitpix) -> Tile<'_> {
    Tile {
        shape,
        width,
        blank: None,
    }
}

/// The first `rows` rows of the float frame.
fn float_rows(rows: usize) -> Data {
    scene::sky(Bitpix::F32, WIDTH, rows, 1).samples
}

/// The first tile of the float frame, `rows` rows, its values widened as the encoder gathers
/// them, and the quantizer of the default quantization with the seed the tile takes.
#[derive(Debug)]
struct QuantizedTile {
    floats: Vec<f32>,
    values: Vec<f64>,
    quantizer: Quantizer,
    scratch: QuantizeScratch,
}

impl QuantizedTile {
    fn new(rows: usize) -> Self {
        let Data::F32(floats) = float_rows(rows) else {
            unreachable!("the float frame is f32")
        };
        let Spacing::Quantized(levels) = Quantization::default().spacing() else {
            unreachable!("the default quantization quantizes")
        };
        let quantizer = Quantizer {
            precision: Precision::Single,
            step: levels.step,
            method: levels.dither.method(),
            zdither0: quantize::sample_seed(FloatSamples::F32(&floats)),
        };
        Self {
            values: floats.iter().map(|&v| f64::from(v)).collect(),
            floats,
            quantizer,
            scratch: QuantizeScratch::default(),
        }
    }

    /// Quantizes the tile into the scratch's integers, as the encoder does; its `ZSCALE` and
    /// `ZZERO`.
    fn quantize(&mut self) -> Option<quantize::Quantized> {
        let tile = TileValues {
            values: &self.values,
            width: WIDTH,
            index: 0,
        };
        self.quantizer.quantize(tile, &mut self.scratch)
    }
}

/// The first row of the float frame as the first tile, its integers at the default
/// quantization, and the dequantization a read takes them back with.
#[derive(Debug)]
struct QuantizedRow {
    tile: QuantizedTile,
    ints: Vec<i64>,
    dequant: Dequant,
}

impl QuantizedRow {
    fn new() -> Self {
        let mut tile = QuantizedTile::new(1);
        let quantized = tile.quantize().expect("the float frame quantizes");
        let ints = tile.scratch.ints.iter().map(|&v| i64::from(v)).collect();
        let dequant = Dequant {
            scale: quantized.bscale,
            zero: quantized.bzero,
            method: tile.quantizer.method,
            irow: tile.quantizer.zdither0,
            zblank: None,
        };
        Self {
            tile,
            ints,
            dequant,
        }
    }

    fn quantize(&mut self) -> Option<quantize::Quantized> {
        self.tile.quantize()
    }

    fn dequantize(&self) -> Vec<f32> {
        let mut out = Vec::new();
        self.dequant.apply(&self.ints, &mut out);
        out
    }

    /// Asserts that each of `dequantized` is within half a step of the row's float.
    fn assert_within_half_step(&self, dequantized: &[f32]) {
        assert_eq!(dequantized.len(), self.tile.floats.len());
        for (&back, &original) in dequantized.iter().zip(&self.tile.floats) {
            assert_within_half_step(
                f64::from(original),
                f64::from(back),
                self.dequant.scale,
                Bitpix::F32,
                "the default quantization",
            );
        }
    }
}
