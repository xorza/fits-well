use crate::error::FitsError;
use crate::header_model::Header;
use crate::header_model::value::Value;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::internals::CEA_GOLDEN;
use crate::world_coordinates::internals::CROTA_GOLDEN;
use crate::world_coordinates::internals::TAN_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;
use crate::world_coordinates::linear_transform::internals as linear;

/// The keyword family a WCS is written in (§8, Table 22): image axes 1 and 2,
/// pixel-list columns 2 and 3, or the axes of vector column 5.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Family {
    Image,
    PixelList,
    VectorCell,
}

/// A two-axis celestial WCS, written into any [`Family`] by [`Celestial::header`].
#[derive(Debug, Clone, Copy)]
struct Celestial {
    projection: &'static str,
    crpix: [f64; 2],
    crval: [f64; 2],
    cdelt: [f64; 2],
    degrees: bool,
    /// Row-major.
    pc: Option<[f64; 4]>,
    /// On the latitude axis; there is no alternate form.
    crota: Option<f64>,
    /// `PV2_1`, on the latitude axis.
    pv: Option<f64>,
    /// `LONPOLE` and `LATPOLE`.
    poles: Option<[f64; 2]>,
}

impl Celestial {
    const fn new(
        projection: &'static str,
        crpix: [f64; 2],
        crval: [f64; 2],
        cdelt: [f64; 2],
    ) -> Celestial {
        Celestial {
            projection,
            crpix,
            crval,
            cdelt,
            degrees: false,
            pc: None,
            crota: None,
            pv: None,
            poles: None,
        }
    }

    fn header(&self, family: Family, alt: Option<char>) -> Header {
        let a = alt.map(String::from).unwrap_or_default();
        let primary = alt.is_none();
        // Both axes' keyword for an image root, a long table root, and the short
        // table root an alternate description takes (Table 22).
        let names = |image_root: &str, long: &str, short: &str| {
            [1, 2].map(|i| match family {
                Family::Image => format!("{image_root}{i}{a}"),
                Family::PixelList if primary => format!("T{long}{}", i + 1),
                Family::PixelList => format!("T{short}{}{a}", i + 1),
                Family::VectorCell if primary => format!("{i}{long}5"),
                Family::VectorCell => format!("{i}{short}5{a}"),
            })
        };
        let matrix = |i: usize, j: usize| match family {
            Family::Image => format!("PC{i}_{j}{a}"),
            Family::PixelList if primary => format!("TPC{}_{}", i + 1, j + 1),
            Family::PixelList => format!("TP{}_{}{a}", i + 1, j + 1),
            Family::VectorCell => format!("{i}{j}PC5{a}"),
        };
        let pole = |name: &str| match family {
            Family::Image => format!("{name}{a}"),
            Family::PixelList => format!("{}P2{a}", &name[..3]),
            Family::VectorCell => format!("{}P5{a}", &name[..3]),
        };

        let mut header = Header::new();
        if family == Family::Image {
            header.set_internal("NAXIS", 2);
        }
        let ctype = names("CTYPE", "CTYP", "CTY");
        header
            .set_internal(&ctype[0], format!("RA---{}", self.projection))
            .set_internal(&ctype[1], format!("DEC--{}", self.projection));
        for (keys, values) in [
            (names("CRPIX", "CRPX", "CRP"), self.crpix),
            (names("CRVAL", "CRVL", "CRV"), self.crval),
            (names("CDELT", "CDLT", "CDE"), self.cdelt),
        ] {
            header
                .set_internal(&keys[0], values[0])
                .set_internal(&keys[1], values[1]);
        }
        if self.degrees {
            let cunit = names("CUNIT", "CUNI", "CUN");
            header
                .set_internal(&cunit[0], "deg")
                .set_internal(&cunit[1], "deg");
        }
        if let Some(pc) = self.pc {
            for (index, value) in pc.into_iter().enumerate() {
                header.set_internal(&matrix(index / 2 + 1, index % 2 + 1), value);
            }
        }
        if let Some(crota) = self.crota {
            assert!(primary, "CROTAi has no alternate form");
            header.set_internal(&names("CROTA", "CROT", "CROT")[1], crota);
        }
        if let Some(pv) = self.pv {
            let key = match family {
                Family::Image => format!("PV2_1{a}"),
                Family::PixelList if primary => "TPV3_1".to_string(),
                Family::PixelList => format!("TV3_1{a}"),
                Family::VectorCell if primary => "2PV5_1".to_string(),
                Family::VectorCell => format!("2V5_1{a}"),
            };
            header.set_internal(&key, pv);
        }
        if let Some([lonpole, latpole]) = self.poles {
            header
                .set_internal(&pole("LONPOLE"), lonpole)
                .set_internal(&pole("LATPOLE"), latpole);
        }
        header
    }

    fn wcs(&self, family: Family, alt: Option<char>) -> Wcs {
        let header = self.header(family, alt);
        match family {
            Family::Image => Wcs::from_header(&header, alt),
            Family::PixelList => Wcs::from_pixel_list(&header, &[2, 3], alt),
            Family::VectorCell => Wcs::from_array_column(&header, 5, alt),
        }
        .unwrap()
    }
}

/// `wcs_tan.fits`: 1″ pixels rotated by 15°, the WCS [`TAN_GOLDEN`] samples.
const TAN_FIXTURE: Celestial = Celestial {
    degrees: true,
    pc: Some([
        0.965_925_826_289_07,
        -0.258_819_045_102_52,
        0.258_819_045_102_52,
        0.965_925_826_289_07,
    ]),
    poles: Some([180.0, 2.5]),
    ..Celestial::new(
        "TAN",
        [256.0, 256.0],
        [150.0, 2.5],
        [-0.000_277_777_8, 0.000_277_777_8],
    )
};

/// A slightly rotated TAN WCS each table family must transform like its image.
const ROTATED_TAN: Celestial = Celestial {
    pc: Some([1.0, -0.05, 0.05, 1.0]),
    ..Celestial::new("TAN", [256.0, 256.0], [150.0, 30.0], [-1e-3, 1e-3])
};

/// The CEA WCS [`CEA_GOLDEN`] samples, without its λ = 0.5.
const CEA: Celestial = Celestial::new("CEA", [50.0, 50.0], [45.0, 30.0], [-0.05, 0.05]);

/// The legacy-rotation WCS [`CROTA_GOLDEN`] samples.
const CROTA: Celestial = Celestial {
    degrees: true,
    crota: Some(25.0),
    ..Celestial::new("TAN", [128.0, 128.0], [83.6, 22.0], [-0.0005, 0.0005])
};

fn assert_same_transform(table: &Wcs, image: &Wcs, pixels: &[[f64; 2]], context: &str) {
    for pixel in pixels {
        let a = table.pixel_to_world(pixel).unwrap();
        let b = image.pixel_to_world(pixel).unwrap();
        // One evaluation path after parsing: the same operations on the same values.
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "{context} {a:?} vs image {b:?} at {pixel:?}"
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
            table.view().axes.len(),
            2,
            "{context}: rank from the keywords"
        );
        assert!(
            table.celestial.is_some(),
            "{context} pair must be celestial"
        );
        assert_same_transform(
            &table,
            &image,
            &[[256.0, 256.0], [1.0, 1.0], [300.0, 100.0], [50.0, 400.0]],
            &context,
        );

        let alternate = TAN_FIXTURE.wcs(family, Some('A'));
        assert_eq!(
            alternate.celestial.as_ref().unwrap().pole,
            CelestialPole {
                ra: 150.0,
                dec: 2.5,
                lonpole: 180.0,
            }
        );
        assert_astropy_golden(&alternate, &TAN_GOLDEN, &format!("alternate {context}"));
    }

    let mut ranked = TAN_FIXTURE.header(Family::VectorCell, Some('A'));
    ranked.set_internal("WCAX5A", 2);
    let explicit = Wcs::from_array_column(&ranked, 5, Some('A')).unwrap();
    assert_astropy_golden(&explicit, &TAN_GOLDEN, "ranked alternate vector-cell TAN");

    let mut pixel_list = ROTATED_TAN.header(Family::PixelList, None);
    pixel_list.set_internal("TCRVL2", "not numeric");
    assert!(matches!(
        Wcs::from_pixel_list(&pixel_list, &[2, 3], None),
        Err(FitsError::TypeMismatch { name, .. }) if name == "TCRVL2"
    ));
    let mut vector = ROTATED_TAN.header(Family::VectorCell, None);
    vector.set_internal("1CRVL5", "not numeric");
    assert!(matches!(
        Wcs::from_array_column(&vector, 5, None),
        Err(FitsError::TypeMismatch { name, .. }) if name == "1CRVL5"
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
            .set_internal(&format!("{root}2_2{suffix}"), expected[0])
            .set_internal(&format!("{root}2_3{suffix}"), expected[1])
            .set_internal(&format!("{root}3_2{suffix}"), expected[2])
            .set_internal(&format!("{root}3_3{suffix}"), expected[3]);
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
        header.set_internal(&format!("WCAX5{suffix}"), 2);
        header
            .set_internal(&format!("11{root}5{suffix}"), expected[0])
            .set_internal(&format!("12{root}5{suffix}"), expected[1])
            .set_internal(&format!("21{root}5{suffix}"), expected[2])
            .set_internal(&format!("22{root}5{suffix}"), expected[3]);
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
    let description = Celestial {
        pv: Some(0.5),
        poles: Some([0.0, -90.0]),
        ..CEA
    };
    let image = description.wcs(Family::Image, None);
    let image_pole = image.celestial.as_ref().unwrap().pole;
    assert_eq!(image_pole.ra, 45.0);
    assert!((image_pole.dec + 60.0).abs() < 1e-12, "{image_pole:?}");
    assert_eq!(image_pole.lonpole, 0.0);

    for family in [Family::PixelList, Family::VectorCell] {
        let table = description.wcs(family, Some('A'));
        assert_eq!(table.celestial.as_ref().unwrap().pole, image_pole);
        assert_same_transform(
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
                .view()
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
            Wcs::from_array_column(header, 5, Some('A'))
                .unwrap()
                .view()
                .axes
                .len(),
            3
        );
    }

    let invalid_long_alternate = build("3CTYP5A", Value::Text("LINEAR".to_string()));
    assert!(matches!(
        Wcs::from_array_column(&invalid_long_alternate, 5, Some('A')),
        Err(FitsError::MissingKeyword { name: "iCTYPn" })
    ));

    // A two-digit array axis: the whole leading digit run is the axis, where in
    // `13PC5` above the run is a *pair* of single-digit axes and the rank comes from
    // the second. Rank inference has to admit both readings of a leading run.
    let two_digit = build("12CTYP5", Value::Text("LINEAR".to_string()));
    assert_eq!(
        Wcs::from_array_column(&two_digit, 5, None)
            .unwrap()
            .view()
            .axes
            .len(),
        12
    );

    // A leading zero is not an index (§4.1.2 indices are unpadded), so `03CTYP5`
    // names no axis and the column has no vector WCS at all.
    let padded_index = build("03CTYP5", Value::Text("LINEAR".to_string()));
    assert!(matches!(
        Wcs::from_array_column(&padded_index, 5, None),
        Err(FitsError::MissingKeyword { name: "iCTYPn" })
    ));
}
