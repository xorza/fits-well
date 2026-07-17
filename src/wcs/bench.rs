//! Criterion entry points for WCS transform throughput.

use std::sync::OnceLock;

use crate::header::Header;
use crate::table_impl::BinTable;
use crate::wcs::Wcs;
use crate::wcs::tabular;

const BATCH_SIZE: usize = 1024;
const TAB_INDEX_LENGTH: usize = 100_000;

static SPECTRAL: OnceLock<Wcs> = OnceLock::new();
static TABULAR: OnceLock<Wcs> = OnceLock::new();

pub(crate) fn prepare() {
    SPECTRAL.get_or_init(spectral_wcs);
    TABULAR.get_or_init(tabular_wcs);
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

fn tabular_wcs() -> Wcs {
    let mut table_header = Header::new();
    let field_width = TAB_INDEX_LENGTH * 8;
    table_header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", (2 * field_width) as i64)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 2)
        .set_internal("TTYPE1", "COORD")
        .set_internal("TFORM1", format!("{TAB_INDEX_LENGTH}D"))
        .set_internal("TDIM1", format!("(1,{TAB_INDEX_LENGTH})"))
        .set_internal("TTYPE2", "INDEX")
        .set_internal("TFORM2", format!("{TAB_INDEX_LENGTH}D"));
    let mut bytes = Vec::with_capacity(2 * field_width);
    bytes.extend((0..TAB_INDEX_LENGTH).flat_map(|index| (index as f64 * 0.25).to_be_bytes()));
    bytes.extend((0..TAB_INDEX_LENGTH).flat_map(|index| (index as f64 * 2.0).to_be_bytes()));
    let table = BinTable::from_data(&table_header, bytes).unwrap();

    let mut header = Header::new();
    header
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "m")
        .set_internal("CRPIX1", 0.0)
        .set_internal("CRVAL1", 0.0)
        .set_internal("CDELT1", 1.0)
        .set_internal("PS1_0", "WCS-TABLE")
        .set_internal("PS1_1", "COORD")
        .set_internal("PS1_2", "INDEX")
        .set_internal("PV1_3", 1);
    let transforms = tabular::descriptors(&header, 1, None)
        .unwrap()
        .into_iter()
        .map(|descriptor| tabular::TabularTransform::from_table(descriptor, &table).unwrap())
        .collect();
    Wcs::from_header_with_tabular(&header, None, transforms).unwrap()
}
