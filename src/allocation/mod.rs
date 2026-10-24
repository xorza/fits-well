//! Fallible allocation for buffers sized directly from untrusted FITS metadata.

use crate::fits_error::FitsError;
use crate::fits_error::Result;

pub(crate) fn try_reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<()> {
    let requested = values
        .len()
        .checked_add(additional)
        .ok_or(FitsError::DataUnitOverflow)?;
    values
        .try_reserve(additional)
        .map_err(|_| FitsError::DataUnitTooLarge {
            bytes: allocation_bytes::<T>(requested),
        })
}

pub(crate) fn try_resize<T: Clone>(values: &mut Vec<T>, len: usize, value: T) -> Result<()> {
    if len > values.len() {
        let additional = len - values.len();
        values
            .try_reserve_exact(additional)
            .map_err(|_| FitsError::DataUnitTooLarge {
                bytes: allocation_bytes::<T>(len),
            })?;
    }
    values.resize(len, value);
    Ok(())
}

pub(crate) fn try_zeroed<T: Clone>(value: T, len: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    try_resize(&mut values, len, value)?;
    Ok(values)
}

const fn allocation_bytes<T>(len: usize) -> u64 {
    (len as u64).saturating_mul(size_of::<T>() as u64)
}

#[cfg(test)]
mod tests;
