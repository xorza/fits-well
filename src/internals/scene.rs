//! The standard scene every case measures on: sky, as a camera writes it, drawn from fixed seeds
//! so two runs time the same work.

use crate::bintable::column_data::ColumnData;
use crate::bitpix::Bitpix;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::header_model::{Header, keywords};
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use crate::internals::memory::{memory_writer, written, written_by};
use crate::internals::rng::SplitMix;

/// The standard frame's width: 6000 × 4000, an APS-C camera's 24 MP, as the lumos benches use.
pub(crate) const WIDTH: usize = 6000;
/// The standard frame's height.
pub(crate) const HEIGHT: usize = 4000;

/// The rows of [`catalog`]: the stars a deep frame's source extraction finds.
pub(crate) const CATALOG_ROWS: usize = 100_000;
/// The bytes of one row of [`catalog`]: an `i32`, four `f64`, four `f32` and an `i16`.
pub(crate) const CATALOG_ROW_BYTES: usize = 4 + 4 * 8 + 4 * 4 + 2;

/// `width` × `height` samples of sky stored as `bitpix` from `seed`: 0.1 of full scale with
/// white noise of σ 0.01, one pixel in fifty a star up to 1 above it, clamped to `[0, 1]`. An
/// `i16` frame is a camera's light: `u16` counts stored with `BZERO = 32768`; the other integers
/// span their positive range, the floats `[0, 1]`.
#[expect(clippy::cast_sign_loss, reason = "every value is clamped to [0, 1]")]
pub(crate) fn sky(bitpix: Bitpix, width: usize, height: usize, seed: u64) -> Image {
    let units = (0..width * height).map(|index| sky_value(pixel(seed, index)));
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

/// The defect mask of [`sky`] at `seed`: `u8` flags, 1 on every star.
#[cfg(feature = "compression")]
pub(crate) fn mask(width: usize, height: usize, seed: u64) -> Image {
    let flags = (0..width * height).map(|index| u8::from(star(&mut pixel(seed, index))));
    Image::new(
        vec![width, height],
        Data::U8(flags.collect()),
        Scaling::IDENTITY,
    )
    .unwrap()
}

/// The header a capture program writes on a light, past the structural keywords the image
/// sets: the exposure, the camera, the mount, the site, the target, the software, and the plate
/// solution: a gnomonic projection at the frame's centre, 3.1″ a pixel rotated by 92.4°.
pub(crate) fn light_header() -> Header {
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
        .set_internal("EGAIN", 0.25)
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
        .set_internal("SWCREATE", "N.I.N.A. 3.1.2.9001")
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
    header
        .push_comment("NINA - Nighttime Imaging 'N' Astronomy")
        .unwrap()
        .push_history("Calibrated: none")
        .unwrap();
    header
}

/// The bytes of the file a capture program writes: the standard frame of `bitpix` under
/// [`light_header`], with its checksums.
pub(crate) fn frame_file(bitpix: Bitpix) -> Vec<u8> {
    written_by(memory_writer().with_checksums(), |w| {
        w.write_image(sky(bitpix, WIDTH, HEIGHT, 1).view(), Some(&light_header()))
    })
}

/// A source catalog of the standard frame, [`CATALOG_ROWS`] stars: a running number, the
/// position in pixels and on the sky, the flux and its error, the width and the elongation, and
/// the extraction flags.
pub(crate) fn catalog() -> Builder {
    let mut draws = SplitMix::new(7);
    let x = column(&mut draws, |d| 0.5 + WIDTH as f64 * d.unit());
    let y = column(&mut draws, |d| 0.5 + HEIGHT as f64 * d.unit());
    let ra = column(&mut draws, |d| 82.3 + 3.0 * d.unit());
    let dec = column(&mut draws, |d| -6.4 + 2.0 * d.unit());
    let flux = column(&mut draws, |d| (8.0 + 1.5 * d.gaussian()).exp() as f32);
    let flux_err = column(&mut draws, |d| (20.0 + 5.0 * d.unit()) as f32);
    let fwhm = column(&mut draws, |d| (2.5 + 0.3 * d.gaussian()) as f32);
    let elong = column(&mut draws, |d| (1.0 + 0.2 * d.unit()) as f32);
    let flags = column(&mut draws, |d| i16::from(d.next().is_multiple_of(20)));
    let number = (1..=CATALOG_ROWS)
        .map(|n| i32::try_from(n).unwrap())
        .collect();
    let columns = vec![
        WriteColumn::scalar("NUMBER", ColumnData::I32(number)),
        WriteColumn::scalar("X_IMAGE", ColumnData::F64(x)),
        WriteColumn::scalar("Y_IMAGE", ColumnData::F64(y)),
        WriteColumn::scalar("RA", ColumnData::F64(ra)),
        WriteColumn::scalar("DEC", ColumnData::F64(dec)),
        WriteColumn::scalar("FLUX", ColumnData::F32(flux)),
        WriteColumn::scalar("FLUX_ERR", ColumnData::F32(flux_err)),
        WriteColumn::scalar("FWHM", ColumnData::F32(fwhm)),
        WriteColumn::scalar("ELONG", ColumnData::F32(elong)),
        WriteColumn::scalar("FLAGS", ColumnData::I16(flags)),
    ];
    Builder::with_columns(CATALOG_ROWS, columns).unwrap()
}

/// The bytes of a file holding [`catalog`] in a binary table after an empty primary.
pub(crate) fn catalog_file() -> Vec<u8> {
    written(|w| w.write_table(&catalog(), None))
}

/// The stream of the pixel at `index` of the sky at `seed`.
const fn pixel(seed: u64, index: usize) -> SplitMix {
    SplitMix::new((seed << 40) ^ index as u64)
}

/// Whether this pixel is a star: one in fifty, from its first draw.
const fn star(draws: &mut SplitMix) -> bool {
    draws.next().is_multiple_of(50)
}

/// This pixel's value on `[0, 1]`: 0.1 with white noise of σ 0.01, and a star up to 1 above it.
fn sky_value(mut draws: SplitMix) -> f64 {
    let amplitude = if star(&mut draws) { draws.unit() } else { 0.0 };
    (0.1 + 0.01 * draws.gaussian() + amplitude).clamp(0.0, 1.0)
}

/// [`CATALOG_ROWS`] values of `draw`, one a row.
fn column<T>(draws: &mut SplitMix, mut draw: impl FnMut(&mut SplitMix) -> T) -> Vec<T> {
    (0..CATALOG_ROWS).map(|_| draw(draws)).collect()
}
