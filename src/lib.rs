//! A blazing-fast reader and writer for **FITS** (Flexible Image Transport
//! System) files — the standard data format of astronomy.
//!
//! # Layering
//!
//! The format's structure maps onto a stack of layers, so the hot decode path
//! stays lean and the semantic layers compute only on demand. WCS (§8) and time
//! (§9) are dependency-free pure math, always compiled and surfaced directly as
//! [`Header`] getters ([`Header::wcs`], [`Header::time`]); tiled compression carries
//! a dependency and stays behind the `compression` feature.
//!
//! ```text
//! bytes ─► block layer ─► HDU layer ─► header model ─► typed data
//!         (2880 grid,    (boundary    (ordered        (images,
//!          padding,       scan, lazy   records +       tables,
//!          I/O quantum)   seeking)     keyword index)  heap, VLAs)
//! ```
//!
//! - [`BLOCK_SIZE`] — the 2880-byte block grid, padding rules, and rounding math.
//! - [`Bitpix`] — the array element type selector (`BITPIX`).
//! - [`Header`], [`Value`] — an *ordered* header model (an internal `Card` list)
//!   whose logical records round-trip with a side index for O(1) keyword lookup;
//!   physical card layout is normalized on write rather than retained. It also
//!   parses the WCS and time layers on request ([`Header::wcs`]/[`Header::time`]).
//! - [`HduKind`] — HDU classification and the data-unit sizing formula that makes
//!   boundaries computable from headers alone (no data read required).
//! - [`FitsReader`] — lazy, seeking access to the HDU sequence of a file.
//!
//! # Status
//!
//! The structural spine (blocks, headers, HDU boundaries, lazy reading) plus
//! typed image decode/encode ([`Image`]), the multi-HDU writer ([`FitsWriter`]),
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
#[cfg(feature = "compression")]
mod compress;
mod data;
mod endian;
mod error;
mod groups;
mod hdu;
mod header;
mod keyword;
mod reader;
mod table;
mod time;
mod unit;
mod wcs;
mod writer;

pub use ascii::{
    AsciiColumn, AsciiColumnData, AsciiColumnReader, AsciiKind, AsciiTable, AsciiTableMetadata,
};
pub use bitpix::Bitpix;

/// Re-exported so callers can name the borrowed [`bitvec::slice::BitSlice`] rows that
/// a [`BitColumn`] (from [`ColumnReader::bits`]/[`ColumnReader::vla_bits`]) yields and
/// the owned [`bitvec::vec::BitVec`] rows accepted by [`WriteColumn::vla_bits`] —
/// without taking their own version-skewed `bitvec` dependency.
pub use bitvec;
#[cfg(feature = "compression")]
pub use compress::{CompressOptions, DitherMethod};
#[cfg(feature = "ndarray")]
pub use data::ImageArray;
pub use data::{
    Image, ImageData, ImageMetadata, ImageView, RawImage, SampleType, Scaling, UnsignedView,
};
pub use error::{FitsError, Result};
pub use groups::{RandomGroupView, RandomGroups, RandomGroupsMetadata};
pub use hdu::HduKind;
pub use header::Header;
pub use header::HeaderEntry;
pub use header::value::{FitsInteger, Value};
/// The complex element type of `C`/`M` columns — the ecosystem-standard
/// [`num_complex::Complex`]. Re-exported (crate and type) so callers can name it and
/// stay on the same version this crate returns.
pub use num_complex;
pub use num_complex::Complex;
#[cfg(feature = "mmap")]
pub use reader::MmapReader;
#[cfg(feature = "mmap")]
pub use reader::source::MmapSource;
pub use reader::source::{SliceSource, Source, StreamSource};
pub use reader::{
    ChecksumReport, ChecksumStatus, DataUnit, DataUnitView, FitsReader, Hdu, SliceReader,
    StreamReader,
};
pub use table::{
    BinTable, BinTableMetadata, BitColumn, CharacterField, Column, ColumnData, ColumnReader, TDisp,
    TDispKind, Tform, TformKind,
};
pub use time::{
    Datetime, Epoch, FitsTime, GtiInterval, PhaseAxis, TimeAxisKind, TimeBounds, TimeCoordinate,
    TimeReferencePosition, TimeScale,
};
pub use wcs::{
    CelestialFrame, CelestialReferenceFrame, Projection, SpectralFrame, SpectralReferenceFrame,
    Wcs, WcsAxis, WcsView,
};
pub use writer::{AsciiWriteColumn, ColumnType, FitsWriter, WriteColumn};

pub use block::{BLOCK_SIZE, CARD_SIZE};

/// Hot internal entry points re-exposed **for benchmarking only** (the `internals`
/// feature). These wrap crate-private functions so the benches under `benches/`
/// can measure them in isolation; they are **not** a stable API — do not depend on
/// them.
#[cfg(feature = "internals")]
pub mod internals {
    use crate::bitpix::Bitpix;
    use crate::data::ImageData;

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
}
