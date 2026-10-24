//! HDU role validation, classification, and data-unit sizing.
//!
//! Boundaries are computable from the header alone using the primary-array,
//! extension, or random-groups formula, rounded up to a block, so the reader never
//! touches data to find the next HDU.

pub(crate) mod image_geometry;
pub(crate) mod prologue;

use crate::block::padded_len;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::header_model::record_name::RecordName;
use crate::header_model::{Header, keywords};

/// An HDU's position-derived header form before its complete role is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HduPosition {
    Primary,
    Extension,
}

impl HduPosition {
    /// The position of the HDU at `index` in file order: the first is the primary.
    pub(crate) const fn of_index(index: usize) -> Self {
        if index == 0 {
            Self::Primary
        } else {
            Self::Extension
        }
    }
}

/// The structural role already established by file position and mandatory header
/// signatures. Extent calculation uses this instead of interpreting reserved cards
/// such as `GROUPS` outside the role where the standard defines them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HduRole {
    Primary,
    Extension,
    RandomGroups,
}

impl HduRole {
    pub(crate) fn from_header(header: &Header, position: HduPosition) -> Result<Self> {
        match position {
            HduPosition::Primary => {
                header
                    .get_logical(keywords::SIMPLE)?
                    .ok_or(FitsError::MissingKeyword {
                        name: RecordName::Keyword(keywords::SIMPLE),
                    })?;
                if header.get_logical(keywords::GROUPS)? == Some(true) {
                    validate_random_groups_axes(&header.axes()?)?;
                    Ok(Self::RandomGroups)
                } else {
                    Ok(Self::Primary)
                }
            }
            HduPosition::Extension => {
                header
                    .get_text(keywords::XTENSION)?
                    .ok_or(FitsError::MissingKeyword {
                        name: RecordName::Keyword(keywords::XTENSION),
                    })?;
                Ok(Self::Extension)
            }
        }
    }
}

/// The structural kind of an HDU, inferred from its mandatory keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HduKind {
    /// Primary array (`SIMPLE = T`), possibly empty (`NAXIS = 0`).
    Primary,
    /// `XTENSION = 'IMAGE'` — same data model as the primary array.
    Image,
    /// `XTENSION = 'TABLE'` — ASCII table.
    AsciiTable,
    /// `XTENSION = 'BINTABLE'` — binary table (with optional heap).
    BinTable,
    /// A tiled-compressed image (§10.1): structurally a `BINTABLE` with `ZIMAGE = T`.
    /// `FitsReader::image` reads it like any other image.
    CompressedImage,
    /// A tiled-compressed table (§10.3): structurally a `BINTABLE` with `ZTABLE = T`.
    /// `FitsReader::read_table` reads it like any other binary table.
    CompressedTable,
    /// Legacy random-groups primary (`GROUPS = T`, `NAXIS1 = 0`). Read-only.
    RandomGroups,
    /// A conforming extension whose `XTENSION` value this crate does not model.
    Other,
}

impl HduKind {
    pub(crate) fn classify(header: &Header, role: HduRole) -> Result<Self> {
        if role == HduRole::Primary {
            return Ok(Self::Primary);
        }
        if role == HduRole::RandomGroups {
            return Ok(Self::RandomGroups);
        }

        // `Value::Text` already stripped the trailing spaces of `'IMAGE   '` etc.
        let xtension = header
            .get_text(keywords::XTENSION)?
            .ok_or(FitsError::MissingKeyword {
                name: RecordName::Keyword(keywords::XTENSION),
            })?;
        Ok(match xtension {
            "IMAGE" => Self::Image,
            "TABLE" => Self::AsciiTable,
            // §10: a tiled-compressed image/table rides inside a BINTABLE,
            // flagged by ZIMAGE/ZTABLE — classify by the payload, not the
            // container, so callers see what they can actually read.
            "BINTABLE" if header.get_logical(keywords::ZIMAGE)? == Some(true) => {
                Self::CompressedImage
            }
            "BINTABLE" if header.get_logical(keywords::ZTABLE)? == Some(true) => {
                Self::CompressedTable
            }
            "BINTABLE" => Self::BinTable,
            _ => Self::Other,
        })
    }
}

/// The size of an HDU's data unit, its length rounded up to the 2880-byte grid known to
/// fit a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataExtent {
    /// Unpadded data length (`Nbits / 8`).
    pub(crate) data_bytes: u64,
}

/// Compute the data-unit extent from a parsed, role-validated header (Eqs. 1, 2, 4).
pub(crate) fn data_extent(header: &Header, role: HduRole) -> Result<DataExtent> {
    let elem = header.bitpix()?.elem_size() as u64;
    let axes = header.axes()?;
    let data_bytes = match role {
        HduRole::Primary => elem
            .checked_mul(axis_product(&axes, 0)?)
            .ok_or(FitsError::DataUnitOverflow)?,
        HduRole::Extension => grouped_data_bytes(elem, axis_product(&axes, 0)?, header)?,
        HduRole::RandomGroups => {
            // `HduRole::from_header` checked the axes. NAXIS1 is the zero sentinel, so the array spans axes[1..] — and with no
            // array axis at all each group still holds one element, not zero.
            grouped_data_bytes(elem, axis_product(&axes[1..], 1)?, header)?
        }
    };
    padded_len(data_bytes).ok_or(FitsError::DataUnitOverflow)?;
    Ok(DataExtent { data_bytes })
}

/// Product of the axis lengths as the on-disk element count. Deliberately `u64`
/// rather than the `usize` of [`crate::image_model::shape_product`]: this sizes a data unit
/// on disk, which a 32-bit `usize` must not cap.
///
/// A zero-length axis always gives 0 (no data). `empty` is the count for an axis list
/// with *no entries*, which differs by role — `NAXIS = 0` means an image has no data
/// array at all (0), while a random group with no array axis still carries one array
/// element per group (the empty product, 1).
fn axis_product(axes: &[usize], empty: u64) -> Result<u64> {
    if axes.contains(&0) {
        return Ok(0);
    }
    if axes.is_empty() {
        return Ok(empty);
    }
    axes.iter()
        .try_fold(1u64, |product, &length| product.checked_mul(length as u64))
        .ok_or(FitsError::DataUnitOverflow)
}

#[derive(Debug, Clone, Copy)]
struct GroupCounts {
    pcount: u64,
    gcount: u64,
}

fn required_group_counts(header: &Header) -> Result<GroupCounts> {
    let pcount = header
        .get_integer(keywords::PCOUNT)?
        .ok_or(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::PCOUNT),
        })?;
    if pcount < 0 {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::PCOUNT,
        });
    }
    let gcount = header
        .get_integer(keywords::GCOUNT)?
        .ok_or(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::GCOUNT),
        })?;
    if gcount < 0 {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::GCOUNT,
        });
    }
    Ok(GroupCounts {
        pcount: pcount.cast_unsigned(),
        gcount: gcount.cast_unsigned(),
    })
}

fn grouped_data_bytes(elem: u64, array_elements: u64, header: &Header) -> Result<u64> {
    let counts = required_group_counts(header)?;
    let group_size = counts
        .pcount
        .checked_add(array_elements)
        .ok_or(FitsError::DataUnitOverflow)?;
    elem.checked_mul(counts.gcount)
        .and_then(|bytes| bytes.checked_mul(group_size))
        .ok_or(FitsError::DataUnitOverflow)
}

fn validate_random_groups_axes(axes: &[usize]) -> Result<()> {
    let Some(&first) = axes.first() else {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::NAXIS,
        });
    };
    if first != 0 {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::NAXIS.indexed(1)?,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
