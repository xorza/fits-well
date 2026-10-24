use crate::fits_error::FitsError;
use crate::fits_error::TimeConversion;
use crate::fits_error::UnitKind;
use crate::header_model::{Header, keywords};
use crate::internals::cards::header_of;

use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::unit::Unit;
use crate::unit;
use crate::unit::SECONDS_PER_DAY;

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

    let tropical = header_of(&[
        ("TIMESYS", "TDB".into()),
        ("MJDREF", 51544.5.into()),
        ("TIMEUNIT", "ta".into()),
    ]);
    let tropical = Frame::from_header(&tropical).unwrap();
    // MJDREF is J2000.0, so the polynomial's terms in centuries all vanish.
    assert_eq!(
        tropical.unit_seconds().unwrap(),
        365.242_190_402_112_4 * SECONDS_PER_DAY
    );

    let besselian = header_of(&[
        ("TIMESYS", "TT".into()),
        ("MJDREF", 15019.5.into()),
        ("TIMEUNIT", "Ba".into()),
    ]);
    let besselian = Frame::from_header(&besselian).unwrap();
    // MJDREF is B1900.0's reference day, JD 2415020.0, so the century term vanishes.
    assert_eq!(
        besselian.unit_seconds().unwrap(),
        365.242_198_781_7 * SECONDS_PER_DAY
    );

    // A year unit keeps its multiple; its length waits for the frame's epoch and scale.
    assert_eq!(tropical.unit, "ta".parse().unwrap());
    assert_eq!(
        "10+2 Ba".parse::<Unit>().unwrap(),
        "10**2 Ba".parse::<Unit>().unwrap()
    );

    let mut invalid = Header::new();
    // The minute, day and century take no SI prefix, the year only multiples (wcslib).
    // The unit is parsed with the frame, so a bad one refuses the frame.
    for value in ["", "m", "Hz", "day", "bogus", "Mmin", "kd", "ma"] {
        invalid.set_internal(keywords::TIMEUNIT, value);
        assert!(
            matches!(
                Frame::from_header(&invalid),
                Err(FitsError::InvalidUnit { unit, expected: UnitKind::Time }) if unit == value
            ),
            "{value:?} should not be accepted as a time unit"
        );
    }

    let mut frame_dependent = Header::new();
    for (unit, scale, needs) in [
        ("ta", "UTC", TimeConversion::TropicalYear),
        ("Ba", "TDB", TimeConversion::BesselianYear),
    ] {
        frame_dependent
            .set_internal(keywords::TIMEUNIT, unit)
            .set_internal(keywords::TIMESYS, scale);
        let time = Frame::from_header(&frame_dependent).unwrap();
        assert!(matches!(
            time.unit_seconds(),
            Err(FitsError::ExternalTimeDataRequired { conversion }) if conversion == needs
        ));
    }
}

/// A unit prints as it is spelled, its multiplier as `10**k`, and parses back to the same
/// unit of the same length: every base of the time table with each prefix it takes, and
/// each year, at three multipliers.
#[test]
fn a_unit_prints_as_it_is_spelled() {
    for (text, printed) in [
        ("s", "s"),
        ("ks", "ks"),
        ("Ma", "Ma"),
        ("10**3 s", "10**3 s"),
        ("(10**12) s", "10**12 s"),
        ("10^-3 d", "10**-3 d"),
        ("10+2 Ba", "10**2 Ba"),
        ("10**0 min", "min"),
        ("ta", "ta"),
    ] {
        assert_eq!(text.parse::<Unit>().unwrap().to_string(), printed, "{text}");
    }
    let prefixes = [
        "", "da", "d", "c", "m", "u", "n", "p", "f", "a", "z", "y", "h", "k", "M", "G", "T", "P",
        "E", "Z", "Y",
    ];
    let tdb = "TDB".parse().unwrap();
    let tt = "TT".parse().unwrap();
    let epoch = Mjd::new(51_544, 0.5);
    let mut spellings = 0;
    for multiplier in ["", "10**3 ", "10**-3 "] {
        let symbols = unit::TIME
            .iter()
            .flat_map(|base| prefixes.map(|prefix| format!("{prefix}{}", base.symbol)))
            .chain(["ta".to_string(), "Ba".to_string()]);
        for symbol in symbols {
            let text = format!("{multiplier}{symbol}");
            let Ok(unit) = text.parse::<Unit>() else {
                continue;
            };
            spellings += 1;
            assert_eq!(unit.to_string(), text);
            let reread = text.parse::<Unit>().unwrap();
            assert_eq!(reread, unit, "{text}");
            let scale = if symbol == "Ba" { &tt } else { &tdb };
            assert_eq!(
                reread.seconds(epoch, scale).unwrap(),
                unit.seconds(epoch, scale).unwrap(),
                "{text}"
            );
        }
    }
    // Per multiplier: `s` with all 20 prefixes, `a` and `yr` with the 10 multiples, the
    // 7 bases bare, and the two years: 49.
    assert_eq!(spellings, 3 * 49);
}
