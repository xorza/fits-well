//! Byte sources the reader fetches header and data units from.
//!
//! A [`Source`] hands back the bytes for a `[offset, offset+len)` range. The
//! in-memory source ([`SliceSource`]) returns a zero-copy borrow, so decoding reads
//! straight from the resident bytes; a streaming source ([`StreamSource`] over any
//! `Read + Seek`) copies the range into the reader's reused scratch first. In memory
//! that saves a whole pass over the data — the staging copy the seeking path can't
//! avoid.

use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;

use crate::allocation;
use crate::error::FitsError;
use crate::error::Result;

/// Seals [`Source`] so it is a closed set implemented only by the in-tree source
/// types (`StreamSource`/`SliceSource`) — not an extension point, and
/// its `slice`/`read_append` plumbing is not a public contract.
mod sealed {
    #[expect(
        unnameable_types,
        reason = "sealed: outside code may use it, never name it"
    )]
    pub trait Sealed {}
}
impl<R> sealed::Sealed for StreamSource<R> {}
impl sealed::Sealed for SliceSource<'_> {}

/// A seekable byte source the reader fetches HDU header and data units from.
/// Sealed — implemented only by this crate's source types, never externally.
#[expect(
    unnameable_types,
    reason = "sealed: outside code may use it, never name it"
)]
pub trait Source: sealed::Sealed {
    /// Total byte length of the source. Fixed for the source's lifetime and used to
    /// reject ranges that run past the end before allocating for them.
    fn size(&self) -> u64;

    /// The `len` bytes at `offset`, borrowed. The in-memory source returns a slice of
    /// itself (zero-copy); a streaming source reads into `scratch` and returns a
    /// slice of that. Errors if the range runs past the source.
    ///
    /// The returned borrow is tied to `&mut self` (a streaming source genuinely seeks
    /// and mutates), so a reader hands out one data unit at a time even for a fully
    /// resident `SliceSource` — an image read borrows `&mut self`, and to hold several
    /// images at once you read each into an owned `Image`. This uniform
    /// single-path API is a deliberate tradeoff; if a caller ever needs *concurrent
    /// zero-copy* views from a resident file, the fix is a separate resident-only
    /// accessor that borrows with the source's own lifetime (not `&mut self`), added
    /// then rather than designed for speculatively.
    fn slice<'a>(
        &'a mut self,
        offset: u64,
        len: usize,
        scratch: &'a mut Vec<u8>,
    ) -> Result<&'a [u8]>;

    /// Append the `len` bytes at `offset` to `out` — used where the bytes must outlive
    /// the read (a parsed table's storage, a table selection). Kept distinct from
    /// `slice` so a streaming source reads straight into `out` (one copy) instead of
    /// staging through `scratch` first (two).
    fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()>;
}

/// Reject `[offset, offset+len)` that overflows or runs past `size`. A hostile
/// header can claim a unit far larger than the file; refusing up front avoids a
/// huge allocation that would only fail at `read_exact`.
fn check_range(offset: u64, len: usize, size: u64) -> Result<()> {
    if offset.checked_add(len as u64).is_none_or(|end| end > size) {
        return Err(FitsError::UnexpectedEof);
    }
    Ok(())
}

/// A streaming `Read + Seek` source. Each fetch seeks and copies the range out —
/// there is no resident image to borrow, so reads cost one extra memory pass.
#[derive(Debug)]
pub struct StreamSource<R> {
    inner: R,
    len: u64,
}

impl<R: Read + Seek> StreamSource<R> {
    /// Capture the source length once (a single seek to the end), so later reads
    /// bounds-check without re-seeking.
    pub(super) fn new(mut inner: R) -> Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self { inner, len })
    }

    pub(super) fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read + Seek> Source for StreamSource<R> {
    fn size(&self) -> u64 {
        self.len
    }

    fn slice<'a>(
        &'a mut self,
        offset: u64,
        len: usize,
        scratch: &'a mut Vec<u8>,
    ) -> Result<&'a [u8]> {
        check_range(offset, len, self.len)?;
        self.inner.seek(SeekFrom::Start(offset))?;
        // Resize keeps `scratch`'s capacity across calls, so a reused buffer
        // reallocates only when a larger unit appears.
        allocation::try_resize(scratch, len, 0)?;
        self.inner.read_exact(scratch.as_mut_slice())?;
        Ok(&scratch[..len])
    }

    fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
        check_range(offset, len, self.len)?;
        self.inner.seek(SeekFrom::Start(offset))?;
        let start = out.len();
        let end = start.checked_add(len).ok_or(FitsError::DataUnitOverflow)?;
        allocation::try_resize(out, end, 0)?;
        self.inner.read_exact(&mut out[start..])?;
        Ok(())
    }
}

/// An in-memory byte source: the whole file already resident as a slice (a mapping
/// the caller holds, or bytes read up front). Fetches return a zero-copy borrow, so
/// a decode reads straight from these bytes with no staging copy.
#[derive(Debug)]
pub struct SliceSource<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceSource<'a> {
    pub(super) const fn new(bytes: &'a [u8]) -> Self {
        SliceSource { bytes }
    }

    pub(super) const fn into_bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// `[offset, offset+len)` of the bytes, bounds-checked.
    fn range(&self, offset: u64, len: usize) -> Result<&'a [u8]> {
        check_range(offset, len, self.bytes.len() as u64)?;
        let start = offset as usize;
        Ok(&self.bytes[start..start + len])
    }
}

impl Source for SliceSource<'_> {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn slice<'a>(
        &'a mut self,
        offset: u64,
        len: usize,
        _scratch: &'a mut Vec<u8>,
    ) -> Result<&'a [u8]> {
        self.range(offset, len)
    }

    fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
        let bytes = self.range(offset, len)?;
        allocation::try_reserve(out, len)?;
        out.extend_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::error::Result;
    use crate::reader::source::SliceSource;
    use crate::reader::source::Source;
    use crate::reader::source::sealed;

    #[derive(Debug)]
    pub(crate) struct CountingSource<'a> {
        pub(crate) inner: SliceSource<'a>,
        /// The bytes fetched through either path.
        pub(crate) fetched: Rc<Cell<usize>>,
    }

    impl sealed::Sealed for CountingSource<'_> {}

    impl Source for CountingSource<'_> {
        fn size(&self) -> u64 {
            self.inner.size()
        }

        fn slice<'a>(
            &'a mut self,
            offset: u64,
            len: usize,
            scratch: &'a mut Vec<u8>,
        ) -> Result<&'a [u8]> {
            self.fetched.set(self.fetched.get() + len);
            self.inner.slice(offset, len, scratch)
        }

        fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
            self.fetched.set(self.fetched.get() + len);
            self.inner.read_append(offset, len, out)
        }
    }
}
