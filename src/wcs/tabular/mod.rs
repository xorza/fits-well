use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table::BinTable;
use crate::table::ColumnReader;
use crate::table::TformKind;
use crate::wcs::axis;
use crate::wcs::celestial_axis;
use crate::wcs::unit_to_degrees;

const TABULAR_TOLERANCE: f64 = 1e-10;

#[derive(Debug, Clone)]
pub(crate) struct TabularReference {
    pub(crate) extension_name: String,
    pub(crate) extension_version: i64,
    pub(crate) extension_level: i64,
    pub(crate) coordinate_column: String,
}

impl TabularReference {
    fn identifies_same_array(&self, other: &TabularReference) -> bool {
        self.extension_name
            .eq_ignore_ascii_case(&other.extension_name)
            && self.extension_version == other.extension_version
            && self.extension_level == other.extension_level
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
    pub(crate) axes: Vec<usize>,
    reference_indices: Vec<f64>,
    lengths: Vec<usize>,
    variable_axes: Vec<usize>,
    indices: Vec<Option<Vec<f64>>>,
    coordinates: Vec<f64>,
}

pub(crate) fn descriptors(
    header: &Header,
    axis_count: usize,
    alt: Option<char>,
) -> Result<Vec<TabularDescriptor>> {
    let suffix = alt.map(|value| value.to_string()).unwrap_or_default();
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
            extension_version: integer_parameter(header, axis, 1, &suffix, 1)?,
            extension_level: integer_parameter(header, axis, 2, &suffix, 1)?,
            coordinate_column,
        };
        let table_axis = integer_parameter(header, axis, 3, &suffix, 1)?;
        let table_axis = usize::try_from(table_axis)
            .ok()
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| invalid("PVi_3 must be a positive table-axis number"))?;
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
            validate_index(&index.values)?;
            indices.push(Some(index.values));
        }
        let variable_axes = lengths
            .iter()
            .enumerate()
            .filter_map(|(axis, &length)| (length > 1).then_some(axis))
            .collect();
        Ok(TabularTransform {
            axes: descriptor.axes,
            reference_indices: descriptor.reference_indices,
            lengths,
            variable_axes,
            indices,
            coordinates,
        })
    }

    pub(crate) fn to_world(&self, intermediate: &[f64], world: &mut [f64]) -> Result<()> {
        let mut base = Vec::with_capacity(self.axes.len());
        let mut delta = Vec::with_capacity(self.axes.len());
        for table_axis in 0..self.axes.len() {
            let image_axis = self.axes[table_axis];
            let psi = intermediate[image_axis] + self.reference_indices[table_axis];
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
        let values = self.interpolate(&base, &delta);
        for (table_axis, value) in values.into_iter().enumerate() {
            world[self.axes[table_axis]] = value;
        }
        Ok(())
    }

    pub(crate) fn to_intermediate(&self, world: &[f64], intermediate: &mut [f64]) -> Result<()> {
        let target: Vec<f64> = self.axes.iter().map(|&axis| world[axis]).collect();
        let location = if self.axes.len() == 1 {
            self.locate_one_dimensional(target[0])?
        } else {
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

    fn index_to_array(&self, table_axis: usize, psi: f64) -> Result<f64> {
        if !psi.is_finite() {
            return Err(domain(self.axes[table_axis]));
        }
        let Some(index) = &self.indices[table_axis] else {
            return Ok(psi);
        };
        if index.len() == 1 {
            if (index[0] - 0.5..=index[0] + 0.5).contains(&psi) {
                return Ok(psi);
            }
            return Err(domain(self.axes[table_axis]));
        }
        let increasing = index[0] < *index.last().unwrap();
        let first_step = (index[1] - index[0]).abs();
        let last = index.len() - 1;
        let last_step = (index[last] - index[last - 1]).abs();
        if increasing {
            if psi < index[0] && psi < index[0] - 0.5 * first_step {
                return Err(domain(self.axes[table_axis]));
            }
            if psi > index[last] && psi > index[last] + 0.5 * last_step {
                return Err(domain(self.axes[table_axis]));
            }
        } else {
            if psi > index[0] && psi > index[0] + 0.5 * first_step {
                return Err(domain(self.axes[table_axis]));
            }
            if psi < index[last] && psi < index[last] - 0.5 * last_step {
                return Err(domain(self.axes[table_axis]));
            }
        }
        let segment = if (increasing && psi < index[0]) || (!increasing && psi > index[0]) {
            0
        } else if (increasing && psi > index[last]) || (!increasing && psi < index[last]) {
            last - 1
        } else {
            (0..last)
                .find(|&position| {
                    if increasing {
                        (index[position] == psi && psi < index[position + 1])
                            || (index[position] < psi && psi <= index[position + 1])
                    } else {
                        (index[position] == psi && psi > index[position + 1])
                            || (index[position] > psi && psi >= index[position + 1])
                    }
                })
                .ok_or_else(|| domain(self.axes[table_axis]))?
        };
        Ok(segment as f64 + 1.0 + (psi - index[segment]) / (index[segment + 1] - index[segment]))
    }

    fn array_to_index(&self, table_axis: usize, upsilon: f64) -> f64 {
        let Some(index) = &self.indices[table_axis] else {
            return upsilon;
        };
        if index.len() == 1 {
            return index[0];
        }
        let position = upsilon.floor() as usize;
        let lower = position.saturating_sub(1).min(index.len() - 2);
        index[lower] + (upsilon - lower as f64 - 1.0) * (index[lower + 1] - index[lower])
    }

    fn interpolate(&self, base: &[usize], delta: &[f64]) -> Vec<f64> {
        let dimensions = self.axes.len();
        let vertex_count = 1usize << self.variable_axes.len();
        let mut result = vec![0.0; dimensions];
        let mut indices = base.to_vec();
        for vertex in 0..vertex_count {
            indices.copy_from_slice(base);
            let mut weight = 1.0;
            for (bit, &table_axis) in self.variable_axes.iter().enumerate() {
                if vertex & (1 << bit) == 0 {
                    weight *= 1.0 - delta[table_axis];
                } else {
                    indices[table_axis] += 1;
                    weight *= delta[table_axis];
                }
            }
            if weight == 0.0 {
                continue;
            }
            let offset = self.coordinate_offset(&indices);
            for (table_axis, value) in result.iter_mut().enumerate() {
                *value += self.coordinates[offset + table_axis] * weight;
            }
            if weight == 1.0 {
                break;
            }
        }
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
                .is_none_or(|index| index[position] != index[position + 1]);
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
        for cell in 0..cell_count {
            let mut remainder = cell;
            let mut base = Vec::with_capacity(self.axes.len());
            for &length in &cell_lengths {
                base.push(remainder % length);
                remainder /= length;
            }
            let mut start = Vec::with_capacity(self.axes.len());
            let mut extent = Vec::with_capacity(self.axes.len());
            for (table_axis, &position) in base.iter().enumerate() {
                let length = self.lengths[table_axis];
                let first = if position == 0 { -0.5 } else { 0.0 };
                let last = if length == 1 || position == length - 2 {
                    1.5
                } else {
                    1.0
                };
                start.push(first);
                extent.push(if length == 1 { 1.0 } else { last - first });
            }
            let bounds = SubvoxelBounds { start, extent };
            let mut delta = vec![0.0; self.axes.len()];
            if self.locate_subvoxel(target, &base, &bounds, 0, &[], &mut delta) {
                return Ok(TabularLocation { base, delta });
            }
        }
        Err(domain(self.axes[0]))
    }

    fn locate_subvoxel(
        &self,
        target: &[f64],
        base: &[usize],
        bounds: &SubvoxelBounds,
        level: usize,
        voxel: &[usize],
        solution: &mut [f64],
    ) -> bool {
        let dimensions = self.axes.len();
        let vertex_count = 1usize << self.variable_axes.len();
        let size = 2.0f64.powi(-(level as i32));
        let mut lower = vec![false; dimensions];
        let mut upper = vec![false; dimensions];
        let mut equal = vec![false; dimensions];
        for vertex in 0..vertex_count {
            let mut delta = vec![0.0; dimensions];
            for table_axis in 0..dimensions {
                delta[table_axis] = if level == 0 {
                    bounds.start[table_axis]
                } else {
                    bounds.start[table_axis]
                        + size * bounds.extent[table_axis] * voxel[table_axis] as f64
                };
            }
            for (bit, &table_axis) in self.variable_axes.iter().enumerate() {
                if vertex & (1 << bit) != 0 {
                    delta[table_axis] += size * bounds.extent[table_axis];
                }
            }
            let coordinate = self.interpolate(base, &delta);
            let mut exact = true;
            for table_axis in 0..dimensions {
                let difference = coordinate[table_axis] - target[table_axis];
                if difference.abs() < TABULAR_TOLERANCE {
                    equal[table_axis] = true;
                } else {
                    exact = false;
                    if difference < 0.0 {
                        lower[table_axis] = true;
                    } else {
                        upper[table_axis] = true;
                    }
                }
            }
            if exact {
                solution.copy_from_slice(&delta);
                return true;
            }
        }
        let possible = (0..dimensions)
            .all(|axis| (lower[axis] || equal[axis]) && (upper[axis] || equal[axis]));
        if !possible {
            return false;
        }
        if level == 31 {
            let half = size / 2.0;
            for table_axis in 0..dimensions {
                solution[table_axis] = bounds.start[table_axis]
                    + half * bounds.extent[table_axis] * (2 * voxel[table_axis] + 1) as f64;
            }
            return true;
        }
        for subdivision in 0..vertex_count {
            let mut next = vec![0; dimensions];
            for (bit, &table_axis) in self.variable_axes.iter().enumerate() {
                let parent = if level == 0 { 0 } else { 2 * voxel[table_axis] };
                next[table_axis] = parent + usize::from(subdivision & (1 << bit) != 0);
            }
            if self.locate_subvoxel(target, base, bounds, level + 1, &next, solution) {
                return true;
            }
        }
        false
    }
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
struct SubvoxelBounds {
    start: Vec<f64>,
    extent: Vec<f64>,
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

fn validate_index(index: &[f64]) -> Result<()> {
    if index.iter().any(|value| !value.is_finite()) {
        return Err(invalid("TAB index vectors must contain finite values"));
    }
    if index.len() < 2 {
        return Ok(());
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
    Ok(())
}

fn integer_parameter(
    header: &Header,
    axis: usize,
    parameter: usize,
    suffix: &str,
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
