//! One header and data unit as the reader's scan found it.

use crate::bintable::schema::Schema;
use crate::block::padded_len;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;
use crate::image_model::Metadata;

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
    /// `None` when the HDU is not structurally a binary table, or its header does not
    /// resolve to one; [`Hdu::container_schema`] then reports why.
    table: Option<Schema>,
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
    ) -> Self {
        Self {
            image: ImageGeometry::from_header(&header, kind).ok(),
            table: table_schema(&header, kind).ok(),
            header,
            kind,
            header_sum,
            data_offset,
            data_bytes,
        }
    }

    /// Whether this HDU holds an image array of at least one axis: a primary array, an
    /// image extension or a tiled-compressed image whose `NAXIS` (`ZNAXIS` for a
    /// compressed image) is above 0. `NAXIS = 0` holds no array, whatever the kind; a
    /// rank that does not parse is none either. The other image keywords are not
    /// judged: [`Hdu::image`] reports why they do not resolve.
    pub fn is_image(&self) -> bool {
        let rank = match self.kind {
            HduKind::Primary | HduKind::Image => self.header.get_integer("NAXIS"),
            HduKind::CompressedImage => self.header.get_integer("ZNAXIS"),
            HduKind::AsciiTable
            | HduKind::BinTable
            | HduKind::CompressedTable
            | HduKind::RandomGroups
            | HduKind::Other => return false,
        };
        rank.is_ok_and(|rank| rank.is_some_and(|rank| rank > 0))
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

    /// The data unit's length on disk: [`Hdu::data_bytes`](Hdu#structfield.data_bytes)
    /// rounded up to the 2880-byte block grid.
    pub const fn padded_data_bytes(&self) -> u64 {
        padded_len(self.data_bytes).expect("the scan bounded the padded data unit")
    }

    /// The shape, stored element type and scaling of the image this HDU holds — for a
    /// tiled-compressed image, those of the image it encodes. Errors with
    /// [`FitsError::NotAnImage`] for any other kind, with
    /// [`FitsError::ImageHasGroups`] for a plain image with group structure, and with
    /// the header's own error when its image keywords are malformed.
    pub fn image(&self) -> Result<Metadata<'_>> {
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

    /// The schema of the binary table this HDU holds. Errors with
    /// [`FitsError::TableIsCompressed`] for a tiled-compressed table, with
    /// [`FitsError::NotABinTable`] for any other kind but a plain `BINTABLE` — a
    /// tiled-compressed image among them — and with the header's own error when its
    /// table keywords are malformed.
    pub fn table_schema(&self) -> Result<&Schema> {
        match self.kind {
            HduKind::BinTable => self.container_schema(),
            HduKind::CompressedTable => Err(FitsError::TableIsCompressed),
            HduKind::Primary
            | HduKind::Image
            | HduKind::AsciiTable
            | HduKind::CompressedImage
            | HduKind::RandomGroups
            | HduKind::Other => Err(FitsError::NotABinTable),
        }
    }

    /// The schema of the `BINTABLE` this HDU is stored as: a plain table's own, or the
    /// container of a tiled-compressed image or table, which the compression layer
    /// reads through the binary-table path.
    pub(crate) fn container_schema(&self) -> Result<&Schema> {
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
fn table_schema(header: &Header, kind: HduKind) -> Result<Schema> {
    match kind {
        HduKind::BinTable | HduKind::CompressedImage => Schema::parse(header),
        HduKind::CompressedTable => Schema::parse_container(header),
        HduKind::Primary
        | HduKind::Image
        | HduKind::AsciiTable
        | HduKind::RandomGroups
        | HduKind::Other => Err(FitsError::NotABinTable),
    }
}
