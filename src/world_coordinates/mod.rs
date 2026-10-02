//! The World Coordinate System parse and transforms, published as [`crate::wcs`].

use std::f64::consts::PI;

use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::AltSuffix;
use crate::keyword::key;
use crate::unit;
use crate::world_coordinates::axis::AxisTransform;
use crate::world_coordinates::axis::spectral_rest::SpectralParameters;
use crate::world_coordinates::axis::spectral_rest::SpectralRest;
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
use crate::world_coordinates::wcs_axis::WcsAxis;

mod axis;
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
pub(crate) mod wcs_axis;

const R2D: f64 = 180.0 / PI;
const D2R: f64 = PI / 180.0;
const DOMAIN_TOLERANCE: f64 = 1e-12;

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
pub struct WcsView<'a> {
    pub axes: &'a [WcsAxis],
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
    axes: Vec<WcsAxis>,
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

impl Wcs {
    /// Parse the primary WCS (`alt = None`) or an alternate description
    /// (`alt = Some('A'..='Z')`) from `header`.
    pub fn from_header(header: &Header, alt: Option<char>) -> Result<Wcs> {
        Wcs::from_header_with_context(header, alt, Vec::new(), None)
    }

    pub(crate) fn from_header_with_tabular(
        header: &Header,
        alt: Option<char>,
        tabular: Vec<tabular::TabularTransform>,
    ) -> Result<Wcs> {
        Wcs::from_header_with_context(header, alt, tabular, None)
    }

    fn from_header_with_context(
        header: &Header,
        alt: Option<char>,
        tabular: Vec<tabular::TabularTransform>,
        spectral_frames: Option<Vec<Option<SpectralFrame>>>,
    ) -> Result<Wcs> {
        let a = AltSuffix::new(alt);
        let naxis = Wcs::image_axis_count(header, alt)?;

        let ctype: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CTYPE{i}{a}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let declared_crval = axis_vec(header, "CRVAL", a.as_str(), naxis, 0.0)?;
        let mut crval = declared_crval.clone();
        let mut world_units = vec![None; naxis];
        let crpix = axis_vec(header, "CRPIX", a.as_str(), naxis, 0.0)?;
        let cdelt = axis_vec(header, "CDELT", a.as_str(), naxis, 1.0)?;
        let cunit: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CUNIT{i}{a}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let celestial_pair = CelestialAxisPair::find(&ctype);
        let celestial_axes = ProjectedCelestialAxes::find(&ctype)?;
        let celestial_frame = CelestialFrame::from_header(header, alt, a.as_str(), &ctype)?;
        let spectral_frames = match spectral_frames {
            Some(frames) => {
                assert_eq!(frames.len(), naxis, "spectral frame count");
                frames
            }
            None => {
                let frame = ctype
                    .iter()
                    .any(|ctype| axis::is_spectral_type(ctype))
                    .then(|| SpectralFrame::from_header(header, alt, a.as_str()))
                    .transpose()?;
                ctype
                    .iter()
                    .map(|ctype| axis::is_spectral_type(ctype).then_some(frame).flatten())
                    .collect()
            }
        };

        let matrix = LinearMatrix::from_header(header, a, naxis, &cdelt, celestial_axes)?;
        // Each axis's `CUNITia` factor, applied to its `CRVAL` here and to its whole
        // matrix row once the axis parse reports it: the projection math runs in
        // degrees and the spectral algorithms in their Table-25 default units, so the
        // two must scale together (§8.2).
        let mut axis_scales = vec![1.0; naxis];
        if let Some(axes) = celestial_axes {
            for ax in [axes.longitude, axes.latitude] {
                axis_scales[ax] = angle_scale(&cunit[ax])?;
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
                axis_transforms.push(AxisTransform::Linear);
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
                *value = header.get_real(key!("PV{}_{parameter}{a}", axis + 1).as_str())?;
            }
            let parameters = SpectralParameters::new(spectral_parameters);
            let rest = match spectral_frames[axis] {
                Some(frame) => SpectralRest::new(frame.rest_frequency_hz, frame.rest_wavelength_m)?,
                None => SpectralRest::NONE,
            };
            let spec =
                AxisTransform::parse(&ctype[axis], &cunit[axis], crval[axis], rest, parameters)?;
            axis_scales[axis] = spec.unit_scale;
            crval[axis] *= spec.unit_scale;
            world_units[axis] = spec.world_unit;
            if matches!(&spec.transform, AxisTransform::Unsupported) && !resolved_tabular {
                unsupported_axes.push(axis);
            }
            axis_transforms.push(if resolved_tabular {
                AxisTransform::Linear
            } else {
                spec.transform
            });
        }
        let linear = matrix.scaled(&axis_scales)?;

        let celestial = CelestialTransform::from_header(
            header,
            a,
            celestial_axes,
            &crval,
            &mut unsupported_axes,
        )?;

        let axes = ctype
            .into_iter()
            .enumerate()
            .map(|(i, ctype)| WcsAxis {
                ctype,
                cunit: cunit[i].clone(),
                crval: declared_crval[i],
                crpix: crpix[i],
                spectral_frame: spectral_frames[i],
            })
            .collect();
        Ok(Wcs {
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
    pub fn view(&self) -> WcsView<'_> {
        WcsView {
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
    pub fn from_pixel_list(header: &Header, columns: &[usize], alt: Option<char>) -> Result<Wcs> {
        let table = TableWcs::pixel_list(alt, columns);
        let translated = table.translate(header)?;
        let mut h = translated.header;
        let ctype = (1..=columns.len())
            .map(|axis| {
                h.get_text(key!("CTYPE{axis}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        // Each axis is its own column, so the pole and frame keywords are those of the
        // celestial columns — and mean nothing until a longitude/latitude pair exists.
        if let Some(pair) = CelestialAxisPair::find(&ctype) {
            let celestial_columns = [columns[pair.longitude], columns[pair.latitude]];
            table.copy_celestial_keywords(header, &mut h, &celestial_columns)?;
        }
        Wcs::from_header_with_context(&h, None, Vec::new(), Some(translated.spectral_frames))
    }

    /// Build a WCS for an image stored in a binary-table **vector cell** (§8,
    /// Table 22): `column` is the 1-based table column whose cells hold a
    /// multidimensional array. Reads the primary and shortened alternate
    /// axis-and-column-indexed families, `ijPCna`/`ijCDna`, `iPVn_ma`/`iVn_ma`,
    /// and `LONPna`/`LATPna`, where `i`/`j` are array axes and `n` is the column.
    /// The rank is taken from `WCAXna`, else inferred through the same resolver.
    pub fn from_array_column(header: &Header, column: usize, alt: Option<char>) -> Result<Wcs> {
        let naxis = TableWcs::array_column_rank(header, alt, column)?;
        let table = TableWcs::array_column(alt, naxis, column);
        let translated = table.translate(header)?;
        let mut h = translated.header;
        // Every axis lives in one column, so that column's pole and frame keywords
        // apply whichever axes turn out to be celestial.
        table.copy_celestial_keywords(header, &mut h, &[column])?;
        Wcs::from_header_with_context(&h, None, Vec::new(), Some(translated.spectral_frames))
    }

    /// Map 1-based pixel coordinates to complete world coordinates. Celestial axes
    /// return `(α, δ)` in degrees, nonlinear spectral axes use their Table-25
    /// default units, and other axes retain their declared units.
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::UnsupportedWcsTransform`] if any nonlinear axis is not
    /// implemented, or a projection error when the coordinate is outside its domain.
    ///
    pub fn pixel_to_world(&self, pixel: &[f64]) -> Result<Vec<f64>> {
        let naxis = self.axes.len();
        if pixel.len() != naxis {
            return Err(FitsError::RankMismatch {
                ranked: Ranked::WcsCoordinate,
                expected: naxis,
                got: pixel.len(),
            });
        }
        self.require_complete_transform()?;
        let intermediate = self.linear.intermediate(pixel, &self.axes);
        let mut world = (0..naxis)
            .map(|axis| {
                self.axis_transforms[axis].to_world(intermediate[axis], self.reference[axis], axis)
            })
            .collect::<Result<Vec<_>>>()?;
        for transform in &self.tabular {
            transform.to_world(&intermediate, &mut world)?;
        }
        if let Some(c) = &self.celestial {
            let native =
                c.projection
                    .deproject(intermediate[c.lng], intermediate[c.lat], &c.parameters)?;
            let celestial = c.pole.to_celestial(native.phi, native.theta);
            world[c.lng] = celestial.ra;
            world[c.lat] = celestial.dec;
        }
        Ok(world)
    }

    /// Map complete world coordinates back to 1-based pixel coordinates, the inverse
    /// of [`Wcs::pixel_to_world`].
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::UnsupportedWcsTransform`] if any nonlinear axis is not
    /// implemented, or a projection error when the coordinate is outside its domain.
    ///
    pub fn world_to_pixel(&self, world: &[f64]) -> Result<Vec<f64>> {
        let naxis = self.axes.len();
        if world.len() != naxis {
            return Err(FitsError::RankMismatch {
                ranked: Ranked::WcsCoordinate,
                expected: naxis,
                got: world.len(),
            });
        }
        self.require_complete_transform()?;
        let mut intermediate = (0..naxis)
            .map(|axis| {
                self.axis_transforms[axis].to_intermediate(world[axis], self.reference[axis], axis)
            })
            .collect::<Result<Vec<_>>>()?;
        for transform in &self.tabular {
            transform.to_intermediate(world, &mut intermediate)?;
        }
        if let Some(c) = self.celestial.as_ref() {
            if !world[c.lng].is_finite()
                || !world[c.lat].is_finite()
                || !(-90.0 - DOMAIN_TOLERANCE..=90.0 + DOMAIN_TOLERANCE).contains(&world[c.lat])
            {
                return Err(c.projection.world_domain_error());
            }
            let native = c.pole.to_native(world[c.lng], world[c.lat]);
            let projected = c
                .projection
                .project(native.phi, native.theta, &c.parameters)?;
            intermediate[c.lng] = projected.x;
            intermediate[c.lat] = projected.y;
        }
        Ok(self.linear.pixel(&intermediate, &self.axes))
    }

    /// The number of WCS axes an image header declares: `WCSAXESa`, else the larger
    /// of `NAXIS` and the highest axis any WCS keyword names.
    pub(crate) fn image_axis_count(header: &Header, alt: Option<char>) -> Result<usize> {
        let suffix = AltSuffix::new(alt);
        let value = match header.get_integer(key!("WCSAXES{suffix}").as_str())? {
            Some(axis_count) => axis_count,
            None => {
                let inferred = infer_image_axis_count(header, suffix.as_str());
                match header.get_integer("NAXIS")? {
                    Some(axis_count) if axis_count >= 0 => axis_count.max(inferred),
                    Some(axis_count) => axis_count,
                    None if inferred != 0 => inferred,
                    None => return Err(FitsError::MissingKeyword { name: "WCSAXES" }),
                }
            }
        };
        validated_axis_count(value, "WCSAXES")
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

/// Degrees per `CUNITia` of a celestial axis, whose absent unit is the degree (§8.2).
pub(crate) fn angle_scale(unit: &str) -> Result<f64> {
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
            header
                .get_real(key!("{prefix}{i}{alt}").as_str())
                .map(|value| value.unwrap_or(default))
        })
        .collect()
}

fn infer_image_axis_count(header: &Header, alt: &str) -> i64 {
    header
        .iter()
        .filter_map(|entry| image_wcs_axis_index(entry.keyword, alt))
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
fn validated_axis_count(value: i64, name: &'static str) -> Result<usize> {
    let count = usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { name })?;
    if !(1..=999).contains(&count) {
        return Err(FitsError::KeywordOutOfRange { name });
    }
    Ok(count)
}

/// The first of two keywords that carries a real value — the shape every
/// long/short and primary/`PVi_m` keyword alternative takes.
fn first_real(header: &Header, first: &str, second: &str) -> Result<Option<f64>> {
    match header.get_real(first)? {
        Some(value) => Ok(Some(value)),
        None => header.get_real(second),
    }
}

/// The quarter turn a multiple of 90° lands on (0 to 3), or `None` for any other angle.
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
    a.rem_euclid(360.0)
}

/// Normalize an angle to `[−180, 180)` degrees.
fn norm180(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// Golden pixel/world sets from `astropy.wcs` for the shared fixtures, and the
/// two-direction assertion over them — used by the test modules of the layers a
/// transform is assembled from, not just by [`Wcs`]'s own.
#[cfg(test)]
pub(crate) mod internals {
    use crate::world_coordinates::Wcs;

    /// Golden pixel→world values from `astropy.wcs` (wcslib) for `wcs_tan.fits`
    /// (`RA---TAN`/`DEC--TAN`, CRVAL 150/2.5, CRPIX 256/256, 1″ pixels, 15° rotation).
    /// Columns: pixel x, pixel y, RA (deg), Dec (deg).
    pub(crate) const TAN_GOLDEN: &[(f64, f64, f64, f64)] = &[
        (1.0, 1.0, 150.050131124369, 2.413246375001),
        (256.0, 256.0, 150.000000000000, 2.500000000000),
        (512.0, 512.0, 149.949665615474, 2.587091911566),
        (100.0, 400.0, 150.052260368590, 2.527420491210),
        (256.5, 256.5, 149.999901697142, 2.500170103464),
        (400.0, 123.0, 149.951756061540, 2.474666292235),
    ];

    pub(crate) const CEA_GOLDEN: &[(f64, f64, f64, f64)] = &[
        (20.0, 70.0, 46.7406870828, 30.4886140110),
        (80.0, 30.0, 43.2767613377, 29.4887155113),
    ];

    pub(crate) const CROTA_GOLDEN: &[(f64, f64, f64, f64)] = &[
        (128.0, 128.0, 83.6000000000, 22.0000000000),
        (1.0, 1.0, 83.6909943156, 21.9692606492),
        (256.0, 200.0, 83.5210288338, 22.0055606050),
        (64.0, 192.0, 83.6166986376, 22.0425247793),
    ];

    pub(crate) fn assert_astropy_golden(wcs: &Wcs, golden: &[(f64, f64, f64, f64)], context: &str) {
        for &(px, py, ra, dec) in golden {
            let world = wcs.pixel_to_world(&[px, py]).unwrap();
            assert!(
                (world[0] - ra).abs() < 1e-9 && (world[1] - dec).abs() < 1e-9,
                "{context} at ({px},{py}): got {world:?}, want ({ra},{dec})"
            );
        }
    }
}

#[cfg(test)]
mod tests;
