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
        let got_jd = d.to_jd(&TimeScale::Utc).unwrap();
        let got_mjd = d.to_mjd(&TimeScale::Utc).unwrap();
        assert!((got_jd - jd).abs() < 1e-7, "{s}: jd {got_jd} vs {jd}",);
        assert!((got_mjd - mjd).abs() < 1e-7, "{s}: mjd {got_mjd} vs {mjd}",);
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
        assert!(datetime.to_jd(&TimeScale::Tt).unwrap().is_finite());
    }
    let mut outside_fits_range = Datetime::parse("+99999-12-31").unwrap();
    outside_fits_range.year = 100000;
    assert!(outside_fits_range.to_jd(&TimeScale::Tt).is_err());
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
        assert_eq!(date.to_jd(&TimeScale::Tt).unwrap(), expected_jd, "{text}");
    }
}

#[test]
fn leap_second_labels_require_utc_and_external_time_data() {
    for (text, scale) in [
        ("2016-12-31T12:00:60", TimeScale::Utc),
        ("2016-12-31T23:59:60", TimeScale::Tai),
    ] {
        let datetime = Datetime::parse(text).unwrap();
        assert!(matches!(
            datetime.to_jd(&scale),
            Err(FitsError::InvalidValue { .. })
        ));
    }

    let leap = Datetime::parse("2030-06-30T23:59:60").unwrap();
    assert!(matches!(
        leap.to_jd(&TimeScale::Utc),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
}

#[test]
fn header_datetimes_use_the_declared_scale() {
    use crate::header::Header;

    let mut header = Header::new();
    header
        .set_internal("TIMESYS", "MET")
        .set_internal("DATEREF", "2017-01-01T00:00:00")
        .set_internal("DATE-OBS", "2017-01-01T00:00:00")
        .set_internal("DATE-END", "2017-01-02T00:00:00");
    let time = header.time().unwrap();
    assert_eq!(time.scale, TimeScale::Local("MET".to_string()));
    assert_eq!(time.mjdref, 57_754.0);
    assert_eq!(header.obs_mjd().unwrap(), Some(57_754.0));
    assert_eq!(header.time_bounds().unwrap().end_mjd, Some(57_755.0));

    header
        .set_internal("TIMESYS", "UTC")
        .set_internal("DATEREF", "2016-12-31T23:59:60")
        .set_internal("DATE-OBS", "2016-12-31T23:59:60")
        .set_internal("DATE-END", "2016-12-31T23:59:60");
    assert!(matches!(
        header.time(),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
    assert!(matches!(
        header.obs_mjd(),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
    assert!(matches!(
        header.time_bounds(),
        Err(FitsError::ExternalTimeDataRequired { .. })
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
        .to_mjd(&TimeScale::Utc)
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
fn reads_phase_axis_metadata() {
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
    h.set_internal("CPERI4", 0.0);
    assert_eq!(h.phase_axis(4, None).unwrap().unwrap().period, None);

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
fn numeric_epochs_match_astropy() {
    let cases: &[(Epoch, f64)] = &[
        (Epoch::Julian(2000.0), 2451545.0),
        (Epoch::Besselian(1950.0), 2433282.42345905),
        (Epoch::Julian(2015.5), 2457206.375),
        (Epoch::Besselian(1900.0), 2415020.31352),
    ];
    for &(epoch, jd) in cases {
        assert!(
            (epoch.to_jd() - jd).abs() < 1e-5,
            "{epoch:?}: {} vs {jd}",
            epoch.to_jd()
        );
    }
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
fn time_scale_preserves_realizations_and_local_names() {
    for (text, expected) in [
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
    ] {
        assert_eq!(text.parse::<TimeScale>().unwrap(), expected, "{text}");
    }
    assert_eq!(
        "TT(TAI)".parse::<TimeScale>().unwrap(),
        TimeScale::Realized {
            kind: TimeScaleKind::Tt,
            realization: "TAI".to_string(),
        }
    );
    assert_eq!(
        "UTC(NIST)".parse::<TimeScale>().unwrap(),
        TimeScale::Realized {
            kind: TimeScaleKind::Utc,
            realization: "NIST".to_string(),
        }
    );
    for name in ["LOCAL", "MET", "OET", "BOGUS"] {
        assert_eq!(
            name.parse::<TimeScale>().unwrap(),
            TimeScale::Local(name.to_string())
        );
    }
    assert_eq!(
        "LOCAL(clock)".parse::<TimeScale>().unwrap(),
        TimeScale::Local("LOCAL(clock)".to_string())
    );
    for malformed in ["UTC(", "UTC()", "UTC(NIST))"] {
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
    assert_eq!(
        unknown.time().unwrap().scale,
        TimeScale::Local("BOGUS".to_string())
    );
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
                FitsTime::from_header(&invalid).unwrap().unit_seconds(),
                Err(FitsError::InvalidValue { .. })
            ),
            "{value:?} should not be accepted as a time unit"
        );
    }

    let mut frame_dependent = Header::new();
    for (unit, scale) in [("ta", "UTC"), ("Ba", "TDB")] {
        frame_dependent
            .set_internal("TIMEUNIT", unit)
            .set_internal("TIMESYS", scale);
        let time = FitsTime::from_header(&frame_dependent).unwrap();
        assert!(matches!(
            time.unit_seconds(),
            Err(FitsError::ExternalTimeDataRequired { .. })
        ));
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
