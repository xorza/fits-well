//! The allocator the allocation tests count with: the allocations each thread makes, and
//! the bytes every thread but the harness's holds, with their peak.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

/// [`System`], counting the calling thread's allocations, and the bytes the allocations
/// of every thread but the harness's hold and the most they held since
/// [`Counting::restart_peak`]. The harness's thread — libtest's main thread, which runs
/// the test on a thread of its own and waits for it — is the first to allocate, and the
/// one left out of the bytes.
#[derive(Debug)]
pub(crate) struct Counting {
    live: AtomicIsize,
    peak: AtomicIsize,
    harness_seen: AtomicBool,
}

thread_local! {
    /// Whether this thread's bytes are counted, once its first allocation decided it.
    static COUNTED: Cell<Option<bool>> = const { Cell::new(None) };
    /// The allocations this thread made.
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
pub(crate) static ALLOCATOR: Counting = Counting {
    live: AtomicIsize::new(0),
    peak: AtomicIsize::new(0),
    harness_seen: AtomicBool::new(false),
};

impl Counting {
    fn counts_this_thread(&self) -> bool {
        // A thread whose locals are gone is counted: a count can over-read, never under.
        COUNTED
            .try_with(|counted| {
                let decided = counted
                    .get()
                    .unwrap_or_else(|| self.harness_seen.swap(true, Ordering::Relaxed));
                counted.set(Some(decided));
                decided
            })
            .unwrap_or(true)
    }

    fn grow(&self, bytes: usize) {
        // A const-initialized `Cell` has no destructor to register, so this access
        // cannot allocate.
        let _ = CALLS.try_with(|calls| calls.set(calls.get() + 1));
        if self.counts_this_thread() {
            let live = self.live.fetch_add(bytes as isize, Ordering::Relaxed) + bytes as isize;
            self.peak.fetch_max(live, Ordering::Relaxed);
        }
    }

    fn shrink(&self, bytes: usize) {
        if self.counts_this_thread() {
            self.live.fetch_sub(bytes as isize, Ordering::Relaxed);
        }
    }

    pub(crate) fn live(&self) -> isize {
        self.live.load(Ordering::Relaxed)
    }

    /// Count the peak again from what is live now, which this returns.
    pub(crate) fn restart_peak(&self) -> isize {
        let live = self.live();
        self.peak.store(live, Ordering::Relaxed);
        live
    }

    pub(crate) fn peak(&self) -> isize {
        self.peak.load(Ordering::Relaxed)
    }

    /// The allocations the calling thread makes running `run`.
    pub(crate) fn calls(&self, run: impl FnOnce() -> f64) -> usize {
        CALLS.with(|calls| calls.set(0));
        black_box(run());
        CALLS.with(Cell::get)
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.grow(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.grow(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        self.shrink(layout.size());
        unsafe { System.dealloc(pointer, layout) }
    }

    /// The new block counts before the old one goes: a block that moves is copied while
    /// both are held.
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.grow(new_size);
        self.shrink(layout.size());
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}
