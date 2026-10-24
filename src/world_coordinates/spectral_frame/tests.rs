use crate::fits_error::FitsError;
use crate::fits_error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::internals::tolerance::Tolerance;
use crate::internals::wcs_header::{assert_round_trip, axis_header};
use crate::world_coordinates::internals::{to_pixel, to_world};

use crate::world_coordinates::Wcs;
use crate::world_coordinates::axis_transform::spectral_kind::SPEED_OF_LIGHT;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;

use crate::world_coordinates::spectral_frame::SpectralFrame;
use crate::world_coordinates::spectral_frame::SpectralReferenceFrame;

/// The `VELO-F2V` velocity two pixels of 1000 m/s from a reference of 0 at RESTFRQ =
/// 1420405751 Hz. The pixels give ν = ν₀ − 2000·ν₀/c = 1420396275.072817 Hz in f64,
/// whose exact velocity c(ν₀² − ν²)/(ν₀² + ν²) is 2000.0066712779978 m/s; the difference
/// of squares lost 1.2e-8 m/s of it. (ν₀ − ν)(ν₀ + ν) keeps it to the four roundings
/// after the exact ν₀ − ν: two ulps of 2.3e-13 m/s.
fn assert_f2v_at_two_pixels(velocity: f64) {
    assert!(
        (velocity - 2_000.006_671_277_997_8).abs() <= 2.0 * 2.3e-13,
        "{velocity:?}"
    );
}

#[test]
fn spectral_rest_metadata_is_required_resolved_and_table_aware() {
    let velocity_axis = |ctype: &str| axis_header(ctype, "m/s", 1.0, 0.0, 1_000.0);
    assert!(matches!(
        Wcs::from_header(&velocity_axis("VELO-F2V"), None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::MissingRest
        })
    ));
    assert!(matches!(
        Wcs::from_header(&velocity_axis("VRAD-W2F"), None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::MissingRest
        })
    ));

    let no_rest = Wcs::from_header(&velocity_axis("VRAD-V2F"), None).unwrap();
    let world = to_world(&no_rest, &[3.0]).unwrap()[0];
    let frequency =
        SPEED_OF_LIGHT * ((SPEED_OF_LIGHT - 2_000.0) / (SPEED_OF_LIGHT + 2_000.0)).sqrt();
    let expected = SPEED_OF_LIGHT * (1.0 - frequency / SPEED_OF_LIGHT);
    assert!((world - expected).abs() < 1e-8);
    // A velocity is c times a ratio less 1, which rounds at c·ε, and a pixel is 1000 m/s.
    assert_round_trip(
        &no_rest,
        &[[3.0]],
        Tolerance::abs(
            SPEED_OF_LIGHT * f64::EPSILON / 1_000.0,
            "c·ε in pixels of 1000 m/s",
        ),
        "VRAD-V2F",
    );

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
    assert!(
        (to_world(&by_frequency, &[3.0]).unwrap()[0]
            - to_world(&by_wavelength, &[3.0]).unwrap()[0])
            .abs()
            < 1e-12
    );
    assert!(matches!(
        to_pixel(&by_frequency, &[SPEED_OF_LIGHT]),
        Err(FitsError::WcsCoordinateOutOfDomain {
            axis: 0,
            algorithm: "F2V"
        })
    ));

    let mut deprecated = velocity_axis("VELO-F2V");
    deprecated.set_internal(keywords::RESTFREQ, 1_420_405_751.0);
    assert_f2v_at_two_pixels(
        to_world(&Wcs::from_header(&deprecated, None).unwrap(), &[3.0]).unwrap()[0],
    );

    let mut invalid = velocity_axis("VELO-F2V");
    invalid.set_internal(keywords::RESTFRQ, 0.0);
    assert!(matches!(
        Wcs::from_header(&invalid, None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::RestValue {
                keyword: keywords::RESTFRQ
            }
        })
    ));

    let mut pixel_list = Header::new();
    pixel_list
        .set_internal("TCTYP2", "VELO-F2V")
        .set_internal("TCUNI2", "m/s")
        .set_internal("TCRPX2", 1.0)
        .set_internal("TCRVL2", 0.0)
        .set_internal("TCDLT2", 1_000.0)
        .set_internal("RFRQ2", 1_420_405_751.0)
        .set_internal("SPEC2", "BARYCENT")
        .set_internal("SOBS2", "GEOCENTR")
        .set_internal("TCTYP4", "WAVE")
        .set_internal("TCUNI4", "m")
        .set_internal("TCRPX4", 1.0)
        .set_internal("TCRVL4", 5.0e-7)
        .set_internal("TCDLT4", 1.0e-9)
        .set_internal("RWAV4", 5.0e-7)
        .set_internal("SPEC4", "SOURCE")
        .set_internal("SOBS4", "HELIOCEN");
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

    let mut vector = Header::new();
    vector
        .set_internal("WCAX5A", 1)
        .set_internal("1CTY5A", "VELO-F2V")
        .set_internal("1CUN5A", "m/s")
        .set_internal("1CRP5A", 1.0)
        .set_internal("1CRV5A", 0.0)
        .set_internal("1CDE5A", 1_000.0)
        .set_internal("RWAV5A", SPEED_OF_LIGHT / 1_420_405_751.0)
        .set_internal("SPEC5A", "LSRK")
        .set_internal("SOBS5A", "TOPOCENT");
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

    let mut alternate = Header::new();
    alternate
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "WAVE")
        .set_internal("CTYPE1A", "FREQ")
        .set_internal(keywords::SPECSYS, "GEOCENTR")
        .set_internal("SPECSYSA", "CMBDIPOL")
        .set_internal("SSYSOBSA", "BARYCENT")
        .set_internal("RESTFRQA", 1_420_405_751.0);
    assert_eq!(
        Wcs::from_header(&alternate, 'A')
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

    alternate.set_internal("SPECSYSA", "UNKNOWN");
    assert!(matches!(
        Wcs::from_header(&alternate, 'A'),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::UnknownSpectralFrame { keyword: keywords::SPECSYS, value }
        }) if value == "UNKNOWN"
    ));

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
                .unwrap()
                .coordinate,
            Some(expected),
            "{value}"
        );
    }
}
