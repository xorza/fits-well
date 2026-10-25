use crate::world_coordinates::table_wcs::table_wcs_resolver::*;
use std::assert_matches;

/// The axis keys of Table 22, and the type keyword a vector cell starts from.
#[test]
fn axis_keys_match_table_22() {
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
    assert_matches!(
        primary.first_vector_type(1000),
        Err(FitsError::InvalidKeyword { name }) if name == "1CTYP1000"
    );
}

/// A key of one description of a table WCS.
type Key = fn(TableWcsResolver) -> Option<Keyword>;

/// The matrix, parameter and column keys of Table 22, for the primary description and an
/// alternate, in the full and the short spelling.
#[test]
fn matrix_parameter_and_column_keys_match_table_22() {
    use KeywordSpelling::{Full, Short};
    use TableMatrixKeyword::{Cd, Pc};
    let primary = TableWcsResolver::new(Alternate::PRIMARY);
    let alternate = TableWcsResolver::new(Alternate::new('A'));
    let keys: [(Key, &str, &str); 13] = [
        (|r| r.pixel_matrix_key(Pc, 2, 3, Full), "TPC2_3", "TPC2_3A"),
        (|r| r.pixel_matrix_key(Pc, 2, 3, Short), "TP2_3", "TP2_3A"),
        (|r| r.pixel_matrix_key(Cd, 2, 3, Full), "TCD2_3", "TCD2_3A"),
        (|r| r.pixel_matrix_key(Cd, 2, 3, Short), "TC2_3", "TC2_3A"),
        (|r| r.vector_matrix_key(Pc, 2, 3, 17), "23PC17", "23PC17A"),
        (|r| r.vector_matrix_key(Cd, 2, 3, 17), "23CD17", "23CD17A"),
        (|r| r.pixel_parameter_key(2, 1, Full), "TPV2_1", "TPV2_1A"),
        (|r| r.pixel_parameter_key(2, 1, Short), "TV2_1", "TV2_1A"),
        (
            |r| r.vector_parameter_key(2, 17, 1, Full),
            "2PV17_1",
            "2PV17_1A",
        ),
        (
            |r| r.vector_parameter_key(2, 17, 1, Short),
            "2V17_1",
            "2V17_1A",
        ),
        (|r| r.column_key("LONP", 17), "LONP17", "LONP17A"),
        (|r| r.column_key("LATP", 17), "LATP17", "LATP17A"),
        (|r| r.column_key("WCAX", 17), "WCAX17", "WCAX17A"),
    ];
    for (key, primary_key, alternate_key) in keys {
        assert_eq!(key(primary).unwrap().as_str(), primary_key);
        assert_eq!(key(alternate).unwrap().as_str(), alternate_key);
    }
}
