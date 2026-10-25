//! [`UnsignedElement`]: the exact integer types an unsigned image or column takes.

use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::unsigned_view::UnsignedView;

/// An exact integer type an unsigned image or column takes as one [`UnsignedData`] case:
/// `i8`, `u16`, `u32` or `u64`. Sealed; it gives [`UnsignedData`] and [`UnsignedView`]
/// their typed conversions.
pub trait UnsignedElement: sealed::Element {}

pub(crate) mod sealed {
    use crate::image_model::unsigned_data::UnsignedData;
    use crate::image_model::unsigned_view::UnsignedView;

    #[expect(
        unnameable_types,
        reason = "sealed: outside code may use it, never name it"
    )]
    pub trait Element: Sized {
        fn into_data(values: Vec<Self>) -> UnsignedData;
        fn from_data(data: UnsignedData) -> Result<Vec<Self>, UnsignedData>;
        fn from_view(view: UnsignedView<'_>) -> Option<&[Self]>;
    }
}

macro_rules! unsigned_element {
    ($($variant:ident: $type:ty),+ $(,)?) => {
        $(
            impl UnsignedElement for $type {}

            impl sealed::Element for $type {
                fn into_data(values: Vec<Self>) -> UnsignedData {
                    UnsignedData::$variant(values)
                }

                fn from_data(data: UnsignedData) -> Result<Vec<Self>, UnsignedData> {
                    match data {
                        UnsignedData::$variant(values) => Ok(values),
                        other => Err(other),
                    }
                }

                fn from_view(view: UnsignedView<'_>) -> Option<&[Self]> {
                    match view {
                        UnsignedView::$variant(values) => Some(values),
                        _ => None,
                    }
                }
            }
        )+
    };
}

unsigned_element!(I8: i8, U16: u16, U32: u32, U64: u64);
