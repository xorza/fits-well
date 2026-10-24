use crate::world_coordinates::table_wcs::table_wcs_resolver::*;

#[test]
fn table_wcs_resolver_matches_table_22() {
    let primary = TableWcsResolver::new(Alternate::PRIMARY);
    let alternate = TableWcsResolver::new(Alternate::new('A'));
    let axis_cases = [
        (
            TableAxisKeyword::Type,
            "TCTYP17",
            "TCTY17A",
            "3CTYP17",
            "3CTY17A",
        ),
        (
            TableAxisKeyword::Unit,
            "TCUNI17",
            "TCUN17A",
            "3CUNI17",
            "3CUN17A",
        ),
        (
            TableAxisKeyword::ReferenceValue,
            "TCRVL17",
            "TCRV17A",
            "3CRVL17",
            "3CRV17A",
        ),
        (
            TableAxisKeyword::Increment,
            "TCDLT17",
            "TCDE17A",
            "3CDLT17",
            "3CDE17A",
        ),
        (
            TableAxisKeyword::ReferencePoint,
            "TCRPX17",
            "TCRP17A",
            "3CRPX17",
            "3CRP17A",
        ),
    ];
    for (keyword, primary_pixel, alternate_pixel, primary_vector, alternate_vector) in axis_cases {
        assert_eq!(
            primary.pixel_axis_key(keyword, 17).unwrap().as_str(),
            primary_pixel
        );
        assert_eq!(
            alternate.pixel_axis_key(keyword, 17).unwrap().as_str(),
            alternate_pixel
        );
        assert_eq!(
            primary.vector_axis_key(keyword, 3, 17).unwrap().as_str(),
            primary_vector
        );
        assert_eq!(
            alternate.vector_axis_key(keyword, 3, 17).unwrap().as_str(),
            alternate_vector
        );
    }
    assert_eq!(
        primary
            .pixel_axis_key(TableAxisKeyword::Rotation, 17)
            .unwrap()
            .as_str(),
        "TCROT17"
    );
    assert_eq!(
        primary
            .vector_axis_key(TableAxisKeyword::Rotation, 3, 17)
            .unwrap()
            .as_str(),
        "3CROT17"
    );
    assert!(
        alternate
            .pixel_axis_key(TableAxisKeyword::Rotation, 17)
            .is_none()
    );
    assert!(
        alternate
            .vector_axis_key(TableAxisKeyword::Rotation, 3, 17)
            .is_none()
    );
    // A family that has a spelling but a column too wide for a keyword: `TCTYP1000`
    // and `3CTY1000A` are 9 bytes, so no card holds them and a read finds nothing,
    // while the one error that names the type keyword still reports the name.
    assert!(
        primary
            .pixel_axis_key(TableAxisKeyword::Type, 1000)
            .is_none()
    );
    assert!(
        alternate
            .vector_axis_key(TableAxisKeyword::Type, 3, 1000)
            .is_none()
    );
    assert_eq!(primary.first_vector_type(17).unwrap().as_str(), "1CTYP17");
    assert_eq!(alternate.first_vector_type(17).unwrap().as_str(), "1CTY17A");
    assert!(matches!(
        primary.first_vector_type(1000),
        Err(FitsError::InvalidKeyword { name }) if name == "1CTYP1000"
    ));

    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Pc, 2, 3, KeywordSpelling::Full)
            .unwrap()
            .as_str(),
        "TPC2_3A"
    );
    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Pc, 2, 3, KeywordSpelling::Short)
            .unwrap()
            .as_str(),
        "TP2_3A"
    );
    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Cd, 2, 3, KeywordSpelling::Full)
            .unwrap()
            .as_str(),
        "TCD2_3A"
    );
    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Cd, 2, 3, KeywordSpelling::Short)
            .unwrap()
            .as_str(),
        "TC2_3A"
    );
    assert_eq!(
        alternate
            .vector_matrix_key(TableMatrixKeyword::Pc, 2, 3, 17)
            .unwrap()
            .as_str(),
        "23PC17A"
    );
    assert_eq!(
        alternate
            .pixel_parameter_key(2, 1, KeywordSpelling::Full)
            .unwrap()
            .as_str(),
        "TPV2_1A"
    );
    assert_eq!(
        alternate
            .pixel_parameter_key(2, 1, KeywordSpelling::Short)
            .unwrap()
            .as_str(),
        "TV2_1A"
    );
    assert_eq!(
        alternate
            .vector_parameter_key(2, 17, 1, KeywordSpelling::Full)
            .unwrap()
            .as_str(),
        "2PV17_1A"
    );
    assert_eq!(
        alternate
            .vector_parameter_key(2, 17, 1, KeywordSpelling::Short)
            .unwrap()
            .as_str(),
        "2V17_1A"
    );
    assert_eq!(
        alternate.column_key("LONP", 17).unwrap().as_str(),
        "LONP17A"
    );
    assert_eq!(
        alternate.column_key("LATP", 17).unwrap().as_str(),
        "LATP17A"
    );
    assert_eq!(
        alternate.column_key("WCAX", 17).unwrap().as_str(),
        "WCAX17A"
    );
}
