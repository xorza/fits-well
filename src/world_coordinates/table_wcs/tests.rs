use crate::fits_error::FitsError;
use crate::header_model::Header;
use crate::header_model::record_name::RecordName;
use crate::header_model::value::Value;
use crate::world_coordinates::internals::to_world;

use crate::internals::tolerance::{Tolerance, assert_close};
use crate::internals::wcs_header::{Family, WcsHeader};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::internals::CEA_GOLDEN;
use crate::world_coordinates::internals::CROTA_GOLDEN;
use crate::world_coordinates::internals::TAN_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;
use crate::world_coordinates::linear_transform::internals as linear;

/// `wcs_tan.fits`: 1″ pixels rotated by 15°, the WCS [`TAN_GOLDEN`] samples.
const TAN_FIXTURE: WcsHeader<'_> = WcsHeader {
    degrees: true,
    pc: Some([
        0.965_925_826_289_07,
        -0.258_819_045_102_52,
        0.258_819_045_102_52,
        0.965_925_826_289_07,
    ]),
    poles: Some([180.0, 2.5]),
    ..WcsHeader::celestial(
        "TAN",
        [256.0, 256.0],
        [150.0, 2.5],
        [-0.000_277_777_8, 0.000_277_777_8],
    )
};

/// A slightly rotated TAN WCS each table family must transform like its image.
const ROTATED_TAN: WcsHeader<'_> = WcsHeader {
    pc: Some([1.0, -0.05, 0.05, 1.0]),
    ..WcsHeader::celestial("TAN", [256.0, 256.0], [150.0, 30.0], [-1e-3, 1e-3])
};

/// The CEA WCS [`CEA_GOLDEN`] samples, without its λ = 0.5.
const CEA: WcsHeader<'_> = WcsHeader::celestial("CEA", [50.0, 50.0], [45.0, 30.0], [-0.05, 0.05]);

/// The legacy-rotation WCS [`CROTA_GOLDEN`] samples.
const CROTA: WcsHeader<'_> = WcsHeader {
    degrees: true,
    crota: Some(25.0),
    ..WcsHeader::celestial("TAN", [128.0, 128.0], [83.6, 22.0], [-0.0005, 0.0005])
};

fn assert_same_world(table: &Wcs, image: &Wcs, pixels: &[[f64; 2]], context: &str) {
    for pixel in pixels {
        assert_close(
            &to_world(table, pixel).unwrap(),
            &to_world(image, pixel).unwrap(),
            Tolerance::abs(
                1e-12,
                "the table form parses its keywords into the same values",
            ),
            &format!("{context} at {pixel:?}"),
        );
    }
}

/// §8.5 and Table 22: a pixel-list WCS on columns 2, 3 and a vector-cell WCS in
/// column 5 transform exactly like the image WCS with the same keywords, in the
/// primary description and an alternate one.
#[test]
fn table_wcs_matches_the_equivalent_image_wcs() {
    let image = ROTATED_TAN.wcs(Family::Image, None);
    for family in [Family::PixelList, Family::VectorCell] {
        let context = format!("{family:?}");
        let table = ROTATED_TAN.wcs(family, None);
        assert_eq!(
            table.description().axes.len(),
            2,
            "{context}: rank from the keywords"
        );
        assert!(
            table.celestial.is_some(),
            "{context} pair must be celestial"
        );
        assert_same_world(
            &table,
            &image,
            &[[256.0, 256.0], [1.0, 1.0], [300.0, 100.0], [50.0, 400.0]],
            &context,
        );

        let alternate = TAN_FIXTURE.wcs(family, 'A');
        assert_eq!(
            alternate.celestial.as_ref().unwrap().rotation.pole,
            CelestialPole {
                ra: 150.0,
                dec: 2.5,
                lonpole: 180.0,
            }
        );
        assert_astropy_golden(&alternate, &TAN_GOLDEN, &format!("alternate {context}"));
    }

    let mut ranked = TAN_FIXTURE.header(Family::VectorCell, 'A');
    ranked.set_internal("WCAX5A", 2);
    let explicit = Wcs::from_array_column(&ranked, 5, 'A').unwrap();
    assert_astropy_golden(&explicit, &TAN_GOLDEN, "ranked alternate vector-cell TAN");

    let mut pixel_list = ROTATED_TAN.header(Family::PixelList, None);
    pixel_list.set_internal("TCRVL2", "not numeric");
    assert!(matches!(
        Wcs::from_pixel_list(&pixel_list, &[2, 3], None),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), .. }) if name == "TCRVL2"
    ));
    let mut vector = ROTATED_TAN.header(Family::VectorCell, None);
    vector.set_internal("1CRVL5", "not numeric");
    assert!(matches!(
        Wcs::from_array_column(&vector, 5, None),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), .. }) if name == "1CRVL5"
    ));
}

#[test]
fn table_wcs_parameter_aliases_match_astropy() {
    for (parameter, family, alt) in [
        ("TPV3_1", Family::PixelList, None),
        ("TV3_1", Family::PixelList, None),
        ("TPV3_1A", Family::PixelList, Some('A')),
        ("TV3_1A", Family::PixelList, Some('A')),
        ("2PV5_1", Family::VectorCell, None),
        ("2V5_1", Family::VectorCell, None),
        ("2PV5_1A", Family::VectorCell, Some('A')),
        ("2V5_1A", Family::VectorCell, Some('A')),
    ] {
        let mut header = CEA.header(family, alt);
        header.set_internal(parameter, 0.5);
        let wcs = match family {
            Family::PixelList => Wcs::from_pixel_list(&header, &[2, 3], alt),
            _ => Wcs::from_array_column(&header, 5, alt),
        }
        .unwrap();
        assert_astropy_golden(&wcs, &CEA_GOLDEN, parameter);
    }
}

/// TPV takes `PVi_m` up to m = 39, so a table WCS translates every index: the degree-5
/// term `x⁵` is m = 17 and the degree-7 term `xy⁶` is m = 35, past the 0 to 20 a
/// projection takes, and each moves the point as in the image form.
#[test]
fn table_wcs_keeps_every_tpv_term() {
    let tpv = WcsHeader::celestial("TPV", [100.0, 100.0], [150.0, 2.5], [-0.01, 0.01]);
    let pixel = [30.0, 170.0];
    let mut image = tpv.image_header();
    let mut table = tpv.header(Family::PixelList, None);
    let plain = to_world(
        &Wcs::from_pixel_list(&table, &[2, 3], None).unwrap(),
        &pixel,
    )
    .unwrap();
    for m in [17, 35] {
        image.set_internal(format!("PV1_{m}"), 1e-3);
        table.set_internal(format!("TPV2_{m}"), 1e-3);
    }
    let from_image = to_world(&Wcs::from_header(&image, None).unwrap(), &pixel).unwrap();
    let from_table = to_world(
        &Wcs::from_pixel_list(&table, &[2, 3], None).unwrap(),
        &pixel,
    )
    .unwrap();
    assert_eq!(from_table, from_image);
    assert_ne!(from_table, plain);
}

#[test]
fn table_wcs_matrix_aliases_resolve_exactly() {
    let expected = [2.0, 0.5, -0.25, 3.0];
    for (root, alternate) in [
        ("TPC", false),
        ("TP", false),
        ("TCD", false),
        ("TC", false),
        ("TPC", true),
        ("TP", true),
        ("TCD", true),
        ("TC", true),
    ] {
        let suffix = if alternate { "A" } else { "" };
        let mut header = Header::new();
        if alternate {
            header
                .set_internal("TCTY2A", "LINEAR")
                .set_internal("TCTY3A", "LINEAR");
        } else {
            header
                .set_internal("TCTYP2", "LINEAR")
                .set_internal("TCTYP3", "LINEAR");
        }
        header
            .set_internal(format!("{root}2_2{suffix}"), expected[0])
            .set_internal(format!("{root}2_3{suffix}"), expected[1])
            .set_internal(format!("{root}3_2{suffix}"), expected[2])
            .set_internal(format!("{root}3_3{suffix}"), expected[3]);
        let wcs = Wcs::from_pixel_list(&header, &[2, 3], alternate.then_some('A')).unwrap();
        assert_eq!(
            linear::matrix(&wcs.linear),
            expected,
            "{root}, alternate={alternate}"
        );
    }

    for (root, alternate) in [("PC", false), ("CD", false), ("PC", true), ("CD", true)] {
        let suffix = if alternate { "A" } else { "" };
        let mut header = Header::new();
        header.set_internal(format!("WCAX5{suffix}"), 2);
        header
            .set_internal(format!("11{root}5{suffix}"), expected[0])
            .set_internal(format!("12{root}5{suffix}"), expected[1])
            .set_internal(format!("21{root}5{suffix}"), expected[2])
            .set_internal(format!("22{root}5{suffix}"), expected[3]);
        let wcs = Wcs::from_array_column(&header, 5, alternate.then_some('A')).unwrap();
        assert_eq!(
            linear::matrix(&wcs.linear),
            expected,
            "{root}, alternate={alternate}"
        );
    }
}

#[test]
fn primary_table_wcs_rotation_matches_astropy() {
    for family in [Family::PixelList, Family::VectorCell] {
        let wcs = CROTA.wcs(family, None);
        assert_astropy_golden(&wcs, &CROTA_GOLDEN, &format!("primary {family:?} CROTA"));
    }
}

#[test]
fn table_wcs_column_poles_match_the_equivalent_image_wcs() {
    let description = WcsHeader {
        pv: &[0.0, 0.5],
        poles: Some([0.0, -90.0]),
        ..CEA
    };
    let image = description.wcs(Family::Image, None);
    let image_pole = image.celestial.as_ref().unwrap().rotation.pole;
    assert_eq!(image_pole.ra, 45.0);
    assert!((image_pole.dec + 60.0).abs() < 1e-12, "{image_pole:?}");
    assert_eq!(image_pole.lonpole, 0.0);

    for family in [Family::PixelList, Family::VectorCell] {
        let table = description.wcs(family, 'A');
        assert_eq!(table.celestial.as_ref().unwrap().rotation.pole, image_pole);
        assert_same_world(
            &table,
            &image,
            &[[50.0, 50.0], [20.0, 70.0], [80.0, 30.0]],
            &format!("{family:?}"),
        );
    }
}

#[test]
fn vector_cell_rank_uses_every_supported_keyword_family() {
    let build = |keyword: &str, value: Value| {
        let mut h = Header::new();
        h.set_internal(keyword, value);
        h
    };
    let mut cd = Header::new();
    cd.set_internal("11CD5", 1.0)
        .set_internal("22CD5", 1.0)
        .set_internal("33CD5", 1.0);
    let cases = [
        build("3CTYP5", Value::Text("LINEAR".to_string())),
        build("3CUNI5", Value::Text("m".to_string())),
        build("3CRPX5", Value::Real(10.0)),
        build("3CRVL5", Value::Real(10.0)),
        build("3CDLT5", Value::Real(2.0)),
        build("3CROT5", Value::Real(10.0)),
        build("3PV5_1", Value::Real(2.0)),
        build("3V5_1", Value::Real(2.0)),
        build("3PS5_1", Value::Text("value".to_string())),
        build("3S5_1", Value::Text("value".to_string())),
        build("13PC5", Value::Real(0.25)),
        cd,
    ];
    for h in &cases {
        assert_eq!(
            Wcs::from_array_column(h, 5, None)
                .unwrap()
                .description()
                .axes
                .len(),
            3
        );
    }

    let mut alternate_cd = Header::new();
    alternate_cd
        .set_internal("11CD5A", 1.0)
        .set_internal("22CD5A", 1.0)
        .set_internal("33CD5A", 1.0);
    let alternate_cases = [
        build("3CTY5A", Value::Text("LINEAR".to_string())),
        build("3CUN5A", Value::Text("m".to_string())),
        build("3CRP5A", Value::Real(10.0)),
        build("3CRV5A", Value::Real(10.0)),
        build("3CDE5A", Value::Real(2.0)),
        build("3PV5_1A", Value::Real(2.0)),
        build("3V5_1A", Value::Real(2.0)),
        build("3PS5_1A", Value::Text("value".to_string())),
        build("3S5_1A", Value::Text("value".to_string())),
        build("13PC5A", Value::Real(0.25)),
        alternate_cd,
    ];
    for header in &alternate_cases {
        assert_eq!(
            Wcs::from_array_column(header, 5, 'A')
                .unwrap()
                .description()
                .axes
                .len(),
            3
        );
    }

    let invalid_long_alternate = build("3CTYP5A", Value::Text("LINEAR".to_string()));
    assert!(matches!(
        Wcs::from_array_column(&invalid_long_alternate, 5, 'A'),
        Err(FitsError::MissingKeyword { name: RecordName::Keyword(keyword) }) if keyword == "1CTY5A"
    ));

    // A two-digit array axis: the whole leading digit run is the axis, where in
    // `13PC5` above the run is a *pair* of single-digit axes and the rank comes from
    // the second. Rank inference has to admit both readings of a leading run.
    let two_digit = build("12CTYP5", Value::Text("LINEAR".to_string()));
    assert_eq!(
        Wcs::from_array_column(&two_digit, 5, None)
            .unwrap()
            .description()
            .axes
            .len(),
        12
    );

    // A leading zero is not an index (§4.1.2 indices are unpadded), so `03CTYP5`
    // names no axis and the column has no vector WCS at all.
    let padded_index = build("03CTYP5", Value::Text("LINEAR".to_string()));
    assert!(matches!(
        Wcs::from_array_column(&padded_index, 5, None),
        Err(FitsError::MissingKeyword { name: RecordName::Keyword(keyword) }) if keyword == "1CTYP5"
    ));

    // A column number too wide for any keyword finds no card and errors, not panics.
    let column = usize::MAX;
    assert!(matches!(
        Wcs::from_array_column(&padded_index, column, 'A'),
        Err(FitsError::InvalidKeyword { name }) if name == format!("1CTY{column}A")
    ));
}
