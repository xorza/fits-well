use crate::error::FitsError;
use crate::error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::internals::tolerance::{Tolerance, assert_close};
use crate::internals::wcs_header::{WcsHeader, assert_round_trip};
use crate::world_coordinates::angle;
use crate::world_coordinates::internals::{to_pixel, to_world};

use crate::world_coordinates::Wcs;
use crate::world_coordinates::angle::{D2R, R2D};
use crate::world_coordinates::internals::AstropyGolden;
use crate::world_coordinates::internals::CEA_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;

use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::conic::{Conic, ConicKind};
use crate::world_coordinates::projection::zenithal::{Air, evaluate_polynomial};
use crate::world_coordinates::projection::{Projection, ProjectionParameters};
use std::f64::consts::SQRT_2;

#[test]
fn zpn_extended_horner_evaluates_value_and_derivative() {
    let mut pv = [0.0; 21];
    pv[..4].copy_from_slice(&[2.0, -3.0, 4.0, 5.0]);
    let evaluation = evaluate_polynomial(2.0, &pv);
    // P(2) = 2 - 3·2 + 4·2² + 5·2³ = 52; P'(2) = -3 + 8·2 + 15·2² = 73.
    assert_eq!(evaluation.value, 52.0);
    assert_eq!(evaluation.derivative, 73.0);
}

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
        assert_round_trip(&wcs, &pixels, tolerance, code);
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
        assert_close(
            &[projected.x, projected.y],
            &row.projected,
            NATIVE_DEGREES,
            &format!("{row:?} forward"),
        );
        let native = kernel.deproject(projected.x, projected.y).unwrap();
        let phi = row.deprojected[0];
        assert_close(
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
            assert_close(
                &[chord(native, back)],
                &[0.0],
                tolerance,
                &format!("{projection:?} ({phi}, {theta}) → {projected:?} → {back:?}"),
            );
        }
    }
}

/// At x = 1e-9° from the centre σ ≈ 1.5e-22: the root (−b − √d)/2a cancels to 0 there,
/// while c/q keeps it, so cos θ = √(σ(2 − σ)) is the radius x/(180/π) itself.
#[test]
fn sin_keeps_its_digits_next_to_the_centre() {
    let x = 1e-9;
    let native = raw(Projection::Sin, &[]).deproject(x, 0.0).unwrap();
    assert!(
        (native.cos_theta() - x / R2D).abs() <= 2.0 * f64::EPSILON * x / R2D,
        "{}",
        native.cos_theta()
    );
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
fn degenerate_projection_parameters_are_rejected() {
    let parse = |projection: &str, parameters: &[f64]| {
        let header = WcsHeader::celestial(projection, [1.0, 1.0], [0.0, 0.0], [1.0, 1.0])
            .pv(parameters)
            .image_header();
        Wcs::from_header(&header, None)
    };

    assert!(matches!(
        parse("CEA", &[0.0, 0.0]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: "CEA" }
        })
    ));
    assert!(matches!(
        parse("CYP", &[0.0, 1.0, 0.0]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: "CYP" }
        })
    ));
    assert!(matches!(
        parse("CYP", &[0.0, -1.0, 1.0]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: "CYP" }
        })
    ));
    for theta_b in [-90.0, -100.0] {
        assert!(matches!(
            parse("AIR", &[0.0, theta_b]),
            Err(FitsError::InvalidWcs {
                defect: WcsDefect::DegenerateProjection { projection: "AIR" }
            })
        ));
    }
    for parameters in [
        [0.0, 0.0, 3.0],
        [0.0, -1.0, 3.0],
        [0.0, 4.0, 0.0],
        [0.0, 4.0, -1.0],
    ] {
        assert!(matches!(
            parse("HPX", &parameters),
            Err(FitsError::InvalidWcs {
                defect: WcsDefect::DegenerateProjection { projection: "HPX" }
            })
        ));
    }
}

#[test]
fn unsupported_projection_codes_reject_complete_transforms() {
    // Short codes represent space-padded algorithm names after FITS text trimming.
    for code in ["XPH", "UV", "U"] {
        let h = WcsHeader::celestial(code, [1.0, 1.0], [10.0, 20.0], [2.0, 3.0]).image_header();
        let w = Wcs::from_header(&h, None).unwrap();
        assert_eq!(w.metadata().unsupported_axes, [0, 1], "{code} axes flagged");
        assert!(w.celestial.is_none(), "{code} not decoded as a projection");
        assert!(matches!(
            to_world(&w, &[3.0, 4.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        ));
        assert!(matches!(
            to_pixel(&w, &[10.0, 20.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        ));
    }
}

#[test]
fn mismatched_celestial_projections_are_rejected() {
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
    assert!(matches!(
        Wcs::from_header(&h, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));

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

#[test]
fn degenerate_conic_without_pv1_rejects_complete_transforms() {
    // A conic's mid-latitude θ_a = PVi_1 is mandatory and must be non-zero; absent
    // (→ 0) the cone is degenerate (1/tan 0 → NaN). Rather than return NaN, the WCS
    // flags the celestial axes, so complete transforms fail rather than returning
    // NaN or silently relabeling linear-stage coordinates as sky coordinates.
    for code in ["COP", "COE", "COD", "COO"] {
        let h = WcsHeader::celestial(code, [1.0, 1.0], [10.0, 20.0], [2.0, 3.0]).image_header();
        // No PV2_1 ⇒ θ_a = 0.
        let w = Wcs::from_header(&h, None).unwrap();
        assert_eq!(w.metadata().unsupported_axes, [0, 1], "{code} axes flagged");
        assert!(w.celestial.is_none(), "{code} degenerate, not deprojected");
        assert!(matches!(
            to_world(&w, &[3.0, 4.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        ));
    }
    // A conic *with* a valid θ_a is still decoded normally (not flagged).
    let mut ok = WcsHeader::celestial("COP", [1.0, 1.0], [10.0, 45.0], [0.5, 0.5]).image_header();
    ok.set_internal("PV2_1", 45.0);
    let w = Wcs::from_header(&ok, None).unwrap();
    assert!(w.metadata().unsupported_axes.is_empty() && w.celestial.is_some());
}

#[test]
fn bonne_with_zero_theta1_equals_sfl() {
    // §5.5.1: Bonne's projection at θ₁ = 0 is exactly the sinusoidal SFL. A BON
    // header with PV2_1 = 0 must decode identically to an SFL header (and never hit
    // the 1/tan 0 singularity), so it is *decoded* — not flagged unsupported.
    let build = |proj: &str, pv1: Option<f64>| {
        let mut h =
            WcsHeader::celestial(proj, [50.0, 50.0], [45.0, 0.0], [-0.5, 0.5]).image_header();
        if let Some(v) = pv1 {
            h.set_internal("PV2_1", v);
        }
        Wcs::from_header(&h, None).unwrap()
    };
    let bon = build("BON", Some(0.0));
    let sfl = build("SFL", None);
    assert!(bon.celestial.is_some() && bon.metadata().unsupported_axes.is_empty());
    for pixel in [[20.0, 70.0], [80.0, 30.0], [55.0, 45.0]] {
        assert_eq!(
            to_world(&bon, &pixel).unwrap(),
            to_world(&sfl, &pixel).unwrap(),
            "{pixel:?}"
        );
    }
    // The forward (SFL) path round-trips too. Each world coordinate is below 256°, whose
    // ulp is 128 ε, and a pixel is 0.5°.
    assert_round_trip(
        &bon,
        &[[40.0, 60.0]],
        Tolerance::abs(
            2.0 * 128.0 * f64::EPSILON / 0.5,
            "two ulps of the world coordinate",
        ),
        "BON θ₁ = 0",
    );
}

/// Next to the native pole the colatitude crosses each projection as an `f64` of its
/// own, so a round trip keeps ζ to an ulp each way, where θ in degrees rounds at
/// 1.4e-14°, 2.5e-16 rad: once all of ζ = 1e-15. The sweep holds each formula that
/// carries ζ: the zenithals from ζ itself, and SIN and SZP from their depth
/// σ = 1 − sin θ.
#[test]
fn next_to_the_pole_the_colatitude_keeps_its_digits() {
    use Projection::*;
    let cases: &[(Projection, &[f64])] = &[
        (Tan, &[]),
        (Sin, &[]),
        (Arc, &[]),
        (Stg, &[]),
        (Zea, &[]),
        (Zpn, &[0.0, 1.0, 0.0, 0.1]),
        (Szp, &[]),
    ];
    for &(projection, pv) in cases {
        let pv = raw(projection, pv);
        for zeta in [1e-15, 1e-9, 1e-6, 1e-3] {
            let native = NativeCoordinate::from_colatitude(30.0, zeta);
            let projected = pv.project(native).unwrap();
            let back = pv.deproject(projected.x, projected.y).unwrap();
            assert!(
                (back.zeta() - zeta).abs() <= 2.0 * f64::EPSILON * zeta,
                "{projection:?} ζ = {zeta:e}: {:e}",
                back.zeta()
            );
        }
    }
}

/// MER's y = ln tan(45° + θ/2) and its inverse 2 atan(eʸ) − 90° cancel near the
/// equator, where y ≈ θ; asinh(tan θ) and atan(sinh y) keep θ to its last bits: at
/// θ = 1e-10°, y = θ (1 + O(θ²)).
#[test]
fn mercator_keeps_its_digits_near_the_equator() {
    let pv = raw(Projection::Mer, &[]);
    for theta in [1e-10, -3e-7, 1e-3] {
        let projected = pv
            .project(NativeCoordinate::from_degrees(10.0, theta))
            .unwrap();
        assert!(
            ((projected.y - theta) / theta).abs() <= theta * theta * 1e-3 + 2.0 * f64::EPSILON,
            "{theta}: {}",
            projected.y
        );
        let back = pv.deproject(10.0, projected.y).unwrap();
        assert!(
            ((back.theta() - theta) / theta).abs() <= 4.0 * f64::EPSILON,
            "{theta}: {}",
            back.theta()
        );
    }
}

/// SFL's meridians meet at the poles, where `x / cos y` takes `cos 90° = 0` exactly: the
/// pole itself is φ = 0, and any other point on that line lies off the map.
#[test]
fn sfl_poles_have_one_longitude() {
    for projection in [Projection::Sfl, Projection::Bon] {
        let pv = raw(projection, &[]);
        for y in [90.0, -90.0] {
            assert_eq!(
                pv.deproject(0.0, y).unwrap(),
                NativeCoordinate::from_degrees(0.0, y),
                "{projection:?}"
            );
            assert!(matches!(
                pv.deproject(1.0, y),
                Err(FitsError::WcsPixelOutOfDomain { .. })
            ));
        }
    }
}

#[test]
fn cube_projections_match_wcslib_faces() {
    let faces = [
        (0.0, 0.0, 0.0, 0.0),
        (90.0, 0.0, 90.0, 0.0),
        (180.0, 0.0, 180.0, 0.0),
        (-90.0, 0.0, 270.0, 0.0),
        (0.0, 90.0, 0.0, 90.0),
        (0.0, -90.0, 0.0, -90.0),
        (45.0, 0.0, 45.0, 0.0),
        (0.0, 45.0, 0.0, 45.0),
        (45.0, 35.264_389_682_754_654, 45.0, 45.0),
    ];
    for projection in [Projection::Tsc, Projection::Csc, Projection::Qsc] {
        for (phi, theta, x, y) in faces {
            let projected = raw(projection, &[])
                .project(NativeCoordinate::from_degrees(phi, theta))
                .unwrap();
            assert!(
                (projected.x - x).abs() < 1e-12 && (projected.y - y).abs() < 1e-12,
                "{projection:?} face ({phi},{theta}): {projected:?}"
            );
        }
    }
}

/// A deprojected native longitude past ±180° is off the map, as wcslib's `prjbchk` takes
/// it. CAR's edge is `x = ±180`. At `x = 0` the AIT ellipse ends at `y = √2·(180/π) ≈
/// 81.03°`; at `y = 85` its `Z² = 1 − (85°)²/4 ≈ 0.4498` is below ½, so `φ = 2·atan2(0,
/// 2Z² − 1) = 360°`. On the MOL equator the ellipse ends at `x = (2√2/π)·180 ≈ 162.03`,
/// and `x = 170` gives `φ = π·170/(2√2) ≈ 188.8°`.
#[test]
fn deprojections_past_a_half_turn_of_longitude_are_refused() {
    let edge = raw(Projection::Car, &[]).deproject(180.0, 10.0).unwrap();
    assert_eq!(edge, NativeCoordinate::from_degrees(180.0, 10.0));
    for (projection, x, y) in [
        (Projection::Car, 180.0 + 1e-9, 10.0),
        (Projection::Car, -200.0, 0.0),
        (Projection::Ait, 0.0, 85.0),
        (Projection::Mol, 170.0, 0.0),
    ] {
        assert!(
            matches!(
                raw(projection, &[]).deproject(x, y),
                Err(FitsError::WcsPixelOutOfDomain { projection: code })
                    if code == projection.code()
            ),
            "{projection:?} ({x}, {y})"
        );
    }
}

#[test]
fn cube_projection_domains_reject_points_outside_the_face_cross() {
    for projection in [Projection::Tsc, Projection::Csc, Projection::Qsc] {
        let pv = raw(projection, &[]);
        assert!(matches!(
            pv.deproject(100.0, 100.0),
            Err(FitsError::WcsPixelOutOfDomain { projection: code })
                if code == projection.code()
        ));
        // (315, 45) is the top-right corner of face 4, which is the cube vertex
        // (1, −1, 1)/√3: φ = −45°, θ = atan(1/√2). The tolerance allows the few
        // roundings of the face equations, which differ by projection.
        let corner = pv.deproject(315.0, 45.0).unwrap();
        let vertex_theta = (1.0 / SQRT_2).atan() * R2D;
        assert!(
            (corner.phi + 45.0).abs() < 1e-12 && (corner.theta() - vertex_theta).abs() < 1e-12,
            "{projection:?}: {corner:?}"
        );
    }
}

#[test]
fn hpx_poles_and_parameters() {
    let default_pv = raw(Projection::Hpx, &[0.0, 4.0, 3.0]);
    let north = default_pv
        .project(NativeCoordinate::from_degrees(0.0, 90.0))
        .unwrap();
    let south = default_pv
        .project(NativeCoordinate::from_degrees(100.0, -90.0))
        .unwrap();
    assert_eq!([north.x, north.y], [45.0, 90.0]);
    assert_eq!([south.x, south.y], [135.0, -90.0]);
    assert_eq!(default_pv.deproject(45.0, 90.0).unwrap().theta(), 90.0);
    assert!(matches!(
        default_pv.deproject(0.0, 90.0),
        Err(FitsError::WcsPixelOutOfDomain { projection: "HPX" })
    ));

    // In the polar cap 1 − sin θ = σ²/K. At y = 90° − 45°·2⁻³⁰, exact in f64, σ = 2⁻³⁰,
    // and sin θ = 1 − 2⁻⁶⁰/3 rounds to 1, which once gave θ = 90°; cos θ =
    // √(σ²/K·(2 − σ²/K)) = 2⁻³⁰·√(2/3) to 1e-19 keeps the point.
    let sigma = 2f64.powi(-30);
    let cap = default_pv.deproject(45.0, 90.0 - 45.0 * sigma).unwrap();
    let expected = sigma * (2.0f64 / 3.0).sqrt();
    assert!(
        (cap.cos_theta() - expected).abs() <= 2.0 * f64::EPSILON * expected,
        "{cap:?}"
    );

    let standard = default_pv
        .project(NativeCoordinate::from_degrees(20.0, -70.0))
        .unwrap();
    let alternate = raw(Projection::Hpx, &[0.0, 3.0, 4.0])
        .project(NativeCoordinate::from_degrees(20.0, -70.0))
        .unwrap();
    assert_ne!([standard.x, standard.y], [alternate.x, alternate.y]);
}

/// Next to a MOL pole the plane's y changes with θ as `c^(2/3)` of the colatitude c, so
/// one ulp of y moves θ by far more than one ulp; the round trip keeps to that change, where
/// π sin θ and asin(y/(√2·180/π)) lost 2.3e6 ε at 1e-5° from the pole.
#[test]
fn mollweide_keeps_its_digits_next_to_the_poles() {
    let kernel = raw(Projection::Mol, &[]);
    for colatitude in [10.0, 1e-1, 1e-3, 1e-5, 1e-9] {
        for theta in [90.0 - colatitude, colatitude - 90.0] {
            let native = NativeCoordinate::from_degrees(73.0, theta);
            let projected = kernel.project(native).unwrap();
            let back = kernel.deproject(projected.x, projected.y).unwrap();
            let next = projected.y.abs().next_down().copysign(projected.y);
            let ulp_of_y = chord(back, kernel.deproject(projected.x, next).unwrap());
            assert_close(
                &[chord(native, back)],
                &[0.0],
                Tolerance::abs(
                    16.0 * f64::EPSILON + 2.0 * ulp_of_y,
                    "a few roundings, and two ulps of y",
                ),
                &format!("θ = {theta}"),
            );
        }
    }
}

/// MOL and AIT take each native pole to (0, ±√2·180/π), on the edge of the map, where
/// every meridian meets, and back to φ = 0.
#[test]
fn equatorial_poles_are_finite_and_have_canonical_longitude() {
    for projection in [Projection::Mol, Projection::Ait] {
        let pv = raw(projection, &[]);
        for theta in [-90.0, 90.0] {
            let projected = pv
                .project(NativeCoordinate::from_degrees(73.0, theta))
                .unwrap();
            assert!(
                projected.x.abs() < 1e-12,
                "{projection:?} projected pole x = {}",
                projected.x
            );
            assert!((projected.y - theta.signum() * SQRT_2 * R2D).abs() < 1e-12);

            let native = pv.deproject(projected.x, projected.y).unwrap();
            assert_eq!(native.phi, 0.0, "{projection:?}");
            assert!((native.theta() - theta).abs() < 1e-12);
        }
    }
    // Past the edge of AIT by more than rounding: z² = 1/2 − 1e-9.
    let y = (2.0f64 * (1.0 + 2e-9)).sqrt() * R2D;
    assert!(matches!(
        raw(Projection::Ait, &[]).deproject(0.0, y),
        Err(FitsError::WcsPixelOutOfDomain { projection: "AIT" })
    ));
}

/// Parameters as given, unvalidated: the kernels' own tests reach past the header path.
/// The kernel of `projection` with `parameters` as its first `PVi_m`, the rest 0,
/// taken as given.
fn raw(projection: Projection, parameters: &[f64]) -> ProjectionParameters {
    let mut pv = [0.0; 21];
    pv[..parameters.len()].copy_from_slice(parameters);
    projection.raw_parameters(pv)
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
            let projected = result.unwrap();
            assert!(
                projected.x.is_finite() && projected.y.is_finite(),
                "{projection:?} {pv:?} ({phi}, {theta}): {projected:?}"
            );
        } else {
            assert!(
                matches!(result, Err(FitsError::WcsWorldOutOfDomain { projection: code }) if code == projection.code()),
                "{projection:?} {pv:?} ({phi}, {theta}): {result:?}"
            );
        }
    }
}

/// The ZPN inverse stays on the branch inside the first point of inflection: a radius
/// past the polynomial's maximum is refused, one inside it inverts the forward map.
#[test]
fn zpn_inverts_only_inside_its_inflection() {
    let mut pv = [0.0; 21];
    pv[..4].copy_from_slice(&[0.0, 1.0, 0.0, -0.5]);
    let parameters = Projection::Zpn.parameters(pv).unwrap();
    // R(ζ) = ζ − ζ³/2 peaks at ζ = √(2/3): R = √(2/3)·(2/3) = 0.5443 rad = 31.19°.
    let peak = (2.0f64 / 3.0).sqrt() * 2.0 / 3.0 * R2D;
    assert!(matches!(
        parameters.deproject(0.0, -(peak + 0.01)),
        Err(FitsError::WcsPixelOutOfDomain { projection: "ZPN" })
    ));
    for theta in [80.0, 60.0, 45.0] {
        let projected = parameters
            .project(NativeCoordinate::from_degrees(0.0, theta))
            .unwrap();
        let native = parameters.deproject(projected.x, projected.y).unwrap();
        // The radius carries an ulp of rounding, which ζ takes divided by R′: at 45°,
        // R′ = 1 − 1.5ζ² = 0.075 near the peak, and the inverse is good to 9 ulps there.
        let zeta = NativeCoordinate::from_degrees(0.0, theta).zeta();
        let radius = evaluate_polynomial(zeta, &pv);
        let conditioned = 2.0 * f64::EPSILON * (radius.value / radius.derivative + zeta);
        assert!(
            (native.zeta() - zeta).abs() <= conditioned,
            "{theta}: {native:?}"
        );
    }
    // A quadratic whose ζ² term is 1e-12 of its linear one: the root (−b + √d)/2a
    // cancels to about 1e-4 of ζ there, c/q keeps it to an ulp.
    let mut quadratic = [0.0; 21];
    quadratic[..3].copy_from_slice(&[0.0, 1.0, 1e-12]);
    let quadratic = Projection::Zpn.parameters(quadratic).unwrap();
    for zeta in [1e-9, 0.5, 2.0] {
        let projected = quadratic
            .project(NativeCoordinate::from_colatitude(0.0, zeta))
            .unwrap();
        let native = quadratic.deproject(projected.x, projected.y).unwrap();
        assert!(
            (native.zeta() - zeta).abs() <= 2.0 * f64::EPSILON * zeta,
            "ζ = {zeta}: {native:?}"
        );
    }
    // Degenerate polynomials have no inverse at all.
    assert!(matches!(
        Projection::Zpn.parameters([0.0; 21]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: "ZPN" }
        })
    ));
    let mut falling = [0.0; 21];
    falling[..3].copy_from_slice(&[0.0, -1.0, 0.5]);
    assert!(matches!(
        Projection::Zpn.parameters(falling),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: "ZPN" }
        })
    ));
}

/// Near their limits the conic, Airy and polyconic formulas once cancelled to
/// nothing: at η = 1e-7° a COO cone constant lost half its digits, at ξ = 1e-8 rad
/// cos ξ rounded to 1 and the Airy constant came out 0 rather than −1/2, and near the
/// equator 1 − cos ω rounded away the polyconic's y offset.
#[test]
fn near_limit_projection_formulas_keep_their_precision() {
    use std::f64::consts::FRAC_PI_2;

    // COO: C = sin θ_a + O(η²), and η² is far below an ulp of sin 45°.
    let mut pv = [0.0; 21];
    pv[1] = 45.0;
    pv[2] = 1e-7;
    let coo = Conic::new(ConicKind::Coo, &pv);
    assert!(
        (coo.c - 45f64.to_radians().sin()).abs() <= 2.0 * f64::EPSILON,
        "{}",
        coo.c
    );

    // AIR: K = −1/2 + ξ²/4 + …, so −1/2 to the last bit at ξ = 1e-8.
    assert_eq!(Air::new(90.0 - 2.0 * 1e-8 * R2D).k, -0.5);
    // The inverse at a tiny colatitude, which a fixed 1e-6 start and an absolute
    // residual stop once returned with a relative error of 1e-3, and a residual stop of
    // 1e-12 with 6.5e-14 at ζ = 1e-6.
    let air = Air::new(45.0);
    for zeta in [1e-9, 1e-6, 1e-3, 1.0, 2.5, 3.0] {
        let back = air.zeta(air.radius(zeta)).unwrap();
        assert!(
            (back - zeta).abs() <= 2.0 * f64::EPSILON * zeta,
            "ζ = {zeta}: {back}, two ulps of the root and its radius"
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
    assert!(
        ((projected.y - expected) / expected).abs() < 1e-9,
        "{}",
        projected.y
    );
}
