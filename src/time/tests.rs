use super::*;

/// Golden values throughout are from `astropy.time` (ERFA).
#[test]
fn iso_to_jd_and_mjd_match_astropy() {
    let cases: &[(&str, f64, f64)] = &[
        ("2000-01-01T12:00:00", 2451545.0, 51544.5),
        ("1858-11-17T00:00:00", 2400000.5, 0.0),
        ("2024-02-29T06:30:15.5", 2460369.771012731, 60369.271012731),
        ("1900-01-01T00:00:00", 2415020.5, 15020.0),
        // A leap-second label rolls over to the next day (matches astropy).
        ("1999-12-31T23:59:60", 2451544.5, 51544.0),
        ("2024-06-01", 2460462.5, 60462.0), // date-only ⇒ midnight
    ];
    for &(s, jd, mjd) in cases {
        let d = Datetime::parse(s).unwrap();
        assert!(
            (d.to_jd() - jd).abs() < 1e-7,
            "{s}: jd {} vs {jd}",
            d.to_jd()
        );
        assert!(
            (d.to_mjd() - mjd).abs() < 1e-7,
            "{s}: mjd {} vs {mjd}",
            d.to_mjd()
        );
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
        let back = Datetime::from_jd(d.to_jd());
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
        "x",
    ] {
        assert!(Datetime::parse(s).is_err(), "{s:?} should be rejected");
    }
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
fn time_axis_resolves_to_mjd() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set("MJDREF", 58000.0);
    h.set("TIMESYS", "TT");
    h.set("TIMEUNIT", "s");
    h.set("CTYPE3", "TIME");
    h.set("CRPIX3", 1.0).set("CRVAL3", 0.0).set("CDELT3", 10.0); // 10 s / pixel
    let t = FitsTime::from_header(&h);
    // Pixel 1 → 0 s → MJDREF; pixel 11 → 100 s later.
    assert!((t.time_axis_mjd(&h, 3, 1.0).unwrap() - 58000.0).abs() < 1e-12);
    assert!((t.time_axis_mjd(&h, 3, 11.0).unwrap() - (58000.0 + 100.0 / 86400.0)).abs() < 1e-12);
    // A non-time axis returns None.
    h.set("CTYPE1", "RA---TAN");
    assert!(t.time_axis_mjd(&h, 1, 1.0).is_none());
}

#[test]
fn leap_seconds_match_iers_table() {
    let at = |y, m, d| {
        leap_seconds(
            Datetime::parse(&format!("{y}-{m:02}-{d:02}"))
                .unwrap()
                .to_mjd(),
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

    let t = FitsTime::from_header(&h);
    assert_eq!(t.scale, TimeScale::Tt);
    assert_eq!(t.mjdref, 58000.0);
    assert_eq!(t.trefpos.as_deref(), Some("TOPOCENTER"));
    assert_eq!(t.unit_seconds(), 1.0);
    // TSTART=0 → MJDREF; TSTOP=86400 s → one day later.
    assert!((t.relative_to_mjd(0.0) - 58000.0).abs() < 1e-12);
    assert!((t.relative_to_mjd(86400.0) - 58001.0).abs() < 1e-12);
    // DATE-OBS 2017-09-04 = MJD 58000.0.
    assert!((t.obs_mjd(&h).unwrap() - 58000.0).abs() < 1e-9);
}

#[test]
fn fits_time_reads_split_and_day_unit_references() {
    use crate::header::Header;
    let mut h = Header::new();
    h.set("MJDREFI", 58000.0);
    h.set("MJDREFF", 0.25);
    h.set("TIMEUNIT", "d");
    let t = FitsTime::from_header(&h);
    assert_eq!(t.scale, TimeScale::Utc); // default
    assert!((t.mjdref - 58000.25).abs() < 1e-12);
    assert_eq!(t.unit_seconds(), 86400.0);
    // 2 days past the reference.
    assert!((t.relative_to_mjd(2.0) - 58002.25).abs() < 1e-12);
}
