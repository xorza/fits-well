use super::*;
use crate::reader::FitsReader;
use std::fs::File;

#[test]
fn reads_the_real_uv_random_groups() {
    let file = File::open("tests/data/fits/DDTSUVDATA.fits").unwrap();
    let mut reader = FitsReader::open(file).unwrap();
    let groups = reader.read_groups(0).unwrap();

    assert_eq!(groups.gcount, 7956);
    assert_eq!(groups.pcount, 6);
    assert_eq!(groups.group_shape, vec![3, 4, 1, 1, 1]);
    assert_eq!(groups.array_len(), 12);
    assert_eq!(groups.bitpix(), Bitpix::F32);
    assert_eq!(
        groups.parameter_names,
        vec!["UU--", "VV--", "WW--", "BASELINE", "DATE", "DATE"]
    );

    // Each group yields PCOUNT params and an array of 12 elements.
    let params = groups.parameters_physical(0);
    assert_eq!(params.len(), 6);
    assert_eq!(groups.array_physical(0).len(), 12);
    // The DATE parameter (index 4) has PZERO5 = 2445728.5 (a Julian date), so
    // its physical value lands in that range, not near zero.
    assert!(
        params[4] > 2_445_728.0 && params[4] < 2_445_730.0,
        "DATE param = {}",
        params[4]
    );
    // §6.3: the two PTYPE='DATE' addends (indices 4, 5) sum to the logical DATE.
    assert_eq!(
        groups.parameter_physical(0, "DATE"),
        Some(params[4] + params[5])
    );
    // A single-occurrence name returns just that parameter; an absent name → None.
    assert_eq!(groups.parameter_physical(0, "BASELINE"), Some(params[3]));
    assert_eq!(groups.parameter_physical(0, "NONE"), None);
}

#[test]
fn parameter_physical_sums_addends_sharing_a_ptype() {
    // §6.3: two group parameters share PTYPEn='DATE' (a high-precision split); the
    // logical value is the SUM of the two addends' physical values — here both
    // non-zero, so a "return the first addend" bug would be caught.
    let mut header = Header::new();
    header
        .set("BITPIX", -32)
        .set("NAXIS", 2)
        .set("NAXIS1", 0)
        .set("NAXIS2", 1)
        .set("GROUPS", true)
        .set("PCOUNT", 2)
        .set("GCOUNT", 1)
        .set("PTYPE1", "DATE")
        .set("PSCAL1", 1.0)
        .set("PZERO1", 2_445_728.5)
        .set("PTYPE2", "DATE")
        .set("PSCAL2", 1.0)
        .set("PZERO2", 0.25);
    // One group: param1 raw 10.0, param2 raw 0.5, then one array element 99.0.
    let mut data = Vec::new();
    for v in [10.0f32, 0.5, 99.0] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let groups = RandomGroups::from_data(&header, &data).unwrap();

    // Raw per-addend values stay available separately.
    assert_eq!(groups.parameters_physical(0), vec![2_445_738.5, 0.75]);
    // Summed logical DATE = (2445728.5 + 10) + (0.25 + 0.5) = 2445739.25.
    assert_eq!(groups.parameter_physical(0, "DATE"), Some(2_445_739.25));
    assert_eq!(groups.parameter_physical(0, "NONE"), None);
}

#[test]
fn read_groups_rejects_non_random_groups_hdus() {
    let file = File::open("tests/data/fits/UITfuv2582gc.fits").unwrap();
    let mut reader = FitsReader::open(file).unwrap();
    assert!(matches!(
        reader.read_groups(0),
        Err(FitsError::NotRandomGroups)
    ));
}
