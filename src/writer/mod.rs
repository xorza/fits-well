//! Header and data-unit serialization.
//!
//! The high-level writers — [`FitsWriter::write_image`], `write_table`,
//! `write_ascii_table`, and the compressed forms — synthesize the mandatory header
//! and emit the data unit through one preflighted HDU transaction (which pads to the block grid and
//! embeds `CHECKSUM`/`DATASUM` when enabled), assembling each unit in the writer's
//! reused `scratch`. [`FitsWriter::write_raw_hdu`] is the low-level escape hatch for
//! callers supplying a complete header and already-encoded data unit themselves.

use std::io::Write;

use crate::block::{BLOCK_SIZE, CARD_SIZE, SPACE_FILL, ZERO_FILL};
use crate::checksum;
use crate::error::{FitsError, Result};
use crate::hdu::{HduKind, HduPosition, HduRole, data_extent};
use crate::header::Header;
use crate::keyword;

pub(crate) mod ascii;
pub(crate) mod image;
pub(crate) mod table;

/// 16-zero `CHECKSUM` value written before the real checksum is solved and
/// patched in (Appendix J.1).
const PLACEHOLDER_CHECKSUM: &str = "0000000000000000";

/// Serialize a header unit into reusable storage: every card rendered to 80 bytes,
/// the `END` record, then space padding to the next 2880-byte boundary.
pub(crate) fn render_header(header: &Header, buf: &mut Vec<u8>) -> Result<()> {
    let min_len = header
        .cards
        .len()
        .checked_add(1)
        .and_then(|records| records.checked_mul(CARD_SIZE))
        .ok_or(FitsError::DataUnitOverflow)?;
    buf.clear();
    buf.reserve(min_len);
    for card in &header.cards {
        card.render_into(buf)?;
    }
    let mut end = [SPACE_FILL; CARD_SIZE];
    end[..3].copy_from_slice(b"END");
    buf.extend_from_slice(&end);
    pad_to_block(buf, SPACE_FILL)
}

/// Round `buf` up to a whole number of 2880-byte blocks using `fill`.
fn pad_to_block(buf: &mut Vec<u8>, fill: u8) -> Result<()> {
    let rem = buf.len() % BLOCK_SIZE;
    if rem != 0 {
        let padded = buf
            .len()
            .checked_add(BLOCK_SIZE - rem)
            .ok_or(FitsError::DataUnitOverflow)?;
        buf.resize(padded, fill);
    }
    Ok(())
}

/// Writes FITS HDUs to a byte sink. The first HDU written becomes the primary
/// array; subsequent images/tables are written as extensions.
#[derive(Debug)]
pub struct FitsWriter<W> {
    sink: W,
    state: WriterState,
    checksum: bool,
    /// Reused buffer the data unit is assembled into before padding + writing, so
    /// writing many HDUs allocates no per-call staging. Each high-level write
    /// `clear`s it, builds the unit, and hands it to the HDU commit path.
    scratch: Vec<u8>,
    /// Reused block-padded header serialization, kept separate because checksum
    /// generation needs the header and data bytes alive at the same time.
    header_scratch: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterState {
    Empty,
    Active,
    Failed,
}

impl<W: Write> FitsWriter<W> {
    pub fn new(sink: W) -> Self {
        FitsWriter {
            sink,
            state: WriterState::Empty,
            checksum: false,
            scratch: Vec::new(),
            header_scratch: Vec::new(),
        }
    }

    /// Enable `DATASUM`/`CHECKSUM` integrity keywords on every HDU written through
    /// this writer (§J), including [`FitsWriter::write_raw_hdu`].
    pub fn with_checksums(mut self) -> Self {
        self.checksum = true;
        self
    }

    /// Write one complete raw HDU after validating the header's role and exact
    /// unpadded data length. The block fill is derived from the HDU kind: spaces
    /// for an ASCII table and NULs for every other data unit.
    pub fn write_raw_hdu(&mut self, header: &Header, raw: &[u8]) -> Result<()> {
        self.ensure_writable()?;
        let position = match self.state {
            WriterState::Empty => HduPosition::Primary,
            WriterState::Active => HduPosition::Extension,
            WriterState::Failed => unreachable!("failed writer rejected above"),
        };
        let role = HduRole::from_header(header, position)?;
        let kind = HduKind::classify(header, role)?;
        let extent = data_extent(header, role)?;
        let expected =
            usize::try_from(extent.data_bytes).map_err(|_| FitsError::DataUnitTooLarge {
                bytes: extent.data_bytes,
            })?;
        if raw.len() != expected {
            return Err(FitsError::DataSizeMismatch {
                expected,
                got: raw.len(),
            });
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(raw);
        let fill = if kind == HduKind::AsciiTable {
            SPACE_FILL
        } else {
            ZERO_FILL
        };
        self.finish_hdu(header.clone(), fill, false)
    }
    fn ensure_writable(&self) -> Result<()> {
        if self.state == WriterState::Failed {
            return Err(FitsError::WriterFailed);
        }
        Ok(())
    }

    /// Finish preflight for one HDU, then commit it and any required automatic
    /// primary without another fallible validation or encoding step between them.
    fn finish_hdu(&mut self, header: Header, fill: u8, automatic_primary: bool) -> Result<()> {
        let rem = self.scratch.len() % BLOCK_SIZE;
        if rem != 0 {
            let padded = self
                .scratch
                .len()
                .checked_add(BLOCK_SIZE - rem)
                .ok_or(FitsError::DataUnitOverflow)?;
            self.scratch.resize(padded, fill);
        }
        prepare_header(
            header,
            &self.scratch,
            self.checksum,
            &mut self.header_scratch,
        )?;

        let primary_header = if automatic_primary && self.state == WriterState::Empty {
            let mut primary_header = Vec::new();
            prepare_header(
                empty_primary_header(),
                &[],
                self.checksum,
                &mut primary_header,
            )?;
            Some(primary_header)
        } else {
            None
        };
        if let Some(primary_header) = primary_header {
            write_prepared(&mut self.sink, &mut self.state, &primary_header, &[])?;
        }
        write_prepared(
            &mut self.sink,
            &mut self.state,
            &self.header_scratch,
            &self.scratch,
        )
    }

    /// Consume the writer and return the underlying sink. HDUs are written eagerly,
    /// so an unbuffered sink (e.g. a `File`) holds the complete file. This does **not**
    /// flush: if the sink is a `BufWriter`, flush it (or rely on its `Drop`) before
    /// trusting the bytes, and check the flush result if you need write errors surfaced.
    pub fn into_inner(self) -> W {
        self.sink
    }
}
fn prepare_header(
    header: Header,
    padded_data: &[u8],
    with_checksum: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    if with_checksum {
        prepare_header_with_data_sum(header, checksum::accumulate(padded_data, 0), out)
    } else {
        render_header(&header, out)
    }
}

fn prepare_header_with_data_sum(
    mut header: Header,
    data_sum: u32,
    out: &mut Vec<u8>,
) -> Result<()> {
    header.set_internal("DATASUM", data_sum.to_string());
    header.set_internal("CHECKSUM", PLACEHOLDER_CHECKSUM);
    render_header(&header, out)?;
    let hdu_sum = checksum::combine(checksum::accumulate(out, 0), data_sum);
    patch_checksum(out, &checksum::encode(hdu_sum, true));
    Ok(())
}

fn write_prepared<W: Write>(
    sink: &mut W,
    state: &mut WriterState,
    header: &[u8],
    data: &[u8],
) -> Result<()> {
    if let Err(error) = sink.write_all(header).and_then(|()| sink.write_all(data)) {
        *state = WriterState::Failed;
        return Err(FitsError::Io(error));
    }
    *state = WriterState::Active;
    Ok(())
}

/// A dataless primary HDU (`NAXIS = 0`), written before extensions when the
/// caller's first HDU is itself an extension.
fn empty_primary_header() -> Header {
    let mut header = Header::new();
    header
        .set_internal("SIMPLE", true)
        .comment_internal("SIMPLE", "file conforms to FITS standard");
    header.set_internal("BITPIX", 8).set_internal("NAXIS", 0);
    header
        .set_internal("EXTEND", true)
        .comment_internal("EXTEND", "extensions follow");
    header
}

/// Reconcile one column's implied row count with the table's — the inference rule
/// [`TableBuilder`](table::TableBuilder) and
/// [`AsciiTableBuilder`](ascii::AsciiTableBuilder) share: the first column to imply
/// a count fixes it, every later column must agree, and a column that implies
/// nothing at all is acceptable only once the count is already known.
///
/// `inferred` is `None` only for a binary column whose payload determines no count
/// (an empty or zero-width one). An ASCII column is always scalar, so its row count
/// is its value count and it never reaches the undetermined case.
fn accept_row_count(
    nrows: &mut Option<usize>,
    column: &str,
    inferred: Option<usize>,
) -> Result<()> {
    match (*nrows, inferred) {
        (Some(expected), Some(got)) if expected != got => Err(FitsError::TableRowCountMismatch {
            column: column.to_string(),
            expected,
            got,
        }),
        (None, Some(rows)) => {
            *nrows = Some(rows);
            Ok(())
        }
        (None, None) => Err(FitsError::TableRowCountUndetermined {
            column: column.to_string(),
        }),
        _ => Ok(()),
    }
}

fn validate_scaling(tscale: Option<f64>, tzero: Option<f64>, allowed: bool) -> Result<()> {
    if tscale.is_some_and(|value| !allowed || !value.is_finite()) {
        return Err(FitsError::KeywordOutOfRange { name: "TSCALn" });
    }
    if tzero.is_some_and(|value| !allowed || !value.is_finite()) {
        return Err(FitsError::KeywordOutOfRange { name: "TZEROn" });
    }
    Ok(())
}

/// Replace the 16 placeholder bytes of the rendered `CHECKSUM` card's value with
/// the solved value. The value occupies bytes 12–27 (0-based 11–26) of its card.
fn patch_checksum(header_bytes: &mut [u8], encoded: &[u8; 16]) {
    for card in header_bytes.chunks_exact_mut(CARD_SIZE) {
        if &card[..8] == b"CHECKSUM" {
            card[11..27].copy_from_slice(encoded);
            return;
        }
    }
}

fn merge_header_template(header: &mut Header, template: Option<&Header>) {
    let Some(template) = template else {
        return;
    };
    header.append_filtered_from(template, |keyword| !is_structural_keyword(keyword));
}

fn is_structural_keyword(keyword: &str) -> bool {
    if matches!(
        keyword,
        "SIMPLE"
            | "XTENSION"
            | "BITPIX"
            | "NAXIS"
            | "PCOUNT"
            | "GCOUNT"
            | "EXTEND"
            | "GROUPS"
            | "BLOCKED"
            | "BSCALE"
            | "BZERO"
            | "BLANK"
            | "CHECKSUM"
            | "DATASUM"
            | "THEAP"
            | "TFIELDS"
            | "ZIMAGE"
            | "ZTABLE"
            | "ZTILELEN"
            | "ZNAXIS"
            | "ZPCOUNT"
            | "ZGCOUNT"
            | "ZSIMPLE"
            | "ZTENSION"
            | "ZEXTEND"
            | "ZBLOCKED"
            | "ZTHEAP"
            | "ZHEAPPTR"
            | "ZHECKSUM"
            | "ZDATASUM"
            | "ZCMPTYPE"
            | "ZBITPIX"
            | "ZQUANTIZ"
            | "ZDITHER0"
            | "ZBLANK"
            | "ZMASKCMP"
    ) {
        return true;
    }
    [
        "NAXIS", "TFORM", "TTYPE", "TUNIT", "TDIM", "TSCAL", "TZERO", "TNULL", "TBCOL", "ZFORM",
        "ZCTYP", "ZNAXIS", "ZTILE", "ZNAME", "ZVAL",
    ]
    .iter()
    .any(|prefix| indexed_keyword(keyword, prefix))
}

fn indexed_keyword(keyword: &str, prefix: &str) -> bool {
    // A conforming card is at most 8 bytes; anything longer is not the indexed
    // structural keyword it superficially resembles.
    keyword.len() <= 8 && keyword::index(keyword, prefix).is_some()
}

#[cfg(test)]
mod tests;
