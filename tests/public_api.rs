use fits_well::header::Header;
use std::io::Cursor;

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use fits_well::image::{Borrowed, Data, Image, PhysicalSample, Region, View};
use fits_well::io::HduKind;
use fits_well::table::{Builder, ColumnData, WriteColumn};
use fits_well::time::{Scale, ScaleKind};
use fits_well::{FitsError, FitsReader, FitsWriter, ImageReader, Result};
use num_complex::Complex;

/// Every published type sits at its canonical path; this compiles or it does not.
#[test]
#[expect(
    clippy::redundant_type_annotations,
    reason = "each annotation names a published type at its canonical path"
)]
fn canonical_api_paths_resolve() -> Result<()> {
    let image = Image::new(vec![3, 2], vec![1i16, 2, 3, 4, 5, 6])?;
    assert_eq!(image.view().metadata().shape, [3, 2]);
    assert!(matches!(
        image.view().samples(),
        View::I16([1, 2, 3, 4, 5, 6])
    ));

    let _: Data = Data::I16(vec![7, 8, 9]);
    let _: Option<FitsError> = None;
    let _: Option<FitsReader<Cursor<Vec<u8>>>> = None;
    let _: Option<ImageReader<'_, Cursor<Vec<u8>>>> = None;
    let _: Region<'_> = Region::All;
    let _: Borrowed<'_> = image.view();
    let mut physical: Vec<f32> = Vec::new();
    appends(&image, &mut physical);
    assert_eq!(physical, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let _: Option<FitsWriter<Vec<u8>>> = None;
    let _: Header = Header::new();
    let _: HduKind = HduKind::Primary;
    let _: ColumnData = ColumnData::I32(Vec::new());
    let _: Builder = Builder::new();
    let _: WriteColumn = WriteColumn::scalar("VALUE", ColumnData::I32(Vec::new()));
    let _: BitVec<u8, Msb0> = BitVec::new();
    let _: Complex<f32> = Complex::new(0.0, 0.0);
    let _: Scale = Scale::known(ScaleKind::Utc);
    let _: ScaleKind = ScaleKind::Utc;

    Ok(())
}

/// The physical plane of `image`, appended to `out`: a sample type a caller names only as a
/// bound.
fn appends<T: PhysicalSample>(image: &Image, out: &mut Vec<T>) {
    image.view().physical_into(out);
}
