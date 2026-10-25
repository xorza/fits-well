//! The ASCII field cases: one row of the source catalog as an ASCII table, parsed and
//! formatted.

use std::hint::black_box;

use criterion::Criterion;

use crate::ascii::{Ascii, FieldScaling};
use crate::bench::policy::{self, Kind, Scale};
use crate::fits_writer::ascii_builder::AsciiBuilder;
use crate::internals::memory;
use crate::internals::scene::{self, ASCII_CATALOG_ROW_BYTES};

/// `atomic/parse_ascii/catalog`: the fields of one row of the ASCII catalog parsed to their
/// physical values, as a column read parses each of its rows. `atomic/format_ascii/catalog`:
/// that row's values formatted into its fields, as the writer formats each row, into a reused
/// buffer, checked once to give the row's bytes. Per row, in its bytes.
pub(crate) fn atomic(c: &mut Criterion) {
    let throughput = policy::throughput(1, ASCII_CATALOG_ROW_BYTES);
    let mut built: Option<Catalog> = None;

    let mut group = Kind::Atomic.group(c, "parse_ascii", Scale::Row);
    group.throughput(throughput.clone());
    group.bench_function("catalog", |b| {
        let Catalog { table, .. } = built.get_or_insert_with(Catalog::new);
        let scalings: Vec<FieldScaling<'_>> = table
            .schema
            .columns
            .iter()
            .map(|col| FieldScaling::of(col).unwrap())
            .collect();
        b.iter(|| parse_row(black_box(table), &scalings));
    });
    group.finish();

    let mut group = Kind::Atomic.group(c, "format_ascii", Scale::Row);
    group.throughput(throughput);
    group.bench_function("catalog", |b| {
        let Catalog { builder, table } = built.get_or_insert_with(Catalog::new);
        let mut number = String::new();
        let mut row = Vec::with_capacity(ASCII_CATALOG_ROW_BYTES);
        builder.encode_row(0, &mut number, &mut row).unwrap();
        assert_eq!(
            row,
            table.rows.as_bytes()[..ASCII_CATALOG_ROW_BYTES],
            "the first row"
        );
        b.iter(|| {
            row.clear();
            builder.encode_row(0, &mut number, &mut row).unwrap();
            black_box(&row);
        });
    });
    group.finish();
}

/// The ASCII catalog, as the writer takes it and as the reader gives it back.
#[derive(Debug)]
struct Catalog {
    builder: AsciiBuilder,
    table: Ascii,
}

impl Catalog {
    fn new() -> Self {
        let builder = scene::ascii_catalog();
        let written = memory::written_by(memory::memory_writer(), |w| {
            w.write_ascii_table(&builder, None)
        });
        let table = written.slice().read_ascii_table(1).unwrap();
        Self { builder, table }
    }
}

/// Parses each field of the first row of `table`, whose columns `scalings` scale.
fn parse_row(table: &Ascii, scalings: &[FieldScaling<'_>]) {
    for (column, &scaling) in scalings.iter().enumerate() {
        black_box(table.physical_value(column, scaling, 0).unwrap());
    }
}
