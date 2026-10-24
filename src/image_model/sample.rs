//! [`Sample`]: the six FITS sample types, by type rather than by tag.

use crate::bitpix::Bitpix;
use crate::endian::{decode_be_into_slice, extend_be};
use crate::image_model::data::Data;
use crate::image_model::view::View;

/// One of the six sample types that `BITPIX` names (§4.4.1.1), in host order: what a
/// [`View`] and a [`Data`] hold. Sealed: exactly `u8`, `i16`, `i32`, `i64`, `f32` and
/// `f64` implement it. A caller generic over the type takes `T: Sample` and converts
/// with the std traits: `View::from`, `Data::from`, `<&[T]>::try_from(view)` and
/// `Vec::<T>::try_from(data)`.
pub trait Sample: sealed::Raw + Copy + Send + Sync + 'static {
    /// The `BITPIX` of the type.
    const BITPIX: Bitpix;
}

pub(crate) mod sealed {
    use crate::image_model::data::Data;
    use crate::image_model::view::View;

    /// The sample operations the crate's loops share.
    ///
    /// # Safety
    ///
    /// Implementors must be plain-data scalars with no invalid bit patterns and an
    /// alignment of at most 8. Both properties are what let
    /// [`words::samples`](crate::words::samples) reinterpret `u64` storage as the type
    /// with a length check alone.
    #[expect(
        unnameable_types,
        reason = "sealed: outside code may use it, never name it"
    )]
    pub unsafe trait Raw: Copy + Default + PartialEq {
        /// `samples`, viewed.
        fn view(samples: &[Self]) -> View<'_>;

        /// The samples of `view` when they are of this type.
        fn of(view: View<'_>) -> Option<&[Self]>;

        /// `samples`, owned.
        fn data(samples: Vec<Self>) -> Data;

        /// The samples of `data` when they are of this type, and `data` back otherwise.
        fn of_data(data: Data) -> Result<Vec<Self>, Data>;

        /// The value as an `f64`: exact for every type, but for an `i64` past 2⁵³,
        /// which rounds to the nearest.
        fn to_f64(self) -> f64;

        /// `value` as this type when it holds integers and `value` is one of them;
        /// `None` for a float, whose undefined values are `NaN`, not a `BLANK`.
        fn integer(value: i64) -> Option<Self>;

        /// Decode the big-endian `src`, `size_of::<Self>()` bytes a value, into `dst`.
        fn decode_be(src: &[u8], dst: &mut [Self]);

        /// Append `values`, big-endian, to `out`.
        fn extend_be(values: &[Self], out: &mut Vec<u8>);

        /// Turn `values`, which hold stored big-endian samples, host-endian, in place.
        fn swap_be_in_place(values: &mut [Self]);

        /// Append the big-endian samples `src` to `out`, host-endian, with no fill of its
        /// new capacity first.
        fn extend_from_be(src: &[u8], out: &mut Vec<Self>);
    }
}

macro_rules! impl_sample {
    ($type:ty, $variant:ident, $to_f64:expr, $integer:expr) => {
        impl Sample for $type {
            const BITPIX: Bitpix = Bitpix::$variant;
        }

        // SAFETY: a primitive number: every bit pattern is a value, and its alignment
        // is at most 8.
        unsafe impl sealed::Raw for $type {
            fn view(samples: &[Self]) -> View<'_> {
                View::$variant(samples)
            }

            fn of(view: View<'_>) -> Option<&[Self]> {
                match view {
                    View::$variant(samples) => Some(samples),
                    _ => None,
                }
            }

            fn data(samples: Vec<Self>) -> Data {
                Data::$variant(samples)
            }

            fn of_data(data: Data) -> Result<Vec<Self>, Data> {
                match data {
                    Data::$variant(samples) => Ok(samples),
                    other => Err(other),
                }
            }

            fn to_f64(self) -> f64 {
                $to_f64(self)
            }

            fn integer(value: i64) -> Option<Self> {
                $integer(value)
            }

            fn decode_be(src: &[u8], dst: &mut [Self]) {
                decode_be_into_slice(src, dst, <$type>::from_be_bytes);
            }

            fn extend_be(values: &[Self], out: &mut Vec<u8>) {
                extend_be(out, values, <$type>::to_be_bytes);
            }

            fn swap_be_in_place(values: &mut [Self]) {
                for value in values {
                    *value = <$type>::from_be_bytes(value.to_ne_bytes());
                }
            }

            fn extend_from_be(src: &[u8], out: &mut Vec<Self>) {
                debug_assert_eq!(src.len() % size_of::<Self>(), 0, "whole big-endian samples");
                out.extend(
                    src.as_chunks()
                        .0
                        .iter()
                        .map(|&bytes| <$type>::from_be_bytes(bytes)),
                );
            }
        }
    };
}

impl_sample!(u8, U8, f64::from, |value| Self::try_from(value).ok());
impl_sample!(i16, I16, f64::from, |value| Self::try_from(value).ok());
impl_sample!(i32, I32, f64::from, |value| Self::try_from(value).ok());
impl_sample!(i64, I64, |value| value as f64, Some);
impl_sample!(f32, F32, f64::from, |_| None);
impl_sample!(f64, F64, |value| value, |_| None);

impl<'a, T: Sample> From<&'a [T]> for View<'a> {
    fn from(samples: &'a [T]) -> Self {
        T::view(samples)
    }
}

impl<T: Sample> From<Vec<T>> for Data {
    fn from(samples: Vec<T>) -> Self {
        T::data(samples)
    }
}

/// The samples of `view` when they are of type `T`, and `view` back otherwise.
impl<'a, T: Sample> TryFrom<View<'a>> for &'a [T] {
    type Error = View<'a>;

    fn try_from(view: View<'a>) -> Result<Self, View<'a>> {
        T::of(view).ok_or(view)
    }
}

impl<T: Sample> TryFrom<Data> for Vec<T> {
    type Error = Data;

    fn try_from(data: Data) -> Result<Self, Data> {
        T::of_data(data)
    }
}
