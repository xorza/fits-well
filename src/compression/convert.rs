//! Byte/type conversions shared across the image and table compression paths:
//! gathering a tile's pixels, widening to/narrowing from the `i64`/`f64` the codecs
//! work in, big-endian (de)serialization, and tile-cell accessors.

use crate::bintable::element_kind::ElementKind;
use crate::bintable::vla_column::VlaCell;
use crate::compression::plane::{FloatSamples, IntBitpix, IntSamples};
use crate::endian;
use crate::fits_error::Result;
use crate::fits_error::{CompressedDataDefect, TileCell};

/// Gather the tile rows starting at each of `row_bases` out of a typed sample plane
/// into `out`, widening each element with `widen`. Axis 0 has stride 1, so a row is
/// `row_len` contiguous elements and the gather is a run of slice copies rather than
/// per-pixel indexing.
fn gather_rows<S: Copy, D>(
    values: &[S],
    row_bases: &[usize],
    row_len: usize,
    out: &mut Vec<D>,
    widen: impl Fn(S) -> D,
) {
    out.clear();
    // A tile is exactly this many elements, so ask for that and no more.
    out.reserve_exact(row_bases.len() * row_len);
    for &base in row_bases {
        out.extend(values[base..base + row_len].iter().map(|&x| widen(x)));
    }
}

/// Gather a tile's integer pixels straight from the typed source into `out`,
/// widening to `i64` — so integer encoding never materializes a whole-image `i64`
/// buffer.
pub(super) fn gather_i64(
    samples: IntSamples<'_>,
    row_bases: &[usize],
    row_len: usize,
    out: &mut Vec<i64>,
) {
    match samples {
        IntSamples::U8(v) => gather_rows(v, row_bases, row_len, out, i64::from),
        IntSamples::I16(v) => gather_rows(v, row_bases, row_len, out, i64::from),
        IntSamples::I32(v) => gather_rows(v, row_bases, row_len, out, i64::from),
        IntSamples::I64(v) => gather_rows(v, row_bases, row_len, out, |x| x),
    }
}

/// Gather a tile's float pixels straight from the typed source into `out`,
/// widening to `f64` — so float encoding never materializes a whole-image `f64`
/// buffer.
pub(super) fn gather_f64(
    samples: FloatSamples<'_>,
    row_bases: &[usize],
    row_len: usize,
    out: &mut Vec<f64>,
) {
    match samples {
        FloatSamples::F32(v) => gather_rows(v, row_bases, row_len, out, f64::from),
        FloatSamples::F64(v) => gather_rows(v, row_bases, row_len, out, |x| x),
    }
}

/// Gather a tile's float pixels straight from the typed source into `out` as their
/// big-endian bytes, with no widening, so every bit survives, a NaN's payload included.
pub(super) fn gather_float_be(
    samples: FloatSamples<'_>,
    row_bases: &[usize],
    row_len: usize,
    out: &mut Vec<u8>,
) {
    out.clear();
    for &base in row_bases {
        match samples {
            FloatSamples::F32(v) => {
                endian::extend_be(out, &v[base..base + row_len], f32::to_be_bytes);
            }
            FloatSamples::F64(v) => {
                endian::extend_be(out, &v[base..base + row_len], f64::to_be_bytes);
            }
        }
    }
}

/// Narrow + pack `i64` values to big-endian `bitpix`-width integers in `out`, in a
/// single pass (no intermediate narrowed `Vec`). `out` is cleared first, so it can
/// be a reused scratch buffer. The narrowing rides along in `extend_be`'s
/// vectorizable grow-once-then-fill-each-slot loop.
pub(super) fn i64_to_be_into(vals: &[i64], bitpix: IntBitpix, out: &mut Vec<u8>) {
    out.clear();
    match bitpix {
        IntBitpix::U8 => endian::extend_be(out, vals, |v| [v.cast_unsigned() as u8]),
        IntBitpix::I16 => endian::extend_be(out, vals, |v| (v as i16).to_be_bytes()),
        IntBitpix::I32 => endian::extend_be(out, vals, |v| (v as i32).to_be_bytes()),
        IntBitpix::I64 => endian::extend_be(out, vals, i64::to_be_bytes),
    }
}

/// Decode a big-endian buffer of `bitpix` integers into widened `i64` values in `out`
/// (cleared first). Single pass — no intermediate narrowed `Vec`; the widening rides
/// along in `decode_be_into`'s inlined, vectorizable per-chunk conversion.
pub(super) fn be_to_i64_into(bytes: &[u8], bitpix: IntBitpix, out: &mut Vec<i64>) {
    match bitpix {
        IntBitpix::U8 => endian::decode_be_into(bytes, out, |[b]| i64::from(b)),
        IntBitpix::I16 => endian::decode_be_into(bytes, out, |b| i64::from(i16::from_be_bytes(b))),
        IntBitpix::I32 => endian::decode_be_into(bytes, out, |b| i64::from(i32::from_be_bytes(b))),
        IntBitpix::I64 => endian::decode_be_into(bytes, out, i64::from_be_bytes),
    }
}

/// An integer image's `UNCOMPRESSED_DATA` cell, widened into `out`. Errors when the
/// cell is not an integer array.
pub(super) fn cell_to_i64_into(cell: VlaCell<'_>, out: &mut Vec<i64>) -> Result<()> {
    let bitpix =
        IntBitpix::from_tform(cell.element_type).ok_or(CompressedDataDefect::CellElement {
            cell: TileCell::Uncompressed,
            element: cell.element_type,
        })?;
    be_to_i64_into(cell.bytes, bitpix, out);
    Ok(())
}

/// The bytes of a compressed stream `cell`, whose array must hold elements of `kind`: a
/// codec's [`TileCodec::stream_kind`], or bytes for a gzip fallback.
///
/// [`TileCodec::stream_kind`]: crate::compression::tile_codec::TileCodec::stream_kind
pub(super) fn stream_cell(cell: VlaCell<'_>, kind: ElementKind) -> Result<&[u8]> {
    if cell.element_type == kind {
        Ok(cell.bytes)
    } else {
        Err(CompressedDataDefect::CellElement {
            cell: TileCell::Compressed,
            element: cell.element_type,
        }
        .into())
    }
}

#[cfg(test)]
mod tests {

    use crate::compression::convert::*;

    fn i64_to_be(values: &[i64], bitpix: IntBitpix) -> Vec<u8> {
        let mut out = Vec::new();
        i64_to_be_into(values, bitpix, &mut out);
        out
    }

    #[test]
    fn i64_be_round_trip_and_buffer_reuse() {
        // I16: narrowing (`as i16`) then big-endian packing, hand-computed. -1 → 0xFFFF,
        // 258 → 0x0102, -2 → 0xFFFE.
        let vals = [0i64, 1, -1, 258, -2];
        let want = [0, 0, 0, 1, 0xFF, 0xFF, 0x01, 0x02, 0xFF, 0xFE];
        assert_eq!(i64_to_be(&vals, IntBitpix::I16), want);
        // Decode is the exact inverse (sign-extending back to i64), into a reused buffer.
        let mut d = Vec::new();
        be_to_i64_into(&want, IntBitpix::I16, &mut d);
        assert_eq!(d, vals);

        // I32 packs four bytes/elem; 0x00010203 = 66051, -1 → all-ones.
        let i32_vals = [66051i64, -1];
        assert_eq!(
            i64_to_be(&i32_vals, IntBitpix::I32),
            [0x00, 0x01, 0x02, 0x03, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        be_to_i64_into(
            &[0, 1, 2, 3, 0xFF, 0xFF, 0xFF, 0xFF],
            IntBitpix::I32,
            &mut d,
        );
        assert_eq!(d, i32_vals);

        // U8 and I64 ends of the range.
        assert_eq!(i64_to_be(&[255, 0], IntBitpix::U8), [0xFF, 0x00]);
        be_to_i64_into(&[0xFF, 0x00], IntBitpix::U8, &mut d);
        assert_eq!(d, [255, 0]);
        assert_eq!(i64_to_be(&[-1], IntBitpix::I64), [0xFF; 8]);

        // `i64_to_be_into` clears + resizes its scratch: a long fill followed by a short
        // one must leave exactly the short result (no stale tail), matching the owning form.
        let mut buf = Vec::new();
        i64_to_be_into(&[7, 8, 9, 10], IntBitpix::I16, &mut buf);
        i64_to_be_into(&vals, IntBitpix::I16, &mut buf);
        assert_eq!(buf, want);
    }
}
