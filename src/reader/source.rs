//! Byte sources the reader fetches header and data units from.
//!
//! A [`Source`] hands back the bytes for a `[offset, offset+len)` range. The
//! in-memory source ([`SliceSource`]) returns a zero-copy borrow, so decoding reads
//! straight from the resident bytes; a streaming source ([`StreamSource`] over any
//! `Read + Seek`) reads the range into a buffer: the caller's own for `read_into` and
//! `read_append`, else the reader's reused staging buffer, a copy the resident source
//! never makes.

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
pub(super) fn check_range(offset: u64, len: usize, size: u64) -> Result<()> {
    if offset.checked_add(len as u64).is_none_or(|end| end > size) {
        return Err(FitsError::UnexpectedEof);
    }
    Ok(())
}

/// A streaming `Read + Seek` source. Each fetch reads the range into a buffer — there is
/// no resident image to borrow — and moves only when the range does not start where the
/// last read ended.
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
    /// Move to `offset`, which `check_range` bounded: by `seek_relative` from the known
    /// position, which a `BufReader` serves from its buffer when the hop lands in it, or
    /// from the start when the position is not known.
    fn seek_to(&mut self, offset: u64) -> Result<()> {
        match self.position {
            Some(position) if position == offset => {}
            Some(position) => {
                self.position = None;
                let hop = i64::try_from(i128::from(offset) - i128::from(position))
                    .expect("two offsets inside a file");
                self.inner.seek_relative(hop)?;
            }
            None => {
                self.inner.seek(SeekFrom::Start(offset))?;
            }
        }
        Ok(())
    }

    fn read_at(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
        check_range(offset, len, self.len)?;
        self.seek_to(offset)?;
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
        // reallocates only when a larger unit appears, and then to that unit's size. The
        // range is checked first, so a size past the source never grows it.
        check_range(offset, len, self.len)?;
        scratch.clear();
        allocation::try_grow_to(scratch, len)?;
        self.read_at(offset, len, scratch)?;
        Ok(scratch)
    }

    fn read_append(&mut self, offset: u64, len: usize, out: &mut Vec<u8>) -> Result<()> {
        self.read_at(offset, len, out)
    }

    fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        check_range(offset, out.len(), self.len)?;
        self.seek_to(offset)?;
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
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
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
    use std::cell::RefCell;
    use std::io::{self, Cursor, Read, Seek, SeekFrom};
    use std::ops::Range;
    use std::rc::Rc;

    use crate::reader::FitsReader;
    use crate::reader::source::StreamSource;

    /// What a [`Counting`] cursor served: the byte range of each read and the seeks.
    #[derive(Debug, Default)]
    pub(crate) struct Fetches {
        pub(crate) ranges: Vec<Range<usize>>,
        pub(crate) seeks: usize,
    }

    impl Fetches {
        /// The bytes the reads served in all.
        pub(crate) fn bytes(&self) -> usize {
            self.ranges.iter().map(Range::len).sum()
        }
    }

    /// A cursor over bytes that records each read it serves and counts its seeks, so a
    /// test sees every fetch a stream source makes.
    #[derive(Debug)]
    pub(crate) struct Counting<'a> {
        inner: Cursor<&'a [u8]>,
        fetches: Rc<RefCell<Fetches>>,
    }

    impl<'a> Counting<'a> {
        pub(crate) fn new(bytes: &'a [u8]) -> Self {
            Self {
                inner: Cursor::new(bytes),
                fetches: Rc::default(),
            }
        }

        /// The record of what this cursor serves, shared, to read after the cursor moves
        /// into a reader.
        pub(crate) fn fetches(&self) -> Rc<RefCell<Fetches>> {
            Rc::clone(&self.fetches)
        }
    }

    impl Read for Counting<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let start = usize::try_from(self.inner.position()).unwrap();
            let count = self.inner.read(buffer)?;
            if count != 0 {
                self.fetches.borrow_mut().ranges.push(start..start + count);
            }
            Ok(count)
        }
    }

    impl Seek for Counting<'_> {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.fetches.borrow_mut().seeks += 1;
            self.inner.seek(position)
        }
    }

    /// A stream reader of bytes over a [`Counting`] cursor, scanned.
    #[derive(Debug)]
    pub(crate) struct CountedReader<'a> {
        pub(crate) reader: FitsReader<StreamSource<Counting<'a>>>,
        fetches: Rc<RefCell<Fetches>>,
        scanned: usize,
    }

    impl<'a> CountedReader<'a> {
        /// The reader of `bytes`, scanned.
        pub(crate) fn open(bytes: &'a [u8]) -> Self {
            let counting = Counting::new(bytes);
            let fetches = counting.fetches();
            let reader = FitsReader::open(counting).unwrap();
            let scanned = fetches.borrow().bytes();
            Self {
                reader,
                fetches,
                scanned,
            }
        }

        /// The bytes fetched since the scan.
        pub(crate) fn fetched(&self) -> usize {
            self.fetches.borrow().bytes() - self.scanned
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::fits_error::FitsError;
    use crate::reader::source::internals::Counting;
    use crate::reader::source::{Source, StreamSource};
    use std::assert_matches;

    const BYTES: [u8; 100] = {
        let mut bytes = [0; 100];
        let mut i = 0;
        while i < 100 {
            bytes[i] = i as u8;
            i += 1;
        }
        bytes
    };

    fn source() -> StreamSource<Counting<'static>> {
        StreamSource::new(Counting::new(&BYTES)).unwrap()
    }

    /// A range past the end is refused before the staging buffer grows for it, so a size a
    /// header claims leaves no capacity behind.
    #[test]
    fn slice_past_the_end_grows_no_staging() {
        let mut source = source();
        let mut scratch = Vec::new();
        assert_matches!(
            source.slice(98, 1 << 30, &mut scratch),
            Err(FitsError::UnexpectedEof)
        );
        assert_eq!(scratch.capacity(), 0);
        assert_eq!(source.slice(98, 2, &mut scratch).unwrap(), [98, 99]);
    }

    /// A read that starts where the last one ended does not seek: the length probe at
    /// the open seeks once, the read at 10 once, and the read at 15 not at all.
    #[test]
    fn read_after_a_read_does_not_seek() {
        let mut source = source();
        let mut out = Vec::new();
        source.read_append(10, 5, &mut out).unwrap();
        source.read_append(15, 5, &mut out).unwrap();
        assert_eq!(out, (10..20).collect::<Vec<u8>>());
        assert_eq!(source.inner.fetches().borrow().seeks, 2);
        assert_matches!(
            source.read_append(98, 5, &mut out),
            Err(FitsError::UnexpectedEof)
        );
    }
}
