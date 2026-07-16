//! HDU role validation, classification, and data-unit sizing.
//!
//! Boundaries are computable from the header alone using the primary-array,
//! extension, or random-groups formula, rounded up to a block, so the reader never
//! touches data to find the next HDU.

use crate::block::checked_padded_len;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

/// An HDU's position-derived header form before its complete role is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HduPosition {
    Primary,
    Extension,
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
    pub(crate) fn from_header(header: &Header, position: HduPosition) -> Result<HduRole> {
        match position {
            HduPosition::Primary => {
                header
                    .get_logical("SIMPLE")?
                    .ok_or(FitsError::MissingKeyword { name: "SIMPLE" })?;
                if header.get_logical("GROUPS")? == Some(true) {
                    validate_random_groups_axes(&header.axes()?)?;
                    Ok(HduRole::RandomGroups)
                } else {
                    Ok(HduRole::Primary)
                }
            }
            HduPosition::Extension => {
                header
                    .get_text("XTENSION")?
                    .ok_or(FitsError::MissingKeyword { name: "XTENSION" })?;
                Ok(HduRole::Extension)
            }
        }
    }
}

/// The structural kind of an HDU, inferred from its mandatory keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// `read_image` reads it like any other image.
    CompressedImage,
    /// A tiled-compressed table (§10.3): structurally a `BINTABLE` with `ZTABLE = T`.
    /// `read_compressed_table` uncompresses it.
    CompressedTable,
    /// Legacy random-groups primary (`GROUPS = T`, `NAXIS1 = 0`). Read-only.
    RandomGroups,
    /// A conforming extension whose `XTENSION` value this crate does not model.
    Other,
}

impl HduKind {
    pub(crate) fn classify(header: &Header, role: HduRole) -> Result<HduKind> {
        if role == HduRole::Primary {
            return Ok(HduKind::Primary);
        }
        if role == HduRole::RandomGroups {
            return Ok(HduKind::RandomGroups);
        }

        // `Value::Text` already stripped the trailing spaces of `'IMAGE   '` etc.
        let xtension = header
            .get_text("XTENSION")?
            .ok_or(FitsError::MissingKeyword { name: "XTENSION" })?;
        Ok(match xtension {
            "IMAGE" => HduKind::Image,
            "TABLE" => HduKind::AsciiTable,
            // §10: a tiled-compressed image/table rides inside a BINTABLE,
            // flagged by ZIMAGE/ZTABLE — classify by the payload, not the
            // container, so callers see what they can actually read.
            "BINTABLE" if header.get_logical("ZIMAGE")? == Some(true) => HduKind::CompressedImage,
            "BINTABLE" if header.get_logical("ZTABLE")? == Some(true) => HduKind::CompressedTable,
            "BINTABLE" => HduKind::BinTable,
            _ => HduKind::Other,
        })
    }
}

/// The size of an HDU's data unit, both as the raw bit-count-derived byte length
/// and rounded to the on-disk block grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataExtent {
    /// Unpadded data length (`Nbits / 8`).
    pub(crate) data_bytes: u64,
    /// Length rounded up to the 2880-byte grid — the bytes occupied on disk.
    pub(crate) padded_bytes: u64,
}

/// Compute the data-unit extent from a parsed, role-validated header (Eqs. 1, 2, 4).
pub(crate) fn data_extent(header: &Header, role: HduRole) -> Result<DataExtent> {
    let elem = header.bitpix()?.elem_size() as u64;
    let axes = header.axes()?;
    let data_bytes = match role {
        HduRole::Primary => elem
            .checked_mul(array_elements(&axes)?)
            .ok_or(FitsError::DataUnitOverflow)?,
        HduRole::Extension => grouped_data_bytes(elem, array_elements(&axes)?, header)?,
        HduRole::RandomGroups => {
            validate_random_groups_axes(&axes)?;
            grouped_data_bytes(elem, array_elements(&axes[1..])?, header)?
        }
    };
    Ok(DataExtent {
        data_bytes,
        padded_bytes: checked_padded_len(data_bytes).ok_or(FitsError::DataUnitOverflow)?,
    })
}

fn array_elements(axes: &[usize]) -> Result<u64> {
    if axes.is_empty() || axes.contains(&0) {
        return Ok(0);
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
        .get_integer("PCOUNT")?
        .ok_or(FitsError::MissingKeyword { name: "PCOUNT" })?;
    if pcount < 0 {
        return Err(FitsError::KeywordOutOfRange { name: "PCOUNT" });
    }
    let gcount = header
        .get_integer("GCOUNT")?
        .ok_or(FitsError::MissingKeyword { name: "GCOUNT" })?;
    if gcount < 1 {
        return Err(FitsError::KeywordOutOfRange { name: "GCOUNT" });
    }
    Ok(GroupCounts {
        pcount: pcount as u64,
        gcount: gcount as u64,
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
        return Err(FitsError::KeywordOutOfRange { name: "NAXIS" });
    };
    if first != 0 {
        return Err(FitsError::KeywordOutOfRange { name: "NAXIS1" });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
