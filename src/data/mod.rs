//! Typed image data model.
//!
//! FITS exposes image data on two planes: a *raw* plane (the stored samples) and a
//! *physical* plane (`BZERO + BSCALE × raw`). The stored samples are big-endian, so
//! [`ImageData::decode`] swaps a data unit into an owned, host-endian [`ImageData`]
//! and [`ImageData::encode_into`] writes them back. When no swap is needed
//! (`BITPIX = 8`, or a big-endian host) an in-memory reader can skip even that copy
//! and borrow the data unit in place — see [`RawImage`] /
//! [`crate::FitsReader::read_image`]. The per-element swap loops are
//! memory-bandwidth-bound, so they lean on autovectorization rather than threads
//! (the thread-parallel layer is the compute-bound tiled codecs in the `compress`
//! module, not this path).

use crate::bitpix::Bitpix;
use crate::endian::decode_be;
use crate::endian::decode_be_into_slice;
use crate::endian::extend_be;
use crate::header::Header;

/// An owned, host-endian sample buffer, tagged by its `BITPIX` element type.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    U8(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

/// Element count for an N-d `shape`: the product of the axis lengths, or `0` for
/// an empty shape (`NAXIS = 0` ⇒ no data, not the empty-product `1`).
pub(crate) fn shape_product(shape: &[usize]) -> usize {
    if shape.is_empty() {
        0
    } else {
        shape.iter().product()
    }
}

impl ImageData {
    /// The `BITPIX` element kind backing this buffer.
    pub fn bitpix(&self) -> Bitpix {
        match self {
            ImageData::U8(_) => Bitpix::U8,
            ImageData::I16(_) => Bitpix::I16,
            ImageData::I32(_) => Bitpix::I32,
            ImageData::I64(_) => Bitpix::I64,
            ImageData::F32(_) => Bitpix::F32,
            ImageData::F64(_) => Bitpix::F64,
        }
    }

    /// Number of samples in the buffer.
    pub fn len(&self) -> usize {
        match self {
            ImageData::U8(v) => v.len(),
            ImageData::I16(v) => v.len(),
            ImageData::I32(v) => v.len(),
            ImageData::I64(v) => v.len(),
            ImageData::F32(v) => v.len(),
            ImageData::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decode the raw, big-endian data unit into host-endian typed samples.
    /// `bytes` is the unpadded data (a whole number of `bitpix` elements); the
    /// fill past the data range must already be sliced off (see
    /// [`crate::DataUnit::data`]).
    pub(crate) fn decode(bytes: &[u8], bitpix: Bitpix) -> ImageData {
        assert_eq!(
            bytes.len() % bitpix.elem_size(),
            0,
            "data length must be a whole number of {bitpix:?} elements"
        );
        match bitpix {
            Bitpix::U8 => ImageData::U8(bytes.to_vec()),
            Bitpix::I16 => ImageData::I16(decode_be(bytes, i16::from_be_bytes)),
            Bitpix::I32 => ImageData::I32(decode_be(bytes, i32::from_be_bytes)),
            Bitpix::I64 => ImageData::I64(decode_be(bytes, i64::from_be_bytes)),
            Bitpix::F32 => ImageData::F32(decode_be(bytes, f32::from_be_bytes)),
            Bitpix::F64 => ImageData::F64(decode_be(bytes, f64::from_be_bytes)),
        }
    }

    /// Append the samples to `out` in big-endian order — the inverse of
    /// [`ImageData::decode`]. This is the unpadded data unit; the writer pads it
    /// to the 2880-byte block grid. Appends (never clears), so a writer reusing one
    /// buffer across HDUs clears it first and pays no per-image staging allocation.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            ImageData::U8(v) => out.extend_from_slice(v),
            ImageData::I16(v) => extend_be(out, v, i16::to_be_bytes),
            ImageData::I32(v) => extend_be(out, v, i32::to_be_bytes),
            ImageData::I64(v) => extend_be(out, v, i64::to_be_bytes),
            ImageData::F32(v) => extend_be(out, v, f32::to_be_bytes),
            ImageData::F64(v) => extend_be(out, v, f64::to_be_bytes),
        }
    }

    /// The physical-plane values for these samples under `scaling`: `BZERO + BSCALE
    /// × sample` (§3.4), with integer samples equal to the `BLANK` sentinel mapped
    /// to `NaN` (float `NaN`/`Inf` pass through). Shared by [`Image::physical`] and
    /// [`RawImage::physical`].
    pub(crate) fn physical(&self, scaling: &Scaling) -> Vec<f64> {
        self.physical_as(scaling)
    }

    /// The physical plane narrowed to `f32` in a single pass — see
    /// [`RawImage::physical_f32`]. Scaling is still evaluated in `f64`, so each
    /// element is the correctly-rounded `f32` of the true physical value.
    pub(crate) fn physical_f32(&self, scaling: &Scaling) -> Vec<f32> {
        self.physical_as(scaling)
    }

    fn physical_as<O: PhysicalOut>(&self, scaling: &Scaling) -> Vec<O> {
        let Scaling {
            bscale,
            bzero,
            blank,
        } = *scaling;
        let scale = |x: f64| O::from_f64(bzero + bscale * x);
        match self {
            ImageData::U8(v) => scale_ints(v, blank, scale),
            ImageData::I16(v) => scale_ints(v, blank, scale),
            ImageData::I32(v) => scale_ints(v, blank, scale),
            ImageData::I64(v) => scale_ints(v, blank, scale),
            ImageData::F32(v) => v.iter().map(|&x| scale(x as f64)).collect(),
            ImageData::F64(v) => v.iter().map(|&x| scale(x)).collect(),
        }
    }

    /// Exact typed unsigned (or signed-byte) reinterpretation when `scaling` is
    /// precisely the FITS unsigned convention (`BSCALE == 1`, no `BLANK`, and
    /// `BZERO` the matching sign-bit offset); `None` otherwise. Exact for all 64-bit
    /// values (no `f64` rounding). Shared by [`Image::unsigned`]/[`RawImage::unsigned`].
    pub(crate) fn unsigned(&self, scaling: &Scaling) -> Option<UnsignedView> {
        if scaling.bscale != 1.0 || scaling.blank.is_some() {
            return None;
        }
        let bzero = scaling.bzero;
        match self {
            ImageData::U8(v) if bzero == -128.0 => Some(UnsignedView::from_signed_byte(v)),
            ImageData::I16(v) if bzero == U16_OFFSET => Some(UnsignedView::from_offset_i16(v)),
            ImageData::I32(v) if bzero == U32_OFFSET => Some(UnsignedView::from_offset_i32(v)),
            ImageData::I64(v) if bzero == U64_OFFSET => Some(UnsignedView::from_offset_i64(v)),
            _ => None,
        }
    }
}

/// A borrowed, host-endian view of an image's samples, tagged by `BITPIX` — the
/// zero-/low-copy counterpart to the owned [`ImageData`], returned by
/// [`crate::FitsReader::read_image_view`]. Match it exactly like [`ImageData`], but
/// the slices borrow the reader's reused decode scratch (or, for `BITPIX = 8`, the
/// source bytes directly), so a view is valid only until the next read — ideal for a
/// hot loop that processes each image and moves on, since reusing one scratch across
/// reads pays the output allocation (and its page faults) once and even reuses it
/// across *different* `BITPIX`. For samples you need to keep, use the owned
/// [`RawImage::decode`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageView<'a> {
    U8(&'a [u8]),
    I16(&'a [i16]),
    I32(&'a [i32]),
    I64(&'a [i64]),
    F32(&'a [f32]),
    F64(&'a [f64]),
}

impl ImageView<'_> {
    /// The `BITPIX` element kind backing this view.
    pub fn bitpix(&self) -> Bitpix {
        match self {
            ImageView::U8(_) => Bitpix::U8,
            ImageView::I16(_) => Bitpix::I16,
            ImageView::I32(_) => Bitpix::I32,
            ImageView::I64(_) => Bitpix::I64,
            ImageView::F32(_) => Bitpix::F32,
            ImageView::F64(_) => Bitpix::F64,
        }
    }

    /// Number of samples in the view.
    pub fn len(&self) -> usize {
        match self {
            ImageView::U8(v) => v.len(),
            ImageView::I16(v) => v.len(),
            ImageView::I32(v) => v.len(),
            ImageView::I64(v) => v.len(),
            ImageView::F32(v) => v.len(),
            ImageView::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Byte-swap big-endian image bytes `src` into `words` — a `u64`-backed (8-byte-
/// aligned) reused scratch, resized to fit — so [`view_words`] can hand back typed
/// `&[T]` slices over the result. `bitpix` must not be `U8` (that needs no swap; the
/// reader borrows the source bytes directly).
pub(crate) fn swap_into_words(src: &[u8], bitpix: Bitpix, words: &mut Vec<u64>) {
    let count = src.len() / bitpix.elem_size();
    words.resize(src.len().div_ceil(8), 0);
    let p = words.as_mut_ptr() as *mut u8;
    // SAFETY: `words` is `u64`-backed (8-aligned, valid for every BITPIX type's
    // alignment) with room for `src.len()` bytes. Each reinterpretation is write-only,
    // covers exactly `count` elements (= src.len() bytes, in bounds), and `src` is a
    // separate buffer so the typed slice never aliases it.
    unsafe {
        match bitpix {
            Bitpix::I16 => decode_be_into_slice(
                src,
                std::slice::from_raw_parts_mut(p as *mut i16, count),
                i16::from_be_bytes,
            ),
            Bitpix::I32 => decode_be_into_slice(
                src,
                std::slice::from_raw_parts_mut(p as *mut i32, count),
                i32::from_be_bytes,
            ),
            Bitpix::I64 => decode_be_into_slice(
                src,
                std::slice::from_raw_parts_mut(p as *mut i64, count),
                i64::from_be_bytes,
            ),
            Bitpix::F32 => decode_be_into_slice(
                src,
                std::slice::from_raw_parts_mut(p as *mut f32, count),
                f32::from_be_bytes,
            ),
            Bitpix::F64 => decode_be_into_slice(
                src,
                std::slice::from_raw_parts_mut(p as *mut f64, count),
                f64::from_be_bytes,
            ),
            Bitpix::U8 => unreachable!("U8 is handled by the caller, never swapped"),
        }
    }
}

/// Reinterpret the first `nbytes` of a `u64`-backed host-endian scratch (written by
/// [`swap_into_words`]) as a typed [`ImageView`]. `nbytes` is a whole number of
/// `bitpix` elements and `<= words.len() * 8`.
pub(crate) fn view_words(words: &[u64], bitpix: Bitpix, nbytes: usize) -> ImageView<'_> {
    let count = nbytes / bitpix.elem_size();
    let p = words.as_ptr() as *const u8;
    // SAFETY: `words` is `u64`-backed (8-aligned ≥ every type's align); `swap_into_words`
    // wrote all `nbytes` (= count elements) host-endian bytes; int/float types have no
    // invalid bit patterns. So each slice is a valid, fully-initialized `&[T]`.
    unsafe {
        match bitpix {
            Bitpix::U8 => ImageView::U8(std::slice::from_raw_parts(p, count)),
            Bitpix::I16 => ImageView::I16(std::slice::from_raw_parts(p as *const i16, count)),
            Bitpix::I32 => ImageView::I32(std::slice::from_raw_parts(p as *const i32, count)),
            Bitpix::I64 => ImageView::I64(std::slice::from_raw_parts(p as *const i64, count)),
            Bitpix::F32 => ImageView::F32(std::slice::from_raw_parts(p as *const f32, count)),
            Bitpix::F64 => ImageView::F64(std::slice::from_raw_parts(p as *const f64, count)),
        }
    }
}

/// The already-host-endian samples reinterpreted as their raw bytes — every element
/// type is `Pod` (no padding, all bit patterns valid), so the byte view is sound.
#[cfg(feature = "compression")]
fn samples_as_bytes(data: &ImageData) -> &[u8] {
    // SAFETY: a typed sample slice viewed as its own bytes (read-only); length is the
    // element count times the element width.
    unsafe {
        let (ptr, len) = match data {
            ImageData::U8(v) => return v,
            ImageData::I16(v) => (v.as_ptr() as *const u8, v.len() * 2),
            ImageData::I32(v) => (v.as_ptr() as *const u8, v.len() * 4),
            ImageData::I64(v) => (v.as_ptr() as *const u8, v.len() * 8),
            ImageData::F32(v) => (v.as_ptr() as *const u8, v.len() * 4),
            ImageData::F64(v) => (v.as_ptr() as *const u8, v.len() * 8),
        };
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Copy already-host-endian `samples` (a decompressed image) into the `u64`-backed
/// `words` scratch so [`view_words`] can hand back a view — the compressed-image
/// path, whose pixels have no on-disk bytes to swap. Returns the byte length written.
#[cfg(feature = "compression")]
pub(crate) fn copy_samples_into_words(samples: &ImageData, words: &mut Vec<u64>) -> usize {
    let bytes = samples_as_bytes(samples);
    words.resize(bytes.len().div_ceil(8), 0);
    // SAFETY: `words` is `u64`-backed (8-aligned) with room for `bytes.len()`; the
    // reinterpreted destination is write-only and does not alias `samples`.
    unsafe {
        std::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, bytes.len())
            .copy_from_slice(bytes);
    }
    bytes.len()
}

/// An image read from an HDU, in whichever form the reader could give cheaply —
/// returned by [`crate::FitsReader::read_image`] for *both* plain and tiled-
/// compressed images, so callers needn't know which they have. Carries the shape,
/// `BITPIX`, and [`Scaling`]; the pixels are exposed lazily through [`decode`],
/// [`u8`], [`physical`], and [`unsigned`].
///
/// A **plain** image borrows the data unit's big-endian bytes in place (zero-copy);
/// a **compressed** one (`ZIMAGE`) holds the reconstructed host-endian samples it had
/// to decompress. The accessors paper over the difference — e.g. [`u8`] is the
/// zero-copy `BITPIX = 8` plane either way — so you only reach for [`raw_bytes`] when
/// you specifically want the undecoded on-disk bytes (plain images only).
///
/// [`decode`]: RawImage::decode
/// [`u8`]: RawImage::u8
/// [`physical`]: RawImage::physical
/// [`unsigned`]: RawImage::unsigned
/// [`raw_bytes`]: RawImage::raw_bytes
#[derive(Debug)]
pub struct RawImage<'a> {
    pub shape: Vec<usize>,
    pub bitpix: Bitpix,
    pub scaling: Scaling,
    data: ImageBytes<'a>,
}

/// The two forms a [`RawImage`]'s pixels can take, by how it was read.
#[derive(Debug)]
enum ImageBytes<'a> {
    /// Plain image: the data unit's big-endian on-disk bytes, viewed in place over
    /// the source (or the reader's reused scratch for a seeking source).
    Raw(&'a [u8]),
    /// Compressed image (`ZIMAGE`): pixels reconstructed into an owned, host-endian
    /// buffer (only the `compression` feature ever builds this).
    #[cfg_attr(not(feature = "compression"), allow(dead_code))]
    Decoded(ImageData),
}

impl<'a> RawImage<'a> {
    /// A plain image over borrowed big-endian bytes.
    pub(crate) fn raw(
        shape: Vec<usize>,
        bitpix: Bitpix,
        scaling: Scaling,
        bytes: &'a [u8],
    ) -> RawImage<'a> {
        RawImage {
            shape,
            bitpix,
            scaling,
            data: ImageBytes::Raw(bytes),
        }
    }

    /// A compressed image over its reconstructed, host-endian samples.
    #[cfg(feature = "compression")]
    pub(crate) fn decoded(samples: ImageData, shape: Vec<usize>, scaling: Scaling) -> RawImage<'a> {
        RawImage {
            shape,
            bitpix: samples.bitpix(),
            scaling,
            data: ImageBytes::Decoded(samples),
        }
    }

    /// The host-endian samples. For a plain image this byte-swaps the on-disk bytes
    /// into an owned buffer; a compressed image's pixels are already decoded (cloned
    /// out here).
    pub fn decode(&self) -> ImageData {
        match &self.data {
            ImageBytes::Raw(bytes) => ImageData::decode(bytes, self.bitpix),
            ImageBytes::Decoded(samples) => samples.clone(),
        }
    }

    /// The samples as a borrowed `&[u8]` when no byte-swap is needed (`BITPIX = 8`):
    /// a plain image's borrowed on-disk bytes, or a compressed image's decoded `u8`
    /// buffer. `None` for multi-byte element types — use [`RawImage::decode`].
    pub fn u8(&self) -> Option<&[u8]> {
        match &self.data {
            ImageBytes::Raw(bytes) if self.bitpix == Bitpix::U8 => Some(bytes),
            ImageBytes::Decoded(ImageData::U8(v)) => Some(v),
            _ => None,
        }
    }

    /// The undecoded big-endian on-disk bytes — `Some` only for a **plain** image
    /// (zero-copy borrow); `None` for a compressed one, whose pixels were
    /// reconstructed and have no on-disk byte form. Use [`RawImage::decode`] for the
    /// samples regardless of form.
    pub fn raw_bytes(&self) -> Option<&[u8]> {
        match &self.data {
            ImageBytes::Raw(bytes) => Some(bytes),
            ImageBytes::Decoded(_) => None,
        }
    }

    /// The physical-plane values: `BZERO + BSCALE × sample`, `BLANK` → `NaN` (§3.4).
    pub fn physical(&self) -> Vec<f64> {
        match &self.data {
            ImageBytes::Raw(bytes) => ImageData::decode(bytes, self.bitpix).physical(&self.scaling),
            ImageBytes::Decoded(samples) => samples.physical(&self.scaling),
        }
    }

    /// The physical plane narrowed to `f32` in a single pass — the compact, lossy
    /// counterpart to [`physical`](RawImage::physical). The scaling is still evaluated
    /// in `f64` (so each value is the correctly-rounded `f32`), but only one `Vec<f32>`
    /// is allocated rather than a `Vec<f64>` the caller then re-walks to narrow. Prefer
    /// it when the consumer wants `f32` regardless (display, GPU upload, `f32`
    /// pipelines); use [`physical`](RawImage::physical) when you need double precision —
    /// e.g. large `BITPIX = 64` integers or fine `BSCALE`/`BZERO` past `f32`'s range.
    pub fn physical_f32(&self) -> Vec<f32> {
        match &self.data {
            ImageBytes::Raw(bytes) => {
                ImageData::decode(bytes, self.bitpix).physical_f32(&self.scaling)
            }
            ImageBytes::Decoded(samples) => samples.physical_f32(&self.scaling),
        }
    }

    /// Exact typed integers when the scaling is the FITS unsigned (or signed-byte)
    /// convention; `None` otherwise — same rule as [`Image::unsigned`].
    pub fn unsigned(&self) -> Option<UnsignedView> {
        match &self.data {
            ImageBytes::Raw(bytes) => ImageData::decode(bytes, self.bitpix).unsigned(&self.scaling),
            ImageBytes::Decoded(samples) => samples.unsigned(&self.scaling),
        }
    }

    /// The effective element type these samples represent, resolving the unsigned and
    /// signed-byte conventions from `BITPIX` + [`Scaling`] without decoding the pixels.
    pub fn sample_type(&self) -> SampleType {
        SampleType::from_scaling(self.bitpix, &self.scaling)
    }
}

/// The `BZERO`/`TZEROn` offsets that realize the FITS unsigned-integer convention:
/// a sign-bit flip (`2^(n-1)`), exactly representable as `f64`. Shared by the image
/// (`BZERO`) and binary-table (`TZEROn`) unsigned paths.
pub(crate) const U16_OFFSET: f64 = 32_768.0; // 2¹⁵
pub(crate) const U32_OFFSET: f64 = 2_147_483_648.0; // 2³¹
pub(crate) const U64_OFFSET: f64 = 9_223_372_036_854_775_808.0; // 2⁶³

/// The effective element type of an image's *physical* samples — the analogue of
/// cfitsio's image "equivalent type". `BITPIX` records only the stored width and
/// signedness; the FITS unsigned and signed-byte conventions then layer a `BZERO`
/// offset on top (`BSCALE == 1` with `BZERO = 2^(n-1)`, or `BZERO = -128` for signed
/// bytes), so the values actually mean an unsigned (or signed-byte) integer. This
/// enum is what [`RawImage::physical`] / [`RawImage::unsigned`] yield, resolved up
/// front from `BITPIX` + [`Scaling`] without touching the pixels — so a caller can
/// pick a code path (e.g. a per-type normalization range) without re-deriving the
/// `BZERO` convention itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleType {
    /// `BITPIX = 8`, `BZERO = -128`: a signed byte.
    I8,
    /// `BITPIX = 8`: an unsigned byte (the FITS default for `BITPIX = 8`).
    U8,
    /// `BITPIX = 16`, no unsigned offset.
    I16,
    /// `BITPIX = 16`, `BZERO = 2¹⁵`.
    U16,
    /// `BITPIX = 32`, no unsigned offset.
    I32,
    /// `BITPIX = 32`, `BZERO = 2³¹`.
    U32,
    /// `BITPIX = 64`, no unsigned offset.
    I64,
    /// `BITPIX = 64`, `BZERO = 2⁶³`.
    U64,
    /// `BITPIX = -32`.
    F32,
    /// `BITPIX = -64`.
    F64,
}

impl SampleType {
    /// Resolve the effective type from the stored `BITPIX` and its [`Scaling`].
    ///
    /// A signed integer `BITPIX` whose scaling is exactly the unsigned (or signed-byte)
    /// convention — `BSCALE == 1` and `BZERO` the matching sign-bit offset — resolves
    /// to the corresponding unsigned (or `I8`) type; any other scaling leaves the
    /// stored type as-is. `BLANK` does not affect the classification: it marks null
    /// samples *within* a type, not the type itself.
    pub fn from_scaling(bitpix: Bitpix, scaling: &Scaling) -> SampleType {
        let offset = scaling.bscale == 1.0;
        match bitpix {
            Bitpix::U8 if offset && scaling.bzero == -128.0 => SampleType::I8,
            Bitpix::U8 => SampleType::U8,
            Bitpix::I16 if offset && scaling.bzero == U16_OFFSET => SampleType::U16,
            Bitpix::I16 => SampleType::I16,
            Bitpix::I32 if offset && scaling.bzero == U32_OFFSET => SampleType::U32,
            Bitpix::I32 => SampleType::I32,
            Bitpix::I64 if offset && scaling.bzero == U64_OFFSET => SampleType::U64,
            Bitpix::I64 => SampleType::I64,
            Bitpix::F32 => SampleType::F32,
            Bitpix::F64 => SampleType::F64,
        }
    }

    /// `true` for `U8`/`U16`/`U32`/`U64`.
    pub fn is_unsigned(self) -> bool {
        matches!(
            self,
            SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::U64
        )
    }

    /// `true` for `F32`/`F64`.
    pub fn is_float(self) -> bool {
        matches!(self, SampleType::F32 | SampleType::F64)
    }

    /// `true` for every integer variant (signed or unsigned).
    pub fn is_integer(self) -> bool {
        !self.is_float()
    }
}

/// A typed integer realization of the FITS unsigned (and signed-byte) storage
/// conventions — `BSCALE == 1` with `BZERO` the sign-bit offset. Values are exact
/// (no `f64` rounding), recovered by flipping the stored sign bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsignedView {
    /// `BITPIX = 8`, `BZERO = -128`: stored `u8` → `i8`.
    I8(Vec<i8>),
    /// `BITPIX = 16`, `BZERO = 2¹⁵`: stored `i16` → `u16`.
    U16(Vec<u16>),
    /// `BITPIX = 32`, `BZERO = 2³¹`: stored `i32` → `u32`.
    U32(Vec<u32>),
    /// `BITPIX = 64`, `BZERO = 2⁶³`: stored `i64` → `u64`.
    U64(Vec<u64>),
}

impl UnsignedView {
    /// Recover unsigned values from sign-bit-offset storage (the §5.2.5 / Table 19
    /// convention) by flipping the sign bit. Shared by [`Image::unsigned`] and
    /// `ColumnReader::unsigned` so the bit math has one definition.
    pub(crate) fn from_signed_byte(stored: &[u8]) -> UnsignedView {
        UnsignedView::I8(stored.iter().map(|&x| (x ^ 0x80) as i8).collect())
    }
    pub(crate) fn from_offset_i16(stored: &[i16]) -> UnsignedView {
        UnsignedView::U16(stored.iter().map(|&x| (x as u16) ^ 0x8000).collect())
    }
    pub(crate) fn from_offset_i32(stored: &[i32]) -> UnsignedView {
        UnsignedView::U32(stored.iter().map(|&x| (x as u32) ^ 0x8000_0000).collect())
    }
    pub(crate) fn from_offset_i64(stored: &[i64]) -> UnsignedView {
        UnsignedView::U64(
            stored
                .iter()
                .map(|&x| (x as u64) ^ 0x8000_0000_0000_0000)
                .collect(),
        )
    }
}

/// An N-dimensional image: a flat, Fortran-ordered buffer (axis 0 varies
/// fastest), the axis lengths from `NAXISn`, and the scaling map that turns its
/// stored (raw) samples into physical values.
#[derive(Debug, Clone)]
pub struct Image {
    pub shape: Vec<usize>,
    pub samples: ImageData,
    pub scaling: Scaling,
}

impl Image {
    /// Build an image storing a `u16` buffer via the FITS unsigned convention
    /// (`BITPIX = 16`, `BZERO = 2¹⁵`, `BSCALE = 1`) — the inverse of
    /// [`Image::unsigned`]. The writer emits the `BZERO` keyword so it round-trips.
    pub fn from_u16(shape: Vec<usize>, data: &[u16]) -> Image {
        Image::offset_image(
            shape,
            ImageData::I16(data.iter().map(|&x| (x ^ 0x8000) as i16).collect()),
            U16_OFFSET,
        )
    }

    /// Build an image storing a `u32` buffer (`BITPIX = 32`, `BZERO = 2³¹`).
    pub fn from_u32(shape: Vec<usize>, data: &[u32]) -> Image {
        Image::offset_image(
            shape,
            ImageData::I32(data.iter().map(|&x| (x ^ 0x8000_0000) as i32).collect()),
            U32_OFFSET,
        )
    }

    /// Build an image storing a `u64` buffer (`BITPIX = 64`, `BZERO = 2⁶³`).
    pub fn from_u64(shape: Vec<usize>, data: &[u64]) -> Image {
        Image::offset_image(
            shape,
            ImageData::I64(
                data.iter()
                    .map(|&x| (x ^ 0x8000_0000_0000_0000) as i64)
                    .collect(),
            ),
            U64_OFFSET,
        )
    }

    /// Build an image storing a signed-`i8` buffer (`BITPIX = 8`, `BZERO = -128`).
    pub fn from_i8(shape: Vec<usize>, data: &[i8]) -> Image {
        Image::offset_image(
            shape,
            ImageData::U8(data.iter().map(|&x| (x as u8) ^ 0x80).collect()),
            -128.0,
        )
    }

    fn offset_image(shape: Vec<usize>, samples: ImageData, bzero: f64) -> Image {
        Image {
            shape,
            samples,
            scaling: Scaling {
                bscale: 1.0,
                bzero,
                blank: None,
            },
        }
    }

    /// Reinterpret the stored buffer as exact typed integers when the scaling is
    /// precisely a FITS unsigned-integer (or signed-byte) convention: `BSCALE == 1`,
    /// no `BLANK`, and `BZERO` the matching sign-bit offset. Unlike
    /// [`Image::physical`], this is exact for all 64-bit values (no `f64` rounding
    /// past 2⁵³). Returns `None` for any other scaling or element type.
    pub fn unsigned(&self) -> Option<UnsignedView> {
        self.samples.unsigned(&self.scaling)
    }

    /// The physical-plane values: `BZERO + BSCALE × sample` for every sample
    /// (§3.4). Integer samples equal to the `BLANK` sentinel become `NaN`; float
    /// `NaN`/`Inf` pass through. The unsigned-integer convention falls out for
    /// free — e.g. a signed-16 buffer with `BZERO = 32768` yields the `u16` value.
    pub fn physical(&self) -> Vec<f64> {
        self.samples.physical(&self.scaling)
    }

    /// The physical plane narrowed to `f32` in a single pass — the compact, lossy
    /// counterpart to [`physical`](Image::physical); see [`RawImage::physical_f32`].
    pub fn physical_f32(&self) -> Vec<f32> {
        self.samples.physical_f32(&self.scaling)
    }

    /// The effective element type these samples represent, resolving the unsigned and
    /// signed-byte conventions from the stored `BITPIX` + [`Scaling`].
    pub fn sample_type(&self) -> SampleType {
        SampleType::from_scaling(self.samples.bitpix(), &self.scaling)
    }
}

/// Scale an integer sample buffer to the physical plane, mapping the `BLANK`
/// sentinel (a stored integer value) to `NaN`.
fn scale_ints<T, O>(v: &[T], blank: Option<i64>, scale: impl Fn(f64) -> O) -> Vec<O>
where
    T: Copy + Into<i64>,
    O: PhysicalOut,
{
    v.iter()
        .map(|&x| {
            let xi: i64 = x.into();
            if blank == Some(xi) {
                O::from_f64(f64::NAN)
            } else {
                scale(xi as f64)
            }
        })
        .collect()
}

/// Output element type of the physical-plane map. Private, hence sealed: the only
/// implementors are `f64` (the canonical plane, [`ImageData::physical`]) and `f32`
/// (the compact plane, [`ImageData::physical_f32`]). The scaling arithmetic always
/// runs in `f64`; `from_f64` is the final per-element narrowing.
trait PhysicalOut: Copy {
    fn from_f64(value: f64) -> Self;
}

impl PhysicalOut for f64 {
    fn from_f64(value: f64) -> f64 {
        value
    }
}

impl PhysicalOut for f32 {
    fn from_f64(value: f64) -> f32 {
        value as f32
    }
}

/// The linear `BSCALE`/`BZERO` map from a stored value to its physical value,
/// plus the integer `BLANK` sentinel marking undefined pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    pub bscale: f64,
    pub bzero: f64,
    pub blank: Option<i64>,
}

impl Scaling {
    /// The public entry point is [`Header::scaling`](crate::Header::scaling).
    pub(crate) fn from_header(header: &Header) -> Scaling {
        Scaling {
            bscale: header.get_real("BSCALE").unwrap_or(1.0),
            bzero: header.get_real("BZERO").unwrap_or(0.0),
            blank: header.get_integer("BLANK"),
        }
    }

    /// `true` when decoding needs no arithmetic — just an endian swap or copy.
    pub fn is_identity(&self) -> bool {
        self.bscale == 1.0 && self.bzero == 0.0
    }
}

/// An image as an N-dimensional [`ndarray`] array, tagged by element type — the n-D
/// analog of [`ImageData`], from [`Image::into_ndarray`] / [`RawImage::to_ndarray`].
/// Requires the `ndarray` feature.
///
/// Axes are in **FITS order** (axis 0 = `NAXIS1`, the fastest-varying), so a 2-D image
/// indexes `arr[[x, y]]`. For the NumPy/Astropy `arr[[y, x]]` convention call
/// `.reversed_axes()` — a zero-copy stride swap.
#[cfg(feature = "ndarray")]
#[derive(Debug, Clone, PartialEq)]
pub enum ImageArray {
    U8(ndarray::ArrayD<u8>),
    I16(ndarray::ArrayD<i16>),
    I32(ndarray::ArrayD<i32>),
    I64(ndarray::ArrayD<i64>),
    F32(ndarray::ArrayD<f32>),
    F64(ndarray::ArrayD<f64>),
}

/// Wrap a flat, FITS-ordered buffer (axis 1 fastest) in an [`ndarray`] without
/// copying — a Fortran-order array so `arr[[i1, i2, …]]` maps to the right element.
/// `NAXIS = 0` (no pixels) becomes an empty 1-D array.
#[cfg(feature = "ndarray")]
fn fortran_array<T>(shape: &[usize], data: Vec<T>) -> ndarray::ArrayD<T> {
    use ndarray::ShapeBuilder as _;
    let dims: Vec<usize> = if shape.is_empty() {
        vec![0]
    } else {
        shape.to_vec()
    };
    ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&dims).f(), data)
        .expect("decoded buffer length equals the axis product")
}

#[cfg(feature = "ndarray")]
impl ImageData {
    /// Move the samples into a typed N-d [`ImageArray`] of `shape` (FITS axis order,
    /// fastest first) — zero-copy: the backing `Vec` is reused, not cloned.
    pub fn into_ndarray(self, shape: &[usize]) -> ImageArray {
        match self {
            ImageData::U8(v) => ImageArray::U8(fortran_array(shape, v)),
            ImageData::I16(v) => ImageArray::I16(fortran_array(shape, v)),
            ImageData::I32(v) => ImageArray::I32(fortran_array(shape, v)),
            ImageData::I64(v) => ImageArray::I64(fortran_array(shape, v)),
            ImageData::F32(v) => ImageArray::F32(fortran_array(shape, v)),
            ImageData::F64(v) => ImageArray::F64(fortran_array(shape, v)),
        }
    }
}

#[cfg(feature = "ndarray")]
impl RawImage<'_> {
    /// The physical plane as an N-d `f64` array (`BZERO + BSCALE × sample`, `BLANK` →
    /// `NaN`), in FITS axis order — index `arr[[x, y]]`.
    pub fn physical_array(&self) -> ndarray::ArrayD<f64> {
        fortran_array(&self.shape, self.physical())
    }

    /// The samples as a typed N-d [`ImageArray`], in FITS axis order. Decodes into an
    /// owned buffer first (the array then owns it, no further copy).
    pub fn to_ndarray(&self) -> ImageArray {
        self.decode().into_ndarray(&self.shape)
    }
}

#[cfg(feature = "ndarray")]
impl Image {
    /// The physical plane as an N-d `f64` array (`BZERO + BSCALE × sample`, `BLANK` →
    /// `NaN`), in FITS axis order — index `arr[[x, y]]`.
    pub fn physical_array(&self) -> ndarray::ArrayD<f64> {
        fortran_array(&self.shape, self.physical())
    }

    /// Move the samples into a typed N-d [`ImageArray`], in FITS axis order — zero-copy.
    pub fn into_ndarray(self) -> ImageArray {
        let Image { shape, samples, .. } = self;
        samples.into_ndarray(&shape)
    }
}

#[cfg(test)]
mod tests;
