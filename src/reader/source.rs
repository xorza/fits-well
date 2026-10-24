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
use crate::fits_error::FitsError;
use crate::fits_error::Result;

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
    /// staging through `scratch` first (two). `out` grows as a `Vec` grows, so a run of
    /// appends costs linear time.
    fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()>;

    /// Fill `out` with the bytes at `offset`: a read that lands in the caller's buffer.
    fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()>;

    /// The whole source, borrowed, when it is resident in memory: a read then decodes
    /// from these bytes and copies nothing first. `None` for a streaming source.
    fn resident(&self) -> Option<&[u8]>;
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

/// A streaming `Read + Seek` source. Each fetch copies the range out — there is no
/// resident image to borrow, so reads cost one extra memory pass — and seeks only when
/// the range does not start where the last read ended.
#[derive(Debug)]
pub struct StreamSource<R> {
    inner: R,
    len: u64,
    /// Where `inner` stands, when known: after a read, the end of what it read.
    position: Option<u64>,
}

impl<R: Read + Seek> StreamSource<R> {
    /// Capture the source length once (a single seek to the end), so later reads
    /// bounds-check without re-seeking.
    pub(crate) fn new(mut inner: R) -> Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self {
            inner,
            len,
            position: Some(len),
        })
    }

    pub(super) fn into_inner(self) -> R {
        self.inner
    }

    /// Append the `len` bytes at `offset` to `out`, with no fill of its new capacity.
    fn read_at(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
        check_range(offset, len, self.len)?;
        if self.position != Some(offset) {
            self.position = None;
            self.inner.seek(SeekFrom::Start(offset))?;
        }
        allocation::try_reserve(out, len)?;
        let read = (&mut self.inner).take(len as u64).read_to_end(out);
        self.position = read.as_ref().ok().map(|&read| offset + read as u64);
        if read? != len {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(())
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
        // `clear` keeps `scratch`'s capacity across calls, so a reused buffer
        // reallocates only when a larger unit appears.
        scratch.clear();
        self.read_at(offset, len, scratch)?;
        Ok(scratch)
    }

    fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
        self.read_at(offset, len, out)
    }

    fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        check_range(offset, out.len(), self.len)?;
        if self.position != Some(offset) {
            self.position = None;
            self.inner.seek(SeekFrom::Start(offset))?;
        }
        let read = self.inner.read_exact(out);
        self.position = read.as_ref().ok().map(|()| offset + out.len() as u64);
        Ok(read?)
    }

    fn resident(&self) -> Option<&[u8]> {
        None
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

    fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        out.copy_from_slice(self.range(offset, out.len())?);
        Ok(())
    }

    fn resident(&self) -> Option<&[u8]> {
        Some(self.bytes)
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::fits_error::Result;
    use crate::reader::FitsReader;
    use crate::reader::source::SliceSource;
    use crate::reader::source::Source;
    use crate::reader::source::sealed;

    /// A reader of bytes whose source counts each byte it fetches.
    #[derive(Debug)]
    pub(crate) struct CountedReader<'a> {
        pub(crate) reader: FitsReader<CountingSource<'a>>,
        fetched: Rc<Cell<usize>>,
        scanned: usize,
    }

    impl<'a> CountedReader<'a> {
        /// The reader of `bytes`, scanned.
        pub(crate) fn open(bytes: &'a [u8]) -> Self {
            let fetched = Rc::new(Cell::new(0));
            let source = CountingSource {
                inner: SliceSource::new(bytes),
                fetched: Rc::clone(&fetched),
            };
            let reader = FitsReader::from_source(source).unwrap();
            let scanned = fetched.get();
            Self {
                reader,
                fetched,
                scanned,
            }
        }

        /// The bytes fetched since the scan.
        pub(crate) fn fetched(&self) -> usize {
            self.fetched.get() - self.scanned
        }
    }

    #[derive(Debug)]
    pub(crate) struct CountingSource<'a> {
        inner: SliceSource<'a>,
        /// The bytes fetched through either path.
        fetched: Rc<Cell<usize>>,
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

        fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
            self.fetched.set(self.fetched.get() + out.len());
            self.inner.read_into(offset, out)
        }

        /// Not resident, so every read is a fetch it counts.
        fn resident(&self) -> Option<&[u8]> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Seek, SeekFrom};

    use crate::fits_error::FitsError;
    use crate::reader::source::{Source, StreamSource};

    /// A cursor that counts its seeks and its reads.
    #[derive(Debug)]
    struct Counted {
        inner: Cursor<Vec<u8>>,
        seeks: usize,
        reads: usize,
    }

    impl Read for Counted {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            self.inner.read(out)
        }
    }

    impl Seek for Counted {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.seeks += 1;
            self.inner.seek(position)
        }
    }

    fn source() -> StreamSource<Counted> {
        StreamSource::new(Counted {
            inner: Cursor::new((0..100).collect()),
            seeks: 0,
            reads: 0,
        })
        .unwrap()
    }

    /// A read that starts where the last one ended does not seek: the length probe at
    /// the open seeks once, the read at 10 once, and the read at 15 not at all.
    #[test]
    fn a_read_after_a_read_does_not_seek() {
        let mut source = source();
        let mut out = Vec::new();
        source.read_append(10, 5, &mut out).unwrap();
        source.read_append(15, 5, &mut out).unwrap();
        assert_eq!(out, (10..20).collect::<Vec<u8>>());
        assert_eq!(source.inner.seeks, 2);
        assert!(matches!(
            source.read_append(98, 5, &mut out),
            Err(FitsError::UnexpectedEof)
        ));
    }
}
