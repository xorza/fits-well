use crate::fits_error::FitsError;
use crate::header_model::{Header, keywords};

use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::scale::{Scale, ScaleKind};

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
    // Each scale prints as its standard code, or as it was read, and parses back.
    for (text, printed) in [
        ("tt", "TT"),
        ("ET", "TT"),
        ("GMT", "UTC"),
        ("UT", "UT1"),
        ("IAT", "TAI"),
        ("TCG", "TCG"),
        ("TDB", "TDB"),
        ("TCB", "TCB"),
        ("GPS", "GPS"),
        ("TDT (TAI)", "TT(TAI)"),
        ("UTC(NIST)", "UTC(NIST)"),
        ("MET", "MET"),
        ("LOCAL(clock)", "LOCAL(clock)"),
    ] {
        let scale = text.parse::<Scale>().unwrap();
        assert_eq!(scale.to_string(), printed, "{text}");
        assert_eq!(printed.parse::<Scale>().unwrap(), scale, "{text}");
    }
    for malformed in ["UTC(", "UTC()", "UTC(NIST))"] {
        assert!(
            matches!(
                malformed.parse::<Scale>(),
                Err(FitsError::InvalidTimeScale { keyword: None })
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
