//! Binary-table WCS (§8.5, Table 22) rewritten as the equivalent image header.
//!
//! A table header addresses its coordinate axes in one of two forms — a *pixel list*
//! gives each axis its own column (`TCTYPn`), a *vector cell* holds every axis inside
//! one column indexed by array axis (`iCTYPn`) — and spells each keyword family
//! differently again for alternate descriptions. [`TableWcs`] resolves the form and
//! the spelling, then translates the whole description into the image keywords the
//! rest of the WCS pipeline already evaluates.

pub(super) mod table_axis_keyword;
pub(super) mod table_matrix_keyword;
pub(super) mod table_pole_keyword;
pub(super) mod table_wcs_resolver;

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::KeyBuf;
use crate::keyword::key;
use crate::wcs::axis;
use crate::wcs::spectral_frame::SpectralFrame;
use crate::wcs::table_wcs::table_axis_keyword::TableAxisKeyword;
use crate::wcs::table_wcs::table_matrix_keyword::TableMatrixKeyword;
use crate::wcs::table_wcs::table_pole_keyword::TablePoleKeyword;
use crate::wcs::table_wcs::table_wcs_resolver::TableWcsResolver;
use crate::wcs::validated_axis_count;

/// A Table 22 WCS description: how its axes are addressed, plus the resolver that
/// spells each keyword family for the selected alternate.
#[derive(Debug, Clone, Copy)]
pub(super) struct TableWcs<'a> {
    resolver: TableWcsResolver,
    form: TableWcsForm<'a>,
}

/// How a binary-table header addresses the axes of one WCS (Table 22): a pixel list
/// gives each coordinate axis its own column (`TCTYPn`), while an array cell holds
/// every axis inside one column, indexed by array axis (`iCTYPn`).
#[derive(Debug, Clone, Copy)]
enum TableWcsForm<'a> {
    PixelList(&'a [usize]),
    ArrayColumn { naxis: usize, column: usize },
}

/// A Table 22 description rewritten as the equivalent image header. The spectral
/// frames travel alongside rather than inside it: they are column-indexed, so they
/// have no image-keyword spelling.
#[derive(Debug)]
pub(super) struct TranslatedTableWcs {
    pub(super) header: Header,
    pub(super) spectral_frames: Vec<Option<SpectralFrame>>,
}

impl<'a> TableWcs<'a> {
    /// A pixel-list description over the 1-based table `columns`, in axis order.
    pub(super) fn pixel_list(alt: Option<char>, columns: &'a [usize]) -> TableWcs<'a> {
        TableWcs {
            resolver: TableWcsResolver::new(alt),
            form: TableWcsForm::PixelList(columns),
        }
    }

    /// A vector-cell description of rank `naxis` inside the 1-based table `column`.
    pub(super) fn array_column(alt: Option<char>, naxis: usize, column: usize) -> TableWcs<'a> {
        TableWcs {
            resolver: TableWcsResolver::new(alt),
            form: TableWcsForm::ArrayColumn { naxis, column },
        }
    }

    /// The rank of a vector-cell WCS: `WCAXna` when declared, else the highest array
    /// axis any Table-22 vector keyword names for that column.
    pub(super) fn array_column_rank(
        header: &Header,
        alt: Option<char>,
        column: usize,
    ) -> Result<usize> {
        let resolver = TableWcsResolver::new(alt);
        let naxis = match resolver.vector_rank(header, column)? {
            Some(value) => validated_axis_count(value, "WCAXn")?,
            // Highest-first over the axes the cards could name, so the first hit is
            // the rank. Probing a blind `(1..=99).rev()` instead costs ~800 header
            // lookups per axis that turns out to be absent.
            None => candidate_vector_axes(header)
                .into_iter()
                .find(|&axis| resolver.vector_axis_present(header, column, axis))
                .unwrap_or(0),
        };
        if naxis == 0 {
            return Err(FitsError::MissingKeyword { name: "iCTYPn" });
        }
        Ok(naxis)
    }

    fn naxis(&self) -> usize {
        match self.form {
            TableWcsForm::PixelList(columns) => columns.len(),
            TableWcsForm::ArrayColumn { naxis, .. } => naxis,
        }
    }

    /// The table column carrying zero-based axis `index` — the pole, spectral, and
    /// celestial-frame keywords are column-indexed in both forms.
    fn column(&self, index: usize) -> usize {
        match self.form {
            TableWcsForm::PixelList(columns) => columns[index],
            TableWcsForm::ArrayColumn { column, .. } => column,
        }
    }

    fn axis_key(&self, keyword: TableAxisKeyword, index: usize) -> Option<KeyBuf> {
        match self.form {
            TableWcsForm::PixelList(columns) => {
                self.resolver.pixel_axis_key(keyword, columns[index])
            }
            TableWcsForm::ArrayColumn { column, .. } => {
                self.resolver.vector_axis_key(keyword, index + 1, column)
            }
        }
    }

    fn parameter_real(
        &self,
        header: &Header,
        index: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        match self.form {
            TableWcsForm::PixelList(columns) => {
                self.resolver
                    .pixel_parameter_real(header, columns[index], parameter)
            }
            TableWcsForm::ArrayColumn { column, .. } => {
                self.resolver
                    .vector_parameter_real(header, index + 1, column, parameter)
            }
        }
    }

    fn matrix_real(
        &self,
        header: &Header,
        keyword: TableMatrixKeyword,
        row: usize,
        input: usize,
    ) -> Result<Option<f64>> {
        match self.form {
            TableWcsForm::PixelList(columns) => {
                self.resolver
                    .pixel_matrix_real(header, keyword, columns[row], columns[input])
            }
            TableWcsForm::ArrayColumn { column, .. } => {
                let source = self
                    .resolver
                    .vector_matrix_key(keyword, row + 1, input + 1, column);
                header.get_real(source.as_str())
            }
        }
    }

    /// Rewrite the Table 22 keywords as the equivalent image header — axis type and
    /// unit, reference point/value/increment/rotation, `PVi_m`, and the linear
    /// transform — so both table forms evaluate through the same pipeline as an image
    /// WCS. The celestial pole and frame keywords are left to
    /// [`TableWcs::copy_celestial_keywords`]: the two forms choose their source column
    /// differently.
    pub(super) fn translate(&self, header: &Header) -> Result<TranslatedTableWcs> {
        let naxis = self.naxis();
        let mut h = Header::new();
        h.set_internal("WCSAXES", naxis as i64);
        let mut spectral_frames = vec![None; naxis];
        for (index, spectral) in spectral_frames.iter_mut().enumerate() {
            let ax = index + 1;
            let type_key = self
                .axis_key(TableAxisKeyword::Type, index)
                .expect("Table 22 defines primary and alternate axis-type keywords");
            if let Some(t) = header.get_text(type_key.as_str())? {
                h.set_internal(key!("CTYPE{ax}").as_str(), t);
                if axis::is_spectral_type(t) {
                    *spectral = Some(self.resolver.spectral_frame(header, self.column(index))?);
                }
            }
            let unit_key = self
                .axis_key(TableAxisKeyword::Unit, index)
                .expect("Table 22 defines primary and alternate axis-unit keywords");
            if let Some(t) = header.get_text(unit_key.as_str())? {
                h.set_internal(key!("CUNIT{ax}").as_str(), t);
            }
            for keyword in [
                TableAxisKeyword::ReferencePoint,
                TableAxisKeyword::ReferenceValue,
                TableAxisKeyword::Increment,
                TableAxisKeyword::Rotation,
            ] {
                if let Some(source) = self.axis_key(keyword, index)
                    && let Some(value) = header.get_real(source.as_str())?
                {
                    h.set_internal(key!("{}{ax}", keyword.image_root()).as_str(), value);
                }
            }
            // `PVi_m` arrives as `TPVn_ma`/`TVn_ma`, or `iPVn_ma`/`iVn_ma`.
            for m in 0..=20 {
                if let Some(v) = self.parameter_real(header, index, m)? {
                    h.set_internal(key!("PV{ax}_{m}").as_str(), v);
                }
            }
        }
        // Linear transform: `TPCn_ka`/`TCDn_ka` by column pair, or `ijPCna`/`ijCDna`
        // by axis pair.
        for row in 0..naxis {
            for input in 0..naxis {
                for keyword in [TableMatrixKeyword::Pc, TableMatrixKeyword::Cd] {
                    if let Some(value) = self.matrix_real(header, keyword, row, input)? {
                        h.set_internal(
                            key!("{}{}_{}", keyword.root(), row + 1, input + 1).as_str(),
                            value,
                        );
                    }
                }
            }
        }
        Ok(TranslatedTableWcs {
            header: h,
            spectral_frames,
        })
    }

    /// Translate the celestial pole and reference-frame keywords of the celestial
    /// `columns` into `destination`. The pole is the first column's — a pixel list
    /// passes its longitude column first, a vector cell its single column — while the
    /// frame must agree across all of them.
    pub(super) fn copy_celestial_keywords(
        &self,
        header: &Header,
        destination: &mut Header,
        columns: &[usize],
    ) -> Result<()> {
        for pole in TablePoleKeyword::BOTH {
            if let Some(value) = self.resolver.pole_real(header, pole, columns[0])? {
                destination.set_internal(pole.image_root(), value);
            }
        }
        self.resolver
            .copy_celestial_frame(header, destination, columns)
    }
}

/// Every array axis a Table-22 vector keyword could name, gathered in one pass over
/// the header and returned highest-first.
///
/// Each vector family spells its array axis in the keyword's *leading digits* —
/// `iCTYPn`, `iPVn_ma`, and for the matrix forms `ijPCna`/`ijCDna` a pair of them —
/// so an axis that is genuinely present always appears as a contiguous digit
/// substring of some card's leading run. Taking every such substring deliberately
/// over-approximates: [`TableWcsResolver::vector_axis_present`] stays the exact
/// test, and this only spares it the axes no card could possibly mention.
///
/// The alternative — parsing each family's grammar in reverse — has to disentangle
/// `ijPCna` from `iCTYPn` and the abbreviated alternate roots from the primary ones
/// they prefix, and a mistake there silently changes an image's rank. Over-
/// approximating cannot: a spurious candidate is rejected, and a missed one is
/// impossible by the substring argument above.
fn candidate_vector_axes(header: &Header) -> Vec<usize> {
    let mut axes: Vec<usize> = Vec::new();
    for entry in header.iter() {
        let digits = entry
            .keyword
            .find(|c: char| !c.is_ascii_digit())
            .map_or(entry.keyword, |end| &entry.keyword[..end]);
        for start in 0..digits.len() {
            for end in start + 1..=digits.len() {
                let text = &digits[start..end];
                // A FITS index never carries a leading zero, so `03` is not axis 3.
                if text.starts_with('0') {
                    continue;
                }
                match text.parse::<usize>() {
                    Ok(axis) if (1..=99).contains(&axis) && !axes.contains(&axis) => {
                        axes.push(axis);
                    }
                    _ => {}
                }
            }
        }
    }
    axes.sort_unstable_by(|a, b| b.cmp(a));
    axes
}

#[cfg(test)]
mod tests;
