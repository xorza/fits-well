//! The World Coordinate System parse and transforms, published as [`crate::wcs`].

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::error::Result;
use crate::error::WcsConflict;
use crate::header_model::record::RecordView;
use crate::header_model::{Header, keywords};
use crate::image_model::region::Region;
use crate::keyword::Keyword;
use crate::world_coordinates::axis::Axis;
use crate::world_coordinates::axis_plan::{AxisKind, AxisPlan};
use crate::world_coordinates::axis_transform::AxisTransform;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralParameters;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;
use crate::world_coordinates::celestial_frame::CelestialFrame;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::celestial_transform::{
    CelestialPair, CelestialResolution, CelestialTransform, PvMeaning,
};
use crate::world_coordinates::ctype::Ctype;
use crate::world_coordinates::declared::Declared;
use crate::world_coordinates::distortion::Distortion;
use crate::world_coordinates::distortion::sip::Sip;
use crate::world_coordinates::header_read::AxisValues;
use crate::world_coordinates::linear_transform::LinearMatrix;
use crate::world_coordinates::linear_transform::LinearTransform;
use crate::world_coordinates::native_rotation::CelestialCoordinate;
use crate::world_coordinates::projected_celestial_axes::CelestialAxisPair;
use crate::world_coordinates::projected_celestial_axes::ProjectedCelestialAxes;
use crate::world_coordinates::projection::Projection;
use crate::world_coordinates::spectral_frame::SpectralFrame;
use crate::world_coordinates::table_wcs::TableWcs;
use crate::world_coordinates::world_unit::WorldUnit;

mod angle;
pub(crate) mod axis;
mod axis_plan;
pub(crate) mod axis_transform;
#[cfg(feature = "internals")]
pub(crate) mod bench;
mod celestial_axis;
pub(crate) mod celestial_frame;
pub(crate) mod celestial_pole;
mod celestial_transform;
pub(crate) mod ctype;
mod declared;
pub(crate) mod distortion;
mod header_read;
mod linear_transform;
mod native_coordinate;
mod native_rotation;
mod projected_celestial_axes;
pub(crate) mod projection;
pub(crate) mod spectral_frame;
mod table_wcs;
pub(crate) mod tabular;
mod world_unit;

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
    /// The distortion the transform applies, if any.
    pub distortion: Option<Distortion>,
}

/// One axis's world coordinate, from [`Wcs::axis_world`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisWorld<'a> {
    /// The unit of `value`, as [`Wcs::world_unit`] gives it.
    pub cunit: &'a str,
    pub value: f64,
}

/// A parsed world coordinate system for one (optionally alternate) axis set.
#[derive(Debug, Clone)]
pub struct Wcs {
    axes: Vec<Axis>,
    plans: Vec<AxisPlan>,
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
    /// The description as the header declared it, which `write_into` writes back.
    declared: Declared,
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
                header_read::text(header, Keyword::fitting(format_args!("CTYPE{i}{alt}")))
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let AxisValues {
            crval: declared_crval,
            crpix,
            cdelt,
        } = header_read::axis_values(header, alt.as_str(), naxis)?;
        let cunit: Vec<String> = (1..=naxis)
            .map(|i| {
                header_read::text(header, Keyword::fitting(format_args!("CUNIT{i}{alt}")))
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
        let mut resolved_tabular_axes = vec![false; naxis];
        for transform in &tabular {
            for &axis in &transform.axes {
                resolved_tabular_axes[axis] = true;
            }
        }
        // Each axis's `CUNITia` factor, applied to its `CRVAL` and to its whole matrix
        // row: the projection math runs in degrees and the spectral algorithms in their
        // Table-25 default units, so the two must scale together (§8.2).
        let mut axis_scales = vec![1.0; naxis];
        let mut crval = declared_crval.clone();
        let mut plans = Vec::with_capacity(naxis);
        let mut unsupported_axes = Vec::new();
        for axis in 0..naxis {
            let parsed_ctype = Ctype::parse(&ctype[axis]);
            let tabular_axis = parsed_ctype.algorithm == Some("TAB");
            // A `-TAB` axis whose coordinate array was resolved from its BINTABLE is
            // evaluated by the tabular transform, so its own algorithm is never
            // consulted — and never counts as unsupported. Its coordinates come out in
            // the unit the table's were scaled to.
            if tabular_axis && resolved_tabular_axes[axis] {
                plans.push(AxisPlan {
                    kind: AxisKind::Tabular,
                    transform: AxisTransform::Joint,
                    world_unit: WorldUnit::of(parsed_ctype, &cunit[axis])?.unit,
                });
                continue;
            }
            let kind = if tabular_axis {
                AxisKind::Tabular
            } else if parsed_ctype.celestial_axis().is_some() {
                AxisKind::Celestial
            } else {
                AxisKind::Other
            };
            if parsed_ctype.celestial_axis().is_some() {
                // A projection this crate does not evaluate, or an axis with no partner
                // of its own system, has no transform.
                let unpaired = !celestial_pair.is_some_and(|pair| pair.contains(axis));
                if unpaired
                    || parsed_ctype.algorithm.is_some() && parsed_ctype.projection().is_none()
                {
                    unsupported_axes.push(axis);
                }
                let projected = celestial_axes
                    .is_some_and(|axes| axis == axes.longitude || axis == axes.latitude);
                let unit = if projected {
                    let unit = WorldUnit::of(parsed_ctype, &cunit[axis])?;
                    axis_scales[axis] = unit.scale;
                    crval[axis] *= unit.scale;
                    unit.unit
                } else {
                    None
                };
                plans.push(AxisPlan {
                    kind,
                    transform: AxisTransform::Joint,
                    world_unit: unit,
                });
                continue;
            }
            let mut spectral_parameters = [None; 7];
            for (parameter, value) in spectral_parameters.iter_mut().enumerate() {
                *value = header_read::real(
                    header,
                    Keyword::fitting(format_args!("PV{}_{parameter}{alt}", axis + 1)),
                )?;
            }
            let parameters = SpectralParameters::new(spectral_parameters);
            let rest = spectral_frames[axis].map_or(SpectralRest::NONE, |frame| frame.rest);
            let spec =
                AxisTransform::parse(&ctype[axis], &cunit[axis], crval[axis], rest, parameters)?;
            axis_scales[axis] = spec.unit.scale;
            if matches!(&spec.transform, AxisTransform::Unsupported) {
                unsupported_axes.push(axis);
            }
            plans.push(AxisPlan {
                kind,
                transform: spec.transform,
                world_unit: spec.unit.unit,
            });
        }
        // SIP comes from its keywords, with or without the `-SIP` code, as wcslib and
        // astropy read it; its keywords carry no alternate letter, and a `-TPV` code
        // drops them, as astropy does (`wcs.py`, `_read_sip_kw`, `_fix_pre2012`).
        let code = celestial_axes.and_then(|axes| axes.distortion);
        let sip = if alt == Alternate::PRIMARY && code != Some(Distortion::Tpv) {
            Sip::from_header(header)?
        } else {
            None
        };
        if code == Some(Distortion::Sip) && alt == Alternate::PRIMARY && sip.is_none() {
            return Err(FitsError::MissingKeyword {
                keyword: Keyword::new("A_ORDER"),
            });
        }
        if sip.is_some() && naxis < 2 {
            return Err(FitsError::ConflictingWcsKeywords {
                conflict: WcsConflict::SipAxisCount,
            });
        }
        let pv_meaning = match celestial_axes {
            Some(axes) if sip.is_some() && has_scamp_terms(header, alt, axes) => {
                PvMeaning::Distortion
            }
            _ => PvMeaning::Projection,
        };
        let declared = Declared::read(header, alt, naxis, matrix.declared())?;
        let linear = matrix.scaled(&axis_scales, crpix.clone(), sip)?;

        let celestial = match celestial_axes {
            None => None,
            Some(axes) => {
                let pair = CelestialPair {
                    axes,
                    reference: CelestialCoordinate {
                        ra: crval[axes.longitude],
                        dec: crval[axes.latitude],
                    },
                    pv_meaning,
                };
                match CelestialTransform::from_header(header, alt, pair)? {
                    CelestialResolution::Transform(transform) => Some(transform),
                    CelestialResolution::DegenerateCone => {
                        unsupported_axes.extend([axes.longitude, axes.latitude]);
                        unsupported_axes.sort_unstable();
                        None
                    }
                }
            }
        };

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
            plans,
            linear,
            celestial,
            celestial_frame,
            tabular,
            unsupported_axes,
            declared,
        })
    }

    /// The WCS of the part of the image `region` selects, its ranges' starts subtracted
    /// from `CRPIX`: the `Region` the image read takes, so one value crops the image and
    /// its WCS. `Region::All` gives this WCS. Errors as [`Wcs::write_into`], and with
    /// `RankMismatch` for a region of another rank.
    pub fn crop(&self, region: Region<'_>) -> Result<Self> {
        let Region::Section(ranges) = region else {
            return Ok(self.clone());
        };
        if ranges.len() != self.axes.len() {
            return Err(FitsError::RankMismatch {
                ranked: Ranked::ImageRegion,
                expected: self.axes.len(),
                got: ranges.len(),
            });
        }
        self.derive(|header| {
            for (axis, range) in ranges.iter().enumerate() {
                let crpix = Keyword::try_from(format_args!("CRPIX{}", axis + 1))?;
                let value = header.require_real(crpix)? - range.start as f64;
                header.set(crpix, value)?;
            }
            Ok(())
        })
    }

    /// The WCS of the image binned by `factors`, one per pixel axis, each block of
    /// `b` pixels one pixel. On axis `j`, `CRPIX` becomes `(CRPIX − 0.5)/b + 0.5`, the
    /// centre of the first block, as astropy's `slice` with a step gives it, and column
    /// `j` of `PC` is multiplied by `b`, as a bin scales a pixel axis. With factors `b₁`
    /// and `b₂` on the SIP axes, `A_p_q` becomes `A_p_q·b₁^p·b₂^q/b₁` and `B_p_q`
    /// `B_p_q·b₁^p·b₂^q/b₂`, as the offsets from `CRPIX` scale by `b` and the focal-plane
    /// offset is divided by `b` again; `AP` and `BP` scale alike, and TPV, after the
    /// matrix, stays. Errors as [`Wcs::write_into`], and with `RankMismatch` for
    /// factors of another count.
    ///
    /// # Panics
    ///
    /// When a factor is 0.
    pub fn bin(&self, factors: &[usize]) -> Result<Self> {
        let naxis = self.axes.len();
        if factors.len() != naxis {
            return Err(FitsError::RankMismatch {
                ranked: Ranked::BinFactors,
                expected: naxis,
                got: factors.len(),
            });
        }
        assert!(factors.iter().all(|&b| b > 0), "a bin factor is at least 1");
        let b: Vec<f64> = factors.iter().map(|&b| b as f64).collect();
        self.derive(|header| {
            for (j, &bj) in b.iter().enumerate() {
                let crpix = Keyword::try_from(format_args!("CRPIX{}", j + 1))?;
                let value = (header.require_real(crpix)? - 0.5) / bj + 0.5;
                header.set(crpix, value)?;
                for i in 0..naxis {
                    let pc = Keyword::try_from(format_args!("PC{}_{}", i + 1, j + 1))?;
                    let value = header
                        .get_real(pc)?
                        .unwrap_or(if i == j { 1.0 } else { 0.0 });
                    header.set(pc, value * bj)?;
                }
            }
            let sip_terms: Vec<(Keyword, f64)> = header
                .iter()
                .filter_map(|record| {
                    let RecordView::Valued { keyword, value, .. } = record else {
                        return None;
                    };
                    let (root, p, q) = sip_term(keyword.as_str())?;
                    let value = value.as_real().ok().flatten()?;
                    // `A` and `AP` correct the first axis, `B` and `BP` the second.
                    let divisor = if root.starts_with('A') { b[0] } else { b[1] };
                    Some((keyword, value * b[0].powi(p) * b[1].powi(q) / divisor))
                })
                .collect();
            for (keyword, value) in sip_terms {
                header.set(keyword, value)?;
            }
            Ok(())
        })
    }

    /// The WCS `edit` makes of this one's primary-form header.
    fn derive(&self, edit: impl FnOnce(&mut Header) -> Result<()>) -> Result<Self> {
        let mut header = Header::new();
        self.write_into(&mut header, None)?;
        edit(&mut header)?;
        Self::from_header(&header, None)
    }

    /// Write this description into `header` as its description `alt` (`None` for the
    /// primary one), in the image-axis form of §8.2 and wcslib's layout, as `from_header`
    /// reads it, whatever form it was read from: a description from
    /// [`Wcs::from_pixel_list`] or [`Wcs::from_array_column`] is written with image
    /// keywords too. First every record of that description is
    /// removed, so no `CD` stays beside the `PC` written; then `WCSAXES`, `CRPIX`, `PCi_j`,
    /// `CDELT`, `CUNIT`, `CTYPE`, `CRVAL`, the pole, frame and spectral keywords and
    /// `PVi_m` as they were read, and the SIP terms. [`Wcs::from_header`] of the result
    /// gives back this transform. A `-TAB` axis is `UnsupportedWcsTransform`, as its
    /// arrays live in another HDU; SIP as an alternate is `ConflictingWcsKeywords`, as its
    /// keywords have no alternate form.
    pub fn write_into(
        &self,
        header: &mut Header,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<()> {
        let tabular: Vec<usize> = (0..self.axes.len())
            .filter(|&axis| self.plans[axis].kind == AxisKind::Tabular)
            .collect();
        if !tabular.is_empty() {
            return Err(FitsError::UnsupportedWcsTransform { axes: tabular });
        }
        self.declared
            .write(header, Alternate::of(alt)?, &self.axes, self.linear.sip())
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
                    projection: celestial.parameters.projection(),
                    pole: celestial.rotation.pole,
                }),
            unsupported_axes: &self.unsupported_axes,
            distortion: self.distortion(),
        }
    }

    /// The distortion the transform applies: SIP before the matrix, or TPV after it.
    fn distortion(&self) -> Option<Distortion> {
        if self.linear.sip().is_some() {
            Some(Distortion::Sip)
        } else {
            self.celestial
                .as_ref()
                .and_then(|celestial| celestial.tpv.as_ref())
                .map(|_| Distortion::Tpv)
        }
    }

    /// The unit the world coordinates of the zero-based `axis` come out in: degrees for a
    /// celestial axis, the Table-25 default unit for a spectral one, also where a table
    /// holds its coordinates, and the declared `CUNITi` for any other.
    pub fn world_unit(&self, axis: usize) -> &str {
        self.plans[axis]
            .world_unit
            .unwrap_or(&self.axes[axis].cunit)
    }

    /// The world coordinate of the zero-based `axis` at `pixel`, with the unit it is in.
    /// Only that axis is evaluated, with its celestial pair or its coordinate table, so a
    /// transform another axis lacks does not stop it. A time axis's value goes on to
    /// [`time::Axis::mjd`](crate::time::Axis::mjd).
    pub fn axis_world(&self, axis: usize, pixel: &[f64]) -> Result<AxisWorld<'_>> {
        let naxis = self.axes.len();
        if axis >= naxis {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::WcsAxis,
                index: axis.saturating_add(1),
                len: naxis,
            });
        }
        self.require_point(pixel)?;
        if self.unsupported_axes.contains(&axis) {
            return Err(FitsError::UnsupportedWcsTransform { axes: vec![axis] });
        }
        let offsets = self.linear.offsets(pixel);
        let intermediate = |axis| self.linear.intermediate_axis(axis, pixel, offsets);
        let plan = &self.plans[axis];
        let value = match plan.kind {
            AxisKind::Tabular => self
                .tabular
                .iter()
                .find(|transform| transform.axes.contains(&axis))
                .expect("a supported TAB axis is resolved")
                .to_world_axis(axis, intermediate)?,
            AxisKind::Celestial => {
                let c = self
                    .celestial
                    .as_ref()
                    .expect("a supported celestial axis is projected");
                let world = c.to_world(intermediate(c.lng), intermediate(c.lat))?;
                if axis == c.lng { world.ra } else { world.dec }
            }
            AxisKind::Other => plan.transform.to_world(intermediate(axis), axis)?,
        };
        Ok(AxisWorld {
            cunit: self.world_unit(axis),
            value,
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
        self.linear.intermediate(pixel, world);
        for (axis, value) in world.iter_mut().enumerate() {
            *value = self.plans[axis].transform.to_world(*value, axis)?;
        }
        for transform in &self.tabular {
            transform.to_world(world)?;
        }
        if let Some(c) = &self.celestial {
            let celestial = c.to_world(world[c.lng], world[c.lat])?;
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
            *value = self.plans[axis].transform.to_intermediate(world, axis)?;
        }
        for transform in &self.tabular {
            transform.to_intermediate(pixel)?;
        }
        if let Some(c) = &self.celestial {
            [pixel[c.lng], pixel[c.lat]] = c.to_intermediate(world[c.lng], world[c.lat])?;
        }
        self.linear.pixel(pixel)
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
            header_read::integer(header, Keyword::fitting(format_args!("WCSAXES{alt}")))?
        {
            axis_count
        } else {
            let inferred = header_read::inferred_axis_count(header, alt.as_str());
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
        header_read::axis_count(value, keywords::WCSAXES)
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

/// The root and powers of a SIP term `A_p_q`, `B_p_q`, `AP_p_q` or `BP_p_q`.
fn sip_term(name: &str) -> Option<(&str, i32, i32)> {
    let (root, rest) = name.split_once('_')?;
    if !matches!(root, "A" | "B" | "AP" | "BP") {
        return None;
    }
    let (p, q) = rest.split_once('_')?;
    let digit = |text: &str| (text.len() == 1).then(|| text.parse().ok()).flatten();
    Some((root, digit(p)?, digit(q)?))
}

/// Whether the celestial pair carries SCAMP's TPV terms, `PVi_m` with `m` of 5 or more,
/// which no projection takes beside SIP (astropy `wcs.py`, `_fix_scamp`).
fn has_scamp_terms(header: &Header, alt: Alternate, axes: ProjectedCelestialAxes) -> bool {
    [axes.longitude, axes.latitude].into_iter().any(|axis| {
        (5..=39).any(|m| {
            header_read::value(
                header,
                Keyword::fitting(format_args!("PV{}_{m}{alt}", axis + 1)),
            )
            .is_some()
        })
    })
}

/// Read `PREFIX1..PREFIXn` (with alternate suffix) into a vector, defaulting
/// missing entries.
/// Golden pixel→world sets from `astropy.wcs` for the shared fixtures, and the
/// assertion over them — used by the test modules of the layers a transform is
/// assembled from, not just by [`Wcs`]'s own.
#[cfg(test)]
pub(crate) mod internals {
    use crate::error::Result;
    use crate::world_coordinates::Wcs;
    use crate::world_coordinates::angle::DEGREE_TOLERANCE;

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

    /// CEA with λ = `PV2_1` = 0.5, CRPIX 50/50, CRVAL 45/30, CDELT (−0.05, 0.05).
    pub(crate) const CEA_GOLDEN: AstropyGolden<'_> = AstropyGolden {
        decimals: 10,
        points: &[
            (20.0, 70.0, 46.740_687_082_8, 30.488_614_011_0),
            (80.0, 30.0, 43.276_761_337_7, 29.488_715_511_3),
        ],
    };

    /// TAN with `CROTA2` = 25°, CRPIX 128/128, CRVAL 83.6/22, CDELT (−5e-4, 5e-4).
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
