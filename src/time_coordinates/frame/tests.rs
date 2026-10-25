use crate::fits_error::FitsError;
use crate::fits_error::Indexed;
use crate::fits_error::TimeConversion;
use crate::fits_error::UnitKind;
use crate::fits_error::ValueKind;
use crate::header_model::record_name::RecordName;
use crate::header_model::value::Value;
use crate::header_model::{Header, keywords};
use crate::internals::cards;
use crate::internals::tolerance;
use crate::internals::tolerance::Tolerance;
use std::assert_matches;

use crate::time_coordinates::bounds::Bounds;
use crate::time_coordinates::coordinate::Coordinate;
use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::reference_position::ReferencePosition;
use crate::time_coordinates::scale::{Scale, ScaleKind};
use crate::unit::SECONDS_PER_DAY;
use crate::world_coordinates::Wcs;

#[test]
fn header_datetimes_use_the_declared_scale() {
    let mut header = cards::header_of(&[
        ("TIMESYS", "MET".into()),
        ("DATEREF", "2017-01-01T00:00:00".into()),
        ("DATE-OBS", "2017-01-01T00:00:00".into()),
        ("DATE-END", "2017-01-02T00:00:00".into()),
    ]);
    let time = Frame::from_header(&header).unwrap();
    assert_eq!(time.scale, Scale::Local("MET".to_string()));
    assert_eq!(time.reference_mjd, Mjd::new(57_754, 0.0));
    assert_eq!(
        Coordinate::observation(&header).unwrap(),
        Some(Coordinate {
            mjd: Mjd::new(57_754, 0.0),
            scale: Scale::Local("MET".to_string())
        })
    );
    assert_eq!(
        Bounds::new(&header, &time).end().unwrap(),
        Some(Coordinate {
            mjd: Mjd::new(57_755, 0.0),
            scale: Scale::Local("MET".to_string())
        })
    );

    header
        .set_internal(keywords::TIMESYS, "UTC")
        .set_internal(keywords::DATEREF, "2016-12-31T23:59:60")
        .set_internal(keywords::DATE_OBS, "2016-12-31T23:59:60")
        .set_internal(keywords::DATE_END, "2016-12-31T23:59:60");
    assert_matches!(
        Frame::from_header(&header),
        Err(FitsError::ExternalTimeDataRequired {
            conversion: TimeConversion::LeapSecond
        })
    );
    assert_matches!(
        Coordinate::observation(&header),
        Err(FitsError::ExternalTimeDataRequired {
            conversion: TimeConversion::LeapSecond
        })
    );
    assert_matches!(
        Bounds::new(&header, &time).end(),
        Err(FitsError::ExternalTimeDataRequired {
            conversion: TimeConversion::LeapSecond
        })
    );
}

/// A relative time keeps a spacing an `f64` MJD has not: MJD 60000.5 and 1e-6 s keep
/// the microsecond to its 2⁻⁵³-day spacing, where one `f64` rounds 1.157e-11 day to the
/// two units of 2⁻³⁷ day nearest, 1.257 µs.
#[test]
fn relative_times_keep_their_precision_and_range() {
    let mut h = Header::new();
    h.set_internal(keywords::MJDREFI, 60000.0)
        .set_internal(keywords::MJDREFF, 0.5);
    let mjd = Frame::from_header(&h)
        .unwrap()
        .relative_axis()
        .unwrap()
        .mjd(1e-6)
        .unwrap();
    assert_eq!(mjd.day(), 60000);
    let seconds = (mjd.fraction() - 0.5) * SECONDS_PER_DAY;
    tolerance::assert_within(
        seconds,
        1e-6,
        Tolerance::abs(
            2.0_f64.powi(-53) * SECONDS_PER_DAY,
            "an ulp of the day's fraction near 0.5, in seconds",
        ),
        "relative time at MJDREF + 0.5",
    );
    let one_f64 = (60_000.5 + 1e-6 / SECONDS_PER_DAY - 60_000.5) * SECONDS_PER_DAY;
    assert_eq!(one_f64, 2.0 * 2.0_f64.powi(-37) * SECONDS_PER_DAY);

    // 12 h before the reference's noon is the midnight that starts its day, and 1 s
    // more is the day before, 86399 s into it.
    let frame = Frame::from_header(&h).unwrap();
    assert_eq!(
        frame.relative_axis().unwrap().mjd(-43_200.0).unwrap(),
        Mjd::new(60000, 0.0)
    );
    assert_eq!(
        frame.relative_axis().unwrap().mjd(-43_201.0).unwrap(),
        Mjd::new(59999, 86_399.0 / SECONDS_PER_DAY)
    );

    // A day past 2⁵³ holds no fraction, and the header that gives one is refused.
    assert_matches!(
        frame.relative_axis().unwrap().mjd(1e300),
        Err(FitsError::TimeOutOfRange)
    );
    for keyword in [keywords::MJDREF, keywords::MJD_OBS, keywords::JEPOCH] {
        let mut far = Header::new();
        far.set_internal(keyword, 1e300);
        let error = if keyword == keywords::MJDREF {
            Frame::from_header(&far).map(|_| ())
        } else {
            Coordinate::observation(&far).map(|_| ())
        };
        assert_matches!(error, Err(FitsError::InvalidKeywordValue { keyword: got, .. }) if got == keyword,
            "{keyword}"
        );
    }
}

#[test]
fn time_axis_uses_complete_wcs_row_unit_and_scale() {
    let mut h = cards::header_of(&[
        ("NAXIS", 1.into()),
        ("MJDREF", 58000.0.into()),
        ("TIMESYS", "UTC".into()),
        ("TIMEUNIT", "s".into()),
        ("CTYPE1A", "TAI".into()),
        ("CUNIT1A", "d".into()),
        ("CRPIX1A", 1.0.into()),
        ("CRVAL1A", 0.0.into()),
        ("CD1_1A", 2.0.into()),
    ]);
    let t = Frame::from_header(&h).unwrap();
    let alternate = Wcs::from_header(&h, 'A').unwrap();
    // CD1_1 = 2 d/pixel and pixel offset 0.5 produce exactly one day. CUNIT1A and
    // CTYPE1A override the global seconds/UTC frame.
    assert_eq!(
        t.wcs_time(&alternate, 0, &[1.5]).unwrap(),
        Some(Coordinate {
            mjd: Mjd::new(58001, 0.0),
            scale: Scale::known(ScaleKind::Tai),
        })
    );

    h.set_internal("CUNIT1A", "ms");
    let milliseconds = Wcs::from_header(&h, 'A').unwrap();
    // With the same CD row, an offset of 500 pixels is 1000 ms = 1 s.
    let coordinate = t.wcs_time(&milliseconds, 0, &[501.0]).unwrap().unwrap();
    assert_eq!(coordinate.mjd, Mjd::new(58000, 1.0 / SECONDS_PER_DAY));

    h.set_internal("CUNIT1A", "Hz");
    let invalid_unit = Wcs::from_header(&h, 'A').unwrap();
    assert_matches!(
        t.wcs_time(&invalid_unit, 0, &[1.0]),
        Err(FitsError::InvalidUnit {
            unit,
            expected: UnitKind::Time
        }) if unit == "Hz"
    );

    let coupled = cards::header_of(&[
        ("NAXIS", 2.into()),
        ("MJDREF", 58000.0.into()),
        ("TIMEUNIT", "s".into()),
        ("CTYPE1", "TIME".into()),
        ("CTYPE2", "LINEAR".into()),
        ("CRPIX1", 1.0.into()),
        ("CRPIX2", 1.0.into()),
        ("CRVAL1", 10.0.into()),
        ("CDELT1", 2.0.into()),
        ("CDELT2", 1.0.into()),
        ("PC1_1", 1.0.into()),
        ("PC1_2", 0.5.into()),
        ("PC2_1", 0.0.into()),
        ("PC2_2", 1.0.into()),
    ]);
    let wcs = Wcs::from_header(&coupled, None).unwrap();
    // Row 1 is CDELT1 × PC1_j = [2, 1]. At pixel [3, 5], offsets [2, 4]
    // contribute 2×2 + 1×4 = 8 s, then CRVAL1 adds 10 s.
    let coordinate = t.wcs_time(&wcs, 0, &[3.0, 5.0]).unwrap().unwrap();
    assert_eq!(coordinate.scale, Scale::known(ScaleKind::Utc));
    assert_eq!(coordinate.mjd, Mjd::new(58000, 18.0 / SECONDS_PER_DAY));

    h.set_internal("CTYPE1A", "TIME-LOG")
        .set_internal("CUNIT1A", "d")
        .set_internal("CRVAL1A", 10.0)
        .set_internal("CD1_1A", 2.0);
    let logarithmic = Wcs::from_header(&h, 'A').unwrap();
    let coordinate = t.wcs_time(&logarithmic, 0, &[2.0]).unwrap().unwrap();
    let expected_days = 10.0 * 0.2_f64.exp();
    assert_eq!(coordinate.mjd, Mjd::new(58000, expected_days));

    let mut non_time = Header::new();
    non_time
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "LINEAR");
    assert_matches!(
        t.wcs_time(&Wcs::from_header(&non_time, None).unwrap(), 0, &[1.0]),
        Ok(None)
    );
    assert_matches!(t.time_axis("LINEAR", ""), Ok(None));
    assert_matches!(t.time_axis("PHASE", ""), Ok(None));

    h.set_internal("CTYPE1A", "TIME-TAB")
        .set_internal("CUNIT1A", "d");
    let unsupported = Wcs::from_header(&h, 'A').unwrap();
    assert_matches!(
        t.wcs_time(&unsupported, 0, &[1.0]),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == vec![0]
    );
}

#[test]
fn frame_resolves_reference_and_relative_times() {
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
    assert_eq!(t.reference_mjd, Mjd::new(58000, 0.0));
    assert_eq!(t.reference_position, ReferencePosition::Topocenter);
    assert_eq!(t.unit_seconds().unwrap(), 1.0);
    // TSTART=0 → MJDREF; TSTOP=86400 s → one day later.
    assert_eq!(
        t.relative_axis().unwrap().mjd(0.0).unwrap(),
        Mjd::new(58000, 0.0)
    );
    assert_eq!(
        t.relative_axis().unwrap().mjd(86400.0).unwrap(),
        Mjd::new(58001, 0.0)
    );
    // DATE-OBS 2017-09-04 = MJD 58000.0, a whole day, so exact.
    assert_eq!(
        Coordinate::observation(&h).unwrap(),
        Some(Coordinate {
            mjd: Mjd::new(58000, 0.0),
            scale: Scale::known(ScaleKind::Tt)
        })
    );

    let mut malformed = h.clone();
    malformed.set_internal(keywords::TIMEOFFS, "not a real");
    assert_matches!(
        Frame::from_header(&malformed),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), expected: ValueKind::Real })
            if name == "TIMEOFFS"
    );

    let mut positions = Header::new();
    assert_eq!(
        Frame::from_header(&positions).unwrap().reference_position,
        ReferencePosition::Topocenter
    );
    positions
        .set_internal(keywords::TREFPOS, "BARYCENT")
        .set_internal("TRPOS4", "GEOCENTR");
    assert_eq!(
        Frame::from_header(&positions).unwrap().reference_position,
        ReferencePosition::Barycenter
    );
    assert_eq!(
        Frame::from_column(&positions, 4)
            .unwrap()
            .reference_position,
        ReferencePosition::Geocenter
    );
    assert_matches!(
        Frame::from_column(&positions, 0),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::Column
        })
    );
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
            Frame::from_header(&positions).unwrap().reference_position,
            expected,
            "{value}"
        );
    }
    positions.set_internal(keywords::TREFPOS, "topocenter");
    assert_eq!(
        Frame::from_header(&positions).unwrap().reference_position,
        ReferencePosition::Other("topocenter".to_string())
    );

    positions
        .set_internal(keywords::TREFPOS, 42)
        .set_internal("TRPOS4", "GEOCENTR");
    assert_eq!(
        Frame::from_column(&positions, 4)
            .unwrap()
            .reference_position,
        ReferencePosition::Geocenter
    );
    assert_matches!(
        Frame::from_header(&positions),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), expected: ValueKind::Text })
            if name == "TREFPOS"
    );
}

/// A keyword and its value.
type Card = (&'static str, Value);

/// A relative time lands at the reference plus its value in the unit, after the clock
/// offset `TIMEOFFS` (§9.4.1): 2 days past a split reference; 0 and 5 s past a 10 s offset;
/// 1000 ms; and 86.4 ks, which in `f64` is 86.400000000000005684341886080801… ks, whose
/// 5.684341886080801e-12 s over 86400 the fraction keeps as 6.579099405186112e-17 day. The
/// scale defaults to UTC.
#[test]
fn relative_times_land_past_the_reference_in_their_unit() {
    let cases: [(&[Card], f64, Mjd); 5] = [
        (
            &[
                ("MJDREFI", 58000.0.into()),
                ("MJDREFF", 0.25.into()),
                ("TIMEUNIT", "d".into()),
            ],
            2.0,
            Mjd::new(58002, 0.25),
        ),
        (
            &[
                ("MJDREF", 58000.0.into()),
                ("TIMEUNIT", "s".into()),
                ("TIMEOFFS", 10.0.into()),
            ],
            0.0,
            Mjd::new(58000, 10.0 / 86400.0),
        ),
        (
            &[
                ("MJDREF", 58000.0.into()),
                ("TIMEUNIT", "s".into()),
                ("TIMEOFFS", 10.0.into()),
            ],
            5.0,
            Mjd::new(58000, 15.0 / 86400.0),
        ),
        (
            &[("MJDREF", 58000.0.into()), ("TIMEUNIT", "ms".into())],
            1000.0,
            Mjd::new(58000, 1.0 / SECONDS_PER_DAY),
        ),
        (
            &[("MJDREF", 58000.0.into()), ("TIMEUNIT", "ks".into())],
            86.4,
            Mjd::new(58001, 6.579_099_405_186_112e-17),
        ),
    ];
    for (cards, relative, mjd) in cases {
        let frame = Frame::from_header(&cards::header_of(cards)).unwrap();
        assert_eq!(frame.scale, Scale::known(ScaleKind::Utc));
        assert_eq!(
            frame.relative_axis().unwrap().mjd(relative).unwrap(),
            mjd,
            "{cards:?}"
        );
    }
    // One `f64` MJD rounds the 86.4 ks case's fraction away.
    assert_eq!(Mjd::new(58001, 6.579_099_405_186_112e-17).to_f64(), 58001.0);
}

#[test]
fn split_reference_takes_precedence_over_single_mjdref() {
    let reference_mjd = |pairs: &[(&str, f64)]| {
        let mut h = Header::new();
        for &(k, v) in pairs {
            h.set_internal(k, v);
        }
        Frame::from_header(&h).unwrap().reference_mjd
    };
    // §9.2.2: a full integer+fractional split wins over the single value.
    assert_eq!(
        reference_mjd(&[("MJDREF", 58000.0), ("MJDREFI", 59000.0), ("MJDREFF", 0.5)]),
        Mjd::new(59000, 0.5)
    );
    // Single value alone is used as-is.
    assert_eq!(reference_mjd(&[("MJDREF", 58000.0)]), Mjd::new(58000, 0.0));
    // An incomplete split (integer part only) defers to the single value.
    assert_eq!(
        reference_mjd(&[("MJDREF", 58000.0), ("MJDREFI", 59000.0)]),
        Mjd::new(58000, 0.0)
    );
    // A JD split keeps the fraction exactly: 0.5003 − 0.5 is exact (Sterbenz).
    assert_eq!(
        reference_mjd(&[("JDREFI", 2_451_544.0), ("JDREFF", 0.5003)]),
        Mjd::new(51544, 0.5003 - 0.5)
    );
    assert_eq!(
        reference_mjd(&[("JDREFI", 2_460_000.0), ("JDREFF", 0.75)]),
        Mjd::new(60000, 0.25)
    );
    assert_eq!(
        reference_mjd(&[("JDREF", 2_451_544.5)]),
        Mjd::new(51544, 0.0)
    );
    // A single MJDREF that carries a fraction splits into the day and the fraction.
    assert_eq!(reference_mjd(&[("MJDREF", 60000.5)]), Mjd::new(60000, 0.5));
}
