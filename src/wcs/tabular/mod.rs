use crate::allocation;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::AltSuffix;
use crate::keyword::key;
use crate::table_impl::BinTable;
use crate::table_impl::ColumnReader;
use crate::table_impl::TformKind;
use crate::wcs::axis;
use crate::wcs::celestial_axis;
use crate::wcs::unit_to_degrees;

const TABULAR_TOLERANCE: f64 = 1e-10;
const MAX_INTERPOLATION_VERTICES: usize = 1 << 20;
const MAX_INVERSE_WORK: usize = 1 << 22;

#[derive(Debug, Clone)]
pub(crate) struct TabularReference {
    pub(crate) extension_name: String,
    pub(crate) extension_version: i64,
    pub(crate) extension_level: i64,
    pub(crate) coordinate_column: String,
}

impl TabularReference {
    pub(crate) fn identifies_same_extension(&self, other: &TabularReference) -> bool {
        self.extension_name
            .eq_ignore_ascii_case(&other.extension_name)
            && self.extension_version == other.extension_version
            && self.extension_level == other.extension_level
    }

    fn identifies_same_array(&self, other: &TabularReference) -> bool {
        self.identifies_same_extension(other)
            && self
                .coordinate_column
                .eq_ignore_ascii_case(&other.coordinate_column)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TabularDescriptor {
    pub(crate) reference: TabularReference,
    axes: Vec<usize>,
    reference_indices: Vec<f64>,
    index_columns: Vec<Option<String>>,
    world_scales: Vec<f64>,
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
    alt: Option<char>,
) -> Result<Vec<TabularDescriptor>> {
    let suffix = AltSuffix::new(alt);
    let mut descriptors = Vec::<TabularDescriptor>::new();
    for axis in 0..axis_count {
        let ctype = header
            .get_text(key!("CTYPE{}{suffix}", axis + 1).as_str())?
            .unwrap_or("");
        if !ctype.ends_with("-TAB") {
            continue;
        }
        let extension_name = required_text(
            header,
            key!("PS{}_0{suffix}", axis + 1).as_str(),
            "TAB requires PSi_0",
        )?;
        let coordinate_column = required_text(
            header,
            key!("PS{}_1{suffix}", axis + 1).as_str(),
            "TAB requires PSi_1",
        )?;
        let reference = TabularReference {
            extension_name,
            extension_version: integer_parameter(header, axis, 1, suffix, 1)?,
            extension_level: integer_parameter(header, axis, 2, suffix, 1)?,
            coordinate_column,
        };
        let table_axis = integer_parameter(header, axis, 3, suffix, 1)?;
        let table_axis = usize::try_from(table_axis)
            .ok()
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| invalid("PVi_3 must be a positive table-axis number"))?;
        if table_axis >= axis_count {
            return Err(invalid("PVi_3 exceeds the WCS axis count"));
        }
        let reference_index = header
            .get_real(key!("CRVAL{}{suffix}", axis + 1).as_str())?
            .unwrap_or(0.0);
        if !reference_index.is_finite() {
            return Err(invalid("TAB CRVAL must be finite"));
        }
        let index_column = header
            .get_text(key!("PS{}_2{suffix}", axis + 1).as_str())?
            .map(str::to_string);

        let descriptor = match descriptors
            .iter_mut()
            .find(|descriptor| descriptor.reference.identifies_same_array(&reference))
        {
            Some(descriptor) => descriptor,
            None => {
                descriptors.push(TabularDescriptor {
                    reference,
                    axes: Vec::new(),
                    reference_indices: Vec::new(),
                    index_columns: Vec::new(),
                    world_scales: Vec::new(),
                });
                descriptors.last_mut().unwrap()
            }
        };
        let required_len = table_axis
            .checked_add(1)
            .ok_or_else(|| invalid("TAB table-axis number is too large"))?;
        descriptor.axes.resize(required_len, usize::MAX);
        descriptor.reference_indices.resize(required_len, 0.0);
        descriptor.index_columns.resize(required_len, None);
        descriptor.world_scales.resize(required_len, 1.0);
        if descriptor.axes[table_axis] != usize::MAX {
            return Err(invalid("TAB table-axis mapping is duplicated"));
        }
        descriptor.axes[table_axis] = axis;
        descriptor.reference_indices[table_axis] = reference_index;
        descriptor.index_columns[table_axis] = index_column;
        let cunit = header
            .get_text(key!("CUNIT{}{suffix}", axis + 1).as_str())?
            .unwrap_or("");
        descriptor.world_scales[table_axis] =
            if let Some(scale) = axis::spectral_unit_scale(ctype, cunit)? {
                scale
            } else if celestial_axis(ctype).is_some() {
                unit_to_degrees(cunit)
            } else {
                1.0
            };
    }
    for descriptor in &descriptors {
        if descriptor.axes.contains(&usize::MAX) {
            return Err(invalid("TAB table-axis mapping is incomplete"));
        }
    }
    Ok(descriptors)
}

impl TabularDescriptor {
    pub(crate) fn referenced_columns(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.reference.coordinate_column.as_str()).chain(
            self.index_columns
                .iter()
                .filter_map(|column| column.as_deref()),
        )
    }
}

impl TabularTransform {
    pub(crate) fn from_table(
        descriptor: TabularDescriptor,
        table: &BinTable,
    ) -> Result<TabularTransform> {
        let metadata = table.metadata();
        let dimensions = descriptor.axes.len();
        let coordinate = table.column_by_name(&descriptor.reference.coordinate_column)?;
        let coordinate_shape = first_row_shape_and_values(metadata.nrows, coordinate)?;
        let mut shape = coordinate_shape.shape;
        if dimensions == 1 && shape.len() == 1 {
            shape.insert(0, 1);
        }
        if shape.len() != dimensions + 1 || shape[0] != dimensions {
            return Err(invalid("TAB coordinate TDIM must be (M,K1,...,KM)"));
        }
        let lengths = shape[1..].to_vec();
        if lengths.contains(&0) {
            return Err(invalid("TAB coordinate axes must be non-empty"));
        }
        let variable_axes: Vec<usize> = lengths
            .iter()
            .enumerate()
            .filter_map(|(axis, &length)| (length > 1).then_some(axis))
            .collect();
        let vertex_count = interpolation_vertex_count(variable_axes.len())?;
        let coordinate_count = lengths
            .iter()
            .try_fold(dimensions, |count, &length| count.checked_mul(length))
            .ok_or_else(|| invalid("TAB coordinate array is too large"))?;
        if coordinate_shape.values.len() != coordinate_count {
            return Err(invalid(
                "TAB coordinate TDIM does not match its first-row element count",
            ));
        }
        if coordinate_shape
            .values
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(invalid("TAB coordinate array must contain finite values"));
        }
        let mut coordinates = coordinate_shape.values;
        for point in coordinates.chunks_exact_mut(dimensions) {
            for (table_axis, value) in point.iter_mut().enumerate() {
                *value *= descriptor.world_scales[table_axis];
            }
        }

        let mut indices = Vec::with_capacity(dimensions);
        for (table_axis, column) in descriptor.index_columns.iter().enumerate() {
            let Some(column) = column else {
                indices.push(None);
                continue;
            };
            let reader = table.column_by_name(column)?;
            let index = first_row_shape_and_values(metadata.nrows, reader)?;
            if index.shape.len() > 1 || index.values.len() != lengths[table_axis] {
                return Err(invalid(
                    "TAB index-vector length must match its coordinate axis",
                ));
            }
            let direction = validate_index(&index.values)?;
            indices.push(Some(IndexVector {
                values: index.values,
                direction,
            }));
        }
        Ok(TabularTransform {
            axes: descriptor.axes,
            reference_indices: descriptor.reference_indices,
            lengths,
            variable_axes,
            vertex_count,
            indices,
            coordinates,
        })
    }

    pub(super) fn to_world(&self, intermediate: &[f64], world: &mut [f64]) -> Result<()> {
        let mut location = self.interpolation_location(|_, image_axis| intermediate[image_axis])?;
        for &image_axis in &self.axes {
            world[image_axis] = 0.0;
        }
        self.for_each_interpolation_vertex(
            &mut location.base,
            &location.delta,
            |offset, weight| {
                for (table_axis, &image_axis) in self.axes.iter().enumerate() {
                    world[image_axis] += self.coordinates[offset + table_axis] * weight;
                }
            },
        );
        Ok(())
    }

    pub(super) fn to_world_axis(&self, image_axis: usize, intermediate: &[f64]) -> Result<f64> {
        debug_assert_eq!(intermediate.len(), self.axes.len());
        let table_axis = self
            .axes
            .iter()
            .position(|&axis| axis == image_axis)
            .expect("requested TAB axis belongs to the transform");
        let mut location = self.interpolation_location(|table_axis, _| intermediate[table_axis])?;
        Ok(self.interpolate_axis(&mut location.base, &location.delta, table_axis))
    }

    pub(super) fn to_intermediate(&self, world: &[f64], intermediate: &mut [f64]) -> Result<()> {
        let location = if self.axes.len() == 1 {
            self.locate_one_dimensional(world[self.axes[0]])?
        } else {
            let target: Vec<f64> = self.axes.iter().map(|&axis| world[axis]).collect();
            self.locate_multidimensional(&target)?
        };
        for table_axis in 0..self.axes.len() {
            let upsilon = location.base[table_axis] as f64 + 1.0 + location.delta[table_axis];
            if !(0.5..=self.lengths[table_axis] as f64 + 0.5).contains(&upsilon) {
                return Err(domain(self.axes[table_axis]));
            }
            let psi = self.array_to_index(table_axis, upsilon);
            intermediate[self.axes[table_axis]] = psi - self.reference_indices[table_axis];
        }
        Ok(())
    }

    fn interpolation_location(
        &self,
        mut intermediate: impl FnMut(usize, usize) -> f64,
    ) -> Result<TabularLocation> {
        let mut base = Vec::with_capacity(self.axes.len());
        let mut delta = Vec::with_capacity(self.axes.len());
        for table_axis in 0..self.axes.len() {
            let image_axis = self.axes[table_axis];
            let psi = intermediate(table_axis, image_axis) + self.reference_indices[table_axis];
            let upsilon = self.index_to_array(table_axis, psi)?;
            let length = self.lengths[table_axis];
            if !(0.5..=length as f64 + 0.5).contains(&upsilon) {
                return Err(domain(image_axis));
            }
            let one_relative = upsilon.floor() as isize;
            let mut zero_relative = one_relative - 1;
            let mut fraction = upsilon - one_relative as f64;
            if one_relative == 0 {
                zero_relative += 1;
                fraction -= 1.0;
            } else if one_relative == length as isize && length > 1 {
                zero_relative -= 1;
                fraction += 1.0;
            }
            base.push(usize::try_from(zero_relative).expect("TAB base index"));
            delta.push(fraction);
        }
        Ok(TabularLocation { base, delta })
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
            if (values[0] - 0.5..=values[0] + 0.5).contains(&psi) {
                return Ok(psi);
            }
            return Err(domain(self.axes[table_axis]));
        }
        let first_step = (values[1] - values[0]).abs();
        let last = values.len() - 1;
        let last_step = (values[last] - values[last - 1]).abs();
        match index.direction {
            IndexDirection::Increasing => {
                if psi < values[0] && psi < values[0] - 0.5 * first_step {
                    return Err(domain(self.axes[table_axis]));
                }
                if psi > values[last] && psi > values[last] + 0.5 * last_step {
                    return Err(domain(self.axes[table_axis]));
                }
            }
            IndexDirection::Decreasing => {
                if psi > values[0] && psi > values[0] + 0.5 * first_step {
                    return Err(domain(self.axes[table_axis]));
                }
                if psi < values[last] && psi < values[last] - 0.5 * last_step {
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

    fn array_to_index(&self, table_axis: usize, upsilon: f64) -> f64 {
        let Some(index) = &self.indices[table_axis] else {
            return upsilon;
        };
        let values = &index.values;
        if values.len() == 1 {
            return values[0];
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

    fn interpolate_axis(&self, indices: &mut [usize], delta: &[f64], table_axis: usize) -> f64 {
        let mut result = 0.0;
        self.for_each_interpolation_vertex(indices, delta, |offset, weight| {
            result += self.coordinates[offset + table_axis] * weight;
        });
        result
    }

    fn coordinate_offset(&self, indices: &[usize]) -> usize {
        let mut point = 0;
        for table_axis in (0..self.axes.len()).rev() {
            point = point * self.lengths[table_axis] + indices[table_axis];
        }
        point * self.axes.len()
    }

    fn locate_one_dimensional(&self, target: f64) -> Result<TabularLocation> {
        if !target.is_finite() {
            return Err(domain(self.axes[0]));
        }
        if target == self.coordinates[0] {
            return Ok(TabularLocation {
                base: vec![0],
                delta: vec![0.0],
            });
        }
        for position in 0..self.lengths[0].saturating_sub(1) {
            let first = self.coordinates[position];
            let second = self.coordinates[position + 1];
            let usable_index = self.indices[0]
                .as_ref()
                .is_none_or(|index| index.values[position] != index.values[position + 1]);
            if usable_index
                && first != second
                && (first.min(second)..=first.max(second)).contains(&target)
            {
                return Ok(TabularLocation {
                    base: vec![position],
                    delta: vec![(target - first) / (second - first)],
                });
            }
        }
        if self.lengths[0] > 1 {
            for position in [0, self.lengths[0] - 2] {
                let first = self.coordinates[position];
                let second = self.coordinates[position + 1];
                if first == second {
                    continue;
                }
                let fraction = (target - first) / (second - first);
                let allowed = if position == 0 { -0.5..=0.0 } else { 1.0..=1.5 };
                if allowed.contains(&fraction) {
                    return Ok(TabularLocation {
                        base: vec![position],
                        delta: vec![fraction],
                    });
                }
            }
        }
        Err(domain(self.axes[0]))
    }

    fn locate_multidimensional(&self, target: &[f64]) -> Result<TabularLocation> {
        if target.iter().any(|value| !value.is_finite()) {
            return Err(domain(self.axes[0]));
        }
        let cell_lengths: Vec<usize> = self
            .lengths
            .iter()
            .map(|&length| length.saturating_sub(1).max(1))
            .collect();
        let cell_count = cell_lengths
            .iter()
            .try_fold(1usize, |count, &length| count.checked_mul(length))
            .ok_or_else(|| invalid("TAB coordinate array is too large"))?;
        let dimensions = self.axes.len();
        let mut base = vec![0; dimensions];
        let mut start = vec![0.0; dimensions];
        let mut extent = vec![0.0; dimensions];
        let mut solution = vec![0.0; dimensions];
        let mut scratch = TabularSearchScratch::new(dimensions, self.vertex_count)?;
        for cell in 0..cell_count {
            let mut remainder = cell;
            for (position, &length) in base.iter_mut().zip(&cell_lengths) {
                *position = remainder % length;
                remainder /= length;
            }
            for (table_axis, &position) in base.iter().enumerate() {
                let length = self.lengths[table_axis];
                let first = if position == 0 { -0.5 } else { 0.0 };
                let last = if length == 1 || position == length - 2 {
                    1.5
                } else {
                    1.0
                };
                start[table_axis] = first;
                extent[table_axis] = if length == 1 { 1.0 } else { last - first };
            }
            scratch.voxel.fill(0);
            let mut search = SubvoxelSearch {
                transform: self,
                target,
                base: &base,
                start: &start,
                extent: &extent,
                solution: &mut solution,
                scratch: &mut scratch,
            };
            if search.locate(0)? {
                return Ok(TabularLocation {
                    base,
                    delta: solution,
                });
            }
        }
        Err(domain(self.axes[0]))
    }
}

fn interpolation_vertex_count(variable_axes: usize) -> Result<usize> {
    let shift = u32::try_from(variable_axes)
        .map_err(|_| invalid("TAB interpolation dimensionality is too large"))?;
    1usize
        .checked_shl(shift)
        .filter(|&count| count <= MAX_INTERPOLATION_VERTICES)
        .ok_or_else(|| invalid("TAB interpolation dimensionality is too large"))
}

#[derive(Debug)]
struct FirstRow {
    shape: Vec<usize>,
    values: Vec<f64>,
}

#[derive(Debug)]
struct TabularLocation {
    base: Vec<usize>,
    delta: Vec<f64>,
}

#[derive(Debug)]
struct TabularSearchScratch {
    lower: Vec<bool>,
    upper: Vec<bool>,
    equal: Vec<bool>,
    delta: Vec<f64>,
    indices: Vec<usize>,
    corners: Vec<f64>,
    voxel: Vec<usize>,
    node_work: usize,
    remaining_work: usize,
}

impl TabularSearchScratch {
    fn new(dimensions: usize, vertex_count: usize) -> Result<TabularSearchScratch> {
        let node_work = inverse_node_work(dimensions, vertex_count)?;
        let corner_count = dimensions
            .checked_mul(vertex_count)
            .ok_or(FitsError::DataUnitOverflow)?;
        Ok(TabularSearchScratch {
            lower: allocation::try_zeroed(false, dimensions)?,
            upper: allocation::try_zeroed(false, dimensions)?,
            equal: allocation::try_zeroed(false, dimensions)?,
            delta: allocation::try_zeroed(0.0, dimensions)?,
            indices: allocation::try_zeroed(0, dimensions)?,
            corners: allocation::try_zeroed(0.0, corner_count)?,
            voxel: allocation::try_zeroed(0, dimensions)?,
            node_work,
            remaining_work: MAX_INVERSE_WORK,
        })
    }
}

#[derive(Debug)]
struct SubvoxelSearch<'a> {
    transform: &'a TabularTransform,
    target: &'a [f64],
    base: &'a [usize],
    start: &'a [f64],
    extent: &'a [f64],
    solution: &'a mut [f64],
    scratch: &'a mut TabularSearchScratch,
}

impl SubvoxelSearch<'_> {
    fn locate(&mut self, level: usize) -> Result<bool> {
        let dimensions = self.transform.axes.len();
        let size = 2.0f64.powi(-(level as i32));
        self.evaluate_corners(level, size)?;
        self.scratch.lower.fill(false);
        self.scratch.upper.fill(false);
        self.scratch.equal.fill(false);
        for vertex in 0..self.transform.vertex_count {
            let mut exact = true;
            for table_axis in 0..dimensions {
                let difference = self.scratch.corners[vertex * dimensions + table_axis]
                    - self.target[table_axis];
                if difference.abs() < TABULAR_TOLERANCE {
                    self.scratch.equal[table_axis] = true;
                } else {
                    exact = false;
                    if difference < 0.0 {
                        self.scratch.lower[table_axis] = true;
                    } else {
                        self.scratch.upper[table_axis] = true;
                    }
                }
            }
            if exact {
                self.solution.copy_from_slice(&self.scratch.delta);
                for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
                    if vertex & (1 << bit) != 0 {
                        self.solution[table_axis] += size * self.extent[table_axis];
                    }
                }
                return Ok(true);
            }
        }
        let possible = (0..dimensions).all(|axis| {
            (self.scratch.lower[axis] || self.scratch.equal[axis])
                && (self.scratch.upper[axis] || self.scratch.equal[axis])
        });
        if !possible {
            return Ok(false);
        }
        if level == 31 {
            let half = size / 2.0;
            for table_axis in 0..dimensions {
                self.solution[table_axis] = self.start[table_axis]
                    + half
                        * self.extent[table_axis]
                        * (2 * self.scratch.voxel[table_axis] + 1) as f64;
            }
            return Ok(true);
        }
        for subdivision in 0..self.transform.vertex_count {
            for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
                let parent = if level == 0 {
                    0
                } else {
                    2 * self.scratch.voxel[table_axis]
                };
                self.scratch.voxel[table_axis] =
                    parent + usize::from(subdivision & (1 << bit) != 0);
            }
            if self.locate(level + 1)? {
                return Ok(true);
            }
            for &table_axis in &self.transform.variable_axes {
                self.scratch.voxel[table_axis] = if level == 0 {
                    0
                } else {
                    self.scratch.voxel[table_axis] / 2
                };
            }
        }
        Ok(false)
    }

    fn evaluate_corners(&mut self, level: usize, size: f64) -> Result<()> {
        let dimensions = self.transform.axes.len();
        if self.scratch.node_work > self.scratch.remaining_work {
            return Err(FitsError::WcsNoConvergence { algorithm: "TAB" });
        }
        self.scratch.remaining_work -= self.scratch.node_work;

        for table_axis in 0..dimensions {
            self.scratch.delta[table_axis] = if level == 0 {
                self.start[table_axis]
            } else {
                self.start[table_axis]
                    + size * self.extent[table_axis] * self.scratch.voxel[table_axis] as f64
            };
        }
        for vertex in 0..self.transform.vertex_count {
            self.scratch.indices.copy_from_slice(self.base);
            for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
                if vertex & (1 << bit) != 0 {
                    self.scratch.indices[table_axis] += 1;
                }
            }
            let source = self.transform.coordinate_offset(&self.scratch.indices);
            let destination = vertex * dimensions;
            self.scratch.corners[destination..destination + dimensions]
                .copy_from_slice(&self.transform.coordinates[source..source + dimensions]);
        }
        for (bit, &table_axis) in self.transform.variable_axes.iter().enumerate() {
            let lower = self.scratch.delta[table_axis];
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
                        let first = self.scratch.corners[lower_index];
                        let second = self.scratch.corners[upper_index];
                        self.scratch.corners[lower_index] = first * (1.0 - lower) + second * lower;
                        self.scratch.corners[upper_index] = first * (1.0 - upper) + second * upper;
                    }
                }
            }
        }
        Ok(())
    }
}

fn inverse_node_work(dimensions: usize, vertex_count: usize) -> Result<usize> {
    let work = dimensions
        .checked_mul(dimensions)
        .and_then(|work| work.checked_mul(vertex_count))
        .unwrap_or(usize::MAX);
    if work > MAX_INVERSE_WORK {
        return Err(FitsError::WcsNoConvergence { algorithm: "TAB" });
    }
    Ok(work)
}

fn first_row_shape_and_values(row_count: usize, reader: ColumnReader<'_>) -> Result<FirstRow> {
    if row_count == 0 {
        return Err(invalid("TAB lookup table must contain a row"));
    }
    let descriptor = reader.descriptor();
    let variable = matches!(
        descriptor.tform.kind,
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64
    );
    let values = if variable {
        reader
            .vla_physical()?
            .into_iter()
            .next()
            .expect("TAB table has at least one row")
    } else {
        let values = reader.physical()?;
        values[..descriptor.tform.repeat].to_vec()
    };
    let shape = descriptor
        .tdim
        .clone()
        .unwrap_or_else(|| vec![values.len()]);
    Ok(FirstRow { shape, values })
}

fn validate_index(index: &[f64]) -> Result<IndexDirection> {
    if index.iter().any(|value| !value.is_finite()) {
        return Err(invalid("TAB index vectors must contain finite values"));
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
                return Err(invalid("TAB index vectors must be monotonic"));
            }
            direction = comparison;
        }
    }
    if direction == 0 {
        return Err(invalid("TAB index vectors must not be constant"));
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
    suffix: AltSuffix,
    default: i64,
) -> Result<i64> {
    let key = key!("PV{}_{parameter}{suffix}", axis + 1);
    match header.get_real(key.as_str())? {
        None => Ok(default),
        Some(value)
            if value.is_finite()
                && value >= 1.0
                && value.fract() == 0.0
                && value <= i64::MAX as f64 =>
        {
            Ok(value as i64)
        }
        Some(_) => Err(invalid(
            "TAB PVi_1, PVi_2, and PVi_3 must be positive integers",
        )),
    }
}

fn required_text(header: &Header, keyword: &str, detail: &'static str) -> Result<String> {
    header
        .get_text(keyword)?
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid(detail))
}

fn invalid(detail: impl Into<String>) -> FitsError {
    FitsError::InvalidValue {
        card: detail.into(),
    }
}

fn domain(axis: usize) -> FitsError {
    FitsError::WcsCoordinateDomain {
        axis,
        algorithm: "TAB",
    }
}

#[cfg(test)]
mod tests;
