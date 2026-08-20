//! `TDIMn` array-shape parsing and the two validity rules it must satisfy (§7.3.2).

use crate::error::FitsError;
use crate::error::Result;

/// Parse a `TDIMn` value `'(d1,d2,…)'` into axis lengths (fastest-varying first).
pub(super) fn parse(value: &str) -> Result<Vec<usize>> {
    let invalid = || FitsError::KeywordOutOfRange { name: "TDIMn" };
    let inner = value
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(invalid)?;
    let dims: Vec<usize> = inner
        .split(',')
        .map(|value| value.trim().parse::<usize>().map_err(|_| invalid()))
        .collect::<Result<_>>()?;
    validate_shape(&dims)?;
    Ok(dims)
}

/// A `TDIMn` shape must name at least one axis and no zero-length one — the rule
/// both the parsed (`TDIMn` card) and supplied (writer) shapes obey.
fn validate_shape(dims: &[usize]) -> Result<()> {
    if dims.is_empty() || dims.contains(&0) {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
    Ok(())
}

/// §7.3.2: a `TDIMn` shape may describe fewer elements than the cell holds (trailing
/// elements beyond the declared view are permitted) but never more.
pub(super) fn validate_extent(dims: &[usize], element_count: usize) -> Result<()> {
    let product = dims
        .iter()
        .try_fold(1usize, |product, &len| product.checked_mul(len))
        .ok_or(FitsError::KeywordOutOfRange { name: "TDIMn" })?;
    if product > element_count {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
    Ok(())
}

/// The extent rule as it applies to a `P`/`Q` heap array: an empty descriptor
/// carries no elements to reshape, so the declared shape is simply not applied to
/// that row.
pub(super) fn validate_vla_extent(dims: &[usize], element_count: usize) -> Result<()> {
    if element_count == 0 {
        return Ok(());
    }
    validate_extent(dims, element_count)
}

/// Both rules for a *caller-supplied* fixed-width `TDIMn`: the shape is well-formed
/// and describes no more elements than the cell holds.
///
/// The read path applies only the extent rule, because a shape that came off a
/// `TDIMn` card was already checked for well-formedness by [`parse`]; the writer's
/// shape arrives straight from the caller and so needs both.
pub(crate) fn validate_declared(shape: Option<&[usize]>, element_count: usize) -> Result<()> {
    let Some(shape) = shape else {
        return Ok(());
    };
    validate_shape(shape)?;
    validate_extent(shape, element_count)
}

/// [`validate_declared`] for a `P`/`Q` row, which skips the extent rule on an empty
/// heap array.
pub(crate) fn validate_declared_vla(shape: Option<&[usize]>, element_count: usize) -> Result<()> {
    let Some(shape) = shape else {
        return Ok(());
    };
    validate_shape(shape)?;
    validate_vla_extent(shape, element_count)
}

#[cfg(test)]
mod tests;
