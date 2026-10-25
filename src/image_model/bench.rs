//! The sample cases, on one row of the standard frame: the big-endian swap both ways, the
//! physical scaling, and the unsigned reinterpretation.

use std::hint::black_box;

use criterion::{Bencher, BenchmarkId, Criterion};

use crate::bench::policy::{self, Kind, Scale};
use crate::image_model::bitpix::Bitpix;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::scaling::Scaling;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::view::Borrowed;
use crate::internals::samples::BITPIXES;
use crate::internals::scene;
use crate::internals::scene::WIDTH;

/// `atomic/decode/<type>`, `atomic/encode/<type>`, `atomic/physical/<output>/<scaling>-<type>` and
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
    let mut group = Kind::Atomic.group(c, "decode", Scale::Row);
    let mut words = Vec::new();
    for bitpix in swapped_types() {
        group.throughput(policy::throughput(WIDTH, bitpix.elem_size()));
        group.bench_function(policy::bitpix_id(bitpix), |b| {
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
    let mut group = Kind::Atomic.group(c, "encode", Scale::Row);
    let mut out = Vec::new();
    for bitpix in swapped_types() {
        group.throughput(policy::throughput(WIDTH, bitpix.elem_size()));
        group.bench_function(policy::bitpix_id(bitpix), |b| {
            let row = scene::sky(bitpix, WIDTH, 1, 1);
            b.iter(|| {
                out.clear();
                black_box(row.samples.view()).encode_into(&mut out);
            });
        });
    }
    group.finish();
}

/// `atomic/physical/<output>/<scaling>-<type>`: `BZERO + BSCALE × sample` straight from the
/// big-endian row into a reused row of `f32` or `f64`, the one pass a physical read makes,
/// under the frame's own scaling (`stored`) and, for an integer, with a `BLANK` sentinel too
/// (`blank`), the branch a sentinel adds.
fn physical(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "physical", Scale::Row);
    for bitpix in BITPIXES {
        let scalings: &[(&str, Option<i64>)] = if bitpix.is_integer() {
            &[("stored", None), ("blank", Some(0))]
        } else {
            &[("stored", None)]
        };
        for &(name, blank) in scalings {
            group.throughput(policy::throughput(WIDTH, bitpix.elem_size()));
            let input = format!("{name}-{}", policy::bitpix_id(bitpix));
            group.bench_function(BenchmarkId::new("f32", &input), |b| {
                physical_case::<f32>(b, bitpix, blank);
            });
            group.bench_function(BenchmarkId::new("f64", &input), |b| {
                physical_case::<f64>(b, bitpix, blank);
            });
        }
    }
    group.finish();
}

/// Times one row of the standard frame of `bitpix`, under its scaling with `blank`, to
/// physical values of `O`.
fn physical_case<O: PhysicalSample>(b: &mut Bencher<'_>, bitpix: Bitpix, blank: Option<i64>) {
    let row = scene::sky(bitpix, WIDTH, 1, 1);
    let scaling = Scaling {
        blank,
        ..row.scaling
    };
    let mut bytes = Vec::new();
    row.samples.view().encode_into(&mut bytes);
    let mut out: Vec<O> = Vec::new();
    b.iter(|| {
        out.clear();
        bitpix.extend_physical_be(black_box(&bytes), &scaling, &mut out);
    });
}

/// `atomic/unsigned/<type>`: the exact reinterpretation of a stored signed integer as its
/// unsigned type under the FITS sign-bit `BZERO` (a `u8` as signed bytes), into a new row:
/// the case times that row's allocation too, until the unsigned read fills the caller's
/// buffer.
fn unsigned(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "unsigned", Scale::Row);
    for bitpix in BITPIXES.into_iter().filter(|bitpix| bitpix.is_integer()) {
        group.throughput(policy::throughput(WIDTH, bitpix.elem_size()));
        group.bench_function(policy::bitpix_id(bitpix), |b| {
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
            let mut unsigned = UnsignedData::U16(Vec::new());
            b.iter(|| {
                assert!(black_box(view).unsigned_into(&mut unsigned));
                black_box(&unsigned);
            });
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
