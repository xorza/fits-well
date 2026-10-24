//! One header and data unit as the reader's scan found it.

use crate::ascii::AsciiSchema;
use crate::bintable::schema::Schema;
use crate::block::{BLOCK_SIZE, padded_len};
#[cfg(feature = "compression")]
use crate::compress::table::TiledTable;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::hdu::HduPosition;
use crate::hdu::HduRole;
use crate::hdu::data_extent;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::{Header, keywords};
use crate::image_model::Metadata;
use crate::reader::hdu_selector::HduSelector;

/// One Header/Data Unit located by the reader.
///
/// The data unit itself is read lazily via [`FitsReader::read_data_unit`]; this
/// record carries the parsed header, the inferred [`HduKind`], the data unit's byte
/// range within the source, and the image geometry or table schema the header
/// declares, resolved once when the file is opened.
///
/// [`FitsReader::read_data_unit`]: crate::FitsReader::read_data_unit
#[derive(Debug)]
pub struct Hdu {
    /// The HDU's 0-based position in the file.
    pub index: usize,
    pub header: Header,
    pub kind: HduKind,
    /// One's-complement sum of the exact block-padded header bytes as read.
    pub(crate) header_sum: u32,
    /// Byte offset of the header from the start of the source; the header fills the
    /// whole blocks up to `data_offset`.
    pub(crate) header_offset: u64,
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
    /// `None` when the HDU is not an ASCII table, or its header does not resolve to
    /// one; [`Hdu::ascii_schema`] then reports why.
    ascii: Option<AsciiSchema>,
    /// `None` when the HDU is not a tiled-compressed table, or its header does not
    /// resolve to one; [`Hdu::tiled_table`] then reports why.
    #[cfg(feature = "compression")]
    tiled_table: Option<TiledTable>,
}

/// A header unit as a scan reads it: its bytes, their checksum sum, and the file offset
/// it starts at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeaderUnit<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) sum: u32,
    pub(crate) offset: u64,
}

impl Hdu {
    /// The HDU at `index` whose header unit is `unit`; its data unit follows the header.
    /// Resolves the image geometry and table schema the header declares. A header that
    /// does not resolve leaves its HDU readable for what it does hold — the rest of the
    /// file, and the header itself.
    pub(crate) fn parse(index: usize, unit: HeaderUnit<'_>) -> Result<Self> {
        let HeaderUnit {
            bytes,
            sum: header_sum,
            offset: header_offset,
        } = unit;
        let position = HduPosition::of_index(index);
        let header = Header::parse(bytes)?;
        let role = HduRole::from_header(&header, position)?;
        let kind = HduKind::classify(&header, role)?;
        let extent = data_extent(&header, role)?;
        let table = table_schema(&header, kind).ok();
        Ok(Self {
            index,
            image: ImageGeometry::from_header(&header, kind).ok(),
            #[cfg(feature = "compression")]
            tiled_table: match (&table, kind) {
                (Some(_), HduKind::CompressedTable) => TiledTable::new(&header).ok(),
                _ => None,
            },
            table,
            ascii: if kind == HduKind::AsciiTable {
                AsciiSchema::parse(&header).ok()
            } else {
                None
            },
            header,
            kind,
            header_sum,
            header_offset,
            data_offset: header_offset + bytes.len() as u64,
            data_bytes: extent.data_bytes,
        })
    }

    /// The number of 2880-byte blocks the header fills.
    pub(crate) const fn header_blocks(&self) -> u64 {
        (self.data_offset - self.header_offset) / BLOCK_SIZE as u64
    }

    /// Whether this HDU holds an image array of at least one axis: a primary array, an
    /// image extension or a tiled-compressed image whose `NAXIS` (`ZNAXIS` for a
    /// compressed image) is above 0. `NAXIS = 0` holds no array, whatever the kind; a
    /// rank that does not parse is none either. The other image keywords are not
    /// judged: [`Hdu::image_metadata`] reports why they do not resolve.
    pub fn is_image(&self) -> bool {
        let rank = match self.kind {
            HduKind::Primary | HduKind::Image => self.header.get_integer(keywords::NAXIS),
            #[cfg(feature = "compression")]
            HduKind::CompressedImage => self.header.get_integer(keywords::ZNAXIS),
            #[cfg(not(feature = "compression"))]
            HduKind::CompressedImage => return false,
            HduKind::AsciiTable
            | HduKind::BinTable
            | HduKind::CompressedTable
            | HduKind::RandomGroups
            | HduKind::Other => return false,
        };
        rank.is_ok_and(|rank| rank.is_some_and(|rank| rank > 0))
    }

    /// Whether `selector` names this HDU, as [`HduSelector`] states. `EXTVER` and
    /// `EXTLEVEL` are read only once the name matches, so a malformed version card on
    /// an unrelated HDU cannot fail a lookup.
    pub fn matches<'a>(&self, selector: impl Into<HduSelector<'a>>) -> Result<bool> {
        self.matches_selector(selector.into())
    }

    fn matches_selector(&self, selector: HduSelector<'_>) -> Result<bool> {
        let (name, version, level) = match selector {
            HduSelector::Index(index) => return Ok(index == self.index),
            HduSelector::Extension {
                name,
                version,
                level,
            } => (name, version, level),
        };
        // An `EXTNAME` that is no text names nothing, so a lookup passes over its HDU
        // rather than failing for a name it does not ask for.
        let own_name = match self.extname() {
            Ok(Some(own_name)) => own_name,
            Ok(None) if self.index == 0 => "PRIMARY",
            Ok(None) | Err(_) => return Ok(false),
        };
        if !own_name.eq_ignore_ascii_case(name) {
            return Ok(false);
        }
        if let Some(version) = version
            && self.extver()? != version
        {
            return Ok(false);
        }
        if let Some(level) = level
            && self.extlevel()? != level
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// The `EXTNAME` of this HDU (§4.4.2.6), `None` when absent. Errors when it is no
    /// text, which no name lookup then matches.
    pub fn extname(&self) -> Result<Option<&str>> {
        self.header.get_text(keywords::EXTNAME)
    }

    /// The `EXTVER` of this HDU, 1 when absent (§4.4.2.6). Errors when it is no integer.
    pub fn extver(&self) -> Result<i64> {
        Ok(self.header.get_integer(keywords::EXTVER)?.unwrap_or(1))
    }

    /// The `EXTLEVEL` of this HDU, 1 when absent (§4.4.2.6). Errors when it is no
    /// integer.
    pub fn extlevel(&self) -> Result<i64> {
        Ok(self.header.get_integer(keywords::EXTLEVEL)?.unwrap_or(1))
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
    pub fn image_metadata(&self) -> Result<Metadata<'_>> {
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

    /// The schema of the binary table this HDU holds: a plain `BINTABLE`'s, or the table a
    /// tiled-compressed one restores to. Errors with [`FitsError::NotABinTable`] for any
    /// other kind — a tiled-compressed image among them, and a compressed table without
    /// the `compression` feature — and with the header's own error when its table
    /// keywords are malformed.
    pub fn table_schema(&self) -> Result<&Schema> {
        match self.kind {
            HduKind::BinTable => self.container_schema(),
            #[cfg(feature = "compression")]
            HduKind::CompressedTable => Ok(&self
                .tiled_table()?
                .expect("a compressed table has a tile layout")
                .schema),
            #[cfg(not(feature = "compression"))]
            HduKind::CompressedTable => Err(FitsError::NotABinTable),
            HduKind::Primary
            | HduKind::Image
            | HduKind::AsciiTable
            | HduKind::CompressedImage
            | HduKind::RandomGroups
            | HduKind::Other => Err(FitsError::NotABinTable),
        }
    }

    /// The structure of an ASCII `TABLE`, parsed from its header when the file was
    /// opened. Errors with [`FitsError::NotAnAsciiTable`] for any other HDU, and with
    /// the header's own error when its table keywords are malformed.
    pub fn ascii_schema(&self) -> Result<&AsciiSchema> {
        if self.kind != HduKind::AsciiTable {
            return Err(FitsError::NotAnAsciiTable);
        }
        match &self.ascii {
            Some(schema) => Ok(schema),
            None => Err(AsciiSchema::parse(&self.header)
                .expect_err("the scan could not resolve this header")),
        }
    }

    /// The tile layout of a tiled-compressed table; `None` for any other HDU.
    #[cfg(feature = "compression")]
    pub(crate) fn tiled_table(&self) -> Result<Option<&TiledTable>> {
        if self.kind != HduKind::CompressedTable {
            return Ok(None);
        }
        if let Some(tiled) = &self.tiled_table {
            return Ok(Some(tiled));
        }
        self.container_schema()?;
        Err(TiledTable::new(&self.header).expect_err("the scan could not resolve this header"))
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
