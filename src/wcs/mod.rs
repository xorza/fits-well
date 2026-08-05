//! Typed World Coordinate System (§8).
//!
//! Parses the per-axis WCS keywords from a [`Header`] and evaluates the standard
//! pixel↔world pipeline (Greisen & Calabretta, FITS WCS papers I & II):
//!
//! ```text
//! pixel ─ CRPIX ─►  ·(PC|CD, ×CDELT)  ─► intermediate coordinate
//!        ─► CTYPE algorithm ─► world coordinate
//! ```
//!
//! The linear layer is `PC`+`CDELT`, `CD`, or legacy `CDELT`+`CROTA`, with general
//! matrix inversion for the reverse direction, and full `PVi_m` parameters
//! (φ₀/θ₀/LONPOLE/LATPOLE overrides plus per-projection params). Projections, via
//! the general fiducial-point pole computation: zenithal `TAN`/`SIN`/`ARC`/`STG`/
//! `ZEA`/`ZPN`/`AIR`, zenithal-perspective `AZP`/`SZP`, cylindrical `CAR`/`CEA`/
//! `MER`/`SFL`/`CYP`, all-sky `AIT`/`MOL`/`PAR`, conic `COP`/`COE`/`COD`/`COO`,
//! pseudoconic `BON`, polyconic `PCO`, quad-cube `TSC`/`CSC`/`QSC`, and HEALPix
//! `HPX`. Every Table-26 spectral algorithm (`F2*`/`W2*`/`V2*`/`A2*`, detector
//! `GRI`/`GRA`, and generic `LOG`) is evaluated in both directions. `-TAB`
//! coordinate arrays are resolved from their BINTABLE through
//! [`FitsReader::read_wcs`](crate::FitsReader::read_wcs). All are validated against
//! `astropy.wcs`, wcslib, or exact interpolation fixtures. Convention-only `XPH`
//! transforms remain readable in [`WcsView::unsupported_axes`]; complete transforms
//! then return
//! [`FitsError::UnsupportedWcsTransform`].
//!
//! Binary-table WCS (Table 22) is supported for both the pixel-list
//! ([`Header::wcs_pixel_list`](crate::header::Header::wcs_pixel_list)) and vector-cell
//! ([`Header::wcs_array_column`](crate::header::Header::wcs_array_column)) forms.
//!
//! Pixel↔world yields celestial coordinates in the frame the file declares;
//! [`WcsView::celestial_frame`] and [`WcsAxis::spectral_frame`] expose that typed
//! `RADESYS`/`EQUINOX` and spectral frame/rest metadata. Converting *between*
//! reference frames is astrometry beyond the FITS standard and is intentionally
//! out of scope. Transform methods return explicit errors for invalid projection
//! domains or failed iterations.

use std::f64::consts::FRAC_PI_2;
use std::f64::consts::PI;

use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::AltSuffix;
use crate::keyword::KeyBuf;
use crate::keyword::key;
use crate::wcs::axis::AxisTransform;
use crate::wcs::axis::SpectralParameters;
use crate::wcs::axis::SpectralRest;
use crate::wcs::projection::NativeCoordinate;

mod axis;
#[cfg(feature = "internals")]
pub(crate) mod bench;
mod projection;
pub(crate) mod tabular;

const R2D: f64 = 180.0 / PI;
const D2R: f64 = PI / 180.0;
const DOMAIN_TOLERANCE: f64 = 1e-12;

/// A celestial projection algorithm — the 3-letter `CTYPE` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `TAN` — gnomonic (zenithal).
    Tan,
    /// `SIN` — orthographic/slant (zenithal).
    Sin,
    /// `ARC` — zenithal equidistant.
    Arc,
    /// `STG` — stereographic (zenithal).
    Stg,
    /// `ZEA` — zenithal equal-area.
    Zea,
    /// `CAR` — plate carrée (cylindrical).
    Car,
    /// `CEA` — cylindrical equal-area.
    Cea,
    /// `MER` — Mercator (cylindrical).
    Mer,
    /// `SFL` — Sanson–Flamsteed (pseudo-cylindrical).
    Sfl,
    /// `AIT` — Hammer–Aitoff (all-sky, pseudo-cylindrical).
    Ait,
    /// `MOL` — Mollweide (all-sky, pseudo-cylindrical).
    Mol,
    /// `ZPN` — zenithal polynomial (`PVi_m` coefficients).
    Zpn,
    /// `CYP` — cylindrical perspective (`μ = PVi_1`, `λ = PVi_2`).
    Cyp,
    /// `PAR` — parabolic (pseudo-cylindrical).
    Par,
    /// `COP` — conic perspective (`θ_a = PVi_1`, `η = PVi_2`).
    Cop,
    /// `COE` — conic equal-area.
    Coe,
    /// `COD` — conic equidistant.
    Cod,
    /// `COO` — conic orthomorphic.
    Coo,
    /// `BON` — Bonne's equal-area (pseudo-conic, `θ₁ = PVi_1`).
    Bon,
    /// `AIR` — Airy (zenithal, minimum-error; `θ_b = PVi_1`).
    Air,
    /// `AZP` — zenithal perspective (`μ = PVi_1`, tilt `γ = PVi_2`).
    Azp,
    /// `PCO` — polyconic.
    Pco,
    /// `SZP` — slant zenithal perspective (`μ = PVi_1`, `φc = PVi_2`, `θc = PVi_3`).
    Szp,
    /// `TSC` — tangential spherical cube.
    Tsc,
    /// `CSC` — COBE quadrilateralized spherical cube.
    Csc,
    /// `QSC` — quadrilateralized spherical cube.
    Qsc,
    /// `HPX` — HEALPix (`H = PVi_1`, `K = PVi_2`).
    Hpx,
}

#[derive(Debug, Clone, Copy)]
struct CelestialCoordinate {
    ra: f64,
    dec: f64,
}

/// The reference frame named by `RADESYSa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CelestialReferenceFrame {
    Icrs,
    Fk5,
    Fk4,
    Fk4NoE,
    Gappt,
}

impl CelestialReferenceFrame {
    fn parse(value: &str) -> Result<CelestialReferenceFrame> {
        match value.trim() {
            "ICRS" => Ok(CelestialReferenceFrame::Icrs),
            "FK5" => Ok(CelestialReferenceFrame::Fk5),
            "FK4" => Ok(CelestialReferenceFrame::Fk4),
            "FK4-NO-E" => Ok(CelestialReferenceFrame::Fk4NoE),
            "GAPPT" => Ok(CelestialReferenceFrame::Gappt),
            value => Err(FitsError::InvalidValue {
                card: format!("RADESYS {value:?} is not a standard reference frame"),
            }),
        }
    }
}

/// Resolved celestial-frame metadata for an equatorial/ecliptic WCS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CelestialFrame {
    /// `RADESYSa`, including the standard default selected from `EQUINOXa`.
    pub reference_frame: CelestialReferenceFrame,
    /// The declared non-negative `EQUINOXa`, or `None` when omitted.
    pub equinox: Option<f64>,
}

/// A standard spectral reference system from FITS Table 27.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpectralReferenceFrame {
    Topocentric,
    Geocentric,
    Barycentric,
    Heliocentric,
    LsrKinematic,
    LsrDynamic,
    Galactocentric,
    LocalGroup,
    CmbDipole,
    Source,
}

impl SpectralReferenceFrame {
    fn parse(keyword: &str, value: &str) -> Result<SpectralReferenceFrame> {
        match value.trim() {
            "TOPOCENT" => Ok(SpectralReferenceFrame::Topocentric),
            "GEOCENTR" => Ok(SpectralReferenceFrame::Geocentric),
            "BARYCENT" => Ok(SpectralReferenceFrame::Barycentric),
            "HELIOCEN" => Ok(SpectralReferenceFrame::Heliocentric),
            "LSRK" => Ok(SpectralReferenceFrame::LsrKinematic),
            "LSRD" => Ok(SpectralReferenceFrame::LsrDynamic),
            "GALACTOC" => Ok(SpectralReferenceFrame::Galactocentric),
            "LOCALGRP" => Ok(SpectralReferenceFrame::LocalGroup),
            "CMBDIPOL" => Ok(SpectralReferenceFrame::CmbDipole),
            "SOURCE" => Ok(SpectralReferenceFrame::Source),
            value => Err(FitsError::InvalidValue {
                card: format!("{keyword} {value:?} is not a standard spectral reference frame"),
            }),
        }
    }
}

/// Resolved reference-frame and rest-value metadata for a spectral WCS axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralFrame {
    /// `SPECSYSa`; `None` when the coordinate frame was not declared.
    pub coordinate: Option<SpectralReferenceFrame>,
    /// `SSYSOBSa`, including its `TOPOCENT` default.
    pub observer: SpectralReferenceFrame,
    /// `RESTFRQa` in Hz.
    pub rest_frequency_hz: Option<f64>,
    /// `RESTWAVa` in metres.
    pub rest_wavelength_m: Option<f64>,
}

/// Immutable source metadata for one WCS axis.
#[derive(Debug, Clone, PartialEq)]
pub struct WcsAxis {
    /// The `CTYPEi` string.
    pub ctype: String,
    /// Declared `CUNITi`, normalized only where the transform requires standard
    /// projection units.
    pub cunit: String,
    /// `CRVALi` — world coordinate at the reference pixel.
    pub crval: f64,
    /// `CRPIXi` — reference pixel (1-based).
    pub crpix: f64,
    /// Spectral reference metadata for a spectral axis.
    pub spectral_frame: Option<SpectralFrame>,
}

/// Public celestial-pair metadata for a parsed WCS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CelestialProjection {
    /// Zero-based longitude axis.
    pub longitude_axis: usize,
    /// Zero-based latitude axis.
    pub latitude_axis: usize,
    pub projection: Projection,
    /// Native-to-celestial pole `(right ascension/longitude, declination/latitude,
    /// LONPOLE)`, in degrees.
    pub pole: [f64; 3],
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

/// A parsed world coordinate system for one (optionally alternate) axis set.
#[derive(Debug, Clone)]
pub struct Wcs {
    axes: Vec<WcsAxis>,
    axis_transforms: Vec<AxisTransform>,
    /// Linear transform `A` mapping `(pixel − CRPIX)` to intermediate world
    /// coordinates: `PCi_j × CDELTi`, or `CDi_j` directly. Row-major `naxis²`.
    matrix: Vec<f64>,
    /// Inverse of `matrix`, for world→pixel.
    inverse: Vec<f64>,
    /// The (longitude axis, latitude axis, projection, celestial pole) when a
    /// celestial pair is present; `None` for an all-linear system.
    celestial: Option<Celestial>,
    celestial_frame: Option<CelestialFrame>,
    tabular: Vec<tabular::TabularTransform>,
    /// Axes (0-based) whose non-linear transform is not evaluated — an unsupported
    /// celestial projection or convention, or another non-linear coordinate
    /// algorithm (§8.3/§8.4). Complete transforms reject these axes.
    unsupported_axes: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AxisWorld<'a> {
    pub(crate) cunit: &'a str,
    pub(crate) value: f64,
}

/// The rotation from native to celestial coordinates: the celestial pole
/// `(α_p, δ_p)` and the native longitude of the pole `φ_p` (LONPOLE), all degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CelestialPole {
    ra: f64,
    dec: f64,
    lonpole: f64,
}

#[derive(Debug, Clone)]
struct Celestial {
    lng: usize,
    lat: usize,
    projection: Projection,
    /// The native→celestial pole, computed from the fiducial point.
    pole: CelestialPole,
    pv: [f64; 21],
}

#[derive(Debug, Clone, Copy)]
enum TableAxisKeyword {
    Type,
    Unit,
    ReferenceValue,
    Increment,
    ReferencePoint,
    Rotation,
}

impl TableAxisKeyword {
    const ALL: [TableAxisKeyword; 6] = [
        TableAxisKeyword::Type,
        TableAxisKeyword::Unit,
        TableAxisKeyword::ReferenceValue,
        TableAxisKeyword::Increment,
        TableAxisKeyword::ReferencePoint,
        TableAxisKeyword::Rotation,
    ];

    fn table_root(self, alternate: bool) -> Option<&'static str> {
        match (self, alternate) {
            (TableAxisKeyword::Type, false) => Some("CTYP"),
            (TableAxisKeyword::Type, true) => Some("CTY"),
            (TableAxisKeyword::Unit, false) => Some("CUNI"),
            (TableAxisKeyword::Unit, true) => Some("CUN"),
            (TableAxisKeyword::ReferenceValue, false) => Some("CRVL"),
            (TableAxisKeyword::ReferenceValue, true) => Some("CRV"),
            (TableAxisKeyword::Increment, false) => Some("CDLT"),
            (TableAxisKeyword::Increment, true) => Some("CDE"),
            (TableAxisKeyword::ReferencePoint, false) => Some("CRPX"),
            (TableAxisKeyword::ReferencePoint, true) => Some("CRP"),
            (TableAxisKeyword::Rotation, false) => Some("CROT"),
            (TableAxisKeyword::Rotation, true) => None,
        }
    }

    fn image_root(self) -> &'static str {
        match self {
            TableAxisKeyword::Type => "CTYPE",
            TableAxisKeyword::Unit => "CUNIT",
            TableAxisKeyword::ReferenceValue => "CRVAL",
            TableAxisKeyword::Increment => "CDELT",
            TableAxisKeyword::ReferencePoint => "CRPIX",
            TableAxisKeyword::Rotation => "CROTA",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TableMatrixKeyword {
    Pc,
    Cd,
}

impl TableMatrixKeyword {
    fn root(self) -> &'static str {
        match self {
            TableMatrixKeyword::Pc => "PC",
            TableMatrixKeyword::Cd => "CD",
        }
    }

    fn pixel_root(self, abbreviated: bool) -> &'static str {
        match (self, abbreviated) {
            (TableMatrixKeyword::Pc, false) => "TPC",
            (TableMatrixKeyword::Pc, true) => "TP",
            (TableMatrixKeyword::Cd, false) => "TCD",
            (TableMatrixKeyword::Cd, true) => "TC",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TablePoleKeyword {
    Longitude,
    Latitude,
}

impl TablePoleKeyword {
    fn table_root(self) -> &'static str {
        match self {
            TablePoleKeyword::Longitude => "LONP",
            TablePoleKeyword::Latitude => "LATP",
        }
    }

    fn image_root(self) -> &'static str {
        match self {
            TablePoleKeyword::Longitude => "LONPOLE",
            TablePoleKeyword::Latitude => "LATPOLE",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TableWcsResolver {
    suffix: AltSuffix,
}

impl TableWcsResolver {
    fn new(alternate: Option<char>) -> TableWcsResolver {
        TableWcsResolver {
            suffix: AltSuffix::new(alternate),
        }
    }

    fn pixel_axis_key(self, keyword: TableAxisKeyword, column: usize) -> Option<KeyBuf> {
        let root = keyword.table_root(self.suffix.is_alternate())?;
        let suffix = self.suffix;
        Some(key!("T{root}{column}{suffix}"))
    }

    fn vector_axis_key(
        self,
        keyword: TableAxisKeyword,
        axis: usize,
        column: usize,
    ) -> Option<KeyBuf> {
        let root = keyword.table_root(self.suffix.is_alternate())?;
        let suffix = self.suffix;
        Some(key!("{axis}{root}{column}{suffix}"))
    }

    fn pixel_matrix_key(
        self,
        keyword: TableMatrixKeyword,
        row_column: usize,
        input_column: usize,
        abbreviated: bool,
    ) -> KeyBuf {
        let root = keyword.pixel_root(abbreviated);
        let suffix = self.suffix;
        key!("{root}{row_column}_{input_column}{suffix}")
    }

    fn pixel_matrix_real(
        self,
        header: &Header,
        keyword: TableMatrixKeyword,
        row_column: usize,
        input_column: usize,
    ) -> Result<Option<f64>> {
        let long = self.pixel_matrix_key(keyword, row_column, input_column, false);
        let short = self.pixel_matrix_key(keyword, row_column, input_column, true);
        first_real(header, long.as_str(), short.as_str())
    }

    fn vector_matrix_key(
        self,
        keyword: TableMatrixKeyword,
        row_axis: usize,
        input_axis: usize,
        column: usize,
    ) -> KeyBuf {
        let root = keyword.root();
        let suffix = self.suffix;
        key!("{row_axis}{input_axis}{root}{column}{suffix}")
    }

    fn pixel_parameter_key(self, column: usize, parameter: usize, short: bool) -> KeyBuf {
        let root = if short { "TV" } else { "TPV" };
        let suffix = self.suffix;
        key!("{root}{column}_{parameter}{suffix}")
    }

    fn pixel_parameter_real(
        self,
        header: &Header,
        column: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        let long = self.pixel_parameter_key(column, parameter, false);
        let short = self.pixel_parameter_key(column, parameter, true);
        first_real(header, long.as_str(), short.as_str())
    }

    fn vector_parameter_key(
        self,
        axis: usize,
        column: usize,
        parameter: usize,
        short: bool,
    ) -> KeyBuf {
        let root = if short { "V" } else { "PV" };
        let suffix = self.suffix;
        key!("{axis}{root}{column}_{parameter}{suffix}")
    }

    fn vector_parameter_real(
        self,
        header: &Header,
        axis: usize,
        column: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        let long = self.vector_parameter_key(axis, column, parameter, false);
        let short = self.vector_parameter_key(axis, column, parameter, true);
        first_real(header, long.as_str(), short.as_str())
    }

    fn vector_string_parameter_key(
        self,
        axis: usize,
        column: usize,
        parameter: usize,
        short: bool,
    ) -> KeyBuf {
        let root = if short { "S" } else { "PS" };
        let suffix = self.suffix;
        key!("{axis}{root}{column}_{parameter}{suffix}")
    }

    fn column_key(self, root: &str, column: usize) -> KeyBuf {
        let suffix = self.suffix;
        key!("{root}{column}{suffix}")
    }

    fn pole_real(
        self,
        header: &Header,
        pole: TablePoleKeyword,
        column: usize,
    ) -> Result<Option<f64>> {
        let key = self.column_key(pole.table_root(), column);
        header.get_real(key.as_str())
    }

    fn vector_rank(self, header: &Header, column: usize) -> Result<Option<i64>> {
        let key = self.column_key("WCAX", column);
        header.get_integer(key.as_str())
    }

    fn vector_axis_present(self, header: &Header, column: usize, axis: usize) -> bool {
        if TableAxisKeyword::ALL.into_iter().any(|keyword| {
            self.vector_axis_key(keyword, axis, column)
                .is_some_and(|key| header.get(key.as_str()).is_some())
        }) {
            return true;
        }
        if (0..=99).any(|parameter| {
            [false, true].into_iter().any(|short| {
                let real = self.vector_parameter_key(axis, column, parameter, short);
                let text = self.vector_string_parameter_key(axis, column, parameter, short);
                header.get(real.as_str()).is_some() || header.get(text.as_str()).is_some()
            })
        }) {
            return true;
        }
        (1..=99).any(|other| {
            [TableMatrixKeyword::Pc, TableMatrixKeyword::Cd]
                .into_iter()
                .any(|keyword| {
                    let row = self.vector_matrix_key(keyword, axis, other, column);
                    let input = self.vector_matrix_key(keyword, other, axis, column);
                    header.get(row.as_str()).is_some() || header.get(input.as_str()).is_some()
                })
        })
    }
}

/// How a binary-table header addresses the axes of one WCS (Table 22): a pixel list
/// gives each coordinate axis its own column (`TCTYPn`), while an array cell holds
/// every axis inside one column, indexed by array axis (`iCTYPn`).
#[derive(Debug, Clone, Copy)]
enum TableWcsForm<'a> {
    PixelList(&'a [usize]),
    ArrayColumn { naxis: usize, column: usize },
}

/// A Table 22 WCS description: how its axes are addressed, plus the resolver that
/// spells each keyword family for the selected alternate.
#[derive(Debug, Clone, Copy)]
struct TableWcs<'a> {
    resolver: TableWcsResolver,
    form: TableWcsForm<'a>,
}

/// A Table 22 description rewritten as the equivalent image header. The spectral
/// frames travel alongside rather than inside it: they are column-indexed, so they
/// have no image-keyword spelling.
#[derive(Debug)]
struct TranslatedTableWcs {
    header: Header,
    spectral_frames: Vec<Option<SpectralFrame>>,
}

impl TableWcs<'_> {
    fn naxis(&self) -> usize {
        match self.form {
            TableWcsForm::PixelList(columns) => columns.len(),
            TableWcsForm::ArrayColumn { naxis, .. } => naxis,
        }
    }

    /// The table column carrying zero-based axis `index` — the pole, spectral, and
    /// celestial-frame keywords are column-indexed in both forms.
    fn column(&self, index: usize) -> usize {
        match self.form {
            TableWcsForm::PixelList(columns) => columns[index],
            TableWcsForm::ArrayColumn { column, .. } => column,
        }
    }

    fn axis_key(&self, keyword: TableAxisKeyword, index: usize) -> Option<KeyBuf> {
        match self.form {
            TableWcsForm::PixelList(columns) => {
                self.resolver.pixel_axis_key(keyword, columns[index])
            }
            TableWcsForm::ArrayColumn { column, .. } => {
                self.resolver.vector_axis_key(keyword, index + 1, column)
            }
        }
    }

    fn parameter_real(
        &self,
        header: &Header,
        index: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        match self.form {
            TableWcsForm::PixelList(columns) => {
                self.resolver
                    .pixel_parameter_real(header, columns[index], parameter)
            }
            TableWcsForm::ArrayColumn { column, .. } => {
                self.resolver
                    .vector_parameter_real(header, index + 1, column, parameter)
            }
        }
    }

    fn matrix_real(
        &self,
        header: &Header,
        keyword: TableMatrixKeyword,
        row: usize,
        input: usize,
    ) -> Result<Option<f64>> {
        match self.form {
            TableWcsForm::PixelList(columns) => {
                self.resolver
                    .pixel_matrix_real(header, keyword, columns[row], columns[input])
            }
            TableWcsForm::ArrayColumn { column, .. } => {
                let source = self
                    .resolver
                    .vector_matrix_key(keyword, row + 1, input + 1, column);
                header.get_real(source.as_str())
            }
        }
    }

    /// Rewrite the Table 22 keywords as the equivalent image header — axis type and
    /// unit, reference point/value/increment/rotation, `PVi_m`, and the linear
    /// transform — so both table forms evaluate through the same pipeline as an image
    /// WCS. The celestial pole and frame keywords are left to the caller: the two
    /// forms choose their source column differently.
    fn translate(&self, header: &Header) -> Result<TranslatedTableWcs> {
        let naxis = self.naxis();
        let mut h = Header::new();
        h.set_internal("WCSAXES", naxis as i64);
        let mut spectral_frames = vec![None; naxis];
        for (index, spectral) in spectral_frames.iter_mut().enumerate() {
            let ax = index + 1;
            let type_key = self
                .axis_key(TableAxisKeyword::Type, index)
                .expect("Table 22 defines primary and alternate axis-type keywords");
            if let Some(t) = header.get_text(type_key.as_str())? {
                h.set_internal(key!("CTYPE{ax}").as_str(), t);
                if axis::is_spectral_type(t) {
                    *spectral = Some(table_spectral_frame(
                        header,
                        self.resolver,
                        self.column(index),
                    )?);
                }
            }
            let unit_key = self
                .axis_key(TableAxisKeyword::Unit, index)
                .expect("Table 22 defines primary and alternate axis-unit keywords");
            if let Some(t) = header.get_text(unit_key.as_str())? {
                h.set_internal(key!("CUNIT{ax}").as_str(), t);
            }
            for keyword in [
                TableAxisKeyword::ReferencePoint,
                TableAxisKeyword::ReferenceValue,
                TableAxisKeyword::Increment,
                TableAxisKeyword::Rotation,
            ] {
                if let Some(source) = self.axis_key(keyword, index)
                    && let Some(value) = header.get_real(source.as_str())?
                {
                    h.set_internal(key!("{}{ax}", keyword.image_root()).as_str(), value);
                }
            }
            // `PVi_m` arrives as `TPVn_ma`/`TVn_ma`, or `iPVn_ma`/`iVn_ma`.
            for m in 0..=20 {
                if let Some(v) = self.parameter_real(header, index, m)? {
                    h.set_internal(key!("PV{ax}_{m}").as_str(), v);
                }
            }
        }
        // Linear transform: `TPCn_ka`/`TCDn_ka` by column pair, or `ijPCna`/`ijCDna`
        // by axis pair.
        for row in 0..naxis {
            for input in 0..naxis {
                for keyword in [TableMatrixKeyword::Pc, TableMatrixKeyword::Cd] {
                    if let Some(value) = self.matrix_real(header, keyword, row, input)? {
                        h.set_internal(
                            key!("{}{}_{}", keyword.root(), row + 1, input + 1).as_str(),
                            value,
                        );
                    }
                }
            }
        }
        Ok(TranslatedTableWcs {
            header: h,
            spectral_frames,
        })
    }
}

impl Wcs {
    /// Parse the primary WCS (`alt = None`) or an alternate description
    /// (`alt = Some('A'..='Z')`) from `header`. The public entry point is
    /// [`Header::wcs`](crate::header::Header::wcs), which forwards here.
    pub(crate) fn from_header(header: &Header, alt: Option<char>) -> Result<Wcs> {
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
        let naxis = image_axis_count(header, alt)?;

        let ctype: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CTYPE{i}{a}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let mut crval = axis_vec(header, "CRVAL", a.as_str(), naxis, 0.0)?;
        let crpix = axis_vec(header, "CRPIX", a.as_str(), naxis, 0.0)?;
        let cdelt = axis_vec(header, "CDELT", a.as_str(), naxis, 1.0)?;
        let cunit: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CUNIT{i}{a}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let celestial_axes = find_celestial(&ctype)?;
        let celestial_frame = celestial_frame(header, alt, a.as_str(), &ctype)?;
        let spectral_frames = match spectral_frames {
            Some(frames) => {
                assert_eq!(frames.len(), naxis, "spectral frame count");
                frames
            }
            None => {
                let frame = ctype
                    .iter()
                    .any(|ctype| axis::is_spectral_type(ctype))
                    .then(|| spectral_frame(header, alt, a.as_str()))
                    .transpose()?;
                ctype
                    .iter()
                    .map(|ctype| axis::is_spectral_type(ctype).then_some(frame).flatten())
                    .collect()
            }
        };

        let mut matrix = linear_transform(header, a, naxis, &cdelt, celestial_axes)?;
        // §8.2: CRVAL/CDELT are in CUNITia units, but the projection math runs in
        // degrees — scale each celestial axis's reference value and its matrix row
        // (the inverse is computed after, so both directions stay consistent).
        if let Some(axes) = celestial_axes {
            let lng = axes.longitude;
            let lat = axes.latitude;
            for ax in [lng, lat] {
                let f = unit_to_degrees(&cunit[ax]);
                crval[ax] *= f;
                for j in 0..naxis {
                    matrix[ax * naxis + j] *= f;
                }
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
            if celestial_axis(&ctype[axis]).is_some() {
                axis_transforms.push(AxisTransform::Linear);
                let resolved_tabular =
                    projection_code(&ctype[axis]) == Some("TAB") && resolved_tabular_axes[axis];
                if !resolved_tabular
                    && projection_code(&ctype[axis])
                        .is_some_and(|code| Projection::from_code(code).is_none())
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
            let parsed =
                AxisTransform::parse(&ctype[axis], &cunit[axis], crval[axis], rest, parameters)?;
            crval[axis] *= parsed.unit_scale;
            for column in 0..naxis {
                matrix[axis * naxis + column] *= parsed.unit_scale;
            }
            let resolved_tabular =
                projection_code(&ctype[axis]) == Some("TAB") && resolved_tabular_axes[axis];
            if matches!(&parsed.transform, AxisTransform::Unsupported) && !resolved_tabular {
                unsupported_axes.push(axis);
            }
            axis_transforms.push(if resolved_tabular {
                AxisTransform::Linear
            } else {
                parsed.transform
            });
        }
        let inverse = invert(&matrix, naxis).ok_or(FitsError::InvalidValue {
            card: "singular WCS transform matrix".to_string(),
        })?;

        let celestial =
            celestial_transform(header, a, celestial_axes, &crval, &mut unsupported_axes)?;

        let axes = ctype
            .into_iter()
            .enumerate()
            .map(|(i, ctype)| WcsAxis {
                ctype,
                cunit: cunit[i].clone(),
                crval: crval[i],
                crpix: crpix[i],
                spectral_frame: spectral_frames[i],
            })
            .collect();
        Ok(Wcs {
            axes,
            axis_transforms,
            matrix,
            inverse,
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
                    pole: [
                        celestial.pole.ra,
                        celestial.pole.dec,
                        celestial.pole.lonpole,
                    ],
                }),
            unsupported_axes: &self.unsupported_axes,
        }
    }

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
        if let Some(transform) = self
            .tabular
            .iter()
            .find(|transform| transform.axes.contains(&axis))
        {
            let intermediate: Vec<f64> = transform
                .axes
                .iter()
                .map(|&image_axis| {
                    let row = &self.matrix[image_axis * naxis..(image_axis + 1) * naxis];
                    matvec_pixel_offset_row(row, pixel, &self.axes)
                })
                .collect();
            return Ok(AxisWorld {
                cunit: &self.axes[axis].cunit,
                value: transform.to_world_axis(axis, &intermediate)?,
            });
        }
        let row = &self.matrix[axis * naxis..(axis + 1) * naxis];
        let intermediate = matvec_pixel_offset_row(row, pixel, &self.axes);
        Ok(AxisWorld {
            cunit: &self.axes[axis].cunit,
            value: self.axis_transforms[axis].to_world(
                intermediate,
                self.axes[axis].crval,
                axis,
            )?,
        })
    }

    /// Build a WCS for a binary-table **pixel list** (event list, §8.5, Table 22):
    /// `columns` lists the 1-based table column numbers forming the coordinate axes
    /// in order. Reads both the primary and shortened alternate column-indexed
    /// families, both matrix/parameter spellings (`TPC`/`TP`, `TCD`/`TC`,
    /// `TPV`/`TV`), and the longitude column's `LONPna`/`LATPna` pole keywords,
    /// then evaluates them through the same pipeline as image WCS.
    pub(crate) fn from_pixel_list(
        header: &Header,
        columns: &[usize],
        alt: Option<char>,
    ) -> Result<Wcs> {
        let table = TableWcs {
            resolver: TableWcsResolver::new(alt),
            form: TableWcsForm::PixelList(columns),
        };
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
        if let Some(pair) = find_celestial_pair(&ctype) {
            let longitude = columns[pair.longitude];
            let latitude = columns[pair.latitude];
            for pole in [TablePoleKeyword::Longitude, TablePoleKeyword::Latitude] {
                if let Some(value) = table.resolver.pole_real(header, pole, longitude)? {
                    h.set_internal(pole.image_root(), value);
                }
            }
            copy_table_celestial_frame(header, &mut h, table.resolver, &[longitude, latitude])?;
        }
        Wcs::from_header_with_context(&h, None, Vec::new(), Some(translated.spectral_frames))
    }

    /// Build a WCS for an image stored in a binary-table **vector cell** (§8,
    /// Table 22): `column` is the 1-based table column whose cells hold a
    /// multidimensional array. Reads the primary and shortened alternate
    /// axis-and-column-indexed families, `ijPCna`/`ijCDna`, `iPVn_ma`/`iVn_ma`,
    /// and `LONPna`/`LATPna`, where `i`/`j` are array axes and `n` is the column.
    /// The rank is taken from `WCAXna`, else inferred through the same resolver.
    pub(crate) fn from_array_column(
        header: &Header,
        column: usize,
        alt: Option<char>,
    ) -> Result<Wcs> {
        let resolver = TableWcsResolver::new(alt);
        let naxis = match resolver.vector_rank(header, column)? {
            Some(value) => axis_count(value, "WCAXn")?,
            // Highest-first over the axes the cards could name, so the first hit is
            // the rank. Probing a blind `(1..=99).rev()` instead costs ~800 header
            // lookups per axis that turns out to be absent.
            None => candidate_vector_axes(header)
                .into_iter()
                .find(|&axis| resolver.vector_axis_present(header, column, axis))
                .unwrap_or(0),
        };
        if naxis == 0 {
            return Err(FitsError::MissingKeyword { name: "iCTYPn" });
        }
        let table = TableWcs {
            resolver,
            form: TableWcsForm::ArrayColumn { naxis, column },
        };
        let translated = table.translate(header)?;
        let mut h = translated.header;
        // Every axis lives in one column, so that column's pole and frame keywords
        // apply whichever axes turn out to be celestial.
        for pole in [TablePoleKeyword::Longitude, TablePoleKeyword::Latitude] {
            if let Some(value) = resolver.pole_real(header, pole, column)? {
                h.set_internal(pole.image_root(), value);
            }
        }
        copy_table_celestial_frame(header, &mut h, resolver, &[column])?;
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
        let intermediate = matvec_pixel_offset(&self.matrix, pixel, &self.axes);
        let mut world = (0..naxis)
            .map(|axis| {
                self.axis_transforms[axis].to_world(intermediate[axis], self.axes[axis].crval, axis)
            })
            .collect::<Result<Vec<_>>>()?;
        for transform in &self.tabular {
            transform.to_world(&intermediate, &mut world)?;
        }
        if let Some(c) = &self.celestial {
            let native = c
                .projection
                .deproject(intermediate[c.lng], intermediate[c.lat], &c.pv)?;
            let celestial = native_to_celestial(c.pole, native.phi, native.theta);
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
                self.axis_transforms[axis].to_intermediate(world[axis], self.axes[axis].crval, axis)
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
                return Err(c.projection.domain_error());
            }
            let native = celestial_to_native(c.pole, world[c.lng], world[c.lat]);
            let projected = c.projection.project(native.phi, native.theta, &c.pv)?;
            intermediate[c.lng] = projected.x;
            intermediate[c.lat] = projected.y;
        }
        let mut pixel = matvec(&self.inverse, &intermediate, naxis);
        for (value, axis) in pixel.iter_mut().zip(&self.axes) {
            *value += axis.crpix;
        }
        Ok(pixel)
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

/// Which celestial coordinate an axis carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CelestialAxis {
    Longitude,
    Latitude,
}

#[derive(Debug, Clone, Copy)]
struct CelestialAxisPair {
    longitude: usize,
    latitude: usize,
}

#[derive(Debug, Clone, Copy)]
struct ProjectedCelestialAxes {
    longitude: usize,
    latitude: usize,
    projection: Projection,
}

/// The celestial coordinate an axis carries, from its `CTYPE` head (§8.2): `RA` and
/// the `xLON`/`yzLN` forms are longitudes; `DEC` and `xLAT`/`yzLT` are latitudes;
/// `None` for any non-celestial axis.
fn celestial_axis(ctype: &str) -> Option<CelestialAxis> {
    let head = ctype.split('-').next().unwrap_or("").trim();
    if head == "RA" || head.ends_with("LON") || (head.len() == 4 && head.ends_with("LN")) {
        Some(CelestialAxis::Longitude)
    } else if head == "DEC" || head.ends_with("LAT") || (head.len() == 4 && head.ends_with("LT")) {
        Some(CelestialAxis::Latitude)
    } else {
        None
    }
}

/// The trailing projection/algorithm code of a `CTYPE` (`RA---TAN` → `TAN`); `None`
/// when there is no hyphen-delimited suffix (a bare `RA`/`GLON`).
fn projection_code(ctype: &str) -> Option<&str> {
    ctype
        .rsplit_once('-')
        .map(|(_, code)| code)
        .map(str::trim_end)
        .filter(|c| !c.is_empty())
}

fn celestial_frame(
    header: &Header,
    alt: Option<char>,
    suffix: &str,
    ctype: &[String],
) -> Result<Option<CelestialFrame>> {
    let key = key!("RADESYS{suffix}");
    let mut declared = header.get_text(key.as_str())?;
    if declared.is_none() && alt.is_none() {
        declared = header.get_text("RADECSYS")?;
    }
    let equinox = header.get_real(key!("EQUINOX{suffix}").as_str())?;
    if equinox.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err(FitsError::InvalidValue {
            card: "EQUINOX must be finite and non-negative".to_string(),
        });
    }
    let applies = ctype.iter().any(|ctype| {
        matches!(
            ctype.split('-').next().unwrap_or("").trim(),
            "RA" | "DEC" | "ELON" | "ELAT"
        )
    });
    if !applies && declared.is_none() && equinox.is_none() {
        return Ok(None);
    }
    let reference_frame = match declared {
        Some(value) => CelestialReferenceFrame::parse(value)?,
        None => match equinox {
            Some(value) if value < 1984.0 => CelestialReferenceFrame::Fk4,
            Some(_) => CelestialReferenceFrame::Fk5,
            None => CelestialReferenceFrame::Icrs,
        },
    };
    Ok(Some(CelestialFrame {
        reference_frame,
        equinox,
    }))
}

fn spectral_frame(header: &Header, alt: Option<char>, suffix: &str) -> Result<SpectralFrame> {
    let rest = spectral_rest(header, alt, suffix)?;
    let coordinate = header
        .get_text(key!("SPECSYS{suffix}").as_str())?
        .map(|value| SpectralReferenceFrame::parse("SPECSYS", value))
        .transpose()?;
    let observer = header
        .get_text(key!("SSYSOBS{suffix}").as_str())?
        .map(|value| SpectralReferenceFrame::parse("SSYSOBS", value))
        .transpose()?
        .unwrap_or(SpectralReferenceFrame::Topocentric);
    Ok(SpectralFrame {
        coordinate,
        observer,
        rest_frequency_hz: rest.frequency,
        rest_wavelength_m: rest.wavelength,
    })
}

fn spectral_rest(header: &Header, alt: Option<char>, suffix: &str) -> Result<SpectralRest> {
    let mut frequency = header.get_real(key!("RESTFRQ{suffix}").as_str())?;
    if frequency.is_none() && alt.is_none() {
        frequency = header.get_real("RESTFREQ")?;
    }
    let wavelength = header.get_real(key!("RESTWAV{suffix}").as_str())?;
    SpectralRest::new(frequency, wavelength)
}

fn table_spectral_frame(
    header: &Header,
    resolver: TableWcsResolver,
    column: usize,
) -> Result<SpectralFrame> {
    let mut translated = Header::new();
    for (table_root, image_root) in [("RFRQ", "RESTFRQ"), ("RWAV", "RESTWAV")] {
        let source = resolver.column_key(table_root, column);
        if let Some(value) = header.get_real(source.as_str())? {
            translated.set_internal(image_root, value);
        }
    }
    for (table_root, image_root) in [("SPEC", "SPECSYS"), ("SOBS", "SSYSOBS")] {
        let source = resolver.column_key(table_root, column);
        if let Some(value) = header.get_text(source.as_str())? {
            translated.set_internal(image_root, value);
        }
    }
    spectral_frame(&translated, None, "")
}

fn copy_table_celestial_frame(
    source: &Header,
    destination: &mut Header,
    resolver: TableWcsResolver,
    columns: &[usize],
) -> Result<()> {
    for &column in columns {
        let radesys = resolver.column_key("RADE", column);
        if let Some(value) = source.get_text(radesys.as_str())? {
            if let Some(existing) = destination.get_text("RADESYS")?
                && existing != value
            {
                return Err(FitsError::ConflictingWcsKeywords {
                    detail: "table celestial axes declare different RADESYS values",
                });
            }
            destination.set_internal("RADESYS", value);
        }
        let equinox = resolver.column_key("EQUI", column);
        if let Some(value) = source.get_real(equinox.as_str())? {
            if let Some(existing) = destination.get_real("EQUINOX")?
                && existing != value
            {
                return Err(FitsError::ConflictingWcsKeywords {
                    detail: "table celestial axes declare different EQUINOX values",
                });
            }
            destination.set_internal("EQUINOX", value);
        }
    }
    Ok(())
}

/// Degrees per `CUNITia` angle unit; `1.0` for an absent, unknown, or `deg` unit.
fn unit_to_degrees(unit: &str) -> f64 {
    match unit.trim() {
        "arcmin" => 1.0 / 60.0,
        "arcsec" => 1.0 / 3600.0,
        "mas" => 1.0 / 3_600_000.0,
        "rad" => R2D,
        _ => 1.0, // "deg", "", or anything unrecognized
    }
}

/// Resolve the celestial pair's projection parameters and its native→celestial pole
/// (CG 2002 §2.4), or `None` when the header declares no complete celestial pair.
///
/// `unsupported_axes` gains the pair when the projection is present but cannot be
/// evaluated, so a complete transform refuses rather than returning `NaN`.
fn celestial_transform(
    header: &Header,
    a: AltSuffix,
    celestial_axes: Option<ProjectedCelestialAxes>,
    crval: &[f64],
    unsupported_axes: &mut Vec<usize>,
) -> Result<Option<Celestial>> {
    let Some(axes) = celestial_axes else {
        return Ok(None);
    };
    let lng = axes.longitude;
    let lat = axes.latitude;
    let proj = axes.projection;
    let mut pv = proj.parameter_defaults();
    for (m, value) in pv.iter_mut().enumerate() {
        if let Some(header_value) = header.get_real(key!("PV{}_{m}{a}", lat + 1).as_str())? {
            *value = header_value;
        }
    }
    proj.validate_parameters(&pv)?;
    // A conic's mid-latitude θ_a = PVi_1 is mandatory and must be non-zero; θ_a = 0
    // (absent, or explicitly 0) is a degenerate cone (`1/tan 0`). Treat it like an
    // unimplemented projection — flag the axes so complete transforms fail rather
    // than returning NaN.
    if proj.is_conic() && pv[1] == 0.0 {
        unsupported_axes.push(lng);
        unsupported_axes.push(lat);
        unsupported_axes.sort_unstable();
        return Ok(None);
    }
    // Fiducial point: projection default, overridable by PVi_1a/PVi_2a on the
    // longitude axis (§8.3).
    let reference = proj.reference_point(&pv);
    let (mut phi0, mut theta0) = (reference.phi, reference.theta);
    if let Some(v) = header.get_real(key!("PV{}_1{a}", lng + 1).as_str())? {
        phi0 = v;
    }
    if let Some(v) = header.get_real(key!("PV{}_2{a}", lng + 1).as_str())? {
        theta0 = v;
    }
    let (alpha0, delta0) = (crval[lng], crval[lat]);
    // LONPOLE (= LONPOLEa or PVi_3a): default φ0 if δ0 ≥ θ0, else φ0 + 180°.
    let phip = first_real(
        header,
        key!("LONPOLE{a}").as_str(),
        key!("PV{}_3{a}", lng + 1).as_str(),
    )?
    .unwrap_or(if delta0 >= theta0 { phi0 } else { phi0 + 180.0 });
    // LATPOLE (= LATPOLEa or PVi_4a): default 90°.
    let thetap = first_real(
        header,
        key!("LATPOLE{a}").as_str(),
        key!("PV{}_4{a}", lng + 1).as_str(),
    )?
    .unwrap_or(90.0);
    Ok(Some(Celestial {
        lng,
        lat,
        projection: proj,
        pole: compute_pole(phi0, theta0, alpha0, delta0, phip, thetap),
        pv,
    }))
}

/// Build the linear transform `A` mapping `(pixel − CRPIX)` to intermediate world
/// coordinates, row-major `naxis²`.
///
/// Precedence (§8.1): `CDi_j` if present, else `PCi_j × CDELTi`, else the legacy
/// `CROTAi` rotation of the celestial pair, else a bare `CDELT` diagonal. The
/// conventions are mutually exclusive, so a header mixing them is rejected rather
/// than silently resolved.
fn linear_transform(
    header: &Header,
    a: AltSuffix,
    naxis: usize,
    cdelt: &[f64],
    celestial_axes: Option<ProjectedCelestialAxes>,
) -> Result<Vec<f64>> {
    let has_cd = (1..=naxis)
        .any(|i| (1..=naxis).any(|j| header.get(key!("CD{i}_{j}{a}").as_str()).is_some()));
    let has_pc = (1..=naxis)
        .any(|i| (1..=naxis).any(|j| header.get(key!("PC{i}_{j}{a}").as_str()).is_some()));
    let has_crota = (1..=naxis).any(|i| header.get(key!("CROTA{i}{a}").as_str()).is_some());
    if has_cd && has_pc {
        return Err(FitsError::ConflictingWcsKeywords {
            detail: "PC and CD conventions overlap",
        });
    }
    if has_pc && has_crota {
        return Err(FitsError::ConflictingWcsKeywords {
            detail: "PC and CROTA conventions overlap",
        });
    }
    let mut matrix = vec![0.0; naxis * naxis];
    if has_cd {
        for i in 0..naxis {
            for j in 0..naxis {
                matrix[i * naxis + j] = header
                    .get_real(key!("CD{}_{}{a}", i + 1, j + 1).as_str())?
                    .unwrap_or(0.0);
            }
        }
        return Ok(matrix);
    }
    for i in 0..naxis {
        for j in 0..naxis {
            let pc = header
                .get_real(key!("PC{}_{}{a}", i + 1, j + 1).as_str())?
                .unwrap_or(if i == j { 1.0 } else { 0.0 });
            matrix[i * naxis + j] = cdelt[i] * pc;
        }
    }
    // Legacy CROTA: rotate the celestial 2-axis sub-block (only when no PC was
    // given, per the convention that CROTA and PC are exclusive).
    if !has_pc && let Some(axes) = celestial_axes {
        let lng = axes.longitude;
        let lat = axes.latitude;
        let rho = first_real(
            header,
            key!("CROTA{}{a}", lat + 1).as_str(),
            key!("CROTA{}{a}", lng + 1).as_str(),
        )?
        .unwrap_or(0.0);
        if rho != 0.0 {
            let (c, s) = ((rho * D2R).cos(), (rho * D2R).sin());
            matrix[lng * naxis + lng] = cdelt[lng] * c;
            matrix[lng * naxis + lat] = -cdelt[lat] * s;
            matrix[lat * naxis + lng] = cdelt[lng] * s;
            matrix[lat * naxis + lat] = cdelt[lat] * c;
        }
    }
    Ok(matrix)
}

fn find_celestial_pair(ctype: &[String]) -> Option<CelestialAxisPair> {
    let mut lng = None;
    let mut lat = None;
    for (i, t) in ctype.iter().enumerate() {
        match celestial_axis(t) {
            Some(CelestialAxis::Longitude) => lng = lng.or(Some(i)),
            Some(CelestialAxis::Latitude) => lat = lat.or(Some(i)),
            None => {}
        }
    }
    let (Some(lng), Some(lat)) = (lng, lat) else {
        return None;
    };
    Some(CelestialAxisPair {
        longitude: lng,
        latitude: lat,
    })
}

/// Locate the celestial longitude/latitude axis pair and their shared projection,
/// or `None` if the header has no complete supported pair. Errors if the two axes
/// declare different projection codes.
fn find_celestial(ctype: &[String]) -> Result<Option<ProjectedCelestialAxes>> {
    let Some(pair) = find_celestial_pair(ctype) else {
        return Ok(None);
    };
    if projection_code(&ctype[pair.longitude]) != projection_code(&ctype[pair.latitude]) {
        return Err(FitsError::ConflictingWcsKeywords {
            detail: "celestial longitude and latitude axes declare different projections",
        });
    }
    Ok(projection_code(&ctype[pair.longitude])
        .and_then(Projection::from_code)
        .map(|projection| ProjectedCelestialAxes {
            longitude: pair.longitude,
            latitude: pair.latitude,
            projection,
        }))
}

/// Native spherical (φ, θ) → celestial (α, δ), all degrees, given the celestial
/// pole `(α_p, δ_p, φ_p)` (CG 2002 eq. 2).
fn native_to_celestial(pole: CelestialPole, phi: f64, theta: f64) -> CelestialCoordinate {
    let CelestialPole {
        ra: ap,
        dec: dp,
        lonpole: fp,
    } = pole;
    let (tr, dpr, dphi) = (theta * D2R, dp * D2R, (phi - fp) * D2R);
    let sin_d = tr.sin() * dpr.sin() + tr.cos() * dpr.cos() * dphi.cos();
    let dec = sin_d.clamp(-1.0, 1.0).asin() * R2D;
    let y = -tr.cos() * dphi.sin();
    let x = tr.sin() * dpr.cos() - tr.cos() * dpr.sin() * dphi.cos();
    CelestialCoordinate {
        ra: norm360(ap + y.atan2(x) * R2D),
        dec,
    }
}

/// Celestial (α, δ) → native spherical (φ, θ), all degrees (CG 2002 eq. 5).
fn celestial_to_native(pole: CelestialPole, ra: f64, dec: f64) -> NativeCoordinate {
    let CelestialPole {
        ra: ap,
        dec: dp,
        lonpole: fp,
    } = pole;
    let (dr, dpr, dalpha) = (dec * D2R, dp * D2R, (ra - ap) * D2R);
    let sin_t = dr.sin() * dpr.sin() + dr.cos() * dpr.cos() * dalpha.cos();
    let theta = sin_t.clamp(-1.0, 1.0).asin() * R2D;
    let y = -dr.cos() * dalpha.sin();
    let x = dr.sin() * dpr.cos() - dr.cos() * dpr.sin() * dalpha.cos();
    NativeCoordinate {
        phi: norm180(fp + y.atan2(x) * R2D),
        theta,
    }
}

/// Compute the celestial pole `(α_p, δ_p, φ_p)` from the fiducial point
/// `(φ₀, θ₀) → (α₀, δ₀)`, `φ_p` (LONPOLE), and `θ_p` (LATPOLE) (CG 2002 §2.4).
/// Zenithal (`θ₀ = 90°`) reduces to `(α₀, δ₀, φ_p)`.
fn compute_pole(phi0: f64, theta0: f64, a0: f64, d0: f64, phip: f64, thetap: f64) -> CelestialPole {
    if (theta0 - 90.0).abs() < 1e-12 {
        return CelestialPole {
            ra: a0,
            dec: d0,
            lonpole: phip,
        };
    }
    let (t0, d0r) = (theta0 * D2R, d0 * D2R);
    let dphi = (phip - phi0) * D2R;
    // sinδ0 = sinθ0·sinδ_p + cosθ0·cos(φ_p−φ0)·cosδ_p = R·cos(δ_p − β).
    let a = t0.sin();
    let b = t0.cos() * dphi.cos();
    let rmag = a.hypot(b);
    let beta = a.atan2(b);
    let ac = (d0r.sin() / rmag).clamp(-1.0, 1.0).acos();
    // Two δ_p solutions; pick the one in range nearest LATPOLE.
    let c1 = beta + ac;
    let c2 = beta - ac;
    let in_range = |x: f64| (-FRAC_PI_2..=FRAC_PI_2).contains(&x);
    let dpr = match (in_range(c1), in_range(c2)) {
        (true, true) => {
            if (c1 - thetap * D2R).abs() <= (c2 - thetap * D2R).abs() {
                c1
            } else {
                c2
            }
        }
        (true, false) => c1,
        (false, true) => c2,
        (false, false) => c1.clamp(-FRAC_PI_2, FRAC_PI_2),
    };
    let dp = dpr * R2D;
    // α_p from the fiducial constraint (inverting eq. 2 at (φ0, θ0)).
    let fphi = (phi0 - phip) * D2R;
    let y = -t0.cos() * fphi.sin();
    let x = t0.sin() * dpr.cos() - t0.cos() * dpr.sin() * fphi.cos();
    let ap = a0 - y.atan2(x) * R2D;
    CelestialPole {
        ra: norm360(ap),
        dec: dp,
        lonpole: phip,
    }
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

/// Every array axis a Table-22 vector keyword could name, gathered in one pass over
/// the header and returned highest-first.
///
/// Each vector family spells its array axis in the keyword's *leading digits* —
/// `iCTYPn`, `iPVn_ma`, and for the matrix forms `ijPCna`/`ijCDna` a pair of them —
/// so an axis that is genuinely present always appears as a contiguous digit
/// substring of some card's leading run. Taking every such substring deliberately
/// over-approximates: [`TableWcsResolver::vector_axis_present`] stays the exact
/// test, and this only spares it the axes no card could possibly mention.
///
/// The alternative — parsing each family's grammar in reverse — has to disentangle
/// `ijPCna` from `iCTYPn` and the abbreviated alternate roots from the primary ones
/// they prefix, and a mistake there silently changes an image's rank. Over-
/// approximating cannot: a spurious candidate is rejected, and a missed one is
/// impossible by the substring argument above.
fn candidate_vector_axes(header: &Header) -> Vec<usize> {
    let mut axes: Vec<usize> = Vec::new();
    for entry in header.iter() {
        let digits = entry
            .keyword
            .find(|c: char| !c.is_ascii_digit())
            .map_or(entry.keyword, |end| &entry.keyword[..end]);
        for start in 0..digits.len() {
            for end in start + 1..=digits.len() {
                let text = &digits[start..end];
                // A FITS index never carries a leading zero, so `03` is not axis 3.
                if text.starts_with('0') {
                    continue;
                }
                match text.parse::<usize>() {
                    Ok(axis) if (1..=99).contains(&axis) && !axes.contains(&axis) => {
                        axes.push(axis);
                    }
                    _ => {}
                }
            }
        }
    }
    axes.sort_unstable_by(|a, b| b.cmp(a));
    axes
}

fn infer_image_axis_count(header: &Header, alt: &str) -> i64 {
    header
        .iter()
        .filter_map(|entry| image_wcs_axis_index(entry.keyword, alt))
        .max()
        .unwrap_or(0)
}

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
    axis_count(value, "WCSAXES")
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

fn axis_count(value: i64, name: &'static str) -> Result<usize> {
    let count = usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { name })?;
    if !(1..=999).contains(&count) {
        return Err(FitsError::KeywordOutOfRange { name });
    }
    Ok(count)
}

fn first_real(header: &Header, first: &str, second: &str) -> Result<Option<f64>> {
    match header.get_real(first)? {
        Some(value) => Ok(Some(value)),
        None => header.get_real(second),
    }
}

fn matvec(matrix: &[f64], vector: &[f64], size: usize) -> Vec<f64> {
    matrix
        .chunks_exact(size)
        .map(|row| row.iter().zip(vector).map(|(&a, &b)| a * b).sum())
        .collect()
}

fn matvec_pixel_offset(matrix: &[f64], pixel: &[f64], axes: &[WcsAxis]) -> Vec<f64> {
    matrix
        .chunks_exact(axes.len())
        .map(|row| matvec_pixel_offset_row(row, pixel, axes))
        .collect()
}

fn matvec_pixel_offset_row(row: &[f64], pixel: &[f64], axes: &[WcsAxis]) -> f64 {
    row.iter()
        .zip(pixel)
        .zip(axes)
        .map(|((&coefficient, &value), axis)| coefficient * (value - axis.crpix))
        .sum()
}

/// Invert a row-major `n×n` matrix by Gauss–Jordan elimination with partial
/// pivoting. Returns `None` if singular.
fn invert(m: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut a = m.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for col in 0..n {
        // Partial pivot: largest magnitude in this column at or below the diagonal.
        let mut pivot = col;
        for r in (col + 1)..n {
            if a[r * n + col].abs() > a[pivot * n + col].abs() {
                pivot = r;
            }
        }
        if a[pivot * n + col].abs() < 1e-300 {
            return None;
        }
        if pivot != col {
            for k in 0..n {
                a.swap(col * n + k, pivot * n + k);
                inv.swap(col * n + k, pivot * n + k);
            }
        }
        let d = a[col * n + col];
        for k in 0..n {
            a[col * n + k] /= d;
            inv[col * n + k] /= d;
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r * n + col];
            if f != 0.0 {
                for k in 0..n {
                    a[r * n + k] -= f * a[col * n + k];
                    inv[r * n + k] -= f * inv[col * n + k];
                }
            }
        }
    }
    Some(inv)
}

/// Normalize an angle to `[0, 360)` degrees.
fn norm360(a: f64) -> f64 {
    a.rem_euclid(360.0)
}

/// Normalize an angle to `[−180, 180)` degrees.
fn norm180(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests;
