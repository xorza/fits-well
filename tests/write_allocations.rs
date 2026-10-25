//! The heap a binary-table write holds beside its sink, under the counting allocator: a
//! bounded chunk of the data unit, never the whole unit.

#![expect(
    clippy::disallowed_types,
    reason = "a global allocator has no owner to carry its counters"
)]

use std::io;

use fits_well::FitsWriter;
use fits_well::table::{Builder, ColumnData, WriteColumn};

#[path = "support/counting.rs"]
#[expect(dead_code, reason = "this test counts the bytes held, not the calls")]
mod counting;

use crate::counting::ALLOCATOR;

/// A table of 8 MiB of rows, written with checksums to a sink that keeps nothing, peaks at
/// the 1 MiB chunk the writer sums and writes the unit through, and the header it renders:
/// the unit is never staged whole.
#[test]
fn table_write_holds_one_chunk_not_the_unit() {
    const ROWS: usize = 1 << 20;
    let values = ColumnData::F64((0..ROWS).map(|row| row as f64).collect());
    let table = Builder::with_columns(ROWS, vec![WriteColumn::scalar("X", values)]).unwrap();
    let mut writer = FitsWriter::new(io::sink()).with_checksums();
    let before = ALLOCATOR.restart_peak();
    writer.write_table(&table, None).unwrap();
    let held = ALLOCATOR.peak() - before;
    // The chunk is 2¹⁷ rows of 8 bytes, exactly 1 MiB; the header the writer builds and
    // renders takes less than four blocks beside it. The unit is 8 MiB.
    let chunk = 1 << 20;
    assert!(
        (chunk..chunk + 4 * 2880).contains(&held),
        "{held} bytes held"
    );
}
