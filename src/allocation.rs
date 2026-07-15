use std::mem::size_of;

use crate::error::FitsError;
use crate::error::Result;

pub(crate) fn try_reserve_exact<T>(values: &mut Vec<T>, additional: usize) -> Result<()> {
    let requested = values
        .len()
        .checked_add(additional)
        .ok_or(FitsError::DataUnitOverflow)?;
    values
        .try_reserve_exact(additional)
        .map_err(|_| FitsError::DataUnitTooLarge {
            bytes: allocation_bytes::<T>(requested),
        })
}

pub(crate) fn try_resize<T: Clone>(values: &mut Vec<T>, len: usize, value: T) -> Result<()> {
    if len > values.len() {
        try_reserve_exact(values, len - values.len())?;
    }
    values.resize(len, value);
    Ok(())
}

pub(crate) fn try_zeroed<T: Clone>(value: T, len: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    try_resize(&mut values, len, value)?;
    Ok(values)
}

pub(crate) fn try_copy<T: Clone>(values: &[T]) -> Result<Vec<T>> {
    let mut copy = Vec::new();
    try_reserve_exact(&mut copy, values.len())?;
    copy.extend_from_slice(values);
    Ok(copy)
}

fn allocation_bytes<T>(len: usize) -> u64 {
    (len as u64).saturating_mul(size_of::<T>() as u64)
}

#[cfg(test)]
mod tests {
    use crate::allocation::*;

    #[test]
    fn oversized_allocations_return_the_requested_byte_count() {
        let mut values = Vec::<u64>::new();
        let bytes = (usize::MAX as u64).saturating_mul(8);
        assert!(matches!(
            try_reserve_exact(&mut values, usize::MAX),
            Err(FitsError::DataUnitTooLarge { bytes: got }) if got == bytes
        ));
    }

    #[test]
    fn resize_reuses_capacity_and_preserves_exact_length() {
        let mut values = Vec::with_capacity(8);
        let ptr = values.as_ptr();
        try_resize(&mut values, 8, 3u8).unwrap();
        assert_eq!(values, [3; 8]);
        try_resize(&mut values, 2, 9).unwrap();
        assert_eq!(values, [3, 3]);
        assert_eq!(values.as_ptr(), ptr);
    }
}
