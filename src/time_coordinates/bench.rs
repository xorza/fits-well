//! The time cases: a row of times relative to the reference epoch, each to an MJD.

use std::hint::black_box;

use criterion::Criterion;

use crate::bench::policy::{self, Kind, Scale};
use crate::header_model::{Header, keywords};
use crate::internals::scene::WIDTH;
use crate::time_coordinates::frame::Frame;

/// `atomic/to_mjd/<scale>`: one row of times, a sample every 10 s of a night in seconds past
/// `MJDREF`, each to an MJD through the axis `Frame::relative_axis` resolves once, as a caller
/// resolves an event list or a light curve. The scales: `utc`, `tt`, and `tt_tai`, `TT` with
/// its `TAI` realization. Per time, in its `f64`.
pub(crate) fn atomic(c: &mut Criterion) {
    let times: Vec<f64> = (0..WIDTH).map(|i| 10.0 * i as f64).collect();
    let mut group = Kind::Atomic.group(c, "to_mjd", Scale::Row);
    group.throughput(policy::throughput(WIDTH, size_of::<f64>()));
    for (id, timesys) in [("utc", "UTC"), ("tt", "TT"), ("tt_tai", "TT(TAI)")] {
        group.bench_function(id, |b| {
            let axis = frame(timesys).relative_axis().unwrap();
            b.iter(|| {
                for &time in &times {
                    black_box(axis.mjd(black_box(time)).unwrap());
                }
            });
        });
    }
    group.finish();
}

/// The frame of a night's observation in `timesys`: `MJDREF` the evening of 2026-03-14, times
/// in seconds.
fn frame(timesys: &str) -> Frame {
    let mut header = Header::new();
    header
        .set_internal(keywords::TIMESYS, timesys)
        .set_internal(keywords::MJDREF, 61113.75)
        .set_internal(keywords::TIMEUNIT, "s");
    Frame::from_header(&header).unwrap()
}
