use crate::fits_error::FitsError;
use crate::fits_error::WcsConflict;
use crate::fits_error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::internals::cards;
use crate::internals::wcs_header::WcsHeader;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::celestial_frame::CelestialFrame;
use crate::world_coordinates::celestial_frame::CelestialReferenceFrame;
use std::assert_matches;

/// A TAN image header with the given `EQUINOX` and `RADESYS`.
fn image(equinox: Option<f64>, radesys: Option<&str>) -> Header {
    let mut header = WcsHeader::celestial("TAN", [1.0, 1.0], [0.0, 0.0], [1.0, 1.0]).image_header();
    if let Some(equinox) = equinox {
        header.set_internal(keywords::EQUINOX, equinox);
    }
    if let Some(radesys) = radesys {
        header.set_internal(keywords::RADESYS, radesys);
    }
    header
}

/// The frame of the image description `alt` of `header`.
fn frame(header: &Header, alt: Option<char>) -> Option<CelestialFrame> {
    Wcs::from_header(header, alt)
        .unwrap()
        .description()
        .celestial_frame
        .cloned()
}

const fn celestial(
    reference_frame: CelestialReferenceFrame,
    equinox: Option<f64>,
) -> CelestialFrame {
    CelestialFrame {
        reference_frame,
        equinox,
    }
}

/// With neither keyword the frame is ICRS. §8.1: without `RADESYS`, an equinox before
/// 1984.0 is FK4, and from it on FK5. A `RADESYS` names the frame whatever the equinox.
#[test]
fn frame_defaults_from_its_equinox() {
    assert_eq!(
        frame(&image(None, None), None),
        Some(celestial(CelestialReferenceFrame::Icrs, None))
    );
    for (equinox, reference_frame) in [
        (1950.0, CelestialReferenceFrame::Fk4),
        (1983.999, CelestialReferenceFrame::Fk4),
        (1984.0, CelestialReferenceFrame::Fk5),
        (2000.0, CelestialReferenceFrame::Fk5),
    ] {
        assert_eq!(
            frame(&image(Some(equinox), None), None),
            Some(celestial(reference_frame, Some(equinox)))
        );
    }
    assert_eq!(
        frame(&image(Some(1975.0), Some("FK4-NO-E")), None),
        Some(celestial(CelestialReferenceFrame::Fk4NoE, Some(1975.0)))
    );
}

/// §8.3: `EPOCH` is the deprecated `EQUINOX`, read in its absence, on the primary
/// description alone: `EPOCH = 1950` is FK4 of 1950, and `EQUINOX` wins over it.
#[test]
fn epoch_is_the_deprecated_equinox_of_the_primary_description() {
    let mut epoch = image(None, None);
    epoch.set_internal(keywords::EPOCH, 1950.0);
    assert_eq!(
        frame(&epoch, None),
        Some(celestial(CelestialReferenceFrame::Fk4, Some(1950.0)))
    );
    epoch.set_internal(keywords::EQUINOX, 2000.0);
    assert_eq!(
        frame(&epoch, None),
        Some(celestial(CelestialReferenceFrame::Fk5, Some(2000.0)))
    );
    epoch.remove_all(keywords::EQUINOX).unwrap();
    epoch
        .set_internal("CTYPE1A", "RA---TAN")
        .set_internal("CTYPE2A", "DEC--TAN");
    assert_eq!(
        frame(&epoch, Some('A')),
        Some(celestial(CelestialReferenceFrame::Icrs, None))
    );
}

#[test]
fn each_description_reads_its_own_frame() {
    let mut alternate = image(None, Some("GAPPT"));
    alternate
        .set_internal("CTYPE1A", "RA---TAN")
        .set_internal("CTYPE2A", "DEC--TAN")
        .set_internal("EQUINOXA", 1970.0);
    assert_eq!(
        frame(&alternate, None),
        Some(celestial(CelestialReferenceFrame::Gappt, None))
    );
    assert_eq!(
        frame(&alternate, Some('A')),
        Some(celestial(CelestialReferenceFrame::Fk4, Some(1970.0)))
    );
}

/// The pixel-list and array-column forms read their frame, and so does a description
/// whose projection is not supported.
#[test]
fn table_forms_read_their_frame() {
    let pixel_list = cards::header_of(&[
        ("TCTY2A", "RA---TAN".into()),
        ("TCTY3A", "DEC--TAN".into()),
        ("RADE2A", "FK5".into()),
        ("RADE3A", "FK5".into()),
        ("EQUI2A", 2000.0.into()),
        ("EQUI3A", 2000.0.into()),
    ]);
    assert_eq!(
        Wcs::from_pixel_list(&pixel_list, &[2, 3], 'A')
            .unwrap()
            .description()
            .celestial_frame
            .cloned(),
        Some(celestial(CelestialReferenceFrame::Fk5, Some(2000.0)))
    );

    let vector = cards::header_of(&[
        ("1CTY5A", "RA---TAN".into()),
        ("2CTY5A", "DEC--TAN".into()),
        ("RADE5A", "ICRS".into()),
    ]);
    assert_eq!(
        Wcs::from_array_column(&vector, 5, 'A')
            .unwrap()
            .description()
            .celestial_frame
            .cloned(),
        Some(celestial(CelestialReferenceFrame::Icrs, None))
    );

    let unsupported = cards::header_of(&[
        ("TCTYP2", "RA---XPH".into()),
        ("TCTYP3", "DEC--XPH".into()),
        ("RADE2", "FK5".into()),
        ("RADE3", "FK5".into()),
        ("EQUI2", 2000.0.into()),
        ("EQUI3", 2000.0.into()),
    ]);
    let unsupported = Wcs::from_pixel_list(&unsupported, &[2, 3], None).unwrap();
    assert_eq!(
        unsupported.description().celestial_frame.cloned(),
        Some(celestial(CelestialReferenceFrame::Fk5, Some(2000.0)))
    );
    assert_eq!(unsupported.description().unsupported_axes, [0, 1]);
}

/// A frame the standard does not name keeps its text, and the celestial axes read.
#[test]
fn unnamed_frame_keeps_its_text() {
    let other = Wcs::from_header(&image(None, Some("J2000")), None).unwrap();
    assert_eq!(
        other.description().celestial_frame.cloned(),
        Some(celestial(
            CelestialReferenceFrame::Other("J2000".to_string()),
            None
        ))
    );
    assert!(other.description().unsupported_axes.is_empty());
}

/// A negative equinox, and two columns of one pixel list that name two frames, are
/// refused.
#[test]
fn defective_frame_keywords_are_refused() {
    assert_matches!(
        Wcs::from_header(&image(Some(-1.0), None), None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::InvalidEquinox
        })
    );
    let conflicting = cards::header_of(&[
        ("TCTY2A", "RA---TAN".into()),
        ("TCTY3A", "DEC--TAN".into()),
        ("RADE2A", "FK5".into()),
        ("RADE3A", "FK4".into()),
    ]);
    assert_matches!(
        Wcs::from_pixel_list(&conflicting, &[2, 3], 'A'),
        Err(FitsError::ConflictingWcsKeywords {
            conflict: WcsConflict::TableRadesys
        })
    );
}
