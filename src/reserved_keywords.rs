//! The keywords the format owns. A writer generates them from the data it writes,
//! so it drops them from a template header, and table compression moves them
//! between a table and its container.

use crate::header_model::keywords;
use crate::keyword::Keyword;

/// Fixed-name keywords of the HDU structure (§4.4), array scaling (§5.3), the
/// checksums (Appendix J) and the binary-table heap (§7.3).
const STRUCTURE: [Keyword; 16] = [
    keywords::SIMPLE,
    keywords::XTENSION,
    keywords::BITPIX,
    keywords::NAXIS,
    keywords::PCOUNT,
    keywords::GCOUNT,
    keywords::EXTEND,
    keywords::GROUPS,
    keywords::BLOCKED,
    keywords::BSCALE,
    keywords::BZERO,
    keywords::BLANK,
    keywords::CHECKSUM,
    keywords::DATASUM,
    keywords::THEAP,
    keywords::TFIELDS,
];

/// Indexed roots of the array axes (§4.4) and table columns (§7.2, §7.3).
const STRUCTURE_INDEXED: [Keyword; 9] = [
    keywords::NAXIS,
    keywords::TFORM,
    keywords::TTYPE,
    keywords::TUNIT,
    keywords::TDIM,
    keywords::TSCAL,
    keywords::TZERO,
    keywords::TNULL,
    keywords::TBCOL,
];

/// Fixed-name keywords of tiled image compression (§10.1, §10.2).
const IMAGE_COMPRESSION: [Keyword; 14] = [
    keywords::ZIMAGE,
    keywords::ZCMPTYPE,
    keywords::ZBITPIX,
    keywords::ZNAXIS,
    keywords::ZQUANTIZ,
    keywords::ZDITHER0,
    keywords::ZBLANK,
    keywords::ZMASKCMP,
    keywords::ZSIMPLE,
    keywords::ZTENSION,
    keywords::ZEXTEND,
    keywords::ZBLOCKED,
    keywords::ZGCOUNT,
    keywords::ZHEAPPTR,
];

/// Indexed roots of tiled image compression (§10.1).
const IMAGE_COMPRESSION_INDEXED: [Keyword; 4] = [
    keywords::ZNAXIS,
    keywords::ZTILE,
    keywords::ZNAME,
    keywords::ZVAL,
];

/// Fixed-name keywords a tiled-compressed table's header adds (§10.3.1).
pub(crate) const TABLE_COMPRESSION: [Keyword; 5] = [
    keywords::ZTABLE,
    keywords::ZTILELEN,
    Keyword::new("ZNAXIS1"),
    Keyword::new("ZNAXIS2"),
    keywords::ZPCOUNT,
];

/// Column-indexed roots a tiled-compressed table's header adds (§10.3.1).
pub(crate) const TABLE_COMPRESSION_INDEXED: [Keyword; 2] = [keywords::ZFORM, keywords::ZCTYP];

/// The keywords table compression keeps under a `Z` name, as (table, container)
/// pairs (§10.3.1).
pub(crate) const TABLE_PRESERVED: [(Keyword, Keyword); 3] = [
    (keywords::THEAP, keywords::ZTHEAP),
    (keywords::CHECKSUM, keywords::ZHECKSUM),
    (keywords::DATASUM, keywords::ZDATASUM),
];

/// Whether a writer generates `keyword` rather than copying it from a template.
pub(crate) fn is_generated(keyword: Keyword) -> bool {
    let indexed = |root: &Keyword| keyword.index_after(*root).is_some();
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
pub(crate) fn is_table_compression(keyword: Keyword, columns: usize) -> bool {
    TABLE_COMPRESSION.contains(&keyword)
        || TABLE_COMPRESSION_INDEXED.iter().any(|&root| {
            keyword
                .index_after(root)
                .is_some_and(|column| (1..=columns).contains(&column))
        })
}
