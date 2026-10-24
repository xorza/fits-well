//! [`Prologue`]: the structural records an HDU's header opens with.

use crate::bitpix::Bitpix;
use crate::fits_error::{FitsError, Result};
use crate::header_model::{Header, keywords, value};

/// What an HDU's header opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opening {
    /// `SIMPLE = T` (§4.4.1.1), and `EXTEND = T` after the axes.
    Primary,
    /// `XTENSION` of this kind (§7), and `PCOUNT` and `GCOUNT` after the axes.
    Extension(Xtension),
}

/// The kind an `XTENSION` names, of those fits-well writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Xtension {
    Image,
    Table,
    BinTable,
}

/// The structural records of an HDU's header, in the order §4.4.1 requires: the opening
/// record, `BITPIX`, `NAXIS`, the axes, then `EXTEND` for a primary or `PCOUNT` and
/// `GCOUNT` for an extension.
#[derive(Debug)]
pub(crate) struct Prologue<'a> {
    pub(crate) opening: Opening,
    pub(crate) bitpix: Bitpix,
    pub(crate) axes: &'a [usize],
    /// The bytes of the heap; 0 for a primary, which has none.
    pub(crate) pcount: usize,
}

impl Prologue<'_> {
    /// A header that holds these records.
    pub(crate) fn header(&self) -> Result<Header> {
        if self.axes.len() > 999 {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::NAXIS,
            });
        }
        let mut header = Header::new();
        match self.opening {
            Opening::Primary => header
                .set_internal(keywords::SIMPLE, true)
                .set_comment_internal(keywords::SIMPLE, "file conforms to FITS standard"),
            Opening::Extension(kind) => {
                let (name, comment) = match kind {
                    Xtension::Image => ("IMAGE", "image extension"),
                    Xtension::Table => ("TABLE", "ASCII table extension"),
                    Xtension::BinTable => ("BINTABLE", "binary table extension"),
                };
                header
                    .set_internal(keywords::XTENSION, name)
                    .set_comment_internal(keywords::XTENSION, comment)
            }
        };
        header
            .set_internal(keywords::BITPIX, self.bitpix.code())
            .set_comment_internal(keywords::BITPIX, "number of bits per data value");
        header
            .set_internal(keywords::NAXIS, value::fits_i64(self.axes.len())?)
            .set_comment_internal(keywords::NAXIS, "number of data axes");
        for (i, &length) in self.axes.iter().enumerate() {
            header.set_internal(keywords::NAXIS.indexed(i + 1)?, value::fits_i64(length)?);
        }
        match self.opening {
            Opening::Primary => {
                debug_assert_eq!(self.pcount, 0, "a primary has no heap");
                header
                    .set_internal(keywords::EXTEND, true)
                    .set_comment_internal(keywords::EXTEND, "extensions may follow");
            }
            Opening::Extension(_) => {
                header
                    .set_internal(keywords::PCOUNT, value::fits_i64(self.pcount)?)
                    .set_internal(keywords::GCOUNT, 1);
            }
        }
        Ok(header)
    }
}
