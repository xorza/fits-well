//! Typed time coordinates (§9).
//!
//! Covers FITS ISO-8601 datetimes → Julian Date / MJD (strict year forms and
//! proleptic-Gregorian math), `J`/`B` epochs → JD, declared time-scale metadata,
//! and a [`FitsTime`] view over a header's time keywords
//! (`TIMESYS`, `MJDREF*`/`JDREF*`/`DATEREF`, `TIMEUNIT`, resolved
//! `TREFPOS`/`TRPOSn`, all image/table PHASE forms, and the global
//! `DATE-OBS`/`MJD-OBS`/`TSTART`/… set). Conversion between declared time
//! frames requires external ephemeris and Earth-orientation data and is outside
//! this crate.

pub(crate) mod datetime;
pub(crate) mod fits_time;
pub(crate) mod phase_axis;
pub(crate) mod time_axis_kind;
pub(crate) mod time_bounds;
pub(crate) mod time_coordinate;
pub(crate) mod time_reference_position;
pub(crate) mod time_scale;

/// JD of the MJD zero point (1858-11-17T00:00 UTC).
const MJD0: f64 = 2_400_000.5;
const SEC_PER_DAY: f64 = 86_400.0;

#[cfg(test)]
mod tests;
