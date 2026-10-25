use crate::fits_error::FitsError;
use crate::fits_error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::internals::cards;
use crate::internals::tolerance;
use crate::internals::tolerance::Tolerance;
use crate::internals::wcs_header;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::axis_transform::spectral_kind::SPEED_OF_LIGHT;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;
use crate::world_coordinates::internals::{to_pixel, to_world};
use crate::world_coordinates::spectral_frame::SpectralFrame;
use crate::world_coordinates::spectral_frame::SpectralReferenceFrame;
use std::assert_matches;

/// A one-axis velocity header of type `ctype`: 1000 m/s a pixel, 0 at pixel 1.
fn velocity_axis(ctype: &str) -> Header {
    wcs_header::axis_header(ctype, "m/s", 1.0, 0.0, 1_000.0)
}

/// The `VELO-F2V` velocity two pixels of 1000 m/s from a reference of 0 at RESTFRQ =
/// 1420405751 Hz. The pixels give ν = ν₀ − 2000·ν₀/c = 1420396275.072817 Hz in f64,
/// whose exact velocity c(ν₀² − ν²)/(ν₀² + ν²) is 2000.0066712779978 m/s; the difference
/// of squares lost 1.2e-8 m/s of it. (ν₀ − ν)(ν₀ + ν) keeps it to the four roundings
/// after the exact ν₀ − ν: two ulps of 2.3e-13 m/s.
fn assert_f2v_at_two_pixels(velocity: f64) {
    tolerance::assert_within(
        velocity,
        2_000.006_671_277_997_8,
        Tolerance::abs(2.0 * 2.3e-13, "two ulps of 2000 m/s"),
        "VELO-F2V at two pixels",
    );
}

/// A velocity from a frequency or a wavelength needs the rest value it is relative to.
#[test]
fn velocity_of_a_frequency_or_wavelength_needs_a_rest_value() {
    assert_matches!(
        Wcs::from_header(&velocity_axis("VELO-F2V"), None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::MissingRest
        })
    );
    assert_matches!(
        Wcs::from_header(&velocity_axis("VRAD-W2F"), None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::MissingRest
        })
    );
}

/// A radio velocity from an apparent radial velocity needs no rest value: the ratio of
/// frequencies is the ratio of velocities.
#[test]
fn radio_velocity_of_a_velocity_needs_no_rest_value() {
    let no_rest = Wcs::from_header(&velocity_axis("VRAD-V2F"), None).unwrap();
    let world = to_world(&no_rest, &[3.0]).unwrap()[0];
    let frequency =
        SPEED_OF_LIGHT * ((SPEED_OF_LIGHT - 2_000.0) / (SPEED_OF_LIGHT + 2_000.0)).sqrt();
    let expected = SPEED_OF_LIGHT * (1.0 - frequency / SPEED_OF_LIGHT);
    tolerance::assert_within(
        world,
        expected,
        Tolerance::abs(
            SPEED_OF_LIGHT * f64::EPSILON,
            "c times a ratio less 1 rounds at c·ε",
        ),
        "VRAD-V2F without a rest frequency",
    );
    // A velocity is c times a ratio less 1, which rounds at c·ε, and a pixel is 1000 m/s.
    wcs_header::assert_round_trip(
        &no_rest,
        &[[3.0]],
        Tolerance::abs(
            SPEED_OF_LIGHT * f64::EPSILON / 1_000.0,
            "c·ε in pixels of 1000 m/s",
        ),
        "VRAD-V2F",
    );
}

/// `RESTFRQ` and `RESTWAV` are two forms of one rest value, and `SPECSYS` names the frame.
/// The wavelength is c/ν₀, the frequency ν₀ = 1420405751 Hz.
#[test]
fn rest_frequency_or_wavelength_gives_one_velocity() {
    let mut by_frequency = velocity_axis("VELO-F2V");
    by_frequency
        .set_internal(keywords::RESTFRQ, 1_420_405_751.0)
        .set_internal(keywords::SPECSYS, "BARYCENT");
    let by_frequency = Wcs::from_header(&by_frequency, None).unwrap();
    assert_eq!(
        by_frequency.description().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Barycentric),
            observer: SpectralReferenceFrame::Topocentric,
            rest: SpectralRest {
                frequency: Some(1_420_405_751.0),
                wavelength: None,
            },
        })
    );
    let mut by_wavelength = velocity_axis("VELO-F2V");
    by_wavelength.set_internal(keywords::RESTWAV, SPEED_OF_LIGHT / 1_420_405_751.0);
    let by_wavelength = Wcs::from_header(&by_wavelength, None).unwrap();
    // c/(c/ν₀) rounds back to ν₀ exactly, so the two velocities agree bit for bit.
    assert_eq!(
        to_world(&by_wavelength, &[3.0]).unwrap()[0].to_bits(),
        to_world(&by_frequency, &[3.0]).unwrap()[0].to_bits()
    );
}

/// A velocity of c is a frequency of 0, which F2V cannot invert.
#[test]
fn velocity_of_light_has_no_pixel() {
    let mut header = velocity_axis("VELO-F2V");
    header.set_internal(keywords::RESTFRQ, 1_420_405_751.0);
    assert_matches!(
        to_pixel(&Wcs::from_header(&header, None).unwrap(), &[SPEED_OF_LIGHT]),
        Err(FitsError::WcsCoordinateOutOfDomain {
            axis: 0,
            algorithm: "F2V"
        })
    );
}

#[test]
fn deprecated_restfreq_reads_as_restfrq() {
    let mut deprecated = velocity_axis("VELO-F2V");
    deprecated.set_internal(keywords::RESTFREQ, 1_420_405_751.0);
    assert_f2v_at_two_pixels(
        to_world(&Wcs::from_header(&deprecated, None).unwrap(), &[3.0]).unwrap()[0],
    );
}

#[test]
fn zero_rest_frequency_is_refused() {
    let mut invalid = velocity_axis("VELO-F2V");
    invalid.set_internal(keywords::RESTFRQ, 0.0);
    assert_matches!(
        Wcs::from_header(&invalid, None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::RestValue {
                keyword: keywords::RESTFRQ
            }
        })
    );
}

/// The pixel-list and array-column forms read their frames and rest values, each column
/// its own.
#[test]
fn table_forms_read_their_frames_and_rest_values() {
    let pixel_list = cards::header_of(&[
        ("TCTYP2", "VELO-F2V".into()),
        ("TCUNI2", "m/s".into()),
        ("TCRPX2", 1.0.into()),
        ("TCRVL2", 0.0.into()),
        ("TCDLT2", 1_000.0.into()),
        ("RFRQ2", 1_420_405_751.0.into()),
        ("SPEC2", "BARYCENT".into()),
        ("SOBS2", "GEOCENTR".into()),
        ("TCTYP4", "WAVE".into()),
        ("TCUNI4", "m".into()),
        ("TCRPX4", 1.0.into()),
        ("TCRVL4", 5.0e-7.into()),
        ("TCDLT4", 1.0e-9.into()),
        ("RWAV4", 5.0e-7.into()),
        ("SPEC4", "SOURCE".into()),
        ("SOBS4", "HELIOCEN".into()),
    ]);
    let pixel_list = Wcs::from_pixel_list(&pixel_list, &[2, 4], None).unwrap();
    assert_f2v_at_two_pixels(to_world(&pixel_list, &[3.0, 1.0]).unwrap()[0]);
    assert_eq!(
        pixel_list.description().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Barycentric),
            observer: SpectralReferenceFrame::Geocentric,
            rest: SpectralRest {
                frequency: Some(1_420_405_751.0),
                wavelength: None,
            },
        })
    );
    assert_eq!(
        pixel_list.description().axes[1].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Source),
            observer: SpectralReferenceFrame::Heliocentric,
            rest: SpectralRest {
                frequency: None,
                wavelength: Some(5.0e-7),
            },
        })
    );

    let vector = cards::header_of(&[
        ("WCAX5A", 1.into()),
        ("1CTY5A", "VELO-F2V".into()),
        ("1CUN5A", "m/s".into()),
        ("1CRP5A", 1.0.into()),
        ("1CRV5A", 0.0.into()),
        ("1CDE5A", 1_000.0.into()),
        ("RWAV5A", (SPEED_OF_LIGHT / 1_420_405_751.0).into()),
        ("SPEC5A", "LSRK".into()),
        ("SOBS5A", "TOPOCENT".into()),
    ]);
    let vector = Wcs::from_array_column(&vector, 5, 'A').unwrap();
    assert_f2v_at_two_pixels(to_world(&vector, &[3.0]).unwrap()[0]);
    assert_eq!(
        vector.description().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::LsrKinematic),
            observer: SpectralReferenceFrame::Topocentric,
            rest: SpectralRest {
                frequency: None,
                wavelength: Some(SPEED_OF_LIGHT / 1_420_405_751.0),
            },
        })
    );
}

/// A header whose two descriptions name different frames.
fn alternate_frames() -> Header {
    let mut alternate = Header::new();
    alternate
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "WAVE")
        .set_internal("CTYPE1A", "FREQ")
        .set_internal(keywords::SPECSYS, "GEOCENTR")
        .set_internal("SPECSYSA", "CMBDIPOL")
        .set_internal("SSYSOBSA", "BARYCENT")
        .set_internal("RESTFRQA", 1_420_405_751.0);
    alternate
}

#[test]
fn each_description_reads_its_own_frame() {
    assert_eq!(
        Wcs::from_header(&alternate_frames(), 'A')
            .unwrap()
            .description()
            .axes[0]
            .spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::CmbDipole),
            observer: SpectralReferenceFrame::Barycentric,
            rest: SpectralRest {
                frequency: Some(1_420_405_751.0),
                wavelength: None,
            },
        })
    );
}

/// A system Table 27 does not list keeps its text, and the spectral axis reads.
#[test]
fn unlisted_system_keeps_its_text() {
    let mut alternate = alternate_frames();
    alternate.set_internal("SPECSYSA", " UNKNOWN");
    let other = Wcs::from_header(&alternate, 'A').unwrap();
    let frame = other
        .description()
        .axes
        .iter()
        .find_map(|axis| axis.spectral_frame.clone())
        .expect("a spectral axis");
    assert_eq!(
        frame.coordinate,
        Some(SpectralReferenceFrame::Other("UNKNOWN".to_string()))
    );
    assert!(other.description().unsupported_axes.is_empty());
}

#[test]
fn each_table_27_system_parses() {
    for (value, expected) in [
        ("TOPOCENT", SpectralReferenceFrame::Topocentric),
        ("GEOCENTR", SpectralReferenceFrame::Geocentric),
        ("BARYCENT", SpectralReferenceFrame::Barycentric),
        ("HELIOCEN", SpectralReferenceFrame::Heliocentric),
        ("LSRK", SpectralReferenceFrame::LsrKinematic),
        ("LSRD", SpectralReferenceFrame::LsrDynamic),
        ("GALACTOC", SpectralReferenceFrame::Galactocentric),
        ("LOCALGRP", SpectralReferenceFrame::LocalGroup),
        ("CMBDIPOL", SpectralReferenceFrame::CmbDipole),
        ("SOURCE", SpectralReferenceFrame::Source),
    ] {
        let mut header = Header::new();
        header
            .set_internal(keywords::NAXIS, 1)
            .set_internal("CTYPE1", "WAVE")
            .set_internal(keywords::SPECSYS, value);
        assert_eq!(
            Wcs::from_header(&header, None).unwrap().description().axes[0]
                .spectral_frame
                .clone()
                .unwrap()
                .coordinate,
            Some(expected),
            "{value}"
        );
    }
}
