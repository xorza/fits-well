//! Whole-column reads for tests and benches: each decodes into a new buffer through the
//! public reads into a caller's buffer, as a one-off check takes a column.

use crate::ascii::ascii_text::AsciiText;
use crate::ascii::{AsciiColumnData, AsciiColumnReader};
use crate::bintable::column_data::ColumnData;
use crate::bintable::column_reader::ColumnReader;
use crate::fits_error::Result;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::view::Borrowed;
use crate::ragged::Ragged;

/// A binary-table column read whole into new buffers.
pub trait WholeColumn {
    fn raw(&self) -> Result<ColumnData>;
    fn physical(&self) -> Result<Vec<f64>>;
    fn unsigned(&self) -> Result<Option<UnsignedData>>;
    fn vla(&self) -> Result<Ragged<ColumnData>>;
    fn vla_physical(&self) -> Result<Ragged<Vec<f64>>>;
}

impl WholeColumn for ColumnReader<'_> {
    fn raw(&self) -> Result<ColumnData> {
        let mut out = ColumnData::U8(Vec::new());
        self.raw_into(&mut out)?;
        Ok(out)
    }

    fn physical(&self) -> Result<Vec<f64>> {
        let mut out = Vec::new();
        self.physical_into(&mut out)?;
        Ok(out)
    }

    fn unsigned(&self) -> Result<Option<UnsignedData>> {
        let mut out = UnsignedData::empty(UnsignedKind::I8);
        Ok(self.unsigned_into(&mut out)?.then_some(out))
    }

    fn vla(&self) -> Result<Ragged<ColumnData>> {
        let mut out = Ragged::<ColumnData>::new(ColumnData::U8(Vec::new()), Vec::new());
        self.vla_into(&mut out)?;
        Ok(out)
    }

    fn vla_physical(&self) -> Result<Ragged<Vec<f64>>> {
        let mut out = Ragged::<Vec<f64>>::new(Vec::new(), Vec::new());
        self.vla_physical_into(&mut out)?;
        Ok(out)
    }
}

/// An ASCII-table column read whole into new buffers.
pub trait WholeAsciiColumn {
    fn raw(&self) -> Result<AsciiColumnData>;
    fn physical(&self) -> Result<Vec<f64>>;
}

impl WholeAsciiColumn for AsciiColumnReader<'_> {
    fn raw(&self) -> Result<AsciiColumnData> {
        let mut out = AsciiColumnData::Text(AsciiText::default());
        self.raw_into(&mut out)?;
        Ok(out)
    }

    fn physical(&self) -> Result<Vec<f64>> {
        let mut out = Vec::new();
        self.physical_into(&mut out)?;
        Ok(out)
    }
}

/// An image's exact unsigned values read whole into a new buffer.
pub trait WholeView {
    fn unsigned(&self) -> Option<UnsignedData>;
}

impl WholeView for Borrowed<'_> {
    fn unsigned(&self) -> Option<UnsignedData> {
        let mut out = UnsignedData::empty(UnsignedKind::I8);
        self.unsigned_into(&mut out).then_some(out)
    }
}
