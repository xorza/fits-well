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
        let back = Datetime::from_jd(d.to_jd(TimeScale::Utc).unwrap(), TimeScale::Utc);
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
    header.set("DATE-OBS", "2024-13-01");
    assert!(matches!(
        header.obs_mjd(),
        Err(FitsError::InvalidValue { card }) if card == "DATE '2024-13-01'"
    ));
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
        let round_trip = Datetime::from_jd(datetime.to_jd(TimeScale::Tt).unwrap(), TimeScale::Tt);
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
        let round_trip = Datetime::from_jd(expected_jd, TimeScale::Tt);
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
        let converted = TimeScale::Utc.convert(utc, TimeScale::Tai);
        assert!(
            (converted - erfa_tai).abs() < 5e-10,
            "{text}: TAI JD {converted:.12}"
        );
        tai.push(converted);
    }
    assert!(((tai[1] - tai[0]) * SEC_PER_DAY - 1.0).abs() < 1e-4);
    assert!(((tai[2] - tai[1]) * SEC_PER_DAY - 1.0).abs() < 1e-4);

    let leap = Datetime::from_jd(cases[1].1, TimeScale::Utc);
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
        .set("TIMESYS", "UTC")
        .set("DATEREF", "2016-12-31T23:59:60")
        .set("DATE-OBS", "2016-12-31T23:59:60")
        .set("DATE-END", "2016-12-31T23:59:60");
    assert!((header.time().unwrap().mjdref - erfa_mjd).abs() < 5e-10);
    assert!((header.obs_mjd().unwrap().unwrap() - erfa_mjd).abs() < 5e-10);
    assert!((header.time_bounds().unwrap().end_mjd.unwrap() - erfa_mjd).abs() < 5e-10);

    header.set("TIMESYS", "TAI");
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
    hj.set("JEPOCH", 2000.0);
    let ej = FitsTime::epoch(&hj).unwrap().unwrap();
    assert!((ej.mjd - 51544.5).abs() < 1e-6);
    assert_eq!(ej.scale, TimeScale::Tdb);
    // BEPOCH=1950.0 ⇒ B1950.0 = MJD 33281.92345905, implied scale ET ≈ TT.
    let mut hb = Header::new();
    hb.set("BEPOCH", 1950.0);
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
    h.set("MJD-BEG", 58000.0);
    h.set("DATE-END", "2017-09-05T00:00:00");
    h.set("MJD-AVG", 58000.5);
    h.set("XPOSURE", 1200.0);
    h.set("TELAPSE", 1500.0);
    h.set("TIMEDEL", 0.1);
    h.set("TIMSYER", 1e-6);
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

    h.set("TIMESYS", 1).set("MJD-END", 58001.0);
    assert_eq!(FitsTime::bounds(&h).unwrap().end_mjd, Some(58001.0));
}

#[test]
fn gti_intervals_convert_to_absolute_mjd() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set("MJDREF", 58000.0);
    h.set("TIMEUNIT", "d");
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
    milliseconds.set("MJDREF", 58000.0).set("TIMEUNIT", "ms");
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
    use crate::header::Header;
    // §9.6: a PHASE axis carries CZPHSia (zero-phase time) and CPERIia (period).
    let mut h = Header::new();
    h.set("CTYPE2", "PHASE");
    h.set("CZPHS2", 5.0);
    h.set("CPERI2", 2.0);
    let pa = FitsTime::phase_axis(&h, 2).unwrap().unwrap();
    assert_eq!(pa.zero_phase, 5.0);
    assert_eq!(pa.period, 2.0);
    // Fold: ((8 − 5)/2) mod 1 = 1.5 mod 1 = 0.5; the zero-phase time folds to 0.
    assert_eq!(pa.fold(8.0), 0.5);
    assert_eq!(pa.fold(5.0), 0.0);
    // A non-phase axis yields nothing.
    h.set("CTYPE1", "RA---TAN");
    assert_eq!(FitsTime::phase_axis(&h, 1).unwrap(), None);
}

#[test]
fn obs_mjd_falls_back_to_jepoch() {
    use crate::header::Header;
    // §9.5: absent DATE-OBS/MJD-OBS, JEPOCH stands in for the observation time.
    let mut h = Header::new();
    h.set("JEPOCH", 2000.0); // J2000.0 = MJD 51544.5
    assert!((FitsTime::obs_mjd(&h).unwrap().unwrap() - 51544.5).abs() < 1e-6);
    // An explicit MJD-OBS still wins.
    h.set("MJD-OBS", 58000.0);
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
        let got_frac = TimeScale::Utc.convert(utc_jd, scale) - MJD0 - BASE;
        assert!(
            (got_frac - want_frac).abs() < 1e-9,
            "UTC→{scale:?}: {got_frac:.12} vs {want_frac:.12} (Δ={:.2e} s)",
            (got_frac - want_frac) * 86400.0
        );
        // Round-trip back to UTC.
        let back = scale.convert(BASE + want_frac + MJD0, TimeScale::Utc) - MJD0;
        assert!(
            (back - BASE).abs() < 1e-9,
            "{scale:?}→UTC round-trip: {back}"
        );
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
    let utc = TimeScale::Tai.convert(tai, TimeScale::Utc);
    assert!((utc - expected_utc).abs() * SEC_PER_DAY < 1e-5);

    let dut1 = 0.25;
    let ut1 = TimeScale::Tai.convert_dut1(tai, TimeScale::Ut1, dut1);
    let expected_ut1 = utc_quasi_to_linear(expected_utc) + dut1 / SEC_PER_DAY;
    assert!((ut1 - expected_ut1).abs() * SEC_PER_DAY < 1e-5);
}

#[test]
fn ut1_uses_explicit_dut1() {
    const MJD0: f64 = 2_400_000.5;
    let utc_jd = 60462.0 + MJD0;
    let dut1 = -0.020434661; // astropy ΔUT1 = UT1 − UTC at 2024-06-01
    let ut1 = TimeScale::Utc.convert_dut1(utc_jd, TimeScale::Ut1, dut1) - MJD0;
    // astropy UT1 MJD, as the day-fraction beyond 60462 (UT1 − 60462).
    let want = -0.000000236512;
    assert!(
        (ut1 - 60462.0 - want).abs() < 1e-9,
        "UT1 {ut1:.12} (Δ={:.4e} s)",
        (ut1 - 60462.0 - want) * 86400.0
    );
    // Round-trip back to UTC.
    let back = TimeScale::Ut1.convert_dut1(ut1 + MJD0, TimeScale::Utc, dut1) - MJD0;
    assert!((back - 60462.0).abs() < 1e-9);
    // With ΔUT1 = 0, UT1 collapses to UTC (the `convert` default).
    assert_eq!(TimeScale::Utc.convert(utc_jd, TimeScale::Ut1), utc_jd);
}

#[test]
fn time_axis_uses_complete_wcs_row_unit_and_scale() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set("NAXIS", 1).set("MJDREF", 58000.0);
    h.set("TIMESYS", "UTC").set("TIMEUNIT", "s");
    h.set("CTYPE1A", "TAI").set("CUNIT1A", "d");
    h.set("CRPIX1A", 1.0).set("CRVAL1A", 0.0).set("CD1_1A", 2.0);
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

    h.set("CUNIT1A", "ms");
    let milliseconds = h.wcs(Some('A')).unwrap();
    // With the same CD row, an offset of 500 pixels is 1000 ms = 1 s.
    let coordinate = t
        .time_axis_mjd(&milliseconds, 1, &[501.0])
        .unwrap()
        .unwrap();
    assert!((coordinate.mjd - (58000.0 + 1.0 / SEC_PER_DAY)).abs() < 1e-12);

    h.set("CUNIT1A", "Hz");
    let invalid_unit = h.wcs(Some('A')).unwrap();
    assert!(matches!(
        t.time_axis_mjd(&invalid_unit, 1, &[1.0]),
        Err(FitsError::InvalidValue { .. })
    ));

    let mut coupled = Header::new();
    coupled
        .set("NAXIS", 2)
        .set("MJDREF", 58000.0)
        .set("TIMEUNIT", "s");
    coupled.set("CTYPE1", "TIME").set("CTYPE2", "LINEAR");
    coupled
        .set("CRPIX1", 1.0)
        .set("CRPIX2", 1.0)
        .set("CRVAL1", 10.0);
    coupled.set("CDELT1", 2.0).set("CDELT2", 1.0);
    coupled
        .set("PC1_1", 1.0)
        .set("PC1_2", 0.5)
        .set("PC2_1", 0.0)
        .set("PC2_2", 1.0);
    let wcs = coupled.wcs(None).unwrap();
    // Row 1 is CDELT1 × PC1_j = [2, 1]. At pixel [3, 5], offsets [2, 4]
    // contribute 2×2 + 1×4 = 8 s, then CRVAL1 adds 10 s.
    let coordinate = t.time_axis_mjd(&wcs, 1, &[3.0, 5.0]).unwrap().unwrap();
    assert_eq!(coordinate.scale, TimeScale::Utc);
    assert!((coordinate.mjd - (58000.0 + 18.0 / SEC_PER_DAY)).abs() < 1e-12);

    h.set("CTYPE1A", "TIME-LOG")
        .set("CUNIT1A", "d")
        .set("CRVAL1A", 10.0)
        .set("CD1_1A", 2.0);
    let logarithmic = h.wcs(Some('A')).unwrap();
    let coordinate = t.time_axis_mjd(&logarithmic, 1, &[2.0]).unwrap().unwrap();
    let expected_days = 10.0 * 0.2_f64.exp();
    assert!((coordinate.mjd - (58000.0 + expected_days)).abs() < 1e-12);

    let mut non_time = Header::new();
    non_time.set("NAXIS", 1).set("CTYPE1", "LINEAR");
    assert!(
        t.time_axis_mjd(&non_time.wcs(None).unwrap(), 1, &[1.0])
            .unwrap()
            .is_none()
    );

    h.set("CTYPE1A", "TIME-TAB").set("CUNIT1A", "d");
    let unsupported = h.wcs(Some('A')).unwrap();
    assert!(matches!(
        t.time_axis_mjd(&unsupported, 1, &[1.0]),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == vec![0]
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
    h.set("TIMESYS", "TT");
    h.set("MJDREF", 58000.0);
    h.set("TIMEUNIT", "s");
    h.set("TREFPOS", "TOPOCENTER");
    h.set("TSTART", 0.0);
    h.set("TSTOP", 86400.0); // one day, in seconds
    h.set("DATE-OBS", "2017-09-04T00:00:00");

    let t = FitsTime::from_header(&h).unwrap();
    assert_eq!(t.scale, TimeScale::Tt);
    assert_eq!(t.mjdref, 58000.0);
    assert_eq!(t.trefpos.as_deref(), Some("TOPOCENTER"));
    assert_eq!(t.unit_seconds().unwrap(), 1.0);
    // TSTART=0 → MJDREF; TSTOP=86400 s → one day later.
    assert!((t.relative_to_mjd(0.0).unwrap() - 58000.0).abs() < 1e-12);
    assert!((t.relative_to_mjd(86400.0).unwrap() - 58001.0).abs() < 1e-12);
    // DATE-OBS 2017-09-04 = MJD 58000.0.
    assert!((FitsTime::obs_mjd(&h).unwrap().unwrap() - 58000.0).abs() < 1e-9);

    let mut malformed = h.clone();
    malformed.set("TIMEOFFS", "not a real");
    assert!(matches!(
        malformed.time(),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "TIMEOFFS" && expected == "real"
    ));
}

#[test]
fn fits_time_reads_split_and_day_unit_references() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set("MJDREFI", 58000.0);
    h.set("MJDREFF", 0.25);
    h.set("TIMEUNIT", "d");
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
    assert_eq!(TimeScale::parse("TT(TAI)"), TimeScale::Tt);
    assert_eq!(TimeScale::parse("UTC(NIST)"), TimeScale::Utc);
    assert_eq!(TimeScale::parse("tt"), TimeScale::Tt);
    assert_eq!(TimeScale::parse("TDT"), TimeScale::Tt); // alias
    assert_eq!(TimeScale::parse("IAT"), TimeScale::Tai); // alias
    assert_eq!(TimeScale::parse("GMT"), TimeScale::Utc); // §9.2.1: GMT ≡ UTC
    assert_eq!(TimeScale::parse("BOGUS"), TimeScale::Local);
}

#[test]
fn timeoffs_shifts_relative_times() {
    use crate::header::Header;
    // MJDREF=58000, TIMEUNIT=s, TIMEOFFS=10 s: the offset is added before scaling,
    // so a relative value of 0 lands 10 s past the reference (§9.4.1).
    let mut h = Header::new();
    h.set("MJDREF", 58000.0);
    h.set("TIMEUNIT", "s");
    h.set("TIMEOFFS", 10.0);
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
        h.set("TIMEUNIT", u);
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
        .set("TIMESYS", "TDB")
        .set("MJDREF", 51544.5)
        .set("TIMEUNIT", "ta");
    let tropical = FitsTime::from_header(&tropical).unwrap();
    assert!((tropical.unit_seconds().unwrap() / SEC_PER_DAY - 365.242_190_402_112_4).abs() < 1e-12);

    let mut besselian = Header::new();
    besselian
        .set("TIMESYS", "TT")
        .set("MJDREF", 15019.5)
        .set("TIMEUNIT", "Ba");
    let besselian = FitsTime::from_header(&besselian).unwrap();
    assert!((besselian.unit_seconds().unwrap() / SEC_PER_DAY - 365.242_198_781_7).abs() < 1e-12);

    let mut invalid = Header::new();
    for value in ["", "m", "Hz", "day", "bogus"] {
        invalid.set("TIMEUNIT", value);
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
    milliseconds.set("MJDREF", 58000.0).set("TIMEUNIT", "ms");
    let milliseconds = FitsTime::from_header(&milliseconds).unwrap();
    assert!(
        (milliseconds.relative_to_mjd(1000.0).unwrap() - (58000.0 + 1.0 / SEC_PER_DAY)).abs()
            < 1e-12
    );

    let mut kiloseconds = Header::new();
    kiloseconds.set("MJDREF", 58000.0).set("TIMEUNIT", "ks");
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
            h.set(k, v);
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
