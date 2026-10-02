//! Criterion group for WCS transform throughput.

use std::hint::black_box;

use criterion::{Criterion, Throughput};

use crate::header_model::Header;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular::internals::{INDEX_LENGTH, indexed_wcs};

const BATCH_SIZE: usize = 1024;

/// `wcs` — one batch of pixels forward and back through a four-axis linear WCS,
/// forward through a Table-26 spectral axis, and forward through a 100 000-entry
/// `-TAB` index.
pub fn wcs(c: &mut Criterion) {
    let linear = linear_wcs();
    let spectral = spectral_wcs();
    let tabular = indexed_wcs();
    let mut group = c.benchmark_group("wcs");
    group.throughput(Throughput::Elements(BATCH_SIZE as u64));
    group.bench_function("linear_4d_round_trip", |bench| {
        bench.iter(|| {
            black_box(
                (0..BATCH_SIZE)
                    .map(|index| {
                        let step = index as f64 * 0.001;
                        let pixel = [11.0 + step, -4.0 - step, 8.0 + step, 23.0 - step];
                        let world = linear.pixel_to_world(&pixel).unwrap();
                        linear
                            .world_to_pixel(&world)
                            .unwrap()
                            .into_iter()
                            .sum::<f64>()
                    })
                    .sum::<f64>(),
            )
        });
    });
    group.bench_function("spectral", |bench| {
        bench.iter(|| {
            black_box(
                (0..BATCH_SIZE)
                    .map(|index| {
                        spectral
                            .pixel_to_world(&[1.0 + index as f64 * 0.001])
                            .unwrap()[0]
                    })
                    .sum::<f64>(),
            )
        });
    });
    let span = 2 * (INDEX_LENGTH - 1);
    group.bench_function("tabular_index_100k", |bench| {
        bench.iter(|| {
            black_box(
                (0..BATCH_SIZE)
                    .map(|index| {
                        let pixel = (index * 7919 % span) as f64;
                        tabular.pixel_to_world(&[pixel]).unwrap()[0]
                    })
                    .sum::<f64>(),
            )
        });
    });
    group.finish();
}

fn spectral_wcs() -> Wcs {
    let mut header = Header::new();
    header
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "FREQ-W2F")
        .set_internal("CUNIT1", "Hz")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 1_420_405_751.0)
        .set_internal("CDELT1", 1e6)
        .set_internal("RESTFRQ", 1_420_405_751.0);
    Wcs::from_header(&header, None).unwrap()
}

fn linear_wcs() -> Wcs {
    let mut header = Header::new();
    header
        .set_internal("NAXIS", 4)
        .set_internal("CTYPE1", "LINEAR")
        .set_internal("CTYPE2", "LINEAR")
        .set_internal("CTYPE3", "LINEAR")
        .set_internal("CTYPE4", "LINEAR")
        .set_internal("CRPIX1", 10.0)
        .set_internal("CRPIX2", -3.0)
        .set_internal("CRPIX3", 5.0)
        .set_internal("CRPIX4", 20.0)
        .set_internal("CRVAL1", 100.0)
        .set_internal("CRVAL2", -20.0)
        .set_internal("CRVAL3", 0.5)
        .set_internal("CRVAL4", 1_000.0)
        .set_internal("CDELT1", 0.25)
        .set_internal("CDELT2", 2.0)
        .set_internal("CDELT3", -0.5)
        .set_internal("CDELT4", 4.0)
        .set_internal("PC1_2", 0.1)
        .set_internal("PC2_3", -0.2)
        .set_internal("PC3_4", 0.3)
        .set_internal("PC4_1", -0.4);
    Wcs::from_header(&header, None).unwrap()
}
