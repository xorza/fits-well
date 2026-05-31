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
//! - [`Header`], [`Card`], [`Value`] — an *ordered* header model that round-trips
//!   byte-for-byte, with a side index for O(1) keyword lookup.
//! - [`HduKind`] — HDU classification and the data-unit sizing formula that makes
//!   boundaries computable from headers alone (no data read required).
//! - [`FitsReader`] — lazy, seeking access to the HDU sequence of a file.
//!
//! # Status
//!
//! The structural spine (blocks, headers, HDU boundaries, lazy reading) is
//! implemented and tested. Typed data decode ([`data`]), the writer ([`writer`]),
//! ASCII/binary tables, WCS, and tiled compression are scaffolded — see their
//! module docs for the intended design.

mod bitpix;
mod block;
mod data;
mod error;
mod hdu;
mod header;
mod reader;
mod table;
mod writer;

pub use bitpix::Bitpix;
pub use data::{Image, ImageData, Scaling};
pub use error::{FitsError, Result};
pub use hdu::HduKind;
pub use header::Header;
pub use header::card::{Card, CardKind};
pub use header::value::Value;
pub use reader::{DataUnit, FitsReader, Hdu};
pub use table::{BinTable, Column, ColumnData, Tform, TformKind};
pub use writer::FitsWriter;

pub use block::{BLOCK_SIZE, CARD_SIZE, CARDS_PER_BLOCK, SPACE_FILL, ZERO_FILL};
