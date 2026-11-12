//! The failure of the lowest index among the items of a parallel run, as rayon's
//! `find_map_first` keeps its leftmost match: rayon's own `try_for_each` and `collect` into a
//! `Result` return whichever failure a worker meets first in time.

use crate::fits_error::{FitsError, Result};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Run `f` on each item of `items` with its index and a scratch from `init`, as rayon's
/// `enumerate().try_for_each_init(…)` does, and return the failure of the lowest index.
/// `find_map_first` skips the work to the right of a failure it found, as no failure there
/// could be the first.
pub(super) fn try_for_each_init<I, S, F>(
    items: I,
    init: impl Fn() -> S + Sync + Send,
    f: F,
) -> Result<()>
where
    I: IndexedParallelIterator,
    F: Fn(&mut S, (usize, I::Item)) -> Result<()> + Sync + Send,
{
    let failure = items
        .enumerate()
        .map_init(init, |scratch, item| f(scratch, item).err())
        .find_map_first(|failure| failure);
    failure.map_or(Ok(()), Err)
}

/// What an item of [`try_map_init`] leaves: `None` where it skipped. The error is boxed, as a
/// `FitsError` is 64 B and every item holds the slot: a slot of a 24 B value stays 24 B.
type Slot<T> = Option<std::result::Result<T, Box<FitsError>>>;

/// `f` of each index of `indices` with a scratch from `init`, in order, as rayon's
/// `map_init(…).collect()` into a `Result` gives them, and the failure of the lowest
/// index.
///
/// An index past the lowest failure seen so far skips, as its failure could not be the first:
/// the hint only falls, so every skipped index comes after the failure the ordered collect
/// stops at.
pub(super) fn try_map_init<S, T: Send>(
    indices: Range<usize>,
    init: impl Fn() -> S + Sync + Send,
    f: impl Fn(&mut S, usize) -> Result<T> + Sync + Send,
) -> Result<Vec<T>> {
    let lowest_failure = AtomicUsize::new(usize::MAX);
    let slots: Vec<Slot<T>> = indices
        .into_par_iter()
        .map_init(init, |scratch, index| {
            if index > lowest_failure.load(Ordering::Relaxed) {
                return None;
            }
            let result = f(scratch, index).map_err(Box::new);
            if result.is_err() {
                lowest_failure.fetch_min(index, Ordering::Relaxed);
            }
            Some(result)
        })
        .collect();
    slots
        .into_iter()
        .map(|slot| {
            slot.expect("an index skips only past a failure, which comes first")
                .map_err(|error| *error)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::compression::first_failure::{self, Slot};
    use crate::fits_error::{FitsError, Indexed};
    use rayon::iter::IntoParallelRefMutIterator;
    use std::assert_matches;

    /// Items 3, 40 and 700 fail, and item 3 works first, so a worker meets 700 or 40
    /// before it: each run still reports item 3's failure, and a run with no failure
    /// keeps every result in order.
    #[test]
    fn lowest_failing_index_wins_and_results_keep_their_order() {
        let fail = |index: usize| -> Result<usize, FitsError> {
            if index == 3 {
                std::hint::black_box((0..200_000u64).sum::<u64>());
            }
            if [3, 40, 700].contains(&index) {
                Err(FitsError::IndexOutOfBounds {
                    indexed: Indexed::Column,
                    index,
                    len: 0,
                })
            } else {
                Ok(index * 2)
            }
        };
        for _ in 0..20 {
            assert_matches!(
                first_failure::try_map_init(0..1000, || (), |(), index| fail(index)),
                Err(FitsError::IndexOutOfBounds { index: 3, .. })
            );
            let mut cells = vec![0usize; 1000];
            assert_matches!(
                first_failure::try_for_each_init(
                    cells.par_iter_mut(),
                    || (),
                    |(), (index, cell)| {
                        *cell = fail(index)?;
                        Ok(())
                    }
                ),
                Err(FitsError::IndexOutOfBounds { index: 3, .. })
            );
        }
        let doubled = first_failure::try_map_init(0..10, || (), |(), index| Ok(index * 2)).unwrap();
        assert_eq!(doubled, (0..10).map(|index| index * 2).collect::<Vec<_>>());
    }

    /// With the error boxed, a slot costs what its value alone does: 24 B for a tile's bytes.
    #[test]
    fn a_slot_is_as_small_as_its_value() {
        assert_eq!(size_of::<FitsError>(), 64);
        assert_eq!(size_of::<Slot<Vec<u8>>>(), size_of::<Option<Vec<u8>>>());
        assert_eq!(size_of::<Slot<Vec<u8>>>(), 24);
    }
}
