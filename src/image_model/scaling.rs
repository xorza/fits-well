//! [`Scaling`]: the `BSCALE`/`BZERO` linear transform between stored and
//! physical values.

use crate::bitpix::Bitpix;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::sample::Sample;
use crate::image_model::sample_type::{SampleType, UnsignedKind};
use crate::image_model::{U64_OFFSET, U64_OFFSET_INTEGER};

/// The linear `BSCALE`/`BZERO` map from a stored value to its physical value,
/// plus the integer `BLANK` sentinel marking undefined pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    pub bscale: f64,
    pub bzero: f64,
    pub blank: Option<i64>,
}

impl Default for Scaling {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Scaling {
    /// Identity physical mapping: `physical = stored`, with no integer null sentinel.
    pub const IDENTITY: Self = Self {
        bscale: 1.0,
        bzero: 0.0,
        blank: None,
    };

    /// The `BSCALE`, `BZERO` and `BLANK` that `header` declares for samples stored as
    /// `bitpix`, checked as [`Scaling::validate`] checks them. A float array ignores
    /// `BLANK`, which §5.2.5 defines for integers only, as cfitsio ignores it and
    /// astropy warns and ignores it; its undefined values are `NaN`.
    pub(crate) fn from_header(header: &Header, bitpix: Bitpix) -> Result<Self> {
        let scaling = Self {
            bscale: header.get_real(keywords::BSCALE)?.unwrap_or(1.0),
            bzero: header.get_real(keywords::BZERO)?.unwrap_or(0.0),
            blank: if bitpix.is_float() {
                None
            } else {
                header.get_integer(keywords::BLANK)?
            },
        };
        scaling.validate(bitpix)?;
        Ok(scaling)
    }

    /// The exact-integer realization this scaling denotes for samples stored as
    /// `bitpix`, or `None` when it is not a sign-bit-offset convention.
    ///
    /// The single place the FITS unsigned convention is resolved. The offset test
    /// itself lives in [`SampleType::from_scaling`]; what this adds is the null
    /// guard — `BLANK` (or a table column's `TZEROn`-paired `TNULLn`, which maps
    /// onto the same field) marks samples with no exact integer value, so an
    /// unsigned view cannot represent them, and `SampleType` deliberately ignores it.
    pub(crate) fn unsigned_kind(&self, bitpix: Bitpix) -> Option<UnsignedKind> {
        if self.blank.is_some() {
            return None;
        }
        SampleType::from_scaling(bitpix, *self).unsigned_kind()
    }

    /// Append the physical values of `samples` to `out`: `BZERO + BSCALE × sample`
    /// (§3.4, §5.2.5), the samples equal to an integer `BLANK` as `NaN`, and a float's
    /// `NaN` and infinities passed through. The affine is one fused multiply-add, the
    /// correctly rounded `f64` of the exact value, where cfitsio rounds twice. The
    /// `BLANK` test and the identity are decided once, outside the loop, which then
    /// has no branch the data takes.
    pub(crate) fn extend_physical<T: Sample, O: PhysicalSample>(
        &self,
        samples: &[T],
        out: &mut Vec<O>,
    ) {
        out.reserve(samples.len());
        let Self {
            bscale,
            bzero,
            blank,
        } = *self;
        let affine = |sample: T| O::from_f64(bscale.mul_add(sample.to_f64(), bzero));
        let identity = |sample: T| O::from_f64(sample.to_f64());
        match (blank.and_then(T::integer), self.is_identity()) {
            (None, true) => out.extend(samples.iter().map(|&sample| identity(sample))),
            (None, false) => out.extend(samples.iter().map(|&sample| affine(sample))),
            (Some(blank), true) => out.extend(samples.iter().map(|&sample| {
                if sample == blank {
                    O::from_f64(f64::NAN)
                } else {
                    identity(sample)
                }
            })),
            (Some(blank), false) => out.extend(samples.iter().map(|&sample| {
                if sample == blank {
                    O::from_f64(f64::NAN)
                } else {
                    affine(sample)
                }
            })),
        }
    }

    /// [`Scaling::extend_physical`] straight from big-endian samples of type `T`,
    /// decoded a block at a time in a stack buffer: no host-endian copy of the whole
    /// run is made.
    pub(crate) fn extend_physical_be<T: Sample, O: PhysicalSample>(
        &self,
        bytes: &[u8],
        out: &mut Vec<O>,
    ) {
        /// The samples decoded at a time: 2 KiB of `f64`, which stays in L1.
        const BLOCK: usize = 256;
        let size = size_of::<T>();
        debug_assert_eq!(bytes.len() % size, 0, "whole big-endian samples");
        out.reserve(bytes.len() / size);
        let mut block = [T::default(); BLOCK];
        for chunk in bytes.chunks(BLOCK * size) {
            let decoded = &mut block[..chunk.len() / size];
            T::decode_be(chunk, decoded);
            self.extend_physical(decoded, out);
        }
    }

    pub(crate) fn validate(&self, bitpix: Bitpix) -> Result<()> {
        if !self.bscale.is_finite() {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::BSCALE,
            });
        }
        if !self.bzero.is_finite() {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::BZERO,
            });
        }
        let Some(blank) = self.blank else {
            return Ok(());
        };
        if !bitpix.holds_integer(blank) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::BLANK,
            });
        }
        Ok(())
    }

    pub(crate) fn add_to_header(&self, header: &mut Header, bitpix: Bitpix) -> Result<()> {
        self.validate(bitpix)?;
        if !self.is_identity() {
            if bitpix == Bitpix::I64 && self.bscale == 1.0 && self.bzero == U64_OFFSET {
                header.set_internal(keywords::BZERO, U64_OFFSET_INTEGER);
            } else {
                header.set_internal(keywords::BZERO, self.bzero);
            }
            header.set_internal(keywords::BSCALE, self.bscale);
        }
        if let Some(blank) = self.blank {
            header.set_internal(keywords::BLANK, blank);
        }
        Ok(())
    }

    /// Whether the affine map is the identity, `BSCALE = 1` and `BZERO = 0`. `BLANK` is
    /// a separate fact: an identity scaling with a `BLANK` still maps the null to `NaN`.
    pub fn is_identity(&self) -> bool {
        self.bscale == 1.0 && self.bzero == 0.0
    }
}
