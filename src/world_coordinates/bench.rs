//! The WCS cases: one row of pixels through each kind of axis, both ways.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::bench::fixture::{Kind, Scale};
use crate::bench::scene::{self, HEIGHT, WIDTH};
use crate::header_model::Header;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular::internals::{INDEX_LENGTH, indexed_wcs};

/// `atomic/wcs/<direction>/<axes>`: one row of the standard frame's pixels, one call a point,
/// `to_world` through `Wcs::pixel_to_world` and `to_pixel` back through `Wcs::world_to_pixel`.
/// The axes: `tan`, the light's plate solution; `linear`, four coupled linear axes; `spectral`,
/// a frequency axis sampled in wavelength (Table 26); `tabular`, a `-TAB` axis over a 100 000-entry
/// index. Per point.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "wcs", Scale::Frame);
    group.throughput(Throughput::Elements(WIDTH as u64));
    for axes in Axes::ALL {
        let mut built = None;
        Scale::Frame.apply(&mut group);
        group.bench_function(BenchmarkId::new("to_world", axes.id()), |b| {
            let Fixture { wcs, pixels, .. } = built.get_or_insert_with(|| axes.fixture());
            let naxis = wcs.axes.len();
            b.iter(|| {
                for pixel in pixels.chunks_exact(naxis) {
                    black_box(wcs.pixel_to_world(black_box(pixel)).unwrap());
                }
            });
        });
        axes.inverse_scale().apply(&mut group);
        group.bench_function(BenchmarkId::new("to_pixel", axes.id()), |b| {
            let Fixture { wcs, worlds, .. } = built.get_or_insert_with(|| axes.fixture());
            let naxis = wcs.axes.len();
            b.iter(|| {
                for world in worlds.chunks_exact(naxis) {
                    black_box(wcs.world_to_pixel(black_box(world)).unwrap());
                }
            });
        });
    }
    group.finish();
}

/// The kinds of axis a case transforms through.
#[derive(Debug, Clone, Copy)]
enum Axes {
    Tan,
    Linear,
    Spectral,
    Tabular,
}

impl Axes {
    const ALL: [Self; 4] = [Self::Tan, Self::Linear, Self::Spectral, Self::Tabular];

    const fn id(self) -> &'static str {
        match self {
            Self::Tan => "tan",
            Self::Linear => "linear",
            Self::Spectral => "spectral",
            Self::Tabular => "tabular",
        }
    }

    /// The sampling of the inverse: a `-TAB` axis searches its coordinates for the interval
    /// holding each point, which costs 476 ms a row over the 100 000-entry index as measured on a
    /// Ryzen 7 6800U.
    const fn inverse_scale(self) -> Scale {
        match self {
            Self::Tabular => Scale::Set {
                iteration: Duration::from_millis(480),
            },
            Self::Tan | Self::Linear | Self::Spectral => Scale::Frame,
        }
    }

    fn fixture(self) -> Fixture {
        match self {
            Self::Tan => Fixture::tan(),
            Self::Linear => Fixture::linear(),
            Self::Spectral => Fixture::spectral(),
            Self::Tabular => Fixture::tabular(),
        }
    }
}

/// A transform and one row of points through it: the pixels, and the world points they map to,
/// each flat, one point after another.
#[derive(Debug)]
struct Fixture {
    wcs: Wcs,
    pixels: Vec<f64>,
    worlds: Vec<f64>,
}

impl Fixture {
    /// `wcs` over the row of points `pixel` gives for each column.
    fn new(wcs: Wcs, pixel: impl Fn(usize) -> Vec<f64>) -> Self {
        let pixels: Vec<f64> = (0..WIDTH).flat_map(pixel).collect();
        let worlds = pixels
            .chunks_exact(wcs.axes.len())
            .flat_map(|pixel| wcs.pixel_to_world(pixel).unwrap())
            .collect();
        Self {
            wcs,
            pixels,
            worlds,
        }
    }

    /// The light's plate solution over the middle row of the frame.
    fn tan() -> Self {
        let mut header = scene::light_header();
        header.set_internal("NAXIS", 2);
        let wcs = Wcs::from_header(&header, None).unwrap();
        Self::new(wcs, |x| vec![x as f64 + 1.0, (HEIGHT / 2) as f64])
    }

    /// Four linear axes coupled by a `PC` matrix, over a diagonal through all four.
    fn linear() -> Self {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 4)
            .set_internal("CTYPE1", "LINEAR")
            .set_internal("CTYPE2", "LINEAR")
            .set_internal("CTYPE3", "LINEAR")
            .set_internal("CTYPE4", "LINEAR")
            .set_internal("CRPIX1", 10.0)
            .set_internal("CRPIX2", -3.0)
            .set_internal("CRPIX3", 5.0)
            .set_internal("CRPIX4", 20.0)
            .set_internal("CRVAL1", 100.0)
            .set_internal("CRVAL2", -20.0)
            .set_internal("CRVAL3", 0.5)
            .set_internal("CRVAL4", 1_000.0)
            .set_internal("CDELT1", 0.25)
            .set_internal("CDELT2", 2.0)
            .set_internal("CDELT3", -0.5)
            .set_internal("CDELT4", 4.0)
            .set_internal("PC1_2", 0.1)
            .set_internal("PC2_3", -0.2)
            .set_internal("PC3_4", 0.3)
            .set_internal("PC4_1", -0.4);
        let wcs = Wcs::from_header(&header, None).unwrap();
        Self::new(wcs, |x| {
            let step = x as f64 * 0.001;
            vec![11.0 + step, -4.0 - step, 8.0 + step, 23.0 - step]
        })
    }

    /// The 21 cm line's frequency axis, 1 MHz a pixel, sampled in wavelength.
    fn spectral() -> Self {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", "FREQ-W2F")
            .set_internal("CUNIT1", "Hz")
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", 1_420_405_751.0)
            .set_internal("CDELT1", 1e6)
            .set_internal("RESTFRQ", 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        Self::new(wcs, |x| vec![1.0 + x as f64 * 0.001])
    }

    /// A `-TAB` axis over a 100 000-entry index, the row spread across the whole index.
    fn tabular() -> Self {
        let span = (2 * (INDEX_LENGTH - 1)) as f64;
        Self::new(indexed_wcs().clone(), |x| {
            vec![x as f64 * span / WIDTH as f64]
        })
    }
}
