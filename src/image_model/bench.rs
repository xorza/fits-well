//! The sample cases, on one row of the standard frame: the big-endian swap both ways, the
//! physical scaling, and the unsigned reinterpretation.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion};

use crate::bench::fixture::{self, Kind, Scale};
use crate::bitpix::Bitpix;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::Borrowed;
use crate::internals::samples::BITPIXES;
use crate::internals::scene::{self, WIDTH};

/// `atomic/decode/<type>`, `atomic/encode/<type>`, `atomic/physical/<scaling>/<type>` and
/// `atomic/unsigned/<type>`, each on one row of the standard frame. Per sample, in the stored
/// bytes.
pub(crate) fn atomic(c: &mut Criterion) {
    decode(c);
    encode(c);
    physical(c);
    unsigned(c);
}

/// `atomic/decode/<type>`: the big-endian row swapped into a reused buffer of words, as a view
/// read decodes. `u8` has no swap, so no case.
fn decode(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "decode", Scale::Sample);
    let mut words = Vec::new();
    for bitpix in swapped_types() {
        group.throughput(fixture::throughput(WIDTH, bitpix.elem_size()));
        group.bench_function(fixture::bitpix_id(bitpix), |b| {
            let mut row = Vec::new();
            scene::sky(bitpix, WIDTH, 1, 1)
                .samples
                .view()
                .encode_into(&mut row);
            b.iter(|| bitpix.swap_into_words(black_box(&row), &mut words));
        });
    }
    group.finish();
}

/// `atomic/encode/<type>`: the row appended big-endian to a reused buffer, as the writer stages
/// a data unit. `u8` is a copy, so no case.
fn encode(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "encode", Scale::Sample);
    let mut out = Vec::new();
    for bitpix in swapped_types() {
        group.throughput(fixture::throughput(WIDTH, bitpix.elem_size()));
        group.bench_function(fixture::bitpix_id(bitpix), |b| {
            let row = scene::sky(bitpix, WIDTH, 1, 1);
            b.iter(|| {
                out.clear();
                black_box(row.samples.view()).encode_into(&mut out);
            });
        });
    }
    group.finish();
}

/// `atomic/physical/<scaling>/<type>`: `BZERO + BSCALE × sample` into a new `f64` row, under the
/// frame's own scaling (`stored`) and, for an integer, with a `BLANK` sentinel too (`blank`),
/// the branch a sentinel adds.
fn physical(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "physical", Scale::Sample);
    for bitpix in BITPIXES {
        let scalings: &[(&str, Option<i64>)] = if bitpix.is_integer() {
            &[("stored", None), ("blank", Some(0))]
        } else {
            &[("stored", None)]
        };
        for &(name, blank) in scalings {
            group.throughput(fixture::throughput(WIDTH, bitpix.elem_size()));
            let id = BenchmarkId::new(name, fixture::bitpix_id(bitpix));
            group.bench_function(id, |b| {
                let row = scene::sky(bitpix, WIDTH, 1, 1);
                let scaling = Scaling {
                    blank,
                    ..row.scaling
                };
                b.iter(|| black_box(row.samples.view()).physical::<f64>(&scaling));
            });
        }
    }
    group.finish();
}

/// `atomic/unsigned/<type>`: the exact reinterpretation of a stored signed integer as its
/// unsigned type under the FITS sign-bit `BZERO` (a `u8` as signed bytes), into a new row.
fn unsigned(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "unsigned", Scale::Sample);
    for bitpix in BITPIXES.into_iter().filter(|bitpix| bitpix.is_integer()) {
        group.throughput(fixture::throughput(WIDTH, bitpix.elem_size()));
        group.bench_function(fixture::bitpix_id(bitpix), |b| {
            let row = scene::sky(bitpix, WIDTH, 1, 1);
            let scaling = Scaling {
                bscale: 1.0,
                bzero: sign_bit_offset(bitpix),
                blank: None,
            };
            let view = Borrowed {
                shape: &row.shape,
                scaling,
                samples: row.samples.view(),
            };
            b.iter(|| black_box(view).unsigned().unwrap());
        });
    }
    group.finish();
}

/// Every type but `u8`, which is stored as it is.
fn swapped_types() -> impl Iterator<Item = Bitpix> {
    BITPIXES.into_iter().filter(|&bitpix| bitpix != Bitpix::U8)
}

/// The `BZERO` that makes `bitpix` the unsigned convention's type (§5.2.5).
fn sign_bit_offset(bitpix: Bitpix) -> f64 {
    match bitpix {
        Bitpix::U8 => -128.0,
        Bitpix::I16 => 32768.0,
        Bitpix::I32 => 2f64.powi(31),
        Bitpix::I64 => 2f64.powi(63),
        Bitpix::F32 | Bitpix::F64 => unreachable!("a float has no unsigned convention"),
    }
}
