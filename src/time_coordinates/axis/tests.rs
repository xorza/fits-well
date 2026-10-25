use crate::header_model::keywords;
use crate::internals::cards;

use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::scale::{Scale, ScaleKind};
use crate::unit::SECONDS_PER_DAY;

#[test]
fn timeoffs_stays_in_timeunit_on_an_axis_of_another_unit() {
    // TIMEUNIT = s, CUNIT1 = d and TIMEOFFS = 10: the offset is 10 s, not 10 days, so
    // a world value of 1 d lands 1 d + 10 s past MJDREF (§9.4.1).
    let mut h = cards::header_of(&[
        ("MJDREF", 58000.0.into()),
        ("TIMEUNIT", "s".into()),
        ("TIMEOFFS", 10.0.into()),
    ]);
    let frame = Frame::from_header(&h).unwrap();
    let days = frame.time_axis("TIME", "d").unwrap().unwrap();
    assert_eq!(
        days.mjd(1.0).unwrap(),
        Mjd::new(58001, 10.0 / SECONDS_PER_DAY)
    );
    // An empty CUNIT takes TIMEUNIT: 5 s and the 10 s offset.
    let seconds = frame.time_axis("TIME", " ").unwrap().unwrap();
    assert_eq!(
        seconds.mjd(5.0).unwrap(),
        Mjd::new(58000, 15.0 / SECONDS_PER_DAY)
    );
    assert_eq!(*seconds.scale(), Scale::known(ScaleKind::Utc));
    // TIMEUNIT = d moves the offset to 10 days on a seconds axis.
    h.set_internal(keywords::TIMEUNIT, "d");
    let frame = Frame::from_header(&h).unwrap();
    assert_eq!(
        frame
            .time_axis("TIME", "s")
            .unwrap()
            .unwrap()
            .mjd(0.0)
            .unwrap(),
        Mjd::new(58010, 0.0)
    );
}
