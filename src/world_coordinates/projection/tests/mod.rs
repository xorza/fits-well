use crate::fits_error::FitsError;
use crate::fits_error::WcsConflict;
use crate::fits_error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::internals::tolerance;
use crate::internals::tolerance::Tolerance;
use crate::internals::wcs_header;
use crate::internals::wcs_header::WcsHeader;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::angle;
use crate::world_coordinates::angle::{D2R, R2D};
use crate::world_coordinates::internals::AstropyGolden;
use crate::world_coordinates::internals::CEA_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;
use crate::world_coordinates::internals::{to_pixel, to_world};
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::conic::{Conic, ConicKind};
use crate::world_coordinates::projection::zenithal::{Air, evaluate_polynomial};
use crate::world_coordinates::projection::{Projection, ProjectionParameters};
use std::assert_matches;
use std::f64::consts::SQRT_2;

mod cube;
mod equatorial;
mod healpix;
mod perspective;
mod zenithal;

/// A celestial WCS and its pixel → world golden: `RA---`/`DEC--` of `projection`, CRPIX
/// `crpix` and CDELT (−`cdelt`, `cdelt`) on both axes, and the projection's
/// [`Projection::test_parameters`].
#[derive(Debug)]
struct ProjectionGolden {
    projection: Projection,
    crpix: f64,
    crval: [f64; 2],
    cdelt: f64,
    golden: AstropyGolden<'static>,
}

/// From `astropy.wcs` (wcslib). Zenithal centres sit at CRVAL (150, 2.5) or (45, 30); the
/// cylindricals at (45, 30) away from the equator, so the whole pole computation runs.
const PROJECTION_GOLDEN: &[ProjectionGolden] = &[
    ProjectionGolden {
        projection: Projection::Sin,
        crpix: 100.0,
        crval: [45.0, 30.0],
        cdelt: 1e-3,
        golden: AstropyGolden {
            decimals: 12,
            points: &[
                (100.0, 100.0, 45.000_000_000_000, 30.000_000_000_000),
                (50.0, 150.0, 45.057_764_154_844, 30.049_987_404_157),
                (1.0, 1.0, 45.114_201_616_520, 29.900_950_619_091),
                (180.0, 20.0, 44.907_698_264_374, 29.919_967_754_584),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Stg,
        crpix: 50.0,
        crval: [150.0, 2.5],
        cdelt: 0.05,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (10.0, 80.0, 152.004_333_716_6, 3.997_931_693_5),
                (90.0, 20.0, 148.000_241_577_3, 0.999_020_079_8),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Zea,
        crpix: 50.0,
        crval: [150.0, 2.5],
        cdelt: 0.05,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (10.0, 80.0, 152.004_811_494_4, 3.998_287_680_0),
                (30.0, 60.0, 151.001_375_296_5, 2.999_601_366_2),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Car,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.05,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (10.0, 80.0, 47.344_516_949_5, 31.479_541_625_1),
                (90.0, 20.0, 42.725_285_575_5, 28.480_150_705_2),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Cea,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.05,
        golden: CEA_GOLDEN,
    },
    ProjectionGolden {
        projection: Projection::Mer,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.05,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (10.0, 80.0, 47.344_512_839_3, 31.479_370_343_0),
                (90.0, 20.0, 42.725_281_706_2, 28.480_321_989_4),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Sfl,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.05,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (10.0, 80.0, 47.345_320_402_9, 31.479_527_599_7),
                (30.0, 60.0, 46.160_552_123_6, 30.494_936_029_2),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Ait,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.2,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (20.0, 70.0, 52.223_519_732_8, 33.810_076_325_4),
                (80.0, 30.0, 38.334_727_495_7, 25.825_831_081_3),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Mol,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.2,
        golden: AstropyGolden {
            decimals: 10,
            points: &[
                (20.0, 70.0, 52.981_660_279_9, 33.369_973_956_3),
                (80.0, 30.0, 37.575_352_555_3, 26.181_823_327_0),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Zpn,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.2,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 52.208_830_352, 33.797_790_311),
                (80.0, 30.0, 38.346_539_565, 25.839_502_283),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Cyp,
        crpix: 50.0,
        crval: [45.0, 0.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 75.0, 13.273_646_093),
                (80.0, 30.0, 15.0, -13.273_646_093),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Par,
        crpix: 50.0,
        crval: [45.0, 0.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 60.1875, 9.554_215_610),
                (80.0, 30.0, 29.8125, -9.554_215_610),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Cop,
        crpix: 50.0,
        crval: [45.0, 45.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 70.886_680_135, 52.802_260_739),
                (80.0, 30.0, 26.716_181_056, 33.063_457_476),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Coe,
        crpix: 50.0,
        crval: [45.0, 45.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 70.744_981_732, 52.427_253_763),
                (80.0, 30.0, 26.612_080_121, 33.642_902_217),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Cod,
        crpix: 50.0,
        crval: [45.0, 45.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 70.845_584_231, 52.615_170_165),
                (80.0, 30.0, 26.678_352_755, 33.316_436_438),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Coo,
        crpix: 50.0,
        crval: [45.0, 45.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (20.0, 70.0, 70.936_065_152, 52.798_760_425),
                (80.0, 30.0, 26.752_614_879, 32.966_997_552),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Bon,
        crpix: 50.0,
        crval: [45.0, 30.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (40.0, 60.0, 51.090_826_613, 34.738_247_010),
                (70.0, 35.0, 34.224_478_842, 21.570_942_288),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Air,
        crpix: 50.0,
        crval: [45.0, 60.0],
        cdelt: 0.3,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (40.0, 60.0, 51.871_584_561, 62.956_093_827),
                (70.0, 35.0, 34.141_611_622, 54.816_671_832),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Azp,
        crpix: 50.0,
        crval: [45.0, 60.0],
        cdelt: 0.3,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (40.0, 60.0, 51.434_150_697, 62.429_650_080),
                (70.0, 35.0, 34.214_637_058, 55.561_587_347),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Pco,
        crpix: 50.0,
        crval: [45.0, 0.0],
        cdelt: 0.5,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (40.0, 60.0, 50.019_002_131, 4.980_985_613),
                (70.0, 35.0, 34.915_451_766, -7.386_849_830),
                (55.0, 55.0, 42.497_621_311, 2.497_620_932),
            ],
        },
    },
    ProjectionGolden {
        projection: Projection::Szp,
        crpix: 50.0,
        crval: [45.0, 60.0],
        cdelt: 0.3,
        golden: AstropyGolden {
            decimals: 9,
            points: &[
                (40.0, 60.0, 51.569_468_511, 62.792_802_068),
                (70.0, 35.0, 34.554_236_543, 54.849_394_924),
                (55.0, 45.0, 42.132_530_460, 58.453_175_902),
            ],
        },
    },
];

/// Each WCS of [`PROJECTION_GOLDEN`] gives astropy's world coordinates, and its
/// world → pixel inverts them.
#[test]
fn projections_match_astropy() {
    for row in PROJECTION_GOLDEN {
        let code = row.projection.code();
        let header = WcsHeader::celestial(code, [row.crpix; 2], row.crval, [-row.cdelt, row.cdelt])
            .pv(row.projection.test_parameters())
            .image_header();
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert_astropy_golden(&wcs, &row.golden, code);
        let pixels: Vec<[f64; 2]> = row
            .golden
            .points
            .iter()
            .map(|&(x, y, _, _)| [x, y])
            .collect();
        // Each world coordinate is below 256°, whose ulp is 128 ε; in pixels it is that
        // over the scale.
        let tolerance = Tolerance::abs(
            2.0 * 128.0 * f64::EPSILON / row.cdelt,
            "two ulps of the world coordinate",
        );
        wcs_header::assert_round_trip(&wcs, &pixels, tolerance, code);
    }
}

/// A kernel's golden: `native` (φ, θ) projects to `projected` (x, y), which deprojects
/// to `deprojected`.
#[derive(Debug)]
struct NativeGolden {
    projection: Projection,
    pv: &'static [f64],
    native: [f64; 2],
    projected: [f64; 2],
    deprojected: [f64; 2],
}

/// The goldens print 17 digits, and each value is below 256°, whose ulp is 128 ε.
const NATIVE_DEGREES: Tolerance = Tolerance::abs(
    2.0 * 128.0 * f64::EPSILON,
    "two ulps of a coordinate below 256°",
);

/// Native points and their plane points: the TSC, CSC, QSC and HPX rows from wcslib 8.5's
/// `*s2x` and `*x2s`, the others worked by hand as their comments show.
const NATIVE_GOLDEN: &[NativeGolden] = &[
    // Paper II eqs. 61–62, σ = 1 − sin θ: x = cos θ sin φ + 0.2σ, y = −cos θ cos φ − 0.1σ,
    // in degrees.
    NativeGolden {
        projection: Projection::Sin,
        pv: &[0.0, 0.2, -0.1],
        native: [30.0, 60.0],
        projected: [15.859_180_663_294_788, -25.577_418_186_492_753],
        deprojected: [30.0, 60.0],
    },
    NativeGolden {
        projection: Projection::Sin,
        pv: &[0.0, 0.2, -0.1],
        native: [-40.0, 45.0],
        projected: [-22.685_738_719_896_246, -32.713_858_525_468_76],
        deprojected: [-40.0, 45.0],
    },
    // ARC's R = 90° − θ: (30°, 60°) is R = 30, x = 30 sin 30° = 15 and y = −30 cos 30° =
    // −15√3; (−120°, −40°) is R = 130, x = 130 sin(−120°) = −65√3 and y = −130 cos(−120°) = 65.
    NativeGolden {
        projection: Projection::Arc,
        pv: &[],
        native: [30.0, 60.0],
        projected: [15.0, -25.980_762_113_533_157],
        deprojected: [30.0, 60.0],
    },
    NativeGolden {
        projection: Projection::Arc,
        pv: &[],
        native: [-120.0, -40.0],
        projected: [-112.583_302_491_977_02, 65.0],
        deprojected: [-120.0, -40.0],
    },
    // wcslib 8.5 `tscs2x`/`cscs2x`/`qscs2x` and their inverses. CSC's inverse is
    // the published polynomial fit, which misses the point by up to 0.04°.
    NativeGolden {
        projection: Projection::Tsc,
        pv: &[],
        native: [30.0, 20.0],
        projected: [25.980_762_113_533_153, 18.912_448_145_754_276],
        deprojected: [30.0, 20.0],
    },
    NativeGolden {
        projection: Projection::Tsc,
        pv: &[],
        native: [-120.0, -40.0],
        projected: [244.019_237_886_466_87, -43.600_895_814_340_646],
        deprojected: [-120.0, -40.0],
    },
    NativeGolden {
        projection: Projection::Csc,
        pv: &[],
        native: [30.0, 20.0],
        projected: [31.303_854_882_717_133, 23.855_872_750_282_288],
        deprojected: [30.000_112_252_702_29, 19.999_029_202_196_91],
    },
    NativeGolden {
        projection: Projection::Csc,
        pv: &[],
        native: [-120.0, -40.0],
        projected: [240.086_374_282_836_9, -44.205_473_363_399_506],
        deprojected: [-119.998_951_963_376_24, -39.998_865_884_223_214],
    },
    NativeGolden {
        projection: Projection::Qsc,
        pv: &[],
        native: [30.0, 20.0],
        projected: [31.867_419_121_895_73, 24.348_069_968_897_35],
        deprojected: [30.0, 20.0],
    },
    NativeGolden {
        projection: Projection::Qsc,
        pv: &[],
        native: [-120.0, -40.0],
        projected: [241.782_551_767_309_14, -44.232_092_201_227_864],
        deprojected: [-120.0, -40.0],
    },
    // wcslib 8.5 `hpxs2x`: the equatorial zone, both polar caps, the 45° transition of
    // H = 4, K = 3, and the H = 3, K = 4 facets either side of the equator.
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 4.0, 3.0],
        native: [30.0, 30.0],
        projected: [30.0, 33.749_999_999_999_99],
        deprojected: [30.0, 30.0],
    },
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 4.0, 3.0],
        native: [30.0, 60.0],
        projected: [35.490_381_056_766_58, 61.471_143_170_299_726],
        deprojected: [30.0, 60.0],
    },
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 4.0, 3.0],
        native: [-120.0, -60.0],
        projected: [-125.490_381_056_766_58, -61.471_143_170_299_726],
        deprojected: [-120.0, -60.0],
    },
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 4.0, 3.0],
        native: [10.0, 41.810_314_895_778_596],
        projected: [10.0, 45.0],
        deprojected: [10.0, 41.810_314_895_778_596],
    },
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 3.0, 4.0],
        native: [20.0, 70.0],
        projected: [9.823_024_317_517_834, 120.530_927_047_446_5],
        deprojected: [20.0, 70.0],
    },
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 3.0, 4.0],
        native: [20.0, -70.0],
        projected: [40.353_951_364_964_33, -120.530_927_047_446_5],
        deprojected: [20.0, -70.0],
    },
    NativeGolden {
        projection: Projection::Hpx,
        pv: &[0.0, 3.0, 4.0],
        native: [-100.0, -70.0],
        projected: [-79.646_048_635_035_67, -120.530_927_047_446_5],
        deprojected: [-100.0, -70.0],
    },
];

/// Each kernel of [`NATIVE_GOLDEN`] gives its plane point and inverts it to its native one.
#[test]
fn projections_match_their_native_goldens() {
    for row in NATIVE_GOLDEN {
        let kernel = raw(row.projection, row.pv);
        let [phi, theta] = row.native;
        let projected = kernel
            .project(NativeCoordinate::from_degrees(phi, theta))
            .unwrap();
        tolerance::assert_close(
            &[projected.x, projected.y],
            &row.projected,
            NATIVE_DEGREES,
            &format!("{row:?} forward"),
        );
        let native = kernel.deproject(projected.x, projected.y).unwrap();
        let phi = row.deprojected[0];
        tolerance::assert_close(
            &[phi + angle::norm180(native.phi - phi), native.theta()],
            &row.deprojected,
            NATIVE_DEGREES,
            &format!("{row:?} inverse"),
        );
    }
    // The slant terms move the point off the radial SIN.
    let slant = raw(Projection::Sin, &[0.0, 0.2, -0.1])
        .project(NativeCoordinate::from_degrees(30.0, 60.0))
        .unwrap();
    let radial = raw(Projection::Sin, &[])
        .project(NativeCoordinate::from_degrees(30.0, 60.0))
        .unwrap();
    assert_ne!([slant.x, slant.y], [radial.x, radial.y]);
}

/// Native points over the sphere: three northern ones, the native north pole, two
/// southern ones and the south pole, and two next to the equator at the far longitudes.
const SPHERE: &[[f64; 2]] = &[
    [30.0, 70.0],
    [-40.0, 50.0],
    [20.0, 55.0],
    [75.0, 90.0],
    [150.0, -30.0],
    [-120.0, -75.0],
    [0.0, -90.0],
    [-170.0, 5.0],
    [179.0, 0.0],
];

/// A kernel keeps the direction of a native point to a few roundings: the worst,
/// QSC at a face edge, misses it by 8 ε rad.
const NATIVE_ROUND_TRIP: Tolerance = Tolerance::abs(16.0 * f64::EPSILON, "a few roundings");

/// The distance between the directions of `a` and `b` (rad): the chord of their unit
/// vectors, which a pole's arbitrary longitude does not move.
fn chord(a: NativeCoordinate, b: NativeCoordinate) -> f64 {
    let unit = |n: NativeCoordinate| {
        let (sin_phi, cos_phi) = (angle::sind(n.phi), angle::cosd(n.phi));
        [
            n.cos_theta() * cos_phi,
            n.cos_theta() * sin_phi,
            n.sin_theta(),
        ]
    };
    let [a, b] = [unit(a), unit(b)];
    a.iter()
        .zip(&b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Every projection's deprojection inverts its projection over the part of [`SPHERE`]
/// its domain holds.
#[test]
fn projections_round_trip() {
    use Projection::*;
    let without_south_pole = |points: &[[f64; 2]]| -> Vec<[f64; 2]> {
        points
            .iter()
            .copied()
            .filter(|&[_, theta]| theta != -90.0)
            .collect()
    };
    let without_poles: Vec<[f64; 2]> = SPHERE
        .iter()
        .copied()
        .filter(|&[_, theta]| theta.abs() != 90.0)
        .collect();
    let above = |limit: f64| -> Vec<[f64; 2]> {
        SPHERE
            .iter()
            .copied()
            .filter(|&[_, theta]| theta > limit)
            .collect()
    };
    for projection in Projection::all() {
        let points: Vec<[f64; 2]> = match projection {
            Tan => above(0.0),
            Sin => above(-1.0),
            Stg | Coo | Air => without_south_pole(SPHERE),
            Cop | Azp | Szp => above(-40.0),
            // Mercator's poles lie at infinity.
            Mer => without_poles.clone(),
            _ => SPHERE.to_vec(),
        };
        let tolerance = match projection {
            Csc => Tolerance::abs(0.04 * D2R, "CSC's inverse is the published polynomial fit"),
            _ => NATIVE_ROUND_TRIP,
        };
        let kernel = raw(projection, projection.test_parameters());
        for [phi, theta] in points {
            let native = NativeCoordinate::from_degrees(phi, theta);
            let projected = kernel.project(native).unwrap();
            let back = kernel.deproject(projected.x, projected.y).unwrap();
            tolerance::assert_within(
                chord(native, back),
                0.0,
                tolerance,
                &format!("{projection:?} ({phi}, {theta}) → {projected:?} → {back:?}"),
            );
        }
    }
}

#[test]
fn projection_parameters_use_standard_defaults() {
    let build = |projection: &str, parameters: &[f64]| {
        let header = WcsHeader::celestial(projection, [1.0, 1.0], [0.0, 0.0], [1.0, 1.0])
            .pv(parameters)
            .image_header();
        Wcs::from_header(&header, None).unwrap()
    };

    // Each default, and an explicit 0 that keeps its place against CYP's default 1.
    for (projection, parameters, expected) in [
        ("AIR", &[][..], raw(Projection::Air, &[0.0, 90.0])),
        ("CYP", &[], raw(Projection::Cyp, &[0.0, 1.0, 1.0])),
        ("CEA", &[], raw(Projection::Cea, &[0.0, 1.0])),
        (
            "SZP",
            &[0.0, 2.0],
            raw(Projection::Szp, &[0.0, 2.0, 0.0, 90.0]),
        ),
        ("HPX", &[], raw(Projection::Hpx, &[0.0, 4.0, 3.0])),
        (
            "CYP",
            &[0.0, 0.0, 1.0],
            raw(Projection::Cyp, &[0.0, 0.0, 1.0]),
        ),
    ] {
        assert_eq!(
            build(projection, parameters).celestial.unwrap().parameters,
            expected,
            "{projection} {parameters:?}"
        );
    }
}

#[test]
fn degenerate_projection_parameters_are_refused() {
    let parse = |projection: &str, parameters: &[f64]| {
        let header = WcsHeader::celestial(projection, [1.0, 1.0], [0.0, 0.0], [1.0, 1.0])
            .pv(parameters)
            .image_header();
        Wcs::from_header(&header, None)
    };

    assert_matches!(
        parse("CEA", &[0.0, 0.0]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection {
                projection: Projection::Cea
            }
        })
    );
    assert_matches!(
        parse("CYP", &[0.0, 1.0, 0.0]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection {
                projection: Projection::Cyp
            }
        })
    );
    assert_matches!(
        parse("CYP", &[0.0, -1.0, 1.0]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection {
                projection: Projection::Cyp
            }
        })
    );
    for theta_b in [-90.0, -100.0] {
        assert_matches!(
            parse("AIR", &[0.0, theta_b]),
            Err(FitsError::InvalidWcs {
                defect: WcsDefect::DegenerateProjection {
                    projection: Projection::Air
                }
            })
        );
    }
    for parameters in [
        [0.0, 0.0, 3.0],
        [0.0, -1.0, 3.0],
        [0.0, 4.0, 0.0],
        [0.0, 4.0, -1.0],
    ] {
        assert_matches!(
            parse("HPX", &parameters),
            Err(FitsError::InvalidWcs {
                defect: WcsDefect::DegenerateProjection {
                    projection: Projection::Hpx
                }
            })
        );
    }
}

#[test]
fn unsupported_projection_codes_refuse_complete_transforms() {
    // Short codes represent space-padded algorithm names after FITS text trimming.
    for code in ["XPH", "UV", "U"] {
        let h = WcsHeader::celestial(code, [1.0, 1.0], [10.0, 20.0], [2.0, 3.0]).image_header();
        let w = Wcs::from_header(&h, None).unwrap();
        assert_eq!(
            w.description().unsupported_axes,
            [0, 1],
            "{code} axes flagged"
        );
        assert!(w.celestial.is_none(), "{code} not decoded as a projection");
        assert_matches!(
            to_world(&w, &[3.0, 4.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        );
        assert_matches!(
            to_pixel(&w, &[10.0, 20.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        );
    }
}

#[test]
fn mismatched_celestial_projections_are_refused() {
    // §8.2: the longitude and latitude axes must share one projection. Pairing
    // RA---TAN with DEC--SIN is malformed — reject it rather than silently adopt
    // whichever axis is seen first.
    let mut h = Header::new();
    h.set_internal(keywords::NAXIS, 2);
    h.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--SIN");
    h.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
    h.set_internal("CRVAL1", 10.0).set_internal("CRVAL2", 20.0);
    h.set_internal("CDELT1", 1.0).set_internal("CDELT2", 1.0);
    assert_matches!(
        Wcs::from_header(&h, None),
        Err(FitsError::ConflictingWcsKeywords {
            conflict: WcsConflict::MismatchedProjections
        })
    );

    // A galactic-frame pair sharing TAN builds fine — exercises the one shared
    // classifier on the non-RA/DEC longitude/latitude forms (`GLON`/`GLAT`).
    let mut g = Header::new();
    g.set_internal(keywords::NAXIS, 2);
    g.set_internal("CTYPE1", "GLON-TAN")
        .set_internal("CTYPE2", "GLAT-TAN");
    g.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
    g.set_internal("CRVAL1", 30.0).set_internal("CRVAL2", 10.0);
    g.set_internal("CDELT1", -1.0).set_internal("CDELT2", 1.0);
    let w = Wcs::from_header(&g, None).unwrap();
    assert!(
        w.celestial.is_some(),
        "GLON/GLAT TAN pair is a celestial WCS"
    );
}

/// A conic's mid-latitude θ_a = `PVi_1` is mandatory and nonzero: absent (→ 0), the cone
/// of `1/tan 0` has no apex, and the description is refused as every other bad parameter
/// is. COE at η = 90° has the constant sinθ_a·cosη = 0 and is refused too; CEA takes
/// 0 < λ ≤ 1.
#[test]
fn degenerate_cone_and_cea_parameters_are_invalid() {
    let degenerate = |code: &str, pv: &[f64]| {
        let header = WcsHeader::celestial(code, [1.0, 1.0], [10.0, 20.0], [2.0, 3.0])
            .pv(pv)
            .image_header();
        match Wcs::from_header(&header, None) {
            Err(FitsError::InvalidWcs {
                defect: WcsDefect::DegenerateProjection { projection },
            }) => Some(projection),
            Err(error) => panic!("{code} {pv:?}: {error}"),
            Ok(_) => None,
        }
    };
    for (code, projection) in [
        ("COP", Projection::Cop),
        ("COE", Projection::Coe),
        ("COD", Projection::Cod),
        ("COO", Projection::Coo),
    ] {
        assert_eq!(degenerate(code, &[]), Some(projection), "{code}");
        assert_eq!(degenerate(code, &[0.0, 45.0]), None, "{code}");
    }
    assert_eq!(degenerate("COE", &[0.0, 45.0, 90.0]), Some(Projection::Coe));
    assert_eq!(degenerate("COE", &[0.0, 45.0, 89.0]), None);
    assert_eq!(degenerate("CEA", &[0.0, 1.5]), Some(Projection::Cea));
    assert_eq!(degenerate("CEA", &[0.0, -0.5]), Some(Projection::Cea));
    assert_eq!(degenerate("CEA", &[0.0, 1.0]), None);
}

/// Parameters as given, unvalidated: the kernels' own tests reach past the header path.
/// The kernel of `projection` with `parameters` as its first `PVi_m`, the rest 0,
/// taken as given.
fn raw(projection: Projection, parameters: &[f64]) -> ProjectionParameters {
    projection.raw_parameters(pv(parameters))
}

/// The 21 `PVi_m` of `values`, the rest 0.
fn pv(values: &[f64]) -> [f64; 21] {
    let mut pv = [0.0; 21];
    pv[..values.len()].copy_from_slice(values);
    pv
}

/// Each projection's world domain, at points either side of its boundary — wcslib's
/// `*s2x` bounds, except SZP's limb, which is the exact tangency (see `Projection::project`).
#[test]
fn world_points_without_an_image_are_refused() {
    use Projection::{Air, Azp, Coo, Cop, Mer, Sin, Stg, Szp, Tan, Zpn};
    // ZPN 0 + ζ − ζ³/2: R′ = 1 − 1.5ζ² vanishes at ζ = √(2/3) = 46.7647°, θ = 43.2353°.
    let cases: &[(Projection, &[f64], f64, f64, bool)] = &[
        (Tan, &[], 0.0, 10.0, true),
        (Tan, &[], 0.0, 0.0, false),
        (Tan, &[], 0.0, -10.0, false),
        (Stg, &[], 0.0, -89.0, true),
        (Stg, &[], 0.0, -90.0, false),
        (Sin, &[], 30.0, 0.0, true),
        (Sin, &[], 30.0, -1.0, false),
        // Slant SIN: the horizon is θ = −atan(ξ sinφ − η cosφ) = −atan(0.2) = −11.3099° at φ = 90°.
        (Sin, &[0.0, 0.2, -0.1], 90.0, -11.3, true),
        (Sin, &[0.0, 0.2, -0.1], 90.0, -11.4, false),
        // AZP μ = 2: the limb of the tangent cone, sinθ = −1/μ, θ = −30°.
        (Azp, &[0.0, 2.0], 0.0, -29.9, true),
        (Azp, &[0.0, 2.0], 0.0, -30.1, false),
        // AZP μ = 0.5: rays from inside the sphere diverge past θ = −asin(μ) = −30°.
        (Azp, &[0.0, 0.5], 0.0, -29.9, true),
        (Azp, &[0.0, 0.5], 0.0, -30.1, false),
        // SZP with θc = 90° is AZP: the same −30° limb for μ = 2 …
        (Szp, &[0.0, 2.0, 0.0, 90.0], 0.0, -29.9, true),
        (Szp, &[0.0, 2.0, 0.0, 90.0], 0.0, -30.1, false),
        // … and for μ = 0.5 the divergence at sinθ = 1 − z_p = −0.5.
        (Szp, &[0.0, 0.5, 0.0, 90.0], 0.0, -29.9, true),
        (Szp, &[0.0, 0.5, 0.0, 90.0], 0.0, -30.1, false),
        (Zpn, &[0.0, 1.0, 0.0, -0.5], 0.0, 43.3, true),
        (Zpn, &[0.0, 1.0, 0.0, -0.5], 0.0, 43.1, false),
        (Air, &[], 0.0, -89.9, true),
        (Air, &[], 0.0, -90.0, false),
        (Mer, &[], 0.0, 89.9, true),
        (Mer, &[], 0.0, 90.0, false),
        (Mer, &[], 0.0, -90.0, false),
        // COP θa = 45°: θ − θa = −90° diverges, the far pole is off the cone, and below
        // θ = −45° the radius changes sign.
        (Cop, &[0.0, 45.0], 0.0, 90.0, true),
        (Cop, &[0.0, 45.0], 0.0, -30.0, true),
        (Cop, &[0.0, 45.0], 0.0, -45.0, false),
        (Cop, &[0.0, 45.0], 0.0, -60.0, false),
        (Cop, &[0.0, 45.0], 0.0, -90.0, false),
        (Coo, &[0.0, 45.0], 0.0, -89.0, true),
        (Coo, &[0.0, 45.0], 0.0, -90.0, false),
    ];
    for &(projection, parameters, phi, theta, imaged) in cases {
        let mut pv = projection.parameter_defaults();
        pv[..parameters.len()].copy_from_slice(parameters);
        let parameters = projection.parameters(pv).unwrap();
        let result = parameters.project(NativeCoordinate::from_degrees(phi, theta));
        if imaged {
            // An imaged point is one the inverse takes back: a finite image is not enough.
            let native = NativeCoordinate::from_degrees(phi, theta);
            let projected = result.unwrap();
            let back = parameters.deproject(projected.x, projected.y).unwrap();
            tolerance::assert_within(
                chord(native, back),
                0.0,
                Tolerance::abs(
                    1e-9,
                    "next to a limb the inverse's two roots meet, and its digits go as the \
                     square root of the distance: 0.01° from SIN's horizon keeps 13",
                ),
                &format!("{projection:?} {pv:?} ({phi}, {theta})"),
            );
        } else {
            assert_matches!(result, Err(FitsError::WcsWorldOutOfDomain { projection: code }) if code == projection,
                "{projection:?} {pv:?} ({phi}, {theta}): {result:?}"
            );
        }
    }
}

/// Each projection's plane domain, at points either side of its boundary, the inverse
/// of the table above. A native longitude past ±180° is off the map, as wcslib's `prjbchk`
/// takes it: CAR's edge is `x = ±180`. At `x = 0` the AIT ellipse ends at `y = √2·(180/π)
/// ≈ 81.03°`, and on the MOL equator at `x = (2√2/π)·180 ≈ 162.03`. ZPN `ζ − ζ³/2` peaks at
/// `R = √(2/3)·(2/3) rad ≈ 31.19°`. SFL and BON meet at each pole on `x = 0` alone, HPX's
/// north pole is at `x = 45`, and the cube faces leave `(100, 100)` empty.
#[test]
fn plane_points_without_a_world_point_are_refused() {
    use Projection::{Ait, Bon, Car, Csc, Hpx, Mol, Qsc, Sfl, Tsc, Zpn};
    let zpn_peak = (2.0f64 / 3.0).sqrt() * 2.0 / 3.0 * R2D;
    let ait_edge = SQRT_2 * R2D;
    let cases: &[(Projection, &[f64], f64, f64, bool)] = &[
        (Car, &[], 180.0, 10.0, true),
        (Car, &[], 180.0 + 1e-9, 10.0, false),
        (Car, &[], -200.0, 0.0, false),
        (Ait, &[], 0.0, 80.0, true),
        // Past the edge by more than rounding: z² = 1/2 − 1e-9.
        (Ait, &[], 0.0, ait_edge * (1.0f64 + 2e-9).sqrt(), false),
        (Ait, &[], 0.0, 85.0, false),
        (Mol, &[], 160.0, 0.0, true),
        (Mol, &[], 170.0, 0.0, false),
        (Zpn, &[0.0, 1.0, 0.0, -0.5], 0.0, -(zpn_peak - 0.01), true),
        (Zpn, &[0.0, 1.0, 0.0, -0.5], 0.0, -(zpn_peak + 0.01), false),
        (Sfl, &[], 0.0, 90.0, true),
        (Sfl, &[], 1.0, 90.0, false),
        (Sfl, &[], 1.0, -90.0, false),
        (Bon, &[45.0], 0.0, 90.0, true),
        (Bon, &[45.0], 1.0, 90.0, false),
        (Hpx, &[0.0, 4.0, 3.0], 45.0, 90.0, true),
        (Hpx, &[0.0, 4.0, 3.0], 0.0, 90.0, false),
        (Tsc, &[], 10.0, 10.0, true),
        (Tsc, &[], 100.0, 100.0, false),
        (Csc, &[], 10.0, 10.0, true),
        (Csc, &[], 100.0, 100.0, false),
        (Qsc, &[], 10.0, 10.0, true),
        (Qsc, &[], 100.0, 100.0, false),
    ];
    for &(projection, parameters, x, y, imaged) in cases {
        let kernel = raw(projection, parameters);
        let result = kernel.deproject(x, y);
        if imaged {
            // The domain alone: each kernel's own tests hold its accuracy, and CSC's
            // inverse is the published approximation, whose round trip misses (10, 10) by
            // 2.8e-3°.
            result.unwrap_or_else(|error| panic!("{projection:?} ({x}, {y}): {error}"));
        } else {
            assert_matches!(
                result,
                Err(FitsError::WcsPixelOutOfDomain { projection: code }) if code == projection,
                "{projection:?} ({x}, {y}): {result:?}"
            );
        }
    }
}

/// Near their limits the conic, Airy and polyconic formulas once cancelled to
/// nothing: at η = 1e-7° a COO cone constant lost half its digits, at ξ = 1e-8 rad
/// cos ξ rounded to 1 and the Airy constant came out 0 rather than −1/2, and near the
/// equator 1 − cos ω rounded away the polyconic's y offset.
#[test]
fn near_limit_projection_formulas_keep_their_precision() {
    use std::f64::consts::FRAC_PI_2;

    // COO: C = sin θ_a + O(η²), and η² is far below an ulp of sin 45°.
    let pv = pv(&[0.0, 45.0, 1e-7]);
    let coo = Conic::new(ConicKind::Coo, &pv);
    tolerance::assert_within(
        coo.c,
        45f64.to_radians().sin(),
        Tolerance::abs(
            2.0 * f64::EPSILON,
            "the η → 0 limit's two roundings, below 1",
        ),
        "COO C at η = 1e-7",
    );

    // AIR: K = −1/2 + ξ²/4 + …, so −1/2 to the last bit at ξ = 1e-8.
    assert_eq!(Air::new(90.0 - 2.0 * 1e-8 * R2D).k, -0.5);
    // The inverse at a tiny colatitude, which a fixed 1e-6 start and an absolute
    // residual stop once returned with a relative error of 1e-3, and a residual stop of
    // 1e-12 with 6.5e-14 at ζ = 1e-6.
    let air = Air::new(45.0);
    for zeta in [1e-9, 1e-6, 1e-3, 1.0, 2.5, 3.0] {
        let back = air.zeta(air.radius(zeta)).unwrap();
        tolerance::assert_within(
            back,
            zeta,
            Tolerance::ulps(2.0, "two ulps of the root and its radius"),
            &format!("AIR ζ = {zeta}"),
        );
    }

    // PCO: y = θ + (180/π)·cot t·2 sin²(ω/2) with ω = φ·sin t; for t = 1e-9 rad and
    // φ = 90°, y = θ + (180/π)·(π/2)²·t/2 to within O(t³).
    let t = 1e-9;
    let projected = Projection::Pco
        .raw_parameters([0.0; 21])
        .project(NativeCoordinate::from_degrees(90.0, t * R2D))
        .unwrap();
    let expected = t * R2D + R2D * FRAC_PI_2.powi(2) * t / 2.0;
    tolerance::assert_within(
        projected.y,
        expected,
        Tolerance::rel(
            1e-9,
            "the series' next term, of order t² = 1e-18 relative, and roundings",
        ),
        "PCO near the equator",
    );
}

/// Each projection prints as its `CTYPEi` code, which names it back.
#[test]
fn projection_prints_its_code() {
    assert_eq!(Projection::Tan.to_string(), "TAN");
    for projection in Projection::all() {
        let code = projection.to_string();
        assert_eq!(code, projection.code());
        assert_eq!(Projection::from_code(&code), Some(projection));
    }
}
