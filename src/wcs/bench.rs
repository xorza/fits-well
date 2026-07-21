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

fn tabular_inverse_wcs() -> Wcs {
    let coordinates = [100.0, 200.0, 110.0, 200.0, 100.0, 220.0, 110.0, 220.0];
    let mut table_header = Header::new();
    table_header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", (coordinates.len() * size_of::<f64>()) as i64)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TTYPE1", "COORD")
        .set_internal("TFORM1", format!("{}D", coordinates.len()))
        .set_internal("TDIM1", "(2,2,2)");
    let bytes = coordinates.into_iter().flat_map(f64::to_be_bytes).collect();
    let table = BinTable::from_data(&table_header, bytes).unwrap();

    let mut header = Header::new();
    header.set_internal("NAXIS", 2);
    for axis in 1..=2 {
        header
            .set_internal(format!("CTYPE{axis}").as_str(), format!("AX{axis:02}-TAB"))
            .set_internal(format!("CRPIX{axis}").as_str(), 0.0)
            .set_internal(format!("CRVAL{axis}").as_str(), 0.0)
            .set_internal(format!("CDELT{axis}").as_str(), 1.0)
            .set_internal(format!("PS{axis}_0").as_str(), "WCS-TABLE")
            .set_internal(format!("PS{axis}_1").as_str(), "COORD")
            .set_internal(format!("PV{axis}_3").as_str(), axis as i64);
    }
    let transforms = tabular::descriptors(&header, 2, None)
        .unwrap()
        .into_iter()
        .map(|descriptor| tabular::TabularTransform::from_table(descriptor, &table).unwrap())
        .collect();
    Wcs::from_header_with_tabular(&header, None, transforms).unwrap()
}
