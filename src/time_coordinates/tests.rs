use crate::error::FitsError;
use crate::error::Indexed;
use crate::header_model::{Header, keywords};

use crate::time_coordinates::SEC_PER_DAY;
use crate::time_coordinates::bounds::Bounds;
use crate::time_coordinates::coordinate::{Coordinate, Epoch};
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::phase_axis::PhaseAxis;
use crate::time_coordinates::reference_position::ReferencePosition;
use crate::time_coordinates::scale::{Scale, ScaleKind};
use crate::time_coordinates::time_axis_kind::TimeAxisKind;
use crate::world_coordinates::Wcs;

/// Golden values throughout are from `astropy.time` (ERFA).
#[test]
fn iso_to_jd_and_mjd_match_astropy() {
    // 06:30:15.5 is 23415.5 s into the day. astropy prints 2460369.771012731 and
    // 60369.271012731; each value below is that day's start plus one rounded
    // fraction, the MJD rounding at its own precision — taken through the JD, it
    // would end at 60369.27101273136, nine ulps short.
    let fraction = 23_415.5 / 86_400.0;
    let cases: &[(&str, f64, f64)] = &[
        ("2000-01-01T12:00:00", 2_451_545.0, 51544.5),
        ("1858-11-17T00:00:00", 2_400_000.5, 0.0),
        (
            "2024-02-29T06:30:15.5",
            2_460_369.5 + fraction,
            60369.0 + fraction,
        ),
        ("1900-01-01T00:00:00", 2_415_020.5, 15020.0),
        ("2024-06-01", 2_460_462.5, 60462.0), // date-only ⇒ midnight
    ];
    for &(s, jd, mjd) in cases {
        let d = Datetime::parse(s).unwrap();
        let utc = Scale::known(ScaleKind::Utc);
        assert_eq!(d.to_jd(&utc).unwrap(), jd, "{s}");
        assert_eq!(d.to_mjd(&utc).unwrap(), mjd, "{s}");
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

    let mut header = Header::new();
    header.set_internal(keywords::DATE_OBS, "2024-13-01");
    assert!(matches!(
        Coordinate::observation(&header),
        Err(FitsError::InvalidTime { detail }) if detail == "DATE '2024-13-01'"
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
    // A Gregorian 400-year cycle is 146097 days. 2000-01-01 is JD 2451544.5, so
    // 100000-01-01 is 245 cycles on and 99999-12-31 a day before it; 2001-01-01 is
    // JD 2451910.5, and -99999-01-01 is 255 cycles before it.
    for (text, year, jd) in [
        ("-99999-01-01", -99999, 2_451_910.5 - 255.0 * 146_097.0),
        ("+99999-12-31", 99999, 2_451_544.5 + 245.0 * 146_097.0 - 1.0),
    ] {
        let datetime = Datetime::parse(text).unwrap();
        assert_eq!(datetime.year, year);
        assert_eq!(datetime.to_jd(&Scale::known(ScaleKind::Tt)).unwrap(), jd);
    }
    let mut outside_fits_range = Datetime::parse("+99999-12-31").unwrap();
    outside_fits_range.year = 100_000;
    assert!(
        outside_fits_range
            .to_jd(&Scale::known(ScaleKind::Tt))
            .is_err()
    );
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
        assert_eq!(
            date.to_jd(&Scale::known(ScaleKind::Tt)).unwrap(),
            expected_jd,
            "{text}"
        );
    }
}

#[test]
fn leap_second_labels_require_utc_and_external_time_data() {
    for (text, scale) in [
        ("2016-12-31T12:00:60", Scale::known(ScaleKind::Utc)),
        ("2016-12-31T23:59:60", Scale::known(ScaleKind::Tai)),
    ] {
        let datetime = Datetime::parse(text).unwrap();
        assert!(matches!(
            datetime.to_jd(&scale),
            Err(FitsError::InvalidTime { .. })
        ));
    }

    let leap = Datetime::parse("2030-06-30T23:59:60").unwrap();
    assert!(matches!(
        leap.to_jd(&Scale::known(ScaleKind::Utc)),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
}

#[test]
fn header_datetimes_use_the_declared_scale() {
    let mut header = Header::new();
    header
        .set_internal(keywords::TIMESYS, "MET")
        .set_internal(keywords::DATEREF, "2017-01-01T00:00:00")
        .set_internal(keywords::DATE_OBS, "2017-01-01T00:00:00")
        .set_internal(keywords::DATE_END, "2017-01-02T00:00:00");
    let time = Frame::from_header(&header).unwrap();
    assert_eq!(time.scale, Scale::Local("MET".to_string()));
    assert_eq!(time.mjdref, 57_754.0);
    assert_eq!(
        Coordinate::observation(&header).unwrap(),
        Some(Coordinate {
            mjd: 57_754.0,
            scale: Scale::Local("MET".to_string())
        })
    );
    assert_eq!(
        Bounds::from_header(&header).unwrap().end_mjd,
        Some(57_755.0)
    );

    header
        .set_internal(keywords::TIMESYS, "UTC")
        .set_internal(keywords::DATEREF, "2016-12-31T23:59:60")
        .set_internal(keywords::DATE_OBS, "2016-12-31T23:59:60")
        .set_internal(keywords::DATE_END, "2016-12-31T23:59:60");
    assert!(matches!(
        Frame::from_header(&header),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
    assert!(matches!(
        Coordinate::observation(&header),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
    assert!(matches!(
        Bounds::from_header(&header),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
}

#[test]
fn reads_jepoch_and_bepoch_keywords() {
    // JEPOCH is in its implied TDB, BEPOCH in ET ≈ TT; `numeric_epochs_match_astropy`
    // pins the dates themselves.
    let mut hj = Header::new();
    hj.set_internal(keywords::JEPOCH, 2000.0);
    assert_eq!(
        Coordinate::epoch(&hj).unwrap(),
        Some(Coordinate {
            mjd: Epoch::Julian(2000.0).to_mjd(),
            scale: Scale::known(ScaleKind::Tdb),
        })
    );
    let mut hb = Header::new();
    hb.set_internal(keywords::BEPOCH, 1950.0);
    assert_eq!(
        Coordinate::epoch(&hb).unwrap(),
        Some(Coordinate {
            mjd: Epoch::Besselian(1950.0).to_mjd(),
            scale: Scale::known(ScaleKind::Tt),
        })
    );
    // Neither keyword ⇒ None.
    let empty = Header::new();
    assert!(Coordinate::epoch(&empty).unwrap().is_none());
}

#[test]
fn reads_bound_duration_and_error_keywords() {
    use crate::error::FitsError;
    let mut h = Header::new();
    h.set_internal(keywords::MJD_BEG, 58000.0);
    h.set_internal(keywords::DATE_END, "2017-09-05T00:00:00");
    h.set_internal(keywords::MJD_AVG, 58000.5);
    h.set_internal(keywords::XPOSURE, 1200.0);
    h.set_internal(keywords::TELAPSE, 1500.0);
    h.set_internal(keywords::TIMEDEL, 0.1);
    h.set_internal(keywords::TIMSYER, 1e-6);
    let b = Bounds::from_header(&h).unwrap();
    assert_eq!(b.beg_mjd, Some(58000.0));
    // Resolved from DATE-END: 2017-09-04 is MJD 58000.
    assert_eq!(b.end_mjd, Some(58001.0));
    assert_eq!(b.avg_mjd, Some(58000.5)); // §9.5 midpoint
    assert_eq!(b.xposure, Some(1200.0));
    assert_eq!(b.telapse, Some(1500.0));
    assert_eq!(b.timedel, Some(0.1));
    assert_eq!(b.timepixr, 0.5); // default when absent
    assert_eq!(b.timsyer, Some(1e-6));
    assert_eq!(b.timrder, None);

    h.set_internal(keywords::TIMESYS, 1)
        .set_internal(keywords::MJD_END, 58001.0);
    assert_eq!(Bounds::from_header(&h).unwrap().end_mjd, Some(58001.0));

    for invalid in [-f64::EPSILON, 1.0 + f64::EPSILON] {
        h.set_internal(keywords::TIMEPIXR, invalid);
        assert!(matches!(
            Bounds::from_header(&h),
            Err(FitsError::KeywordOutOfRange {
                keyword: keywords::TIMEPIXR
            })
        ));
    }
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

    let image = PhaseAxis::from_header(&h, 2, None).unwrap().unwrap();
    assert_eq!(image.zero_phase, 5.0);
    assert_eq!(image.period, Some(2.0));

    let alternate = PhaseAxis::from_header(&h, 2, Some('A')).unwrap().unwrap();
    assert_eq!(
        alternate,
        PhaseAxis {
            zero_phase: 7.0,
            period: Some(4.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_pixel_list(&h, 3, None).unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 11.0,
            period: Some(5.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_pixel_list(&h, 3, Some('A'))
            .unwrap()
            .unwrap(),
        PhaseAxis {
            zero_phase: 13.0,
            period: Some(6.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_array_column(&h, 1, 5, None)
            .unwrap()
            .unwrap(),
        PhaseAxis {
            zero_phase: 17.0,
            period: Some(8.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_array_column(&h, 1, 5, Some('A'))
            .unwrap()
            .unwrap(),
        PhaseAxis {
            zero_phase: 19.0,
            period: Some(10.0),
        }
    );

    h.set_internal("CTYPE1", "RA---TAN");
    assert_eq!(PhaseAxis::from_header(&h, 1, None).unwrap(), None);

    h.set_internal("CTYPE4", "PHASE")
        .set_internal("CZPHS4", 23.0);
    let varying = PhaseAxis::from_header(&h, 4, None).unwrap().unwrap();
    assert_eq!(varying.period, None);
    h.set_internal("CPERI4", 0.0);
    assert_eq!(
        PhaseAxis::from_header(&h, 4, None).unwrap().unwrap().period,
        None
    );

    h.set_internal("CTYPE6", "PHASE");
    assert!(matches!(
        PhaseAxis::from_header(&h, 6, None),
        Err(FitsError::InvalidTime { detail }) if detail.contains("CZPHS6")
    ));
}

#[test]
fn observation_falls_back_to_jepoch_in_its_own_scale() {
    // §9.5: absent DATE-OBS/MJD-OBS, JEPOCH stands in for the observation time, in
    // its implied TDB.
    let mut h = Header::new();
    h.set_internal(keywords::JEPOCH, 2000.0);
    assert_eq!(
        Coordinate::observation(&h).unwrap(),
        Some(Coordinate {
            mjd: Epoch::Julian(2000.0).to_mjd(),
            scale: Scale::known(ScaleKind::Tdb)
        })
    );
    // An explicit MJD-OBS still wins, in the declared scale (UTC by default).
    h.set_internal(keywords::MJD_OBS, 58000.0);
    assert_eq!(
        Coordinate::observation(&h).unwrap(),
        Some(Coordinate {
            mjd: 58000.0,
            scale: Scale::known(ScaleKind::Utc)
        })
    );
}

#[test]
fn numeric_epochs_match_astropy() {
    // astropy's JDs less 2400000.5; each is the f64 nearest its decimal.
    let cases: &[(Epoch, f64)] = &[
        (Epoch::Julian(2000.0), 51544.5),
        (Epoch::Besselian(1950.0), 33_281.923_459_05),
        (Epoch::Julian(2015.5), 57205.875),
        (Epoch::Besselian(1900.0), 15019.81352),
    ];
    for &(epoch, mjd) in cases {
        assert_eq!(epoch.to_mjd(), mjd, "{epoch:?}");
    }
}

#[test]
fn time_axis_uses_complete_wcs_row_unit_and_scale() {
    let mut h = Header::new();
    h.set_internal(keywords::NAXIS, 1)
        .set_internal(keywords::MJDREF, 58000.0);
    h.set_internal(keywords::TIMESYS, "UTC")
        .set_internal(keywords::TIMEUNIT, "s");
    h.set_internal("CTYPE1A", "TAI")
        .set_internal("CUNIT1A", "d");
    h.set_internal("CRPIX1A", 1.0)
        .set_internal("CRVAL1A", 0.0)
        .set_internal("CD1_1A", 2.0);
    let t = Frame::from_header(&h).unwrap();
    let alternate = Wcs::from_header(&h, Some('A')).unwrap();
    // CD1_1 = 2 d/pixel and pixel offset 0.5 produce exactly one day. CUNIT1A and
    // CTYPE1A override the global seconds/UTC frame.
    assert_eq!(
        t.time_axis_mjd(&alternate, 1, &[1.5]).unwrap(),
        Some(Coordinate {
            mjd: 58001.0,
            scale: Scale::known(ScaleKind::Tai),
        })
    );

    h.set_internal("CUNIT1A", "ms");
    let milliseconds = Wcs::from_header(&h, Some('A')).unwrap();
    // With the same CD row, an offset of 500 pixels is 1000 ms = 1 s.
    let coordinate = t
        .time_axis_mjd(&milliseconds, 1, &[501.0])
        .unwrap()
        .unwrap();
    assert_eq!(coordinate.mjd, 58000.0 + 1.0 / SEC_PER_DAY);

    h.set_internal("CUNIT1A", "Hz");
    let invalid_unit = Wcs::from_header(&h, Some('A')).unwrap();
    assert!(matches!(
        t.time_axis_mjd(&invalid_unit, 1, &[1.0]),
        Err(FitsError::InvalidUnit {
            expected: "a time unit",
            ..
        })
    ));

    let mut coupled = Header::new();
    coupled
        .set_internal(keywords::NAXIS, 2)
        .set_internal(keywords::MJDREF, 58000.0)
        .set_internal(keywords::TIMEUNIT, "s");
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
    let wcs = Wcs::from_header(&coupled, None).unwrap();
    // Row 1 is CDELT1 × PC1_j = [2, 1]. At pixel [3, 5], offsets [2, 4]
    // contribute 2×2 + 1×4 = 8 s, then CRVAL1 adds 10 s.
    let coordinate = t.time_axis_mjd(&wcs, 1, &[3.0, 5.0]).unwrap().unwrap();
    assert_eq!(coordinate.scale, Scale::known(ScaleKind::Utc));
    assert_eq!(coordinate.mjd, 58000.0 + 18.0 / SEC_PER_DAY);

    h.set_internal("CTYPE1A", "TIME-LOG")
        .set_internal("CUNIT1A", "d")
        .set_internal("CRVAL1A", 10.0)
        .set_internal("CD1_1A", 2.0);
    let logarithmic = Wcs::from_header(&h, Some('A')).unwrap();
    let coordinate = t.time_axis_mjd(&logarithmic, 1, &[2.0]).unwrap().unwrap();
    let expected_days = 10.0 * 0.2_f64.exp();
    assert_eq!(coordinate.mjd, 58000.0 + expected_days);

    let mut non_time = Header::new();
    non_time
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "LINEAR");
    assert!(
        t.time_axis_mjd(&Wcs::from_header(&non_time, None).unwrap(), 1, &[1.0])
            .unwrap()
            .is_none()
    );

    h.set_internal("CTYPE1A", "TIME-TAB")
        .set_internal("CUNIT1A", "d");
    let unsupported = Wcs::from_header(&h, Some('A')).unwrap();
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
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::WcsAxis,
            index: 2,
            len: 1
        })
    ));
}

#[test]
fn fits_time_resolves_reference_and_relative_times() {
    let mut h = Header::new();
    h.set_internal(keywords::TIMESYS, "TT");
    h.set_internal(keywords::MJDREF, 58000.0);
    h.set_internal(keywords::TIMEUNIT, "s");
    h.set_internal(keywords::TREFPOS, "TOPOCENTER");
    h.set_internal("TSTART", 0.0);
    h.set_internal("TSTOP", 86400.0); // one day, in seconds
    h.set_internal(keywords::DATE_OBS, "2017-09-04T00:00:00");

    let t = Frame::from_header(&h).unwrap();
    assert_eq!(t.scale, Scale::known(ScaleKind::Tt));
    assert_eq!(t.mjdref, 58000.0);
    assert_eq!(t.trefpos, ReferencePosition::Topocenter);
    assert_eq!(t.unit_seconds().unwrap(), 1.0);
    // TSTART=0 → MJDREF; TSTOP=86400 s → one day later.
    assert_eq!(t.relative_to_mjd(0.0).unwrap(), 58000.0);
    assert_eq!(t.relative_to_mjd(86400.0).unwrap(), 58001.0);
    // DATE-OBS 2017-09-04 = MJD 58000.0, a whole day, so exact.
    assert_eq!(
        Coordinate::observation(&h).unwrap(),
        Some(Coordinate {
            mjd: 58000.0,
            scale: Scale::known(ScaleKind::Tt)
        })
    );

    let mut malformed = h.clone();
    malformed.set_internal(keywords::TIMEOFFS, "not a real");
    assert!(matches!(
        Frame::from_header(&malformed),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "TIMEOFFS" && expected == "real"
    ));

    let mut positions = Header::new();
    assert_eq!(
        Frame::from_header(&positions).unwrap().trefpos,
        ReferencePosition::Topocenter
    );
    positions
        .set_internal(keywords::TREFPOS, "BARYCENT")
        .set_internal("TRPOS4", "GEOCENTR");
    assert_eq!(
        Frame::from_header(&positions).unwrap().trefpos,
        ReferencePosition::Barycenter
    );
    assert_eq!(
        Frame::for_column(&positions, 4).unwrap().trefpos,
        ReferencePosition::Geocenter
    );
    assert!(matches!(
        Frame::for_column(&positions, 0),
        Err(FitsError::OneBasedIndexRequired {
            kind: "table column"
        })
    ));
    assert!(matches!(
        PhaseAxis::from_header(&positions, 0, None),
        Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" })
    ));
    assert!(matches!(
        PhaseAxis::from_pixel_list(&positions, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            kind: "table column"
        })
    ));
    assert!(matches!(
        PhaseAxis::from_array_column(&positions, 0, 1, None),
        Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" })
    ));
    assert!(matches!(
        PhaseAxis::from_array_column(&positions, 1, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            kind: "table column"
        })
    ));
    for (value, expected) in [
        ("TOP", ReferencePosition::Topocenter),
        ("GEOCENTER", ReferencePosition::Geocenter),
        ("BARYCENTER", ReferencePosition::Barycenter),
        ("RELOCATABLE", ReferencePosition::Relocatable),
        ("CUSTOM", ReferencePosition::Custom),
        ("HELIOCENTER", ReferencePosition::Heliocenter),
        ("GALACTIC", ReferencePosition::GalacticCenter),
        ("EMBARYCENTER", ReferencePosition::EarthMoonBarycenter),
        ("MERCURY", ReferencePosition::Mercury),
        ("VENUS", ReferencePosition::Venus),
        ("MARS", ReferencePosition::Mars),
        ("JUPITER", ReferencePosition::Jupiter),
        ("SATURN", ReferencePosition::Saturn),
        ("URANUS", ReferencePosition::Uranus),
        ("NEPTUNE", ReferencePosition::Neptune),
    ] {
        positions.set_internal(keywords::TREFPOS, value);
        assert_eq!(
            Frame::from_header(&positions).unwrap().trefpos,
            expected,
            "{value}"
        );
    }
    positions.set_internal(keywords::TREFPOS, "topocenter");
    assert_eq!(
        Frame::from_header(&positions).unwrap().trefpos,
        ReferencePosition::Other("topocenter".to_string())
    );

    positions
        .set_internal(keywords::TREFPOS, 42)
        .set_internal("TRPOS4", "GEOCENTR");
    assert_eq!(
        Frame::for_column(&positions, 4).unwrap().trefpos,
        ReferencePosition::Geocenter
    );
    assert!(matches!(
        Frame::from_header(&positions),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "TREFPOS" && expected == "text"
    ));
}

#[test]
fn fits_time_reads_split_and_day_unit_references() {
    let mut h = Header::new();
    h.set_internal(keywords::MJDREFI, 58000.0);
    h.set_internal(keywords::MJDREFF, 0.25);
    h.set_internal(keywords::TIMEUNIT, "d");
    let t = Frame::from_header(&h).unwrap();
    assert_eq!(t.scale, Scale::known(ScaleKind::Utc)); // default
    assert_eq!(t.mjdref, 58000.25);
    assert_eq!(t.unit_seconds().unwrap(), 86400.0);
    // 2 days past the reference.
    assert_eq!(t.relative_to_mjd(2.0).unwrap(), 58002.25);
}

#[test]
fn time_scale_preserves_realizations_and_local_names() {
    for (text, expected) in [
        ("tt", Scale::known(ScaleKind::Tt)),
        ("TDT", Scale::known(ScaleKind::Tt)),
        ("IAT", Scale::known(ScaleKind::Tai)),
        ("GMT", Scale::known(ScaleKind::Utc)),
        ("UT1", Scale::known(ScaleKind::Ut1)),
        ("TAI", Scale::known(ScaleKind::Tai)),
        ("TCG", Scale::known(ScaleKind::Tcg)),
        ("TDB", Scale::known(ScaleKind::Tdb)),
        ("TCB", Scale::known(ScaleKind::Tcb)),
        ("GPS", Scale::known(ScaleKind::Gps)),
    ] {
        assert_eq!(text.parse::<Scale>().unwrap(), expected, "{text}");
    }
    assert_eq!(
        "TT(TAI)".parse::<Scale>().unwrap(),
        Scale::Known {
            kind: ScaleKind::Tt,
            realization: Some("TAI".to_string()),
        }
    );
    assert_eq!(
        "UTC(NIST)".parse::<Scale>().unwrap(),
        Scale::Known {
            kind: ScaleKind::Utc,
            realization: Some("NIST".to_string()),
        }
    );
    for name in ["LOCAL", "MET", "OET", "BOGUS"] {
        assert_eq!(
            name.parse::<Scale>().unwrap(),
            Scale::Local(name.to_string())
        );
    }
    assert_eq!(
        "LOCAL(clock)".parse::<Scale>().unwrap(),
        Scale::Local("LOCAL(clock)".to_string())
    );
    for malformed in ["UTC(", "UTC()", "UTC(NIST))"] {
        assert!(
            matches!(
                malformed.parse::<Scale>(),
                Err(FitsError::InvalidTime { .. })
            ),
            "{malformed}"
        );
    }

    let mut unknown = Header::new();
    unknown.set_internal(keywords::TIMESYS, "BOGUS");
    assert_eq!(
        Frame::from_header(&unknown).unwrap().scale,
        Scale::Local("BOGUS".to_string())
    );
}

#[test]
fn timeoffs_shifts_relative_times() {
    // MJDREF=58000, TIMEUNIT=s, TIMEOFFS=10 s: the offset is added before scaling,
    // so a relative value of 0 lands 10 s past the reference (§9.4.1).
    let mut h = Header::new();
    h.set_internal(keywords::MJDREF, 58000.0);
    h.set_internal(keywords::TIMEUNIT, "s");
    h.set_internal(keywords::TIMEOFFS, 10.0);
    let t = Frame::from_header(&h).unwrap();
    assert_eq!(t.timeoffs, 10.0);
    assert_eq!(t.relative_to_mjd(0.0).unwrap(), 58000.0 + 10.0 / 86400.0);
    assert_eq!(t.relative_to_mjd(5.0).unwrap(), 58000.0 + 15.0 / 86400.0);
}

#[test]
fn time_units_parse_prefixes_and_epoch_dependent_years() {
    let unit = |u: &str| {
        let mut h = Header::new();
        h.set_internal(keywords::TIMEUNIT, u);
        Frame::from_header(&h).unwrap().unit_seconds().unwrap()
    };
    assert_eq!(unit("min"), 60.0);
    assert_eq!(unit("h"), 3600.0);
    assert_eq!(unit("d"), 86400.0);
    assert_eq!(unit("a"), 365.25 * 86400.0); // Julian year
    assert_eq!(unit("cy"), 36525.0 * 86400.0); // Julian century
    assert_eq!(unit("s"), 1.0);
    assert_eq!(unit("ms"), 1e-3);
    assert_eq!(unit("ks"), 1e3);
    assert_eq!(unit("Ma"), 1e6 * 365.25 * 86400.0);
    assert_eq!(unit("10**3 s"), 1e3);

    let mut tropical = Header::new();
    tropical
        .set_internal(keywords::TIMESYS, "TDB")
        .set_internal(keywords::MJDREF, 51544.5)
        .set_internal(keywords::TIMEUNIT, "ta");
    let tropical = Frame::from_header(&tropical).unwrap();
    // MJDREF is J2000.0, so the polynomial's terms in centuries all vanish.
    assert_eq!(
        tropical.unit_seconds().unwrap(),
        365.242_190_402_112_4 * SEC_PER_DAY
    );

    let mut besselian = Header::new();
    besselian
        .set_internal(keywords::TIMESYS, "TT")
        .set_internal(keywords::MJDREF, 15019.5)
        .set_internal(keywords::TIMEUNIT, "Ba");
    let besselian = Frame::from_header(&besselian).unwrap();
    // MJDREF is B1900.0's reference day, JD 2415020.0, so the century term vanishes.
    assert_eq!(
        besselian.unit_seconds().unwrap(),
        365.242_198_781_7 * SEC_PER_DAY
    );

    let mut invalid = Header::new();
    // The minute, day and century take no SI prefix, the year only multiples (wcslib).
    for value in ["", "m", "Hz", "day", "bogus", "Mmin", "kd", "ma"] {
        invalid.set_internal(keywords::TIMEUNIT, value);
        assert!(
            matches!(
                Frame::from_header(&invalid).unwrap().unit_seconds(),
                Err(FitsError::InvalidUnit { unit, expected: "a time unit" }) if unit == value
            ),
            "{value:?} should not be accepted as a time unit"
        );
    }

    let mut frame_dependent = Header::new();
    for (unit, scale) in [("ta", "UTC"), ("Ba", "TDB")] {
        frame_dependent
            .set_internal(keywords::TIMEUNIT, unit)
            .set_internal(keywords::TIMESYS, scale);
        let time = Frame::from_header(&frame_dependent).unwrap();
        assert!(matches!(
            time.unit_seconds(),
            Err(FitsError::ExternalTimeDataRequired { .. })
        ));
    }
}

#[test]
fn prefixed_relative_time_uses_the_declared_scale() {
    let mut milliseconds = Header::new();
    milliseconds
        .set_internal(keywords::MJDREF, 58000.0)
        .set_internal(keywords::TIMEUNIT, "ms");
    let milliseconds = Frame::from_header(&milliseconds).unwrap();
    assert_eq!(
        milliseconds.relative_to_mjd(1000.0).unwrap(),
        58000.0 + 1.0 / SEC_PER_DAY
    );

    let mut kiloseconds = Header::new();
    kiloseconds
        .set_internal(keywords::MJDREF, 58000.0)
        .set_internal(keywords::TIMEUNIT, "ks");
    let kiloseconds = Frame::from_header(&kiloseconds).unwrap();
    // 86.4 ks = 86,400 s = one day.
    assert_eq!(kiloseconds.relative_to_mjd(86.4).unwrap(), 58001.0);
}

#[test]
fn split_reference_takes_precedence_over_single_mjdref() {
    let mjdref = |pairs: &[(&str, f64)]| {
        let mut h = Header::new();
        for &(k, v) in pairs {
            h.set_internal(k, v);
        }
        Frame::from_header(&h).unwrap().mjdref
    };
    // §9.2.2: a full integer+fractional split wins over the single value.
    assert_eq!(
        mjdref(&[("MJDREF", 58000.0), ("MJDREFI", 59000.0), ("MJDREFF", 0.5)]),
        59000.5
    );
    // Single value alone is used as-is.
    assert_eq!(mjdref(&[("MJDREF", 58000.0)]), 58000.0);
    // An incomplete split (integer part only) defers to the single value.
    assert_eq!(
        mjdref(&[("MJDREF", 58000.0), ("MJDREFI", 59000.0)]),
        58000.0
    );
    // A JD split is shifted to an MJD before its fraction is added: summed as a JD
    // first, 0.5003 would round to the JD's precision and end at 51544.000299999956.
    assert_eq!(
        mjdref(&[("JDREFI", 2_451_544.0), ("JDREFF", 0.5003)]),
        51544.0003
    );
    assert_eq!(mjdref(&[("JDREF", 2_451_544.5)]), 51544.0);
}
