//! A blazing-fast reader and writer for **FITS** (Flexible Image Transport
//! System) files — the standard data format of astronomy.
//!
//! # Layering
//!
//! The format's structure maps onto a stack of layers; each only depends on the
//! ones below it, so the hot decode path stays lean and the optional semantic
//! layers (WCS, compression) are opt-in.
//!
//! ```text
//! bytes ─► block layer ─► HDU layer ─► header model ─► typed data
//!         (2880 grid,    (boundary    (ordered        (images,
//!          padding,       scan, lazy   records +       tables,
//!          I/O quantum)   seeking)     keyword index)  heap, VLAs)
//! ```
//!
//! - [`block`] — the 2880-byte block grid, padding rules, and rounding math.
//! - [`Bitpix`] — the array element type selector (`BITPIX`).
//! - [`Header`], [`Value`] — an *ordered* header model (an internal `Card` list)
//!   that round-trips byte-for-byte, with a side index for O(1) keyword lookup.
//! - [`HduKind`] — HDU classification and the data-unit sizing formula that makes
//!   boundaries computable from headers alone (no data read required).
//! - [`FitsReader`] — lazy, seeking access to the HDU sequence of a file.
//!
//! # Status
//!
//! The structural spine (blocks, headers, HDU boundaries, lazy reading) plus
//! typed image decode/encode ([`data`]), the multi-HDU writer ([`writer`]),
//! ASCII/binary tables, WCS, time coordinates, and tiled image+table compression
//! are implemented and tested — see each module's docs for its design.

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
mod wcs;
mod writer;

pub use ascii::{AsciiColumn, AsciiKind, AsciiTable};
pub use bitpix::Bitpix;
pub use data::{Image, ImageData, Scaling, UnsignedView};
pub use error::{FitsError, Result};
pub use groups::RandomGroups;
pub use hdu::HduKind;
pub use header::Header;
pub use header::value::Value;
pub use reader::{ChecksumReport, DataUnit, FitsReader, Hdu};
pub use table::{BinTable, Column, ColumnData, TDisp, TDispKind, Tform, TformKind};
pub use time::{
    Datetime, Epoch, EpochTime, FitsTime, GtiInterval, PhaseAxis, TimeAxisKind, TimeBounds,
    TimeScale, time_axis_kind,
};
pub use wcs::{Projection, Wcs};
pub use writer::{AsciiWriteColumn, FitsWriter, WriteColumn};

pub use block::{BLOCK_SIZE, CARD_SIZE, CARDS_PER_BLOCK, SPACE_FILL, ZERO_FILL};

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
