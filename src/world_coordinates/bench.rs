//! The WCS cases: the light's plate solution parsed from its header, and one row of pixels
//! through each kind of axis, both ways.

use std::hint::black_box;

use criterion::Criterion;

use crate::bench::policy::{self, Kind, Scale};
use crate::block::CARD_SIZE;
use crate::header_model::{Header, keywords};
use crate::internals::scene;
use crate::internals::scene::{HEIGHT, WIDTH};
use crate::internals::tolerance::{self, Tolerance};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular::internals::{
    INDEX_LENGTH, indexed_wcs, lookup_table, resolved_wcs, tab_header,
};

/// `atomic/parse_wcs/light`: the light's plate solution read from its header by
/// `Wcs::from_header`, as a caller reads it once an HDU. Per card of the header.
///
/// `atomic/to_world/<axes>` and `atomic/to_pixel/<axes>`: one row of the standard frame's
/// pixels, one call a point, through `Wcs::pixel_to_world` and back through
/// `Wcs::world_to_pixel`, checked once to round-trip. The axes: `tan`, the light's plate
/// solution; `sip`, that solution with an order-3 SIP distortion and no inverse polynomial, so
/// `to_pixel` iterates; `tpv`, the solution with a cubic TPV distortion of the same size;
/// `linear`, four coupled linear axes; `spectral`, a frequency axis sampled in wavelength
/// (Table 26); `tabular`, a `-TAB` axis over a 100 000-entry index; `tabular_coupled`, two
/// `-TAB` axes over one 64 × 64 grid. Per point, in the `f64` coordinates of its axes.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "parse_wcs", Scale::Row);
    let header = solved_header(Distortion::None);
    group.throughput(policy::throughput(header.len(), CARD_SIZE));
    group.bench_function("light", |b| {
        b.iter(|| Wcs::from_header(black_box(&header), None).unwrap());
    });
    group.finish();

    let mut built: [Option<Fixture>; Axes::ALL.len()] = Default::default();
    let mut group = Kind::Atomic.group(c, "to_world", Scale::Frame);
    for (axes, slot) in Axes::ALL.into_iter().zip(&mut built) {
        group.throughput(policy::throughput(WIDTH, axes.count() * size_of::<f64>()));
        group.bench_function(axes.id(), |b| {
            let Fixture { wcs, pixels, .. } = slot.get_or_insert_with(|| axes.fixture());
            let mut world = vec![0.0; wcs.axes.len()];
            b.iter(|| {
                for pixel in pixels.chunks_exact(world.len()) {
                    wcs.pixel_to_world(black_box(pixel), &mut world).unwrap();
                    black_box(&world);
                }
            });
        });
    }
    group.finish();
    let mut group = Kind::Atomic.group(c, "to_pixel", Scale::Frame);
    for (axes, slot) in Axes::ALL.into_iter().zip(&mut built) {
        group.throughput(policy::throughput(WIDTH, axes.count() * size_of::<f64>()));
        group.bench_function(axes.id(), |b| {
            let Fixture { wcs, worlds, .. } = slot.get_or_insert_with(|| axes.fixture());
            let mut pixel = vec![0.0; wcs.axes.len()];
            b.iter(|| {
                for world in worlds.chunks_exact(pixel.len()) {
                    wcs.world_to_pixel(black_box(world), &mut pixel).unwrap();
                    black_box(&pixel);
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
    Sip,
    Tpv,
    Linear,
    Spectral,
    Tabular,
    TabularCoupled,
}

impl Axes {
    const ALL: [Self; 7] = [
        Self::Tan,
        Self::Sip,
        Self::Tpv,
        Self::Linear,
        Self::Spectral,
        Self::Tabular,
        Self::TabularCoupled,
    ];

    /// The axes of the transform, the coordinates of each point.
    const fn count(self) -> usize {
        match self {
            Self::Tan | Self::Sip | Self::Tpv | Self::TabularCoupled => 2,
            Self::Linear => 4,
            Self::Spectral | Self::Tabular => 1,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Tan => "tan",
            Self::Sip => "sip",
            Self::Tpv => "tpv",
            Self::Linear => "linear",
            Self::Spectral => "spectral",
            Self::Tabular => "tabular",
            Self::TabularCoupled => "tabular_coupled",
        }
    }

    fn fixture(self) -> Fixture {
        let fixture = match self {
            Self::Tan => Fixture::celestial(Distortion::None),
            Self::Sip => Fixture::celestial(Distortion::Sip),
            Self::Tpv => Fixture::celestial(Distortion::Tpv),
            Self::Linear => Fixture::linear(),
            Self::Spectral => Fixture::spectral(),
            Self::Tabular => Fixture::tabular(),
            Self::TabularCoupled => Fixture::tabular_coupled(),
        };
        assert_eq!(
            fixture.wcs.axes.len(),
            self.count(),
            "the throughput counts each axis"
        );
        fixture
    }
}

/// The distortion a plate solution carries beside its projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Distortion {
    None,
    /// An order-3 SIP polynomial in pixels, with no `AP`/`BP` inverse, as a solver fits it:
    /// 3 px of quadratic and 4 px of cubic distortion at the frame's corners.
    Sip,
    /// A cubic TPV polynomial in degrees of the same size.
    Tpv,
}

/// The light's header, two image axes, with its plate solution and `distortion`.
fn solved_header(distortion: Distortion) -> Header {
    let mut header = scene::light_header();
    header.set_internal(keywords::NAXIS, 2);
    match distortion {
        Distortion::None => {}
        Distortion::Sip => {
            header
                .set_internal("CTYPE1", "RA---TAN-SIP")
                .set_internal("CTYPE2", "DEC--TAN-SIP")
                .set_internal("A_ORDER", 3)
                .set_internal("A_2_0", 2.1e-7)
                .set_internal("A_1_1", -1.3e-7)
                .set_internal("A_0_2", 0.8e-7)
                .set_internal("A_3_0", -1.5e-10)
                .set_internal("A_2_1", 0.7e-10)
                .set_internal("A_1_2", -1.1e-10)
                .set_internal("A_0_3", 0.4e-10)
                .set_internal("B_ORDER", 3)
                .set_internal("B_2_0", -0.9e-7)
                .set_internal("B_1_1", 1.7e-7)
                .set_internal("B_0_2", -1.2e-7)
                .set_internal("B_3_0", 0.5e-10)
                .set_internal("B_2_1", -1.4e-10)
                .set_internal("B_1_2", 0.9e-10)
                .set_internal("B_0_3", -1.6e-10);
        }
        Distortion::Tpv => {
            header
                .set_internal("CTYPE1", "RA---TPV")
                .set_internal("CTYPE2", "DEC--TPV")
                .set_internal("PV1_1", 1.0)
                .set_internal("PV1_4", 4.1e-4)
                .set_internal("PV1_5", -2.5e-4)
                .set_internal("PV1_6", 1.6e-4)
                .set_internal("PV1_7", -2.9e-4)
                .set_internal("PV1_8", 1.4e-4)
                .set_internal("PV1_9", -2.1e-4)
                .set_internal("PV1_10", 0.8e-4)
                .set_internal("PV2_1", 1.0)
                .set_internal("PV2_4", -2.3e-4)
                .set_internal("PV2_5", 3.1e-4)
                .set_internal("PV2_6", -1.2e-4)
                .set_internal("PV2_7", 1.9e-4)
                .set_internal("PV2_8", -0.6e-4)
                .set_internal("PV2_9", 2.4e-4)
                .set_internal("PV2_10", -1.7e-4);
        }
    }
    header
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
        let mut worlds = vec![0.0; pixels.len()];
        for (pixel, world) in pixels
            .chunks_exact(wcs.axes.len())
            .zip(worlds.chunks_exact_mut(wcs.axes.len()))
        {
            wcs.pixel_to_world(pixel, world).unwrap();
        }
        let mut back = vec![0.0; pixels.len()];
        for (world, pixel) in worlds
            .chunks_exact(wcs.axes.len())
            .zip(back.chunks_exact_mut(wcs.axes.len()))
        {
            wcs.world_to_pixel(world, pixel).unwrap();
        }
        let converged = Tolerance::abs(1e-6, "each inverse converges to a micro-pixel");
        tolerance::assert_close(&back, &pixels, converged, "the row round-trips");
        Self {
            wcs,
            pixels,
            worlds,
        }
    }

    /// The light's plate solution with `distortion`, over the row a tenth of the frame from
    /// its top, where the distortion is near its largest.
    fn celestial(distortion: Distortion) -> Self {
        let wcs = Wcs::from_header(&solved_header(distortion), None).unwrap();
        Self::new(wcs, |x| vec![x as f64 + 1.0, (HEIGHT / 10) as f64])
    }

    /// Four linear axes coupled by a `PC` matrix, over a diagonal through all four.
    fn linear() -> Self {
        let mut header = Header::new();
        header
            .set_internal(keywords::NAXIS, 4)
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
            .set_internal(keywords::NAXIS, 1)
            .set_internal("CTYPE1", "FREQ-W2F")
            .set_internal("CUNIT1", "Hz")
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", 1_420_405_751.0)
            .set_internal("CDELT1", 1e6)
            .set_internal(keywords::RESTFRQ, 1_420_405_751.0);
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

    /// Two `-TAB` axes over one 64 × 64 grid of coordinate pairs, gently curved so each
    /// voxel is its own trilinear map, the row crossing the grid on a slant.
    fn tabular_coupled() -> Self {
        let mut grid = Vec::with_capacity(2 * COUPLED_SIDE * COUPLED_SIDE);
        for j in 0..COUPLED_SIDE {
            for i in 0..COUPLED_SIDE {
                let (i, j) = (i as f64, j as f64);
                grid.push(100.0 + 10.0 * i + 0.02 * i * i + 0.5 * j);
                grid.push(200.0 + 20.0 * j + 0.03 * j * j + 0.25 * i);
            }
        }
        let shape = format!("(2,{COUPLED_SIDE},{COUPLED_SIDE})");
        let table = lookup_table(&[("COORD", &grid, Some(shape.as_str()))]);
        let wcs = resolved_wcs(&tab_header(2), &table);
        let last = (COUPLED_SIDE - 1) as f64;
        Self::new(wcs, |x| {
            let t = x as f64 / WIDTH as f64;
            vec![1.0 + (last - 0.5) * t, 1.0 + last * (0.25 + 0.5 * t)]
        })
    }
}

/// The vertices along each axis of the coupled `-TAB` grid.
const COUPLED_SIDE: usize = 64;
