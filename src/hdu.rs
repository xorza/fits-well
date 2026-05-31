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
    let naxis = header.naxis()?;
    let axes = header.axes()?;
    // PCOUNT/GCOUNT are mandatory ≥0 / ≥1 integers; a present-but-out-of-range
    // value is malformed and must not be silently clamped (it would yield a
    // plausible-but-wrong extent and a bad seek). Absence keeps the primary/IMAGE
    // defaults of 0 and 1.
    let pcount = match header.get_integer("PCOUNT") {
        Some(p) if p < 0 => return Err(FitsError::WrongValueType { name: "PCOUNT" }),
        Some(p) => p as u64,
        None => 0,
    };
    let gcount = match header.get_integer("GCOUNT") {
        Some(g) if g < 1 => return Err(FitsError::WrongValueType { name: "GCOUNT" }),
        Some(g) => g as u64,
        None => 1,
    };
    let random_groups = header.get_logical("GROUPS") == Some(true);

    // `NAXIS = 0` means no data array at all (the empty product would be 1, not
    // 0). Otherwise multiply the axis lengths — skipping the leading zero sentinel
    // for random groups. All arithmetic is checked: the axis lengths come from an
    // untrusted file and an overflowed product would drive a wild allocation in
    // the reader.
    let array_elems = if naxis == 0 {
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
mod tests {
    use super::*;
    use crate::block::CARD_SIZE;

    fn header(lines: &[&str]) -> Header {
        let mut buf = Vec::new();
        for line in lines {
            let mut card = [b' '; CARD_SIZE];
            card[..line.len()].copy_from_slice(line.as_bytes());
            buf.extend_from_slice(&card);
        }
        let mut end = [b' '; CARD_SIZE];
        end[..3].copy_from_slice(b"END");
        buf.extend_from_slice(&end);
        Header::parse(&buf).unwrap()
    }

    #[test]
    fn classifies_by_mandatory_keywords() {
        assert_eq!(
            HduKind::classify(&header(&["SIMPLE  = T"])),
            HduKind::Primary
        );
        assert_eq!(
            HduKind::classify(&header(&["XTENSION= 'IMAGE   '"])),
            HduKind::Image
        );
        assert_eq!(
            HduKind::classify(&header(&["XTENSION= 'TABLE   '"])),
            HduKind::AsciiTable
        );
        assert_eq!(
            HduKind::classify(&header(&["XTENSION= 'BINTABLE'"])),
            HduKind::BinTable
        );
        assert_eq!(
            HduKind::classify(&header(&["SIMPLE  = T", "GROUPS  = T"])),
            HduKind::RandomGroups
        );
        assert_eq!(
            HduKind::classify(&header(&["XTENSION= 'FOO     '"])),
            HduKind::Other
        );
    }

    #[test]
    fn image_data_extent_matches_hand_computed_size() {
        // 512×512 signed 16-bit image: 2 × 512 × 512 = 524288 bytes,
        // rounded up to 183 blocks = 527040 bytes.
        let h = header(&[
            "BITPIX  = 16",
            "NAXIS   = 2",
            "NAXIS1  = 512",
            "NAXIS2  = 512",
        ]);
        let e = data_extent(&h).unwrap();
        assert_eq!(e.data_bytes, 524_288);
        assert_eq!(e.padded_bytes, 527_040);
    }

    #[test]
    fn dataless_primary_has_no_data_unit() {
        let e = data_extent(&header(&["BITPIX  = 8", "NAXIS   = 0"])).unwrap();
        assert_eq!(e.data_bytes, 0);
        assert_eq!(e.padded_bytes, 0);
    }

    #[test]
    fn rejects_malformed_pcount_and_gcount_instead_of_clamping() {
        let neg_pcount = header(&[
            "XTENSION= 'BINTABLE'",
            "BITPIX  = 8",
            "NAXIS   = 2",
            "NAXIS1  = 4",
            "NAXIS2  = 3",
            "PCOUNT  = -1",
        ]);
        assert!(matches!(
            data_extent(&neg_pcount),
            Err(FitsError::WrongValueType { name: "PCOUNT" })
        ));

        let zero_gcount = header(&[
            "XTENSION= 'IMAGE   '",
            "BITPIX  = 8",
            "NAXIS   = 1",
            "NAXIS1  = 4",
            "GCOUNT  = 0",
        ]);
        assert!(matches!(
            data_extent(&zero_gcount),
            Err(FitsError::WrongValueType { name: "GCOUNT" })
        ));
    }

    #[test]
    fn axis_product_overflow_is_an_error_not_a_wrap() {
        // Three axes near 2^32 overflow u64 when multiplied — must not wrap into
        // a small, plausible-but-wrong byte count.
        let h = header(&[
            "BITPIX  = 8",
            "NAXIS   = 3",
            "NAXIS1  = 4294967296",
            "NAXIS2  = 4294967296",
            "NAXIS3  = 4294967296",
        ]);
        assert!(matches!(data_extent(&h), Err(FitsError::DataUnitOverflow)));
    }

    #[test]
    fn random_groups_extent_skips_the_zero_first_axis() {
        // BITPIX=-32 (4 bytes), per-group array = 3×4×1×1×1 = 12 elems,
        // plus PCOUNT=6 params, over GCOUNT=7956 groups:
        // 4 × 7956 × (6 + 12) = 572832 bytes → 199 blocks = 573120.
        let h = header(&[
            "BITPIX  = -32",
            "NAXIS   = 6",
            "NAXIS1  = 0",
            "NAXIS2  = 3",
            "NAXIS3  = 4",
            "NAXIS4  = 1",
            "NAXIS5  = 1",
            "NAXIS6  = 1",
            "GROUPS  = T",
            "PCOUNT  = 6",
            "GCOUNT  = 7956",
        ]);
        let e = data_extent(&h).unwrap();
        assert_eq!(e.data_bytes, 572_832);
        assert_eq!(e.padded_bytes, 573_120);
    }
}
