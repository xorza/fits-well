//! One header and data unit as the reader's scan found it.

use crate::bintable::table_schema::TableSchema;
use crate::data::ImageMetadata;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;

/// One Header/Data Unit located by the reader.
///
/// The data unit itself is read lazily via [`FitsReader::read_data_raw`]; this
/// record carries the parsed header, the inferred [`HduKind`], the data unit's byte
/// range within the source, and the image geometry or table schema the header
/// declares, resolved once when the file is opened.
///
/// [`FitsReader::read_data_raw`]: crate::FitsReader::read_data_raw
#[derive(Debug)]
pub struct Hdu {
    pub header: Header,
    pub kind: HduKind,
    /// One's-complement sum of the exact block-padded header bytes as read.
    pub(crate) header_sum: u32,
    /// Byte offset of the data unit from the start of the source.
    pub(crate) data_offset: u64,
    /// Unpadded data length (`Nbits / 8`) — where the meaningful data ends within
    /// the padded unit. The on-disk length rounds it up to the block grid.
    pub data_bytes: u64,
    /// `None` when the HDU is not an image, or its header does not resolve to one;
    /// [`Hdu::image_geometry`] then reports why.
    image: Option<ImageGeometry>,
    /// `None` when the HDU is not a binary table, or its header does not resolve to
    /// one; [`Hdu::table_schema`] then reports why.
    table: Option<TableSchema>,
}

impl Hdu {
    /// Resolves the image geometry and table schema `header` declares. A header that
    /// does not resolve leaves its HDU readable for what it does hold — the rest of
    /// the file, and the header itself.
    pub(crate) fn new(
        header: Header,
        kind: HduKind,
        header_sum: u32,
        data_offset: u64,
        data_bytes: u64,
    ) -> Hdu {
        Hdu {
            image: ImageGeometry::from_header(&header, kind).ok(),
            table: table_schema(&header, kind).ok(),
            header,
            kind,
            header_sum,
            data_offset,
            data_bytes,
        }
    }

    /// Whether this HDU holds an image array: an image extension, a tiled-compressed
    /// image, or a primary array with at least one axis (`NAXIS = 0` makes the
    /// primary a container). A primary whose `NAXIS` does not parse is not one.
    pub fn is_image(&self) -> bool {
        match self.kind {
            HduKind::Image | HduKind::CompressedImage => true,
            HduKind::Primary => self.header.naxis().is_ok_and(|naxis| naxis > 0),
            HduKind::AsciiTable
            | HduKind::BinTable
            | HduKind::CompressedTable
            | HduKind::RandomGroups
            | HduKind::Other => false,
        }
    }

    /// Whether this HDU's `EXTNAME` matches `name` case-insensitively, with `version`
    /// and `level` (`EXTVER`/`EXTLEVEL`, each 1 where the card is absent, §4.4.1)
    /// narrowing the match when given. `EXTVER`/`EXTLEVEL` are read only once the
    /// name matches, so a malformed version card on an unrelated HDU cannot fail a
    /// lookup.
    pub fn matches_extension(
        &self,
        name: &str,
        version: Option<i64>,
        level: Option<i64>,
    ) -> Result<bool> {
        let name_matches = self
            .header
            .get_text("EXTNAME")?
            .is_some_and(|value| value.eq_ignore_ascii_case(name));
        if !name_matches {
            return Ok(false);
        }
        if let Some(version) = version
            && self.header.get_integer("EXTVER")?.unwrap_or(1) != version
        {
            return Ok(false);
        }
        if let Some(level) = level
            && self.header.get_integer("EXTLEVEL")?.unwrap_or(1) != level
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// The shape, stored element type and scaling of the image this HDU holds — for a
    /// tiled-compressed image, those of the image it encodes. Errors with
    /// [`FitsError::NotAnImage`] for any other kind, with
    /// [`FitsError::ImageHasGroups`] for a plain image with group structure, and with
    /// the header's own error when its image keywords are malformed.
    pub fn image(&self) -> Result<ImageMetadata<'_>> {
        self.image_geometry().map(ImageGeometry::metadata)
    }

    pub(crate) fn image_geometry(&self) -> Result<&ImageGeometry> {
        match &self.image {
            Some(geometry) => Ok(geometry),
            None => Err(ImageGeometry::from_header(&self.header, self.kind)
                .expect_err("the scan could not resolve this header")),
        }
    }

    /// A plain image's geometry: [`Hdu::image_geometry`], refusing a tiled-compressed
    /// image with [`FitsError::NotAnImage`].
    pub(crate) fn plain_image_geometry(&self) -> Result<&ImageGeometry> {
        if self.kind == HduKind::CompressedImage {
            return Err(FitsError::NotAnImage);
        }
        self.image_geometry()
    }

    /// The binary-table schema of this HDU — for a tiled-compressed image or table,
    /// that of its `BINTABLE` container. Errors with [`FitsError::NotABinTable`] for
    /// any other kind, and with the header's own error when its table keywords are
    /// malformed.
    pub fn table_schema(&self) -> Result<&TableSchema> {
        match &self.table {
            Some(schema) => Ok(schema),
            None => Err(table_schema(&self.header, self.kind)
                .expect_err("the scan could not resolve this header")),
        }
    }
}

/// The schema a binary-table path reads an HDU of `kind` with. A tiled-compressed
/// image or table is structurally a `BINTABLE`, and the compression layer reads its
/// container through the same path.
fn table_schema(header: &Header, kind: HduKind) -> Result<TableSchema> {
    match kind {
        HduKind::BinTable | HduKind::CompressedImage => TableSchema::parse(header),
        HduKind::CompressedTable => TableSchema::parse_container(header),
        HduKind::Primary
        | HduKind::Image
        | HduKind::AsciiTable
        | HduKind::RandomGroups
        | HduKind::Other => Err(FitsError::NotABinTable),
    }
}
