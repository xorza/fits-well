//! [`ColumnElement`]: the element types a binary-table column holds as one case.

use num_complex::Complex;

use crate::bintable::column_data::ColumnData;
use crate::bintable::column_view::ColumnView;

/// An element type a binary-table column holds as one [`ColumnData`] case: `Option<bool>`
/// for `L`, `u8` for `B`, the integer, real and complex widths. Sealed; it gives
/// [`ColumnData`] and [`ColumnView`] their typed conversions. An `A` column's bytes and
/// an `X` column's bits are cases of their own, which no element type reaches.
pub trait ColumnElement: sealed::Element {}

pub(crate) mod sealed {
    use crate::bintable::column_data::ColumnData;
    use crate::bintable::column_view::ColumnView;

    #[expect(
        unnameable_types,
        reason = "sealed: outside code may use it, never name it"
    )]
    pub trait Element: Sized {
        fn into_data(values: Vec<Self>) -> ColumnData;
        fn from_data(data: ColumnData) -> Result<Vec<Self>, ColumnData>;
        fn from_view(view: ColumnView<'_>) -> Option<&[Self]>;
    }
}

macro_rules! column_element {
    ($($variant:ident: $type:ty),+ $(,)?) => {
        $(
            impl ColumnElement for $type {}

            impl sealed::Element for $type {
                fn into_data(values: Vec<Self>) -> ColumnData {
                    ColumnData::$variant(values)
                }

                fn from_data(data: ColumnData) -> Result<Vec<Self>, ColumnData> {
                    match data {
                        ColumnData::$variant(values) => Ok(values),
                        other => Err(other),
                    }
                }

                fn from_view(view: ColumnView<'_>) -> Option<&[Self]> {
                    match view {
                        ColumnView::$variant(values) => Some(values),
                        _ => None,
                    }
                }
            }
        )+
    };
}

column_element!(
    Logical: Option<bool>,
    U8: u8,
    I16: i16,
    I32: i32,
    I64: i64,
    F32: f32,
    F64: f64,
    ComplexF32: Complex<f32>,
    ComplexF64: Complex<f64>,
);
