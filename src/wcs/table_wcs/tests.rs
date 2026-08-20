use crate::error::FitsError;
use crate::header::Header;
use crate::header::value::Value;
use crate::wcs::Wcs;
use crate::wcs::celestial_pole::CelestialPole;
use crate::wcs::internals::CEA_GOLDEN;
use crate::wcs::internals::CROTA_GOLDEN;
use crate::wcs::internals::TAN_GOLDEN;
use crate::wcs::internals::assert_astropy_golden;
use crate::wcs::linear_transform::internals as linear;

#[test]
fn pixel_list_wcs_matches_the_equivalent_image_wcs() {
    // §8.5: a pixel-list (event) WCS on columns 2,3 must transform identically to
    // an image WCS with the same CTYPE/CRPIX/CRVAL/CDELT and PC rotation.
    let mut tab = Header::new();
    tab.set_internal("TCTYP2", "RA---TAN")
        .set_internal("TCTYP3", "DEC--TAN");
    tab.set_internal("TCRPX2", 256.0)
        .set_internal("TCRPX3", 256.0);
    tab.set_internal("TCRVL2", 150.0)
        .set_internal("TCRVL3", 30.0);
    tab.set_internal("TCDLT2", -1e-3)
        .set_internal("TCDLT3", 1e-3);
    tab.set_internal("TPC2_2", 1.0)
        .set_internal("TPC2_3", -0.05);
    tab.set_internal("TPC3_2", 0.05).set_internal("TPC3_3", 1.0);
    let wt = Wcs::from_pixel_list(&tab, &[2, 3], None).unwrap();

    let mut img = Header::new();
    img.set_internal("NAXIS", 2);
    img.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    img.set_internal("CRPIX1", 256.0)
        .set_internal("CRPIX2", 256.0);
    img.set_internal("CRVAL1", 150.0)
        .set_internal("CRVAL2", 30.0);
    img.set_internal("CDELT1", -1e-3)
        .set_internal("CDELT2", 1e-3);
    img.set_internal("PC1_1", 1.0).set_internal("PC1_2", -0.05);
    img.set_internal("PC2_1", 0.05).set_internal("PC2_2", 1.0);
    let wi = Wcs::from_header(&img, None).unwrap();

    assert!(wt.celestial.is_some(), "pixel-list pair must be celestial");
    for &(px, py) in &[(256.0, 256.0), (1.0, 1.0), (300.0, 100.0), (50.0, 400.0)] {
        let a = wt.pixel_to_world(&[px, py]).unwrap();
        let b = wi.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "pixel-list {a:?} vs image {b:?} at ({px},{py})"
        );
    }

    let mut alternate = Header::new();
    alternate
        .set_internal("TCTY2A", "RA---TAN")
        .set_internal("TCTY3A", "DEC--TAN");
    alternate
        .set_internal("TCRP2A", 256.0)
        .set_internal("TCRP3A", 256.0);
    alternate
        .set_internal("TCRV2A", 150.0)
        .set_internal("TCRV3A", 2.5);
    alternate
        .set_internal("TCDE2A", -0.0002777778)
        .set_internal("TCDE3A", 0.0002777778);
    alternate
        .set_internal("TCUN2A", "deg")
        .set_internal("TCUN3A", "deg");
    alternate
        .set_internal("TP2_2A", 0.96592582628907)
        .set_internal("TP2_3A", -0.25881904510252)
        .set_internal("TP3_2A", 0.25881904510252)
        .set_internal("TP3_3A", 0.96592582628907);
    alternate
        .set_internal("LONP2A", 180.0)
        .set_internal("LATP2A", 2.5);
    let alternate_wcs = Wcs::from_pixel_list(&alternate, &[2, 3], Some('A')).unwrap();
    assert_eq!(
        alternate_wcs.celestial.as_ref().unwrap().pole,
        CelestialPole {
            ra: 150.0,
            dec: 2.5,
            lonpole: 180.0,
        }
    );
    assert_astropy_golden(&alternate_wcs, TAN_GOLDEN, "alternate pixel-list TAN");

    tab.set_internal("TCRVL2", "not numeric");
    assert!(matches!(
        Wcs::from_pixel_list(&tab, &[2, 3], None),
        Err(FitsError::TypeMismatch { name, .. }) if name == "TCRVL2"
    ));
}

#[test]
fn vector_cell_wcs_matches_the_equivalent_image_wcs() {
    // §8 Table 22: an image in a binary-table vector cell (here column 5) uses the
    // axis+column-indexed keyword family (`iCTYPn`, `ijPCn`, …, with leading-digit
    // keyword names); it must transform exactly like the equivalent image WCS.
    let mut tab = Header::new();
    tab.set_internal("1CTYP5", "RA---TAN")
        .set_internal("2CTYP5", "DEC--TAN");
    tab.set_internal("1CRPX5", 256.0)
        .set_internal("2CRPX5", 256.0);
    tab.set_internal("1CRVL5", 150.0)
        .set_internal("2CRVL5", 30.0);
    tab.set_internal("1CDLT5", -1e-3)
        .set_internal("2CDLT5", 1e-3);
    tab.set_internal("11PC5", 1.0).set_internal("12PC5", -0.05);
    tab.set_internal("21PC5", 0.05).set_internal("22PC5", 1.0);
    let wt = Wcs::from_array_column(&tab, 5, None).unwrap();

    let mut img = Header::new();
    img.set_internal("NAXIS", 2);
    img.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    img.set_internal("CRPIX1", 256.0)
        .set_internal("CRPIX2", 256.0);
    img.set_internal("CRVAL1", 150.0)
        .set_internal("CRVAL2", 30.0);
    img.set_internal("CDELT1", -1e-3)
        .set_internal("CDELT2", 1e-3);
    img.set_internal("PC1_1", 1.0).set_internal("PC1_2", -0.05);
    img.set_internal("PC2_1", 0.05).set_internal("PC2_2", 1.0);
    let wi = Wcs::from_header(&img, None).unwrap();

    assert_eq!(wt.view().axes.len(), 2); // rank inferred from the iCTYP5 keywords
    assert!(wt.celestial.is_some(), "vector-cell pair must be celestial");
    for &(px, py) in &[(256.0, 256.0), (1.0, 1.0), (300.0, 100.0), (50.0, 400.0)] {
        let a = wt.pixel_to_world(&[px, py]).unwrap();
        let b = wi.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "vector-cell {a:?} vs image {b:?} at ({px},{py})"
        );
    }

    let mut alternate = Header::new();
    alternate
        .set_internal("1CTY5A", "RA---TAN")
        .set_internal("2CTY5A", "DEC--TAN");
    alternate
        .set_internal("1CRP5A", 256.0)
        .set_internal("2CRP5A", 256.0);
    alternate
        .set_internal("1CRV5A", 150.0)
        .set_internal("2CRV5A", 2.5);
    alternate
        .set_internal("1CDE5A", -0.0002777778)
        .set_internal("2CDE5A", 0.0002777778);
    alternate
        .set_internal("1CUN5A", "deg")
        .set_internal("2CUN5A", "deg");
    alternate
        .set_internal("11PC5A", 0.96592582628907)
        .set_internal("12PC5A", -0.25881904510252)
        .set_internal("21PC5A", 0.25881904510252)
        .set_internal("22PC5A", 0.96592582628907);
    alternate
        .set_internal("LONP5A", 180.0)
        .set_internal("LATP5A", 2.5);
    let inferred = Wcs::from_array_column(&alternate, 5, Some('A')).unwrap();
    assert_eq!(inferred.view().axes.len(), 2);
    assert_eq!(
        inferred.celestial.as_ref().unwrap().pole,
        CelestialPole {
            ra: 150.0,
            dec: 2.5,
            lonpole: 180.0,
        }
    );
    assert_astropy_golden(&inferred, TAN_GOLDEN, "alternate vector-cell TAN");

    alternate.set_internal("WCAX5A", 2);
    let explicit = Wcs::from_array_column(&alternate, 5, Some('A')).unwrap();
    assert_astropy_golden(&explicit, TAN_GOLDEN, "ranked alternate vector-cell TAN");

    tab.set_internal("1CRVL5", "not numeric");
    assert!(matches!(
        Wcs::from_array_column(&tab, 5, None),
        Err(FitsError::TypeMismatch { name, .. }) if name == "1CRVL5"
    ));
}

#[test]
fn table_wcs_parameter_aliases_match_astropy() {
    let pixel = |parameter: &str, alternate: bool| {
        let mut header = Header::new();
        if alternate {
            header
                .set_internal("TCTY2A", "RA---CEA")
                .set_internal("TCTY3A", "DEC--CEA");
            header
                .set_internal("TCRP2A", 50.0)
                .set_internal("TCRP3A", 50.0);
            header
                .set_internal("TCRV2A", 45.0)
                .set_internal("TCRV3A", 30.0);
            header
                .set_internal("TCDE2A", -0.05)
                .set_internal("TCDE3A", 0.05);
        } else {
            header
                .set_internal("TCTYP2", "RA---CEA")
                .set_internal("TCTYP3", "DEC--CEA");
            header
                .set_internal("TCRPX2", 50.0)
                .set_internal("TCRPX3", 50.0);
            header
                .set_internal("TCRVL2", 45.0)
                .set_internal("TCRVL3", 30.0);
            header
                .set_internal("TCDLT2", -0.05)
                .set_internal("TCDLT3", 0.05);
        }
        header.set_internal(parameter, 0.5);
        Wcs::from_pixel_list(&header, &[2, 3], alternate.then_some('A')).unwrap()
    };
    for parameter in ["TPV3_1", "TV3_1"] {
        let wcs = pixel(parameter, false);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }
    for parameter in ["TPV3_1A", "TV3_1A"] {
        let wcs = pixel(parameter, true);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }

    let vector = |parameter: &str, alternate: bool| {
        let mut header = Header::new();
        if alternate {
            header.set_internal("WCAX5A", 2);
            header
                .set_internal("1CTY5A", "RA---CEA")
                .set_internal("2CTY5A", "DEC--CEA");
            header
                .set_internal("1CRP5A", 50.0)
                .set_internal("2CRP5A", 50.0);
            header
                .set_internal("1CRV5A", 45.0)
                .set_internal("2CRV5A", 30.0);
            header
                .set_internal("1CDE5A", -0.05)
                .set_internal("2CDE5A", 0.05);
        } else {
            header.set_internal("WCAX5", 2);
            header
                .set_internal("1CTYP5", "RA---CEA")
                .set_internal("2CTYP5", "DEC--CEA");
            header
                .set_internal("1CRPX5", 50.0)
                .set_internal("2CRPX5", 50.0);
            header
                .set_internal("1CRVL5", 45.0)
                .set_internal("2CRVL5", 30.0);
            header
                .set_internal("1CDLT5", -0.05)
                .set_internal("2CDLT5", 0.05);
        }
        header.set_internal(parameter, 0.5);
        Wcs::from_array_column(&header, 5, alternate.then_some('A')).unwrap()
    };
    for parameter in ["2PV5_1", "2V5_1"] {
        let wcs = vector(parameter, false);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }
    for parameter in ["2PV5_1A", "2V5_1A"] {
        let wcs = vector(parameter, true);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
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
    let mut pixel = Header::new();
    pixel
        .set_internal("TCTYP2", "RA---TAN")
        .set_internal("TCTYP3", "DEC--TAN");
    pixel
        .set_internal("TCUNI2", "deg")
        .set_internal("TCUNI3", "deg");
    pixel
        .set_internal("TCRPX2", 128.0)
        .set_internal("TCRPX3", 128.0);
    pixel
        .set_internal("TCRVL2", 83.6)
        .set_internal("TCRVL3", 22.0);
    pixel
        .set_internal("TCDLT2", -0.0005)
        .set_internal("TCDLT3", 0.0005);
    pixel.set_internal("TCROT3", 25.0);
    let pixel_wcs = Wcs::from_pixel_list(&pixel, &[2, 3], None).unwrap();
    assert_astropy_golden(&pixel_wcs, CROTA_GOLDEN, "primary pixel-list CROTA");

    let mut vector = Header::new();
    vector.set_internal("WCAX5", 2);
    vector
        .set_internal("1CTYP5", "RA---TAN")
        .set_internal("2CTYP5", "DEC--TAN");
    vector
        .set_internal("1CUNI5", "deg")
        .set_internal("2CUNI5", "deg");
    vector
        .set_internal("1CRPX5", 128.0)
        .set_internal("2CRPX5", 128.0);
    vector
        .set_internal("1CRVL5", 83.6)
        .set_internal("2CRVL5", 22.0);
    vector
        .set_internal("1CDLT5", -0.0005)
        .set_internal("2CDLT5", 0.0005);
    vector.set_internal("2CROT5", 25.0);
    let vector_wcs = Wcs::from_array_column(&vector, 5, None).unwrap();
    assert_astropy_golden(&vector_wcs, CROTA_GOLDEN, "primary vector-cell CROTA");
}

#[test]
fn table_wcs_column_poles_match_the_equivalent_image_wcs() {
    let mut image = Header::new();
    image.set_internal("NAXIS", 2);
    image
        .set_internal("CTYPE1", "RA---CEA")
        .set_internal("CTYPE2", "DEC--CEA");
    image
        .set_internal("CRPIX1", 50.0)
        .set_internal("CRPIX2", 50.0);
    image
        .set_internal("CRVAL1", 45.0)
        .set_internal("CRVAL2", 30.0);
    image
        .set_internal("CDELT1", -0.05)
        .set_internal("CDELT2", 0.05);
    image.set_internal("PV2_1", 0.5);
    image
        .set_internal("LONPOLE", 0.0)
        .set_internal("LATPOLE", -90.0);
    let image_wcs = Wcs::from_header(&image, None).unwrap();
    let image_pole = image_wcs.celestial.as_ref().unwrap().pole;
    assert_eq!(image_pole.ra, 45.0);
    assert!((image_pole.dec + 60.0).abs() < 1e-12, "{image_pole:?}");
    assert_eq!(image_pole.lonpole, 0.0);

    let mut pixel = Header::new();
    pixel
        .set_internal("TCTY2A", "RA---CEA")
        .set_internal("TCTY3A", "DEC--CEA");
    pixel
        .set_internal("TCRP2A", 50.0)
        .set_internal("TCRP3A", 50.0);
    pixel
        .set_internal("TCRV2A", 45.0)
        .set_internal("TCRV3A", 30.0);
    pixel
        .set_internal("TCDE2A", -0.05)
        .set_internal("TCDE3A", 0.05);
    pixel.set_internal("TV3_1A", 0.5);
    pixel
        .set_internal("LONP2A", 0.0)
        .set_internal("LATP2A", -90.0);
    let pixel_wcs = Wcs::from_pixel_list(&pixel, &[2, 3], Some('A')).unwrap();

    let mut vector = Header::new();
    vector.set_internal("WCAX5A", 2);
    vector
        .set_internal("1CTY5A", "RA---CEA")
        .set_internal("2CTY5A", "DEC--CEA");
    vector
        .set_internal("1CRP5A", 50.0)
        .set_internal("2CRP5A", 50.0);
    vector
        .set_internal("1CRV5A", 45.0)
        .set_internal("2CRV5A", 30.0);
    vector
        .set_internal("1CDE5A", -0.05)
        .set_internal("2CDE5A", 0.05);
    vector.set_internal("2V5_1A", 0.5);
    vector
        .set_internal("LONP5A", 0.0)
        .set_internal("LATP5A", -90.0);
    let vector_wcs = Wcs::from_array_column(&vector, 5, Some('A')).unwrap();

    for table_wcs in [&pixel_wcs, &vector_wcs] {
        assert_eq!(
            table_wcs.celestial.as_ref().unwrap().pole,
            image_wcs.celestial.as_ref().unwrap().pole
        );
        for pixel in [[50.0, 50.0], [20.0, 70.0], [80.0, 30.0]] {
            let table_world = table_wcs.pixel_to_world(&pixel).unwrap();
            let image_world = image_wcs.pixel_to_world(&pixel).unwrap();
            assert!(
                (table_world[0] - image_world[0]).abs() < 1e-12
                    && (table_world[1] - image_world[1]).abs() < 1e-12,
                "table {table_world:?} vs image {image_world:?} at {pixel:?}"
            );
        }
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
