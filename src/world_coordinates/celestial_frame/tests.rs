use crate::fits_error::FitsError;
use crate::fits_error::WcsDefect;
use crate::header_model::{Header, keywords};

use crate::internals::wcs_header::WcsHeader;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::celestial_frame::CelestialFrame;
use crate::world_coordinates::celestial_frame::CelestialReferenceFrame;

#[test]
fn celestial_frame_metadata_resolves_defaults_alternates_and_table_forms() {
    let image = |equinox: Option<f64>, radesys: Option<&str>| {
        let mut header =
            WcsHeader::celestial("TAN", [1.0, 1.0], [0.0, 0.0], [1.0, 1.0]).image_header();
        if let Some(equinox) = equinox {
            header.set_internal(keywords::EQUINOX, equinox);
        }
        if let Some(radesys) = radesys {
            header.set_internal(keywords::RADESYS, radesys);
        }
        header
    };
    assert_eq!(
        Wcs::from_header(&image(None, None), None)
            .unwrap()
            .description()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Icrs,
            equinox: None,
        })
    );
    // §8.1: without RADESYS, an equinox before 1984.0 is FK4, and from it on FK5.
    for (equinox, reference_frame) in [
        (1950.0, CelestialReferenceFrame::Fk4),
        (1983.999, CelestialReferenceFrame::Fk4),
        (1984.0, CelestialReferenceFrame::Fk5),
        (2000.0, CelestialReferenceFrame::Fk5),
    ] {
        assert_eq!(
            Wcs::from_header(&image(Some(equinox), None), None)
                .unwrap()
                .description()
                .celestial_frame,
            Some(CelestialFrame {
                reference_frame,
                equinox: Some(equinox),
            })
        );
    }
    assert_eq!(
        Wcs::from_header(&image(Some(1975.0), Some("FK4-NO-E")), None)
            .unwrap()
            .description()
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
            .description()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Gappt,
            equinox: None,
        })
    );
    assert_eq!(
        Wcs::from_header(&alternate, 'A')
            .unwrap()
            .description()
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
        Wcs::from_pixel_list(&pixel_list, &[2, 3], 'A')
            .unwrap()
            .description()
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
        Wcs::from_array_column(&vector, 5, 'A')
            .unwrap()
            .description()
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
        unsupported.description().celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );
    assert_eq!(unsupported.description().unsupported_axes, [0, 1]);

    assert!(matches!(
        Wcs::from_header(&image(None, Some("J2000")), None),
        Err(FitsError::InvalidWcs { defect: WcsDefect::UnknownCelestialFrame { value } }) if value == "J2000"
    ));
    assert!(matches!(
        Wcs::from_header(&image(Some(-1.0), None), None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::InvalidEquinox
        })
    ));
    pixel_list.set_internal("RADE3A", "FK4");
    assert!(matches!(
        Wcs::from_pixel_list(&pixel_list, &[2, 3], 'A'),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
}
