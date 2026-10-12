//! The header case: a capture program's light header parsed from its bytes.

use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};

use crate::bench::fixture::{Kind, Scale};
use crate::bench::scene;
use crate::bitpix::Bitpix;
use crate::block::CARD_SIZE;
use crate::header_model::Header;
use crate::reader::FitsReader;
use crate::writer::FitsWriter;

/// `atomic/header/light`: the header unit of a light, as a capture program writes it, parsed into
/// a [`Header`], as the reader parses each HDU it opens. Per card of the unit, the block's blank
/// fill included. The unit is a one-pixel file's, so it is built at once.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "header", Scale::Sample);
    let unit = header_unit();
    group.throughput(Throughput::ElementsAndBytes {
        elements: (unit.len() / CARD_SIZE) as u64,
        bytes: unit.len() as u64,
    });
    group.bench_function("light", |b| {
        b.iter(|| Header::parse(black_box(&unit)).unwrap());
    });
    group.finish();
}

/// The header unit of a one-pixel light under [`scene::light_header`], up to its `END` card.
fn header_unit() -> Vec<u8> {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    let image = scene::sky(Bitpix::I16, 1, 1, 1);
    writer
        .write_image(&image, Some(&scene::light_header()))
        .unwrap();
    let file = writer.into_inner().into_inner();
    let data_offset = FitsReader::from_bytes(&file).unwrap().hdus()[0].data_offset;
    file[..usize::try_from(data_offset).unwrap()].to_vec()
}
