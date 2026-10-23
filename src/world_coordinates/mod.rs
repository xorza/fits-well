//! The World Coordinate System parse and transforms, published as [`crate::wcs`].

use std::f64::consts::PI;

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::error::Result;
use crate::header_model::value::Value;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::unit;
use crate::world_coordinates::axis::Axis;
use crate::world_coordinates::axis_transform::AxisTransform;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralParameters;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;
use crate::world_coordinates::celestial_frame::CelestialFrame;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::celestial_transform::CelestialTransform;
use crate::world_coordinates::ctype::Ctype;
use crate::world_coordinates::linear_transform::LinearMatrix;
use crate::world_coordinates::linear_transform::LinearTransform;
use crate::world_coordinates::projected_celestial_axes::CelestialAxisPair;
use crate::world_coordinates::projected_celestial_axes::ProjectedCelestialAxes;
use crate::world_coordinates::projection::Projection;
use crate::world_coordinates::spectral_frame::SpectralFrame;
use crate::world_coordinates::table_wcs::TableWcs;

pub(crate) mod axis;
pub(crate) mod axis_transform;
#[cfg(feature = "internals")]
pub(crate) mod bench;
mod celestial_axis;
pub(crate) mod celestial_frame;
pub(crate) mod celestial_pole;
mod celestial_transform;
pub(crate) mod ctype;
mod linear_transform;
mod projected_celestial_axes;
pub(crate) mod projection;
pub(crate) mod spectral_frame;
mod table_wcs;
pub(crate) mod tabular;

const R2D: f64 = 180.0 / PI;
const D2R: f64 = PI / 180.0;
/// How far past a bound a value in degrees may land through rounding and still
/// read as the bound — a latitude past ±90°, a projection-plane coordinate past its
/// edge. 1e-12° is about 70 ulp of 90 in degrees: more than the few
/// chained trigonometric operations behind such a value lose, and within the band
/// of 1e-13° to 1e-11° wcslib's `prjbchk` allows for the same test.
const DEGREE_TOLERANCE: f64 = 1e-12;

/// The same allowance for a dimensionless value of order one — a sine or cosine
/// past ±1, a cube-face coordinate past its edge, the perspective σ past [0, 2] —
/// and, scaled by the operands, for a difference that should not be negative. About
/// 4500 ulp of 1, the order of wcslib's 1e-13 `tol` for its zenithal and cube tests.
const UNIT_TOLERANCE: f64 = 1e-12;

/// Public celestial-pair metadata for a parsed WCS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CelestialProjection {
    /// Zero-based longitude axis.
    pub longitude_axis: usize,
    /// Zero-based latitude axis.
    pub latitude_axis: usize,
    pub projection: Projection,
    /// The native-to-celestial rotation.
    pub pole: CelestialPole,
}

/// A read-only snapshot of a parsed WCS's source metadata and support status.
#[derive(Debug, Clone, Copy)]
pub struct Metadata<'a> {
    pub axes: &'a [Axis],
    /// The resolved `RADESYSa`/`EQUINOXa` metadata when applicable.
    pub celestial_frame: Option<CelestialFrame>,
    /// Resolved celestial axes, projection, and native pole.
    pub celestial_projection: Option<CelestialProjection>,
    /// Zero-based axes whose non-linear transform is not evaluated.
    pub unsupported_axes: &'a [usize],
}

/// One axis's world coordinate together with the unit it is expressed in.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AxisWorld<'a> {
    pub(crate) cunit: &'a str,
    pub(crate) value: f64,
}

/// A parsed world coordinate system for one (optionally alternate) axis set.
#[derive(Debug, Clone)]
pub struct Wcs {
    axes: Vec<Axis>,
    /// Each axis's `CRVALi` in the unit its transform works in: degrees for a celestial
    /// axis, the Table-25 default unit for a spectral one, the declared unit otherwise.
    reference: Vec<f64>,
    /// The unit an axis's world coordinates come out in, where it is not the declared
    /// `CUNITi`.
    world_units: Vec<Option<&'static str>>,
    axis_transforms: Vec<AxisTransform>,
    /// The linear layer `(pixel − CRPIX) → intermediate world coordinate`, and its
    /// inverse.
    linear: LinearTransform,
    /// The celestial axes, projection, and pole when a celestial pair is present;
    /// `None` for an all-linear system.
    celestial: Option<CelestialTransform>,
    celestial_frame: Option<CelestialFrame>,
    tabular: Vec<tabular::TabularTransform>,
    /// Axes (0-based) whose non-linear transform is not evaluated — an unsupported
    /// celestial projection or convention, or another non-linear coordinate
    /// algorithm (§8.3/§8.4). Complete transforms reject these axes.
    unsupported_axes: Vec<usize>,
}

/// Where the spectral reference frame of each image axis comes from.
#[derive(Debug)]
enum SpectralFrames {
    /// The header's own keywords, one frame for every spectral axis.
    Header,
    /// Each axis's frame as a binary table's columns stated it, one entry per axis.
    Columns(Vec<Option<SpectralFrame>>),
}

impl Wcs {
    /// Parse the description `alt` selects from `header`: `None` for the primary one,
    /// `'A'` to `'Z'` for an alternate.
    pub fn from_header(
        header: &Header,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Self> {
        Self::from_header_with_context(
            header,
            Alternate::of(alt)?,
            Vec::new(),
            SpectralFrames::Header,
        )
    }

    pub(crate) fn from_header_with_tabular(
        header: &Header,
        alt: Alternate,
        tabular: Vec<tabular::TabularTransform>,
    ) -> Result<Self> {
        Self::from_header_with_context(header, alt, tabular, SpectralFrames::Header)
    }

    fn from_header_with_context(
        header: &Header,
        alt: Alternate,
        tabular: Vec<tabular::TabularTransform>,
        spectral_frames: SpectralFrames,
    ) -> Result<Self> {
        let naxis = Self::image_axis_count(header, alt)?;

        let ctype: Vec<String> = (1..=naxis)
            .map(|i| {
                read_text(header, Keyword::fitting(format_args!("CTYPE{i}{alt}")))
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let declared_crval = axis_vec(header, "CRVAL", alt.as_str(), naxis, 0.0)?;
        let mut crval = declared_crval.clone();
        let mut world_units = vec![None; naxis];
        let crpix = axis_vec(header, "CRPIX", alt.as_str(), naxis, 0.0)?;
        let cdelt = axis_vec(header, "CDELT", alt.as_str(), naxis, 1.0)?;
        let cunit: Vec<String> = (1..=naxis)
            .map(|i| {
                read_text(header, Keyword::fitting(format_args!("CUNIT{i}{alt}")))
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let celestial_pair = CelestialAxisPair::find(&ctype);
        let celestial_axes = ProjectedCelestialAxes::find(&ctype)?;
        let celestial_frame = CelestialFrame::from_header(header, alt, &ctype)?;
        let spectral_frames = if let SpectralFrames::Columns(frames) = spectral_frames {
            assert_eq!(frames.len(), naxis, "spectral frame count");
            frames
        } else {
            let frame = ctype
                .iter()
                .any(|ctype| axis_transform::is_spectral_type(ctype))
                .then(|| SpectralFrame::from_header(header, alt))
                .transpose()?;
            ctype
                .iter()
                .map(|ctype| frame.filter(|_| axis_transform::is_spectral_type(ctype)))
                .collect()
        };

        let matrix = LinearMatrix::from_header(header, alt, naxis, &cdelt, celestial_axes)?;
        // Each axis's `CUNITia` factor, applied to its `CRVAL` here and to its whole
        // matrix row once the axis parse reports it: the projection math runs in
        // degrees and the spectral algorithms in their Table-25 default units, so the
        // two must scale together (§8.2).
        let mut axis_scales = vec![1.0; naxis];
        if let Some(axes) = celestial_axes {
            for ax in [axes.longitude, axes.latitude] {
                axis_scales[ax] = world_unit_scale(Ctype::parse(&ctype[ax]), &cunit[ax])?;
                crval[ax] *= axis_scales[ax];
                world_units[ax] = Some("deg");
            }
        }
        let mut axis_transforms = Vec::with_capacity(naxis);
        let mut unsupported_axes = Vec::new();
        let mut resolved_tabular_axes = vec![false; naxis];
        for transform in &tabular {
            for &axis in &transform.axes {
                resolved_tabular_axes[axis] = true;
            }
        }
        for axis in 0..naxis {
            let parsed_ctype = Ctype::parse(&ctype[axis]);
            // A `-TAB` axis whose coordinate array was resolved from its BINTABLE is
            // evaluated by the tabular transform, so its own algorithm is never
            // consulted — and never counts as unsupported.
            let resolved_tabular =
                parsed_ctype.algorithm == Some("TAB") && resolved_tabular_axes[axis];
            if parsed_ctype.celestial_axis().is_some() {
                axis_transforms.push(AxisTransform::Joint);
                // A projection this crate does not evaluate, or an axis with no partner
                // of its own system, has no transform.
                let unpaired = !celestial_pair.is_some_and(|pair| pair.contains(axis));
                if !resolved_tabular
                    && (unpaired
                        || parsed_ctype
                            .algorithm
                            .is_some_and(|code| Projection::from_code(code).is_none()))
                {
                    unsupported_axes.push(axis);
                }
                continue;
            }
            let mut spectral_parameters = [None; 7];
            for (parameter, value) in spectral_parameters.iter_mut().enumerate() {
                *value = read_real(
                    header,
                    Keyword::fitting(format_args!("PV{}_{parameter}{alt}", axis + 1)),
                )?;
            }
            let parameters = SpectralParameters::new(spectral_parameters);
            let rest = spectral_frames[axis].map_or(SpectralRest::NONE, |frame| frame.rest);
            let spec =
                AxisTransform::parse(&ctype[axis], &cunit[axis], crval[axis], rest, parameters)?;
            axis_scales[axis] = spec.unit_scale;
            crval[axis] *= spec.unit_scale;
            world_units[axis] = spec.world_unit;
            if matches!(&spec.transform, AxisTransform::Unsupported) && !resolved_tabular {
                unsupported_axes.push(axis);
            }
            axis_transforms.push(if resolved_tabular {
                AxisTransform::Joint
            } else {
                spec.transform
            });
        }
        let linear = matrix.scaled(&axis_scales)?;

        let celestial = CelestialTransform::from_header(
            header,
            alt,
            celestial_axes,
            &crval,
            &mut unsupported_axes,
        )?;

        let axes = ctype
            .into_iter()
            .enumerate()
            .map(|(i, ctype)| Axis {
                ctype,
                cunit: cunit[i].clone(),
                crval: declared_crval[i],
                crpix: crpix[i],
                spectral_frame: spectral_frames[i],
            })
            .collect();
        Ok(Self {
            axes,
            reference: crval,
            world_units,
            axis_transforms,
            linear,
            celestial,
            celestial_frame,
            tabular,
            unsupported_axes,
        })
    }

    /// Borrow the parsed axis metadata and unsupported-axis flags without exposing
    /// the transform's invariant-bearing owned state for mutation.
    pub fn metadata(&self) -> Metadata<'_> {
        Metadata {
            axes: &self.axes,
            celestial_frame: self.celestial_frame,
            celestial_projection: self
                .celestial
                .as_ref()
                .map(|celestial| CelestialProjection {
                    longitude_axis: celestial.lng,
                    latitude_axis: celestial.lat,
                    projection: celestial.projection,
                    pole: celestial.pole,
                }),
            unsupported_axes: &self.unsupported_axes,
        }
    }

    /// One axis's world coordinate at `pixel`, with the unit it is in. A celestial axis
    /// evaluates its pair's projection, as [`Self::pixel_to_world`] does.
    pub(crate) fn axis_world(&self, axis: usize, pixel: &[f64]) -> Result<AxisWorld<'_>> {
        let naxis = self.axes.len();
        if axis >= naxis {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::WcsAxis,
                index: axis.saturating_add(1),
                len: naxis,
            });
        }
        if pixel.len() != naxis {
            return Err(FitsError::RankMismatch {
                ranked: Ranked::WcsCoordinate,
                expected: naxis,
                got: pixel.len(),
            });
        }
        if self.unsupported_axes.contains(&axis) {
            return Err(FitsError::UnsupportedWcsTransform { axes: vec![axis] });
        }
        let cunit = self.world_units[axis].unwrap_or(&self.axes[axis].cunit);
        if let Some(transform) = self
            .tabular
            .iter()
            .find(|transform| transform.axes.contains(&axis))
        {
            let intermediate: Vec<f64> = transform
                .axes
                .iter()
                .map(|&image_axis| self.linear.intermediate_axis(image_axis, pixel, &self.axes))
                .collect();
            return Ok(AxisWorld {
                cunit,
                value: transform.to_world_axis(axis, &intermediate)?,
            });
        }
        if let Some(c) = self
            .celestial
            .as_ref()
            .filter(|c| axis == c.lng || axis == c.lat)
        {
            let native = c.projection.deproject(
                self.linear.intermediate_axis(c.lng, pixel, &self.axes),
                self.linear.intermediate_axis(c.lat, pixel, &self.axes),
                &c.parameters,
            )?;
            let celestial = c.pole.to_celestial(native.phi, native.theta);
            let value = if axis == c.lng {
                celestial.ra
            } else {
                celestial.dec
            };
            return Ok(AxisWorld { cunit, value });
        }
        let intermediate = self.linear.intermediate_axis(axis, pixel, &self.axes);
        Ok(AxisWorld {
            cunit,
            value: self.axis_transforms[axis].to_world(intermediate, self.reference[axis], axis)?,
        })
    }

    /// Build a WCS for a binary-table **pixel list** (event list, §8.5, Table 22):
    /// `columns` lists the 1-based table column numbers forming the coordinate axes
    /// in order. Reads both the primary and shortened alternate column-indexed
    /// families, both matrix/parameter spellings (`TPC`/`TP`, `TCD`/`TC`,
    /// `TPV`/`TV`), and the longitude column's `LONPna`/`LATPna` pole keywords,
    /// then evaluates them through the same pipeline as image WCS.
    pub fn from_pixel_list(
        header: &Header,
        columns: &[usize],
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Self> {
        Self::parse_pixel_list(header, columns, Alternate::of(alt)?)
    }

    fn parse_pixel_list(header: &Header, columns: &[usize], alt: Alternate) -> Result<Self> {
        let table = TableWcs::pixel_list(alt, columns);
        let translated = table.translate(header)?;
        let mut h = translated.header;
        let ctype = (1..=columns.len())
            .map(|axis| {
                h.get_text(keywords::CTYPE.indexed(axis)?)
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        // Each axis is its own column, so the pole and frame keywords are those of the
        // celestial columns — and mean nothing until a longitude/latitude pair exists.
        if let Some(pair) = CelestialAxisPair::find(&ctype) {
            let celestial_columns = [columns[pair.longitude], columns[pair.latitude]];
            table.copy_celestial_keywords(header, &mut h, &celestial_columns)?;
        }
        Self::from_header_with_context(
            &h,
            Alternate::PRIMARY,
            Vec::new(),
            SpectralFrames::Columns(translated.spectral_frames),
        )
    }

    /// Build a WCS for an image stored in a binary-table **vector cell** (§8,
    /// Table 22): `column` is the 1-based table column whose cells hold a
    /// multidimensional array. Reads the primary and shortened alternate
    /// axis-and-column-indexed families, `ijPCna`/`ijCDna`, `iPVn_ma`/`iVn_ma`,
    /// and `LONPna`/`LATPna`, where `i`/`j` are array axes and `n` is the column.
    /// The rank is taken from `WCAXna`, else inferred through the same resolver.
    pub fn from_array_column(
        header: &Header,
        column: usize,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Self> {
        Self::parse_array_column(header, column, Alternate::of(alt)?)
    }

    fn parse_array_column(header: &Header, column: usize, alt: Alternate) -> Result<Self> {
        let naxis = TableWcs::array_column_rank(header, alt, column)?;
        let table = TableWcs::array_column(alt, naxis, column);
        let translated = table.translate(header)?;
        let mut h = translated.header;
        // Every axis lives in one column, so that column's pole and frame keywords
        // apply whichever axes turn out to be celestial.
        table.copy_celestial_keywords(header, &mut h, &[column])?;
        Self::from_header_with_context(
            &h,
            Alternate::PRIMARY,
            Vec::new(),
            SpectralFrames::Columns(translated.spectral_frames),
        )
    }

    /// Write the complete world coordinate of the 1-based `pixel` coordinate to `world`,
    /// one value per axis. Celestial axes give `(α, δ)` in degrees, nonlinear spectral
    /// axes their Table-25 default units, and other axes their declared units. A point
    /// makes no allocation unless an axis is `-TAB`.
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::RankMismatch`] unless `pixel` and `world` each hold one value
    /// per axis, [`FitsError::UnsupportedWcsTransform`] if any nonlinear axis is not
    /// implemented, or a projection error when the coordinate is outside its domain. On
    /// an error, `world` holds no defined value.
    pub fn pixel_to_world(&self, pixel: &[f64], world: &mut [f64]) -> Result<()> {
        self.require_point(pixel)?;
        self.require_point(world)?;
        self.require_complete_transform()?;
        self.linear.intermediate(pixel, &self.axes, world);
        for (axis, value) in world.iter_mut().enumerate() {
            *value = self.axis_transforms[axis].to_world(*value, self.reference[axis], axis)?;
        }
        for transform in &self.tabular {
            transform.to_world(world)?;
        }
        if let Some(c) = &self.celestial {
            let native = c
                .projection
                .deproject(world[c.lng], world[c.lat], &c.parameters)?;
            let celestial = c.pole.to_celestial(native.phi, native.theta);
            world[c.lng] = celestial.ra;
            world[c.lat] = celestial.dec;
        }
        Ok(())
    }

    /// Write the 1-based pixel coordinate of the complete `world` coordinate to `pixel`,
    /// the inverse of [`Wcs::pixel_to_world`].
    ///
    /// # Errors
    ///
    /// As [`Wcs::pixel_to_world`]. On an error, `pixel` holds no defined value.
    pub fn world_to_pixel(&self, world: &[f64], pixel: &mut [f64]) -> Result<()> {
        self.require_point(world)?;
        self.require_point(pixel)?;
        self.require_complete_transform()?;
        for (axis, (value, &world)) in pixel.iter_mut().zip(world).enumerate() {
            *value =
                self.axis_transforms[axis].to_intermediate(world, self.reference[axis], axis)?;
        }
        for transform in &self.tabular {
            transform.to_intermediate(pixel)?;
        }
        if let Some(c) = self.celestial.as_ref() {
            if !world[c.lng].is_finite()
                || !world[c.lat].is_finite()
                || !(-90.0 - DEGREE_TOLERANCE..=90.0 + DEGREE_TOLERANCE).contains(&world[c.lat])
            {
                return Err(c.projection.world_domain_error());
            }
            let native = c.pole.to_native(world[c.lng], world[c.lat]);
            let projected = c
                .projection
                .project(native.phi, native.theta, &c.parameters)?;
            pixel[c.lng] = projected.x;
            pixel[c.lat] = projected.y;
        }
        self.linear.pixel(pixel, &self.axes);
        Ok(())
    }

    /// Refuse a `point` that does not hold one value per axis.
    const fn require_point(&self, point: &[f64]) -> Result<()> {
        let naxis = self.axes.len();
        if point.len() != naxis {
            return Err(FitsError::RankMismatch {
                ranked: Ranked::WcsCoordinate,
                expected: naxis,
                got: point.len(),
            });
        }
        Ok(())
    }

    /// The number of WCS axes an image header declares: `WCSAXESa`, else the larger
    /// of `NAXIS` and the highest axis any WCS keyword names.
    pub(crate) fn image_axis_count(header: &Header, alt: Alternate) -> Result<usize> {
        let value = if let Some(axis_count) =
            read_integer(header, Keyword::fitting(format_args!("WCSAXES{alt}")))?
        {
            axis_count
        } else {
            let inferred = infer_image_axis_count(header, alt.as_str());
            match header.get_integer(keywords::NAXIS)? {
                Some(axis_count) if axis_count >= 0 => axis_count.max(inferred),
                Some(axis_count) => axis_count,
                None if inferred != 0 => inferred,
                None => {
                    return Err(FitsError::MissingKeyword {
                        keyword: keywords::WCSAXES,
                    });
                }
            }
        };
        validated_axis_count(value, keywords::WCSAXES)
    }

    fn require_complete_transform(&self) -> Result<()> {
        if self.unsupported_axes.is_empty() {
            Ok(())
        } else {
            Err(FitsError::UnsupportedWcsTransform {
                axes: self.unsupported_axes.clone(),
            })
        }
    }
}

/// The factor from an axis's declared `CUNITi` to the unit its world coordinates
/// come out in: degrees for a celestial axis (§8.2), the Table-25 default for a
/// spectral one (§8.4), and the declared unit for any other.
pub(crate) fn world_unit_scale(ctype: Ctype<'_>, cunit: &str) -> Result<f64> {
    if let Some(scale) = axis_transform::spectral_unit_scale(ctype.head, cunit)? {
        return Ok(scale);
    }
    if ctype.celestial_axis().is_some() {
        return angle_scale(cunit);
    }
    Ok(1.0)
}

fn angle_scale(unit: &str) -> Result<f64> {
    if unit.trim().is_empty() {
        return Ok(1.0);
    }
    unit::resolve(unit, unit::ANGLE).ok_or_else(|| FitsError::InvalidUnit {
        unit: unit.to_string(),
        expected: "an angle unit",
    })
}

/// Read `PREFIX1..PREFIXn` (with alternate suffix) into a vector, defaulting
/// missing entries.
fn axis_vec(
    header: &Header,
    prefix: &str,
    alt: &str,
    naxis: usize,
    default: f64,
) -> Result<Vec<f64>> {
    (1..=naxis)
        .map(|i| {
            read_real(header, Keyword::fitting(format_args!("{prefix}{i}{alt}")))
                .map(|value| value.unwrap_or(default))
        })
        .collect()
}

fn infer_image_axis_count(header: &Header, alt: &str) -> i64 {
    header
        .iter()
        .filter_map(|record| image_wcs_axis_index(record.name(), alt))
        .max()
        .unwrap_or(0)
}

fn image_wcs_axis_index(keyword: &str, alt: &str) -> Option<i64> {
    let keyword = if alt.is_empty() {
        keyword
    } else {
        keyword.strip_suffix(alt)?
    };
    for prefix in [
        "CTYPE", "CUNIT", "CRVAL", "CDELT", "CRPIX", "CROTA", "CNAME", "CRDER", "CSYER", "CZPHS",
        "CPERI",
    ] {
        if let Some(index) = keyword.strip_prefix(prefix).and_then(parse_wcs_index) {
            return Some(index);
        }
    }
    for prefix in ["PC", "CD"] {
        if let Some(indices) = keyword.strip_prefix(prefix)
            && let Some((i, j)) = indices.split_once('_')
        {
            return Some(parse_wcs_index(i)?.max(parse_wcs_index(j)?));
        }
    }
    for prefix in ["PV", "PS"] {
        if let Some(indices) = keyword.strip_prefix(prefix)
            && let Some((i, m)) = indices.split_once('_')
        {
            m.parse::<u64>().ok()?;
            return parse_wcs_index(i);
        }
    }
    None
}

fn parse_wcs_index(value: &str) -> Option<i64> {
    value.parse().ok().filter(|&index| index > 0)
}

/// A declared axis count, checked against the standard's `1..=999` range.
fn validated_axis_count(value: i64, keyword: Keyword) -> Result<usize> {
    let count = usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { keyword })?;
    if !(1..=999).contains(&count) {
        return Err(FitsError::KeywordOutOfRange { keyword });
    }
    Ok(count)
}

/// The first of two keywords that carries a real value — the shape every
/// long/short and primary/`PVi_m` keyword alternative takes.
fn first_real(
    header: &Header,
    first: impl Into<Option<Keyword>>,
    second: impl Into<Option<Keyword>>,
) -> Result<Option<f64>> {
    match read_real(header, first)? {
        Some(value) => Ok(Some(value)),
        None => read_real(header, second),
    }
}

/// The value of `keyword`; `None` when the card is absent, or when there is no keyword,
/// as where a wide axis or column number makes a formatted name longer than a keyword,
/// and so no card can hold it.
fn read_value(header: &Header, keyword: impl Into<Option<Keyword>>) -> Option<&Value> {
    header
        .get(keyword.into()?)
        .expect("a `Keyword` argument needs no conversion")
}

/// [`read_value`] for a real keyword.
fn read_real(header: &Header, keyword: impl Into<Option<Keyword>>) -> Result<Option<f64>> {
    keyword
        .into()
        .map_or(Ok(None), |keyword| header.get_real(keyword))
}

/// [`read_value`] for a text keyword.
fn read_text(header: &Header, keyword: impl Into<Option<Keyword>>) -> Result<Option<&str>> {
    keyword
        .into()
        .map_or(Ok(None), |keyword| header.get_text(keyword))
}

/// [`read_value`] for an integer keyword.
fn read_integer(header: &Header, keyword: impl Into<Option<Keyword>>) -> Result<Option<i64>> {
    keyword
        .into()
        .map_or(Ok(None), |keyword| header.get_integer(keyword))
}

/// The quarter turn a multiple of 90° lands on (0 to 3), or `None` for any other angle.
#[expect(
    clippy::cast_sign_loss,
    reason = "a multiple of 90° divided by 90 and reduced mod 4 is a whole number in 0..4"
)]
fn quarter_turn(degrees: f64) -> Option<u8> {
    (degrees.rem_euclid(90.0) == 0.0).then(|| (degrees / 90.0).rem_euclid(4.0) as u8)
}

/// `sin` of an angle in degrees, exact at multiples of 90°, where the conversion to
/// radians would leave a residue of order `1e-17` — wcslib's `sind`, which its boundary
/// tests rely on.
fn sind(degrees: f64) -> f64 {
    match quarter_turn(degrees) {
        Some(0 | 2) => 0.0,
        Some(1) => 1.0,
        Some(_) => -1.0,
        None => (degrees * D2R).sin(),
    }
}

/// `cos` of an angle in degrees, exact at multiples of 90° — wcslib's `cosd`.
fn cosd(degrees: f64) -> f64 {
    match quarter_turn(degrees) {
        Some(0) => 1.0,
        Some(1 | 3) => 0.0,
        Some(_) => -1.0,
        None => (degrees * D2R).cos(),
    }
}

/// Normalize an angle to `[0, 360)` degrees.
fn norm360(a: f64) -> f64 {
    // A tiny negative angle's remainder, 360 − |a|, rounds to 360 itself.
    let normalized = a.rem_euclid(360.0);
    if normalized == 360.0 { 0.0 } else { normalized }
}

/// Normalize an angle to `[−180, 180)` degrees, exactly: the truncated remainder is
/// exact, and the one turn added or taken back is exact by Sterbenz's lemma.
fn norm180(a: f64) -> f64 {
    let r = a % 360.0;
    if r >= 180.0 {
        r - 360.0
    } else if r < -180.0 {
        r + 360.0
    } else {
        r
    }
}

/// Golden pixel→world sets from `astropy.wcs` for the shared fixtures, and the
/// assertion over them — used by the test modules of the layers a transform is
/// assembled from, not just by [`Wcs`]'s own.
#[cfg(test)]
pub(crate) mod internals {
    use crate::error::Result;
    use crate::header_model::Header;
    use crate::header_model::keywords;
    use crate::world_coordinates::DEGREE_TOLERANCE;
    use crate::world_coordinates::Wcs;

    /// [`Wcs::pixel_to_world`] into a new `Vec`, for the tests that compare whole points.
    pub(crate) fn to_world(wcs: &Wcs, pixel: &[f64]) -> Result<Vec<f64>> {
        let mut world = vec![0.0; wcs.axes.len()];
        wcs.pixel_to_world(pixel, &mut world)?;
        Ok(world)
    }

    /// [`Wcs::world_to_pixel`] into a new `Vec`, for the tests that compare whole points.
    pub(crate) fn to_pixel(wcs: &Wcs, world: &[f64]) -> Result<Vec<f64>> {
        let mut pixel = vec![0.0; wcs.axes.len()];
        wcs.world_to_pixel(world, &mut pixel)?;
        Ok(pixel)
    }

    /// A one-axis image header.
    pub(crate) fn axis_header(
        ctype: &str,
        cunit: &str,
        crpix: f64,
        crval: f64,
        cdelt: f64,
    ) -> Header {
        let mut header = Header::new();
        header
            .set_internal(keywords::NAXIS, 1)
            .set_internal("CTYPE1", ctype)
            .set_internal("CUNIT1", cunit)
            .set_internal("CRPIX1", crpix)
            .set_internal("CRVAL1", crval)
            .set_internal("CDELT1", cdelt);
        header
    }

    /// A two-axis `RA---{projection}`/`DEC--{projection}` image header.
    pub(crate) fn celestial_header(
        projection: &str,
        crpix: [f64; 2],
        crval: [f64; 2],
        cdelt: [f64; 2],
    ) -> Header {
        let mut header = Header::new();
        header
            .set_internal(keywords::NAXIS, 2)
            .set_internal("CTYPE1", format!("RA---{projection}"))
            .set_internal("CTYPE2", format!("DEC--{projection}"))
            .set_internal("CRPIX1", crpix[0])
            .set_internal("CRPIX2", crpix[1])
            .set_internal("CRVAL1", crval[0])
            .set_internal("CRVAL2", crval[1])
            .set_internal("CDELT1", cdelt[0])
            .set_internal("CDELT2", cdelt[1]);
        header
    }

    /// Pixel→world values from `astropy.wcs` (wcslib), printed to `decimals` places.
    #[derive(Debug)]
    pub(crate) struct AstropyGolden<'a> {
        pub(crate) decimals: i32,
        /// Pixel x, pixel y, longitude (deg), latitude (deg).
        pub(crate) points: &'a [(f64, f64, f64, f64)],
    }

    /// `wcs_tan.fits`: `RA---TAN`/`DEC--TAN`, CRVAL 150/2.5, CRPIX 256/256, 1″
    /// pixels, 15° rotation.
    pub(crate) const TAN_GOLDEN: AstropyGolden<'_> = AstropyGolden {
        decimals: 12,
        points: &[
            (1.0, 1.0, 150.050_131_124_369, 2.413_246_375_001),
            (256.0, 256.0, 150.000_000_000_000, 2.500_000_000_000),
            (512.0, 512.0, 149.949_665_615_474, 2.587_091_911_566),
            (100.0, 400.0, 150.052_260_368_590, 2.527_420_491_210),
            (256.5, 256.5, 149.999_901_697_142, 2.500_170_103_464),
            (400.0, 123.0, 149.951_756_061_540, 2.474_666_292_235),
        ],
    };

    pub(crate) const CEA_GOLDEN: AstropyGolden<'_> = AstropyGolden {
        decimals: 10,
        points: &[
            (20.0, 70.0, 46.740_687_082_8, 30.488_614_011_0),
            (80.0, 30.0, 43.276_761_337_7, 29.488_715_511_3),
        ],
    };

    pub(crate) const CROTA_GOLDEN: AstropyGolden<'_> = AstropyGolden {
        decimals: 10,
        points: &[
            (128.0, 128.0, 83.600_000_000_0, 22.000_000_000_0),
            (1.0, 1.0, 83.690_994_315_6, 21.969_260_649_2),
            (256.0, 200.0, 83.521_028_833_8, 22.005_560_605_0),
            (64.0, 192.0, 83.616_698_637_6, 22.042_524_779_3),
        ],
    };

    pub(crate) fn assert_astropy_golden(wcs: &Wcs, golden: &AstropyGolden<'_>, context: &str) {
        // A printed value lies within half a unit in its last place of wcslib's
        // answer, and DEGREE_TOLERANCE covers where this evaluation and wcslib's part.
        let tolerance = 0.5 * 10f64.powi(-golden.decimals) + DEGREE_TOLERANCE;
        for &(px, py, ra, dec) in golden.points {
            let world = to_world(wcs, &[px, py]).unwrap();
            assert!(
                (world[0] - ra).abs() <= tolerance && (world[1] - dec).abs() <= tolerance,
                "{context} at ({px},{py}): got {world:?}, want ({ra},{dec})"
            );
        }
    }
}

#[cfg(test)]
mod tests;
