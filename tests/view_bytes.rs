//! The bytes an image reader holds viewing a tiled-compressed image, held to the
//! allocator on a pool of one worker: [`ImageReader::view_bytes`] is the peak a run of
//! views reaches, for each tiling and codec, as the one worker makes the one tile set
//! the figure counts.
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
fn a_run_of_views_on_one_worker_peaks_at_its_view_bytes() {
    #[cfg(feature = "parallel")]
    {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .use_current_thread()
            .build_global()
            .unwrap();
    }
    peak::check(&peak::cases());
}
