//! The criterion benches, which `benches/fits_well.rs` runs: `./x.py bench`.
//!
//! Two kinds, and every case is one. An `atomic` case times one operation on the unit the read
//! and write paths hand it: one row of the standard frame for a sample kernel, a checksum or a
//! codec, whose default tile is one row; one header for the parser; one row of pixels for a WCS
//! transform. An `integration` case times a public entry point end to end on the standard frame,
//! from opening the source to the last sample, as a caller pays.
//!
//! A case's id is `<kind>/<group>/<case>`, with the same depth across a group; the group is the
//! operation, and the case its variant, where the group times one operation several ways, then its
//! input. A read's variant is its source (`stream` over a `File`, `slice` over bytes in memory,
//! `mmap`), a write's the `file` it writes. Each case states its throughput in samples and in the
//! uncompressed bytes they take. A case builds its input in its own closure, from the scene's fixed
//! seeds, so a filtered run builds only what it times; files go to a scratch directory and stay in
//! the page cache, so a read times the decode, not the disk.

pub(crate) mod fixture;
pub(crate) mod scene;

use criterion::Criterion;

#[cfg(feature = "compression")]
use crate::compress;
use crate::{fits_writer, header_model, image_model, reader, world_coordinates};

/// Every case, atomic first.
pub fn run(c: &mut Criterion) {
    image_model::bench::atomic(c);
    reader::bench::atomic(c);
    header_model::bench::atomic(c);
    #[cfg(feature = "compression")]
    compress::bench::atomic(c);
    world_coordinates::bench::atomic(c);

    reader::bench::integration(c);
    fits_writer::bench::integration(c);
    #[cfg(feature = "compression")]
    compress::bench::integration(c);
}
