//! [`PlainRead`]: the stored bytes of a plain image or random-groups data unit, read by
//! runs.

use std::ops::Range;

use crate::allocation;
use crate::fits_error::{FitsError, Result};
use crate::hdu::image_geometry::ImageGeometry;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::region::{ByteRun, SectionRuns};
use crate::image_model::sample::Sample;
use crate::image_model::view::View;
use crate::image_model::words;
use crate::image_model::{shape_product, validate_image_region};
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source::{DataSource, offset_at};
use crate::reader::source::Source;

/// Where the stored bytes of a region of a plain image lie in its data unit: one run for
/// the whole image, or the runs of a section.
#[derive(Debug, Clone, Copy)]
pub(super) enum StoredRuns<'a> {
    All { len: usize, shape: &'a [usize] },
    Section { runs: SectionRuns<'a>, len: usize },
}

impl<'a> StoredRuns<'a> {
    /// The runs of the stored bytes of the section `ranges` selects of the plain image
    /// `image`, whose shape is written to `section_shape`.
    pub(super) fn section(
        image: &'a ImageGeometry,
        ranges: &'a [Range<usize>],
        section_shape: &'a mut Vec<usize>,
    ) -> Result<Self> {
        let element_size = image.bitpix.elem_size();
        validate_image_region(ranges, &image.shape, section_shape)?;
        let len = shape_product(section_shape)?
            .checked_mul(element_size)
            .ok_or(FitsError::DataUnitOverflow)?;
        Ok(Self::Section {
            runs: SectionRuns {
                shape: &image.shape,
                ranges,
                selected_shape: section_shape,
                element_size,
            },
            len,
        })
    }

    /// The stored bytes of the region.
    pub(super) const fn len(&self) -> usize {
        match self {
            Self::All { len, .. } | Self::Section { len, .. } => *len,
        }
    }

    /// The shape of the region.
    pub(super) const fn shape(&self) -> &'a [usize] {
        match self {
            Self::All { shape, .. } => shape,
            Self::Section { runs, .. } => runs.selected_shape,
        }
    }

    /// Give `visit` each run, in order.
    fn visit(self, mut visit: impl FnMut(ByteRun) -> Result<()>) -> Result<()> {
        match self {
            Self::All { len, .. } => visit(ByteRun { offset: 0, len }),
            Self::Section { runs, .. } => runs.visit(visit),
        }
    }
}

/// A plain image's data unit read by runs, each fed to the checksum, when one is summed,
/// as it is read.
#[derive(Debug)]
pub(super) struct PlainRead<'a, S> {
    pub(super) data: &'a mut DataSource<S>,
    pub(super) data_offset: u64,
    pub(super) sum: Option<&'a mut DataChecksum>,
}

impl<S: Source> PlainRead<'_, S> {
    /// Fill `out` with the samples of `runs`, host-endian: decoded from the borrowed
    /// bytes of a resident source, or read into `out` and swapped in place, with no
    /// staging copy.
    fn fill<T: Sample>(&mut self, runs: StoredRuns<'_>, out: &mut [T]) -> Result<()> {
        let size = size_of::<T>();
        debug_assert_eq!(size_of_val(out), runs.len(), "a sample for each stored one");
        let resident = self.data.is_resident();
        let mut at = 0;
        runs.visit(|run| {
            let offset = offset_at(self.data_offset, run.offset)?;
            let samples = &mut out[at..at + run.len / size];
            at += samples.len();
            if resident {
                let bytes = self.data.resident_range(offset, run.len)?;
                if let Some(sum) = self.sum.as_deref_mut() {
                    sum.feed(offset, bytes);
                }
                T::decode_be(bytes, samples);
            } else {
                let bytes = words::bytes_mut(samples);
                self.data.read_into(offset, bytes)?;
                if let Some(sum) = self.sum.as_deref_mut() {
                    sum.feed(offset, bytes);
                }
            }
            Ok(())
        })?;
        if !resident {
            T::swap_be_in_place(out);
        }
        Ok(())
    }

    /// The samples of `runs`, of type `bitpix`, owned.
    pub(super) fn data(&mut self, runs: StoredRuns<'_>, bitpix: Bitpix) -> Result<Data> {
        let count = runs.len() / bitpix.elem_size();
        Ok(match bitpix {
            Bitpix::U8 => Data::U8(self.filled(runs, count)?),
            Bitpix::I16 => Data::I16(self.filled(runs, count)?),
            Bitpix::I32 => Data::I32(self.filled(runs, count)?),
            Bitpix::I64 => Data::I64(self.filled(runs, count)?),
            Bitpix::F32 => Data::F32(self.filled(runs, count)?),
            Bitpix::F64 => Data::F64(self.filled(runs, count)?),
        })
    }

    /// The samples of `runs`, owned: decoded from the borrowed bytes of a resident source
    /// with no fill first, or read into a zeroed buffer and swapped in place.
    fn filled<T: Sample>(&mut self, runs: StoredRuns<'_>, count: usize) -> Result<Vec<T>> {
        if !self.data.is_resident() {
            let mut values = allocation::try_zeroed(T::default(), count)?;
            self.fill(runs, &mut values)?;
            return Ok(values);
        }
        let mut values = Vec::new();
        allocation::try_reserve(&mut values, count)?;
        runs.visit(|run| {
            let offset = offset_at(self.data_offset, run.offset)?;
            let bytes = self.data.resident_range(offset, run.len)?;
            if let Some(sum) = self.sum.as_deref_mut() {
                sum.feed(offset, bytes);
            }
            T::extend_from_be(bytes, &mut values);
            Ok(())
        })?;
        Ok(values)
    }

    /// The samples of `runs`, of type `bitpix`, in `words`, viewed.
    pub(super) fn words<'w>(
        &mut self,
        runs: StoredRuns<'_>,
        bitpix: Bitpix,
        words: &'w mut Vec<u64>,
    ) -> Result<View<'w>> {
        let len = runs.len();
        let count = len / bitpix.elem_size();
        allocation::try_resize(words, len.div_ceil(8), 0)?;
        match bitpix {
            Bitpix::U8 => self.fill(runs, words::samples_mut::<u8>(words, count))?,
            Bitpix::I16 => self.fill(runs, words::samples_mut::<i16>(words, count))?,
            Bitpix::I32 => self.fill(runs, words::samples_mut::<i32>(words, count))?,
            Bitpix::I64 => self.fill(runs, words::samples_mut::<i64>(words, count))?,
            Bitpix::F32 => self.fill(runs, words::samples_mut::<f32>(words, count))?,
            Bitpix::F64 => self.fill(runs, words::samples_mut::<f64>(words, count))?,
        }
        Ok(bitpix.view_words(words, len))
    }

    /// Append the physical values of `runs`, of type `bitpix`, to `out`, from the
    /// borrowed bytes of a resident source or a bounded chunk at a time.
    pub(super) fn physical<O: PhysicalSample>(
        &mut self,
        runs: StoredRuns<'_>,
        image: &ImageGeometry,
        out: &mut Vec<O>,
    ) -> Result<()> {
        allocation::try_reserve(out, runs.len() / image.bitpix.elem_size())?;
        runs.visit(|run| {
            let offset = offset_at(self.data_offset, run.offset)?;
            let sum = &mut self.sum;
            self.data.chunks(offset, run.len, |offset, bytes| {
                if let Some(sum) = sum.as_deref_mut() {
                    sum.feed(offset, bytes);
                }
                image.bitpix.extend_physical_be(bytes, &image.scaling, out);
                Ok(())
            })
        })
    }
}
