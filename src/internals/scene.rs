//! The standard scene every case measures on: sky, as a camera writes it, drawn from fixed seeds
//! so two runs time the same work.

use crate::ascii::ascii_tform::AsciiFloatFormat;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor::DescriptorWidth;
use crate::fits_writer::ascii_builder::{AsciiBuilder, AsciiWriteColumn};
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::{Header, keywords};
use crate::image_model::Image;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use crate::internals::rng::SplitMix;
use crate::ragged::Ragged;
use crate::world_coordinates::Wcs;
use std::f64::consts::TAU;

/// The standard frame's width: 6000 × 4000, an APS-C camera's 24 MP, as the lumos benches use.
pub(crate) const WIDTH: usize = 6000;
/// The standard frame's height.
pub(crate) const HEIGHT: usize = 4000;

/// The rows of [`catalog`]: the stars a deep frame's source extraction finds.
pub(crate) const CATALOG_ROWS: usize = 100_000;
/// The bytes of one row of [`catalog`]: an `i32`, four `f64`, four `f32` and an `i16`.
pub(crate) const CATALOG_ROW_BYTES: usize = 4 + 4 * 8 + 4 * 4 + 2;

/// The sky level, a fraction of full scale.
const SKY: f64 = 0.1;
/// The full scale of the light in ADU: a 16-bit camera.
const FULL_SCALE_ADU: f64 = 65535.0;
/// The camera's electrons per ADU, the `EGAIN` of [`light_header`].
const EGAIN: f64 = 0.25;
/// The read noise in electrons, an ASI2600's at gain 100.
const READ_NOISE_E: f64 = 1.5;
/// The stars' full width at half maximum in pixels, the catalog's typical seeing.
const FWHM: f64 = 2.5;
/// The pixels a star takes on average: the deep frame's star density.
const PIXELS_PER_STAR: usize = WIDTH * HEIGHT / CATALOG_ROWS;
/// The index of the stars' flux law, `N(> F) ∝ F^−1.5`, the Euclidean counts.
const FLUX_INDEX: f64 = 1.5;
/// The faintest star's peak, a fraction of full scale: a few times the sky noise.
const FAINTEST_PEAK: f64 = 0.01;

/// One star of the scene: its position in pixels, 0-based, and its flux, a fraction of full
/// scale summed over its pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Star {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) flux: f64,
}

/// The stars of a `width` × `height` field at `seed`: one for each [`PIXELS_PER_STAR`]
/// pixels, uniform in position, their fluxes a power law above the faintest.
pub(crate) fn stars(width: usize, height: usize, seed: u64) -> Vec<Star> {
    let mut draws = SplitMix::new(seed ^ 0x5741_5253);
    let sigma = FWHM / (8.0 * 2f64.ln()).sqrt();
    let faintest = FAINTEST_PEAK * TAU * sigma * sigma;
    (0..(width * height).div_ceil(PIXELS_PER_STAR))
        .map(|_| Star {
            x: draws.unit() * width as f64,
            y: draws.unit() * height as f64,
            flux: faintest * draws.power_law(FLUX_INDEX),
        })
        .collect()
}

/// The noise-free light of a `width` × `height` field at `seed`, a fraction of full scale a
/// pixel: the sky and each star as a Gaussian of [`FWHM`], cut at 4σ.
fn signal(width: usize, height: usize, seed: u64) -> Vec<f64> {
    let mut plane = vec![SKY; width * height];
    let sigma = FWHM / (8.0 * 2f64.ln()).sqrt();
    let reach = (4.0 * sigma).ceil() as isize;
    for star in stars(width, height, seed) {
        let peak = star.flux / (TAU * sigma * sigma);
        let (cx, cy) = (star.x.floor() as isize, star.y.floor() as isize);
        for y in (cy - reach).max(0)..(cy + reach + 1).min(height as isize) {
            for x in (cx - reach).max(0)..(cx + reach + 1).min(width as isize) {
                let (dx, dy) = (x as f64 + 0.5 - star.x, y as f64 + 0.5 - star.y);
                let r2 = (dx * dx + dy * dy) / (sigma * sigma);
                plane[y.cast_unsigned() * width + x.cast_unsigned()] += peak * (-0.5 * r2).exp();
            }
        }
    }
    plane
}

/// `width` × `height` samples of sky stored as `bitpix` from `seed`, as a camera writes a
/// light: [`signal`] with the photon noise its own electrons give at [`EGAIN`] and the read
/// noise, clamped to `[0, 1]` of full scale. An `i16` frame is the camera's: `u16` counts stored
/// with `BZERO = 32768`; the other integers span their positive range, the floats `[0, 1]`.
#[expect(clippy::cast_sign_loss, reason = "every value is clamped to [0, 1]")]
pub(crate) fn sky(bitpix: Bitpix, width: usize, height: usize, seed: u64) -> Image {
    let mut draws = SplitMix::new(seed);
    let units = signal(width, height, seed).into_iter().map(|value| {
        let electrons = value * FULL_SCALE_ADU * EGAIN;
        let sigma = (electrons + READ_NOISE_E * READ_NOISE_E).sqrt() / EGAIN / FULL_SCALE_ADU;
        (value + sigma * draws.gaussian()).clamp(0.0, 1.0)
    });
    let shape = vec![width, height];
    let data = match bitpix {
        Bitpix::U8 => Data::U8(units.map(|v| (v * 255.0).round() as u8).collect()),
        Bitpix::I16 => {
            let light = units.map(|v| ((v * 65535.0).round() as u16 ^ 0x8000).cast_signed());
            let scaling = Scaling {
                bscale: 1.0,
                bzero: 32768.0,
                blank: None,
            };
            return Image::new(shape, Data::I16(light.collect()), scaling).unwrap();
        }
        Bitpix::I32 => Data::I32(
            units
                .map(|v| (v * f64::from(i32::MAX)).round() as i32)
                .collect(),
        ),
        Bitpix::I64 => Data::I64(units.map(|v| (v * 2f64.powi(53)).round() as i64).collect()),
        Bitpix::F32 => Data::F32(units.map(|v| v as f32).collect()),
        Bitpix::F64 => Data::F64(units.collect()),
    };
    Image::new(shape, data, Scaling::IDENTITY).unwrap()
}

/// The defect mask of [`sky`] at `seed`, `u8` flags of a defect map: one pixel in 2000 hot,
/// three bad columns, and each star core the signal saturates.
#[cfg(feature = "compression")]
pub(crate) fn mask(width: usize, height: usize, seed: u64) -> Image {
    let mut draws = SplitMix::new(seed ^ 0x4D41_534B);
    let bad_columns = [width / 7, width / 3, 4 * width / 5];
    let flags = signal(width, height, seed)
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let hot = draws.next().is_multiple_of(2000);
            u8::from(hot || value >= 1.0 || bad_columns.contains(&(index % width)))
        });
    Image::new(
        vec![width, height],
        Data::U8(flags.collect()),
        Scaling::IDENTITY,
    )
    .unwrap()
}

/// The header a capture program writes on a light, past the structural keywords the image
/// sets: the exposure, the camera, the mount, the site, the target and the software. A plate
/// solver adds [`light_header`]'s solution to it later.
pub(crate) fn capture_header() -> Header {
    let mut header = Header::new();
    header
        .set_internal("IMAGETYP", "LIGHT")
        .set_internal("EXPOSURE", 120.0)
        .set_internal("EXPTIME", 120.0)
        .set_internal("DATE-LOC", "2026-03-14T22:41:07.123")
        .set_internal(keywords::DATE_OBS, "2026-03-14T21:41:07.123")
        .set_internal("XBINNING", 1)
        .set_internal("YBINNING", 1)
        .set_internal("GAIN", 100)
        .set_internal("OFFSET", 50)
        .set_internal("EGAIN", EGAIN)
        .set_internal("XPIXSZ", 3.76)
        .set_internal("YPIXSZ", 3.76)
        .set_internal(keywords::INSTRUME, "ZWO ASI2600MC Pro")
        .set_internal("SET-TEMP", -10.0)
        .set_internal("CCD-TEMP", -10.1)
        .set_internal(keywords::TELESCOP, "RedCat 51")
        .set_internal("FOCALLEN", 250.0)
        .set_internal("FOCRATIO", 4.9)
        .set_internal("RA", 83.822)
        .set_internal("DEC", -5.391)
        .set_internal("CENTALT", 41.7)
        .set_internal("CENTAZ", 168.2)
        .set_internal("AIRMASS", 1.497)
        .set_internal("PIERSIDE", "WEST")
        .set_internal("SITEELEV", 312.0)
        .set_internal("SITELAT", 47.37)
        .set_internal("SITELONG", 8.54)
        .set_internal("FWHEEL", "ZWO EFW")
        .set_internal("FILTER", "L-eXtreme")
        .set_internal(keywords::OBJECT, "M 42")
        .set_internal("OBJCTRA", "05 35 17")
        .set_internal("OBJCTDEC", "-05 23 28")
        .set_internal("OBJCTROT", 92.4)
        .set_internal("ROWORDER", "TOP-DOWN")
        .set_internal(keywords::EQUINOX, 2000.0)
        .set_internal("BAYERPAT", "RGGB")
        .set_internal("XBAYROFF", 0)
        .set_internal("YBAYROFF", 0)
        .set_internal("FOCNAME", "ZWO EAF")
        .set_internal("FOCPOS", 4312)
        .set_internal("FOCTEMP", 6.4)
        .set_internal("SWCREATE", "N.I.N.A. 3.1.2.9001");
    header
        .push_commentary(
            CommentaryName::COMMENT,
            "NINA - Nighttime Imaging 'N' Astronomy",
        )
        .unwrap()
        .push_commentary(CommentaryName::HISTORY, "Calibrated: none")
        .unwrap();
    header
}

/// [`capture_header`] with the plate solution of [`solve`].
pub(crate) fn light_header() -> Header {
    let mut header = capture_header();
    solve(&mut header);
    header
}

/// Adds to `header` the plate solution a solver writes for the standard frame: a gnomonic
/// projection at its centre, 3.1″ a pixel rotated by 92.4°.
pub(crate) fn solve(header: &mut Header) {
    header
        .set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN")
        .set_internal("CUNIT1", "deg")
        .set_internal("CUNIT2", "deg")
        .set_internal("CRPIX1", 3000.5)
        .set_internal("CRPIX2", 2000.5)
        .set_internal("CRVAL1", 83.822)
        .set_internal("CRVAL2", -5.391)
        .set_internal("CD1_1", -3.584e-5)
        .set_internal("CD1_2", 8.6047e-4)
        .set_internal("CD2_1", -8.6047e-4)
        .set_internal("CD2_2", -3.584e-5);
}

/// The columns of the source catalog, one value a star.
#[derive(Debug)]
struct CatalogColumns {
    number: Vec<i32>,
    x: Vec<f64>,
    y: Vec<f64>,
    ra: Vec<f64>,
    dec: Vec<f64>,
    flux: Vec<f32>,
    flux_err: Vec<f32>,
    fwhm: Vec<f32>,
    elong: Vec<f32>,
    flags: Vec<i16>,
}

impl CatalogColumns {
    /// The [`CATALOG_ROWS`] stars of [`stars`] at the frame's seed: a running number, the
    /// position in pixels (1-based) and on the sky through [`light_header`]'s plate solution,
    /// the flux and its error, the width and the elongation, and the extraction flags.
    fn new() -> Self {
        let stars = stars(WIDTH, HEIGHT, 1);
        assert_eq!(stars.len(), CATALOG_ROWS, "a row for each star");
        let mut header = light_header();
        header.set_internal(keywords::NAXIS, 2);
        let wcs = Wcs::from_header(&header, None).unwrap();
        let mut world = [0.0; 2];
        let (mut ra, mut dec) = (
            Vec::with_capacity(CATALOG_ROWS),
            Vec::with_capacity(CATALOG_ROWS),
        );
        for star in &stars {
            wcs.pixel_to_world(&[star.x + 1.0, star.y + 1.0], &mut world)
                .unwrap();
            ra.push(world[0]);
            dec.push(world[1]);
        }
        let mut draws = SplitMix::new(7);
        Self {
            number: (1..=CATALOG_ROWS)
                .map(|n| i32::try_from(n).unwrap())
                .collect(),
            x: stars.iter().map(|star| star.x + 1.0).collect(),
            y: stars.iter().map(|star| star.y + 1.0).collect(),
            ra,
            dec,
            flux: stars.iter().map(|star| star.flux as f32).collect(),
            flux_err: column(&mut draws, |d| (20.0 + 5.0 * d.unit()) as f32),
            fwhm: column(&mut draws, |d| (FWHM + 0.3 * d.gaussian()) as f32),
            elong: column(&mut draws, |d| (1.0 + 0.2 * d.unit()) as f32),
            flags: column(&mut draws, |d| i16::from(d.next().is_multiple_of(20))),
        }
    }
}

/// The source catalog of the standard frame, the stars of [`stars`] at its seed, as a
/// binary table.
pub(crate) fn catalog() -> Builder {
    let c = CatalogColumns::new();
    let columns = vec![
        WriteColumn::scalar("NUMBER", ColumnData::I32(c.number)),
        WriteColumn::scalar("X_IMAGE", ColumnData::F64(c.x)),
        WriteColumn::scalar("Y_IMAGE", ColumnData::F64(c.y)),
        WriteColumn::scalar("RA", ColumnData::F64(c.ra)),
        WriteColumn::scalar("DEC", ColumnData::F64(c.dec)),
        WriteColumn::scalar("FLUX", ColumnData::F32(c.flux)),
        WriteColumn::scalar("FLUX_ERR", ColumnData::F32(c.flux_err)),
        WriteColumn::scalar("FWHM", ColumnData::F32(c.fwhm)),
        WriteColumn::scalar("ELONG", ColumnData::F32(c.elong)),
        WriteColumn::scalar("FLAGS", ColumnData::I16(c.flags)),
    ];
    Builder::with_columns(CATALOG_ROWS, columns).unwrap()
}

/// The bytes of one row of [`ascii_catalog`]: the widths of its fields.
pub(crate) const ASCII_CATALOG_ROW_BYTES: usize = 7 + 2 * 10 + 2 * 13 + 2 * 15 + 2 * 8 + 3;

/// The source catalog as an ASCII table, as older survey catalogs store it.
pub(crate) fn ascii_catalog() -> AsciiBuilder {
    let c = CatalogColumns::new();
    let real = |values: Vec<f64>| values.into_iter().map(Some).collect::<Vec<_>>();
    let narrow = |values: Vec<f32>| {
        values
            .into_iter()
            .map(|v| Some(f64::from(v)))
            .collect::<Vec<_>>()
    };
    let f = |width, decimals| AsciiFloatFormat::F { width, decimals };
    let e = |width, decimals| AsciiFloatFormat::E { width, decimals };
    let columns = vec![
        AsciiWriteColumn::integer(
            "NUMBER",
            c.number.into_iter().map(|v| Some(i64::from(v))),
            7,
        ),
        AsciiWriteColumn::float("X_IMAGE", real(c.x), f(10, 3)),
        AsciiWriteColumn::float("Y_IMAGE", real(c.y), f(10, 3)),
        AsciiWriteColumn::float("RA", real(c.ra), f(13, 7)),
        AsciiWriteColumn::float("DEC", real(c.dec), f(13, 7)),
        AsciiWriteColumn::float("FLUX", narrow(c.flux), e(15, 7)),
        AsciiWriteColumn::float("FLUX_ERR", narrow(c.flux_err), e(15, 7)),
        AsciiWriteColumn::float("FWHM", narrow(c.fwhm), f(8, 3)),
        AsciiWriteColumn::float("ELONG", narrow(c.elong), f(8, 4)),
        AsciiWriteColumn::integer("FLAGS", c.flags.into_iter().map(|v| Some(i64::from(v))), 3),
    ];
    AsciiBuilder::with_columns(CATALOG_ROWS, columns).unwrap()
}

/// The CCDs of [`mosaic`], as a wide-field camera lays them out (DECam has 62).
pub(crate) const MOSAIC_CCDS: usize = 60;

/// The CCD images of a mosaic: [`MOSAIC_CCDS`] `i16` images of 512 × 256 sky, each from its
/// own seed.
pub(crate) fn mosaic() -> Vec<Image> {
    (1..=MOSAIC_CCDS)
        .map(|ccd| sky(Bitpix::I16, 512, 256, ccd as u64))
        .collect()
}

/// The `EXTNAME` of the mosaic CCD `ccd`, counted from one: `CCD1` to `CCD60`.
pub(crate) fn ccd_name(ccd: usize) -> String {
    format!("CCD{ccd}")
}

/// The rows of [`spectra`].
pub(crate) const SPECTRA_ROWS: usize = 2000;

/// The samples of each row of [`spectra`], 1000 to 4000.
pub(crate) fn spectra_lengths() -> impl Iterator<Item = usize> {
    let mut draws = SplitMix::new(11);
    (0..SPECTRA_ROWS).map(move |_| 1000 + usize::try_from(draws.next() % 3001).unwrap())
}

/// A table of spectra, one row a source: its number and position, and its flux sampled on
/// [`spectra_lengths`] pixels, a variable-length `f32` array, the shape tiled-compressed tables
/// and spectral catalogs store.
pub(crate) fn spectra() -> Builder {
    let mut draws = SplitMix::new(12);
    let rows: Vec<ColumnData> = spectra_lengths()
        .map(|samples| {
            ColumnData::F32(
                (0..samples)
                    .map(|_| (1.0 + 0.05 * draws.gaussian()) as f32)
                    .collect(),
            )
        })
        .collect();
    let number = (1..=SPECTRA_ROWS)
        .map(|n| i32::try_from(n).unwrap())
        .collect();
    let ra = (0..SPECTRA_ROWS)
        .map(|_| 82.3 + 3.0 * draws.unit())
        .collect();
    let dec = (0..SPECTRA_ROWS)
        .map(|_| -6.4 + 2.0 * draws.unit())
        .collect();
    let columns = vec![
        WriteColumn::scalar("NUMBER", ColumnData::I32(number)),
        WriteColumn::scalar("RA", ColumnData::F64(ra)),
        WriteColumn::scalar("DEC", ColumnData::F64(dec)),
        WriteColumn::vla(
            "FLUX",
            Ragged::<ColumnData>::from_rows(rows).unwrap(),
            DescriptorWidth::P,
        ),
    ];
    Builder::with_columns(SPECTRA_ROWS, columns).unwrap()
}

/// [`CATALOG_ROWS`] values of `draw`, one a row.
fn column<T>(draws: &mut SplitMix, mut draw: impl FnMut(&mut SplitMix) -> T) -> Vec<T> {
    (0..CATALOG_ROWS).map(|_| draw(draws)).collect()
}
