//! Spelling the Table-22 keyword families for one alternate description.

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Result;
use crate::error::WcsConflict;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::world_coordinates::header_read;
use crate::world_coordinates::spectral_frame::SpectralFrame;
use crate::world_coordinates::table_wcs::keyword_spelling::KeywordSpelling;
use crate::world_coordinates::table_wcs::table_axis_keyword::TableAxisKeyword;
use crate::world_coordinates::table_wcs::table_matrix_keyword::TableMatrixKeyword;
use crate::world_coordinates::table_wcs::table_pole_keyword::TablePoleKeyword;

/// Builds the Table-22 keyword names for one (optionally alternate) WCS description.
/// Every family has a pixel-list and a vector-cell spelling, and several have a
/// second, shorter spelling used when an alternate suffix takes the last character —
/// so the names are built here rather than at each read site.
#[derive(Debug, Clone, Copy)]
pub(super) struct TableWcsResolver {
    alt: Alternate,
}

impl TableWcsResolver {
    pub(super) const fn new(alt: Alternate) -> Self {
        Self { alt }
    }

    /// The spelling the axis roots take: the short one for an alternate description, whose
    /// suffix takes the last character.
    const fn spelling(self) -> KeywordSpelling {
        if self.alt.is_alternate() {
            KeywordSpelling::Short
        } else {
            KeywordSpelling::Full
        }
    }

    /// The keyword of `keyword`'s family for the pixel-list `column`; `None` where no
    /// card can hold one: the family has no spelling in this description, or a wide
    /// column number makes the name longer than a keyword.
    pub(super) fn pixel_axis_key(
        self,
        keyword: TableAxisKeyword,
        column: usize,
    ) -> Option<Keyword> {
        let root = keyword.table_root(self.spelling())?;
        Keyword::fitting(format_args!("T{root}{column}{}", self.alt))
    }

    /// [`Self::pixel_axis_key`] for array axis `axis` of the vector-cell `column`.
    pub(super) fn vector_axis_key(
        self,
        keyword: TableAxisKeyword,
        axis: usize,
        column: usize,
    ) -> Option<Keyword> {
        let root = keyword.table_root(self.spelling())?;
        Keyword::fitting(format_args!("{axis}{root}{column}{}", self.alt))
    }

    /// The type keyword of the first array axis of the vector-cell `column`, which a
    /// cell with no axis reports missing. Errors with [`FitsError::InvalidKeyword`] for
    /// a column so wide that no card can hold the name.
    pub(super) fn first_vector_type(self, column: usize) -> Result<Keyword> {
        let root = TableAxisKeyword::Type
            .table_root(self.spelling())
            .expect("Table 22 spells the axis type in every description");
        Keyword::try_from(format_args!("1{root}{column}{}", self.alt))
    }

    fn pixel_matrix_key(
        self,
        keyword: TableMatrixKeyword,
        row_column: usize,
        input_column: usize,
        spelling: KeywordSpelling,
    ) -> Option<Keyword> {
        let root = keyword.pixel_root(spelling);
        Keyword::fitting(format_args!(
            "{root}{row_column}_{input_column}{}",
            self.alt
        ))
    }

    pub(super) fn pixel_matrix_real(
        self,
        header: &Header,
        keyword: TableMatrixKeyword,
        row_column: usize,
        input_column: usize,
    ) -> Result<Option<f64>> {
        let [long, short] = KeywordSpelling::BOTH
            .map(|spelling| self.pixel_matrix_key(keyword, row_column, input_column, spelling));
        header_read::first_real(header, long, short)
    }

    pub(super) fn vector_matrix_key(
        self,
        keyword: TableMatrixKeyword,
        row_axis: usize,
        input_axis: usize,
        column: usize,
    ) -> Option<Keyword> {
        let root = keyword.root();
        Keyword::fitting(format_args!(
            "{row_axis}{input_axis}{root}{column}{}",
            self.alt
        ))
    }

    fn pixel_parameter_key(
        self,
        column: usize,
        parameter: usize,
        spelling: KeywordSpelling,
    ) -> Option<Keyword> {
        let root = match spelling {
            KeywordSpelling::Full => "TPV",
            KeywordSpelling::Short => "TV",
        };
        Keyword::fitting(format_args!("{root}{column}_{parameter}{}", self.alt))
    }

    pub(super) fn pixel_parameter_real(
        self,
        header: &Header,
        column: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        let [long, short] = KeywordSpelling::BOTH
            .map(|spelling| self.pixel_parameter_key(column, parameter, spelling));
        header_read::first_real(header, long, short)
    }

    fn vector_parameter_key(
        self,
        axis: usize,
        column: usize,
        parameter: usize,
        spelling: KeywordSpelling,
    ) -> Option<Keyword> {
        let root = match spelling {
            KeywordSpelling::Full => "PV",
            KeywordSpelling::Short => "V",
        };
        Keyword::fitting(format_args!("{axis}{root}{column}_{parameter}{}", self.alt))
    }

    pub(super) fn vector_parameter_real(
        self,
        header: &Header,
        axis: usize,
        column: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        let [long, short] = KeywordSpelling::BOTH
            .map(|spelling| self.vector_parameter_key(axis, column, parameter, spelling));
        header_read::first_real(header, long, short)
    }

    fn vector_string_parameter_key(
        self,
        axis: usize,
        column: usize,
        parameter: usize,
        spelling: KeywordSpelling,
    ) -> Option<Keyword> {
        let root = match spelling {
            KeywordSpelling::Full => "PS",
            KeywordSpelling::Short => "S",
        };
        Keyword::fitting(format_args!("{axis}{root}{column}_{parameter}{}", self.alt))
    }

    pub(super) fn column_key(self, root: &str, column: usize) -> Option<Keyword> {
        Keyword::fitting(format_args!("{root}{column}{}", self.alt))
    }

    pub(super) fn pole_real(
        self,
        header: &Header,
        pole: TablePoleKeyword,
        column: usize,
    ) -> Result<Option<f64>> {
        header_read::real(header, self.column_key(pole.table_root(), column))
    }

    pub(super) fn vector_rank(self, header: &Header, column: usize) -> Result<Option<i64>> {
        header_read::integer(header, self.column_key("WCAX", column))
    }

    /// Whether any Table-22 vector keyword names array axis `axis` of `column` — the
    /// exact test the rank inference falls back on.
    pub(super) fn vector_axis_present(self, header: &Header, column: usize, axis: usize) -> bool {
        if TableAxisKeyword::ALL.into_iter().any(|keyword| {
            self.vector_axis_key(keyword, axis, column)
                .is_some_and(|key| header_read::value(header, key).is_some())
        }) {
            return true;
        }
        if (0..=99).any(|parameter| {
            KeywordSpelling::BOTH.into_iter().any(|spelling| {
                let real = self.vector_parameter_key(axis, column, parameter, spelling);
                let text = self.vector_string_parameter_key(axis, column, parameter, spelling);
                header_read::value(header, real).is_some()
                    || header_read::value(header, text).is_some()
            })
        }) {
            return true;
        }
        (1..=99).any(|other| {
            [TableMatrixKeyword::Pc, TableMatrixKeyword::Cd]
                .into_iter()
                .any(|keyword| {
                    let row = self.vector_matrix_key(keyword, axis, other, column);
                    let input = self.vector_matrix_key(keyword, other, axis, column);
                    header_read::value(header, row).is_some()
                        || header_read::value(header, input).is_some()
                })
        })
    }

    /// The spectral frame declared for `column`, read through the Table-22
    /// column-indexed roots and resolved by the image-header path.
    pub(super) fn spectral_frame(self, header: &Header, column: usize) -> Result<SpectralFrame> {
        let mut translated = Header::new();
        for (table_root, image_root) in [("RFRQ", keywords::RESTFRQ), ("RWAV", keywords::RESTWAV)] {
            if let Some(value) = header_read::real(header, self.column_key(table_root, column))? {
                translated.set_internal(image_root, value);
            }
        }
        for (table_root, image_root) in [("SPEC", keywords::SPECSYS), ("SOBS", keywords::SSYSOBS)] {
            if let Some(value) = header_read::text(header, self.column_key(table_root, column))? {
                translated.set_internal(image_root, value);
            }
        }
        SpectralFrame::from_header(&translated, Alternate::PRIMARY)
    }

    /// Copy the `RADEna`/`EQUIna` celestial-frame keywords of every column in
    /// `columns` into `destination` under their image spellings, rejecting columns
    /// that disagree.
    pub(super) fn copy_celestial_frame(
        self,
        source: &Header,
        destination: &mut Header,
        columns: &[usize],
    ) -> Result<()> {
        for &column in columns {
            if let Some(value) = header_read::text(source, self.column_key("RADE", column))? {
                if let Some(existing) = destination.get_text(keywords::RADESYS)?
                    && existing != value
                {
                    return Err(FitsError::ConflictingWcsKeywords {
                        conflict: WcsConflict::TableRadesys,
                    });
                }
                destination.set_internal(keywords::RADESYS, value);
            }
            if let Some(value) = header_read::real(source, self.column_key("EQUI", column))? {
                if let Some(existing) = destination.get_real(keywords::EQUINOX)?
                    && existing != value
                {
                    return Err(FitsError::ConflictingWcsKeywords {
                        conflict: WcsConflict::TableEquinox,
                    });
                }
                destination.set_internal(keywords::EQUINOX, value);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
