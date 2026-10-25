//! Typed time coordinates (§9).
//!
//! Covers FITS ISO-8601 datetimes → MJD (strict year forms and
//! proleptic-Gregorian math), `J`/`B` epochs → MJD, declared time-scale metadata,
//! and a [`Frame`](frame::Frame) view over a header's time keywords
//! (`TIMESYS`, `MJDREF*`/`JDREF*`/`DATEREF`, `TIMEUNIT`, resolved
//! `TREFPOS`/`TRPOSn`, all image/table PHASE forms, and the global
//! `DATE-OBS`/`MJD-OBS`/`TSTART`/… set). Conversion between declared time
//! frames requires external ephemeris and Earth-orientation data and is outside
//! this crate.

pub(crate) mod axis;
#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod bounds;
pub(crate) mod coordinate;
pub(crate) mod datetime;
pub(crate) mod frame;
pub(crate) mod mjd;
pub(crate) mod phase_axis;
pub(crate) mod reference_position;
pub(crate) mod scale;
pub(crate) mod time_axis_kind;
pub(crate) mod unit;
