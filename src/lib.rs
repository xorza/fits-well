//! A blazing-fast reader and writer for **FITS** (Flexible Image Transport
//! System) files — the standard data format of astronomy.
//!
//! # Layering
//!
//! The format's structure maps onto a stack of layers, so the hot decode path
//! stays lean and the semantic layers compute only on demand. WCS (§8) and time
//! (§9) are dependency-free, always compiled, and parsed from a header on request
//! ([`wcs::Wcs::from_header`], [`time::Frame::from_header`]); tiled compression
//! carries a dependency and stays behind the `compression` feature.
//!
//! ```text
//! bytes ─► block layer ─► HDU layer ─► header model ─► typed data
//!         (2880 grid,    (boundary    (ordered        (images,
//!          padding,       scan, lazy   records +       tables,
//!          I/O quantum)   seeking)     keyword index)  heap, VLAs)
//! ```
//!
//! - [`io::BLOCK_SIZE`] — the 2880-byte block grid, padding rules, and rounding math.
//! - [`image::Bitpix`] — the array element type selector (`BITPIX`).
//! - [`header::Header`], [`header::Value`] — an *ordered* header model
//!   (an internal `Card` list)
//!   whose logical records round-trip with a side index for O(1) keyword lookup;
//!   physical card layout is normalized on write rather than retained.
//! - [`io::HduKind`] — HDU classification; the reader sizes each data unit from its
//!   header, so the boundaries are known with no data read.
//! - [`FitsReader`] — lazy, seeking access to the HDU sequence of a file.
//!
//! # Status
//!
//! The structural spine (blocks, headers, HDU boundaries, lazy reading) plus
//! typed image decode/encode ([`image::Image`]), the multi-HDU writer ([`FitsWriter`]),
//! ASCII/binary tables, WCS, time coordinates, and tiled image+table compression
//! are implemented and tested — see each module's docs for its design.
#![cfg_attr(docsrs, feature(doc_cfg))]

// The README's `rust` snippets are compiled by `cargo test --doc` so they can't
// silently drift from the API. They are `no_run` (they name on-disk files), so this
// checks the API surface, not execution. `#[cfg(doctest)]` pulls the file in only
// while rustdoc collects doctests, so the README's title and badges never surface in
// the rendered crate docs and the item never exists in a normal build.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

mod allocation;
mod alternate;
mod ascii;
mod bintable;
mod blank;
mod block;
mod checksum;
mod column;
#[cfg(feature = "compression")]
mod compression;
mod endian;
mod fits_editor;
mod fits_error;
mod fits_writer;
mod groups;
mod hdu;
mod header_model;
mod image_model;
mod inline_name;
mod keyword;
mod ragged;
mod reader;
mod reserved_keywords;
mod time_coordinates;
mod unit;
mod world_coordinates;

pub use fits_editor::FitsEditor;
pub use fits_error::{FitsError, Result};
pub use fits_writer::FitsWriter;
pub use fits_writer::image_writer::ImageWriter;
pub use reader::FitsReader;
pub use reader::image_reader::{ImageReader, ViewBytes};

/// The cases a [`FitsError`] variant carries: what was indexed, ranked or expected,
/// and why a value, a WCS or a compressed tile fails.
pub mod error {
    pub use crate::fits_error::{
        AsciiValueDefect, CompressedDataDefect, CompressionFeature, CompressionParameter,
        DescriptorField, ExpectedColumn, ExpectedHdu, GrismDefect, HcompressDefect, Indexed,
        KeywordDefect, NonFinite, NumericTarget, PlioDefect, PoleDefect, Ranked, SizeUnit,
        TabularDefect, TextOrigin, TileCell, TimeConversion, UnitKind, ValueKind, WcsConflict,
        WcsDefect,
    };
}

/// Tiled compression (§10): the codec, its quantization and dither, and the tile
/// shape, for images and binary tables alike.
#[cfg(feature = "compression")]
pub mod compress {
    pub use crate::compression::quantization::{Dither, DitherSeed, Quantization};
    pub use crate::compression::{Compression, CompressionConfig, Gzip, Hcompress};
}

/// Typed image values, scratch-backed views, and incremental output.
pub mod image {
    pub use crate::image_model::bitpix::Bitpix;
    pub use crate::image_model::data::Data;
    pub use crate::image_model::physical_sample::PhysicalSample;
    pub use crate::image_model::region::Region;
    pub use crate::image_model::sample::Sample;
    pub use crate::image_model::sample_type::SampleType;
    pub use crate::image_model::scaling::Scaling;
    pub use crate::image_model::unsigned_data::UnsignedData;
    pub use crate::image_model::unsigned_element::UnsignedElement;
    pub use crate::image_model::unsigned_view::UnsignedView;
    pub use crate::image_model::view::{Borrowed, View};
    pub use crate::image_model::{Image, Metadata, Parts};
}

/// The ordered header model: keyword records and their typed values.
pub mod header {
    pub use crate::header_model::commentary_name::CommentaryName;
    pub use crate::header_model::defect::Defect;
    pub use crate::header_model::hierarch_name::HierarchName;
    pub use crate::header_model::keywords;
    pub use crate::header_model::record::{Record, RecordView};
    pub use crate::header_model::record_name::RecordName;
    pub use crate::header_model::repair::{Repair, RepairKind};
    pub use crate::header_model::value::{FitsInteger, Value};
    pub use crate::header_model::{Header, Iter};
    pub use crate::keyword::Keyword;
}

pub mod wcs {
    //! Typed World Coordinate System (§8).
    //!
    //! Parses the per-axis WCS keywords from a [`Header`](crate::header::Header) and
    //! evaluates the standard pixel↔world pipeline (Greisen & Calabretta, FITS WCS
    //! papers I & II):
    //!
    //! ```text
    //! pixel ─ CRPIX ─►  ·(PC|CD, ×CDELT)  ─► intermediate coordinate
    //!        ─► CTYPE algorithm ─► world coordinate
    //! ```
    //!
    //! The linear layer is `PC`+`CDELT`, `CD`, or legacy `CDELT`+`CROTA`, solved through
    //! its LU factors for the reverse direction, and full `PVi_m` parameters
    //! (φ₀/θ₀/LONPOLE/LATPOLE overrides plus per-projection params). Projections, via
    //! the general fiducial-point pole computation: zenithal `TAN`/`SIN`/`ARC`/`STG`/
    //! `ZEA`/`ZPN`/`AIR`, zenithal-perspective `AZP`/`SZP`, cylindrical `CAR`/`CEA`/
    //! `MER`/`SFL`/`CYP`, all-sky `AIT`/`MOL`/`PAR`, conic `COP`/`COE`/`COD`/`COO`,
    //! pseudoconic `BON`, polyconic `PCO`, quad-cube `TSC`/`CSC`/`QSC`, and `HEALPix`
    //! `HPX`. Every Table-26 spectral algorithm (`F2*`/`W2*`/`V2*`/`A2*`, detector
    //! `GRI`/`GRA`, and generic `LOG`) is evaluated in both directions. `-TAB`
    //! coordinate arrays are resolved from their BINTABLE through
    //! [`FitsReader::read_wcs`](crate::FitsReader::read_wcs). All are validated against
    //! `astropy.wcs`, wcslib, or exact interpolation fixtures. The SIP polynomial (a
    //! prior distortion of the pixel offsets) and the TPV polynomial (a sequent
    //! distortion of the celestial intermediate coordinates) are evaluated in both
    //! directions; [`Description::distortion`] names the one that applies.
    //! [`Wcs::write_into`] writes a description back into a header, in wcslib's form,
    //! and [`Wcs::crop`] and [`Wcs::bin`] derive the WCS of a cut-out or a binned image.
    //! Convention-only `XPH`
    //! transforms remain readable in [`Description::unsupported_axes`]; complete transforms
    //! then return
    //! [`FitsError::UnsupportedWcsTransform`](crate::FitsError::UnsupportedWcsTransform).
    //!
    //! Binary-table WCS (Table 22) is supported for both the pixel-list
    //! ([`Wcs::from_pixel_list`]) and vector-cell ([`Wcs::from_array_column`]) forms.
    //!
    //! Pixel↔world yields celestial coordinates in the frame the file declares;
    //! [`Description::celestial_frame`] and [`AxisDescription::spectral_frame`] expose that typed
    //! `RADESYS`/`EQUINOX` and spectral frame/rest metadata. Converting *between*
    //! reference frames is astrometry beyond the FITS standard and is intentionally
    //! out of scope. Transform methods return explicit errors for invalid projection
    //! domains or failed iterations.

    pub use crate::alternate::Alternate;
    pub use crate::world_coordinates::axis_description::AxisDescription;
    pub use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;
    pub use crate::world_coordinates::celestial_frame::{CelestialFrame, CelestialReferenceFrame};
    pub use crate::world_coordinates::celestial_pole::CelestialPole;
    pub use crate::world_coordinates::distortion::Distortion;
    pub use crate::world_coordinates::projection::Projection;
    pub use crate::world_coordinates::spectral_frame::{SpectralFrame, SpectralReferenceFrame};
    pub use crate::world_coordinates::{AxisWorld, CelestialProjection, Description, Wcs};
}

/// Typed time coordinates (§9): calendar datetimes, time scales, and a
/// header's time frame.
pub mod time {
    pub use crate::time_coordinates::axis::Axis;
    pub use crate::time_coordinates::bounds::Bounds;
    pub use crate::time_coordinates::coordinate::Coordinate;
    pub use crate::time_coordinates::datetime::Datetime;
    pub use crate::time_coordinates::frame::Frame;
    pub use crate::time_coordinates::mjd::Mjd;
    pub use crate::time_coordinates::phase_axis::PhaseAxis;
    pub use crate::time_coordinates::reference_position::ReferencePosition;
    pub use crate::time_coordinates::scale::{Scale, ScaleKind};
    pub use crate::time_coordinates::unit::Unit;
}

/// Binary and ASCII table values, schema and selection metadata, and write
/// builders.
pub mod table {
    pub use crate::ascii::ascii_text::{AsciiRows, AsciiText};
    pub use crate::ascii::ascii_tform::{AsciiFloatFormat, AsciiTform};
    pub use crate::ascii::{Ascii, AsciiColumn, AsciiColumnData, AsciiColumnReader, AsciiSchema};
    pub use crate::bintable::Table;
    pub use crate::bintable::bit_column::{BitColumn, BitRows};
    pub use crate::bintable::character_field::CharacterField;
    pub use crate::bintable::column::Column;
    pub use crate::bintable::column_data::ColumnData;
    pub use crate::bintable::column_element::ColumnElement;
    pub use crate::bintable::column_reader::ColumnReader;
    pub use crate::bintable::column_view::ColumnView;
    pub use crate::bintable::descriptor::DescriptorWidth;
    pub use crate::bintable::element_kind::ElementKind;
    pub use crate::bintable::schema::Schema;
    pub use crate::bintable::selection::{SelectedColumn, SelectedData, Selection};
    pub use crate::bintable::tform::{Storage, Tform};
    pub use crate::column::ColumnSelector;
    pub use crate::fits_writer::ascii_builder::{AsciiBuilder, AsciiWriteColumn};
    pub use crate::fits_writer::builder::{Builder, WriteColumn};
    pub use crate::ragged::{Ragged, RowValues, Rows};
}

/// Lazy FITS source access and HDU-bound operations. Concrete source wrappers
/// live here because they appear in reader type aliases; the source abstraction
/// is sealed and is not an extension point.
pub mod io {
    pub use crate::block::{BLOCK_SIZE, CARD_SIZE};
    pub use crate::checksum::checksum_report::{ChecksumReport, ChecksumStatus};
    pub use crate::groups::{GroupParameter, RandomGroupView, RandomGroups, RandomGroupsMetadata};
    pub use crate::hdu::Hdu;
    pub use crate::hdu::hdu_kind::HduKind;
    pub use crate::hdu::hdu_selector::HduSelector;
    pub use crate::reader::source::{SliceSource, StreamSource};
    pub use crate::reader::{SliceReader, StreamReader};
}

/// The fixtures that tests and benches share, and the entry points of the
/// allocation-counting integration test; they are **not** a stable API.
#[cfg(feature = "internals")]
pub mod internals {
    #![cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "a bench or the allocation test uses part of the fixtures"
        )
    )]

    #[cfg(feature = "compression")]
    use crate::compression::gzip;

    /// The bound a tile set counts for its gzip inflate state.
    #[cfg(feature = "compression")]
    pub const INFLATE_STATE_BYTES: usize = gzip::INFLATE_STATE_BYTES;

    pub use crate::internals::whole_column::{WholeAsciiColumn, WholeColumn, WholeView};
    pub use crate::world_coordinates::tabular::internals::{
        tabular_forward_at_fraction, tabular_forward_at_pixel, tabular_inverse_at_fraction,
        tabular_inverse_at_world,
    };

    pub(crate) mod cards;
    #[cfg(all(test, feature = "compression"))]
    pub(crate) mod container;
    #[cfg(test)]
    pub(crate) mod fixture;
    pub(crate) mod memory;
    pub(crate) mod raw_table;
    pub(crate) mod rng;
    pub(crate) mod samples;
    pub(crate) mod scene;
    pub(crate) mod tolerance;
    #[cfg(test)]
    pub(crate) mod wcs_header;
    pub(crate) mod whole_column;
}

#[cfg(feature = "internals")]
pub mod bench;
