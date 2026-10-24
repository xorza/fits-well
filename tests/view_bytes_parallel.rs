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

#[test]
fn a_run_of_views_on_four_workers_peaks_within_its_view_bytes() {
    #[cfg(feature = "parallel")]
    {
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .use_current_thread()
            .build_global()
            .unwrap();
        rayon::broadcast(|_| ());
    }
    peak::check(&peak::cases());
}
