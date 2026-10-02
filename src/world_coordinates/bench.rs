//! Criterion entry points for WCS transform throughput.

use std::sync::OnceLock;

use crate::header_model::Header;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular::internals::{
    COUPLED_GRID, lookup_table, resolved_wcs, tab_header,
};

const BATCH_SIZE: usize = 1024;
const TAB_INDEX_LENGTH: usize = 100_000;

static SPECTRAL: OnceLock<Wcs> = OnceLock::new();
static TABULAR: OnceLock<Wcs> = OnceLock::new();
static TABULAR_INVERSE: OnceLock<Wcs> = OnceLock::new();
static LINEAR: OnceLock<Wcs> = OnceLock::new();

pub(crate) fn prepare() {
    SPECTRAL.get_or_init(spectral_wcs);
    TABULAR.get_or_init(tabular_wcs);
    TABULAR_INVERSE.get_or_init(tabular_inverse_wcs);
    LINEAR.get_or_init(linear_wcs);
}

pub(crate) fn linear_round_trip_batch() -> f64 {
    let wcs = LINEAR.get_or_init(linear_wcs);
    (0..BATCH_SIZE)
        .map(|index| {
            let step = index as f64 * 0.001;
            let pixel = [11.0 + step, -4.0 - step, 8.0 + step, 23.0 - step];
            let world = wcs.pixel_to_world(&pixel).unwrap();
            wcs.world_to_pixel(&world).unwrap().into_iter().sum::<f64>()
        })
        .sum()
}

pub(crate) fn spectral_batch() -> f64 {
    let wcs = SPECTRAL.get_or_init(spectral_wcs);
    (0..BATCH_SIZE)
        .map(|index| {
            let pixel = 1.0 + index as f64 * 0.001;
            wcs.pixel_to_world(&[pixel]).unwrap()[0]
        })
        .sum()
}

pub(crate) fn tabular_index_batch() -> f64 {
    let wcs = TABULAR.get_or_init(tabular_wcs);
    let span = 2 * (TAB_INDEX_LENGTH - 1);
    (0..BATCH_SIZE)
        .map(|index| {
            let pixel = (index * 7919 % span) as f64;
            wcs.pixel_to_world(&[pixel]).unwrap()[0]
        })
        .sum()
}

pub(crate) fn tabular_forward_at_pixel(pixel: f64) -> f64 {
    TABULAR
        .get_or_init(tabular_wcs)
        .pixel_to_world(&[pixel])
        .unwrap()[0]
}

pub(crate) fn tabular_inverse_at_world(world: f64) -> f64 {
    TABULAR
        .get_or_init(tabular_wcs)
        .world_to_pixel(&[world])
        .unwrap()[0]
}

pub(crate) fn tabular_inverse_at_fraction(fraction: f64) -> f64 {
    let wcs = TABULAR_INVERSE.get_or_init(tabular_inverse_wcs);
    let world = [100.0 + 10.0 * fraction, 200.0 + 20.0 * fraction];
    wcs.world_to_pixel(&world).unwrap().into_iter().sum()
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

fn tabular_wcs() -> Wcs {
    let coordinates: Vec<f64> = (0..TAB_INDEX_LENGTH)
        .map(|index| index as f64 * 0.25)
        .collect();
    let index: Vec<f64> = (0..TAB_INDEX_LENGTH)
        .map(|index| index as f64 * 2.0)
        .collect();
    let shape = format!("(1,{TAB_INDEX_LENGTH})");
    let table = lookup_table(&[
        ("COORD", &coordinates, Some(shape.as_str())),
        ("INDEX", &index, None),
    ]);
    let mut header = tab_header(1, "COORD");
    header
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "m")
        .set_internal("PS1_2", "INDEX");
    resolved_wcs(&header, &table)
}

fn tabular_inverse_wcs() -> Wcs {
    let table = lookup_table(&[("COORD", &COUPLED_GRID, Some("(2,2,2)"))]);
    resolved_wcs(&tab_header(2, "COORD"), &table)
}
