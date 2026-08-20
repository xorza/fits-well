//! A blazing-fast reader and writer for **FITS** (Flexible Image Transport
//! System) files — the standard data format of astronomy.
//!
//! # Layering
//!
//! The format's structure maps onto a stack of layers, so the hot decode path
//! stays lean and the semantic layers compute only on demand. WCS (§8) and time
//! (§9) are dependency-free, always compiled, and surfaced directly as
//! [`header::Header`] getters ([`header::Header::wcs`],
//! [`header::Header::time`]); tiled compression carries a dependency and stays
//! behind the `compression` feature.
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
//! - [`header::Header`], [`header::value::Value`] — an *ordered* header model
//!   (an internal `Card` list)
//!   whose logical records round-trip with a side index for O(1) keyword lookup;
//!   physical card layout is normalized on write rather than retained. It also
//!   parses the WCS and time layers on request
//!   ([`header::Header::wcs`]/[`header::Header::time`]).
//! - [`io::HduKind`] — HDU classification and the data-unit sizing formula that makes
//!   boundaries computable from headers alone (no data read required).
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
mod ascii;
mod bitpix;
mod block;
mod checksum;
mod column;
#[cfg(feature = "compression")]
mod compress;
mod data;
mod endian;
mod error;
mod groups;
mod hdu;
pub mod header;
mod keyword;
mod reader;
#[path = "table/mod.rs"]
mod table_impl;
#[path = "time/mod.rs"]
mod time_impl;
mod unit;
pub mod wcs;
mod words;
mod writer;

pub use error::{FitsError, Indexed, Ranked, Result};
pub use reader::FitsReader;
pub use writer::FitsWriter;

/// Typed image values, scratch-backed views, compression, and incremental output.
pub mod image {
    pub use crate::bitpix::Bitpix;
    #[cfg(feature = "compression")]
    pub use crate::compress::{Compression, CompressionOptions, DitherMethod, Gzip, Hcompress};
    pub use crate::data::image_data::ImageData;
    pub use crate::data::image_view::{BorrowedImage, ImageView};
    pub use crate::data::read_image::ReadImage;
    pub use crate::data::sample_type::SampleType;
    pub use crate::data::scaling::Scaling;
    pub use crate::data::unsigned_data::UnsignedData;
    pub use crate::data::{Image, ImageMetadata};
    pub use crate::writer::image::ImageStream;
}

/// Typed time coordinates (§9): calendar datetimes, time scales, and a
/// header's time frame.
pub mod time {
    pub use crate::time_impl::datetime::Datetime;
    pub use crate::time_impl::phase_axis::PhaseAxis;
    pub use crate::time_impl::time_reference_position::TimeReferencePosition;
    pub use crate::time_impl::time_scale::{TimeScale, TimeScaleKind};
    pub use crate::time_impl::{FitsTime, TimeBounds, TimeCoordinate};
}

/// Binary and ASCII table values, schema and selection metadata, and write
/// builders.
pub mod table {
    pub use bitvec::order::Msb0;
    pub use bitvec::slice::BitSlice;
    pub use bitvec::vec::BitVec;
    pub use num_complex::Complex;

    pub use crate::ascii::{
        AsciiColumn, AsciiColumnData, AsciiColumnReader, AsciiKind, AsciiTable, AsciiTableMetadata,
    };
    pub use crate::reader::{ColumnSelector, SelectedColumn, TableColumnData, TableSelection};
    pub use crate::table_impl::{
        BinTable, BinTableMetadata, BitColumn, CharacterField, Column, ColumnData, ColumnReader,
        TableSchema, Tform, TformKind,
    };
    pub use crate::writer::ascii::{AsciiTableBuilder, AsciiWriteColumn};
    pub use crate::writer::table::{ColumnType, TableBuilder, WriteColumn};
}

/// Lazy FITS source access and HDU-bound operations. Concrete source wrappers
/// live here because they appear in reader type aliases; the source abstraction
/// is sealed and is not an extension point.
pub mod io {
    pub use crate::block::{BLOCK_SIZE, CARD_SIZE};
    pub use crate::groups::{RandomGroupView, RandomGroups, RandomGroupsMetadata};
    pub use crate::hdu::HduKind;
    #[cfg(feature = "mmap")]
    pub use crate::reader::MmapReader;
    #[cfg(feature = "mmap")]
    pub use crate::reader::source::MmapSource;
    pub use crate::reader::source::{SliceSource, StreamSource};
    pub use crate::reader::{
        ChecksumReport, ChecksumStatus, DataUnit, DataUnitView, Hdu, SliceReader, StreamReader,
    };
}

/// Hot internal entry points re-exposed **for benchmarking only** (the `internals`
/// feature). These wrap crate-private functions so the benches under `benches/`
/// can measure them in isolation; they are **not** a stable API — do not depend on
/// them.
#[cfg(feature = "internals")]
pub mod internals {
    use crate::bitpix::Bitpix;
    use crate::data::image_data::ImageData;
    use crate::wcs::bench;

    /// Decode a big-endian data unit into host-endian samples — the per-element
    /// byte-swap (`ImageData::decode`).
    pub fn decode_image(bytes: &[u8], bitpix: Bitpix) -> ImageData {
        ImageData::decode(bytes, bitpix)
    }

    /// Encode samples back to a big-endian buffer — the inverse swap
    /// (`ImageData::encode_into` into a fresh buffer).
    pub fn encode_image(data: &ImageData) -> Vec<u8> {
        let mut out = Vec::new();
        data.encode_into(&mut out);
        out
    }

    /// Build and cache the WCS benchmark fixtures outside timed iterations.
    pub fn prepare_wcs_benchmarks() {
        bench::prepare();
    }

    /// Transform one fixed batch forward and backward through a four-axis linear WCS.
    pub fn linear_wcs_round_trip_batch() -> f64 {
        bench::linear_round_trip_batch()
    }

    /// Transform one fixed batch through a Table-26 spectral axis.
    pub fn spectral_wcs_batch() -> f64 {
        bench::spectral_batch()
    }

    /// Transform one fixed batch through a large monotonic `-TAB` index vector.
    pub fn tabular_wcs_batch() -> f64 {
        bench::tabular_index_batch()
    }

    /// Transform one pixel through the large monotonic `-TAB` fixture.
    pub fn tabular_forward_at_pixel(pixel: f64) -> f64 {
        bench::tabular_forward_at_pixel(pixel)
    }

    /// Invert one world coordinate through the large monotonic `-TAB` fixture.
    pub fn tabular_inverse_at_world(world: f64) -> f64 {
        bench::tabular_inverse_at_world(world)
    }

    /// Invert one two-dimensional affine `-TAB` coordinate at a chosen dyadic
    /// fraction, used to guard the inverse search's allocation count by depth.
    pub fn tabular_inverse_at_fraction(fraction: f64) -> f64 {
        bench::tabular_inverse_at_fraction(fraction)
    }
}
