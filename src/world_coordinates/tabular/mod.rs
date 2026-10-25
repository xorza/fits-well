use crate::allocation;
use crate::alternate::Alternate;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::{TabularDefect, WcsDefect};
use crate::header_model::Header;
use crate::keyword::Keyword;
use crate::world_coordinates::ctype::Ctype;
use crate::world_coordinates::header_read;
use crate::world_coordinates::world_unit::WorldUnit;
use std::iter;

/// The table axes a transform takes, so a point's interpolation and its inverse search
/// work in fixed arrays on the stack: 2⁷ corners of 7 coordinates, 7.5 KB, at most,
/// within clippy's 16 KiB bound on a local array.
const MAX_TABLE_AXES: usize = 7;
const MAX_INVERSE_WORK: usize = 1 << 22;
/// The reals a point's search holds at most: six per table axis, then the corners.
const MAX_SEARCH_REALS: usize = MAX_TABLE_AXES * (1 << MAX_TABLE_AXES) + 6 * MAX_TABLE_AXES;

#[derive(Debug, Clone)]
pub(crate) struct TabularReference {
    pub(crate) extension_name: String,
    pub(crate) extension_version: i64,
    pub(crate) extension_level: i64,
    pub(crate) coordinate_column: String,
}

impl TabularReference {
    pub(crate) fn identifies_same_extension(&self, other: &Self) -> bool {
        self.extension_name
            .eq_ignore_ascii_case(&other.extension_name)
            && self.extension_version == other.extension_version
            && self.extension_level == other.extension_level
    }

    fn identifies_same_array(&self, other: &Self) -> bool {
        self.identifies_same_extension(other)
            && self
                .coordinate_column
                .eq_ignore_ascii_case(&other.coordinate_column)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TabularDescriptor {
    pub(crate) reference: TabularReference,
    /// Each table axis, in order.
    axes: Vec<TableAxis>,
}

impl TabularDescriptor {
    /// The column of each table axis's index vector, in order: `None` for an axis that
    /// declares none.
    pub(crate) fn index_columns(&self) -> impl Iterator<Item = Option<&str>> {
        self.axes.iter().map(|axis| axis.index_column.as_deref())
    }
}

/// One axis of a `-TAB` table: the image axis it maps, its reference index (`CRVAL`), the
/// column of its index vector when it has one, and the scale of its world unit.
#[derive(Debug, Clone)]
struct TableAxis {
    image_axis: usize,
    reference_index: f64,
    index_column: Option<String>,
    world_scale: f64,
}

/// A descriptor whose table axes are still being read: each table axis once a header axis
/// names it.
#[derive(Debug)]
struct PendingDescriptor {
    reference: TabularReference,
    axes: Vec<Option<TableAxis>>,
}

#[derive(Debug, Clone)]
pub(crate) struct TabularTransform {
    pub(super) axes: Vec<usize>,
    reference_indices: Vec<f64>,
    lengths: Vec<usize>,
    variable_axes: Vec<usize>,
    vertex_count: usize,
    indices: Vec<Option<IndexVector>>,
    coordinates: Vec<f64>,
    inverse: InverseIndex,
}

/// What the inverse searches, built with the transform.
#[derive(Debug, Clone)]
enum InverseIndex {
    /// One table axis: the coordinate array's maximal strictly monotonic runs, in order.
    Runs(Vec<MonotonicRun>),
    /// More: the cells along each table axis, and for each cell and coordinate axis the
    /// least and the greatest of its extended corners and their rounding tolerance,
    /// `[min, max, tolerance]`, which the search's first level tests.
    Cells {
        cell_lengths: Vec<usize>,
        bounds: Vec<f64>,
    },
}

/// Nodes `start..=end` of a one-axis coordinate array, which rise or fall strictly and
/// whose intervals all have distinct index values.
#[derive(Debug, Clone, Copy)]
struct MonotonicRun {
    start: usize,
    end: usize,
    direction: IndexDirection,
}

/// Where a point lies along one table axis: the zero-based node below it, and the
/// fraction of the interval past that node, in [−0.5, 1.5] at the ends.
#[derive(Debug, Clone, Copy)]
struct Interval {
    position: usize,
    fraction: f64,
}

#[derive(Debug, Clone, Copy)]
enum IndexDirection {
    Increasing,
    Decreasing,
}

#[derive(Debug, Clone)]
struct IndexVector {
    values: Vec<f64>,
    direction: IndexDirection,
}

pub(crate) fn descriptors(
    header: &Header,
    axis_count: usize,
    alt: Alternate,
) -> Result<Vec<TabularDescriptor>> {
    let mut descriptors = Vec::<PendingDescriptor>::new();
    for axis in 0..axis_count {
        let ctype = header_read::text(
            header,
            Keyword::fitting(format_args!("CTYPE{}{alt}", axis + 1)),
        )?
        .unwrap_or("");
        if Ctype::parse(ctype).algorithm != Some("TAB") {
            continue;
        }
        let extension_name = required_text(
            header,
            Keyword::try_from(format_args!("PS{}_0{alt}", axis + 1))?,
            TabularDefect::MissingExtension,
        )?;
        let coordinate_column = required_text(
            header,
            Keyword::try_from(format_args!("PS{}_1{alt}", axis + 1))?,
            TabularDefect::MissingCoordinateColumn,
        )?;
        let reference = TabularReference {
            extension_name,
            extension_version: integer_parameter(header, axis, 1, alt, 1)?,
            extension_level: integer_parameter(header, axis, 2, alt, 1)?,
            coordinate_column,
        };
        let table_axis = integer_parameter(header, axis, 3, alt, 1)?;
        let table_axis = usize::try_from(table_axis)
            .ok()
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| invalid(TabularDefect::NonPositiveParameter))?;
        if table_axis >= axis_count {
            return Err(invalid(TabularDefect::TableAxisPastCount));
        }
        let reference_index = header_read::real(
            header,
            Keyword::fitting(format_args!("CRVAL{}{alt}", axis + 1)),
        )?
        .unwrap_or(0.0);
        if !reference_index.is_finite() {
            return Err(invalid(TabularDefect::NonFiniteReference));
        }
        let index_column = header_read::text(
            header,
            Keyword::fitting(format_args!("PS{}_2{alt}", axis + 1)),
        )?
        .map(str::to_string);

        let descriptor = if let Some(descriptor) = descriptors
            .iter_mut()
            .find(|descriptor| descriptor.reference.identifies_same_array(&reference))
        {
            descriptor
        } else {
            descriptors.push(PendingDescriptor {
                reference,
                axes: Vec::new(),
            });
            descriptors.last_mut().unwrap()
        };
        let required_len = table_axis
            .checked_add(1)
            .ok_or_else(|| invalid(TabularDefect::TableAxisPastCount))?;
        if descriptor.axes.len() < required_len {
            descriptor.axes.resize_with(required_len, || None);
        }
        if descriptor.axes[table_axis].is_some() {
            return Err(invalid(TabularDefect::DuplicateTableAxis));
        }
        let cunit = header_read::text(
            header,
            Keyword::fitting(format_args!("CUNIT{}{alt}", axis + 1)),
        )?
        .unwrap_or("");
        descriptor.axes[table_axis] = Some(TableAxis {
            image_axis: axis,
            reference_index,
            index_column,
            world_scale: WorldUnit::of(Ctype::parse(ctype), cunit)?.scale,
        });
    }
    descriptors
        .into_iter()
        .map(|PendingDescriptor { reference, axes }| {
            Ok(TabularDescriptor {
                reference,
                axes: axes
                    .into_iter()
                    .collect::<Option<_>>()
                    .ok_or_else(|| invalid(TabularDefect::IncompleteTableAxes))?,
            })
        })
        .collect()
}

impl TabularDescriptor {
    pub(crate) fn referenced_columns(&self) -> impl Iterator<Item = &str> {
        iter::once(self.reference.coordinate_column.as_str()).chain(
            self.axes
                .iter()
                .filter_map(|axis| axis.index_column.as_deref()),
        )
    }
}

impl TabularTransform {
    /// The transform `descriptor` declares over `arrays`, its table's first-row arrays,
    /// checked against the declaration.
    pub(crate) fn from_arrays(
        descriptor: &TabularDescriptor,
        arrays: TabularArrays,
    ) -> Result<Self> {
        let dimensions = descriptor.axes.len();
        if dimensions > MAX_TABLE_AXES {
            return Err(invalid(TabularDefect::TooManyAxes));
        }
        let TabularArrays {
            coordinates: coordinate_shape,
            indices: index_arrays,
        } = arrays;
        debug_assert_eq!(index_arrays.len(), dimensions, "an index slot a table axis");
        let mut shape = coordinate_shape.shape;
        if dimensions == 1 && shape.len() == 1 {
            shape.insert(0, 1);
        }
        if shape.len() != dimensions + 1 || shape[0] != dimensions {
            return Err(invalid(TabularDefect::CoordinateShape));
        }
        let lengths = shape[1..].to_vec();
        if lengths.contains(&0) {
            return Err(invalid(TabularDefect::EmptyAxis));
        }
        let variable_axes: Vec<usize> = lengths
            .iter()
            .enumerate()
            .filter_map(|(axis, &length)| (length > 1).then_some(axis))
            .collect();
        let vertex_count = 1 << variable_axes.len();
        let coordinate_count = lengths
            .iter()
            .try_fold(dimensions, |count, &length| count.checked_mul(length))
            .ok_or_else(|| invalid(TabularDefect::CoordinateArrayTooLarge))?;
        if coordinate_shape.values.len() != coordinate_count {
            return Err(invalid(TabularDefect::CoordinateCount));
        }
        if coordinate_shape
            .values
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(invalid(TabularDefect::NonFiniteCoordinate));
        }
        let mut coordinates = coordinate_shape.values;
        for point in coordinates.chunks_exact_mut(dimensions) {
            for (value, axis) in point.iter_mut().zip(&descriptor.axes) {
                *value *= axis.world_scale;
            }
        }

        let mut indices = Vec::with_capacity(dimensions);
        for (table_axis, index) in index_arrays.into_iter().enumerate() {
            let Some(index) = index else {
                indices.push(None);
                continue;
            };
            if index.shape.len() > 1 || index.values.len() != lengths[table_axis] {
                return Err(invalid(TabularDefect::IndexLength));
            }
            let direction = validate_index(&index.values)?;
            indices.push(Some(IndexVector {
                values: index.values,
                direction,
            }));
        }
        let mut transform = Self {
            axes: descriptor.axes.iter().map(|axis| axis.image_axis).collect(),
            reference_indices: descriptor
                .axes
                .iter()
                .map(|axis| axis.reference_index)
                .collect(),
            lengths,
            variable_axes,
            vertex_count,
            indices,
            coordinates,
            inverse: InverseIndex::Runs(Vec::new()),
        };
        transform.inverse = if dimensions == 1 {
            InverseIndex::Runs(transform.monotonic_runs())
        } else {
            transform.cell_bounds()?
        };
        Ok(transform)
    }

    /// The maximal strictly monotonic runs of a one-axis array. A run ends where two
    /// neighbours are equal, where their index values are, and where the array turns.
    fn monotonic_runs(&self) -> Vec<MonotonicRun> {
        let coordinates = &self.coordinates;
        let usable = |position: usize| {
            coordinates[position] != coordinates[position + 1]
                && self.indices[0]
                    .as_ref()
                    .is_none_or(|index| index.values[position] != index.values[position + 1])
        };
        let mut runs = Vec::new();
        let mut position = 0;
        while position + 1 < coordinates.len() {
            if !usable(position) {
                position += 1;
                continue;
            }
            let rising = coordinates[position] < coordinates[position + 1];
            let start = position;
            while position + 1 < coordinates.len()
                && usable(position)
                && (coordinates[position] < coordinates[position + 1]) == rising
            {
                position += 1;
            }
            runs.push(MonotonicRun {
                start,
                end: position,
                direction: if rising {
                    IndexDirection::Increasing
                } else {
                    IndexDirection::Decreasing
                },
            });
        }
        runs
    }

    /// Each cell's level-0 corners, as the search evaluates them first: the bounds that
    /// let it pass over a cell that cannot hold the point without spending its budget.
    fn cell_bounds(&self) -> Result<InverseIndex> {
        let cell_lengths: Vec<usize> = self
            .lengths
            .iter()
            .map(|&length| length.saturating_sub(1).max(1))
            .collect();
        let cell_count = cell_lengths
            .iter()
            .try_fold(1usize, |count, &length| count.checked_mul(length))
            .ok_or_else(|| invalid(TabularDefect::CoordinateArrayTooLarge))?;
        let dimensions = self.axes.len();
        let mut bounds = Vec::new();
        allocation::try_reserve(&mut bounds, cell_count.saturating_mul(3 * dimensions))?;
        let mut buffers = SearchBuffers::new();
        // The corners alone, no search: no node spends the budget.
        let mut search = buffers.search(self, 0);
        for cell in 0..cell_count {
            search.enter_cell(cell, &cell_lengths);
            search.fill_corners(0, 1.0);
            for axis in 0..dimensions {
                let corners =
                    (0..self.vertex_count).map(|vertex| search.corners[vertex * dimensions + axis]);
                let (min, max) = corners
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
                        (min.min(value), max.max(value))
                    });
                bounds.extend([min, max, search.tolerance[axis]]);
            }
        }
        Ok(InverseIndex::Cells {
            cell_lengths,
            bounds,
        })
    }

    /// Replace the intermediate world coordinate of this transform's axes in
    /// `coordinates` with their world coordinate.
    pub(super) fn to_world(&self, coordinates: &mut [f64]) -> Result<()> {
        if self.axes.len() == 1 {
            let image_axis = self.axes[0];
            coordinates[image_axis] = self.world_one_dimensional(coordinates[image_axis])?;
            return Ok(());
        }
        let mut location = self.interpolation_location(|_, image_axis| coordinates[image_axis])?;
        for &image_axis in &self.axes {
            coordinates[image_axis] = 0.0;
        }
        self.for_each_interpolation_vertex(
            &mut location.base,
            &location.delta,
            |offset, weight| {
                for (table_axis, &image_axis) in self.axes.iter().enumerate() {
                    coordinates[image_axis] += self.coordinates[offset + table_axis] * weight;
                }
            },
        );
        Ok(())
    }

    /// The world coordinate of `image_axis` alone, from the intermediate coordinate
    /// `intermediate` gives each image axis of the transform.
    pub(super) fn to_world_axis(
        &self,
        image_axis: usize,
        intermediate: impl Fn(usize) -> f64,
    ) -> Result<f64> {
        if self.axes.len() == 1 {
            return self.world_one_dimensional(intermediate(self.axes[0]));
        }
        let table_axis = self
            .axes
            .iter()
            .position(|&axis| axis == image_axis)
            .expect("requested TAB axis belongs to the transform");
        let mut location = self.interpolation_location(|_, image_axis| intermediate(image_axis))?;
        let mut result = 0.0;
        self.for_each_interpolation_vertex(
            &mut location.base,
            &location.delta,
            |offset, weight| {
                result += self.coordinates[offset + table_axis] * weight;
            },
        );
        Ok(result)
    }

    /// Replace the world coordinate of this transform's axes in `coordinates` with
    /// their intermediate world coordinate.
    pub(super) fn to_intermediate(&self, coordinates: &mut [f64]) -> Result<()> {
        match &self.inverse {
            InverseIndex::Runs(runs) => {
                let image_axis = self.axes[0];
                let interval = self.locate_one_dimensional(runs, coordinates[image_axis])?;
                coordinates[image_axis] = self.intermediate(0, interval)?;
                Ok(())
            }
            InverseIndex::Cells {
                cell_lengths,
                bounds,
            } => self.invert_multidimensional(cell_lengths, bounds, coordinates),
        }
    }

    /// The intermediate coordinate of `interval` along `table_axis`.
    fn intermediate(&self, table_axis: usize, interval: Interval) -> Result<f64> {
        let upsilon = interval.position as f64 + 1.0 + interval.fraction;
        if !(0.5..=self.lengths[table_axis] as f64 + 0.5).contains(&upsilon) {
            return Err(domain(self.axes[table_axis]));
        }
        Ok(self.array_to_index(table_axis, upsilon) - self.reference_indices[table_axis])
    }

    /// The world coordinate of a one-axis table at `intermediate`: the vertex loop's two
    /// weights, so the same bits as [`Self::for_each_interpolation_vertex`] gives.
    fn world_one_dimensional(&self, intermediate: f64) -> Result<f64> {
        let interval = self.axis_interval(0, intermediate)?;
        if self.lengths[0] == 1 {
            return Ok(self.coordinates[0]);
        }
        let Interval { position, fraction } = interval;
        Ok(self.coordinates[position] * (1.0 - fraction)
            + self.coordinates[position + 1] * fraction)
    }

    fn interpolation_location(
        &self,
        mut intermediate: impl FnMut(usize, usize) -> f64,
    ) -> Result<TabularLocation> {
        let mut location = TabularLocation {
            base: [0; MAX_TABLE_AXES],
            delta: [0.0; MAX_TABLE_AXES],
        };
        for table_axis in 0..self.axes.len() {
            let interval =
                self.axis_interval(table_axis, intermediate(table_axis, self.axes[table_axis]))?;
            location.base[table_axis] = interval.position;
            location.delta[table_axis] = interval.fraction;
        }
        Ok(location)
    }

    /// Where the intermediate coordinate `intermediate` lies along `table_axis`: the
    /// interval below its array coordinate, the first or the last one past an end.
    fn axis_interval(&self, table_axis: usize, intermediate: f64) -> Result<Interval> {
        let psi = intermediate + self.reference_indices[table_axis];
        let upsilon = self.index_to_array(table_axis, psi)?;
        let length = self.lengths[table_axis];
        if !(0.5..=length as f64 + 0.5).contains(&upsilon) {
            return Err(domain(self.axes[table_axis]));
        }
        let one_relative = upsilon.floor() as isize;
        let mut zero_relative = one_relative - 1;
        let mut fraction = upsilon - one_relative as f64;
        if one_relative == 0 {
            zero_relative += 1;
            fraction -= 1.0;
        } else if one_relative == isize::try_from(length).expect("a table axis fits isize")
            && length > 1
        {
            zero_relative -= 1;
            fraction += 1.0;
        }
        Ok(Interval {
            position: usize::try_from(zero_relative).expect("TAB base index"),
            fraction,
        })
    }

    fn index_to_array(&self, table_axis: usize, psi: f64) -> Result<f64> {
        if !psi.is_finite() {
            return Err(domain(self.axes[table_axis]));
        }
        let Some(index) = &self.indices[table_axis] else {
            return Ok(psi);
        };
        let values = &index.values;
        if values.len() == 1 {
            // One index value spans the half-units either side of it: Υ = ψ − Ψ₁ + 1 on
            // [0.5, 1.5]. wcslib's `tabx2s` takes Υ = ψ here and then refuses any Υ past
            // 1.5, which leaves the case usable only when Ψ₁ ≈ 1.
            if (values[0] - 0.5..=values[0] + 0.5).contains(&psi) {
                return Ok(psi - values[0] + 1.0);
            }
            return Err(domain(self.axes[table_axis]));
        }
        let first_step = (values[1] - values[0]).abs();
        let last = values.len() - 1;
        let last_step = (values[last] - values[last - 1]).abs();
        match index.direction {
            IndexDirection::Increasing => {
                if psi < values[0] - 0.5 * first_step || psi > values[last] + 0.5 * last_step {
                    return Err(domain(self.axes[table_axis]));
                }
            }
            IndexDirection::Decreasing => {
                if psi > values[0] + 0.5 * first_step || psi < values[last] - 0.5 * last_step {
                    return Err(domain(self.axes[table_axis]));
                }
            }
        }
        let segment = match index.direction {
            IndexDirection::Increasing if psi < values[0] => 0,
            IndexDirection::Increasing if psi > values[last] => last - 1,
            IndexDirection::Decreasing if psi > values[0] => 0,
            IndexDirection::Decreasing if psi < values[last] => last - 1,
            IndexDirection::Increasing => {
                let lower = values.partition_point(|&value| value < psi);
                if lower != 0 {
                    lower - 1
                } else {
                    values.partition_point(|&value| value <= psi) - 1
                }
            }
            IndexDirection::Decreasing => {
                let lower = values.partition_point(|&value| value > psi);
                if lower != 0 {
                    lower - 1
                } else {
                    values.partition_point(|&value| value >= psi) - 1
                }
            }
        };
        if values[segment] == values[segment + 1] {
            return Err(domain(self.axes[table_axis]));
        }
        Ok(
            segment as f64
                + 1.0
                + (psi - values[segment]) / (values[segment + 1] - values[segment]),
        )
    }

    #[expect(
        clippy::cast_sign_loss,
        reason = "the caller's domain check keeps upsilon at 0.5 or more"
    )]
    fn array_to_index(&self, table_axis: usize, upsilon: f64) -> f64 {
        let Some(index) = &self.indices[table_axis] else {
            return upsilon;
        };
        let values = &index.values;
        if values.len() == 1 {
            return upsilon + values[0] - 1.0;
        }
        let position = upsilon.floor() as usize;
        let lower = position.saturating_sub(1).min(values.len() - 2);
        values[lower] + (upsilon - lower as f64 - 1.0) * (values[lower + 1] - values[lower])
    }

    fn for_each_interpolation_vertex(
        &self,
        indices: &mut [usize],
        delta: &[f64],
        mut visit: impl FnMut(usize, f64),
    ) {
        for vertex in 0..self.vertex_count {
            if vertex != 0 {
                let changed = vertex ^ (vertex - 1);
                for (bit, &table_axis) in self.variable_axes.iter().enumerate() {
                    if changed & (1 << bit) != 0 {
                        if vertex & (1 << bit) == 0 {
                            indices[table_axis] -= 1;
                        } else {
                            indices[table_axis] += 1;
                        }
                    }
                }
            }
            let mut weight = 1.0;
            for (bit, &table_axis) in self.variable_axes.iter().enumerate() {
                if vertex & (1 << bit) == 0 {
                    weight *= 1.0 - delta[table_axis];
                } else {
                    weight *= delta[table_axis];
                }
            }
            if weight == 0.0 {
                continue;
            }
            let offset = self.coordinate_offset(indices);
            visit(offset, weight);
            if weight == 1.0 {
                break;
            }
        }
    }

    fn coordinate_offset(&self, indices: &[usize]) -> usize {
        let mut point = 0;
        for table_axis in (0..self.axes.len()).rev() {
            point = point * self.lengths[table_axis] + indices[table_axis];
        }
        point * self.axes.len()
    }

    /// The interval of a one-axis array that holds `target`: the first one in array
    /// order, as wcslib's `tabx2s` scan takes it, which the standard asks of an array
    /// that need not be monotonic. A binary search in each strictly monotonic run, in
    /// order, finds it: a run holds the target once, the lower of two intervals at a
    /// node. Past the ends, the half-interval extrapolations.
    fn locate_one_dimensional(&self, runs: &[MonotonicRun], target: f64) -> Result<Interval> {
        if !target.is_finite() {
            return Err(domain(self.axes[0]));
        }
        let coordinates = &self.coordinates;
        if target == coordinates[0] {
            return Ok(Interval {
                position: 0,
                fraction: 0.0,
            });
        }
        for run in runs {
            let nodes = &coordinates[run.start..=run.end];
            let (first, last) = (nodes[0], nodes[nodes.len() - 1]);
            let below = match run.direction {
                IndexDirection::Increasing if (first..=last).contains(&target) => {
                    nodes.partition_point(|&value| value < target)
                }
                IndexDirection::Decreasing if (last..=first).contains(&target) => {
                    nodes.partition_point(|&value| value > target)
                }
                IndexDirection::Increasing | IndexDirection::Decreasing => continue,
            };
            let position = run.start + below.saturating_sub(1);
            let (first, second) = (coordinates[position], coordinates[position + 1]);
            return Ok(Interval {
                position,
                fraction: (target - first) / (second - first),
            });
        }
        if self.lengths[0] > 1 {
            for position in [0, self.lengths[0] - 2] {
                let first = coordinates[position];
                let second = coordinates[position + 1];
                if first == second {
                    continue;
                }
                let fraction = (target - first) / (second - first);
                let allowed = if position == 0 { -0.5..=0.0 } else { 1.0..=1.5 };
                if allowed.contains(&fraction) {
                    return Ok(Interval { position, fraction });
                }
            }
        }
        Err(domain(self.axes[0]))
    }

    /// Replace the world coordinates of this transform's axes in `coordinates` with
    /// their intermediate coordinates: the first cell in array order whose sub-voxel
    /// search holds them, past each cell whose `bounds` cannot.
    fn invert_multidimensional(
        &self,
        cell_lengths: &[usize],
        bounds: &[f64],
        coordinates: &mut [f64],
    ) -> Result<()> {
        let dimensions = self.axes.len();
        let node_work = inverse_node_work(dimensions, self.vertex_count);
        let mut buffers = SearchBuffers::new();
        let mut search = buffers.search(self, node_work);
        for (target, &image_axis) in search.target.iter_mut().zip(&self.axes) {
            *target = coordinates[image_axis];
        }
        if search.target.iter().any(|value| !value.is_finite()) {
            return Err(domain(self.axes[0]));
        }
        for (cell, cell_bounds) in bounds.chunks_exact(3 * dimensions).enumerate() {
            // The search's first level, with its corners known: no corner within
            // tolerance of the target, and none on its other side.
            let outside = cell_bounds
                .as_chunks::<3>()
                .0
                .iter()
                .zip(search.target.iter())
                .any(|(&[min, max, tolerance], &target)| {
                    max - target < -tolerance || min - target > tolerance
                });
            if outside {
                continue;
            }
            search.enter_cell(cell, cell_lengths);
            if search.locate(0)? {
                for table_axis in 0..dimensions {
                    let interval = Interval {
                        position: search.base[table_axis],
                        fraction: search.solution[table_axis],
                    };
                    coordinates[self.axes[table_axis]] = self.intermediate(table_axis, interval)?;
                }
                return Ok(());
            }
        }
        Err(domain(self.axes[0]))
    }
}

/// One array of a `-TAB` table's first row, with its `TDIMn` shape.
#[derive(Debug)]
pub(crate) struct TableArray {
    pub(crate) shape: Vec<usize>,
    pub(crate) values: Vec<f64>,
}

/// The first-row arrays a `-TAB` transform reads from its table: the coordinate array,
/// and each table axis's index vector, `None` where its axis declares none.
#[derive(Debug)]
pub(crate) struct TabularArrays {
    pub(crate) coordinates: TableArray,
    pub(crate) indices: Vec<Option<TableArray>>,
}

/// Where a point lies in the table: each table axis's interval below it and its fraction
/// past the interval's start, in the first entries, one a table axis.
#[derive(Debug)]
struct TabularLocation {
    base: [usize; MAX_TABLE_AXES],
    delta: [f64; MAX_TABLE_AXES],
}

/// The state of one point's sub-voxel search in three buffers, which
/// [`SearchBuffers::search`] splits into its parts: per table axis the target, the
/// cell's box start and extent, the solution, the box offset and the rounding
/// tolerance, then the corners; the cell base, the voxel and a corner's index; and
/// whether a corner lies below, above or on the target.
#[derive(Debug)]
struct SearchBuffers {
    reals: [f64; MAX_SEARCH_REALS],
    counts: [usize; 3 * MAX_TABLE_AXES],
    flags: [bool; 3 * MAX_TABLE_AXES],
}

impl SearchBuffers {
    const fn new() -> Self {
        Self {
            reals: [0.0; MAX_SEARCH_REALS],
            counts: [0; 3 * MAX_TABLE_AXES],
            flags: [false; 3 * MAX_TABLE_AXES],
        }
    }

    /// The search of `transform`'s cells, each of whose nodes costs `node_work` of its
    /// budget.
    fn search<'a>(
        &'a mut self,
        transform: &'a TabularTransform,
        node_work: usize,
    ) -> SubvoxelSearch<'a> {
        let dimensions = transform.axes.len();
        let reals = &mut self.reals[..dimensions * transform.vertex_count + 6 * dimensions];
        let (target, reals) = reals.split_at_mut(dimensions);
        let (start, reals) = reals.split_at_mut(dimensions);
        let (extent, reals) = reals.split_at_mut(dimensions);
        let (solution, reals) = reals.split_at_mut(dimensions);
        let (delta, reals) = reals.split_at_mut(dimensions);
        let (tolerance, corners) = reals.split_at_mut(dimensions);
        let (base, counts) = self.counts[..3 * dimensions].split_at_mut(dimensions);
        let (voxel, indices) = counts.split_at_mut(dimensions);
        let (lower, flags) = self.flags[..3 * dimensions].split_at_mut(dimensions);
        let (upper, equal) = flags.split_at_mut(dimensions);
        SubvoxelSearch {
            transform,
            target,
            start,
            extent,
            solution,
            delta,
            tolerance,
            corners,
            base,
            voxel,
            indices,
            lower,
            upper,
            equal,
            node_work,
            remaining_work: MAX_INVERSE_WORK,
        }
    }
}

#[derive(Debug)]
struct SubvoxelSearch<'a> {
    transform: &'a TabularTransform,
    target: &'a mut [f64],
    start: &'a mut [f64],
    extent: &'a mut [f64],
    solution: &'a mut [f64],
    delta: &'a mut [f64],
    /// Per coordinate axis, how far an interpolated corner may sit from the
    /// exact value through rounding alone — see [`SubvoxelSearch::fill_corners`].
    tolerance: &'a mut [f64],
    corners: &'a mut [f64],
    base: &'a mut [usize],
    voxel: &'a mut [usize],
    indices: &'a mut [usize],
    lower: &'a mut [bool],
    upper: &'a mut [bool],
    equal: &'a mut [bool],
    node_work: usize,
    remaining_work: usize,
}

impl SubvoxelSearch<'_> {
    /// Start on cell `cell` of the grid of `cell_lengths`, in array order: its base node
    /// and the box it searches, half a cell past each end of the array.
    fn enter_cell(&mut self, cell: usize, cell_lengths: &[usize]) {
        let mut remainder = cell;
        for (table_axis, &cell_length) in cell_lengths.iter().enumerate() {
            let position = remainder % cell_length;
            remainder /= cell_length;
            let length = self.transform.lengths[table_axis];
            let first = if position == 0 { -0.5 } else { 0.0 };
            let last = if length == 1 || position == length - 2 {
                1.5
            } else {
                1.0
            };
            self.base[table_axis] = position;
            self.start[table_axis] = first;
            self.extent[table_axis] = if length == 1 { 1.0 } else { last - first };
        }
        self.voxel.fill(0);
    }

    fn locate(&mut self, level: usize) -> Result<bool> {
        let dimensions = self.transform.axes.len();
        let size = 2.0f64.powi(-i32::try_from(level).expect("the search depth fits i32"));
        if self.node_work > self.remaining_work {
            return Err(FitsError::WcsNoConvergence { algorithm: "TAB" });
        }
        self.remaining_work -= self.node_work;
        self.fill_corners(level, size);
        self.lower.fill(false);
        self.upper.fill(false);
        self.equal.fill(false);
        for vertex in 0..self.transform.vertex_count {
            let mut exact = true;
            for table_axis in 0..dimensions {
                let difference =
                    self.corners[vertex * dimensions + table_axis] - self.target[table_axis];
                if difference.abs() <= self.tolerance[table_axis] {
                    self.equal[table_axis] = true;
                } else {
                    exact = false;
                    if difference < 0.0 {
                        self.lower[table_axis] = true;
                    } else {
                        self.upper[table_axis] = true;
                    }
                }
            }
            if exact {
                self.solution.copy_from_slice(self.delta);
                for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
                    if vertex & (1 << bit) != 0 {
                        self.solution[table_axis] += size * self.extent[table_axis];
                    }
                }
                return Ok(true);
            }
        }
        let possible =
            (0..dimensions).all(|axis| self.equal[axis] || (self.lower[axis] && self.upper[axis]));
        if !possible {
            return Ok(false);
        }
        if level == 31 {
            let half = size / 2.0;
            for table_axis in 0..dimensions {
                self.solution[table_axis] = self.start[table_axis]
                    + half * self.extent[table_axis] * (2 * self.voxel[table_axis] + 1) as f64;
            }
            return Ok(true);
        }
        for subdivision in 0..self.transform.vertex_count {
            for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
                let parent = if level == 0 {
                    0
                } else {
                    2 * self.voxel[table_axis]
                };
                self.voxel[table_axis] = parent + usize::from(subdivision & (1 << bit) != 0);
            }
            if self.locate(level + 1)? {
                return Ok(true);
            }
            for &table_axis in &self.transform.variable_axes {
                self.voxel[table_axis] = if level == 0 {
                    0
                } else {
                    self.voxel[table_axis] / 2
                };
            }
        }
        Ok(false)
    }

    /// The corners of the current voxel at `level`, of side `size` of the cell's box,
    /// and their rounding tolerance.
    fn fill_corners(&mut self, level: usize, size: f64) {
        let dimensions = self.transform.axes.len();
        for table_axis in 0..dimensions {
            self.delta[table_axis] = if level == 0 {
                self.start[table_axis]
            } else {
                self.start[table_axis]
                    + size * self.extent[table_axis] * self.voxel[table_axis] as f64
            };
        }
        for vertex in 0..self.transform.vertex_count {
            self.indices.copy_from_slice(self.base);
            for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
                if vertex & (1 << bit) != 0 {
                    self.indices[table_axis] += 1;
                }
            }
            let source = self.transform.coordinate_offset(self.indices);
            let destination = vertex * dimensions;
            self.corners[destination..destination + dimensions]
                .copy_from_slice(&self.transform.coordinates[source..source + dimensions]);
        }
        // A corner is M linear interpolations of the voxel's coordinates, one per
        // variable axis, at offsets in [−0.5, 1.5]. Each pass rounds once per term and
        // at most doubles the magnitude (|1 − δ| + |δ| ≤ 2), so a corner's rounding
        // error is at most 3·M·2^M·ε/2 of the largest coordinate. A world value the
        // forward transform interpolated carries the same bound, hence twice that.
        // Relative to the coordinates, the test means the same in metres as in Hz.
        let passes = self.transform.variable_axes.len() as f64;
        let rounding = 3.0 * passes * self.transform.vertex_count as f64 * f64::EPSILON;
        for (table_axis, tolerance) in self.tolerance.iter_mut().enumerate() {
            let largest = (0..self.transform.vertex_count)
                .map(|vertex| self.corners[vertex * dimensions + table_axis].abs())
                .fold(0.0, f64::max);
            *tolerance = rounding * largest;
        }
        for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
            let lower = self.delta[table_axis];
            let upper = lower + size * self.extent[table_axis];
            let stride = 1usize << bit;
            let block = 2 * stride;
            for block_start in (0..self.transform.vertex_count).step_by(block) {
                for offset in 0..stride {
                    let lower_vertex = block_start + offset;
                    let upper_vertex = lower_vertex + stride;
                    for coordinate_axis in 0..dimensions {
                        let lower_index = lower_vertex * dimensions + coordinate_axis;
                        let upper_index = upper_vertex * dimensions + coordinate_axis;
                        let first = self.corners[lower_index];
                        let second = self.corners[upper_index];
                        self.corners[lower_index] = first * (1.0 - lower) + second * lower;
                        self.corners[upper_index] = first * (1.0 - upper) + second * upper;
                    }
                }
            }
        }
    }
}

const fn inverse_node_work(dimensions: usize, vertex_count: usize) -> usize {
    dimensions * dimensions * vertex_count
}

fn validate_index(index: &[f64]) -> Result<IndexDirection> {
    if index.iter().any(|value| !value.is_finite()) {
        return Err(invalid(TabularDefect::NonFiniteIndex));
    }
    if index.len() < 2 {
        return Ok(IndexDirection::Increasing);
    }
    let mut direction = 0i8;
    for pair in index.windows(2) {
        let comparison = if pair[0] < pair[1] {
            1
        } else if pair[0] > pair[1] {
            -1
        } else {
            0
        };
        if comparison != 0 {
            if direction != 0 && direction != comparison {
                return Err(invalid(TabularDefect::NonMonotonicIndex));
            }
            direction = comparison;
        }
    }
    if direction == 0 {
        return Err(invalid(TabularDefect::ConstantIndex));
    }
    Ok(if direction > 0 {
        IndexDirection::Increasing
    } else {
        IndexDirection::Decreasing
    })
}

fn integer_parameter(
    header: &Header,
    axis: usize,
    parameter: usize,
    alt: Alternate,
    default: i64,
) -> Result<i64> {
    let key = Keyword::fitting(format_args!("PV{}_{parameter}{alt}", axis + 1));
    match header_read::integer(header, key)? {
        None => Ok(default),
        Some(value) if value >= 1 => Ok(value),
        Some(_) => Err(invalid(TabularDefect::NonPositiveParameter)),
    }
}

fn required_text(header: &Header, keyword: Keyword, defect: TabularDefect) -> Result<String> {
    header
        .get_text(keyword)?
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid(defect))
}

const fn invalid(defect: TabularDefect) -> FitsError {
    FitsError::InvalidWcs {
        defect: WcsDefect::Tabular(defect),
    }
}

const fn domain(axis: usize) -> FitsError {
    FitsError::WcsCoordinateOutOfDomain {
        axis,
        algorithm: "TAB",
    }
}

/// `-TAB` fixtures for the tabular, reader and WCS bench code.
#[cfg(feature = "internals")]
pub(crate) mod internals {
    use crate::alternate::Alternate;
    use crate::bintable::table_view::TableView;
    use crate::fits_error::Result;
    use crate::header_model::{Header, keywords};
    use crate::reader;
    use crate::world_coordinates::tabular::TabularDescriptor;
    use std::sync::OnceLock;

    use crate::bintable::Table;
    use crate::internals::raw_table::RawTable;
    use crate::internals::samples;
    use crate::world_coordinates::Wcs;
    use crate::world_coordinates::tabular;
    use crate::world_coordinates::tabular::TabularTransform;

    /// A 2×2 grid of two-axis coordinates, `(2,2,2)`: vertex (i, j) is
    /// (100 + 10i, 200 + 20j), so the coupled transform it defines is affine.
    pub(crate) const COUPLED_GRID: [f64; 8] =
        [100.0, 200.0, 110.0, 200.0, 100.0, 220.0, 110.0, 220.0];

    /// A one-row lookup table: each column (`TTYPEn`, values, optional `TDIMn`) is `nD`
    /// doubles.
    fn lookup_raw(columns: &[(&str, &[f64], Option<&str>)]) -> RawTable {
        columns
            .iter()
            .fold(RawTable::new(1), |raw, &(name, values, _)| {
                let tform = format!("{}D", values.len());
                raw.fixed(
                    Some(name),
                    &tform,
                    &[&samples::be(values, f64::to_be_bytes)],
                )
            })
    }

    /// The header of [`lookup_raw`], with each column's `TDIMn`.
    pub(crate) fn lookup_header(columns: &[(&str, &[f64], Option<&str>)]) -> Header {
        let mut header = lookup_raw(columns).header();
        for (index, &(_, _, shape)) in columns.iter().enumerate() {
            if let Some(shape) = shape {
                header.set_internal(format!("TDIM{}", index + 1).as_str(), shape);
            }
        }
        header
    }

    /// The row [`lookup_header`] describes.
    pub(crate) fn lookup_bytes(columns: &[(&str, &[f64], Option<&str>)]) -> Vec<u8> {
        lookup_raw(columns).data()
    }

    pub(crate) fn lookup_table(columns: &[(&str, &[f64], Option<&str>)]) -> Table {
        Table::from_data(&lookup_header(columns), lookup_bytes(columns)).unwrap()
    }

    /// An image header of `axis_count` `-TAB` axes, axis i reading table axis i of
    /// the `COORD` column in `WCS-TABLE`.
    pub(crate) fn tab_header(axis_count: usize) -> Header {
        let mut header = Header::new();
        header.set_internal(keywords::NAXIS, i64::try_from(axis_count).unwrap());
        for axis in 1..=axis_count {
            header.set_tab_axis(axis, "COORD", axis);
        }
        header
    }

    impl Header {
        /// Make image axis `axis` a `-TAB` axis that reads table axis `table_axis` of the
        /// `column` column in `WCS-TABLE`, with unit pixels and no offsets.
        pub(crate) fn set_tab_axis(&mut self, axis: usize, column: &str, table_axis: usize) {
            self.set_internal(format!("CTYPE{axis}").as_str(), format!("AX{axis:02}-TAB"))
                .set_internal(format!("CRPIX{axis}").as_str(), 0.0)
                .set_internal(format!("CRVAL{axis}").as_str(), 0.0)
                .set_internal(format!("CDELT{axis}").as_str(), 1.0)
                .set_internal(format!("PS{axis}_0").as_str(), "WCS-TABLE")
                .set_internal(format!("PS{axis}_1").as_str(), column)
                .set_internal(
                    format!("PV{axis}_3").as_str(),
                    i64::try_from(table_axis).unwrap(),
                );
        }
    }

    /// Entries in the [`indexed_wcs`] lookup.
    pub(crate) const INDEX_LENGTH: usize = 100_000;

    /// A `WAVE-TAB` axis over a 100 000-entry monotonic lookup: index value 2i holds
    /// coordinate 0.25i. Built once per process.
    pub(crate) fn indexed_wcs() -> &'static Wcs {
        static WCS: OnceLock<Wcs> = OnceLock::new();
        WCS.get_or_init(|| {
            let coordinates: Vec<f64> = (0..INDEX_LENGTH).map(|i| i as f64 * 0.25).collect();
            let index: Vec<f64> = (0..INDEX_LENGTH).map(|i| i as f64 * 2.0).collect();
            let shape = format!("(1,{INDEX_LENGTH})");
            let table = lookup_table(&[
                ("COORD", &coordinates, Some(shape.as_str())),
                ("INDEX", &index, None),
            ]);
            let mut header = tab_header(1);
            header
                .set_internal("CTYPE1", "WAVE-TAB")
                .set_internal("CUNIT1", "m")
                .set_internal("PS1_2", "INDEX");
            resolved_wcs(&header, &table)
        })
    }

    /// The two coupled `-TAB` axes over [`COUPLED_GRID`]. Built once per process.
    pub(crate) fn coupled_wcs() -> &'static Wcs {
        static WCS: OnceLock<Wcs> = OnceLock::new();
        WCS.get_or_init(|| {
            let table = lookup_table(&[("COORD", &COUPLED_GRID, Some("(2,2,2)"))]);
            resolved_wcs(&tab_header(2), &table)
        })
    }

    /// One pixel through the indexed lookup.
    pub fn tabular_forward_at_pixel(pixel: f64) -> f64 {
        let mut world = [0.0];
        indexed_wcs().pixel_to_world(&[pixel], &mut world).unwrap();
        world[0]
    }

    /// One world coordinate back through the indexed lookup.
    pub fn tabular_inverse_at_world(world: f64) -> f64 {
        let mut pixel = [0.0];
        indexed_wcs().world_to_pixel(&[world], &mut pixel).unwrap();
        pixel[0]
    }

    /// The pixel point at `fraction` along the diagonal of the coupled grid, to world.
    pub fn tabular_forward_at_fraction(fraction: f64) -> f64 {
        let pixel = [0.5 + fraction, 0.5 + fraction];
        let mut world = [0.0; 2];
        coupled_wcs().pixel_to_world(&pixel, &mut world).unwrap();
        world.into_iter().sum()
    }

    /// The world point at `fraction` along the diagonal of the coupled grid, inverted;
    /// a dyadic fraction sets how deep the sub-voxel search goes.
    pub fn tabular_inverse_at_fraction(fraction: f64) -> f64 {
        let world = [100.0 + 10.0 * fraction, 200.0 + 20.0 * fraction];
        let mut pixel = [0.0; 2];
        coupled_wcs().world_to_pixel(&world, &mut pixel).unwrap();
        pixel.into_iter().sum()
    }

    /// The transform `descriptor` declares over the arrays the reader reads from `table`.
    pub(crate) fn from_table(
        descriptor: &TabularDescriptor,
        table: TableView<'_>,
    ) -> Result<TabularTransform> {
        TabularTransform::from_arrays(descriptor, reader::tabular_arrays(descriptor, table)?)
    }

    /// The transform `header` declares, with every `-TAB` axis read from `table`.
    pub(crate) fn resolved_wcs(header: &Header, table: &Table) -> Wcs {
        let axis_count =
            usize::try_from(header.get_integer(keywords::NAXIS).unwrap().unwrap()).unwrap();
        let transforms = tabular::descriptors(header, axis_count, Alternate::PRIMARY)
            .unwrap()
            .into_iter()
            .map(|descriptor| from_table(&descriptor, table.view()).unwrap())
            .collect();
        Wcs::from_header_with_tabular(header, Alternate::PRIMARY, transforms).unwrap()
    }
}

#[cfg(test)]
mod tests;
