use crate::groups::*;
use crate::header_model::record_name::RecordName;
use crate::internals::cards;
use crate::internals::fixture::Fixture;
use crate::internals::samples;
use std::assert_matches;

/// The physical parameter values of `group`.
fn parameters(groups: &RandomGroups, group: usize) -> Result<Vec<f64>> {
    let mut out = Vec::new();
    groups.parameters_physical_into(group, &mut out)?;
    Ok(out)
}

/// The physical array values of `group`.
fn array(groups: &RandomGroups, group: usize) -> Result<Vec<f64>> {
    let mut out = Vec::new();
    groups.array_physical_into(group, &mut out)?;
    Ok(out)
}

/// The physical value of the parameter `name` names in `group`.
fn named(groups: &RandomGroups, group: usize, name: &str) -> Result<f64> {
    groups.parameter_physical(group, &groups.parameter(name)?)
}

/// A random-groups header: `NAXIS1 = 0`, then `axes` for each group's array.
fn groups_header(bitpix: i64, axes: &[i64], pcount: i64, gcount: i64) -> Header {
    let axes: Vec<i64> = std::iter::once(0).chain(axes.iter().copied()).collect();
    let mut header = cards::structural(None, bitpix, &axes);
    header
        .set_internal(keywords::GROUPS, true)
        .set_internal(keywords::PCOUNT, pcount)
        .set_internal(keywords::GCOUNT, gcount);
    header
}

#[test]
fn reads_the_real_uv_random_groups() {
    let mut reader = Fixture::Ddts.stream();
    let groups = reader.read_groups(0).unwrap();
    let metadata = groups.metadata();

    assert_eq!(metadata.gcount, 7956);
    assert_eq!(metadata.pcount, 6);
    assert_eq!(metadata.group_shape, [3, 4, 1, 1, 1]);
    assert_eq!(groups.array_len(), 12);
    assert_eq!(metadata.bitpix, Bitpix::F32);
    assert_eq!(
        metadata.parameter_names,
        ["UU--", "VV--", "WW--", "BASELINE", "DATE", "DATE"].map(|name| Some(name.to_string()))
    );

    // A physical read appends to the caller's buffer, of either float type.
    let mut out = vec![-1.0];
    groups.parameters_physical_into(0, &mut out).unwrap();
    assert_eq!((out.len(), out[0]), (7, -1.0));
    let mut narrow = Vec::<f32>::new();
    groups.array_physical_into(0, &mut narrow).unwrap();
    assert_eq!(narrow.len(), 12);

    // Group 0's stored f32s, scaled by PSCALn and offset by PZEROn: the u, v, w
    // scale is 7.04218409114E-10, and DATE (index 4) adds PZERO5 = 2445728.5 to
    // 0.2133636474609375 — exact in f64, whose step at 2^21 is 2^-31.
    let params = parameters(&groups, 0).unwrap();
    assert_eq!(
        params,
        [
            -11_642.337_890_625 * 7.042_184_091_14e-10,
            17_055.679_687_5 * 7.042_184_091_14e-10,
            -14_359.027_343_75 * 7.042_184_091_14e-10,
            258.0,
            2_445_728.5 + 0.213_363_647_460_937_5,
            0.0,
        ]
    );
    // BSCALE 1 and BZERO 0 leave the array as stored.
    assert_eq!(
        array(&groups, 0).unwrap(),
        [
            12.430_867_195_129_395,
            0.568_607_449_531_555_2,
            3.999_938_726_425_171,
            12.740_436_553_955_078,
            0.313_985_109_329_223_63,
            3.999_938_726_425_171,
            0.0,
            0.0,
            3.999_938_726_425_171,
            0.0,
            0.0,
            3.999_938_726_425_171,
        ]
    );
    // §6.3: the two PTYPE='DATE' addends (indices 4, 5) sum to the logical DATE.
    assert_eq!(named(&groups, 0, "DATE").unwrap(), params[4] + params[5]);
    // A single-occurrence name returns just that parameter; an absent name is refused.
    assert_eq!(named(&groups, 0, "BASELINE").unwrap(), params[3]);
    assert_matches!(
        named(&groups, 0, "NONE"),
        Err(FitsError::GroupParameterNotFound { name }) if name == "NONE"
    );

    for result in [
        parameters(&groups, metadata.gcount).map(|_| ()),
        array(&groups, metadata.gcount).map(|_| ()),
        named(&groups, metadata.gcount, "DATE").map(|_| ()),
    ] {
        assert_matches!(
            result,
            Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::Group,
                index: 7956,
                len: 7956
            })
        );
    }
    assert_eq!(parameters(&groups, 0).unwrap().len(), 6);
}

#[test]
fn parameter_physical_sums_addends_sharing_a_ptype() {
    // §6.3: two group parameters share PTYPEn='DATE' (a high-precision split); the
    // logical value is the SUM of the two addends' physical values — here both
    // non-zero, so a "return the first addend" bug would be caught.
    let mut header = groups_header(-32, &[1], 2, 1);
    header
        .set_internal("PTYPE1", "DATE")
        .set_internal("PSCAL1", 1.0)
        .set_internal("PZERO1", 2_445_728.5)
        .set_internal("PTYPE2", "DATE")
        .set_internal("PSCAL2", 1.0)
        .set_internal("PZERO2", 0.25);
    // One group: param1 raw 10.0, param2 raw 0.5, then one array element 99.0.
    let data = samples::be(&[10.0f32, 0.5, 99.0], f32::to_be_bytes);
    let groups = RandomGroups::from_data(&header, &data).unwrap();

    // Raw per-addend values stay available separately.
    assert_eq!(parameters(&groups, 0).unwrap(), vec![2_445_738.5, 0.75]);
    // Summed logical DATE = (2445728.5 + 10) + (0.25 + 0.5) = 2445739.25.
    assert_eq!(named(&groups, 0, "DATE").unwrap(), 2_445_739.25);
    // `PTYPEn` names compare without regard to case, as a column's `TTYPEn` does.
    assert_eq!(named(&groups, 0, "date").unwrap(), 2_445_739.25);
    assert_matches!(
        named(&groups, 0, "NONE"),
        Err(FitsError::GroupParameterNotFound { name }) if name == "NONE"
    );

    for keyword in ["PSCAL1", "PZERO1", "BSCALE"] {
        let mut malformed = header.clone();
        malformed.set_internal(keyword, "not numeric");
        assert_matches!(
            RandomGroups::from_data(&malformed, &data),
            Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), .. }) if name == keyword
        );
    }
}

#[test]
fn array_physical_maps_blank_to_nan_for_every_integer_bitpix() {
    for bitpix in [Bitpix::U8, Bitpix::I16, Bitpix::I32, Bitpix::I64] {
        let mut header = groups_header(bitpix.code(), &[3], 1, 1);
        header
            .set_internal("PTYPE1", "PARAM")
            .set_internal(keywords::BSCALE, 2.0)
            .set_internal(keywords::BZERO, 5.0)
            .set_internal(keywords::BLANK, 10);
        let stored = [10i64, 9, 10, 11];
        let data = match bitpix {
            Bitpix::U8 => Data::U8(stored.map(|v| u8::try_from(v).unwrap()).to_vec()),
            Bitpix::I16 => Data::I16(stored.map(|v| i16::try_from(v).unwrap()).to_vec()),
            Bitpix::I32 => Data::I32(stored.map(|v| i32::try_from(v).unwrap()).to_vec()),
            Bitpix::I64 => Data::I64(stored.to_vec()),
            Bitpix::F32 | Bitpix::F64 => unreachable!("integer cases only"),
        }
        .encode();
        let groups = RandomGroups::from_data(&header, &data).unwrap();

        assert_eq!(parameters(&groups, 0).unwrap(), vec![10.0], "{bitpix:?}");
        let physical = array(&groups, 0).unwrap();
        assert_eq!(physical[0], 23.0, "5 + 2·9 for {bitpix:?}");
        assert!(physical[1].is_nan(), "BLANK for {bitpix:?}");
        assert_eq!(physical[2], 27.0, "5 + 2·11 for {bitpix:?}");
    }
}

#[test]
fn naxis1_group_has_one_array_element_matching_data_bytes() {
    // A `NAXIS = 1` random group (only the `NAXIS1 = 0` sentinel, no array axis) has,
    // per Eqs. 4 and 6, PCOUNT params + an empty-product array of 1 element per
    // group — the way `HduKind::data_bytes` sizes the unit. `array_len` must agree (1, not
    // 0), or `from_data` refuses a unit the reader already sized as readable.
    let mut header = groups_header(-32, &[], 1, 2);
    header.set_internal("PTYPE1", "P");
    // 2 groups × (1 param + 1 array element) = 4 floats.
    let data = samples::be(&[1.0f32, 10.0, 2.0, 20.0], f32::to_be_bytes);
    let groups = RandomGroups::from_data(&header, &data).unwrap();
    assert!(groups.metadata().group_shape.is_empty());
    assert_eq!(groups.array_len(), 1); // empty product, not 0
    // group 0: param 1.0, array [10.0]; group 1: param 2.0, array [20.0].
    assert_eq!(parameters(&groups, 0).unwrap(), vec![1.0]);
    assert_eq!(array(&groups, 0).unwrap(), vec![10.0]);
    assert_eq!(parameters(&groups, 1).unwrap(), vec![2.0]);
    assert_eq!(array(&groups, 1).unwrap(), vec![20.0]);
}

#[test]
fn raw_group_view_preserves_i64_extremes_and_group_boundaries() {
    let header = groups_header(64, &[2], 2, 2);
    let stored = [
        i64::MIN,
        i64::MAX,
        9_007_199_254_740_993,
        -9_007_199_254_740_993,
        7,
        8,
        9,
        10,
    ];
    let data = samples::be(&stored, i64::to_be_bytes);
    let groups = RandomGroups::from_data(&header, &data).unwrap();

    let first = groups.group(0).unwrap();
    assert_eq!(first.parameters, View::I64(&[i64::MIN, i64::MAX]));
    assert_eq!(
        first.array,
        View::I64(&[9_007_199_254_740_993, -9_007_199_254_740_993])
    );
    let second = groups.group(1).unwrap();
    assert_eq!(second.parameters, View::I64(&[7, 8]));
    assert_eq!(second.array, View::I64(&[9, 10]));
    assert_matches!(
        groups.group(2),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Group,
            index: 2,
            len: 2
        })
    );
}

#[test]
fn raw_group_view_preserves_float_bit_patterns() {
    let f32_header = groups_header(-32, &[2], 1, 1);
    let f32_bits = [0x7fc0_1234, 0x8000_0000, 0x0000_0001];
    let f32_data = samples::be(&f32_bits, u32::to_be_bytes);
    let f32_groups = RandomGroups::from_data(&f32_header, &f32_data).unwrap();
    let f32_group = f32_groups.group(0).unwrap();
    let View::F32(f32_parameters) = f32_group.parameters else {
        panic!("BITPIX -32 parameters must be f32");
    };
    let View::F32(f32_array) = f32_group.array else {
        panic!("BITPIX -32 array must be f32");
    };
    let f32_bits = f32_bits.map(u64::from);
    assert_eq!(samples::bits_of(f32_parameters), f32_bits[..1]);
    assert_eq!(samples::bits_of(f32_array), f32_bits[1..]);

    let f64_header = groups_header(-64, &[2], 1, 1);
    let f64_bits = [
        0x7ff8_0000_0000_1234,
        0x8000_0000_0000_0000,
        0x0000_0000_0000_0001,
    ];
    let f64_data = samples::be(&f64_bits, u64::to_be_bytes);
    let f64_groups = RandomGroups::from_data(&f64_header, &f64_data).unwrap();
    let f64_group = f64_groups.group(0).unwrap();
    let View::F64(f64_parameters) = f64_group.parameters else {
        panic!("BITPIX -64 parameters must be f64");
    };
    let View::F64(f64_array) = f64_group.array else {
        panic!("BITPIX -64 array must be f64");
    };
    assert_eq!(samples::bits_of(f64_parameters), f64_bits[..1]);
    assert_eq!(samples::bits_of(f64_array), f64_bits[1..]);
}

/// A parameter past the 999th has no `PTYPEn`/`PSCALn`/`PZEROn` keyword, so it is
/// unnamed and unscaled. 1001 byte parameters, all 0 but the last three (3, 5, 7):
/// `PSCAL999 = 2` doubles 3 to 6 under `PTYPE999 = 'A'`, and 5 and 7 read as stored.
/// The unnamed sum is the 998 zeros plus 5 + 7 = 12.
#[test]
fn parameters_past_the_999th_take_the_keyword_defaults() {
    let mut header = groups_header(8, &[1], 1001, 1);
    header
        .set_internal("PTYPE999", "A")
        .set_internal("PSCAL999", 2.0);
    let mut data = vec![0u8; 1002];
    data[998..].copy_from_slice(&[3, 5, 7, 9]);
    let groups = RandomGroups::from_data(&header, &data).unwrap();

    let metadata = groups.metadata();
    assert_eq!(metadata.pcount, 1001);
    assert_eq!(metadata.parameter_names.len(), 999);
    assert_eq!(metadata.parameter_names[998].as_deref(), Some("A"));
    assert_eq!(metadata.parameter_names[0], None);
    let values = parameters(&groups, 0).unwrap();
    assert_eq!(values.len(), 1001);
    assert!(values[..998].iter().all(|&value| value == 0.0));
    assert_eq!(values[998..], [6.0, 5.0, 7.0]);
    assert_eq!(named(&groups, 0, "A").unwrap(), 6.0);
    // An absent `PTYPEn` names nothing, not the empty name.
    assert_matches!(
        groups.parameter(""),
        Err(FitsError::GroupParameterNotFound { name }) if name.is_empty()
    );
    assert_eq!(array(&groups, 0).unwrap(), vec![9.0]);
}

/// The counts are checked against the data before anything is sized from them.
/// With no groups the unit is empty for any `PCOUNT`, so 2⁶⁰ parameters read as
/// zero groups with the 999 keyword slots. (2⁶³ − 1)·3 overflows a group's array,
/// 3·2⁶² + 2⁶² = 2⁶⁴ overflows a group's length, and 4 groups of 2⁶² + 1 elements
/// overflow the unit.
#[test]
fn group_counts_are_checked_before_they_size_anything() {
    let huge = RandomGroups::from_data(&groups_header(8, &[1], 1 << 60, 0), &[]).unwrap();
    assert_eq!(huge.metadata().pcount, 1 << 60);
    assert_eq!(huge.metadata().parameter_names.len(), 999);

    for (header, data) in [
        (groups_header(8, &[i64::MAX, 3], 0, 1), &[][..]),
        (groups_header(8, &[1 << 62, 3], 1 << 62, 0), &[]),
        (groups_header(8, &[1], 1 << 62, 4), &[]),
    ] {
        assert_matches!(
            RandomGroups::from_data(&header, data),
            Err(FitsError::DataUnitOverflow)
        );
    }
}
