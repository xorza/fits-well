//! Spelling the Table-22 keyword families for one alternate description.

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::AltSuffix;
use crate::keyword::KeyBuf;
use crate::keyword::key;
use crate::wcs::first_real;
use crate::wcs::spectral_frame::SpectralFrame;
use crate::wcs::table_wcs::table_axis_keyword::TableAxisKeyword;
use crate::wcs::table_wcs::table_matrix_keyword::TableMatrixKeyword;
use crate::wcs::table_wcs::table_pole_keyword::TablePoleKeyword;

/// Builds the Table-22 keyword names for one (optionally alternate) WCS description.
/// Every family has a pixel-list and a vector-cell spelling, and several have a
/// second, shorter spelling used when an alternate suffix takes the last character —
/// so the names are built here rather than at each read site.
#[derive(Debug, Clone, Copy)]
pub(super) struct TableWcsResolver {
    suffix: AltSuffix,
}

impl TableWcsResolver {
    pub(super) fn new(alternate: Option<char>) -> TableWcsResolver {
        TableWcsResolver {
            suffix: AltSuffix::new(alternate),
        }
    }

    pub(super) fn pixel_axis_key(self, keyword: TableAxisKeyword, column: usize) -> Option<KeyBuf> {
        let root = keyword.table_root(self.suffix.is_alternate())?;
        let suffix = self.suffix;
        Some(key!("T{root}{column}{suffix}"))
    }

    pub(super) fn vector_axis_key(
        self,
        keyword: TableAxisKeyword,
        axis: usize,
        column: usize,
    ) -> Option<KeyBuf> {
        let root = keyword.table_root(self.suffix.is_alternate())?;
        let suffix = self.suffix;
        Some(key!("{axis}{root}{column}{suffix}"))
    }

    fn pixel_matrix_key(
        self,
        keyword: TableMatrixKeyword,
        row_column: usize,
        input_column: usize,
        abbreviated: bool,
    ) -> KeyBuf {
        let root = keyword.pixel_root(abbreviated);
        let suffix = self.suffix;
        key!("{root}{row_column}_{input_column}{suffix}")
    }

    pub(super) fn pixel_matrix_real(
        self,
        header: &Header,
        keyword: TableMatrixKeyword,
        row_column: usize,
        input_column: usize,
    ) -> Result<Option<f64>> {
        let long = self.pixel_matrix_key(keyword, row_column, input_column, false);
        let short = self.pixel_matrix_key(keyword, row_column, input_column, true);
        first_real(header, long.as_str(), short.as_str())
    }

    pub(super) fn vector_matrix_key(
        self,
        keyword: TableMatrixKeyword,
        row_axis: usize,
        input_axis: usize,
        column: usize,
    ) -> KeyBuf {
        let root = keyword.root();
        let suffix = self.suffix;
        key!("{row_axis}{input_axis}{root}{column}{suffix}")
    }

    fn pixel_parameter_key(self, column: usize, parameter: usize, short: bool) -> KeyBuf {
        let root = if short { "TV" } else { "TPV" };
        let suffix = self.suffix;
        key!("{root}{column}_{parameter}{suffix}")
    }

    pub(super) fn pixel_parameter_real(
        self,
        header: &Header,
        column: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        let long = self.pixel_parameter_key(column, parameter, false);
        let short = self.pixel_parameter_key(column, parameter, true);
        first_real(header, long.as_str(), short.as_str())
    }

    fn vector_parameter_key(
        self,
        axis: usize,
        column: usize,
        parameter: usize,
        short: bool,
    ) -> KeyBuf {
        let root = if short { "V" } else { "PV" };
        let suffix = self.suffix;
        key!("{axis}{root}{column}_{parameter}{suffix}")
    }

    pub(super) fn vector_parameter_real(
        self,
        header: &Header,
        axis: usize,
        column: usize,
        parameter: usize,
    ) -> Result<Option<f64>> {
        let long = self.vector_parameter_key(axis, column, parameter, false);
        let short = self.vector_parameter_key(axis, column, parameter, true);
        first_real(header, long.as_str(), short.as_str())
    }

    fn vector_string_parameter_key(
        self,
        axis: usize,
        column: usize,
        parameter: usize,
        short: bool,
    ) -> KeyBuf {
        let root = if short { "S" } else { "PS" };
        let suffix = self.suffix;
        key!("{axis}{root}{column}_{parameter}{suffix}")
    }

    fn column_key(self, root: &str, column: usize) -> KeyBuf {
        let suffix = self.suffix;
        key!("{root}{column}{suffix}")
    }

    pub(super) fn pole_real(
        self,
        header: &Header,
        pole: TablePoleKeyword,
        column: usize,
    ) -> Result<Option<f64>> {
        let key = self.column_key(pole.table_root(), column);
        header.get_real(key.as_str())
    }

    pub(super) fn vector_rank(self, header: &Header, column: usize) -> Result<Option<i64>> {
        let key = self.column_key("WCAX", column);
        header.get_integer(key.as_str())
    }

    /// Whether any Table-22 vector keyword names array axis `axis` of `column` — the
    /// exact test the rank inference falls back on.
    pub(super) fn vector_axis_present(self, header: &Header, column: usize, axis: usize) -> bool {
        if TableAxisKeyword::ALL.into_iter().any(|keyword| {
            self.vector_axis_key(keyword, axis, column)
                .is_some_and(|key| header.get(key.as_str()).is_some())
        }) {
            return true;
        }
        if (0..=99).any(|parameter| {
            [false, true].into_iter().any(|short| {
                let real = self.vector_parameter_key(axis, column, parameter, short);
                let text = self.vector_string_parameter_key(axis, column, parameter, short);
                header.get(real.as_str()).is_some() || header.get(text.as_str()).is_some()
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
                    header.get(row.as_str()).is_some() || header.get(input.as_str()).is_some()
                })
        })
    }

    /// The spectral frame declared for `column`, read through the Table-22
    /// column-indexed roots and resolved by the image-header path.
    pub(super) fn spectral_frame(self, header: &Header, column: usize) -> Result<SpectralFrame> {
        let mut translated = Header::new();
        for (table_root, image_root) in [("RFRQ", "RESTFRQ"), ("RWAV", "RESTWAV")] {
            let source = self.column_key(table_root, column);
            if let Some(value) = header.get_real(source.as_str())? {
                translated.set_internal(image_root, value);
            }
        }
        for (table_root, image_root) in [("SPEC", "SPECSYS"), ("SOBS", "SSYSOBS")] {
            let source = self.column_key(table_root, column);
            if let Some(value) = header.get_text(source.as_str())? {
                translated.set_internal(image_root, value);
            }
        }
        SpectralFrame::from_header(&translated, None, "")
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
            let radesys = self.column_key("RADE", column);
            if let Some(value) = source.get_text(radesys.as_str())? {
                if let Some(existing) = destination.get_text("RADESYS")?
                    && existing != value
                {
                    return Err(FitsError::ConflictingWcsKeywords {
                        detail: "table celestial axes declare different RADESYS values",
                    });
                }
                destination.set_internal("RADESYS", value);
            }
            let equinox = self.column_key("EQUI", column);
            if let Some(value) = source.get_real(equinox.as_str())? {
                if let Some(existing) = destination.get_real("EQUINOX")?
                    && existing != value
                {
                    return Err(FitsError::ConflictingWcsKeywords {
                        detail: "table celestial axes declare different EQUINOX values",
                    });
                }
                destination.set_internal("EQUINOX", value);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_wcs_resolver_matches_table_22() {
        let primary = TableWcsResolver::new(None);
        let alternate = TableWcsResolver::new(Some('A'));
        let axis_cases = [
            (
                TableAxisKeyword::Type,
                "TCTYP17",
                "TCTY17A",
                "3CTYP17",
                "3CTY17A",
            ),
            (
                TableAxisKeyword::Unit,
                "TCUNI17",
                "TCUN17A",
                "3CUNI17",
                "3CUN17A",
            ),
            (
                TableAxisKeyword::ReferenceValue,
                "TCRVL17",
                "TCRV17A",
                "3CRVL17",
                "3CRV17A",
            ),
            (
                TableAxisKeyword::Increment,
                "TCDLT17",
                "TCDE17A",
                "3CDLT17",
                "3CDE17A",
            ),
            (
                TableAxisKeyword::ReferencePoint,
                "TCRPX17",
                "TCRP17A",
                "3CRPX17",
                "3CRP17A",
            ),
        ];
        for (keyword, primary_pixel, alternate_pixel, primary_vector, alternate_vector) in
            axis_cases
        {
            assert_eq!(
                primary.pixel_axis_key(keyword, 17).unwrap().as_str(),
                primary_pixel
            );
            assert_eq!(
                alternate.pixel_axis_key(keyword, 17).unwrap().as_str(),
                alternate_pixel
            );
            assert_eq!(
                primary.vector_axis_key(keyword, 3, 17).unwrap().as_str(),
                primary_vector
            );
            assert_eq!(
                alternate.vector_axis_key(keyword, 3, 17).unwrap().as_str(),
                alternate_vector
            );
        }
        assert_eq!(
            primary
                .pixel_axis_key(TableAxisKeyword::Rotation, 17)
                .unwrap()
                .as_str(),
            "TCROT17"
        );
        assert_eq!(
            primary
                .vector_axis_key(TableAxisKeyword::Rotation, 3, 17)
                .unwrap()
                .as_str(),
            "3CROT17"
        );
        assert!(
            alternate
                .pixel_axis_key(TableAxisKeyword::Rotation, 17)
                .is_none()
        );
        assert!(
            alternate
                .vector_axis_key(TableAxisKeyword::Rotation, 3, 17)
                .is_none()
        );

        assert_eq!(
            alternate
                .pixel_matrix_key(TableMatrixKeyword::Pc, 2, 3, false)
                .as_str(),
            "TPC2_3A"
        );
        assert_eq!(
            alternate
                .pixel_matrix_key(TableMatrixKeyword::Pc, 2, 3, true)
                .as_str(),
            "TP2_3A"
        );
        assert_eq!(
            alternate
                .pixel_matrix_key(TableMatrixKeyword::Cd, 2, 3, false)
                .as_str(),
            "TCD2_3A"
        );
        assert_eq!(
            alternate
                .pixel_matrix_key(TableMatrixKeyword::Cd, 2, 3, true)
                .as_str(),
            "TC2_3A"
        );
        assert_eq!(
            alternate
                .vector_matrix_key(TableMatrixKeyword::Pc, 2, 3, 17)
                .as_str(),
            "23PC17A"
        );
        assert_eq!(
            alternate.pixel_parameter_key(2, 1, false).as_str(),
            "TPV2_1A"
        );
        assert_eq!(alternate.pixel_parameter_key(2, 1, true).as_str(), "TV2_1A");
        assert_eq!(
            alternate.vector_parameter_key(2, 17, 1, false).as_str(),
            "2PV17_1A"
        );
        assert_eq!(
            alternate.vector_parameter_key(2, 17, 1, true).as_str(),
            "2V17_1A"
        );
        assert_eq!(alternate.column_key("LONP", 17).as_str(), "LONP17A");
        assert_eq!(alternate.column_key("LATP", 17).as_str(), "LATP17A");
        assert_eq!(alternate.column_key("WCAX", 17).as_str(), "WCAX17A");
    }
}
