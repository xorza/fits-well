//! Reading WCS keywords from a header: a keyword that a formatted name cannot fit has
//! no card, and an axis-indexed family reads as one value per axis.

use crate::fits_error::{FitsError, Result};
use crate::header_model::Header;
use crate::header_model::value::Value;
use crate::keyword::Keyword;

/// The largest `m` of a `PVi_m` keyword, two digits (§8.2). A table WCS translates
/// the whole range into image keywords, so each reader of the parameters finds every
/// one it takes: a projection's 0 to 20 and TPV's 0 to 39.
pub(super) const MAX_PARAMETER_INDEX: usize = 99;

/// The per-axis `CRVALia`, `CRPIXia` and `CDELTia` of a description, each absent card
/// at its default: 0, 0 and 1 (§8.1).
#[derive(Debug)]
pub(super) struct AxisValues {
    pub(super) crval: Vec<f64>,
    pub(super) crpix: Vec<f64>,
    pub(super) cdelt: Vec<f64>,
}

/// The [`AxisValues`] of the description `alt` of `header`, of `naxis` axes.
pub(super) fn axis_values(header: &Header, alt: &str, naxis: usize) -> Result<AxisValues> {
    let family = |prefix: &str, default: f64| -> Result<Vec<f64>> {
        (1..=naxis)
            .map(|i| {
                real(header, Keyword::fitting(format_args!("{prefix}{i}{alt}")))
                    .map(|value| value.unwrap_or(default))
            })
            .collect()
    };
    Ok(AxisValues {
        crval: family("CRVAL", 0.0)?,
        crpix: family("CRPIX", 0.0)?,
        cdelt: family("CDELT", 1.0)?,
    })
}

pub(super) fn inferred_axis_count(header: &Header, alt: &str) -> i64 {
    header
        .iter()
        .filter_map(|record| image_wcs_axis_index(record.keyword()?.as_str(), alt))
        .max()
        .unwrap_or(0)
}

fn image_wcs_axis_index(keyword: &str, alt: &str) -> Option<i64> {
    let keyword = if alt.is_empty() {
        keyword
    } else {
        keyword.strip_suffix(alt)?
    };
    for prefix in [
        "CTYPE", "CUNIT", "CRVAL", "CDELT", "CRPIX", "CROTA", "CNAME", "CRDER", "CSYER", "CZPHS",
        "CPERI",
    ] {
        if let Some(index) = keyword.strip_prefix(prefix).and_then(parse_wcs_index) {
            return Some(index);
        }
    }
    for prefix in ["PC", "CD"] {
        if let Some(indices) = keyword.strip_prefix(prefix)
            && let Some((i, j)) = indices.split_once('_')
        {
            return Some(parse_wcs_index(i)?.max(parse_wcs_index(j)?));
        }
    }
    for prefix in ["PV", "PS"] {
        if let Some(indices) = keyword.strip_prefix(prefix)
            && let Some((i, m)) = indices.split_once('_')
        {
            m.parse::<u64>().ok()?;
            return parse_wcs_index(i);
        }
    }
    None
}

fn parse_wcs_index(value: &str) -> Option<i64> {
    value.parse().ok().filter(|&index| index > 0)
}

/// A declared axis count, checked against the standard's `1..=999` range.
pub(super) fn axis_count(value: i64, keyword: Keyword) -> Result<usize> {
    let count = usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { keyword })?;
    if !(1..=999).contains(&count) {
        return Err(FitsError::KeywordOutOfRange { keyword });
    }
    Ok(count)
}

/// The first of two keywords that carries a real value — the shape every
/// long/short and primary/`PVi_m` keyword alternative takes.
pub(super) fn first_real(
    header: &Header,
    first: impl Into<Option<Keyword>>,
    second: impl Into<Option<Keyword>>,
) -> Result<Option<f64>> {
    match real(header, first)? {
        Some(value) => Ok(Some(value)),
        None => real(header, second),
    }
}

/// The value of `keyword`; `None` when the card is absent, or when there is no keyword,
/// as where a wide axis or column number makes a formatted name longer than a keyword,
/// and so no card can hold it.
pub(super) fn value(header: &Header, keyword: impl Into<Option<Keyword>>) -> Option<&Value> {
    header
        .get(keyword.into()?)
        .expect("a `Keyword` argument needs no conversion")
}

/// [`value`] for a real keyword.
pub(super) fn real(header: &Header, keyword: impl Into<Option<Keyword>>) -> Result<Option<f64>> {
    keyword
        .into()
        .map_or(Ok(None), |keyword| header.get_real(keyword))
}

/// [`value`] for a text keyword.
pub(super) fn text(header: &Header, keyword: impl Into<Option<Keyword>>) -> Result<Option<&str>> {
    keyword
        .into()
        .map_or(Ok(None), |keyword| header.get_text(keyword))
}

/// [`value`] for an integer keyword.
pub(super) fn integer(header: &Header, keyword: impl Into<Option<Keyword>>) -> Result<Option<i64>> {
    keyword
        .into()
        .map_or(Ok(None), |keyword| header.get_integer(keyword))
}
