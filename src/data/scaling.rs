//! [`Scaling`]: the `BSCALE`/`BZERO` linear transform between stored and
//! physical values.

use crate::bitpix::Bitpix;
use crate::data::sample_type::{SampleType, UnsignedKind};
use crate::data::{U64_OFFSET, U64_OFFSET_INTEGER};
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

/// The linear `BSCALE`/`BZERO` map from a stored value to its physical value,
/// plus the integer `BLANK` sentinel marking undefined pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    pub bscale: f64,
    pub bzero: f64,
    pub blank: Option<i64>,
}

impl Default for Scaling {
    fn default() -> Scaling {
        Scaling::IDENTITY
    }
}

impl Scaling {
    /// Identity physical mapping: `physical = stored`, with no integer null sentinel.
    pub const IDENTITY: Scaling = Scaling {
        bscale: 1.0,
        bzero: 0.0,
        blank: None,
    };

    /// The public entry point is [`Header::scaling`](crate::header::Header::scaling).
    pub(crate) fn from_header(header: &Header) -> Result<Scaling> {
        Ok(Scaling {
            bscale: header.get_real("BSCALE")?.unwrap_or(1.0),
            bzero: header.get_real("BZERO")?.unwrap_or(0.0),
            blank: header.get_integer("BLANK")?,
        })
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
        SampleType::from_scaling(bitpix, self).unsigned_kind()
    }

    pub(crate) fn scale(&self, raw: f64) -> f64 {
        self.bzero + self.bscale * raw
    }

    pub(crate) fn scale_integer(&self, raw: i64) -> f64 {
        if self.blank == Some(raw) {
            f64::NAN
        } else {
            self.scale(raw as f64)
        }
    }

    pub(crate) fn validate(&self, bitpix: Bitpix) -> Result<()> {
        if !self.bscale.is_finite() {
            return Err(FitsError::KeywordOutOfRange { name: "BSCALE" });
        }
        if !self.bzero.is_finite() {
            return Err(FitsError::KeywordOutOfRange { name: "BZERO" });
        }
        let Some(blank) = self.blank else {
            return Ok(());
        };
        let valid = match bitpix {
            Bitpix::U8 => u8::try_from(blank).is_ok(),
            Bitpix::I16 => i16::try_from(blank).is_ok(),
            Bitpix::I32 => i32::try_from(blank).is_ok(),
            Bitpix::I64 => true,
            Bitpix::F32 | Bitpix::F64 => false,
        };
        if !valid {
            return Err(FitsError::KeywordOutOfRange { name: "BLANK" });
        }
        Ok(())
    }

    pub(crate) fn add_to_header(&self, header: &mut Header, bitpix: Bitpix) -> Result<()> {
        self.validate(bitpix)?;
        if !self.is_identity() {
            if bitpix == Bitpix::I64 && self.bscale == 1.0 && self.bzero == U64_OFFSET {
                header.set_internal("BZERO", U64_OFFSET_INTEGER);
            } else {
                header.set_internal("BZERO", self.bzero);
            }
            header.set_internal("BSCALE", self.bscale);
        }
        if let Some(blank) = self.blank {
            header.set_internal("BLANK", blank);
        }
        Ok(())
    }

    /// `true` when decoding needs no arithmetic — just an endian swap or copy.
    pub fn is_identity(&self) -> bool {
        self.bscale == 1.0 && self.bzero == 0.0
    }
}
