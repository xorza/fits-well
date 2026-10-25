//! The failure of the lowest index among the items of a parallel run, as rayon's
//! `find_first` keeps its leftmost match: rayon's own `try_for_each` and `collect` into a
//! `Result` return whichever failure a worker meets first in time.

use crate::fits_error::{FitsError, Result};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use std::ops::Range;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The failing item of the lowest index seen so far. An item past it can skip, as its
/// failure could not be the first; an item before it runs, as it could. So the failure
/// kept at the end is the first in item order, whichever worker meets it.
#[derive(Debug)]
struct FirstFailure {
    /// The index of `failure`, or `usize::MAX` for none: a hint that only falls.
    index: AtomicUsize,
    failure: Mutex<Option<Failure>>,
}

#[derive(Debug)]
struct Failure {
    index: usize,
    error: FitsError,
}

impl FirstFailure {
    const fn new() -> Self {
        Self {
            index: AtomicUsize::new(usize::MAX),
            failure: Mutex::new(None),
        }
    }

    /// Whether item `index` comes after a failure already seen.
    fn skips(&self, index: usize) -> bool {
        index > self.index.load(Ordering::Relaxed)
    }

    fn record(&self, index: usize, error: FitsError) {
        let mut failure = self.failure.lock().unwrap();
        if failure.as_ref().is_none_or(|kept| index < kept.index) {
            *failure = Some(Failure { index, error });
            self.index.fetch_min(index, Ordering::Relaxed);
        }
    }

    fn into_result(self) -> Result<()> {
        match self.failure.into_inner().unwrap() {
            Some(Failure { error, .. }) => Err(error),
            None => Ok(()),
        }
    }
}

/// Run `f` on each item of `items` with its index and a scratch from `init`, as rayon's
/// `enumerate().try_for_each_init(…)` does, and return the failure of the lowest index.
pub(super) fn try_for_each_init<I, S, F>(
    items: I,
    init: impl Fn() -> S + Sync + Send,
    f: F,
) -> Result<()>
where
    I: IndexedParallelIterator,
    F: Fn(&mut S, (usize, I::Item)) -> Result<()> + Sync + Send,
{
    let first = FirstFailure::new();
    items
        .enumerate()
        .for_each_init(init, |scratch, (index, item)| {
            if !first.skips(index)
                && let Err(error) = f(scratch, (index, item))
            {
                first.record(index, error);
            }
        });
    first.into_result()
}

/// `f` of each index of `indices` with a scratch from `init`, in order, as rayon's
/// `map_init(…).collect()` into a `Result` gives them, and the failure of the lowest
/// index.
pub(super) fn try_map_init<S, T: Send>(
    indices: Range<usize>,
    init: impl Fn() -> S + Sync + Send,
    f: impl Fn(&mut S, usize) -> Result<T> + Sync + Send,
) -> Result<Vec<T>> {
    let first = FirstFailure::new();
    let values: Vec<Option<T>> = indices
        .into_par_iter()
        .map_init(init, |scratch, index| {
            if first.skips(index) {
                return None;
            }
            f(scratch, index)
                .map_err(|error| first.record(index, error))
                .ok()
        })
        .collect();
    first.into_result()?;
    Ok(values
        .into_iter()
        .map(|value| value.expect("every item ran, as none failed"))
        .collect())
}

#[cfg(test)]
mod tests {
    use crate::compression::first_failure;
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
}
