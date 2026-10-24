use crate::fits_error::FitsError;
use crate::fits_error::TimeConversion;
use crate::header_model::keywords;
use crate::internals::cards::header_of;

use crate::time_coordinates::bounds::Bounds;
use crate::time_coordinates::coordinate::Coordinate;
use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::scale::{Scale, ScaleKind};

#[test]
fn reads_bound_duration_and_error_keywords() {
    let mut h = header_of(&[
        ("MJD-BEG", 58000.0.into()),
        ("DATE-END", "2017-09-05T00:00:00".into()),
        ("MJD-AVG", 58000.5.into()),
        ("XPOSURE", 1200.0.into()),
        ("TELAPSE", 1500.0.into()),
        ("TIMEDEL", 0.1.into()),
        ("TIMSYER", 1e-6.into()),
        ("TIMESYS", "TT".into()),
    ]);
    let frame = Frame::from_header(&h).unwrap();
    let tt = |mjd| {
        Some(Coordinate {
            mjd,
            scale: Scale::known(ScaleKind::Tt),
        })
    };
    let b = Bounds::new(&h, &frame);
    assert_eq!(b.start().unwrap(), tt(Mjd::new(58000, 0.0)));
    // Resolved from DATE-END: 2017-09-04 is MJD 58000.
    assert_eq!(b.end().unwrap(), tt(Mjd::new(58001, 0.0)));
    assert_eq!(b.mid().unwrap(), tt(Mjd::new(58000, 0.5))); // §9.5 midpoint
    assert_eq!(b.exposure().unwrap(), Some(1200.0));
    assert_eq!(b.elapsed().unwrap(), Some(1500.0));
    assert_eq!(b.resolution().unwrap(), Some(0.1));
    assert_eq!(b.pixel_position().unwrap(), 0.5); // default when absent
    assert_eq!(b.systematic_error().unwrap(), Some(1e-6));
    assert_eq!(b.random_error().unwrap(), None);

    // MJD-END wins over DATE-END.
    h.set_internal(keywords::MJD_END, 58002.0);
    assert_eq!(
        Bounds::new(&h, &frame).end().unwrap(),
        tt(Mjd::new(58002, 0.0))
    );

    // A bad field fails alone: a leap-second DATE-BEG and an out-of-range TIMEPIXR
    // leave the other fields readable.
    h.remove(keywords::MJD_BEG).unwrap();
    h.set_internal(keywords::DATE_BEG, "2016-12-31T23:59:60");
    for invalid in [-f64::EPSILON, 1.0 + f64::EPSILON] {
        h.set_internal(keywords::TIMEPIXR, invalid);
        let b = Bounds::new(&h, &frame);
        assert!(matches!(
            b.pixel_position(),
            Err(FitsError::KeywordOutOfRange {
                keyword: keywords::TIMEPIXR
            })
        ));
        assert!(matches!(
            b.start(),
            Err(FitsError::ExternalTimeDataRequired {
                conversion: TimeConversion::LeapSecond
            })
        ));
        assert_eq!(b.end().unwrap(), tt(Mjd::new(58002, 0.0)));
        assert_eq!(b.exposure().unwrap(), Some(1200.0));
        assert_eq!(b.elapsed().unwrap(), Some(1500.0));
    }
}
