use crate::time::*;

/// Golden values throughout are from `astropy.time` (ERFA).
#[test]
fn iso_to_jd_and_mjd_match_astropy() {
    let cases: &[(&str, f64, f64)] = &[
        ("2000-01-01T12:00:00", 2451545.0, 51544.5),
        ("1858-11-17T00:00:00", 2400000.5, 0.0),
        ("2024-02-29T06:30:15.5", 2460369.771012731, 60369.271012731),
        ("1900-01-01T00:00:00", 2415020.5, 15020.0),
        ("2024-06-01", 2460462.5, 60462.0), // date-only ⇒ midnight
    ];
    for &(s, jd, mjd) in cases {
        let d = Datetime::parse(s).unwrap();
        let got_jd = d.to_jd(TimeScale::Utc).unwrap();
        let got_mjd = d.to_mjd(TimeScale::Utc).unwrap();
        assert!((got_jd - jd).abs() < 1e-7, "{s}: jd {got_jd} vs {jd}",);
        assert!((got_mjd - mjd).abs() < 1e-7, "{s}: mjd {got_mjd} vs {mjd}",);
    }
}

#[test]
fn datetime_round_trips_through_jd() {
    for s in [
        "2024-02-29T06:30:15.5",
        "1900-01-01T00:00:00",
        "2000-01-01T12:00:00",
    ] {
        let d = Datetime::parse(s).unwrap();
        let back = Datetime::from_jd(d.to_jd(TimeScale::Utc).unwrap(), TimeScale::Utc).unwrap();
        assert_eq!(
            (back.year, back.month, back.day),
            (d.year, d.month, d.day),
            "{s}"
        );
        assert_eq!((back.hour, back.minute), (d.hour, d.minute), "{s}");
        // Single-f64 JD at this epoch resolves the second to ~0.1 ms.
        assert!((back.second - d.second).abs() < 1e-3, "{s} second");
    }
}

#[test]
fn rejects_malformed_datetimes() {
    for s in [
        "",
        "2024",
        "2024-13-01",
        "2024-01-32",
        "2024-01-01T25:00:00",
        "2024-01-01T06:30",
        "x",
    ] {
        assert!(Datetime::parse(s).is_err(), "{s:?} should be rejected");
    }

    let mut header = crate::header::Header::new();
    header.set_internal("DATE-OBS", "2024-13-01");
    assert!(matches!(
        header.obs_mjd(),
        Err(FitsError::InvalidValue { card }) if card == "DATE '2024-13-01'"
    ));

    for jd in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        calendar_day_start(-99_999, 1, 1) - 1.0,
        calendar_day_start(100_000, 1, 1),
    ] {
        assert!(matches!(
            Datetime::from_jd(jd, TimeScale::Tt),
            Err(FitsError::InvalidValue { .. })
        ));
    }
}

#[test]
fn iso_8601_strictness() {
    // §9.1.1: omitted leading zeros, a `Z` designator, and wrong-width years are
    // all rejected.
    for bad in [
        "2024-1-01",            // 1-digit month
        "2024-01-1",            // 1-digit day
        "2024-01-01T6:30:00",   // 1-digit hour
        "2024-01-01T06:30:5",   // 1-digit second
        "2024-01-01T06:30:00Z", // forbidden Z designator
        "999-01-01",            // 3-digit year
        "10000-01-01",          // unsigned 5-digit year
        "-0044-03-15",          // signed 4-digit year
        "+2024-06-01",          // signed 4-digit year
        "+100000-01-01",        // signed 6-digit year
        "-100000-01-01",        // signed 6-digit year
    ] {
        assert!(Datetime::parse(bad).is_err(), "{bad:?} should be rejected");
    }
    assert_eq!(Datetime::parse("-00044-03-15").unwrap().year, -44);
    assert_eq!(Datetime::parse("+02024-06-01").unwrap().year, 2024);
    for (text, year) in [("-99999-01-01", -99999), ("+99999-12-31", 99999)] {
        let datetime = Datetime::parse(text).unwrap();
        assert_eq!(datetime.year, year);
        let round_trip =
            Datetime::from_jd(datetime.to_jd(TimeScale::Tt).unwrap(), TimeScale::Tt).unwrap();
        assert_eq!(
            (round_trip.year, round_trip.month, round_trip.day),
            (datetime.year, datetime.month, datetime.day)
        );
    }
    let mut outside_fits_range = Datetime::parse("+99999-12-31").unwrap();
    outside_fits_range.year = 100000;
    assert!(outside_fits_range.to_jd(TimeScale::Tt).is_err());
}

#[test]
fn signed_gregorian_years_use_floor_division() {
    let cases = [
        ("0000-01-01T00:00:00", 1_721_059.5),
        ("-00001-01-01T00:00:00", 1_720_694.5),
        ("-04800-01-01T00:00:00", -32_104.5),
        ("-04713-11-24T12:00:00", 0.0),
    ];
    for (text, expected_jd) in cases {
        let date = Datetime::parse(text).unwrap();
        assert_eq!(date.to_jd(TimeScale::Tt).unwrap(), expected_jd, "{text}");
        let round_trip = Datetime::from_jd(expected_jd, TimeScale::Tt).unwrap();
        assert_eq!(round_trip, date, "{text}");
    }
}

#[test]
fn utc_leap_second_matches_erfa_and_remains_distinct() {
    let cases = [
        (
            "2016-12-31T23:59:59",
            2_457_754.499_976_852,
            2_457_754.500_405_092_7,
        ),
        (
            "2016-12-31T23:59:60",
            2_457_754.499_988_426,
            2_457_754.500_416_666_7,
        ),
        ("2017-01-01T00:00:00", 2_457_754.5, 2_457_754.500_428_240_7),
    ];
    let mut tai = Vec::new();
    for (text, erfa_utc, erfa_tai) in cases {
        let datetime = Datetime::parse(text).unwrap();
        let utc = datetime.to_jd(TimeScale::Utc).unwrap();
        assert!((utc - erfa_utc).abs() < 5e-10, "{text}: UTC JD {utc:.12}");
        let converted = TimeScale::Utc.convert(utc, TimeScale::Tai).unwrap();
        assert!(
            (converted - erfa_tai).abs() < 5e-10,
            "{text}: TAI JD {converted:.12}"
        );
        tai.push(converted);
    }
    assert!(((tai[1] - tai[0]) * SEC_PER_DAY - 1.0).abs() < 1e-4);
    assert!(((tai[2] - tai[1]) * SEC_PER_DAY - 1.0).abs() < 1e-4);

    let leap = Datetime::from_jd(cases[1].1, TimeScale::Utc).unwrap();
    assert_eq!(
        (leap.year, leap.month, leap.day, leap.hour, leap.minute),
        (2016, 12, 31, 23, 59)
    );
    assert!((leap.second - 60.0).abs() < 1e-4);
}

#[test]
fn leap_second_labels_require_the_actual_utc_final_minute() {
    for (text, scale) in [
        ("2016-12-30T23:59:60", TimeScale::Utc),
        ("2016-12-31T12:00:60", TimeScale::Utc),
        ("2016-12-31T23:59:60", TimeScale::Tai),
    ] {
        let datetime = Datetime::parse(text).unwrap();
        assert!(matches!(
            datetime.to_jd(scale),
            Err(FitsError::InvalidValue { .. })
        ));
    }

    for transition in LEAP_SECONDS.windows(2) {
        assert_eq!(transition[1].3 - transition[0].3, 1.0);
        let (year, month, day, _) = transition[1];
        let transition_jdn = gregorian_to_jdn(year, month, day);
        let insertion_date = jdn_to_gregorian(transition_jdn - 1);
        let leap = Datetime {
            year: insertion_date.year,
            month: insertion_date.month,
            day: insertion_date.day,
            hour: 23,
            minute: 59,
            second: 60.0,
        };
        assert!(leap.to_jd(TimeScale::Utc).is_ok(), "{leap:?}");

        let ordinary_day = Datetime {
            year,
            month: month as u32,
            day: day as u32,
            hour: 23,
            minute: 59,
            second: 60.0,
        };
        assert!(
            ordinary_day.to_jd(TimeScale::Utc).is_err(),
            "{ordinary_day:?}"
        );
    }
}

#[test]
fn header_datetimes_use_the_declared_scale() {
    use crate::header::Header;

    let erfa_mjd = 2_457_754.499_988_426 - MJD0;
    let mut header = Header::new();
    header
        .set_internal("TIMESYS", "UTC")
        .set_internal("DATEREF", "2016-12-31T23:59:60")
        .set_internal("DATE-OBS", "2016-12-31T23:59:60")
        .set_internal("DATE-END", "2016-12-31T23:59:60");
    assert!((header.time().unwrap().mjdref - erfa_mjd).abs() < 5e-10);
    assert!((header.obs_mjd().unwrap().unwrap() - erfa_mjd).abs() < 5e-10);
    assert!((header.time_bounds().unwrap().end_mjd.unwrap() - erfa_mjd).abs() < 5e-10);

    header.set_internal("TIMESYS", "TAI");
    assert!(matches!(header.time(), Err(FitsError::InvalidValue { .. })));
    assert!(matches!(
        header.obs_mjd(),
        Err(FitsError::InvalidValue { .. })
    ));
    assert!(matches!(
        header.time_bounds(),
        Err(FitsError::InvalidValue { .. })
    ));
}

#[test]
fn reads_jepoch_and_bepoch_keywords() {
    use crate::header::Header;
    // JEPOCH=2000.0 ⇒ J2000.0 = MJD 51544.5, implied scale TDB.
    let mut hj = Header::new();
    hj.set_internal("JEPOCH", 2000.0);
    let ej = FitsTime::epoch(&hj).unwrap().unwrap();
    assert!((ej.mjd - 51544.5).abs() < 1e-6);
    assert_eq!(ej.scale, TimeScale::Tdb);
    // BEPOCH=1950.0 ⇒ B1950.0 = MJD 33281.92345905, implied scale ET ≈ TT.
    let mut hb = Header::new();
    hb.set_internal("BEPOCH", 1950.0);
    let eb = FitsTime::epoch(&hb).unwrap().unwrap();
    assert!((eb.mjd - 33281.92345905).abs() < 1e-4);
    assert_eq!(eb.scale, TimeScale::Tt);
    // Neither keyword ⇒ None.
    let empty = Header::new();
    assert!(FitsTime::epoch(&empty).unwrap().is_none());
}

#[test]
fn reads_bound_duration_and_error_keywords() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set_internal("MJD-BEG", 58000.0);
    h.set_internal("DATE-END", "2017-09-05T00:00:00");
    h.set_internal("MJD-AVG", 58000.5);
    h.set_internal("XPOSURE", 1200.0);
    h.set_internal("TELAPSE", 1500.0);
    h.set_internal("TIMEDEL", 0.1);
    h.set_internal("TIMSYER", 1e-6);
    let b = FitsTime::bounds(&h).unwrap();
    assert_eq!(b.beg_mjd, Some(58000.0));
    let end = Datetime::parse("2017-09-05T00:00:00")
        .unwrap()
        .to_mjd(TimeScale::Utc)
        .unwrap();
    assert!((b.end_mjd.unwrap() - end).abs() < 1e-9); // resolved from DATE-END
    assert_eq!(b.avg_mjd, Some(58000.5)); // §9.5 midpoint
    assert_eq!(b.xposure, Some(1200.0));
    assert_eq!(b.telapse, Some(1500.0));
    assert_eq!(b.timedel, Some(0.1));
    assert_eq!(b.timepixr, 0.5); // default when absent
    assert_eq!(b.timsyer, Some(1e-6));
    assert_eq!(b.timrder, None);

    h.set_internal("TIMESYS", 1)
        .set_internal("MJD-END", 58001.0);
    assert_eq!(FitsTime::bounds(&h).unwrap().end_mjd, Some(58001.0));
}

#[test]
fn gti_intervals_convert_to_absolute_mjd() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set_internal("MJDREF", 58000.0);
    h.set_internal("TIMEUNIT", "d");
    let t = FitsTime::from_header(&h).unwrap();
    let gtis = t.gti_intervals(&[0.0, 2.0], &[1.0, 3.0]).unwrap();
    assert_eq!(
        gtis,
        vec![
            GtiInterval {
                start_mjd: 58000.0,
                stop_mjd: 58001.0,
            },
            GtiInterval {
                start_mjd: 58002.0,
                stop_mjd: 58003.0,
            },
        ]
    );

    assert!(matches!(
        t.gti_intervals(&[0.0, 2.0], &[1.0]),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1,
        })
    ));

    let mut milliseconds = Header::new();
    milliseconds
        .set_internal("MJDREF", 58000.0)
        .set_internal("TIMEUNIT", "ms");
    let milliseconds = FitsTime::from_header(&milliseconds).unwrap();
    let gti = milliseconds.gti_intervals(&[0.0], &[1000.0]).unwrap()[0];
    assert_eq!(gti.start_mjd, 58000.0);
    assert!((gti.stop_mjd - (58000.0 + 1.0 / SEC_PER_DAY)).abs() < 1e-12);
}

#[test]
fn classifies_time_related_axes() {
    use TimeAxisKind::*;
    assert_eq!(TimeAxisKind::from_ctype("TIME"), Some(Time));
    assert_eq!(TimeAxisKind::from_ctype("time"), Some(Time));
    assert_eq!(TimeAxisKind::from_ctype("UTC"), Some(Time)); // a scale name is a time axis
    assert_eq!(TimeAxisKind::from_ctype("PHASE"), Some(Phase));
    assert_eq!(TimeAxisKind::from_ctype("TIMELAG"), Some(Timelag));
    assert_eq!(TimeAxisKind::from_ctype("FREQUENCY"), Some(Frequency));
    assert_eq!(TimeAxisKind::from_ctype("RA---TAN"), None);
}

#[test]
fn reads_phase_axis_and_folds() {
    use crate::error::FitsError;
    use crate::header::Header;
    let mut h = Header::new();
    h.set_internal("CTYPE2", "PHASE")
        .set_internal("CZPHS2", 5.0)
        .set_internal("CPERI2", 2.0)
        .set_internal("CTYPE2A", "PHASE")
        .set_internal("CZPHS2A", 7.0)
        .set_internal("CPERI2A", 4.0)
        .set_internal("TCTYP3", "PHASE")
        .set_internal("TCZPH3", 11.0)
        .set_internal("TCPER3", 5.0)
        .set_internal("TCTY3A", "PHASE")
        .set_internal("TCZP3A", 13.0)
        .set_internal("TCPR3A", 6.0)
        .set_internal("1CTYP5", "PHASE")
        .set_internal("1CZPH5", 17.0)
        .set_internal("1CPER5", 8.0)
        .set_internal("1CTY5A", "PHASE")
        .set_internal("1CZP5A", 19.0)
        .set_internal("1CPR5A", 10.0);

    let image = h.phase_axis(2, None).unwrap().unwrap();
    assert_eq!(image.zero_phase, 5.0);
    assert_eq!(image.period, Some(2.0));
    assert_eq!(image.fold(8.0).unwrap(), 0.5);
    assert_eq!(image.fold(5.0).unwrap(), 0.0);

    let alternate = h.phase_axis(2, Some('A')).unwrap().unwrap();
    assert_eq!(
        alternate,
        PhaseAxis {
            zero_phase: 7.0,
            period: Some(4.0),
        }
    );
    assert_eq!(
        h.phase_axis_pixel_list(3, None).unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 11.0,
            period: Some(5.0),
        }
    );
    assert_eq!(
        h.phase_axis_pixel_list(3, Some('A')).unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 13.0,
            period: Some(6.0),
        }
    );
    assert_eq!(
        h.phase_axis_array_column(1, 5, None).unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 17.0,
            period: Some(8.0),
        }
    );
    assert_eq!(
        h.phase_axis_array_column(1, 5, Some('A')).unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 19.0,
            period: Some(10.0),
        }
    );

    h.set_internal("CTYPE1", "RA---TAN");
    assert_eq!(h.phase_axis(1, None).unwrap(), None);

    h.set_internal("CTYPE4", "PHASE")
        .set_internal("CZPHS4", 23.0);
    let varying = h.phase_axis(4, None).unwrap().unwrap();
    assert_eq!(varying.period, None);
    assert!(matches!(
        varying.fold(30.0),
        Err(FitsError::InvalidValue { card }) if card.contains("no constant CPERI")
    ));
    h.set_internal("CPERI4", 0.0);
    assert_eq!(h.phase_axis(4, None).unwrap().unwrap().period, None);
    let overflow = PhaseAxis {
        zero_phase: 0.0,
        period: Some(f64::MIN_POSITIVE),
    };
    assert!(matches!(
        overflow.fold(f64::MAX),
        Err(FitsError::InvalidValue { card }) if card.contains("overflowed")
    ));

    h.set_internal("CTYPE6", "PHASE");
    assert!(matches!(
        h.phase_axis(6, None),
        Err(FitsError::InvalidValue { card }) if card.contains("CZPHS6")
    ));
}

#[test]
fn obs_mjd_falls_back_to_jepoch() {
    use crate::header::Header;
    // §9.5: absent DATE-OBS/MJD-OBS, JEPOCH stands in for the observation time.
    let mut h = Header::new();
    h.set_internal("JEPOCH", 2000.0); // J2000.0 = MJD 51544.5
    assert!((FitsTime::obs_mjd(&h).unwrap().unwrap() - 51544.5).abs() < 1e-6);
    // An explicit MJD-OBS still wins.
    h.set_internal("MJD-OBS", 58000.0);
    assert_eq!(FitsTime::obs_mjd(&h).unwrap(), Some(58000.0));
}

#[test]
fn epochs_match_astropy() {
    let cases: &[(&str, f64)] = &[
        ("J2000.0", 2451545.0),
        ("B1950.0", 2433282.42345905),
        ("J2015.5", 2457206.375),
        ("B1900.0", 2415020.31352),
    ];
    for &(s, jd) in cases {
        let e = Epoch::parse(s).unwrap();
        assert!((e.to_jd() - jd).abs() < 1e-5, "{s}: {} vs {jd}", e.to_jd());
    }
}

#[test]
fn scale_conversions_match_astropy() {
    // `convert` works in Julian Date; the golden values are astropy MJD in each
    // scale at UTC MJD 60462.0 (2024-06-01), given as the day-fraction beyond
    // 60462 (which `f64` represents without excess precision).
    const MJD0: f64 = 2_400_000.5;
    const BASE: f64 = 60462.0;
    let utc_jd = BASE + MJD0;
    let cases: &[(TimeScale, f64)] = &[
        (TimeScale::Tai, 0.000428240739),
        (TimeScale::Tt, 0.000800740738),
        (TimeScale::Tcg, 0.000812810154),
        (TimeScale::Tdb, 0.000800751230),
        (TimeScale::Tcb, 0.001069271013),
        (TimeScale::Gps, 0.000208333331),
    ];
    for &(scale, want_frac) in cases {
        let got_frac = TimeScale::Utc.convert(utc_jd, scale).unwrap() - MJD0 - BASE;
        assert!(
            (got_frac - want_frac).abs() < 1e-9,
            "UTC→{scale:?}: {got_frac:.12} vs {want_frac:.12} (Δ={:.2e} s)",
            (got_frac - want_frac) * 86400.0
        );
        // Round-trip back to UTC.
        let back = scale
            .convert(BASE + want_frac + MJD0, TimeScale::Utc)
            .unwrap()
            - MJD0;
        assert!(
            (back - BASE).abs() < 1e-9,
            "{scale:?}→UTC round-trip: {back}"
        );
    }
}

#[test]
fn local_and_non_finite_time_conversions_are_rejected() {
    let jd = 2_460_462.5;
    assert_eq!(TimeScale::Local.convert(jd, TimeScale::Local).unwrap(), jd);
    for result in [
        TimeScale::Local.convert(jd, TimeScale::Utc),
        TimeScale::Utc.convert(jd, TimeScale::Local),
        TimeScale::Utc.convert(f64::NAN, TimeScale::Tai),
        TimeScale::Utc.convert(f64::INFINITY, TimeScale::Tai),
        TimeScale::Utc.convert(f64::MAX, TimeScale::Tai),
        TimeScale::Utc.convert_dut1(jd, TimeScale::Ut1, f64::NAN),
    ] {
        assert!(matches!(result, Err(FitsError::InvalidValue { .. })));
    }
}

#[test]
fn tai_to_utc_selects_leap_offset_at_the_utc_instant() {
    let utc_transition = Datetime::parse("2017-01-01T00:00:00")
        .unwrap()
        .to_jd(TimeScale::Utc)
        .unwrap();
    // A TAI label ten seconds after UTC midnight still represents 23:59:34 UTC,
    // where TAI−UTC was 36 s. Looking up the leap count at the TAI label would
    // switch to 37 s six seconds too early.
    let tai = utc_transition + 10.0 / SEC_PER_DAY;
    let expected_utc = Datetime::parse("2016-12-31T23:59:34")
        .unwrap()
        .to_jd(TimeScale::Utc)
        .unwrap();
    let utc = TimeScale::Tai.convert(tai, TimeScale::Utc).unwrap();
    assert!((utc - expected_utc).abs() * SEC_PER_DAY < 1e-5);

    let dut1 = 0.25;
    let ut1 = TimeScale::Tai
        .convert_dut1(tai, TimeScale::Ut1, dut1)
        .unwrap();
    let expected_ut1 = utc_quasi_to_linear(expected_utc) + dut1 / SEC_PER_DAY;
    assert!((ut1 - expected_ut1).abs() * SEC_PER_DAY < 1e-5);
}

#[test]
fn ut1_uses_explicit_dut1() {
    const MJD0: f64 = 2_400_000.5;
    let utc_jd = 60462.0 + MJD0;
    let dut1 = -0.020434661; // astropy ΔUT1 = UT1 − UTC at 2024-06-01
    let ut1 = TimeScale::Utc
        .convert_dut1(utc_jd, TimeScale::Ut1, dut1)
        .unwrap()
        - MJD0;
    // astropy UT1 MJD, as the day-fraction beyond 60462 (UT1 − 60462).
    let want = -0.000000236512;
    assert!(
        (ut1 - 60462.0 - want).abs() < 1e-9,
        "UT1 {ut1:.12} (Δ={:.4e} s)",
        (ut1 - 60462.0 - want) * 86400.0
    );
    // Round-trip back to UTC.
    let back = TimeScale::Ut1
        .convert_dut1(ut1 + MJD0, TimeScale::Utc, dut1)
        .unwrap()
        - MJD0;
    assert!((back - 60462.0).abs() < 1e-9);
    // With ΔUT1 = 0, UT1 collapses to UTC (the `convert` default).
    assert_eq!(
        TimeScale::Utc.convert(utc_jd, TimeScale::Ut1).unwrap(),
        utc_jd
    );
}

#[test]
fn time_axis_uses_complete_wcs_row_unit_and_scale() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set_internal("NAXIS", 1).set_internal("MJDREF", 58000.0);
    h.set_internal("TIMESYS", "UTC")
        .set_internal("TIMEUNIT", "s");
    h.set_internal("CTYPE1A", "TAI")
        .set_internal("CUNIT1A", "d");
    h.set_internal("CRPIX1A", 1.0)
        .set_internal("CRVAL1A", 0.0)
        .set_internal("CD1_1A", 2.0);
    let t = FitsTime::from_header(&h).unwrap();
    let alternate = h.wcs(Some('A')).unwrap();
    // CD1_1 = 2 d/pixel and pixel offset 0.5 produce exactly one day. CUNIT1A and
    // CTYPE1A override the global seconds/UTC frame.
    assert_eq!(
        t.time_axis_mjd(&alternate, 1, &[1.5]).unwrap(),
        Some(TimeCoordinate {
            mjd: 58001.0,
            scale: TimeScale::Tai,
        })
    );

    h.set_internal("CUNIT1A", "ms");
    let milliseconds = h.wcs(Some('A')).unwrap();
    // With the same CD row, an offset of 500 pixels is 1000 ms = 1 s.
    let coordinate = t
        .time_axis_mjd(&milliseconds, 1, &[501.0])
        .unwrap()
        .unwrap();
    assert!((coordinate.mjd - (58000.0 + 1.0 / SEC_PER_DAY)).abs() < 1e-12);

    h.set_internal("CUNIT1A", "Hz");
    let invalid_unit = h.wcs(Some('A')).unwrap();
    assert!(matches!(
        t.time_axis_mjd(&invalid_unit, 1, &[1.0]),
        Err(FitsError::InvalidValue { .. })
    ));

    let mut coupled = Header::new();
    coupled
        .set_internal("NAXIS", 2)
        .set_internal("MJDREF", 58000.0)
        .set_internal("TIMEUNIT", "s");
    coupled
        .set_internal("CTYPE1", "TIME")
        .set_internal("CTYPE2", "LINEAR");
    coupled
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRPIX2", 1.0)
        .set_internal("CRVAL1", 10.0);
    coupled
        .set_internal("CDELT1", 2.0)
        .set_internal("CDELT2", 1.0);
    coupled
        .set_internal("PC1_1", 1.0)
        .set_internal("PC1_2", 0.5)
        .set_internal("PC2_1", 0.0)
        .set_internal("PC2_2", 1.0);
    let wcs = coupled.wcs(None).unwrap();
    // Row 1 is CDELT1 × PC1_j = [2, 1]. At pixel [3, 5], offsets [2, 4]
    // contribute 2×2 + 1×4 = 8 s, then CRVAL1 adds 10 s.
    let coordinate = t.time_axis_mjd(&wcs, 1, &[3.0, 5.0]).unwrap().unwrap();
    assert_eq!(coordinate.scale, TimeScale::Utc);
    assert!((coordinate.mjd - (58000.0 + 18.0 / SEC_PER_DAY)).abs() < 1e-12);

    h.set_internal("CTYPE1A", "TIME-LOG")
        .set_internal("CUNIT1A", "d")
        .set_internal("CRVAL1A", 10.0)
        .set_internal("CD1_1A", 2.0);
    let logarithmic = h.wcs(Some('A')).unwrap();
    let coordinate = t.time_axis_mjd(&logarithmic, 1, &[2.0]).unwrap().unwrap();
    let expected_days = 10.0 * 0.2_f64.exp();
    assert!((coordinate.mjd - (58000.0 + expected_days)).abs() < 1e-12);

    let mut non_time = Header::new();
    non_time
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "LINEAR");
    assert!(
        t.time_axis_mjd(&non_time.wcs(None).unwrap(), 1, &[1.0])
            .unwrap()
            .is_none()
    );

    h.set_internal("CTYPE1A", "TIME-TAB")
        .set_internal("CUNIT1A", "d");
    let unsupported = h.wcs(Some('A')).unwrap();
    assert!(matches!(
        t.time_axis_mjd(&unsupported, 1, &[1.0]),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == vec![0]
    ));
    assert!(matches!(
        t.time_axis_mjd(&unsupported, 0, &[1.0]),
        Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" })
    ));
    assert!(matches!(
        t.time_axis_mjd(&unsupported, 2, &[1.0]),
        Err(FitsError::WcsAxisIndexOutOfBounds { axis: 2, len: 1 })
    ));
}

#[test]
fn leap_seconds_match_iers_table() {
    let at = |y, m, d| {
        leap_seconds(
            Datetime::parse(&format!("{y}-{m:02}-{d:02}"))
                .unwrap()
                .to_mjd(TimeScale::Utc)
                .unwrap(),
        )
    };
    assert_eq!(at(1972, 1, 1), 10.0);
    assert_eq!(at(1999, 1, 1), 32.0);
    assert_eq!(at(2017, 1, 1), 37.0);
    assert_eq!(at(2024, 6, 1), 37.0);
    assert_eq!(at(1980, 1, 1), 19.0);
    // Just before the 1999 step is still 31 s.
    assert_eq!(at(1998, 12, 31), 31.0);
}

#[test]
fn fits_time_resolves_reference_and_relative_times() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set_internal("TIMESYS", "TT");
    h.set_internal("MJDREF", 58000.0);
    h.set_internal("TIMEUNIT", "s");
    h.set_internal("TREFPOS", "TOPOCENTER");
    h.set_internal("TSTART", 0.0);
    h.set_internal("TSTOP", 86400.0); // one day, in seconds
    h.set_internal("DATE-OBS", "2017-09-04T00:00:00");

    let t = FitsTime::from_header(&h).unwrap();
    assert_eq!(t.scale, TimeScale::Tt);
    assert_eq!(t.mjdref, 58000.0);
    assert_eq!(t.trefpos, TimeReferencePosition::Topocenter);
    assert_eq!(t.unit_seconds().unwrap(), 1.0);
    // TSTART=0 → MJDREF; TSTOP=86400 s → one day later.
    assert!((t.relative_to_mjd(0.0).unwrap() - 58000.0).abs() < 1e-12);
    assert!((t.relative_to_mjd(86400.0).unwrap() - 58001.0).abs() < 1e-12);
    // DATE-OBS 2017-09-04 = MJD 58000.0.
    assert!((FitsTime::obs_mjd(&h).unwrap().unwrap() - 58000.0).abs() < 1e-9);

    let mut malformed = h.clone();
    malformed.set_internal("TIMEOFFS", "not a real");
    assert!(matches!(
        malformed.time(),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "TIMEOFFS" && expected == "real"
    ));

    let mut positions = Header::new();
    assert_eq!(
        positions.time().unwrap().trefpos,
        TimeReferencePosition::Topocenter
    );
    positions
        .set_internal("TREFPOS", "BARYCENT")
        .set_internal("TRPOS4", "GEOCENTR");
    assert_eq!(
        positions.time().unwrap().trefpos,
        TimeReferencePosition::Barycenter
    );
    assert_eq!(
        positions.time_for_column(4).unwrap().trefpos,
        TimeReferencePosition::Geocenter
    );
    assert!(matches!(
        positions.time_for_column(0),
        Err(FitsError::OneBasedIndexRequired {
            kind: "table column"
        })
    ));
    assert!(matches!(
        positions.phase_axis(0, None),
        Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" })
    ));
    assert!(matches!(
        positions.phase_axis_pixel_list(0, None),
        Err(FitsError::OneBasedIndexRequired {
            kind: "table column"
        })
    ));
    assert!(matches!(
        positions.phase_axis_array_column(0, 1, None),
        Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" })
    ));
    assert!(matches!(
        positions.phase_axis_array_column(1, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            kind: "table column"
        })
    ));
    for (value, expected) in [
        ("TOP", TimeReferencePosition::Topocenter),
        ("GEOCENTER", TimeReferencePosition::Geocenter),
        ("BARYCENTER", TimeReferencePosition::Barycenter),
        ("RELOCATABLE", TimeReferencePosition::Relocatable),
        ("CUSTOM", TimeReferencePosition::Custom),
        ("HELIOCENTER", TimeReferencePosition::Heliocenter),
        ("GALACTIC", TimeReferencePosition::GalacticCenter),
        ("EMBARYCENTER", TimeReferencePosition::EarthMoonBarycenter),
        ("MERCURY", TimeReferencePosition::Mercury),
        ("VENUS", TimeReferencePosition::Venus),
        ("MARS", TimeReferencePosition::Mars),
        ("JUPITER", TimeReferencePosition::Jupiter),
        ("SATURN", TimeReferencePosition::Saturn),
        ("URANUS", TimeReferencePosition::Uranus),
        ("NEPTUNE", TimeReferencePosition::Neptune),
    ] {
        positions.set_internal("TREFPOS", value);
        assert_eq!(positions.time().unwrap().trefpos, expected, "{value}");
    }
    positions.set_internal("TREFPOS", "topocenter");
    assert_eq!(
        positions.time().unwrap().trefpos,
        TimeReferencePosition::Other("topocenter".to_string())
    );

    positions
        .set_internal("TREFPOS", 42)
        .set_internal("TRPOS4", "GEOCENTR");
    assert_eq!(
        positions.time_for_column(4).unwrap().trefpos,
        TimeReferencePosition::Geocenter
    );
    assert!(matches!(
        positions.time(),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "TREFPOS" && expected == "text"
    ));
}

#[test]
fn fits_time_reads_split_and_day_unit_references() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set_internal("MJDREFI", 58000.0);
    h.set_internal("MJDREFF", 0.25);
    h.set_internal("TIMEUNIT", "d");
    let t = FitsTime::from_header(&h).unwrap();
    assert_eq!(t.scale, TimeScale::Utc); // default
    assert!((t.mjdref - 58000.25).abs() < 1e-12);
    assert_eq!(t.unit_seconds().unwrap(), 86400.0);
    // 2 days past the reference.
    assert!((t.relative_to_mjd(2.0).unwrap() - 58002.25).abs() < 1e-12);
}

#[test]
fn time_scale_parse_strips_realization_and_aliases() {
    // §9.2.1: a parenthesised realization suffix is stripped before matching.
    for (text, expected) in [
        ("TT(TAI)", TimeScale::Tt),
        ("UTC(NIST)", TimeScale::Utc),
        ("tt", TimeScale::Tt),
        ("TDT", TimeScale::Tt),
        ("IAT", TimeScale::Tai),
        ("GMT", TimeScale::Utc),
        ("UT1", TimeScale::Ut1),
        ("TAI", TimeScale::Tai),
        ("TCG", TimeScale::Tcg),
        ("TDB", TimeScale::Tdb),
        ("TCB", TimeScale::Tcb),
        ("GPS", TimeScale::Gps),
        ("LOCAL", TimeScale::Local),
    ] {
        assert_eq!(text.parse::<TimeScale>().unwrap(), expected, "{text}");
    }
    assert!(matches!(
        "BOGUS".parse::<TimeScale>(),
        Err(FitsError::InvalidValue { card }) if card == "time scale 'BOGUS'"
    ));
    for malformed in ["LOCAL(clock)", "UTC(", "UTC()", "UTC(NIST))"] {
        assert!(
            matches!(
                malformed.parse::<TimeScale>(),
                Err(FitsError::InvalidValue { .. })
            ),
            "{malformed}"
        );
    }

    let mut unknown = crate::header::Header::new();
    unknown.set_internal("TIMESYS", "BOGUS");
    assert!(matches!(
        unknown.time(),
        Err(FitsError::InvalidValue { card }) if card == "time scale 'BOGUS'"
    ));
}

#[test]
fn timeoffs_shifts_relative_times() {
    use crate::header::Header;
    // MJDREF=58000, TIMEUNIT=s, TIMEOFFS=10 s: the offset is added before scaling,
    // so a relative value of 0 lands 10 s past the reference (§9.4.1).
    let mut h = Header::new();
    h.set_internal("MJDREF", 58000.0);
    h.set_internal("TIMEUNIT", "s");
    h.set_internal("TIMEOFFS", 10.0);
    let t = FitsTime::from_header(&h).unwrap();
    assert_eq!(t.timeoffs, 10.0);
    assert!((t.relative_to_mjd(0.0).unwrap() - (58000.0 + 10.0 / 86400.0)).abs() < 1e-12);
    assert!((t.relative_to_mjd(5.0).unwrap() - (58000.0 + 15.0 / 86400.0)).abs() < 1e-12);
}

#[test]
fn time_units_parse_prefixes_and_epoch_dependent_years() {
    use crate::header::Header;
    let unit = |u: &str| {
        let mut h = Header::new();
        h.set_internal("TIMEUNIT", u);
        FitsTime::from_header(&h).unwrap().unit_seconds().unwrap()
    };
    assert_eq!(unit("min"), 60.0);
    assert_eq!(unit("h"), 3600.0);
    assert_eq!(unit("d"), 86400.0);
    assert_eq!(unit("a"), 365.25 * 86400.0); // Julian year
    assert_eq!(unit("cy"), 36525.0 * 86400.0); // Julian century
    assert_eq!(unit("s"), 1.0);
    assert_eq!(unit("ms"), 1e-3);
    assert_eq!(unit("ks"), 1e3);
    assert_eq!(unit("Mmin"), 60e6);
    assert_eq!(unit("10**3 s"), 1e3);

    let mut tropical = Header::new();
    tropical
        .set_internal("TIMESYS", "TDB")
        .set_internal("MJDREF", 51544.5)
        .set_internal("TIMEUNIT", "ta");
    let tropical = FitsTime::from_header(&tropical).unwrap();
    assert!((tropical.unit_seconds().unwrap() / SEC_PER_DAY - 365.242_190_402_112_4).abs() < 1e-12);

    let mut besselian = Header::new();
    besselian
        .set_internal("TIMESYS", "TT")
        .set_internal("MJDREF", 15019.5)
        .set_internal("TIMEUNIT", "Ba");
    let besselian = FitsTime::from_header(&besselian).unwrap();
    assert!((besselian.unit_seconds().unwrap() / SEC_PER_DAY - 365.242_198_781_7).abs() < 1e-12);

    let mut invalid = Header::new();
    for value in ["", "m", "Hz", "day", "bogus"] {
        invalid.set_internal("TIMEUNIT", value);
        assert!(
            matches!(
                FitsTime::from_header(&invalid),
                Err(FitsError::InvalidValue { .. })
            ),
            "{value:?} should not be accepted as a time unit"
        );
    }
}

#[test]
fn prefixed_relative_time_uses_the_declared_scale() {
    use crate::header::Header;
    let mut milliseconds = Header::new();
    milliseconds
        .set_internal("MJDREF", 58000.0)
        .set_internal("TIMEUNIT", "ms");
    let milliseconds = FitsTime::from_header(&milliseconds).unwrap();
    assert!(
        (milliseconds.relative_to_mjd(1000.0).unwrap() - (58000.0 + 1.0 / SEC_PER_DAY)).abs()
            < 1e-12
    );

    let mut kiloseconds = Header::new();
    kiloseconds
        .set_internal("MJDREF", 58000.0)
        .set_internal("TIMEUNIT", "ks");
    let kiloseconds = FitsTime::from_header(&kiloseconds).unwrap();
    // 86.4 ks = 86,400 s = one day.
    assert!((kiloseconds.relative_to_mjd(86.4).unwrap() - 58001.0).abs() < 1e-12);
}

#[test]
fn split_reference_takes_precedence_over_single_mjdref() {
    use crate::header::Header;
    let mjdref = |pairs: &[(&str, f64)]| {
        let mut h = Header::new();
        for &(k, v) in pairs {
            h.set_internal(k, v);
        }
        FitsTime::from_header(&h).unwrap().mjdref
    };
    // §9.2.2: a full integer+fractional split wins over the single value.
    assert!(
        (mjdref(&[("MJDREF", 58000.0), ("MJDREFI", 59000.0), ("MJDREFF", 0.5)]) - 59000.5).abs()
            < 1e-9
    );
    // Single value alone is used as-is.
    assert!((mjdref(&[("MJDREF", 58000.0)]) - 58000.0).abs() < 1e-9);
    // An incomplete split (integer part only) defers to the single value.
    assert!((mjdref(&[("MJDREF", 58000.0), ("MJDREFI", 59000.0)]) - 58000.0).abs() < 1e-9);
}
