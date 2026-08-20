use crate::error::FitsError;
use crate::header::Header;
use crate::wcs::spectral_frame::SpectralFrame;
use crate::wcs::spectral_frame::SpectralReferenceFrame;

#[test]
fn spectral_rest_metadata_is_required_resolved_and_table_aware() {
    let velocity_axis = |ctype: &str| {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", ctype)
            .set_internal("CUNIT1", "m/s")
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", 0.0)
            .set_internal("CDELT1", 1_000.0);
        header
    };
    assert!(matches!(
        velocity_axis("VELO-F2V").wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RESTFRQ or RESTWAV")
    ));
    assert!(matches!(
        velocity_axis("VRAD-W2F").wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RESTFRQ or RESTWAV")
    ));

    let no_rest = velocity_axis("VRAD-V2F").wcs(None).unwrap();
    let world = no_rest.pixel_to_world(&[3.0]).unwrap()[0];
    let speed_of_light: f64 = 2.997_924_58e8;
    let frequency =
        speed_of_light * ((speed_of_light - 2_000.0) / (speed_of_light + 2_000.0)).sqrt();
    let expected = speed_of_light * (1.0 - frequency / speed_of_light);
    assert!((world - expected).abs() < 1e-8);
    assert!((no_rest.world_to_pixel(&[world]).unwrap()[0] - 3.0).abs() < 1e-10);

    let mut by_frequency = velocity_axis("VELO-F2V");
    by_frequency
        .set_internal("RESTFRQ", 1_420_405_751.0)
        .set_internal("SPECSYS", "BARYCENT");
    let by_frequency = by_frequency.wcs(None).unwrap();
    assert_eq!(
        by_frequency.view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Barycentric),
            observer: SpectralReferenceFrame::Topocentric,
            rest_frequency_hz: Some(1_420_405_751.0),
            rest_wavelength_m: None,
        })
    );
    let mut by_wavelength = velocity_axis("VELO-F2V");
    by_wavelength.set_internal("RESTWAV", 2.997_924_58e8 / 1_420_405_751.0);
    let by_wavelength = by_wavelength.wcs(None).unwrap();
    assert!(
        (by_frequency.pixel_to_world(&[3.0]).unwrap()[0]
            - by_wavelength.pixel_to_world(&[3.0]).unwrap()[0])
            .abs()
            < 1e-12
    );
    assert!(matches!(
        by_frequency.world_to_pixel(&[2.997_924_58e8]),
        Err(FitsError::WcsCoordinateDomain {
            axis: 0,
            algorithm: "F2V"
        })
    ));

    let mut deprecated = velocity_axis("VELO-F2V");
    deprecated.set_internal("RESTFREQ", 1_420_405_751.0);
    assert!(
        (deprecated
            .wcs(None)
            .unwrap()
            .pixel_to_world(&[3.0])
            .unwrap()[0]
            - 2_000.006_671_265_423_6)
            .abs()
            < 1e-9
    );

    let mut invalid = velocity_axis("VELO-F2V");
    invalid.set_internal("RESTFRQ", 0.0);
    assert!(matches!(
        invalid.wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RESTFRQ")
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
    let pixel_list = pixel_list.wcs_pixel_list(&[2, 4], None).unwrap();
    assert!(
        (pixel_list.pixel_to_world(&[3.0, 1.0]).unwrap()[0] - 2_000.006_671_265_423_6).abs() < 1e-9
    );
    assert_eq!(
        pixel_list.view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Barycentric),
            observer: SpectralReferenceFrame::Geocentric,
            rest_frequency_hz: Some(1_420_405_751.0),
            rest_wavelength_m: None,
        })
    );
    assert_eq!(
        pixel_list.view().axes[1].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Source),
            observer: SpectralReferenceFrame::Heliocentric,
            rest_frequency_hz: None,
            rest_wavelength_m: Some(5.0e-7),
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
        .set_internal("RWAV5A", 2.997_924_58e8 / 1_420_405_751.0)
        .set_internal("SPEC5A", "LSRK")
        .set_internal("SOBS5A", "TOPOCENT");
    let vector = vector.wcs_array_column(5, Some('A')).unwrap();
    assert!((vector.pixel_to_world(&[3.0]).unwrap()[0] - 2_000.006_671_265_423_6).abs() < 1e-9);
    assert_eq!(
        vector.view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::LsrKinematic),
            observer: SpectralReferenceFrame::Topocentric,
            rest_frequency_hz: None,
            rest_wavelength_m: Some(2.997_924_58e8 / 1_420_405_751.0),
        })
    );

    let mut alternate = Header::new();
    alternate
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "WAVE")
        .set_internal("CTYPE1A", "FREQ")
        .set_internal("SPECSYS", "GEOCENTR")
        .set_internal("SPECSYSA", "CMBDIPOL")
        .set_internal("SSYSOBSA", "BARYCENT")
        .set_internal("RESTFRQA", 1_420_405_751.0);
    assert_eq!(
        alternate.wcs(Some('A')).unwrap().view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::CmbDipole),
            observer: SpectralReferenceFrame::Barycentric,
            rest_frequency_hz: Some(1_420_405_751.0),
            rest_wavelength_m: None,
        })
    );

    alternate.set_internal("SPECSYSA", "UNKNOWN");
    assert!(matches!(
        alternate.wcs(Some('A')),
        Err(FitsError::InvalidValue { card }) if card.contains("SPECSYS")
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
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", "WAVE")
            .set_internal("SPECSYS", value);
        assert_eq!(
            header.wcs(None).unwrap().view().axes[0]
                .spectral_frame
                .unwrap()
                .coordinate,
            Some(expected),
            "{value}"
        );
    }
}
