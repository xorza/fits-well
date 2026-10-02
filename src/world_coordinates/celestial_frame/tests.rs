use crate::error::FitsError;
use crate::header_model::Header;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::celestial_frame::CelestialFrame;
use crate::world_coordinates::celestial_frame::CelestialReferenceFrame;

#[test]
fn celestial_frame_metadata_resolves_defaults_alternates_and_table_forms() {
    let image = |equinox: Option<f64>, radesys: Option<&str>| {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 2)
            .set_internal("CTYPE1", "RA---TAN")
            .set_internal("CTYPE2", "DEC--TAN");
        if let Some(equinox) = equinox {
            header.set_internal("EQUINOX", equinox);
        }
        if let Some(radesys) = radesys {
            header.set_internal("RADESYS", radesys);
        }
        header
    };
    assert_eq!(
        Wcs::from_header(&image(None, None), None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Icrs,
            equinox: None,
        })
    );
    assert_eq!(
        Wcs::from_header(&image(Some(1950.0), None), None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk4,
            equinox: Some(1950.0),
        })
    );
    assert_eq!(
        Wcs::from_header(&image(Some(2000.0), None), None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );
    assert_eq!(
        Wcs::from_header(&image(Some(1975.0), Some("FK4-NO-E")), None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk4NoE,
            equinox: Some(1975.0),
        })
    );

    let mut alternate = image(None, Some("GAPPT"));
    alternate
        .set_internal("CTYPE1A", "RA---TAN")
        .set_internal("CTYPE2A", "DEC--TAN")
        .set_internal("EQUINOXA", 1970.0);
    assert_eq!(
        Wcs::from_header(&alternate, None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Gappt,
            equinox: None,
        })
    );
    assert_eq!(
        Wcs::from_header(&alternate, Some('A'))
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk4,
            equinox: Some(1970.0),
        })
    );

    let mut pixel_list = Header::new();
    pixel_list
        .set_internal("TCTY2A", "RA---TAN")
        .set_internal("TCTY3A", "DEC--TAN")
        .set_internal("RADE2A", "FK5")
        .set_internal("RADE3A", "FK5")
        .set_internal("EQUI2A", 2000.0)
        .set_internal("EQUI3A", 2000.0);
    assert_eq!(
        Wcs::from_pixel_list(&pixel_list, &[2, 3], Some('A'))
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );

    let mut vector = Header::new();
    vector
        .set_internal("1CTY5A", "RA---TAN")
        .set_internal("2CTY5A", "DEC--TAN")
        .set_internal("RADE5A", "ICRS");
    assert_eq!(
        Wcs::from_array_column(&vector, 5, Some('A'))
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Icrs,
            equinox: None,
        })
    );

    let mut unsupported = Header::new();
    unsupported
        .set_internal("TCTYP2", "RA---XPH")
        .set_internal("TCTYP3", "DEC--XPH")
        .set_internal("RADE2", "FK5")
        .set_internal("RADE3", "FK5")
        .set_internal("EQUI2", 2000.0)
        .set_internal("EQUI3", 2000.0);
    let unsupported = Wcs::from_pixel_list(&unsupported, &[2, 3], None).unwrap();
    assert_eq!(
        unsupported.view().celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );
    assert_eq!(unsupported.view().unsupported_axes, [0, 1]);

    assert!(matches!(
        Wcs::from_header(&image(None, Some("J2000")), None),
        Err(FitsError::InvalidWcs { detail }) if detail.contains("RADESYS")
    ));
    assert!(matches!(
        Wcs::from_header(&image(Some(-1.0), None), None),
        Err(FitsError::InvalidWcs { detail }) if detail.contains("EQUINOX")
    ));
    pixel_list.set_internal("RADE3A", "FK4");
    assert!(matches!(
        Wcs::from_pixel_list(&pixel_list, &[2, 3], Some('A')),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
}
