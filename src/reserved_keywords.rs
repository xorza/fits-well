//! The keywords the format owns. A writer generates them from the data it writes,
//! so it drops them from a template header, and table compression moves them
//! between a table and its container.

use crate::keyword;

/// Fixed-name keywords of the HDU structure (§4.4), array scaling (§5.3), the
/// checksums (Appendix J) and the binary-table heap (§7.3).
const STRUCTURE: [&str; 16] = [
    "SIMPLE", "XTENSION", "BITPIX", "NAXIS", "PCOUNT", "GCOUNT", "EXTEND", "GROUPS", "BLOCKED",
    "BSCALE", "BZERO", "BLANK", "CHECKSUM", "DATASUM", "THEAP", "TFIELDS",
];

/// Indexed roots of the array axes (§4.4) and table columns (§7.2, §7.3).
const STRUCTURE_INDEXED: [&str; 9] = [
    "NAXIS", "TFORM", "TTYPE", "TUNIT", "TDIM", "TSCAL", "TZERO", "TNULL", "TBCOL",
];

/// Fixed-name keywords of tiled image compression (§10.1, §10.2).
const IMAGE_COMPRESSION: [&str; 14] = [
    "ZIMAGE", "ZCMPTYPE", "ZBITPIX", "ZNAXIS", "ZQUANTIZ", "ZDITHER0", "ZBLANK", "ZMASKCMP",
    "ZSIMPLE", "ZTENSION", "ZEXTEND", "ZBLOCKED", "ZGCOUNT", "ZHEAPPTR",
];

/// Indexed roots of tiled image compression (§10.1).
const IMAGE_COMPRESSION_INDEXED: [&str; 4] = ["ZNAXIS", "ZTILE", "ZNAME", "ZVAL"];

/// Fixed-name keywords a tiled-compressed table's header adds (§10.3.1).
pub(crate) const TABLE_COMPRESSION: [&str; 5] =
    ["ZTABLE", "ZTILELEN", "ZNAXIS1", "ZNAXIS2", "ZPCOUNT"];

/// Column-indexed roots a tiled-compressed table's header adds (§10.3.1).
pub(crate) const TABLE_COMPRESSION_INDEXED: [&str; 2] = ["ZFORM", "ZCTYP"];

/// The keywords table compression keeps under a `Z` name, as (table, container)
/// pairs (§10.3.1).
pub(crate) const TABLE_PRESERVED: [(&str, &str); 3] = [
    ("THEAP", "ZTHEAP"),
    ("CHECKSUM", "ZHECKSUM"),
    ("DATASUM", "ZDATASUM"),
];

/// Whether a writer generates `keyword` rather than copying it from a template.
pub(crate) fn is_generated(keyword: &str) -> bool {
    // A conforming keyword is at most 8 bytes; anything longer is not the indexed
    // keyword it superficially resembles.
    let indexed = |root: &&str| keyword.len() <= 8 && keyword::index(keyword, root).is_some();
    STRUCTURE.contains(&keyword)
        || IMAGE_COMPRESSION.contains(&keyword)
        || TABLE_COMPRESSION.contains(&keyword)
        || TABLE_PRESERVED
            .iter()
            .any(|&(_, container)| container == keyword)
        || STRUCTURE_INDEXED.iter().any(indexed)
        || IMAGE_COMPRESSION_INDEXED.iter().any(indexed)
        || TABLE_COMPRESSION_INDEXED.iter().any(indexed)
}

/// Whether `keyword` is one a compressed table's header adds for one of its
/// `columns` columns.
#[cfg(feature = "compression")]
pub(crate) fn is_table_compression(keyword: &str, columns: usize) -> bool {
    TABLE_COMPRESSION.contains(&keyword)
        || TABLE_COMPRESSION_INDEXED.iter().any(|root| {
            keyword::index(keyword, root).is_some_and(|column| (1..=columns).contains(&column))
        })
}
