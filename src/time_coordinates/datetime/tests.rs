use crate::error::FitsError;
use crate::header_model::{Header, keywords};

use crate::time_coordinates::coordinate::Coordinate;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;
use crate::unit::SECONDS_PER_DAY;

/// Golden values throughout are from `astropy.time` (ERFA).
#[test]
fn iso_to_mjd_matches_astropy() {
    // 06:30:15.5 is 23415.5 s into the day: astropy prints MJD 60369.271012731, here
    // day 60369 and that fraction rounded once.
    let cases: &[(&str, Mjd)] = &[
        ("2000-01-01T12:00:00", Mjd::new(51544, 0.5)),
        ("1858-11-17T00:00:00", Mjd::new(0, 0.0)),
        (
            "2024-02-29T06:30:15.5",
            Mjd::new(60369, 23_415.5 / 86_400.0),
        ),
        ("1900-01-01T00:00:00", Mjd::new(15020, 0.0)),
        ("2024-06-01", Mjd::new(60462, 0.0)), // date-only ⇒ midnight
    ];
    for &(s, mjd) in cases {
        let d = s.parse::<Datetime>().unwrap();
        assert_eq!(d.to_mjd().unwrap(), mjd, "{s}");
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
        assert!(
            matches!(
                s.parse::<Datetime>(),
                Err(FitsError::InvalidDatetime { keyword: None })
            ),
            "{s:?} should be rejected"
        );
    }

    // The error names the keyword that held the datetime.
    let mut header = Header::new();
    header.set_internal(keywords::DATE_OBS, "2024-13-01");
    assert!(matches!(
        Coordinate::observation(&header),
        Err(FitsError::InvalidDatetime { keyword: Some(keyword) }) if keyword == keywords::DATE_OBS
    ));
    header.remove(keywords::DATE_OBS).unwrap();
    header.set_internal(keywords::DATEREF, "2024-02-30");
    assert!(matches!(
        Frame::from_header(&header),
        Err(FitsError::InvalidDatetime { keyword: Some(keyword) }) if keyword == keywords::DATEREF
    ));
    assert!(matches!(
        "2024-13-01".parse::<Datetime>(),
        Err(FitsError::InvalidDatetime { keyword: None })
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
        assert!(
            matches!(
                bad.parse::<Datetime>(),
                Err(FitsError::InvalidDatetime { keyword: None })
            ),
            "{bad:?} should be rejected"
        );
    }
    assert_eq!("-00044-03-15".parse::<Datetime>().unwrap().year(), -44);
    assert_eq!("+02024-06-01".parse::<Datetime>().unwrap().year(), 2024);
    // A Gregorian 400-year cycle is 146097 days. 2000-01-01 is MJD 51544, so
    // 100000-01-01 is 245 cycles on and 99999-12-31 a day before it; 2001-01-01 is
    // MJD 51910, and -99999-01-01 is 255 cycles before it.
    for (text, year, day) in [
        ("-99999-01-01", -99999, 51_910 - 255 * 146_097),
        ("+99999-12-31", 99999, 51_544 + 245 * 146_097 - 1),
    ] {
        let datetime = text.parse::<Datetime>().unwrap();
        assert_eq!(datetime.year(), year);
        assert_eq!(datetime.to_mjd().unwrap(), Mjd::new(day, 0.0));
    }
    // The constructor holds the range the text forms hold.
    for year in [-100_000, 100_000] {
        assert!(matches!(
            Datetime::date(year, 1, 1),
            Err(FitsError::InvalidDatetime { keyword: None })
        ));
    }
}

#[test]
fn datetime_constructor_checks_each_field() {
    let date = Datetime::date(2024, 2, 29).unwrap();
    assert_eq!(
        [date.year(), i64::from(date.month()), i64::from(date.day())],
        [2024, 2, 29]
    );
    let time = date.at(6, 30, 15.5).unwrap();
    assert_eq!([time.hour(), time.minute(), time.second()], [6, 30, 15]);
    assert_eq!(time.second_fraction(), 0.5);
    assert_eq!(time, "2024-02-29T06:30:15.5".parse().unwrap());
    // 2023 has no 29 February, and a second of 60 is a leap second at 23:59 alone.
    assert!(matches!(
        Datetime::date(2023, 2, 29),
        Err(FitsError::InvalidDatetime { keyword: None })
    ));
    assert!(matches!(
        Datetime::date(2024, 0, 1),
        Err(FitsError::InvalidDatetime { keyword: None })
    ));
    assert!(matches!(
        Datetime::date(2024, 4, 31),
        Err(FitsError::InvalidDatetime { keyword: None })
    ));
    for (hour, minute, second) in [
        (24, 0, 0.0),
        (0, 60, 0.0),
        (12, 0, 60.0),
        (23, 59, 61.0),
        (0, 0, -0.5),
    ] {
        assert!(
            matches!(
                date.at(hour, minute, second),
                Err(FitsError::InvalidDatetime { keyword: None })
            ),
            "{hour}:{minute}:{second}"
        );
    }
    let leap = date.at(23, 59, 60.25).unwrap();
    assert_eq!(leap.second(), 60);
    assert_eq!(leap.second_fraction(), 0.25);
}

/// The time of day enters the fraction in one rounding. 23:59:59.7 is 86399 s plus
/// the `f64` 0.7, which is 0.6999999999999999556 s; that sum over 86400 is
/// 0.99999652777777777726…, whose nearest `f64` ends in …778. Two roundings, of the
/// sum in seconds and then of the quotient, give the `f64` below it, which ends in …777.
#[test]
fn time_of_day_rounds_once() {
    let mjd = "2024-02-29T23:59:59.7"
        .parse::<Datetime>()
        .unwrap()
        .to_mjd()
        .unwrap();
    assert_eq!(mjd, Mjd::new(60369, 0.999_996_527_777_777_8));
    assert_ne!(mjd.fraction(), (86_399.0 + 0.7) / SECONDS_PER_DAY);
}

#[test]
fn a_fraction_next_to_one_stays_in_its_second() {
    let datetime = "2024-01-01T23:59:59.99999999999999999"
        .parse::<Datetime>()
        .unwrap();
    assert_eq!(datetime.second(), 59);
    assert_eq!(datetime.second_fraction(), 1.0 - f64::EPSILON / 2.0);
    assert_eq!(datetime.to_mjd().unwrap(), Mjd::new(60311, 0.0));
}

/// `DATE` held `DD/MM/YY` before 2000 (§4.4.2.1), a year of 1900 to 1999.
#[test]
fn reads_the_legacy_date_form() {
    assert_eq!(
        "14/03/94".parse::<Datetime>().unwrap(),
        Datetime::date(1994, 3, 14).unwrap()
    );
    assert_eq!(
        "01/01/00".parse::<Datetime>().unwrap(),
        Datetime::date(1900, 1, 1).unwrap()
    );
    for bad in [
        "32/01/94",
        "14/13/94",
        "1/03/94",
        "14/03/1994",
        "14-03-94",
        "aa/03/94",
    ] {
        assert!(
            matches!(
                bad.parse::<Datetime>(),
                Err(FitsError::InvalidDatetime { keyword: None })
            ),
            "{bad}"
        );
    }
}

#[test]
fn signed_gregorian_years_use_floor_division() {
    // JD 1721059.5, 1720694.5, −32104.5 and 0, less 2400000.5.
    let cases = [
        ("0000-01-01T00:00:00", Mjd::new(-678_941, 0.0)),
        ("-00001-01-01T00:00:00", Mjd::new(-679_306, 0.0)),
        ("-04800-01-01T00:00:00", Mjd::new(-2_432_105, 0.0)),
        ("-04713-11-24T12:00:00", Mjd::new(-2_400_001, 0.5)),
    ];
    for (text, expected) in cases {
        let date = text.parse::<Datetime>().unwrap();
        assert_eq!(date.to_mjd().unwrap(), expected, "{text}");
    }
}

#[test]
fn leap_second_labels_require_external_time_data() {
    // A second of 60 exists only as the last of a day.
    assert!(matches!(
        "2016-12-31T12:00:60".parse::<Datetime>(),
        Err(FitsError::InvalidDatetime { keyword: None })
    ));
    let leap = "2030-06-30T23:59:60".parse::<Datetime>().unwrap();
    assert_eq!(leap.second(), 60);
    assert!(matches!(
        leap.to_mjd(),
        Err(FitsError::ExternalTimeDataRequired { .. })
    ));
}
