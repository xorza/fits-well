//! The column cases: one column of a table decoded from its big-endian cells.

use std::hint::black_box;

use criterion::Criterion;

use crate::bench::policy::{self, Kind, Scale};
use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
use crate::fits_writer::builder::Builder;
use crate::image_model::bitpix::Bitpix;
use crate::internals::memory;
use crate::internals::scene::{self, CATALOG_ROWS};
use crate::ragged::Ragged;

/// `atomic/read_column/fixed/<type>`: one column of the source catalog decoded into a reused
/// buffer, as a column read fills a caller's: `NUMBER` for i32, `FLAGS` for i16, `FLUX` for f32
/// and `RA` for f64. `atomic/read_column/vla/f32`: the spectra's `FLUX` array column, its
/// descriptors followed into the heap. Each read is checked once to hold every row.
/// Per element, in its bytes.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "read_column", Scale::Frame);
    let mut catalog: Option<Table> = None;
    for (bitpix, column) in [
        (Bitpix::I16, "FLAGS"),
        (Bitpix::I32, "NUMBER"),
        (Bitpix::F32, "FLUX"),
        (Bitpix::F64, "RA"),
    ] {
        group.throughput(policy::throughput(CATALOG_ROWS, bitpix.elem_size()));
        let id = format!("fixed/{}", policy::bitpix_id(bitpix));
        group.bench_function(id, |b| {
            let table = catalog.get_or_insert_with(|| table(&scene::catalog()));
            let reader = table.column(column).unwrap();
            let mut out = ColumnData::U8(Vec::new());
            reader.raw_into(&mut out).unwrap();
            assert_eq!(out.len(), CATALOG_ROWS, "{column}");
            b.iter(|| {
                reader.raw_into(&mut out).unwrap();
                black_box(&out);
            });
        });
    }

    let samples: usize = scene::spectra_lengths().sum();
    group.throughput(policy::throughput(samples, Bitpix::F32.elem_size()));
    group.bench_function(format!("vla/{}", policy::bitpix_id(Bitpix::F32)), |b| {
        let spectra = table(&scene::spectra());
        let reader = spectra.column("FLUX").unwrap();
        let mut out = Ragged::<ColumnData>::new(ColumnData::U8(Vec::new()), Vec::new());
        reader.vla_into(&mut out).unwrap();
        assert_eq!(out.len(), scene::SPECTRA_ROWS, "FLUX");
        assert_eq!(
            out.values().len(),
            samples,
            "the throughput counts each sample"
        );
        b.iter(|| {
            reader.vla_into(&mut out).unwrap();
            black_box(&out);
        });
    });
    group.finish();
}

/// The table `builder` writes, read back from its bytes.
fn table(builder: &Builder) -> Table {
    let written = memory::written_by(memory::memory_writer(), |w| w.write_table(builder, None));
    written.slice().read_table(1).unwrap()
}
