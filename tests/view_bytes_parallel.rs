//! The bytes an image reader holds viewing a tiled-compressed image on a pool of four
//! workers, held to the allocator: [`ImageReader::view_bytes`] counts a tile set for
//! each worker that may take a run of the tiles, so a run of views peaks at most there.
//!
//! [`ImageReader::view_bytes`]: fits_well::ImageReader::view_bytes

#![cfg(feature = "compression")]
#![expect(
    clippy::disallowed_types,
    reason = "a global allocator has no owner to carry its counters"
)]

mod support;

use crate::support::peak;

/// Make every worker of the pool block once in rayon's sleep, so the pool holds what that makes
/// before a run is measured. On macOS std allocates a `Condvar`'s `pthread_cond_t`, 48 B, the
/// first time it waits (rust-lang/rust#93740), and each rayon worker sleeps on a `Condvar` of its
/// own; a worker first blocking mid-run put 48, 96 or 144 B past the figure, which the trace of a
/// loaded Mac caught in `Sleep::sleep`. The calling thread, a worker by `use_current_thread`,
/// waits on the others while they nap, then the others idle while it naps. A nap far longer than
/// rayon's spin before it sleeps.
#[cfg(feature = "parallel")]
fn block_every_worker_once() {
    use std::thread;
    use std::time::Duration;

    const NAP: Duration = Duration::from_millis(50);
    let caller = rayon::current_thread_index().expect("the calling thread is a worker");
    rayon::broadcast(|ctx| {
        if ctx.index() != caller {
            thread::sleep(NAP);
        }
    });
    rayon::broadcast(|ctx| {
        if ctx.index() == caller {
            thread::sleep(NAP);
        }
    });
}

#[test]
fn run_of_views_on_four_workers_peaks_within_its_view_bytes() {
    #[cfg(feature = "parallel")]
    {
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .use_current_thread()
            .build_global()
            .unwrap();
        block_every_worker_once();
    }
    peak::check(&peak::cases());
}
