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
