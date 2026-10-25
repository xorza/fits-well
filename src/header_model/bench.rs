//! The header cases: a capture program's light header parsed from its bytes, and rendered
//! back.

use std::hint::black_box;

use criterion::Criterion;

use crate::bench::policy::{self, Kind, Scale};
use crate::block::CARD_SIZE;
use crate::header_model::{Header, Signing, Space};
use crate::image_model::bitpix::Bitpix;
use crate::internals::memory;
use crate::internals::scene;

/// `atomic/parse_header/light` and `atomic/render_header/light`: the header unit of a light,
/// as a capture program writes it, parsed into a [`Header`], as the reader parses each HDU it
/// opens, and that header rendered into a reused buffer, as the writer renders each header it
/// writes. Per card of the unit, the block's blank fill included. The unit is a one-pixel
/// file's, so it is built at once.
pub(crate) fn atomic(c: &mut Criterion) {
    let unit = header_unit();
    let header = Header::parse(&unit).unwrap();
    let throughput = policy::throughput(unit.len() / CARD_SIZE, CARD_SIZE);

    let mut group = Kind::Atomic.group(c, "parse_header", Scale::Row);
    group.throughput(throughput.clone());
    group.bench_function("light", |b| {
        b.iter(|| Header::parse(black_box(&unit)).unwrap());
    });
    group.finish();

    let mut group = Kind::Atomic.group(c, "render_header", Scale::Row);
    group.throughput(throughput);
    let mut out = Vec::with_capacity(unit.len());
    group.bench_function("light", |b| {
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
    let image = scene::sky(Bitpix::I16, 1, 1, 1);
    let file = memory::written_by(memory::memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), Some(&scene::light_header()))
    });
    let data_offset = file.slice().hdus()[0].data_offset;
    file[..usize::try_from(data_offset).unwrap()].to_vec()
}
