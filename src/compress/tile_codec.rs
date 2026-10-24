//! [`TileCodec`]: the one dispatch between a tile's samples and its compressed stream.

use crate::compress::HcompressScale;
use crate::compress::convert;
use crate::compress::hcompress::Smoothing;
use crate::compress::plane::IntBitpix;
use crate::compress::{gzip, hcompress, nocompress, plio, rice};
use crate::error::{CompressionFeature, FitsError, Result};

/// A tile codec with the parameters its streams take (§10.4). Every image, float,
/// null-mask and table tile encodes and decodes through it, and the bench too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum TileCodec {
    /// `GZIP_1` at a deflate level.
    Gzip1 { level: u32 },
    /// `GZIP_2`: the bytes shuffled into significance planes a sample wide, then gzip'd.
    Gzip2 { level: u32 },
    /// `RICE_1` of `blocksize` samples a block, each `bytepix` wide.
    Rice(rice::Codec),
    /// `PLIO_1`, whose stream holds each sample plus `offset`: 32768 for a u16 image,
    /// whose stream holds the physical values, as PLIO takes no negative one.
    Plio(plio::Codec),
    /// `HCOMPRESS_1`, written at `scale` and read back with `smoothing`.
    Hcompress(hcompress::Codec),
    /// `NOCOMPRESS`: the samples' big-endian bytes.
    NoCompress,
}

/// One tile's samples, in the form the caller holds them.
#[derive(Debug, Clone, Copy)]
pub(super) enum TileSamples<'a> {
    Ints(&'a [i64]),
    /// The samples' big-endian bytes.
    Bytes(&'a [u8]),
}

/// What a codec needs of a tile besides its samples: its shape, fastest axis first;
/// the integer type of its samples' width, which lays their bytes out, shuffles them
/// for `GZIP_2` and types them for `RICE_1`; and the value a null sample takes, if one
/// does.
#[derive(Debug, Clone, Copy)]
pub(super) struct Tile<'a> {
    pub(super) shape: &'a [usize],
    pub(super) width: IntBitpix,
    pub(super) blank: Option<i64>,
}

/// The workspaces a codec reuses across tiles, each with the output it lends: the
/// integers a decode gives, the stream PLIO and NOCOMPRESS encode into, and the
/// streams and planes of the codecs' own workspaces.
#[derive(Debug, Default)]
pub(super) struct CodecScratch {
    ints: Vec<i64>,
    bytes: Vec<u8>,
    decoded: Vec<i64>,
    stream: Vec<u8>,
    gzip: gzip::GzipScratch,
    rice: rice::RiceScratch,
    hcompress: hcompress::HcompressScratch,
}

impl Tile<'_> {
    fn count(&self) -> usize {
        self.shape.iter().product()
    }

    fn byte_len(&self) -> Result<usize> {
        self.count()
            .checked_mul(self.width.elem_size())
            .ok_or(FitsError::DataUnitOverflow)
    }
}

impl<'a> TileSamples<'a> {
    /// The samples as integers of `width`, converted into `buffer` when they are bytes.
    fn ints<'s>(self, width: IntBitpix, buffer: &'s mut Vec<i64>) -> &'s [i64]
    where
        'a: 's,
    {
        match self {
            Self::Ints(values) => values,
            Self::Bytes(bytes) => {
                convert::be_to_i64_into(bytes, width, buffer);
                buffer
            }
        }
    }

    /// The samples as big-endian bytes of `width`, converted into `buffer` when they
    /// are integers.
    fn bytes<'s>(self, width: IntBitpix, buffer: &'s mut Vec<u8>) -> &'s [u8]
    where
        'a: 's,
    {
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Ints(values) => {
                convert::i64_to_be_into(values, width, buffer);
                buffer
            }
        }
    }
}

impl TileCodec {
    /// The codec `name` spells in `ZCMPTYPE`, `ZMASKCMP` or `ZCTYPn`, with the defaults
    /// of Table 37 until its parameters are read: 32-sample blocks of 4-byte RICE
    /// samples, no PLIO offset, and HCOMPRESS with no smoothing.
    pub(super) fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "GZIP_1" => Self::Gzip1 {
                level: gzip::DEFAULT_GZIP_LEVEL,
            },
            "GZIP_2" => Self::Gzip2 {
                level: gzip::DEFAULT_GZIP_LEVEL,
            },
            "RICE_1" => Self::Rice(rice::Codec {
                blocksize: rice::BLOCKSIZE,
                bytepix: IntBitpix::I32,
            }),
            "PLIO_1" => Self::Plio(plio::Codec { offset: 0 }),
            "HCOMPRESS_1" => Self::Hcompress(hcompress::Codec {
                scale: HcompressScale::Lossless,
                smoothing: Smoothing::Off,
            }),
            "NOCOMPRESS" => Self::NoCompress,
            other => {
                return Err(FitsError::UnsupportedCompression {
                    feature: CompressionFeature::Codec {
                        name: other.to_string(),
                    },
                });
            }
        })
    }

    /// The `ZCMPTYPE`, `ZMASKCMP` or `ZCTYPn` spelling, the inverse of [`Self::parse`].
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Gzip1 { .. } => "GZIP_1",
            Self::Gzip2 { .. } => "GZIP_2",
            Self::Rice(_) => "RICE_1",
            Self::Plio(_) => "PLIO_1",
            Self::Hcompress(_) => "HCOMPRESS_1",
            Self::NoCompress => "NOCOMPRESS",
        }
    }

    /// Whether a float image's tiles may be stored losslessly with this codec: the
    /// gzip codecs alone, as cfitsio takes it (`imcomp_init_table`).
    pub(super) const fn stores_lossless_floats(self) -> bool {
        matches!(self, Self::Gzip1 { .. } | Self::Gzip2 { .. })
    }

    /// The bytes of one element of the stream's array: PLIO's are i16 words.
    pub(super) const fn stream_element(self) -> usize {
        match self {
            Self::Plio(_) => 2,
            _ => 1,
        }
    }

    /// Encode `samples`, one tile `tile` describes, and lend its stream from `scratch`.
    pub(super) fn encode<'s>(
        self,
        tile: Tile<'_>,
        samples: TileSamples<'_>,
        scratch: &'s mut CodecScratch,
    ) -> Result<&'s [u8]> {
        match self {
            Self::Gzip1 { level } | Self::Gzip2 { level } => {
                let raw = samples.bytes(tile.width, &mut scratch.bytes);
                let codec = gzip::Codec {
                    level,
                    shuffle: self.shuffle(tile),
                };
                Ok(codec.encode(raw, &mut scratch.gzip))
            }
            Self::Rice(codec) => {
                let values = samples.ints(tile.width, &mut scratch.ints);
                Ok(codec.encode(values, &mut scratch.rice))
            }
            Self::Plio(codec) => {
                scratch.stream.clear();
                codec.encode(
                    samples.ints(tile.width, &mut scratch.ints),
                    &mut scratch.stream,
                )?;
                Ok(&scratch.stream)
            }
            Self::Hcompress(codec) => {
                let values = samples.ints(tile.width, &mut scratch.ints);
                codec.encode(values, tile, &mut scratch.hcompress)
            }
            Self::NoCompress => {
                scratch.stream.clear();
                nocompress::encode(
                    samples.bytes(tile.width, &mut scratch.bytes),
                    &mut scratch.stream,
                );
                Ok(&scratch.stream)
            }
        }
    }

    /// Decode `stream`, one tile `tile` describes, and lend its integer samples from
    /// `scratch`, which a caller may swap out for a buffer of its own.
    pub(super) fn decode_ints<'s>(
        self,
        tile: Tile<'_>,
        stream: &[u8],
        scratch: &'s mut CodecScratch,
    ) -> Result<&'s mut Vec<i64>> {
        let count = tile.count();
        match self {
            Self::Rice(codec) => codec.decode(stream, count, &mut scratch.decoded)?,
            Self::Plio(codec) => codec.decode(stream, count, &mut scratch.decoded)?,
            Self::Hcompress(codec) => {
                return codec.decode(stream, tile, &mut scratch.hcompress);
            }
            Self::Gzip1 { level } | Self::Gzip2 { level } => {
                let codec = gzip::Codec {
                    level,
                    shuffle: self.shuffle(tile),
                };
                let bytes = codec.decode(stream, tile.byte_len()?, &mut scratch.gzip)?;
                convert::be_to_i64_into(bytes, tile.width, &mut scratch.decoded);
            }
            Self::NoCompress => {
                let bytes = nocompress::decode(stream, tile.byte_len()?)?;
                convert::be_to_i64_into(bytes, tile.width, &mut scratch.decoded);
            }
        }
        Ok(&mut scratch.decoded)
    }

    /// Decode `stream`, one tile `tile` describes, and lend its samples' big-endian
    /// bytes.
    pub(super) fn decode_bytes<'s>(
        self,
        tile: Tile<'_>,
        stream: &'s [u8],
        scratch: &'s mut CodecScratch,
    ) -> Result<&'s [u8]> {
        let expected = tile.byte_len()?;
        match self {
            Self::Gzip1 { level } | Self::Gzip2 { level } => {
                let codec = gzip::Codec {
                    level,
                    shuffle: self.shuffle(tile),
                };
                codec.decode(stream, expected, &mut scratch.gzip)
            }
            Self::NoCompress => nocompress::decode(stream, expected),
            Self::Rice(_) | Self::Plio(_) | Self::Hcompress(_) => {
                let mut bytes = std::mem::take(&mut scratch.bytes);
                let decoded = self
                    .decode_ints(tile, stream, scratch)
                    .map(|ints| convert::i64_to_be_into(ints, tile.width, &mut bytes));
                scratch.bytes = bytes;
                decoded?;
                Ok(&scratch.bytes)
            }
        }
    }

    /// The `GZIP_2` shuffle width of `tile`'s samples, 1 for `GZIP_1`.
    const fn shuffle(self, tile: Tile<'_>) -> usize {
        match self {
            Self::Gzip2 { .. } => tile.width.elem_size(),
            _ => 1,
        }
    }
}

/// Encoded tiles for the decode tests.
#[cfg(test)]
pub(crate) mod internals {
    use crate::compress::plane::IntBitpix;
    use crate::compress::rice;
    use crate::compress::tile_codec::{CodecScratch, Tile, TileCodec, TileSamples};

    impl TileCodec {
        /// The codec `name` spells, its `RICE_1` samples `width` wide.
        pub(crate) fn named(name: &str, width: IntBitpix) -> Self {
            match Self::parse(name).unwrap() {
                Self::Rice(rice::Codec { blocksize, .. }) => Self::Rice(rice::Codec {
                    blocksize,
                    bytepix: width,
                }),
                codec => codec,
            }
        }

        /// `values`, one tile of `shape` whose samples are `width` wide, encoded.
        pub(crate) fn encoded(self, shape: &[usize], width: IntBitpix, values: &[i64]) -> Vec<u8> {
            let tile = Tile {
                shape,
                width,
                blank: None,
            };
            self.encode(
                tile,
                TileSamples::Ints(values),
                &mut CodecScratch::default(),
            )
            .unwrap()
            .to_vec()
        }

        /// The bytes `raw`, one tile of single-byte samples, encoded.
        pub(crate) fn encoded_bytes(self, raw: &[u8]) -> Vec<u8> {
            let tile = Tile {
                shape: &[raw.len()],
                width: IntBitpix::U8,
                blank: None,
            };
            self.encode(tile, TileSamples::Bytes(raw), &mut CodecScratch::default())
                .unwrap()
                .to_vec()
        }

        /// The `len` single-byte samples of `stream`, decoded.
        pub(crate) fn decoded_bytes(self, stream: &[u8], len: usize) -> Vec<u8> {
            let tile = Tile {
                shape: &[len],
                width: IntBitpix::U8,
                blank: None,
            };
            self.decode_bytes(tile, stream, &mut CodecScratch::default())
                .unwrap()
                .to_vec()
        }
    }
}
