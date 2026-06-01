//! Byte sources the reader fetches header and data units from.
//!
//! A [`Source`] hands back the bytes for a `[offset, offset+len)` range. In-memory
//! sources ([`SliceSource`], and `MmapSource` under the `mmap` feature) return a
//! zero-copy borrow, so decoding reads straight from the resident bytes; a
//! streaming source ([`StreamSource`] over any `Read + Seek`) copies the range into
//! the reader's reused scratch first. For in-memory sources that saves a whole
//! memory pass over the data — the staging copy the seeking path can't avoid.

use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;

use crate::error::FitsError;
use crate::error::Result;

/// A seekable byte source the reader fetches HDU header and data units from.
pub trait Source {
    /// Total byte length of the source. Fixed for the source's lifetime and used to
    /// reject ranges that run past the end before allocating for them.
    fn size(&self) -> u64;

    /// The `len` bytes at `offset`, borrowed. In-memory sources return a slice of
    /// themselves (zero-copy); a streaming source reads into `scratch` and returns a
    /// slice of that. Errors if the range runs past the source.
    fn slice<'a>(
        &'a mut self,
        offset: u64,
        len: usize,
        scratch: &'a mut Vec<u8>,
    ) -> Result<&'a [u8]>;

    /// The `len` bytes at `offset` in a fresh owned buffer — used where the bytes
    /// must outlive the read (the parsed table / ASCII-table backing store). Kept
    /// distinct from `slice().to_vec()` so a streaming source reads straight into
    /// the owned buffer (one copy) instead of staging through `scratch` first (two).
    fn read_owned(&mut self, offset: u64, len: usize) -> Result<Vec<u8>>;
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
    pub(crate) fn new(mut inner: R) -> Result<StreamSource<R>> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(StreamSource { inner, len })
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
        scratch.resize(len, 0);
        self.inner.read_exact(scratch.as_mut_slice())?;
        Ok(&scratch[..len])
    }

    fn read_owned(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        check_range(offset, len, self.len)?;
        self.inner.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        self.inner.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// An in-memory byte source: the whole file already resident as a slice (e.g. an
/// mmap, or bytes read up front). Fetches return a zero-copy borrow, so a decode
/// reads straight from these bytes with no staging copy.
#[derive(Debug)]
pub struct SliceSource<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceSource<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> SliceSource<'a> {
        SliceSource { bytes }
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
        check_range(offset, len, self.bytes.len() as u64)?;
        let off = offset as usize;
        Ok(&self.bytes[off..off + len])
    }

    fn read_owned(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        check_range(offset, len, self.bytes.len() as u64)?;
        let off = offset as usize;
        Ok(self.bytes[off..off + len].to_vec())
    }
}

/// A memory-mapped file source: the kernel pages the file in on access, and data
/// units decode straight from the mapping (no staging copy, no read syscalls). The
/// owned [`memmap2::Mmap`] keeps the mapping alive for the reader's lifetime.
#[cfg(feature = "mmap")]
#[derive(Debug)]
pub struct MmapSource {
    map: memmap2::Mmap,
}

#[cfg(feature = "mmap")]
impl MmapSource {
    pub(crate) fn open(path: &std::path::Path) -> Result<MmapSource> {
        let file = std::fs::File::open(path)?;
        // SAFETY: standard mmap contract — the mapping is read-only and owned here
        // (no mutable view is ever handed out). The one inherent caveat is that an
        // external process truncating or modifying the file underneath can change the
        // bytes; choosing `mmap` accepts that, exactly as in cfitsio/astropy.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        Ok(MmapSource { map })
    }
}

#[cfg(feature = "mmap")]
impl Source for MmapSource {
    fn size(&self) -> u64 {
        self.map.len() as u64
    }

    fn slice<'a>(
        &'a mut self,
        offset: u64,
        len: usize,
        _scratch: &'a mut Vec<u8>,
    ) -> Result<&'a [u8]> {
        check_range(offset, len, self.map.len() as u64)?;
        let off = offset as usize;
        Ok(&self.map[off..off + len])
    }

    fn read_owned(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        check_range(offset, len, self.map.len() as u64)?;
        let off = offset as usize;
        Ok(self.map[off..off + len].to_vec())
    }
}
