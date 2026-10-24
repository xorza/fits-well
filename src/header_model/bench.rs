//! The header cases: a capture program's light header parsed from its bytes, and rendered
//! back.

use std::hint::black_box;
use std::io::Cursor;

use criterion::Criterion;

use crate::bench::fixture::{self, Kind, Scale};
use crate::bitpix::Bitpix;
use crate::block::CARD_SIZE;
use crate::fits_writer::FitsWriter;
use crate::header_model::{Header, Signing, Space};
use crate::internals::scene;
use crate::reader::FitsReader;

/// `atomic/header/light` and `atomic/header/render`: the header unit of a light, as a capture
/// program writes it, parsed into a [`Header`], as the reader parses each HDU it opens, and that
/// header rendered into a reused buffer, as the writer renders each header it writes. Per card of
/// the unit, the block's blank fill included. The unit is a one-pixel file's, so it is built at
/// once.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "header", Scale::Sample);
    let unit = header_unit();
    group.throughput(fixture::throughput(unit.len() / CARD_SIZE, CARD_SIZE));
    group.bench_function("light", |b| {
        b.iter(|| Header::parse(black_box(&unit)).unwrap());
    });
    let header = Header::parse(&unit).unwrap();
    let mut out = Vec::with_capacity(unit.len());
    group.bench_function("render", |b| {
        b.iter(|| {
            black_box(&header)
                .render_into(Signing::Unsigned, Space::Spare(0), &mut out)
                .unwrap();
        });
    });
    group.finish();
}

/// The header unit of a one-pixel light under [`scene::light_header`], up to its `END` card.
fn header_unit() -> Vec<u8> {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    let image = scene::sky(Bitpix::I16, 1, 1, 1);
    writer
        .write_image(image.view(), Some(&scene::light_header()))
        .unwrap();
    let file = writer.into_inner().into_inner();
    let data_offset = FitsReader::from_bytes(&file).unwrap().hdus()[0].data_offset;
    file[..usize::try_from(data_offset).unwrap()].to_vec()
}
