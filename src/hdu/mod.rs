//! HDU classification and the data-unit sizing formula.
//!
//! Boundaries are computable from the header alone — `Nbits = |BITPIX| · GCOUNT ·
//! (PCOUNT + Π NAXISn)`, rounded up to a block — so the reader never touches data
//! to find the next HDU.

use crate::block::padded_len;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

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
    pub(crate) fn classify(header: &Header) -> HduKind {
        // `Value::Text` already stripped the trailing spaces of `'IMAGE   '` etc.
        if let Some(xtension) = header.get_text("XTENSION") {
            match xtension {
                "IMAGE" => HduKind::Image,
                "TABLE" => HduKind::AsciiTable,
                // §10: a tiled-compressed image/table rides inside a BINTABLE,
                // flagged by ZIMAGE/ZTABLE — classify by the payload, not the
                // container, so callers see what they can actually read.
                "BINTABLE" if header.get_logical("ZIMAGE") == Some(true) => {
                    HduKind::CompressedImage
                }
                "BINTABLE" if header.get_logical("ZTABLE") == Some(true) => {
                    HduKind::CompressedTable
                }
                "BINTABLE" => HduKind::BinTable,
                _ => HduKind::Other,
            }
        } else if header.get_logical("GROUPS") == Some(true) {
            HduKind::RandomGroups
        } else {
            HduKind::Primary
        }
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

/// Compute the data-unit extent from a parsed header (Eq. 2).
pub(crate) fn data_extent(header: &Header) -> Result<DataExtent> {
    let elem = header.bitpix()?.elem_size() as u64;
    let axes = header.axes()?;
    // PCOUNT/GCOUNT are mandatory ≥0 / ≥1 integers; a present-but-out-of-range
    // value is malformed and must not be silently clamped (it would yield a
    // plausible-but-wrong extent and a bad seek). Absence keeps the primary/IMAGE
    // defaults of 0 and 1.
    let pcount = match header.get_integer("PCOUNT") {
        Some(p) if p < 0 => return Err(FitsError::KeywordOutOfRange { name: "PCOUNT" }),
        Some(p) => p as u64,
        None => 0,
    };
    let gcount = match header.get_integer("GCOUNT") {
        Some(g) if g < 1 => return Err(FitsError::KeywordOutOfRange { name: "GCOUNT" }),
        Some(g) => g as u64,
        None => 1,
    };
    let random_groups = header.get_logical("GROUPS") == Some(true);

    // `NAXIS = 0` means no data array at all (the empty product would be 1, not
    // 0). Otherwise multiply the axis lengths — skipping the leading zero sentinel
    // for random groups. All arithmetic is checked: the axis lengths come from an
    // untrusted file and an overflowed product would drive a wild allocation in
    // the reader.
    let array_elems = if axes.is_empty() {
        0
    } else {
        let array_axes: &[usize] = if random_groups { &axes[1..] } else { &axes };
        array_axes
            .iter()
            .try_fold(1u64, |acc, &n| acc.checked_mul(n as u64))
            .ok_or(FitsError::DataUnitOverflow)?
    };

    let group_size = pcount
        .checked_add(array_elems)
        .ok_or(FitsError::DataUnitOverflow)?;
    let data_bytes = elem
        .checked_mul(gcount)
        .and_then(|n| n.checked_mul(group_size))
        .ok_or(FitsError::DataUnitOverflow)?;
    Ok(DataExtent {
        data_bytes,
        padded_bytes: padded_len(data_bytes),
    })
}

#[cfg(test)]
mod tests;
