//! The criterion benches, which `benches/fits_well.rs` runs: `cargo drive bench`.
//!
//! Two kinds, and every case is one. An `atomic` case times one operation on the unit the read
//! and write paths hand it: one row of the standard frame for a sample kernel, a checksum, the
//! quantizer or a codec, whose default tile is one row; one header for the header and WCS
//! parsers; one row of pixels for a WCS transform, and of times for the time frame; one table
//! column for a column read; one row of an ASCII table for its field parser and formatter. An `integration` case times a public entry point end to end on a scene input (the
//! standard frame, its mask, its catalog, a table of spectra or a mosaic), from opening the
//! source or the sink to the last sample or byte, as a caller pays.
//!
//! A group is one operation. An atomic id is `atomic/<operation>/<variant>/<input>`, the variant
//! left out where the operation has one. An integration id is
//! `integration/<operation>/<source|sink>/<storage>/<input>`: a read's source is `stream` (over a
//! `File`) or `slice` (bytes in memory), a write's sink `file` or `memory`; the storage is
//! `plain` or a codec, so plain and compressed results sit side by side, and every operation
//! runs on every storage its input takes (`scene_file::SceneFile`). Each case states its
//! throughput in samples or rows and in the uncompressed bytes they take, or in elements alone
//! where the bytes vary or do not apply: an HDU scanned, a table cell, a header edited. A case builds its input in its
//! own closure, from the scene's fixed seeds, so a filtered run builds only what it times; files
//! go to a scratch directory and stay in the page cache, so a read times the decode, not the
//! disk. A whole read is checked once against the scene before it is timed.

pub(crate) mod policy;
pub(crate) mod scene_file;

use criterion::Criterion;

#[cfg(feature = "compression")]
use crate::compression;
use crate::{
    ascii, bintable, fits_editor, fits_writer, header_model, image_model, reader, time_coordinates,
    world_coordinates,
};

/// Every case, atomic first.
pub fn run(c: &mut Criterion) {
    image_model::bench::atomic(c);
    reader::bench::atomic(c);
    header_model::bench::atomic(c);
    bintable::bench::atomic(c);
    ascii::bench::atomic(c);
    #[cfg(feature = "compression")]
    compression::bench::atomic(c);
    world_coordinates::bench::atomic(c);
    time_coordinates::bench::atomic(c);

    reader::bench::integration(c);
    fits_writer::bench::integration(c);
    fits_editor::bench::integration(c);
}
