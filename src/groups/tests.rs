use crate::data::ImageView;
use crate::error::Indexed;
use crate::groups::*;
use crate::reader::FitsReader;
use std::fs::File;

#[test]
fn reads_the_real_uv_random_groups() {
    let file = File::open("tests/data/fits/DDTSUVDATA.fits").unwrap();
    let mut reader = FitsReader::open(file).unwrap();
    let groups = reader.read_groups(0).unwrap();
    let metadata = groups.metadata();

    assert_eq!(metadata.gcount, 7956);
    assert_eq!(metadata.pcount, 6);
    assert_eq!(metadata.group_shape, [3, 4, 1, 1, 1]);
    assert_eq!(groups.array_len(), 12);
    assert_eq!(metadata.bitpix, Bitpix::F32);
    assert_eq!(
        metadata.parameter_names,
        ["UU--", "VV--", "WW--", "BASELINE", "DATE", "DATE"]
    );

    // Each group yields PCOUNT params and an array of 12 elements.
    let params = groups.parameters_physical(0).unwrap();
    assert_eq!(params.len(), 6);
    assert_eq!(groups.array_physical(0).unwrap().len(), 12);
    // The DATE parameter (index 4) has PZERO5 = 2445728.5 (a Julian date), so
    // its physical value lands in that range, not near zero.
    assert!(
        params[4] > 2_445_728.0 && params[4] < 2_445_730.0,
        "DATE param = {}",
        params[4]
    );
    // §6.3: the two PTYPE='DATE' addends (indices 4, 5) sum to the logical DATE.
    assert_eq!(
        groups.parameter_physical(0, "DATE").unwrap(),
        Some(params[4] + params[5])
    );
    // A single-occurrence name returns just that parameter; an absent name → None.
    assert_eq!(
        groups.parameter_physical(0, "BASELINE").unwrap(),
        Some(params[3])
    );
    assert_eq!(groups.parameter_physical(0, "NONE").unwrap(), None);

    for result in [
        groups.parameters_physical(metadata.gcount).map(|_| ()),
        groups.array_physical(metadata.gcount).map(|_| ()),
        groups
            .parameter_physical(metadata.gcount, "DATE")
            .map(|_| ()),
    ] {
        assert!(matches!(
            result,
            Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::Group,
                index: 7956,
                len: 7956
            })
        ));
    }
    let mut detached = groups.metadata();
    detached.gcount = 0;
    detached.pcount = usize::MAX;
    detached.group_shape = &[];
    detached.parameter_names = &[];
    detached.bitpix = Bitpix::I64;
    assert_eq!(detached.gcount, 0);
    assert_eq!(detached.pcount, usize::MAX);
    assert!(detached.group_shape.is_empty());
    assert!(detached.parameter_names.is_empty());
    assert_eq!(detached.bitpix, Bitpix::I64);
    assert_eq!(groups.parameters_physical(0).unwrap().len(), 6);
}

#[test]
fn parameter_physical_sums_addends_sharing_a_ptype() {
    // §6.3: two group parameters share PTYPEn='DATE' (a high-precision split); the
    // logical value is the SUM of the two addends' physical values — here both
    // non-zero, so a "return the first addend" bug would be caught.
    let mut header = Header::new();
    header
        .set_internal("BITPIX", -32)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 0)
        .set_internal("NAXIS2", 1)
        .set_internal("GROUPS", true)
        .set_internal("PCOUNT", 2)
        .set_internal("GCOUNT", 1)
        .set_internal("PTYPE1", "DATE")
        .set_internal("PSCAL1", 1.0)
        .set_internal("PZERO1", 2_445_728.5)
        .set_internal("PTYPE2", "DATE")
        .set_internal("PSCAL2", 1.0)
        .set_internal("PZERO2", 0.25);
    // One group: param1 raw 10.0, param2 raw 0.5, then one array element 99.0.
    let mut data = Vec::new();
    for v in [10.0f32, 0.5, 99.0] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let groups = RandomGroups::from_data(&header, &data).unwrap();

    // Raw per-addend values stay available separately.
    assert_eq!(
        groups.parameters_physical(0).unwrap(),
        vec![2_445_738.5, 0.75]
    );
    // Summed logical DATE = (2445728.5 + 10) + (0.25 + 0.5) = 2445739.25.
    assert_eq!(
        groups.parameter_physical(0, "DATE").unwrap(),
        Some(2_445_739.25)
    );
    assert_eq!(groups.parameter_physical(0, "NONE").unwrap(), None);

    for keyword in ["PSCAL1", "PZERO1", "BSCALE"] {
        let mut malformed = header.clone();
        malformed.set_internal(keyword, "not numeric");
        assert!(matches!(
            RandomGroups::from_data(&malformed, &data),
            Err(FitsError::TypeMismatch { name, .. }) if name == keyword
        ));
    }
}

#[test]
fn array_physical_maps_blank_to_nan_for_every_integer_bitpix() {
    for bitpix in [Bitpix::U8, Bitpix::I16, Bitpix::I32, Bitpix::I64] {
        let mut header = Header::new();
        header
            .set_internal("BITPIX", bitpix.code())
            .set_internal("NAXIS", 2)
            .set_internal("NAXIS1", 0)
            .set_internal("NAXIS2", 3)
            .set_internal("GROUPS", true)
            .set_internal("PCOUNT", 1)
            .set_internal("GCOUNT", 1)
            .set_internal("PTYPE1", "PARAM")
            .set_internal("BSCALE", 2.0)
            .set_internal("BZERO", 5.0)
            .set_internal("BLANK", 10);
        let data = match bitpix {
            Bitpix::U8 => vec![10, 9, 10, 11],
            Bitpix::I16 => [10i16, 9, 10, 11]
                .into_iter()
                .flat_map(i16::to_be_bytes)
                .collect(),
            Bitpix::I32 => [10i32, 9, 10, 11]
                .into_iter()
                .flat_map(i32::to_be_bytes)
                .collect(),
            Bitpix::I64 => [10i64, 9, 10, 11]
                .into_iter()
                .flat_map(i64::to_be_bytes)
                .collect(),
            Bitpix::F32 | Bitpix::F64 => unreachable!("integer cases only"),
        };
        let groups = RandomGroups::from_data(&header, &data).unwrap();

        assert_eq!(
            groups.parameters_physical(0).unwrap(),
            vec![10.0],
            "{bitpix:?}"
        );
        let physical = groups.array_physical(0).unwrap();
        assert_eq!(physical[0], 23.0, "5 + 2·9 for {bitpix:?}");
        assert!(physical[1].is_nan(), "BLANK for {bitpix:?}");
        assert_eq!(physical[2], 27.0, "5 + 2·11 for {bitpix:?}");
    }
}

#[test]
fn naxis1_group_has_one_array_element_matching_data_extent() {
    // A `NAXIS = 1` random group (only the `NAXIS1 = 0` sentinel, no array axis) has,
    // per Eq. 2, PCOUNT params + an empty-product array of 1 element per group — the
    // way `data_extent` sizes the unit. `array_len` must agree (1, not 0), or
    // `from_data` rejects a unit the reader already sized as readable.
    let mut header = Header::new();
    header
        .set_internal("BITPIX", -32)
        .set_internal("NAXIS", 1)
        .set_internal("NAXIS1", 0)
        .set_internal("GROUPS", true)
        .set_internal("PCOUNT", 1)
        .set_internal("GCOUNT", 2)
        .set_internal("PTYPE1", "P");
    // 2 groups × (1 param + 1 array element) = 4 floats.
    let mut data = Vec::new();
    for v in [1.0f32, 10.0, 2.0, 20.0] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let groups = RandomGroups::from_data(&header, &data).unwrap();
    assert!(groups.metadata().group_shape.is_empty());
    assert_eq!(groups.array_len(), 1); // empty product, not 0
    // group 0: param 1.0, array [10.0]; group 1: param 2.0, array [20.0].
    assert_eq!(groups.parameters_physical(0).unwrap(), vec![1.0]);
    assert_eq!(groups.array_physical(0).unwrap(), vec![10.0]);
    assert_eq!(groups.parameters_physical(1).unwrap(), vec![2.0]);
    assert_eq!(groups.array_physical(1).unwrap(), vec![20.0]);
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

#[test]
fn raw_group_view_preserves_i64_extremes_and_group_boundaries() {
    let mut header = Header::new();
    header
        .set_internal("BITPIX", 64)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 0)
        .set_internal("NAXIS2", 2)
        .set_internal("GROUPS", true)
        .set_internal("PCOUNT", 2)
        .set_internal("GCOUNT", 2);
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
    let data: Vec<u8> = stored.into_iter().flat_map(i64::to_be_bytes).collect();
    let groups = RandomGroups::from_data(&header, &data).unwrap();

    let first = groups.group_by_idx(0).unwrap();
    assert_eq!(first.parameters, ImageView::I64(&[i64::MIN, i64::MAX]));
    assert_eq!(
        first.array,
        ImageView::I64(&[9_007_199_254_740_993, -9_007_199_254_740_993])
    );
    let second = groups.group_by_idx(1).unwrap();
    assert_eq!(second.parameters, ImageView::I64(&[7, 8]));
    assert_eq!(second.array, ImageView::I64(&[9, 10]));
    assert!(matches!(
        groups.group_by_idx(2),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Group,
            index: 2,
            len: 2
        })
    ));
}

#[test]
fn raw_group_view_preserves_float_bit_patterns() {
    let mut f32_header = Header::new();
    f32_header
        .set_internal("BITPIX", -32)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 0)
        .set_internal("NAXIS2", 2)
        .set_internal("GROUPS", true)
        .set_internal("PCOUNT", 1)
        .set_internal("GCOUNT", 1);
    let f32_bits = [0x7fc0_1234, 0x8000_0000, 0x0000_0001];
    let f32_data: Vec<u8> = f32_bits.into_iter().flat_map(u32::to_be_bytes).collect();
    let f32_groups = RandomGroups::from_data(&f32_header, &f32_data).unwrap();
    let f32_group = f32_groups.group_by_idx(0).unwrap();
    let ImageView::F32(f32_parameters) = f32_group.parameters else {
        panic!("BITPIX -32 parameters must be f32");
    };
    let ImageView::F32(f32_array) = f32_group.array else {
        panic!("BITPIX -32 array must be f32");
    };
    assert_eq!(f32_parameters[0].to_bits(), f32_bits[0]);
    assert_eq!(
        f32_array
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        f32_bits[1..]
    );

    let mut f64_header = Header::new();
    f64_header
        .set_internal("BITPIX", -64)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 0)
        .set_internal("NAXIS2", 2)
        .set_internal("GROUPS", true)
        .set_internal("PCOUNT", 1)
        .set_internal("GCOUNT", 1);
    let f64_bits = [
        0x7ff8_0000_0000_1234,
        0x8000_0000_0000_0000,
        0x0000_0000_0000_0001,
    ];
    let f64_data: Vec<u8> = f64_bits.into_iter().flat_map(u64::to_be_bytes).collect();
    let f64_groups = RandomGroups::from_data(&f64_header, &f64_data).unwrap();
    let f64_group = f64_groups.group_by_idx(0).unwrap();
    let ImageView::F64(f64_parameters) = f64_group.parameters else {
        panic!("BITPIX -64 parameters must be f64");
    };
    let ImageView::F64(f64_array) = f64_group.array else {
        panic!("BITPIX -64 array must be f64");
    };
    assert_eq!(f64_parameters[0].to_bits(), f64_bits[0]);
    assert_eq!(
        f64_array
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        f64_bits[1..]
    );
}
