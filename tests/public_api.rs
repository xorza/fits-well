use fits_well::header::Header;
use fits_well::image::{Image, ImageData};
use fits_well::io::HduKind;
use fits_well::table::{BitVec, ColumnData, Complex, Msb0, TableBuilder, WriteColumn};
use fits_well::time::{TimeScale, TimeScaleKind};
use fits_well::{FitsError, FitsReader, FitsWriter, Result};

#[derive(Debug)]
struct I16ArrayAdapter {
    shape: Vec<usize>,
    samples: Vec<i16>,
}

fn adapt_i16(shape: &[usize], data: ImageData) -> I16ArrayAdapter {
    let ImageData::I16(samples) = data else {
        panic!("fixture is i16");
    };
    I16ArrayAdapter {
        shape: shape.to_vec(),
        samples,
    }
}

#[test]
fn canonical_api_paths_support_a_zero_copy_array_adapter() -> Result<()> {
    let image = Image::new(vec![3, 2], vec![1i16, 2, 3, 4, 5, 6])?;
    assert_eq!(image.metadata().shape, [3, 2]);
    assert!(matches!(
        image.stored(),
        fits_well::image::ImageView::I16([1, 2, 3, 4, 5, 6])
    ));

    let owned = ImageData::I16(vec![7, 8, 9]);
    let owned_ptr = match &owned {
        ImageData::I16(values) => values.as_ptr(),
        _ => unreachable!(),
    };
    let adapted = adapt_i16(&[3], owned);
    assert_eq!(adapted.shape, [3]);
    assert_eq!(adapted.samples, [7, 8, 9]);
    assert_eq!(adapted.samples.as_ptr(), owned_ptr);

    let _: Option<FitsError> = None;
    let _: Option<FitsReader<std::io::Cursor<Vec<u8>>>> = None;
    let _: Option<FitsWriter<Vec<u8>>> = None;
    let _: Header = Header::new();
    let _: HduKind = HduKind::Primary;
    let _: ColumnData = ColumnData::I32(Vec::new());
    let _: TableBuilder = TableBuilder::new();
    let _: WriteColumn = WriteColumn::scalar("VALUE", ColumnData::I32(Vec::new()));
    let _: BitVec<u8, Msb0> = BitVec::new();
    let _: Complex<f32> = Complex::new(0.0, 0.0);
    let _: TimeScale = TimeScale::Utc;
    let _: TimeScaleKind = TimeScaleKind::Utc;

    Ok(())
}
