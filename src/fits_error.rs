use std::convert::Infallible;
use std::fmt;
use std::io;

use std::result;
use std::sync::Arc;

use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::element_kind::ElementKind;
use crate::bitpix::Bitpix;
use crate::hdu::HduKind;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::defect::Defect;
use crate::header_model::record_name::RecordName;
use crate::keyword::Keyword;
use crate::world_coordinates::projection::Projection;
use thiserror::Error;

pub type Result<T> = result::Result<T, FitsError>;

/// What an out-of-range index was addressing, naming the bound it exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Indexed {
    /// An HDU within the file's scanned sequence.
    Hdu,
    /// A logical record within a header unit.
    HeaderRecord,
    /// A column within a table.
    Column,
    /// A group within a random-groups array.
    Group,
    /// A **1-based** axis within a WCS description.
    WcsAxis,
}

impl Indexed {
    /// What the index counts, in words.
    const fn name(self) -> &'static str {
        match self {
            Self::Hdu => "HDU",
            Self::HeaderRecord => "header record",
            Self::Column => "table column",
            Self::Group => "group",
            Self::WcsAxis => "WCS axis",
        }
    }

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "thiserror's `fmt` hands the variant and every field by reference"
    )]
    fn fmt_out_of_bounds(
        &self,
        index: &usize,
        len: &usize,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Hdu => write!(f, "HDU index {index} out of bounds (file has {len} HDUs)"),
            Self::HeaderRecord => write!(
                f,
                "header record index {index} out of bounds (header has {len} records)"
            ),
            Self::Column => write!(
                f,
                "column index {index} out of bounds (table has {len} columns)"
            ),
            Self::Group => write!(
                f,
                "group index {index} out of bounds (random-groups array has {len} groups)"
            ),
            Self::WcsAxis => write!(
                f,
                "1-based WCS axis {index} out of bounds (WCS has {len} axes)"
            ),
        }
    }
}

/// The type a header value was read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueKind {
    Logical,
    Integer,
    Real,
    Text,
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Logical => "a logical",
            Self::Integer => "an integer",
            Self::Real => "a real",
            Self::Text => "a text",
        })
    }
}

/// The type an exact FITS integer was converted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NumericTarget {
    I64,
    F64,
}

impl fmt::Display for NumericTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::I64 => "i64",
            Self::F64 => "f64",
        })
    }
}

/// Why a header value has no FITS representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum NonFinite {
    /// A real that is infinite or NaN.
    #[error("a real value must be finite")]
    Real,
    /// A complex real with an infinite or NaN part.
    #[error("the parts of a complex value must be finite")]
    Complex,
}

/// Where the text that held a byte outside FITS restricted ASCII came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextOrigin {
    /// The text value of a header record.
    Value(RecordName),
    /// The inline comment of a header record, or the text of a commentary record.
    Comment(RecordName),
    /// A table column's name (`TTYPEn`).
    ColumnName,
    /// A table column's unit (`TUNITn`).
    ColumnUnit,
    /// An ASCII-table column's null marker (`TNULLn`).
    NullMarker,
    /// An ASCII-table text cell.
    Cell,
    /// An ASCII table's data unit.
    TableData,
}

impl fmt::Display for TextOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(name) => write!(f, "the value of {name}"),
            Self::Comment(name) => write!(f, "the comment of {name}"),
            Self::ColumnName => f.write_str("a column name"),
            Self::ColumnUnit => f.write_str("a column unit"),
            Self::NullMarker => f.write_str("an ASCII null marker"),
            Self::Cell => f.write_str("an ASCII text cell"),
            Self::TableData => f.write_str("the ASCII-table data"),
        }
    }
}

#[expect(
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "thiserror's `fmt` hands every field by reference"
)]
fn fmt_invalid_value(
    name: &Option<RecordName>,
    defect: &Defect,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match name {
        Some(name) => write!(f, "header record {name} does not parse: {defect}"),
        None => write!(f, "a header record does not parse: {defect}"),
    }
}

/// Which of two axis counts disagreed — the thing the caller supplied, against the
/// structure it had to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ranked {
    /// An N-dimensional image region, against its image.
    ImageRegion,
    /// A compression tile shape, against the image it tiles.
    TileShape,
    /// A pixel or world coordinate, against its WCS.
    WcsCoordinate,
    /// The bin factors of a WCS, one per pixel axis, against its WCS.
    BinFactors,
}

impl Ranked {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "thiserror's `fmt` hands the variant and every field by reference"
    )]
    fn fmt_mismatch(
        &self,
        expected: &usize,
        got: &usize,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::ImageRegion => write!(
                f,
                "image region has {got} axes but the image has {expected}"
            ),
            Self::TileShape => {
                write!(f, "tile shape has {got} axes but the image has {expected}")
            }
            Self::WcsCoordinate => write!(
                f,
                "coordinate has {got} {} but the WCS has {expected} axes",
                if *got == 1 { "value" } else { "values" }
            ),
            Self::BinFactors => {
                write!(f, "{got} bin factors given but the WCS has {expected} axes")
            }
        }
    }
}

/// What a [`FitsError::DataSizeMismatch`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeUnit {
    Bytes,
    Samples,
    Rows,
}

impl fmt::Display for SizeUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bytes => "bytes",
            Self::Samples => "samples",
            Self::Rows => "rows",
        })
    }
}

impl SizeUnit {
    /// The [`FitsError::DataSizeMismatch`] of `got` of these where `expected` are
    /// declared.
    pub(crate) const fn mismatch(self, expected: usize, got: usize) -> FitsError {
        // A `usize` count always fits a `u64`.
        FitsError::DataSizeMismatch {
            unit: self,
            expected: expected as u64,
            got: got as u64,
        }
    }
}

/// A tiled-compression parameter outside its valid range, from
/// [`FitsError::InvalidCompressionParameter`].
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[non_exhaustive]
pub enum CompressionParameter {
    /// A deflate level past 9.
    #[error("gzip compression level {level} is outside 0..=9")]
    GzipLevel { level: u32 },
    /// An HCOMPRESS scale that is not finite and positive.
    #[error("HCOMPRESS scale {scale} must be finite and positive")]
    HcompressScale { scale: f64 },
    /// A relative quantization level that is not finite and positive.
    #[error("relative quantization level {level} must be finite and positive")]
    QuantizationLevel { level: f64 },
    /// An absolute quantization step that is not finite and positive.
    #[error("absolute quantization step {step} must be finite and positive")]
    QuantizationStep { step: f64 },
    /// A dither seed outside `1..=10000`.
    #[error("dither seed {seed} is outside 1..=10000")]
    DitherSeed { seed: u16 },
    /// A quantization for an integer image, which stores its values as they are.
    #[error("an integer image takes no float quantization")]
    IntegerQuantization,
    /// A quantization for a table, which no table codec quantizes.
    #[error("a compressed table takes no float quantization")]
    TableQuantization,
    /// `NOCOMPRESS` for a table: cfitsio cannot read `NOCOMPRESS` columns, and
    /// `write_table` writes a table uncompressed.
    #[error("a compressed table takes a codec")]
    TableWithoutCodec,
    /// A tile of 0 along `axis`, zero-based.
    #[error("tile axis {axis} is 0")]
    EmptyTile { axis: usize },
    /// `HCOMPRESS_1` with `SUBTRACTIVE_DITHER_2`, whose exact zeros it cannot keep.
    #[error("HCOMPRESS_1 cannot keep SUBTRACTIVE_DITHER_2's zeros")]
    HcompressDither2,
}

/// What a tiled-compression read or write does not support, from
/// [`FitsError::UnsupportedCompression`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CompressionFeature {
    /// A codec name this crate does not know.
    #[error("{name}")]
    Codec { name: String },
    /// A `ZQUANTIZ` method this crate does not know.
    #[error("float quantization {name}")]
    Quantization { name: String },
    /// `HCOMPRESS_1` of an image that is not two-dimensional.
    #[error("HCOMPRESS_1 requires a two-dimensional image")]
    HcompressDimensions,
    /// Lossy `HCOMPRESS_1` of a tile with undefined pixels, which only a null mask keeps.
    #[error("lossy HCOMPRESS_1 with undefined pixels requires a null mask")]
    LossyHcompressNulls,
    /// An `HCOMPRESS_1` null-pixel mask, which the lossy codec cannot encode.
    #[error("lossy HCOMPRESS_1 cannot encode a null-pixel mask")]
    HcompressNullMask,
    /// An `HCOMPRESS_1` tile scale past the 32-bit stream range.
    #[error("HCOMPRESS_1 tile scale exceeds the 32-bit stream range")]
    HcompressScale,
    /// An `HCOMPRESS_1` tile side past 2³¹ − 1.
    #[error("HCOMPRESS_1 tile dimension {length}, past 2^31 - 1")]
    HcompressTileSide { length: usize },
    /// An `HCOMPRESS_1` transform past the signed 64-bit stream range.
    #[error("HCOMPRESS_1 tile exceeds the signed 64-bit stream range")]
    HcompressRange,
    /// A `PLIO_1` line list of 2³⁰ words or more.
    #[error("PLIO_1 line list of {words} words, 2^30 or more")]
    PlioListLength { words: usize },
    /// A lossless float image with a codec other than the gzip codecs.
    #[error("{codec} for a lossless float image")]
    LosslessFloatCodec { codec: &'static str },
    /// A quantized float image written with `codec`.
    #[error("{codec} for float images (write)")]
    QuantizedFloatCodec { codec: &'static str },
    /// A table compressed with `codec`.
    #[error("{codec} for compressed tables")]
    TableCodec { codec: &'static str },
    /// A table column codec `ZCTYPn` that §10.3 does not allow.
    #[error("table column codec {name}")]
    ColumnCodec { name: String },
    /// `RICE_1` on a column of `TFORMn` type `code`, which holds no `B`, `I` or `J`
    /// value.
    #[error("RICE_1 on a {code} column")]
    RiceColumn { code: char },
}

/// Why a set of WCS keywords describes no valid transform (§8), from
/// [`FitsError::InvalidWcs`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WcsDefect {
    /// The linear matrix, from `PCi_j` and `CDELTi`, `CDi_j` or `CROTAi`, is singular.
    #[error("the transform matrix is singular")]
    SingularMatrix,
    /// A projection's `PVi_m` parameters admit no transform, as `CEA`'s λ = 0.
    #[error("degenerate {projection} projection parameters")]
    DegenerateProjection { projection: Projection },
    /// `RADESYSa` names no standard reference frame (§8.3).
    #[error("RADESYS {value:?} is not a standard reference frame")]
    UnknownCelestialFrame { value: String },
    /// `EQUINOXa` is not finite and non-negative.
    #[error("EQUINOX must be finite and non-negative")]
    InvalidEquinox,
    /// A spectral reference-frame keyword names no standard frame (§8.4).
    #[error("{keyword} {value:?} is not a standard spectral reference frame")]
    UnknownSpectralFrame { keyword: Keyword, value: String },
    /// The `CRVALi` of a logarithmic axis of type `ctype` is not finite and positive.
    #[error("{ctype} requires a finite, positive CRVAL")]
    LogarithmicReference { ctype: String },
    /// A spectral `X2P` code of `ctype` that does not express its coordinate type's own
    /// characteristic, or converts one to itself.
    #[error("spectral CTYPE {ctype:?} has inconsistent variables")]
    InconsistentSpectralAlgorithm { ctype: String },
    /// The reference value of a spectral axis of type `kind` lies outside its domain.
    #[error("{kind} has an invalid spectral reference value")]
    SpectralReference { kind: &'static str },
    /// `RESTFRQa` or `RESTWAVa` is not finite and positive.
    #[error("{keyword} must be finite and positive")]
    RestValue { keyword: Keyword },
    /// A spectral algorithm needs the rest frequency or wavelength, and neither is given.
    #[error("spectral CTYPE requires RESTFRQ or RESTWAV")]
    MissingRest,
    /// A grism axis of `algorithm`, `GRI` or `GRA`, whose parameters describe no grism.
    #[error("{algorithm} {defect}")]
    Grism {
        algorithm: &'static str,
        defect: GrismDefect,
    },
    /// A `-TAB` axis and its coordinate table.
    #[error("{0}")]
    Tabular(TabularDefect),
}

/// Why a grism axis's parameters describe no grism, from [`WcsDefect::Grism`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GrismDefect {
    /// `PVi_m` of index `parameter`, the groove density or the order, is absent, not
    /// finite, or 0.
    #[error("PV_i_{parameter} must be specified, finite, and non-zero")]
    MissingParameter { parameter: usize },
    /// A parameter is not finite.
    #[error("parameters must be finite")]
    NonFinite,
    /// The grating or the detector tilt has a zero cosine.
    #[error("grating and detector tilts must have non-zero cosine")]
    PerpendicularTilt,
    /// The reference wavelength has no diffraction angle.
    #[error("reference wavelength is outside the grism domain")]
    ReferenceOutsideDomain,
    /// The detector parameters give no dispersion.
    #[error("detector parameters produce zero dispersion")]
    ZeroDispersion,
}

/// Why a `-TAB` axis or its coordinate table describes no transform (§8.3.1), from
/// [`WcsDefect::Tabular`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TabularDefect {
    /// `PSi_0`, the table's extension name, is absent or empty.
    #[error("TAB requires PSi_0")]
    MissingExtension,
    /// `PSi_1`, the coordinate column, is absent or empty.
    #[error("TAB requires PSi_1")]
    MissingCoordinateColumn,
    /// `PVi_1`, `PVi_2` or `PVi_3` is not a positive integer.
    #[error("TAB PVi_1, PVi_2, and PVi_3 must be positive integers")]
    NonPositiveParameter,
    /// `PVi_3` names a table axis past the WCS axis count.
    #[error("PVi_3 exceeds the WCS axis count")]
    TableAxisPastCount,
    /// `CRVALi` of a `-TAB` axis is not finite.
    #[error("TAB CRVAL must be finite")]
    NonFiniteReference,
    /// Two axes map the same table axis.
    #[error("TAB table-axis mapping is duplicated")]
    DuplicateTableAxis,
    /// A table axis that no image axis maps.
    #[error("TAB table-axis mapping is incomplete")]
    IncompleteTableAxes,
    /// The coordinate column's `TDIMn` is not `(M,K1,...,KM)` for M table axes.
    #[error("TAB coordinate TDIM must be (M,K1,...,KM)")]
    CoordinateShape,
    /// A coordinate axis of length 0.
    #[error("TAB coordinate axes must be non-empty")]
    EmptyAxis,
    /// The coordinate array's element count overflows.
    #[error("TAB coordinate array is too large")]
    CoordinateArrayTooLarge,
    /// The coordinate column's first row holds another count than its `TDIMn`.
    #[error("TAB coordinate TDIM does not match its first-row element count")]
    CoordinateCount,
    /// A coordinate that is not finite.
    #[error("TAB coordinate array must contain finite values")]
    NonFiniteCoordinate,
    /// An index vector whose length is not its coordinate axis's.
    #[error("TAB index-vector length must match its coordinate axis")]
    IndexLength,
    /// An index value that is not finite.
    #[error("TAB index vectors must contain finite values")]
    NonFiniteIndex,
    /// An index vector that rises and falls.
    #[error("TAB index vectors must be monotonic")]
    NonMonotonicIndex,
    /// An index vector whose values are all equal.
    #[error("TAB index vectors must not be constant")]
    ConstantIndex,
    /// More varying table axes than the interpolation's vertex count allows.
    #[error("TAB interpolation dimensionality is too large")]
    TooManyAxes,
    /// The lookup table has no row.
    #[error("TAB lookup table must contain a row")]
    EmptyTable,
}

/// Which WCS keywords cannot hold together, from [`FitsError::ConflictingWcsKeywords`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WcsConflict {
    /// Both `PCi_j` and `CDi_j` (§8.1).
    #[error("PC and CD conventions overlap")]
    PcAndCd,
    /// Both `PCi_j` and `CROTAi` (§8.1).
    #[error("PC and CROTA conventions overlap")]
    PcAndCrota,
    /// The celestial longitude and latitude axes name different projections.
    #[error("celestial longitude and latitude axes declare different projections")]
    MismatchedProjections,
    /// SIP keywords for an alternate description, which they have no form for.
    #[error("SIP keywords have no alternate form")]
    SipAlternate,
    /// SIP on a WCS of fewer than two pixel axes.
    #[error("SIP needs two pixel axes")]
    SipAxisCount,
    /// The celestial columns of a table WCS declare different `RADESYS` values.
    #[error("table celestial axes declare different RADESYS values")]
    TableRadesys,
    /// The celestial columns of a table WCS declare different `EQUINOX` values.
    #[error("table celestial axes declare different EQUINOX values")]
    TableEquinox,
}

/// Why the fiducial point, `LONPOLE` and `LATPOLE` admit no celestial pole, from
/// [`FitsError::WcsInvalidPole`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PoleDefect {
    /// `|φp − φ0| = 90°` with `θ0 = 0` leaves `δ0 = 0` the only latitude.
    #[error("|φp − φ0| = 90° with θ0 = 0 needs δ0 = 0")]
    EquatorialFiducial,
    /// `|δ0|` is too large for `φp`, `φ0` and `θ0`.
    #[error("|δ0| is too large for this φp, φ0 and θ0")]
    FiducialLatitude,
    /// Neither latitude solution lies within ±90°.
    #[error("no pole latitude within ±90° fits φp, φ0 and θ0")]
    NoLatitude,
}

/// Why a compressed stream or tile does not decode, from
/// [`FitsError::CorruptCompressedData`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CompressedDataDefect {
    /// A gzip stream does not inflate: a bad header, block or checksum.
    #[error("a gzip stream does not inflate: {0}")]
    GzipStream(#[source] io::Error),
    /// A gzip stream inflates past the size of its tile.
    #[error("a gzip stream inflates past the size of its tile")]
    GzipOverrun,
    /// A Rice block declares a field wider than 64 bits.
    #[error("a Rice block declares a {bits}-bit field")]
    RiceBitField { bits: u32 },
    /// A PLIO line list does not decode.
    #[error("PLIO_1: {0}")]
    Plio(PlioDefect),
    /// An HCOMPRESS stream does not decode.
    #[error("HCOMPRESS_1: {0}")]
    Hcompress(HcompressDefect),
    /// A null-pixel mask holds a value other than 0 and 1.
    #[error("a null-pixel mask holds a value other than 0 and 1")]
    NullMask,
    /// A tile has neither a compressed nor an uncompressed cell.
    #[error("a tile has neither a compressed nor an uncompressed cell")]
    EmptyTile,
    /// A tile's cell holds elements its decoder does not read: a compressed stream that
    /// is not bytes, or PLIO's 16-bit words; uncompressed samples of another type.
    #[error("a tile's {cell} cell holds {element:?} elements")]
    CellElement {
        cell: TileCell,
        element: ElementKind,
    },
    /// A decoded integer lies outside the range of the image's `BITPIX`.
    #[error("a decoded value lies outside the range of the image's BITPIX")]
    ValueOutOfRange,
}

/// The cell of a tile that [`CompressedDataDefect::CellElement`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileCell {
    /// The tile's compressed stream.
    Compressed,
    /// The tile's samples, stored as they are (`UNCOMPRESSED_DATA`).
    Uncompressed,
}

impl fmt::Display for TileCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Compressed => "compressed",
            Self::Uncompressed => "uncompressed",
        })
    }
}

/// Why a PLIO line list does not decode, from [`CompressedDataDefect::Plio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PlioDefect {
    /// The list header's length or start lies outside the list.
    #[error("the list bounds lie outside the list")]
    ListBounds,
    /// A list header value is negative.
    #[error("a list header value is negative")]
    NegativeHeader,
    /// An instruction's opcode is not one of the eight.
    #[error("an opcode is not one of the eight")]
    Opcode,
}

/// Why an HCOMPRESS stream does not decode, from [`CompressedDataDefect::Hcompress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum HcompressDefect {
    /// The stream does not start with HCOMPRESS's two magic bytes.
    #[error("the stream does not start with the magic bytes")]
    Magic,
    /// The stream declares a negative tile dimension.
    #[error("the stream declares a negative tile dimension")]
    NegativeDimension,
    /// The stream's tile dimensions are not those of its tile.
    #[error("the stream's tile dimensions are not those of its tile")]
    TileDimensions,
    /// A bit-plane count is past 63.
    #[error("a bit-plane count is past 63")]
    BitPlaneCount,
    /// The bit planes do not end with a zero code.
    #[error("the bit planes do not end with a zero code")]
    BitPlaneEnd,
    /// A quadrant's format code is neither 0 nor 15.
    #[error("a quadrant format code is neither 0 nor 15")]
    FormatCode,
    /// A decoded H-transform coefficient does not fit 64 bits.
    #[error("a decoded coefficient does not fit 64 bits")]
    CoefficientOverflow,
}

impl From<CompressedDataDefect> for FitsError {
    fn from(defect: CompressedDataDefect) -> Self {
        Self::CorruptCompressedData { defect }
    }
}

impl From<PlioDefect> for FitsError {
    fn from(defect: PlioDefect) -> Self {
        CompressedDataDefect::Plio(defect).into()
    }
}

impl From<HcompressDefect> for FitsError {
    fn from(defect: HcompressDefect) -> Self {
        CompressedDataDefect::Hcompress(defect).into()
    }
}

/// The HDU a read asks for, from [`FitsError::HduKindMismatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpectedHdu {
    /// A primary or `IMAGE` array, or, with the `compression` feature, a tiled-compressed
    /// one.
    Image,
    /// A `BINTABLE`, or, with the `compression` feature, a tiled-compressed table.
    BinTable,
    AsciiTable,
    /// A random-groups primary.
    RandomGroups,
}

impl fmt::Display for ExpectedHdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Image => "an image",
            Self::BinTable => "a binary table",
            Self::AsciiTable => "an ASCII table",
            Self::RandomGroups => "random groups",
        })
    }
}

/// What a column read asks of the column's format, from
/// [`FitsError::ColumnKindMismatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpectedColumn {
    Fixed,
    /// A `P` or `Q` variable-length array column.
    Array,
    /// An `X` bit column.
    Bits,
    /// A `C` or `M` complex column.
    Complex,
    /// A column with a numeric physical value.
    Numeric,
    Integer,
}

impl fmt::Display for ExpectedColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fixed => "a fixed-width column",
            Self::Array => "a variable-length array column",
            Self::Bits => "a bit column",
            Self::Complex => "a complex column",
            Self::Numeric => "a numeric column",
            Self::Integer => "an integer column",
        })
    }
}

/// Why an ASCII-table cell cannot hold a value, from [`FitsError::InvalidAsciiValue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AsciiValueDefect {
    /// A float that is not finite, which a field cannot spell; a null is `None`.
    #[error("a float cell must be finite; a null is None")]
    NonFinite,
    /// A value that reads as the column's `TNULLn` marker.
    #[error("the value equals the column's TNULLn marker")]
    NullMarker,
}

/// A time conversion that needs data from outside the file, from
/// [`FitsError::ExternalTimeDataRequired`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeConversion {
    /// A UTC leap-second label to an MJD, which needs a leap-second table.
    LeapSecond,
    /// A tropical year outside TDB, which needs ephemeris data.
    TropicalYear,
    /// A Besselian year outside TT or ET, which needs ephemeris data.
    BesselianYear,
}

impl fmt::Display for TimeConversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LeapSecond => "converting a UTC leap-second label to MJD",
            Self::TropicalYear => "a tropical year outside TDB",
            Self::BesselianYear => "a Besselian year outside TT and ET",
        })
    }
}

/// The kind of unit a keyword needs, from [`FitsError::InvalidUnit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnitKind {
    Angle,
    Time,
    Frequency,
    Energy,
    /// A wavenumber's unit.
    InverseLength,
    Velocity,
    Length,
    /// A redshift's or a velocity ratio's unit, `1`.
    Dimensionless,
}

impl fmt::Display for UnitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Angle => "an angle unit",
            Self::Time => "a time unit",
            Self::Frequency => "a frequency unit",
            Self::Energy => "an energy unit",
            Self::InverseLength => "an inverse length unit",
            Self::Velocity => "a velocity unit",
            Self::Length => "a length unit",
            Self::Dimensionless => "dimensionless",
        })
    }
}

/// The field of a `P`/`Q` descriptor, from [`FitsError::InvalidPqDescriptor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescriptorField {
    Count,
    Offset,
}

impl fmt::Display for DescriptorField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Count => "element count",
            Self::Offset => "heap offset",
        })
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FitsError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A previous sink error may have left a partial HDU in the output. Further
    /// writes are rejected because appending cannot repair that FITS stream.
    #[error("writer is unusable after a previous output failure")]
    WriterFailed,
    /// `write_empty_primary` was called after an HDU, which made the primary.
    #[error("the primary HDU is already written")]
    PrimaryAlreadyWritten,
    /// A keyword name violated FITS syntax.
    #[error("invalid keyword name {name:?}")]
    InvalidKeyword { name: String },
    /// A WCS alternate that is not a letter `A` to `Z` (§8.2).
    #[error("invalid WCS alternate {alternate:?}: an alternate is a letter A to Z")]
    InvalidAlternate { alternate: char },
    /// A control or commentary name where a valued record was asked for: `END`,
    /// `CONTINUE`, `COMMENT`, `HISTORY`, or any [`RecordName::Commentary`].
    #[error("reserved name {name} cannot be used as a valued card")]
    ReservedName { name: RecordName },
    /// A commentary record whose name and text would read back as another record: a
    /// valued, `HIERARCH` or `CONTINUE` one.
    #[error("commentary record {name} would read back as another record")]
    AmbiguousCommentary { name: CommentaryName },
    /// A header value has no conforming FITS representation.
    #[error("header record {name} has an invalid value: {reason}")]
    InvalidHeaderValue { name: RecordName, reason: NonFinite },
    /// A logical header record cannot fit one physical 80-byte record.
    #[error("header record {name} needs {length} bytes but a FITS record holds 80")]
    HeaderCardTooLong { name: RecordName, length: usize },
    /// A header record that did not parse: a lookup of its name, or a write of its
    /// header.
    #[error(fmt = fmt_invalid_value)]
    InvalidValue {
        name: Option<RecordName>,
        defect: Defect,
    },
    /// A header record whose value is not of the type it was read as.
    #[error("the value of {name} is not {expected}")]
    ValueTypeMismatch {
        name: RecordName,
        expected: ValueKind,
    },
    /// WCS keywords that describe no valid transform; [`WcsDefect`] says which case.
    #[error("invalid WCS: {defect}")]
    InvalidWcs { defect: WcsDefect },
    /// A datetime that is not a FITS ISO-8601 or legacy `DD/MM/YY` one, or names no
    /// instant (§9.1.1, §4.4.2.1); its keyword when a header gave it.
    #[error("invalid datetime{}", keyword.map_or(String::new(), |keyword| format!(" in {keyword}")))]
    InvalidDatetime { keyword: Option<Keyword> },
    /// A time-scale code that does not parse (§9.2.1); its keyword when a header gave it.
    #[error("invalid time scale{}", keyword.map_or(String::new(), |keyword| format!(" in {keyword}")))]
    InvalidTimeScale { keyword: Option<Keyword> },
    /// A time whose MJD lies past the range an `Mjd` holds.
    #[error("the time lies outside the MJD range")]
    TimeOutOfRange,
    /// A compression parameter outside its valid range; [`CompressionParameter`] says
    /// which.
    #[error("invalid compression parameter: {parameter}")]
    InvalidCompressionParameter { parameter: CompressionParameter },
    /// An ASCII-table field that does not hold a number of its column's format.
    #[error("ASCII-table field {field:?} is not a valid number")]
    InvalidAsciiField { field: String },
    /// A value the cell of ASCII-table column `column` in `row` cannot hold;
    /// [`AsciiValueDefect`] says why.
    #[error("ASCII-table column {column:?}, row {row}: {defect}")]
    InvalidAsciiValue {
        column: String,
        /// Zero-based table row containing the value.
        row: usize,
        defect: AsciiValueDefect,
    },
    /// Interpreting a valid FITS time value requires leap-second, Earth-orientation,
    /// or ephemeris data that this format library does not provide;
    /// [`TimeConversion`] says which conversion needs it.
    #[error("{conversion} requires external astronomical time data")]
    ExternalTimeDataRequired { conversion: TimeConversion },
    /// A stream chunk whose samples are not of the stream's `BITPIX`.
    #[error("a chunk of {got:?} samples was written to a stream of {expected:?} samples")]
    ChunkTypeMismatch { expected: Bitpix, got: Bitpix },
    /// The variable-length rows built into one column hold two element types: the
    /// zero-based `row` holds `got`, where the rows before it hold `expected`.
    #[error("variable-length row {row} holds {got:?} elements, not {expected:?}")]
    MixedArrayRows {
        row: usize,
        expected: ElementKind,
        got: ElementKind,
    },
    /// Text that holds bytes outside FITS restricted ASCII (0x20–0x7e).
    #[error("{text} contains characters outside FITS restricted ASCII")]
    InvalidAscii { text: TextOrigin },
    /// An ASCII-table value cannot fit the field width declared by its `TFORMn`.
    #[error(
        "ASCII column {column:?} row {row} needs at least {minimum_width} characters but its field width is {width}"
    )]
    AsciiFieldTooWide {
        column: String,
        /// Zero-based table row containing the value.
        row: usize,
        width: usize,
        minimum_width: usize,
    },
    /// An ASCII float column's `TFORMn` is narrower than its smallest cell: a digit, the
    /// decimal point and the decimals, and an exponent form's `E+00`.
    #[error(
        "ASCII column {column:?} has a float field width of {width}, but its cells need at least {minimum_width} characters"
    )]
    AsciiFloatFormatTooNarrow {
        column: String,
        width: usize,
        minimum_width: usize,
    },
    /// A binary-table `A` field is longer than its column's field width.
    #[error(
        "character column {column:?} row {row} holds {length} bytes but its field width is {width}"
    )]
    CharacterFieldTooWide {
        column: String,
        /// Zero-based table row containing the field.
        row: usize,
        length: usize,
        width: usize,
    },
    /// A binary-table `X` row's bit count differs from its column's width.
    #[error("bit column {column:?} row {row} holds {length} bits but its width is {width}")]
    BitFieldLengthMismatch {
        column: String,
        /// Zero-based table row containing the field.
        row: usize,
        length: usize,
        width: usize,
    },
    /// `BITPIX` held a value outside {8, 16, 32, 64, −32, −64}.
    #[error("invalid BITPIX value {code}")]
    InvalidBitpix { code: i64 },
    /// A header unit ended (ran out of cards) without an `END` record.
    #[error("header unit ended without an END record")]
    MissingEnd,
    /// A valued record is absent: a mandatory keyword the structure requires, or the
    /// record a read or an edit names.
    #[error("missing keyword {name}")]
    MissingKeyword { name: RecordName },
    /// A keyword was present and well-typed but its value lies outside the range
    /// the standard permits for its role (e.g. `NAXIS > 999`, `PCOUNT < 0`,
    /// a negative `GCOUNT` or axis length, or a `THEAP` that precedes the heap).
    #[error("keyword {keyword} has an out-of-range value")]
    KeywordOutOfRange { keyword: Keyword },
    /// An exact FITS integer cannot be represented by the requested bounded type; the
    /// record it was read from when a header read it.
    #[error("FITS integer {value} is outside the {target} range")]
    IntegerOutOfRange {
        name: Option<RecordName>,
        value: String,
        target: NumericTarget,
    },
    /// The byte stream ended in the middle of a header or data unit.
    #[error("unexpected end of stream inside a FITS unit")]
    UnexpectedEof,
    /// The data-unit size implied by the header overflows a 64-bit byte count
    /// (a malformed or hostile header with absurd `NAXISn`/`PCOUNT`/`GCOUNT`).
    #[error("header-implied data-unit size overflows 64 bits")]
    DataUnitOverflow,
    /// An output or staging buffer sized directly from untrusted FITS metadata is
    /// too large to allocate. Reader staging and final decompressed image/table
    /// planes allocate fallibly so hostile dimensions surface as this error rather
    /// than an out-of-memory process abort.
    #[error("header-implied data-unit size ({bytes} bytes) is too large to allocate")]
    DataUnitTooLarge { bytes: u64 },
    /// A data unit, a part of one, or a buffer handed in held another size than its
    /// header or its geometry declares — a corrupt or truncated unit, or a caller's
    /// buffer of the wrong length.
    #[error("{got} {unit} where {expected} are declared")]
    DataSizeMismatch {
        unit: SizeUnit,
        expected: u64,
        got: u64,
    },
    /// A `P`/`Q` descriptor's count or offset that the signed integers of its width
    /// cannot hold (§7.3.5).
    #[error("a {width:?} descriptor cannot hold the count {count} and the offset {offset}")]
    DescriptorOutOfRange {
        width: DescriptorWidth,
        count: usize,
        offset: usize,
    },
    /// A signed `P`/`Q` heap-array descriptor contains a negative element count
    /// or byte offset.
    #[error("invalid P/Q descriptor {field}: {value}")]
    InvalidPqDescriptor { field: DescriptorField, value: i64 },
    /// An index named something beyond the bound of whatever it addressed —
    /// [`Indexed`] says which.
    #[error(fmt = Indexed::fmt_out_of_bounds)]
    IndexOutOfBounds {
        indexed: Indexed,
        index: usize,
        len: usize,
    },
    /// Two axis counts that had to agree did not — [`Ranked`] says which pair.
    #[error(fmt = Ranked::fmt_mismatch)]
    RankMismatch {
        ranked: Ranked,
        /// Axis count the structure requires.
        expected: usize,
        /// Axis count the caller supplied.
        got: usize,
    },
    /// One zero-based half-open image-axis range is reversed or exceeds its axis.
    #[error("image region axis {axis} range {start}..{end} exceeds axis length {len}")]
    ImageRegionOutOfBounds {
        axis: usize,
        start: usize,
        end: usize,
        len: usize,
    },
    /// A zero-based half-open table row range is reversed or exceeds the table.
    #[error("table row range {start}..{end} exceeds the table's {len} rows")]
    RowRangeOutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    /// A table column contains a different number of rows from the table being built.
    #[error("table column {column:?} has {got} rows but the table requires {expected}")]
    TableRowCountMismatch {
        column: String,
        expected: usize,
        got: usize,
    },
    /// A fixed-width table column whose elements do not divide into whole rows.
    #[error(
        "table column {column:?} holds {count} elements, not a whole number of {row_width}-element rows"
    )]
    PartialTableRow {
        column: String,
        count: usize,
        row_width: usize,
    },
    /// Empty or zero-width column data did not carry enough information to infer
    /// the intended table row count.
    #[error(
        "table column {column:?} does not determine a row count; declare the table row count explicitly"
    )]
    TableRowCountUndetermined { column: String },
    /// Variable-length rows built from a list of rows need at least one row to
    /// give their element type; an empty column states it with an empty typed
    /// `ColumnData`.
    #[error("variable-length rows need one row to give their element type")]
    EmptyVlaNeedsType,
    /// A FITS keyword family was addressed with zero even though its indices start at 1.
    #[error("{} indices are 1-based and cannot be zero", .indexed.name())]
    OneBasedIndexRequired { indexed: Indexed },
    /// A replacement header needs more 2880-byte blocks than the header it replaces in
    /// place holds.
    #[error("the header needs {needed} blocks but the file holds {available} for it")]
    HeaderDoesNotFit { needed: usize, available: usize },
    /// An optional column keyword (`TSCALn`, `TZEROn`, `TNULLn`, `TDIMn`, `TDISPn`) whose
    /// value did not read: a read of its column that needs it fails, and the other
    /// columns read as they are.
    #[error("column keyword {keyword} is unusable: {source}")]
    InvalidColumnKeyword { keyword: Keyword, source: Arc<Self> },
    /// A file to append to holds bytes after its last HDU, such as special records,
    /// which a reader stops at, so it would not see an appended HDU.
    #[error("the last HDU ends at byte {hdu_end}, but the file is {file_len} bytes")]
    TrailingContent { hdu_end: u64, file_len: u64 },
    /// An edit changes the kind of an HDU, whose data unit is then of the wrong kind.
    #[error("the header makes the {from:?} HDU a {to:?}")]
    HduKindChange { from: HduKind, to: HduKind },
    /// No HDU carries the extension name, version and level a read named.
    #[error("no HDU named {name:?} (EXTVER {version:?}, EXTLEVEL {level:?})")]
    HduNotFound {
        name: String,
        version: Option<i64>,
        level: Option<i64>,
    },
    /// A read of an HDU of another kind than it reads: an image read of a table, a
    /// table read of an image, as [`ExpectedHdu`] says.
    #[error("the HDU is {got:?}, not {expected}")]
    HduKindMismatch { expected: ExpectedHdu, got: HduKind },
    /// An IMAGE/primary HDU carries group structure (`PCOUNT ≠ 0` or `GCOUNT ≠ 1`),
    /// which a plain image array must not have (§4.3).
    #[error("image HDU has group structure (PCOUNT ≠ 0 or GCOUNT ≠ 1)")]
    ImageHasGroups,
    /// WCS keywords that cannot hold together; [`WcsConflict`] says which.
    #[error("conflicting WCS keywords: {conflict}")]
    ConflictingWcsKeywords { conflict: WcsConflict },
    /// A complete pixel↔world transform was requested for axes whose nonlinear
    /// algorithm is not implemented. The indices are zero-based, matching
    /// [`crate::world_coordinates::Description::unsupported_axes`].
    #[error("WCS has unsupported nonlinear transforms on zero-based axes {axes:?}")]
    UnsupportedWcsTransform { axes: Vec<usize> },
    /// An intermediate (projection-plane) coordinate lies outside the region its WCS
    /// projection maps back onto the sphere (wcslib `PRJERR_BAD_PIX`).
    #[error("coordinate is outside the {projection} projection domain")]
    WcsPixelOutOfDomain { projection: Projection },
    /// A native spherical coordinate has no image under its WCS projection: it lies
    /// behind a zenithal projection's horizon, past a perspective projection's limb, or
    /// at a pole a projection sends to infinity (wcslib `PRJERR_BAD_WORLD`).
    #[error("world coordinate has no image under the {projection} projection")]
    WcsWorldOutOfDomain { projection: Projection },
    /// The fiducial point, `LONPOLE` and `LATPOLE` admit no celestial pole (CG 2002 §2.4,
    /// wcslib `celset`).
    #[error("no celestial pole fits the WCS: {defect}")]
    WcsInvalidPole { defect: PoleDefect },
    /// A unit string is not a unit of the kind its keyword requires.
    #[error("unit {unit:?} is not {expected}")]
    InvalidUnit { unit: String, expected: UnitKind },
    /// A world or intermediate coordinate lies outside a non-celestial WCS
    /// algorithm's mathematical domain.
    #[error("coordinate on zero-based axis {axis} is outside the {algorithm} WCS domain")]
    WcsCoordinateOutOfDomain {
        /// Zero-based WCS axis containing the invalid coordinate.
        axis: usize,
        algorithm: &'static str,
    },
    /// An iterative WCS algorithm did not reach a valid solution.
    #[error("{algorithm} WCS iteration did not converge")]
    WcsNoConvergence { algorithm: &'static str },
    /// An `HCOMPRESS_1` tile with a side under 4 pixels, which cfitsio neither writes
    /// nor reads; `axis` is 0-based, `length` the shortest tile along it, the last
    /// tile's remainder included.
    #[error("HCOMPRESS_1 tile is {length} pixels along axis {axis}, under the minimum of 4")]
    HcompressTileTooSmall { axis: usize, length: usize },
    /// A tiled-compression feature this crate does not support; [`CompressionFeature`]
    /// says which.
    #[error("unsupported tiled compression: {feature}")]
    UnsupportedCompression { feature: CompressionFeature },
    /// A compressed stream or tile does not decode; [`CompressedDataDefect`] says why.
    #[error("corrupt compressed data: {defect}")]
    CorruptCompressedData { defect: CompressedDataDefect },
    /// A PLIO tile sample cannot be represented losslessly in its unsigned 24-bit
    /// value domain.
    #[error("PLIO tile sample {index} has value {value}, outside 0..=16777215")]
    PlioValueOutOfRange { index: usize, value: i64 },
    /// A `TFORMn` value could not be parsed as a binary-table column format.
    #[error("invalid column format {tform:?}")]
    InvalidTform { tform: String },
    /// A read of the zero-based `column` that its format does not take, as
    /// [`ExpectedColumn`] says: a fixed-width read of an array column, a bit read of
    /// another type.
    #[error("column {column} is not {expected}")]
    ColumnKindMismatch {
        column: usize,
        expected: ExpectedColumn,
    },
    /// No column with the requested `TTYPEn` name exists in the table.
    #[error("no column named {name:?} in the table")]
    ColumnNotFound { name: String },
    /// No group parameter with the requested `PTYPEn` name exists.
    #[error("no group parameter named {name:?}")]
    GroupParameterNotFound { name: String },
    /// The summed column widths disagree with the declared row width (`NAXIS1`).
    #[error("column widths sum to {computed} bytes but NAXIS1 declares {declared}")]
    RowWidthMismatch { computed: usize, declared: usize },
}

/// A conversion that cannot fail, such as a [`Keyword`] passed where a name is
/// checked into one.
impl From<Infallible> for FitsError {
    fn from(never: Infallible) -> Self {
        match never {}
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::fits_error::*;

    #[test]
    fn every_index_and_rank_arm_names_its_own_structure() {
        for (indexed, message) in [
            (Indexed::Hdu, "HDU index 3 out of bounds (file has 2 HDUs)"),
            (
                Indexed::HeaderRecord,
                "header record index 3 out of bounds (header has 2 records)",
            ),
            (
                Indexed::Column,
                "column index 3 out of bounds (table has 2 columns)",
            ),
            (
                Indexed::Group,
                "group index 3 out of bounds (random-groups array has 2 groups)",
            ),
            (
                Indexed::WcsAxis,
                "1-based WCS axis 3 out of bounds (WCS has 2 axes)",
            ),
        ] {
            assert_eq!(
                FitsError::IndexOutOfBounds {
                    indexed,
                    index: 3,
                    len: 2,
                }
                .to_string(),
                message
            );
        }

        // The last two rows differ only in `got`, so they pin the value/values branch.
        for (ranked, expected, got, message) in [
            (
                Ranked::ImageRegion,
                2,
                3,
                "image region has 3 axes but the image has 2",
            ),
            (
                Ranked::TileShape,
                2,
                3,
                "tile shape has 3 axes but the image has 2",
            ),
            (
                Ranked::WcsCoordinate,
                2,
                1,
                "coordinate has 1 value but the WCS has 2 axes",
            ),
            (
                Ranked::WcsCoordinate,
                3,
                2,
                "coordinate has 2 values but the WCS has 3 axes",
            ),
        ] {
            assert_eq!(
                FitsError::RankMismatch {
                    ranked,
                    expected,
                    got,
                }
                .to_string(),
                message
            );
        }
    }

    #[test]
    fn io_error_is_preserved_as_source() {
        let io_err = io::Error::new(io::ErrorKind::UnexpectedEof, "boom");
        let err = FitsError::from(io_err);
        assert!(matches!(err, FitsError::Io(_)));
        assert!(Error::source(&err).is_some());
    }
}
