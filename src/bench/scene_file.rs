//! The files the integration cases read: each scene input in each storage it takes, written
//! once and checked once against the scene.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

#[cfg(feature = "compression")]
use crate::compression::half_step::assert_within_half_step;
#[cfg(feature = "compression")]
use crate::compression::quantization::Quantization;
#[cfg(feature = "compression")]
use crate::compression::{Compression, CompressionConfig, Hcompress};
use crate::fits_writer::FitsWriter;
use crate::fits_writer::ascii_builder::AsciiBuilder;
use crate::fits_writer::builder::Builder;
use crate::header_model::{Header, keywords};
use crate::image_model::Image;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::internals::samples;
use crate::internals::scene::{self, HEIGHT, WIDTH};
use crate::reader::FitsReader;
use crate::reader::source::Source;

use crate::bench::policy::{self, ScratchDir};

/// How an input is stored in its file: plain, or tiled-compressed by a codec. The codec
/// rows are the ones the readers meet: fpack's defaults and the lossless float form lumos
/// writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Storage {
    Plain,
    #[cfg(feature = "compression")]
    Gzip1,
    #[cfg(feature = "compression")]
    Gzip2,
    #[cfg(feature = "compression")]
    Rice1,
    #[cfg(feature = "compression")]
    Hcompress1,
    #[cfg(feature = "compression")]
    Plio1,
    /// `GZIP_2` of a float image with no quantization.
    #[cfg(feature = "compression")]
    Gzip2Lossless,
}

impl Storage {
    /// The storage part of a case's id.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            #[cfg(feature = "compression")]
            Self::Gzip1 => "gzip_1",
            #[cfg(feature = "compression")]
            Self::Gzip2 => "gzip_2",
            #[cfg(feature = "compression")]
            Self::Rice1 => "rice_1",
            #[cfg(feature = "compression")]
            Self::Hcompress1 => "hcompress_1",
            #[cfg(feature = "compression")]
            Self::Plio1 => "plio_1",
            #[cfg(feature = "compression")]
            Self::Gzip2Lossless => "gzip_2_lossless",
        }
    }

    /// The tiled compression of this storage, in each codec's default tiles; `None` for
    /// plain storage. A float image's quantization is the default, q = 4 with
    /// `SUBTRACTIVE_DITHER_1`, as fpack writes it.
    #[cfg(feature = "compression")]
    pub(crate) fn config(self) -> Option<CompressionConfig> {
        let compression = match self {
            Self::Plain => return None,
            Self::Gzip1 => Compression::GZIP,
            Self::Gzip2 | Self::Gzip2Lossless => Compression::GZIP_SHUFFLED,
            Self::Rice1 => Compression::Rice,
            Self::Hcompress1 => Compression::Hcompress(Hcompress::default()),
            Self::Plio1 => Compression::Plio,
        };
        let config = CompressionConfig::new(compression);
        Some(if self == Self::Gzip2Lossless {
            config.with_quantization(Quantization::NONE)
        } else {
            config
        })
    }

    /// Whether a read gives back the stored samples bit for bit: all but a quantized float.
    #[cfg_attr(
        not(feature = "compression"),
        expect(
            unused_variables,
            reason = "plain storage is lossless whatever the input"
        )
    )]
    const fn is_lossless(self, input: SceneInput) -> bool {
        match self {
            #[cfg(feature = "compression")]
            Self::Rice1 => !matches!(input, SceneInput::Frame(Bitpix::F32 | Bitpix::F64)),
            _ => true,
        }
    }
}

/// What a scene file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneInput {
    /// The standard frame of a `BITPIX`, under the light header.
    Frame(Bitpix),
    /// The frame's `u8` defect mask, under the light header.
    #[cfg(feature = "compression")]
    Mask,
    /// The source catalog, a binary table after an empty primary.
    Catalog,
    /// The source catalog as an ASCII table after an empty primary.
    AsciiCatalog,
    /// The spectra, a binary table with a variable-length column after an empty primary.
    Spectra,
    /// The mosaic: an empty primary under the light header, then each CCD an image
    /// extension named by [`scene::ccd_name`].
    Mosaic,
}

impl SceneInput {
    /// The input part of a case's id.
    pub(crate) fn id(self) -> String {
        match self {
            Self::Frame(bitpix) => policy::bitpix_id(bitpix).to_string(),
            #[cfg(feature = "compression")]
            Self::Mask => "mask".to_string(),
            Self::Catalog | Self::AsciiCatalog => "catalog".to_string(),
            Self::Spectra => "spectra".to_string(),
            Self::Mosaic => "mosaic".to_string(),
        }
    }

    /// The `BITPIX` of the image this input holds; `None` for a table or the mosaic.
    pub(crate) const fn bitpix(self) -> Option<Bitpix> {
        match self {
            Self::Frame(bitpix) => Some(bitpix),
            #[cfg(feature = "compression")]
            Self::Mask => Some(Bitpix::U8),
            Self::Catalog | Self::AsciiCatalog | Self::Spectra | Self::Mosaic => None,
        }
    }

    /// The image this input holds; `None` for a table or the mosaic.
    pub(crate) fn image(self) -> Option<Image> {
        match self {
            Self::Frame(bitpix) => Some(scene::sky(bitpix, WIDTH, HEIGHT, 1)),
            #[cfg(feature = "compression")]
            Self::Mask => Some(scene::mask(WIDTH, HEIGHT, 1)),
            Self::Catalog | Self::AsciiCatalog | Self::Spectra | Self::Mosaic => None,
        }
    }

    /// What a file of this input holds, built from the scene.
    pub(crate) fn data(self) -> SceneData {
        match self {
            Self::Catalog => SceneData::Table(scene::catalog()),
            Self::Spectra => SceneData::Table(scene::spectra()),
            Self::AsciiCatalog => SceneData::AsciiTable(scene::ascii_catalog()),
            Self::Mosaic => SceneData::Mosaic(scene::mosaic()),
            Self::Frame(_) => SceneData::Image(self.image().unwrap()),
            #[cfg(feature = "compression")]
            Self::Mask => SceneData::Image(self.image().unwrap()),
        }
    }

    /// The rows of a compressed table's tile: about 220 kB of the table's bytes in each.
    #[cfg(feature = "compression")]
    fn tile_rows(self) -> usize {
        match self {
            Self::Catalog => 4096,
            Self::Spectra => 22,
            _ => unreachable!("a binary table input"),
        }
    }
}

/// What a scene file holds, built once and written as often as a case writes it.
#[derive(Debug)]
pub(crate) enum SceneData {
    Image(Image),
    Table(Builder),
    AsciiTable(AsciiBuilder),
    Mosaic(Vec<Image>),
}

/// One input in one storage, the file an integration case reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SceneFile {
    pub(crate) input: SceneInput,
    pub(crate) storage: Storage,
}

impl SceneFile {
    /// `input` in plain storage.
    pub(crate) const fn plain(input: SceneInput) -> Self {
        Self {
            input,
            storage: Storage::Plain,
        }
    }

    /// Each frame in plain storage.
    pub(crate) fn plain_frames() -> impl Iterator<Item = Self> {
        samples::BITPIXES
            .into_iter()
            .map(|bitpix| Self::plain(SceneInput::Frame(bitpix)))
    }

    /// Every image file a read case takes: each frame plain; the light in each integer
    /// codec; the mask in `PLIO_1`; the float frame quantized by `RICE_1` and lossless by
    /// `GZIP_2`.
    pub(crate) fn images() -> Vec<Self> {
        #[cfg_attr(
            not(feature = "compression"),
            expect(unused_mut, reason = "with no codec the plain frames are all")
        )]
        let mut files: Vec<Self> = Self::plain_frames().collect();
        #[cfg(feature = "compression")]
        {
            let light = SceneInput::Frame(Bitpix::I16);
            let float = SceneInput::Frame(Bitpix::F32);
            for (input, storage) in [
                (light, Storage::Gzip1),
                (light, Storage::Gzip2),
                (light, Storage::Rice1),
                (light, Storage::Hcompress1),
                (SceneInput::Mask, Storage::Plio1),
                (float, Storage::Rice1),
                (float, Storage::Gzip2Lossless),
            ] {
                files.push(Self { input, storage });
            }
        }
        files
    }

    /// Every file of the binary table `input` a read case takes: plain and in each table
    /// codec.
    pub(crate) fn tables(input: SceneInput) -> Vec<Self> {
        let storages = [
            Storage::Plain,
            #[cfg(feature = "compression")]
            Storage::Gzip1,
            #[cfg(feature = "compression")]
            Storage::Gzip2,
            #[cfg(feature = "compression")]
            Storage::Rice1,
        ];
        storages
            .into_iter()
            .map(|storage| Self { input, storage })
            .collect()
    }

    /// The storage and input parts of a case's id: `rice_1/i16`.
    pub(crate) fn id(self) -> String {
        format!("{}/{}", self.storage.id(), self.input.id())
    }

    /// The HDU that holds the data: the primary of a plain image, the last CCD of the
    /// mosaic, else the first extension.
    pub(crate) const fn hdu(self) -> usize {
        match (self.input, self.storage) {
            (SceneInput::Mosaic, _) => scene::MOSAIC_CCDS,
            (SceneInput::Catalog | SceneInput::AsciiCatalog | SceneInput::Spectra, _) => 1,
            (_, Storage::Plain) => 0,
            #[cfg(feature = "compression")]
            _ => 1,
        }
    }

    /// Writes `data`, this file's input, with `writer`, which sets the form.
    pub(crate) fn write<W: Write>(self, writer: &mut FitsWriter<W>, data: &SceneData) {
        match (data, self.storage) {
            (SceneData::Table(table), Storage::Plain) => writer.write_table(table, None).unwrap(),
            #[cfg(feature = "compression")]
            (SceneData::Table(table), storage) => writer
                .write_compressed_table(
                    table,
                    &storage
                        .config()
                        .unwrap()
                        .with_tiles([self.input.tile_rows()]),
                    None,
                )
                .unwrap(),
            (SceneData::AsciiTable(table), Storage::Plain) => {
                writer.write_ascii_table(table, None).unwrap();
            }
            (SceneData::Image(image), Storage::Plain) => writer
                .write_image(image.view(), Some(&scene::light_header()))
                .unwrap(),
            #[cfg(feature = "compression")]
            (SceneData::Image(image), storage) => writer
                .write_compressed_image(
                    image.view(),
                    &storage.config().unwrap(),
                    Some(&scene::light_header()),
                )
                .unwrap(),
            (SceneData::Mosaic(ccds), Storage::Plain) => {
                writer
                    .write_empty_primary(Some(&scene::light_header()))
                    .unwrap();
                for (index, ccd) in ccds.iter().enumerate() {
                    let mut header = Header::new();
                    header.set_internal(keywords::EXTNAME, scene::ccd_name(index + 1).as_str());
                    writer.write_image(ccd.view(), Some(&header)).unwrap();
                }
            }
            #[cfg(feature = "compression")]
            (SceneData::AsciiTable(_) | SceneData::Mosaic(_), _) => {
                unreachable!("an ASCII table and the mosaic are plain")
            }
        }
    }

    /// The bytes of this file, with its checksums.
    fn bytes(self) -> Vec<u8> {
        let mut writer = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
        self.write(&mut writer, &self.input.data());
        writer.into_inner().into_inner()
    }

    /// Asserts that `read`, the samples a case read from this file, are the scene's: bit
    /// for bit for lossless storage, within half a quantization step for a quantized float.
    pub(crate) fn assert_read<S: Source>(self, reader: &mut FitsReader<S>, read: &Data) {
        let expected = self.input.image().expect("an image file").samples;
        if self.storage.is_lossless(self.input) {
            samples::assert_same_bits(read, &expected, &self.id());
        } else {
            self.assert_quantized(reader, read, &expected);
        }
    }

    /// Asserts that each sample of `read` is within half its tile's quantization step of
    /// `expected`.
    #[cfg(feature = "compression")]
    fn assert_quantized<S: Source>(self, reader: &mut FitsReader<S>, read: &Data, expected: &Data) {
        let container = reader.read_container_table(self.hdu()).unwrap();
        let mut zscale: Vec<f64> = Vec::new();
        container
            .column("ZSCALE")
            .unwrap()
            .physical_into(&mut zscale)
            .unwrap();
        let (Data::F32(read), Data::F32(expected)) = (read, expected) else {
            panic!("{}: a quantized float frame", self.id());
        };
        for (index, (&got, &want)) in read.iter().zip(expected).enumerate() {
            assert_within_half_step(
                f64::from(want),
                f64::from(got),
                zscale[index / WIDTH],
                Bitpix::F32,
                &self.id(),
            );
        }
    }

    #[cfg(not(feature = "compression"))]
    fn assert_quantized<S: Source>(
        self,
        _reader: &mut FitsReader<S>,
        _read: &Data,
        _expected: &Data,
    ) {
        unreachable!("with no codec no storage quantizes");
    }
}

/// The scene files of a scratch directory, each written the first time a case needs it.
#[derive(Debug)]
pub(crate) struct SceneFiles {
    dir: ScratchDir,
    written: Vec<(SceneFile, PathBuf)>,
}

impl SceneFiles {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            dir: ScratchDir::new(name),
            written: Vec::new(),
        }
    }

    /// The path of `file`, written now if no case wrote it before.
    pub(crate) fn path(&mut self, file: SceneFile) -> &Path {
        if !self.written.iter().any(|(written, _)| *written == file) {
            let name = format!("{}.fits", self.written.len());
            let path = self.dir.write(&name, &file.bytes());
            self.written.push((file, path));
        }
        let (_, path) = self
            .written
            .iter()
            .find(|(written, _)| *written == file)
            .expect("written above");
        path
    }

    /// The path of the file `name` in the directory, for a case to write.
    pub(crate) fn scratch(&self, name: &str) -> PathBuf {
        self.dir.file(name)
    }
}
