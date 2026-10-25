use crate::alternate::Alternate;
use crate::fits_error::FitsError;
use crate::fits_error::NumericTarget;
use crate::fits_error::ValueKind;
use crate::fits_error::{TabularDefect, WcsDefect};
use crate::header_model::record_name::RecordName;
use crate::header_model::{Header, keywords};
use crate::internals::tolerance;
use crate::internals::tolerance::Tolerance;
use crate::internals::wcs_header;
use crate::world_coordinates::internals::{to_pixel, to_world};
use std::assert_matches;

use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular;
use crate::world_coordinates::tabular::internals::{
    COUPLED_GRID, from_table, lookup_table, resolved_wcs, tab_header,
};
use crate::world_coordinates::tabular::{TableArray, TabularArrays, TabularTransform};
use std::iter;
use std::panic;

/// The multi-dimensional inverse halves a cell's box 31 times, so the pixel it gives lies
/// within 2⁻³¹ of the box's width of the one it inverts: 2⁻³⁰ for the one cell of a grid
/// of two nodes an axis, 2 pixels wide with the half-cell extrapolations.
const SEARCH: Tolerance = Tolerance::abs(1.0 / (1u64 << 30) as f64, "31 halvings of a 2-pixel box");

fn affine_coordinates(dimensions: usize) -> Vec<f64> {
    let mut coordinates = Vec::with_capacity(dimensions * (1 << dimensions));
    for vertex in 0..1 << dimensions {
        for axis in 0..dimensions {
            let endpoint = usize::from(vertex & (1 << axis) != 0) as f64;
            coordinates.push(100.0 * axis as f64 + endpoint * (axis + 1) as f64);
        }
    }
    coordinates
}

fn coordinate_shape(dimensions: usize) -> String {
    format!(
        "({},{})",
        dimensions,
        iter::repeat_n("2", dimensions)
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[test]
fn one_dimensional_tab_supports_nonmonotonic_coordinates_and_boundaries() {
    let table = lookup_table(&[("COORD", &[10.0, 20.0, 15.0, 30.0], Some("(1,4)"))]);
    let header = tab_header(1);
    let wcs = resolved_wcs(&header, &table);

    let cases = [
        (0.5, 5.0),
        (1.0, 10.0),
        (2.5, 17.5),
        (3.5, 22.5),
        (4.5, 37.5),
    ];
    // The transform takes the arrays alone: built from them with no table, it maps each
    // case as the one the table gives.
    let descriptors = tabular::descriptors(&header, 1, Alternate::PRIMARY).unwrap();
    let arrays = TabularArrays {
        coordinates: TableArray {
            shape: vec![1, 4],
            values: vec![10.0, 20.0, 15.0, 30.0],
        },
        indices: vec![None],
    };
    let transform = TabularTransform::from_arrays(&descriptors[0], arrays).unwrap();
    let from_arrays =
        Wcs::from_header_with_tabular(&header, Alternate::PRIMARY, vec![transform]).unwrap();
    for (pixel, expected) in cases {
        assert_eq!(to_world(&wcs, &[pixel]).unwrap(), [expected]);
        assert_eq!(to_world(&from_arrays, &[pixel]).unwrap(), [expected]);
    }
    assert_eq!(to_pixel(&wcs, &[17.5]).unwrap(), [1.75]);
    assert_eq!(to_pixel(&wcs, &[37.5]).unwrap(), [4.5]);
    assert_matches!(
        to_world(&wcs, &[0.49]),
        Err(FitsError::WcsCoordinateOutOfDomain {
            axis: 0,
            algorithm: "TAB"
        })
    );
}

/// The binary search of the monotonic runs returns the interval the plain scan does —
/// the first in array order that holds the target — over arrays that turn, repeat a
/// value and repeat an index value, at every node, between them and past both ends.
#[test]
fn run_search_matches_the_first_interval_scan() {
    let scan = |coordinates: &[f64], index: Option<&[f64]>, target: f64| {
        (0..coordinates.len() - 1).find_map(|position| {
            let (first, second) = (coordinates[position], coordinates[position + 1]);
            let usable = index.is_none_or(|index| index[position] != index[position + 1]);
            (usable && first != second && (first.min(second)..=first.max(second)).contains(&target))
                .then(|| (position, (target - first) / (second - first)))
        })
    };
    let arrays: [(&[f64], Option<&[f64]>); 4] = [
        (&[10.0, 20.0, 15.0, 30.0, 30.0, 5.0, 40.0], None),
        (&[1.0, 2.0, 3.0, 2.0, 1.0, 1.0, 2.0, 3.0, 4.0], None),
        (&[5.0, 4.0, 3.0, 2.0, 1.0], None),
        (
            &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
            Some(&[0.0, 1.0, 1.0, 2.0, 3.0, 3.0]),
        ),
    ];
    for (coordinates, index) in arrays {
        let shape = format!("(1,{})", coordinates.len());
        let mut columns = vec![("COORD", coordinates, Some(shape.as_str()))];
        if let Some(index) = index {
            columns.push(("INDEX", index, None));
        }
        let table = lookup_table(&columns);
        let mut header = tab_header(1);
        if index.is_some() {
            header.set_internal("PS1_2", "INDEX");
        }
        let descriptor = &tabular::descriptors(&header, 1, Alternate::PRIMARY).unwrap()[0];
        let transform = from_table(descriptor, table.view()).unwrap();
        let tabular::InverseIndex::Runs(runs) = &transform.inverse else {
            panic!("a one-axis table searches runs");
        };
        let mut targets: Vec<f64> = coordinates.to_vec();
        targets.extend(
            coordinates
                .windows(2)
                .map(|pair| f64::midpoint(pair[0], pair[1])),
        );
        targets.extend([-100.0, 100.0]);
        for target in targets {
            let found = transform.locate_one_dimensional(runs, target).ok();
            let scanned = if target == coordinates[0] {
                Some((0, 0.0))
            } else {
                scan(coordinates, index, target)
            };
            if let Some(scanned) = scanned {
                let found = found.expect("the scan finds an interval");
                assert_eq!(
                    (found.position, found.fraction),
                    scanned,
                    "{coordinates:?} at {target}"
                );
            }
        }
    }
}

/// The inverse passes over each cell whose corners cannot hold the point, so its
/// budget of 2²² work units, 16 for each cell of a two-axis table, no longer runs out
/// at 2¹⁸ = 262 144 cells: here a point in the last of 512 × 512.
#[test]
fn coupled_inverse_reaches_past_the_budget_of_every_cell() {
    const NODES: usize = 513;
    let coordinates: Vec<f64> = (0..NODES * NODES)
        .flat_map(|node| [(node % NODES) as f64, (node / NODES) as f64 * 2.0])
        .collect();
    let shape = format!("(2,{NODES},{NODES})");
    let table = lookup_table(&[("COORD", &coordinates, Some(shape.as_str()))]);
    let wcs = resolved_wcs(&tab_header(2), &table);
    // Pixel (p, q) lies at array position (p, q) and world (p − 1, 2(q − 1)).
    let pixel = [512.25, 512.75];
    let world = to_world(&wcs, &pixel).unwrap();
    assert_eq!(world, [511.25, 1023.5]);
    wcs_header::assert_round_trip(
        &wcs,
        &[pixel],
        Tolerance::abs(
            1.5 / (1u64 << 31) as f64,
            "31 halvings of the last cell's box, 1.5 wide with its half-cell extrapolation",
        ),
        "the last cell",
    );
}

#[test]
fn index_vectors_may_decrease_and_change_sampling() {
    let table = lookup_table(&[
        ("COORD", &[100.0, 200.0, 400.0, 800.0], Some("(1,4)")),
        ("INDEX", &[40.0, 30.0, 10.0, 0.0], None),
    ]);
    let mut header = tab_header(1);
    header
        .set_internal("PS1_2", "INDEX")
        .set_internal("CRVAL1", 30.0);
    let wcs = resolved_wcs(&header, &table);

    assert_eq!(to_world(&wcs, &[0.0]).unwrap(), [200.0]);
    assert_eq!(to_world(&wcs, &[5.0]).unwrap(), [150.0]);
    assert_eq!(to_world(&wcs, &[-22.5]).unwrap(), [500.0]);
    wcs_header::assert_round_trip(&wcs, &[[5.0], [0.0], [-22.5]], Tolerance::EXACT, "INDEX");
}

#[test]
fn index_search_keeps_the_duplicate_and_exact_hit_rules() {
    for (index, reference, expected) in [
        (&[0.0, 0.0, 10.0, 20.0][..], 0.0, 200.0),
        (&[0.0, 10.0, 10.0, 20.0], 10.0, 200.0),
        (&[20.0, 20.0, 10.0, 0.0], 20.0, 200.0),
        (&[20.0, 10.0, 10.0, 0.0], 10.0, 200.0),
    ] {
        let table = lookup_table(&[
            ("COORD", &[100.0, 200.0, 400.0, 800.0], Some("(1,4)")),
            ("INDEX", index, None),
        ]);
        let mut header = tab_header(1);
        header
            .set_internal("PS1_2", "INDEX")
            .set_internal("CRVAL1", reference);
        let wcs = resolved_wcs(&header, &table);
        assert_eq!(to_world(&wcs, &[0.0]).unwrap(), [expected]);
        assert_eq!(to_pixel(&wcs, &[expected]).unwrap(), [0.0]);
    }
}

#[test]
fn coupled_axes_interpolate_and_invert() {
    let table = lookup_table(&[("COORD", &COUPLED_GRID, Some("(2,2,2)"))]);
    let mut header = tab_header(2);
    header
        .set_internal("PS1_0", "wcs-table")
        .set_internal("PS1_1", "coord");
    let wcs = resolved_wcs(&header, &table);

    assert_eq!(to_world(&wcs, &[1.25, 1.5]).unwrap(), [102.5, 210.0]);
    wcs_header::assert_round_trip(
        &wcs,
        &[[1.25, 1.5], [0.5, 1.5], [1.25, 2.5], [0.5, 2.5]],
        SEARCH,
        "the inside and the boundary",
    );

    for dimensions in 3..=6 {
        let coordinates = affine_coordinates(dimensions);
        let shape = coordinate_shape(dimensions);
        let table = lookup_table(&[("COORD", &coordinates, Some(&shape))]);
        let header = tab_header(dimensions);
        let wcs = resolved_wcs(&header, &table);
        let pixel = vec![1.25; dimensions];
        let world = to_world(&wcs, &pixel).unwrap();
        let expected: Vec<f64> = (0..dimensions)
            .map(|axis| 100.0 * axis as f64 + 0.25 * (axis + 1) as f64)
            .collect();
        tolerance::assert_close(
            &world,
            &expected,
            Tolerance::abs(
                1e-12,
                "multilinear interpolation, a few ulps of coordinates to 1300",
            ),
            &format!("{dimensions} axes"),
        );
        wcs_header::assert_round_trip(&wcs, &[pixel], SEARCH, &format!("{dimensions} axes"));
    }
}

#[test]
fn coupled_inverse_takes_the_most_axes() {
    // Seven coupled axes, the most a transform takes, invert in their fixed arrays: the
    // affine table maps 1.25 on each axis and back exactly.
    let dimensions = 7;
    let coordinates = affine_coordinates(dimensions);
    let shape = coordinate_shape(dimensions);
    let table = lookup_table(&[("COORD", &coordinates, Some(&shape))]);
    let header = tab_header(dimensions);
    let wcs = resolved_wcs(&header, &table);
    let world = to_world(&wcs, &vec![1.25; dimensions]).unwrap();
    assert_eq!(to_pixel(&wcs, &world).unwrap(), vec![1.25; dimensions]);
}

#[test]
fn time_axes_feed_the_typed_time_layer() {
    let table = lookup_table(&[("COORD", &[0.0, 2.0], Some("(1,2)"))]);
    let mut header = tab_header(1);
    header
        .set_internal("CTYPE1", "TIME-TAB")
        .set_internal("CUNIT1", "d")
        .set_internal(keywords::MJDREF, 50_000.0)
        .set_internal(keywords::TIMESYS, "UTC");
    let wcs = resolved_wcs(&header, &table);
    let coordinate = Frame::from_header(&header)
        .unwrap()
        .wcs_time(&wcs, 0, &[1.5])
        .unwrap()
        .unwrap();
    assert_eq!(coordinate.mjd, Mjd::new(50_001, 0.0));

    let table = lookup_table(&[("COORD", &[100.0, 200.0, 300.0, 400.0], Some("(1,4)"))]);
    let mut header = Header::new();
    header
        .set_internal(keywords::NAXIS, 3)
        .set_internal("CTYPE1", "LINEAR")
        .set_internal("CTYPE2", "TIME-TAB")
        .set_internal("CTYPE3", "LINEAR")
        .set_internal("CUNIT2", "s")
        .set_internal("CRPIX1", 10.0)
        .set_internal("CRPIX2", 20.0)
        .set_internal("CRPIX3", 30.0)
        .set_internal("CRVAL1", 0.0)
        .set_internal("CRVAL2", 0.0)
        .set_internal("CRVAL3", 0.0)
        .set_internal("CDELT1", 1.0)
        .set_internal("CDELT2", 1.0)
        .set_internal("CDELT3", 1.0)
        .set_internal("PC2_1", 0.5)
        .set_internal("PC2_3", 0.25)
        .set_internal("PS2_0", "WCS-TABLE")
        .set_internal("PS2_1", "COORD")
        .set_internal("PV2_3", 1)
        .set_internal(keywords::MJDREF, 50_000.0)
        .set_internal(keywords::TIMESYS, "UTC");
    let wcs = resolved_wcs(&header, &table);
    let coordinate = Frame::from_header(&header)
        .unwrap()
        .wcs_time(&wcs, 1, &[12.0, 20.5, 34.0])
        .unwrap()
        .unwrap();
    assert_eq!(coordinate.mjd, Mjd::new(50_000, 250.0 / 86_400.0));
}

#[test]
fn spectral_coordinates_normalize_declared_units() {
    let table = lookup_table(&[("COORD", &[500.0, 600.0], Some("(1,2)"))]);
    let mut header = tab_header(1);
    header
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "nm");
    let wcs = resolved_wcs(&header, &table);
    let world = to_world(&wcs, &[1.5]).unwrap();
    tolerance::assert_within(
        world[0],
        5.5e-7,
        Tolerance::abs(
            1e-20,
            "the nm factor and the interpolation, ulps of 5.5e-7 m (1.1e-22)",
        ),
        "WAVE-TAB in metres",
    );
    wcs_header::assert_round_trip(
        &wcs,
        &[[1.5]],
        Tolerance::ulps(
            2.0,
            "the scale of nm to m rounds the world, and its inverse rounds it back",
        ),
        "nm",
    );
    // The table's nanometres come out in metres, and the axis says so.
    assert_eq!(wcs.world_unit(0).unwrap(), "m");
    let axis = wcs.axis_world(0, &[1.5]).unwrap();
    assert_eq!((axis.unit, axis.value), ("m", world[0]));
}

#[test]
fn missing_references_bad_shapes_and_nonmonotonic_indices_are_refused() {
    let mut missing = Header::new();
    missing
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "AX01-TAB")
        .set_internal("PS1_1", "COORD");
    assert_matches!(
        tabular::descriptors(&missing, 1, Alternate::PRIMARY),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::Tabular(TabularDefect::MissingExtension)
        })
    );

    let bad_shape = lookup_table(&[("COORD", &[1.0, 2.0, 3.0, 4.0], Some("(2,2)"))]);
    let header = tab_header(1);
    let descriptor = tabular::descriptors(&header, 1, Alternate::PRIMARY)
        .unwrap()
        .remove(0);
    assert_matches!(
        from_table(&descriptor, bad_shape.view()),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::Tabular(TabularDefect::CoordinateShape)
        })
    );

    let bad_index = lookup_table(&[
        ("COORD", &[1.0, 2.0, 3.0], Some("(1,3)")),
        ("INDEX", &[1.0, 3.0, 2.0], None),
    ]);
    let mut header = tab_header(1);
    header.set_internal("PS1_2", "INDEX");
    let descriptor = tabular::descriptors(&header, 1, Alternate::PRIMARY)
        .unwrap()
        .remove(0);
    assert_matches!(
        from_table(&descriptor, bad_index.view()),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::Tabular(TabularDefect::NonMonotonicIndex)
        })
    );

    let mut oversized_axis = tab_header(1);
    oversized_axis.set_internal("PV1_3", i64::MAX);
    assert_matches!(
        tabular::descriptors(&oversized_axis, 1, Alternate::PRIMARY),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::Tabular(TabularDefect::TableAxisPastCount)
        })
    );

    // An EXTVER is a positive integer: zero is refused, a fraction is no integer, and
    // 2⁶³, the first real past i64::MAX, is no EXTVER rather than a saturated one.
    let refusal = |value: f64| {
        let mut header = tab_header(1);
        header.set_internal("PV1_1", value);
        tabular::descriptors(&header, 1, Alternate::PRIMARY).unwrap_err()
    };
    assert_matches!(
        refusal(0.0),
        FitsError::InvalidWcs {
            defect: WcsDefect::Tabular(TabularDefect::NonPositiveParameter)
        }
    );
    assert_matches!(
        refusal(1.5),
        FitsError::ValueTypeMismatch {
            name: RecordName::Keyword(name),
            expected: ValueKind::Integer
        } if name == "PV1_1"
    );
    // f64's shortest form of 2⁶³ is the text the error keeps.
    assert_matches!(
        refusal(2f64.powi(63)),
        FitsError::IntegerOutOfRange {
            name: Some(RecordName::Keyword(name)),
            value,
            target: NumericTarget::I64
        } if name == "PV1_1" && value == "9223372036854776000"
    );

    // A coupled table of seven axes is the most a transform takes; one of eight is
    // refused at parse, so a point never needs more than its fixed arrays.
    let coupled = |axes: usize| {
        let values = vec![1.0; axes];
        let shape = format!("({axes}{})", ",1".repeat(axes));
        let table = lookup_table(&[("COORD", &values, Some(shape.as_str()))]);
        let descriptors =
            tabular::descriptors(&tab_header(axes), axes, Alternate::PRIMARY).unwrap();
        assert_eq!(descriptors.len(), 1, "one coupled table");
        from_table(&descriptors[0], table.view()).map(|_| ())
    };
    coupled(7).unwrap();
    assert_matches!(
        coupled(8),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::Tabular(TabularDefect::TooManyAxes)
        })
    );
}

/// A one-element index vector Ψ = (100) takes ψ ∈ [99.5, 100.5] to Υ = ψ − Ψ₁ + 1 on the
/// one coordinate, and back: the coordinate 42 is ψ = 100.
#[test]
fn one_element_index_vector_maps_onto_its_value() {
    let table = lookup_table(&[("COORD", &[42.0], Some("(1,1)")), ("INDEX", &[100.0], None)]);
    let mut header = tab_header(1);
    header
        .set_internal("PS1_2", "INDEX")
        .set_internal("CRVAL1", 100.0);
    let wcs = resolved_wcs(&header, &table);

    for pixel in [-0.5, 0.0, 0.4, 0.5] {
        assert_eq!(to_world(&wcs, &[pixel]).unwrap(), [42.0], "pixel {pixel}");
    }
    for pixel in [-0.6, 0.6, 100.0] {
        assert_matches!(
            to_world(&wcs, &[pixel]),
            Err(FitsError::WcsCoordinateOutOfDomain {
                axis: 0,
                algorithm: "TAB"
            }),
            "pixel {pixel}"
        );
    }
    assert_eq!(to_pixel(&wcs, &[42.0]).unwrap(), [0.0]);
}

/// The corner test is relative to the coordinates, so a wavelength axis in metres
/// (cells of 1e-8 m) inverts as precisely as one in nanometres: an absolute 1e-10
/// once snapped any target within a hundredth of a cell to a corner. The pixel is
/// off the dyadic grid, so no sub-voxel corner hits it exactly; the inverse then
/// bisects to [`SEARCH`].
#[test]
fn tab_inverse_is_scale_free() {
    for scale in [1.0, 1e-9, 1e9] {
        let coordinates = COUPLED_GRID.map(|value: f64| value * scale);
        let table = lookup_table(&[("COORD", &coordinates, Some("(2,2,2)"))]);
        let wcs = resolved_wcs(&tab_header(2), &table);
        wcs_header::assert_round_trip(&wcs, &[[1.3, 1.7]], SEARCH, &format!("scale {scale}"));
    }
}

/// The subvoxel side is `2^-level` exactly: 1 halved `level` times, each halving exact
/// through the subnormals, the last of which, `2^-1074`, halves to 0.
#[test]
fn subvoxel_size_is_the_exact_power_of_two() {
    let mut size = 1.0f64;
    for level in 0..1100 {
        assert_eq!(
            tabular::subvoxel_size(level).to_bits(),
            size.to_bits(),
            "{level}"
        );
        size /= 2.0;
    }
}
