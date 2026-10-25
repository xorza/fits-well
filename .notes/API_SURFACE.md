# Public API surface

Every item the crate exports, rendered as its declaration from rustdoc JSON
(`cargo drive surface`), sorted by path. The surface is the union of a build with
no features and one build per public feature; a tag names the builds an item or a
line needs or loses. `internals` is the crate's own test surface and is left out.

```text
FitsEditor
    struct FitsEditor<F>
        impl<F: Read + Write + Seek> FitsEditor<F>
            pub fn open(file: F) -> Result<Self>
            pub fn hdus(&self) -> &[Hdu]
            pub fn hdu<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Hdu>
            pub fn images(&self) -> impl Iterator<Item = &Hdu> + '_
            pub fn into_inner(self) -> F
            pub fn replace_header<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, header: &Header) -> Result<()>
            pub fn replace_header_into<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, header: &Header, sink: impl Write) -> Result<()>
            pub fn append(self) -> Result<FitsWriter<F>>
        impl<F: Debug> Debug for FitsEditor<F>

FitsError
    #[non_exhaustive]
    enum FitsError: Debug, Display, Error, From<CompressedDataDefect>, From<Error>, From<HcompressDefect>, From<Infallible>, From<PlioDefect>
        Io(Error)
        WriterFailed
        PrimaryAlreadyWritten
        InvalidKeyword { name: String }
        InvalidAlternate { alternate: char }
        ReservedName { name: RecordName }
        AmbiguousCommentary { name: CommentaryName }
        InvalidHeaderValue { name: RecordName, reason: NonFinite }
        HeaderCardTooLong { name: RecordName, length: usize }
        InvalidValue { name: Option<RecordName>, defect: Defect }
        ValueTypeMismatch { name: RecordName, expected: ValueKind }
        InvalidWcs { defect: WcsDefect }
        InvalidDatetime { keyword: Option<Keyword> }
        InvalidTimeScale { keyword: Option<Keyword> }
        TimeOutOfRange
        InvalidCompressionParameter { parameter: CompressionParameter }
        InvalidAsciiField { column: usize, row: usize, field: String }
        InvalidAsciiValue { column: String, row: usize, defect: AsciiValueDefect }
        ExternalTimeDataRequired { conversion: TimeConversion }
        ChunkTypeMismatch { expected: Bitpix, got: Bitpix }
        MixedArrayRows { row: usize, expected: ElementKind, got: ElementKind }
        InvalidAscii { text: TextOrigin }
        AsciiFieldTooWide { column: String, row: usize, width: usize, minimum_width: usize }
        AsciiFloatFormatTooNarrow { column: String, width: usize, minimum_width: usize }
        CharacterFieldTooWide { column: String, row: usize, length: usize, width: usize }
        BitFieldLengthMismatch { column: String, row: usize, length: usize, width: usize }
        InvalidBitpix { code: i64 }
        MissingEnd
        MissingKeyword { name: RecordName }
        InvalidKeywordValue { keyword: Keyword, defect: KeywordDefect }
        IntegerOutOfRange { name: Option<RecordName>, value: String, target: NumericTarget }
        UnexpectedEof
        DataUnitOverflow
        DataUnitTooLarge { bytes: u64 }
        DataSizeMismatch { unit: SizeUnit, expected: u64, got: u64 }
        DescriptorOutOfRange { width: DescriptorWidth, count: usize, offset: usize }
        DescriptorPastHeap { column: usize, row: usize }
        InvalidPqDescriptor { field: DescriptorField, value: i64 }
        IndexOutOfBounds { indexed: Indexed, index: usize, len: usize }
        RankMismatch { ranked: Ranked, expected: usize, got: usize }
        ImageRegionOutOfBounds { axis: usize, start: usize, end: usize, len: usize }
        RowRangeOutOfBounds { start: usize, end: usize, len: usize }
        TableRowCountMismatch { column: String, expected: usize, got: usize }
        PartialTableRow { column: String, count: usize, row_width: usize }
        TableRowCountUndetermined { column: String }
        EmptyVlaNeedsType
        OneBasedIndexRequired { indexed: Indexed }
        HeaderDoesNotFit { needed: usize, available: usize }
        InvalidColumnKeyword { keyword: Keyword, source: Arc<Self> }
        DefectiveHdu { index: usize, source: Arc<Self> }
        TrailingContent { hdu_end: u64, file_len: u64 }
        HduKindChange { from: HduKind, to: HduKind }
        HduNotFound { name: String, version: Option<i64>, level: Option<i64> }
        HduKindMismatch { expected: ExpectedHdu, got: HduKind }
        ImageHasGroups
        ConflictingWcsKeywords { conflict: WcsConflict }
        UnsupportedWcsTransform { axes: Vec<usize> }
        WcsPixelOutOfDomain { projection: Projection }
        WcsWorldOutOfDomain { projection: Projection }
        WcsInvalidPole { defect: PoleDefect }
        InvalidUnit { unit: String, expected: UnitKind }
        WcsCoordinateOutOfDomain { axis: usize, algorithm: &'static str }
        WcsNoConvergence { algorithm: &'static str }
        HcompressTileTooSmall { axis: usize, length: usize }
        UnsupportedCompression { feature: CompressionFeature }
        CorruptCompressedData { defect: CompressedDataDefect }
        PlioValueOutOfRange { index: usize, value: i64 }
        InvalidTform { tform: String }
        ColumnKindMismatch { column: usize, expected: ExpectedColumn }
        ColumnNotFound { name: String }
        GroupParameterNotFound { name: String }
        RowWidthMismatch { computed: usize, declared: usize }

FitsReader
    struct FitsReader<S>
        impl<R: Read + Seek> FitsReader<StreamSource<R>>
            pub fn open(source: R) -> Result<Self>
            pub fn into_inner(self) -> R
        impl<'a> FitsReader<SliceSource<'a>>
            pub fn from_bytes(bytes: &'a [u8]) -> Result<Self>
            pub fn into_bytes(self) -> &'a [u8]
        impl<S: Source> FitsReader<S>
            pub fn hdus(&self) -> &[Hdu]
            pub fn hdu<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Hdu>
            pub fn scan_defect(&self) -> Option<&FitsError>
            pub fn images(&self) -> impl Iterator<Item = &Hdu> + '_
            pub fn read_data_unit<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Vec<u8>>
            pub fn image<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<ImageReader<'_, S>>
            pub fn read_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Table>
            pub fn read_table_rows<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, rows: impl RangeBounds<usize>) -> Result<Table>
            pub fn read_table_columns<'h, 'c>(&mut self, hdu: impl Into<HduSelector<'h>>, rows: impl RangeBounds<usize>, columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>) -> Result<Selection>
            pub fn read_table_columns_into<'h, 'c>(&mut self, hdu: impl Into<HduSelector<'h>>, rows: impl RangeBounds<usize>, columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>, out: &mut Selection) -> Result<()>
            pub fn read_table_cell<'h, 'c>(&mut self, hdu: impl Into<HduSelector<'h>>, row: usize, column: impl Into<ColumnSelector<'c>>) -> Result<ColumnData>
            pub fn read_wcs<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Wcs>
            pub fn read_ascii_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Ascii>
            pub fn read_groups<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<RandomGroups>
            pub fn verify_checksum<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<ChecksumReport>
        impl<S: Debug> Debug for FitsReader<S>

FitsWriter
    struct FitsWriter<W>
        impl<W: Write> FitsWriter<W>
            pub const fn new(sink: W) -> Self
            pub const fn with_checksums(self) -> Self
            pub const fn with_spare_records(self, records: usize) -> Self
            pub fn write_raw_hdu(&mut self, header: &Header, raw: &[u8]) -> Result<()>
            pub fn into_inner(self) -> W
            pub fn write_ascii_table(&mut self, table: &AsciiBuilder, header: Option<&Header>) -> Result<()>
            pub fn write_table(&mut self, table: &Builder, header: Option<&Header>) -> Result<()>
            pub fn write_image(&mut self, image: Borrowed<'_>, header: Option<&Header>) -> Result<()>
            pub fn write_empty_primary(&mut self, header: Option<&Header>) -> Result<()>
            pub fn write_compressed_image(&mut self, image: Borrowed<'_>, config: &CompressionConfig, header: Option<&Header>) -> Result<()>  [features: compression or parallel]
            pub fn write_compressed_table(&mut self, table: &Builder, config: &CompressionConfig, header: Option<&Header>) -> Result<()>  [features: compression or parallel]
        impl<W: Write + Seek> FitsWriter<W>
            pub fn stream_image(&mut self, layout: Metadata<'_>, header: Option<&Header>) -> Result<ImageWriter<'_, W>>
        impl<W: Debug> Debug for FitsWriter<W>

ImageReader
    struct ImageReader<'r, S>
        impl<'r, S: Source> ImageReader<'r, S>
            pub const fn header(&self) -> &'r Header
            pub const fn metadata(&self) -> Metadata<'r>
            pub fn with_checksums(self) -> Self
            pub fn tiles(&self) -> Option<&[usize]>
            pub fn view<'a, 'g>(&'a mut self, region: impl Into<Region<'g>>, words: &'a mut Vec<u64>) -> Result<Borrowed<'a>>
            pub fn read<'g>(&mut self, region: impl Into<Region<'g>>) -> Result<Image>
            pub fn physical_into<'g, T: PhysicalSample>(&mut self, region: impl Into<Region<'g>>, out: &mut Vec<T>) -> Result<()>
            pub fn view_bytes<'g>(&self, regions: impl IntoIterator<Item = Region<'g>>) -> Result<ViewBytes>
            pub fn finish_bytes(&self) -> u64
            pub fn finish(self) -> Result<ChecksumReport>
        impl<'r, S: Debug> Debug for ImageReader<'r, S>

ImageWriter
    struct ImageWriter<'a, W: Write + Seek>: Drop
        pub fn write_chunk(&mut self, samples: View<'_>) -> Result<()>
        pub fn finish(self) -> Result<()>
        impl<'a, W: Debug + Write + Seek> Debug for ImageWriter<'a, W>

Result
    type Result<T> = Result<T, FitsError>

ViewBytes
    struct ViewBytes: Clone, Copy, Debug, Default, Eq, PartialEq
        pub kept: u64
        pub handle: u64

compress  [features: compression or parallel]
    mod

compress::Compression  [features: compression or parallel]
    enum Compression: Clone, Copy, Debug, PartialEq
        Gzip(Gzip)
        Rice
        Plio
        Hcompress(Hcompress)
        None
        pub const GZIP: Self
        pub const GZIP_SHUFFLED: Self
        pub const fn name(self) -> &'static str

compress::CompressionConfig  [features: compression or parallel]
    struct CompressionConfig: Clone, Debug, PartialEq
        pub const fn new(compression: Compression) -> Self
        pub fn with_tiles(self, tiles: impl Into<Vec<usize>>) -> Self
        pub const fn with_quantization(self, quantization: Quantization) -> Self

compress::Dither  [features: compression or parallel]
    enum Dither: Clone, Copy, Debug, Default, Eq, PartialEq
        None
        Subtractive1(DitherSeed)
        Subtractive2(DitherSeed)

compress::DitherSeed  [features: compression or parallel]
    struct DitherSeed: Clone, Copy, Debug, Eq, PartialEq
        pub const FROM_SAMPLES: Self
        pub fn fixed(seed: u16) -> Result<Self>

compress::Gzip  [features: compression or parallel]
    struct Gzip: Clone, Copy, Debug, Default, Eq, PartialEq
        pub fn new(level: u32) -> Result<Self>
        pub fn shuffled(level: u32) -> Result<Self>

compress::Hcompress  [features: compression or parallel]
    struct Hcompress: Clone, Copy, Debug, Default, PartialEq
        pub fn relative(scale: f64) -> Result<Self>
        pub fn absolute(scale: f64) -> Result<Self>

compress::Quantization  [features: compression or parallel]
    struct Quantization: Clone, Copy, Debug, Default, PartialEq
        pub const NONE: Self
        pub fn relative(level: f64, dither: Dither) -> Result<Self>
        pub fn absolute(step: f64, dither: Dither) -> Result<Self>

error
    mod

error::AsciiValueDefect
    #[non_exhaustive]
    enum AsciiValueDefect: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        NonFinite
        NullMarker

error::CompressedDataDefect
    #[non_exhaustive]
    enum CompressedDataDefect: Debug, Display, Error
        GzipHeader
        GzipStream(DecompressError)  [features: compression or parallel]
        GzipTruncated
        GzipChecksum
        GzipOverrun
        RiceBitField { bits: u32 }
        Plio(PlioDefect)
        Hcompress(HcompressDefect)
        NullMask
        EmptyTile
        CellElement { cell: TileCell, element: ElementKind }
        ValueOutOfRange
        impl From<CompressedDataDefect> for FitsError

error::CompressionFeature
    #[non_exhaustive]
    enum CompressionFeature: Clone, Debug, Display, Eq, Error, PartialEq
        Codec { name: String }
        Quantization { name: String }
        HcompressDimensions
        LossyHcompressNulls
        HcompressNullMask
        HcompressScale
        HcompressTileSide { length: usize }
        HcompressRange
        PlioListLength { words: usize }
        LosslessFloatCodec { codec: &'static str }
        QuantizedFloatCodec { codec: &'static str }
        TableCodec { codec: &'static str }
        ColumnCodec { name: String }
        RiceColumn { code: char }

error::CompressionParameter
    #[non_exhaustive]
    enum CompressionParameter: Clone, Copy, Debug, Display, Error, PartialEq
        GzipLevel { level: u32 }
        HcompressScale { scale: f64 }
        QuantizationLevel { level: f64 }
        QuantizationStep { step: f64 }
        DitherSeed { seed: u16 }
        IntegerQuantization
        TableQuantization
        TableWithoutCodec
        EmptyTile { axis: usize }
        HcompressDither2

error::DescriptorField
    #[non_exhaustive]
    enum DescriptorField: Clone, Copy, Debug, Display, Eq, PartialEq
        Count
        Offset

error::ExpectedColumn
    #[non_exhaustive]
    enum ExpectedColumn: Clone, Copy, Debug, Display, Eq, PartialEq
        Fixed
        Array
        Bits
        Complex
        Numeric
        Integer

error::ExpectedHdu
    #[non_exhaustive]
    enum ExpectedHdu: Clone, Copy, Debug, Display, Eq, PartialEq
        Image
        BinTable
        AsciiTable
        RandomGroups

error::GrismDefect
    #[non_exhaustive]
    enum GrismDefect: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        MissingParameter { parameter: usize }
        NonFinite
        PerpendicularTilt
        ReferenceOutsideDomain
        ZeroDispersion

error::HcompressDefect
    #[non_exhaustive]
    enum HcompressDefect: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        Magic
        NegativeDimension
        TileDimensions
        BitPlaneCount
        BitPlaneEnd
        FormatCode
        CoefficientOverflow
        impl From<HcompressDefect> for FitsError

error::Indexed
    #[non_exhaustive]
    enum Indexed: Clone, Copy, Debug, Eq, PartialEq
        Hdu
        HeaderRecord
        Column
        Group
        WcsAxis

error::KeywordDefect
    enum KeywordDefect: Clone, Copy, Debug, Display, Eq, PartialEq
        Syntax
        Range
        Structure

error::NonFinite
    #[non_exhaustive]
    enum NonFinite: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        Real
        Complex

error::NumericTarget
    #[non_exhaustive]
    enum NumericTarget: Clone, Copy, Debug, Display, Eq, PartialEq
        I64
        U64
        I128
        F64

error::PlioDefect
    #[non_exhaustive]
    enum PlioDefect: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        ListBounds
        NegativeHeader
        Opcode
        impl From<PlioDefect> for FitsError

error::PoleDefect
    #[non_exhaustive]
    enum PoleDefect: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        EquatorialFiducial
        FiducialLatitude
        NoLatitude

error::Ranked
    #[non_exhaustive]
    enum Ranked: Clone, Copy, Debug, Eq, PartialEq
        ImageRegion
        TileShape
        WcsCoordinate
        BinFactors

error::SizeUnit
    enum SizeUnit: Clone, Copy, Debug, Display, Eq, PartialEq
        Bytes
        Samples
        Rows

error::TabularDefect
    #[non_exhaustive]
    enum TabularDefect: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        MissingExtension
        MissingCoordinateColumn
        NonPositiveParameter
        TableAxisPastCount
        NonFiniteReference
        DuplicateTableAxis
        IncompleteTableAxes
        CoordinateShape
        EmptyAxis
        CoordinateArrayTooLarge
        CoordinateCount
        NonFiniteCoordinate
        IndexLength
        NonFiniteIndex
        NonMonotonicIndex
        ConstantIndex
        TooManyAxes
        EmptyTable

error::TextOrigin
    #[non_exhaustive]
    enum TextOrigin: Clone, Debug, Display, Eq, PartialEq
        Value(RecordName)
        Comment(RecordName)
        ColumnName
        ColumnUnit
        NullMarker
        Cell
        TableData

error::TileCell
    #[non_exhaustive]
    enum TileCell: Clone, Copy, Debug, Display, Eq, PartialEq
        Compressed
        Uncompressed

error::TimeConversion
    #[non_exhaustive]
    enum TimeConversion: Clone, Copy, Debug, Display, Eq, PartialEq
        LeapSecond
        TropicalYear
        BesselianYear

error::UnitKind
    #[non_exhaustive]
    enum UnitKind: Clone, Copy, Debug, Display, Eq, PartialEq
        Angle
        Time
        Frequency
        Energy
        InverseLength
        Velocity
        Length
        Dimensionless

error::ValueKind
    #[non_exhaustive]
    enum ValueKind: Clone, Copy, Debug, Display, Eq, PartialEq
        Logical
        Integer
        Real
        Text
        Complex

error::WcsConflict
    #[non_exhaustive]
    enum WcsConflict: Clone, Copy, Debug, Display, Eq, Error, PartialEq
        PcAndCd
        PcAndCrota
        CdAndCrota
        MismatchedProjections
        SipAlternate
        SipAxisCount
        TableRadesys
        TableEquinox
        TableSpectralFrames

error::WcsDefect
    #[non_exhaustive]
    enum WcsDefect: Clone, Debug, Display, Eq, Error, PartialEq
        SingularMatrix
        DegenerateProjection { projection: Projection }
        InvalidEquinox
        LogarithmicReference { ctype: String }
        InconsistentSpectralAlgorithm { ctype: String }
        SpectralReference { kind: &'static str }
        RestValue { keyword: Keyword }
        MissingRest
        Grism { algorithm: &'static str, defect: GrismDefect }
        Tabular(TabularDefect)

header
    mod

header::CommentaryName
    struct CommentaryName: Clone, Copy, Debug, Display, Eq, FromStr, Hash, PartialEq, TryFrom<&String>, TryFrom<&str>, TryFrom<String>
        pub const COMMENT: Self
        pub const HISTORY: Self
        pub const BLANK: Self
        pub const CONTINUE: Self
        pub const fn as_str(&self) -> &str
        impl From<CommentaryName> for RecordName

header::Defect
    enum Defect: Clone, Copy, Debug, Display, Eq, PartialEq
        Name
        Value

header::FitsInteger
    struct FitsInteger: Clone, Debug, Display, Eq, From<i128>, From<i32>, From<i64>, From<u128>, From<u16>, From<u32>, From<u64>, From<usize>, FromStr, Hash, PartialEq
        impl From<FitsInteger> for Value
        impl TryFrom<&FitsInteger> for i128
        impl TryFrom<&FitsInteger> for i64
        impl TryFrom<&FitsInteger> for u64

header::Header
    struct Header: Clone, Debug, Default, PartialEq
        pub fn parse(bytes: &[u8]) -> Result<Self>
        pub fn repairs(&self) -> &[Repair]
        pub fn get(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<&Value>>
        pub fn get_comment(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<&str>>
        pub fn get_logical(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<bool>>
        pub fn get_integer(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<i64>>
        pub fn get_real(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<f64>>
        pub fn get_complex(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<Complex<f64>>>
        pub fn get_text(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<&str>>
        pub fn require_logical(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<bool>
        pub fn require_integer(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<i64>
        pub fn require_real(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<f64>
        pub fn require_complex(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Complex<f64>>
        pub fn require_text(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<&str>
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn contains(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<bool>
        pub fn index_of(&self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<usize>>
        pub fn iter(&self) -> Iter<'_>
        pub fn commentary(&self, name: CommentaryName) -> impl Iterator<Item = &str>
        pub fn new() -> Self
        pub fn set(&mut self, name: impl TryInto<RecordName, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn set_optional(&mut self, name: impl TryInto<RecordName, Error: Into<FitsError>>, value: Option<impl Into<Value>>) -> Result<&mut Self>
        pub fn append(&mut self, name: impl TryInto<RecordName, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn insert(&mut self, index: usize, name: impl TryInto<RecordName, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn insert_record(&mut self, index: usize, record: Record) -> Result<&mut Self>
        pub fn remove(&mut self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<Option<Record>>
        pub fn remove_at(&mut self, index: usize) -> Result<Record>
        pub fn remove_all(&mut self, name: impl TryInto<RecordName, Error: Into<FitsError>>) -> Result<usize>
        pub fn set_comment(&mut self, name: impl TryInto<RecordName, Error: Into<FitsError>>, text: &str) -> Result<&mut Self>
        pub fn push_commentary(&mut self, name: impl TryInto<CommentaryName, Error: Into<FitsError>>, text: &str) -> Result<&mut Self>
        impl<'a> IntoIterator for &'a Header

header::HierarchName
    struct HierarchName: Clone, Debug, Display, Eq, FromStr, Hash, PartialEq, TryFrom<&String>, TryFrom<&str>, TryFrom<String>
        pub fn as_str(&self) -> &str
        impl From<HierarchName> for RecordName

header::Iter
    struct Iter<'a>: Clone, Debug, ExactSizeIterator, Iterator

header::Keyword
    struct Keyword: Clone, Copy, Debug, Display, Eq, FromStr, Hash, PartialEq, PartialEq<&str>, PartialEq<str>, TryFrom<&String>, TryFrom<&str>, TryFrom<Arguments<'_>>, TryFrom<String>
        pub const fn new(name: &str) -> Self
        pub fn indexed(self, index: usize) -> Result<Self>
        pub const fn as_str(&self) -> &str
        impl From<Keyword> for RecordName

header::Record
    enum Record: Clone, Debug, PartialEq
        Valued { keyword: Keyword, value: Value, comment: Option<String> }
        Commentary { name: CommentaryName, text: Option<String> }
        Hierarch { name: HierarchName, value: Value, comment: Option<String> }
        Malformed { name: Option<RecordName>, raw: Box<[u8; 80]>, defect: Defect }
        pub fn view(&self) -> RecordView<'_>

header::RecordName
    enum RecordName: Clone, Debug, Display, Eq, From<CommentaryName>, From<HierarchName>, From<Keyword>, FromStr, Hash, PartialEq, TryFrom<&String>, TryFrom<&str>, TryFrom<String>
        Keyword(Keyword)
        Hierarch(HierarchName)
        Commentary(CommentaryName)

header::RecordView
    enum RecordView<'a>: Clone, Copy, Debug, PartialEq
        Valued { keyword: Keyword, value: &'a Value, comment: Option<&'a str> }
        Commentary { name: CommentaryName, text: Option<&'a str> }
        Hierarch { name: &'a HierarchName, value: &'a Value, comment: Option<&'a str> }
        Malformed { name: Option<&'a RecordName>, raw: &'a [u8; 80], defect: Defect }
        pub const fn keyword(self) -> Option<Keyword>
        pub fn name(self) -> Option<RecordName>

header::Repair
    struct Repair: Clone, Copy, Debug, Eq, PartialEq
        pub record: usize
        pub kind: RepairKind

header::RepairKind
    enum RepairKind: Clone, Copy, Debug, Eq, PartialEq
        LowercaseName
        ValueIndicator
        NonAsciiText
        EndText

header::Value
    enum Value: Clone, Debug, From<&str>, From<FitsInteger>, From<String>, From<bool>, From<f32>, From<f64>, From<i128>, From<i32>, From<i64>, From<u128>, From<u16>, From<u32>, From<u64>, From<usize>, PartialEq
        Logical(bool)
        Integer(FitsInteger)
        Real(f64)
        Text(String)
        ComplexInteger { re: FitsInteger, im: FitsInteger }
        ComplexReal { re: f64, im: f64 }
        Undefined
        pub const fn as_logical(&self) -> Option<bool>
        pub fn as_integer(&self) -> Result<Option<i64>>
        pub fn as_real(&self) -> Result<Option<f64>>
        pub fn as_complex(&self) -> Result<Option<Complex<f64>>>
        pub fn as_text(&self) -> Option<&str>

header::keywords
    mod

header::keywords::AUTHOR
    const AUTHOR: Keyword

header::keywords::BEPOCH
    const BEPOCH: Keyword

header::keywords::BITPIX
    const BITPIX: Keyword

header::keywords::BLANK
    const BLANK: Keyword

header::keywords::BLOCKED
    const BLOCKED: Keyword

header::keywords::BSCALE
    const BSCALE: Keyword

header::keywords::BUNIT
    const BUNIT: Keyword

header::keywords::BZERO
    const BZERO: Keyword

header::keywords::CDELT
    const CDELT: Keyword

header::keywords::CHECKSUM
    const CHECKSUM: Keyword

header::keywords::CNAME
    const CNAME: Keyword

header::keywords::COMMENT
    const COMMENT: Keyword

header::keywords::CONTINUE
    const CONTINUE: Keyword

header::keywords::CRDER
    const CRDER: Keyword

header::keywords::CROTA
    const CROTA: Keyword

header::keywords::CRPIX
    const CRPIX: Keyword

header::keywords::CRVAL
    const CRVAL: Keyword

header::keywords::CSYER
    const CSYER: Keyword

header::keywords::CTYPE
    const CTYPE: Keyword

header::keywords::CUNIT
    const CUNIT: Keyword

header::keywords::DATAMAX
    const DATAMAX: Keyword

header::keywords::DATAMIN
    const DATAMIN: Keyword

header::keywords::DATASUM
    const DATASUM: Keyword

header::keywords::DATE
    const DATE: Keyword

header::keywords::DATEREF
    const DATEREF: Keyword

header::keywords::DATE_AVG
    const DATE_AVG: Keyword

header::keywords::DATE_BEG
    const DATE_BEG: Keyword

header::keywords::DATE_END
    const DATE_END: Keyword

header::keywords::DATE_OBS
    const DATE_OBS: Keyword

header::keywords::END
    const END: Keyword

header::keywords::EPOCH
    const EPOCH: Keyword

header::keywords::EQUINOX
    const EQUINOX: Keyword

header::keywords::EXTEND
    const EXTEND: Keyword

header::keywords::EXTLEVEL
    const EXTLEVEL: Keyword

header::keywords::EXTNAME
    const EXTNAME: Keyword

header::keywords::EXTVER
    const EXTVER: Keyword

header::keywords::GCOUNT
    const GCOUNT: Keyword

header::keywords::GROUPS
    const GROUPS: Keyword

header::keywords::HISTORY
    const HISTORY: Keyword

header::keywords::INSTRUME
    const INSTRUME: Keyword

header::keywords::JDREF
    const JDREF: Keyword

header::keywords::JDREFF
    const JDREFF: Keyword

header::keywords::JDREFI
    const JDREFI: Keyword

header::keywords::JEPOCH
    const JEPOCH: Keyword

header::keywords::LATPOLE
    const LATPOLE: Keyword

header::keywords::LONPOLE
    const LONPOLE: Keyword

header::keywords::MJDREF
    const MJDREF: Keyword

header::keywords::MJDREFF
    const MJDREFF: Keyword

header::keywords::MJDREFI
    const MJDREFI: Keyword

header::keywords::MJD_AVG
    const MJD_AVG: Keyword

header::keywords::MJD_BEG
    const MJD_BEG: Keyword

header::keywords::MJD_END
    const MJD_END: Keyword

header::keywords::MJD_OBS
    const MJD_OBS: Keyword

header::keywords::NAXIS
    const NAXIS: Keyword

header::keywords::OBJECT
    const OBJECT: Keyword

header::keywords::OBSERVER
    const OBSERVER: Keyword

header::keywords::ORIGIN
    const ORIGIN: Keyword

header::keywords::PCOUNT
    const PCOUNT: Keyword

header::keywords::PSCAL
    const PSCAL: Keyword

header::keywords::PTYPE
    const PTYPE: Keyword

header::keywords::PZERO
    const PZERO: Keyword

header::keywords::RADECSYS
    const RADECSYS: Keyword

header::keywords::RADESYS
    const RADESYS: Keyword

header::keywords::REFERENC
    const REFERENC: Keyword

header::keywords::RESTFREQ
    const RESTFREQ: Keyword

header::keywords::RESTFRQ
    const RESTFRQ: Keyword

header::keywords::RESTWAV
    const RESTWAV: Keyword

header::keywords::SIMPLE
    const SIMPLE: Keyword

header::keywords::SPECSYS
    const SPECSYS: Keyword

header::keywords::SSYSOBS
    const SSYSOBS: Keyword

header::keywords::TBCOL
    const TBCOL: Keyword

header::keywords::TCPER
    const TCPER: Keyword

header::keywords::TCTYP
    const TCTYP: Keyword

header::keywords::TCZPH
    const TCZPH: Keyword

header::keywords::TDIM
    const TDIM: Keyword

header::keywords::TDISP
    const TDISP: Keyword

header::keywords::TELAPSE
    const TELAPSE: Keyword

header::keywords::TELESCOP
    const TELESCOP: Keyword

header::keywords::TFIELDS
    const TFIELDS: Keyword

header::keywords::TFORM
    const TFORM: Keyword

header::keywords::THEAP
    const THEAP: Keyword

header::keywords::TIMEDEL
    const TIMEDEL: Keyword

header::keywords::TIMEOFFS
    const TIMEOFFS: Keyword

header::keywords::TIMEPIXR
    const TIMEPIXR: Keyword

header::keywords::TIMESYS
    const TIMESYS: Keyword

header::keywords::TIMEUNIT
    const TIMEUNIT: Keyword

header::keywords::TIMRDER
    const TIMRDER: Keyword

header::keywords::TIMSYER
    const TIMSYER: Keyword

header::keywords::TNULL
    const TNULL: Keyword

header::keywords::TREFPOS
    const TREFPOS: Keyword

header::keywords::TRPOS
    const TRPOS: Keyword

header::keywords::TSCAL
    const TSCAL: Keyword

header::keywords::TSTART
    const TSTART: Keyword

header::keywords::TSTOP
    const TSTOP: Keyword

header::keywords::TTYPE
    const TTYPE: Keyword

header::keywords::TUNIT
    const TUNIT: Keyword

header::keywords::TZERO
    const TZERO: Keyword

header::keywords::WCSAXES
    const WCSAXES: Keyword

header::keywords::WCSNAME
    const WCSNAME: Keyword

header::keywords::XPOSURE
    const XPOSURE: Keyword

header::keywords::XTENSION
    const XTENSION: Keyword

header::keywords::ZBITPIX
    const ZBITPIX: Keyword

header::keywords::ZBLANK
    const ZBLANK: Keyword

header::keywords::ZBLOCKED
    const ZBLOCKED: Keyword

header::keywords::ZCMPTYPE
    const ZCMPTYPE: Keyword

header::keywords::ZCTYP
    const ZCTYP: Keyword

header::keywords::ZDATASUM
    const ZDATASUM: Keyword

header::keywords::ZDITHER0
    const ZDITHER0: Keyword

header::keywords::ZEXTEND
    const ZEXTEND: Keyword

header::keywords::ZFORM
    const ZFORM: Keyword

header::keywords::ZGCOUNT
    const ZGCOUNT: Keyword

header::keywords::ZHEAPPTR
    const ZHEAPPTR: Keyword

header::keywords::ZHECKSUM
    const ZHECKSUM: Keyword

header::keywords::ZIMAGE
    const ZIMAGE: Keyword

header::keywords::ZMASKCMP
    const ZMASKCMP: Keyword

header::keywords::ZNAME
    const ZNAME: Keyword

header::keywords::ZNAXIS
    const ZNAXIS: Keyword

header::keywords::ZPCOUNT
    const ZPCOUNT: Keyword

header::keywords::ZQUANTIZ
    const ZQUANTIZ: Keyword

header::keywords::ZSCALE
    const ZSCALE: Keyword

header::keywords::ZSIMPLE
    const ZSIMPLE: Keyword

header::keywords::ZTABLE
    const ZTABLE: Keyword

header::keywords::ZTENSION
    const ZTENSION: Keyword

header::keywords::ZTHEAP
    const ZTHEAP: Keyword

header::keywords::ZTILE
    const ZTILE: Keyword

header::keywords::ZTILELEN
    const ZTILELEN: Keyword

header::keywords::ZVAL
    const ZVAL: Keyword

header::keywords::ZZERO
    const ZZERO: Keyword

image
    mod

image::Bitpix
    enum Bitpix: Clone, Copy, Debug, Eq, Hash, PartialEq, TryFrom<i64>
        U8
        I16
        I32
        I64
        F32
        F64
        pub const fn code(self) -> i64
        pub const fn elem_size(self) -> usize
        pub const fn is_float(self) -> bool
        pub const fn is_integer(self) -> bool

image::Borrowed
    struct Borrowed<'a>: Clone, Copy, Debug
        pub fn new(shape: &'a [usize], samples: View<'a>, scaling: Scaling) -> Result<Self>
        pub const fn shape(&self) -> &'a [usize]
        pub const fn scaling(&self) -> Scaling
        pub const fn samples(&self) -> View<'a>
        pub const fn metadata(&self) -> Metadata<'a>
        pub fn physical_into<T: PhysicalSample>(&self, out: &mut Vec<T>)
        pub fn unsigned_into(&self, out: &mut UnsignedData) -> bool
        pub fn sample_type(&self) -> SampleType
        pub fn to_image(&self) -> Image

image::Data
    enum Data: Clone, Debug, PartialEq
        U8(Vec<u8>)
        I16(Vec<i16>)
        I32(Vec<i32>)
        I64(Vec<i64>)
        F32(Vec<f32>)
        F64(Vec<f64>)
        pub const fn bitpix(&self) -> Bitpix
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub const fn view(&self) -> View<'_>
        pub const fn as_u8(&self) -> Option<&[u8]>
        pub const fn as_i16(&self) -> Option<&[i16]>
        pub const fn as_i32(&self) -> Option<&[i32]>
        pub const fn as_i64(&self) -> Option<&[i64]>
        pub const fn as_f32(&self) -> Option<&[f32]>
        pub const fn as_f64(&self) -> Option<&[f64]>
        impl<T: Sample> From<Vec<T>> for Data
        impl<T: Sample> TryFrom<Data> for Vec<T>

image::Image
    struct Image: Clone, Debug
        pub fn new(shape: impl Into<Vec<usize>>, samples: impl Into<Data>, scaling: Scaling) -> Result<Self>
        pub fn view(&self) -> Borrowed<'_>
        pub const fn shape(&self) -> &[usize]
        pub const fn samples(&self) -> &Data
        pub const fn scaling(&self) -> Scaling
        pub fn into_samples(self) -> Data
        pub fn into_parts(self) -> Parts
        pub fn from_u16(shape: impl Into<Vec<usize>>, data: &[u16]) -> Result<Self>
        pub fn from_u32(shape: impl Into<Vec<usize>>, data: &[u32]) -> Result<Self>
        pub fn from_u64(shape: impl Into<Vec<usize>>, data: &[u64]) -> Result<Self>
        pub fn from_i8(shape: impl Into<Vec<usize>>, data: &[i8]) -> Result<Self>

image::Metadata
    struct Metadata<'a>: Clone, Copy, Debug, PartialEq
        pub shape: &'a [usize]
        pub bitpix: Bitpix
        pub scaling: Scaling
        pub fn sample_type(&self) -> SampleType

image::Parts
    struct Parts: Clone, Debug
        pub shape: Vec<usize>
        pub samples: Data
        pub scaling: Scaling

image::PhysicalSample
    trait PhysicalSample: Narrow

image::Region
    enum Region<'a>: Clone, Copy, Debug, Eq, From<&'a [Range<usize>]>, From<RangeFull>, PartialEq
        All
        Section(&'a [Range<usize>])
        impl<'a, const N: usize> From<&'a [Range<usize>; N]> for Region<'a>

image::Sample
    trait Sample: Raw + Copy + Send + Sync + 'static
        const BITPIX: Bitpix

image::SampleType
    enum SampleType: Clone, Copy, Debug, Eq, Hash, PartialEq
        I8
        U8
        I16
        U16
        I32
        U32
        I64
        U64
        F32
        F64
        pub fn from_scaling(bitpix: Bitpix, scaling: Scaling) -> Self
        pub const fn is_unsigned(self) -> bool
        pub const fn is_float(self) -> bool
        pub const fn is_integer(self) -> bool

image::Scaling
    struct Scaling: Clone, Copy, Debug, Default, PartialEq
        pub bscale: f64
        pub bzero: f64
        pub blank: Option<i64>
        pub const IDENTITY: Self
        pub fn is_identity(&self) -> bool

image::UnsignedData
    enum UnsignedData: Clone, Debug, Eq, PartialEq, RowValues
        I8(Vec<i8>)
        U16(Vec<u16>)
        U32(Vec<u32>)
        U64(Vec<u64>)
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub const fn view(&self) -> UnsignedView<'_>
        pub const fn as_i8(&self) -> Option<&[i8]>
        pub const fn as_u16(&self) -> Option<&[u16]>
        pub const fn as_u32(&self) -> Option<&[u32]>
        pub const fn as_u64(&self) -> Option<&[u64]>
        impl<T: UnsignedElement> From<Vec<T>> for UnsignedData
        impl<T: UnsignedElement> TryFrom<UnsignedData> for Vec<T>

image::UnsignedElement
    trait UnsignedElement: Element

image::UnsignedView
    enum UnsignedView<'a>: Clone, Copy, Debug, Eq, PartialEq
        I8(&'a [i8])
        U16(&'a [u16])
        U32(&'a [u32])
        U64(&'a [u64])
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub const fn as_i8(&self) -> Option<&'a [i8]>
        pub const fn as_u16(&self) -> Option<&'a [u16]>
        pub const fn as_u32(&self) -> Option<&'a [u32]>
        pub const fn as_u64(&self) -> Option<&'a [u64]>
        pub fn slice(self, range: Range<usize>) -> Self
        pub fn to_data(&self) -> UnsignedData
        impl<'a, T: UnsignedElement> TryFrom<UnsignedView<'a>> for &'a [T]

image::View
    enum View<'a>: Clone, Copy, Debug, PartialEq
        U8(&'a [u8])
        I16(&'a [i16])
        I32(&'a [i32])
        I64(&'a [i64])
        F32(&'a [f32])
        F64(&'a [f64])
        pub const fn bitpix(&self) -> Bitpix
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn slice(self, range: Range<usize>) -> Self
        pub const fn as_u8(&self) -> Option<&'a [u8]>
        pub const fn as_i16(&self) -> Option<&'a [i16]>
        pub const fn as_i32(&self) -> Option<&'a [i32]>
        pub const fn as_i64(&self) -> Option<&'a [i64]>
        pub const fn as_f32(&self) -> Option<&'a [f32]>
        pub const fn as_f64(&self) -> Option<&'a [f64]>
        pub fn to_data(&self) -> Data
        impl<'a, T: Sample> From<&'a [T]> for View<'a>
        impl<'a, T: Sample> TryFrom<View<'a>> for &'a [T]

io
    mod

io::BLOCK_SIZE
    const BLOCK_SIZE: usize

io::CARD_SIZE
    const CARD_SIZE: usize

io::ChecksumReport
    struct ChecksumReport: Clone, Copy, Debug, Eq, PartialEq
        pub datasum: ChecksumStatus
        pub checksum: ChecksumStatus
        pub const fn is_valid(&self) -> bool
        pub const fn has_invalid(&self) -> bool

io::ChecksumStatus
    enum ChecksumStatus: Clone, Copy, Debug, Eq, PartialEq
        Absent
        Unknown
        Valid
        Invalid

io::GroupParameter
    struct GroupParameter: Clone, Debug, Eq, PartialEq

io::Hdu
    struct Hdu: Debug
        pub index: usize
        pub header: Header
        pub kind: HduKind
        pub data_bytes: u64
        pub fn is_image(&self) -> bool
        pub fn matches<'a>(&self, selector: impl Into<HduSelector<'a>>) -> Result<bool>
        pub fn extname(&self) -> Result<Option<&str>>
        pub fn extver(&self) -> Result<i64>
        pub fn extlevel(&self) -> Result<i64>
        pub const fn padded_data_bytes(&self) -> u64
        pub fn image_metadata(&self) -> Result<Metadata<'_>>
        pub fn table_schema(&self) -> Result<&Schema>
        pub fn ascii_schema(&self) -> Result<&AsciiSchema>

io::HduKind
    #[non_exhaustive]
    enum HduKind: Clone, Copy, Debug, Eq, PartialEq
        Primary
        Image
        AsciiTable
        BinTable
        CompressedImage
        CompressedTable
        RandomGroups
        Other

io::HduSelector
    enum HduSelector<'a>: Clone, Copy, Debug, Eq, From<&'a str>, From<(&'a str, i64)>, From<usize>, PartialEq
        Index(usize)
        Extension { name: &'a str, version: Option<i64>, level: Option<i64> }

io::RandomGroupView
    struct RandomGroupView<'a>: Clone, Copy, Debug, PartialEq
        pub parameters: View<'a>
        pub array: View<'a>

io::RandomGroups
    struct RandomGroups: Clone, Debug
        pub fn metadata(&self) -> RandomGroupsMetadata<'_>
        pub const fn array_len(&self) -> usize
        pub fn group(&self, index: usize) -> Result<RandomGroupView<'_>>
        pub fn parameters_physical_into(&self, group: usize, out: &mut Vec<f64>) -> Result<()>
        pub fn parameter(&self, name: &str) -> Result<GroupParameter>
        pub fn parameter_physical(&self, group: usize, parameter: &GroupParameter) -> Result<f64>
        pub fn array_physical_into<T: PhysicalSample>(&self, group: usize, out: &mut Vec<T>) -> Result<()>

io::RandomGroupsMetadata
    struct RandomGroupsMetadata<'a>: Clone, Copy, Debug
        pub parameter_names: &'a [Option<String>]
        pub group_shape: &'a [usize]
        pub gcount: usize
        pub pcount: usize
        pub bitpix: Bitpix

io::SliceReader
    type SliceReader<'a> = FitsReader<SliceSource<'a>>

io::SliceSource
    struct SliceSource<'a>: Debug

io::StreamReader
    type StreamReader<R> = FitsReader<StreamSource<R>>

io::StreamSource
    struct StreamSource<R>
        impl<R: Debug> Debug for StreamSource<R>

table
    mod

table::Ascii
    struct Ascii: Clone, Debug
        pub const fn schema(&self) -> &AsciiSchema
        pub fn column<'n>(&self, column: impl Into<ColumnSelector<'n>>) -> Result<AsciiColumnReader<'_>>

table::AsciiBuilder
    struct AsciiBuilder: Clone, Debug, Default
        pub const fn new() -> Self
        pub const fn with_rows(row_count: usize) -> Self
        pub fn push(&mut self, column: AsciiWriteColumn) -> Result<&mut Self>
        pub fn with_columns(row_count: usize, columns: impl IntoIterator<Item = AsciiWriteColumn>) -> Result<Self>

table::AsciiColumn
    struct AsciiColumn: Clone, Debug
        pub tform: AsciiTform
        pub const fn byte_offset(&self) -> usize
        pub fn name(&self) -> Result<Option<&str>>
        pub fn unit(&self) -> Result<Option<&str>>
        pub fn tscale(&self) -> Result<f64>
        pub fn tzero(&self) -> Result<f64>
        pub fn tnull(&self) -> Result<Option<&str>>
        pub fn tdisp(&self) -> Result<Option<&str>>

table::AsciiColumnData
    enum AsciiColumnData: Clone, Debug, From<AsciiText>, From<Vec<Option<f64>>>, From<Vec<Option<i64>>>, PartialEq
        Text(AsciiText)
        Integer(Vec<Option<i64>>)
        Float(Vec<Option<f64>>)
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub const fn as_text(&self) -> Option<&AsciiText>
        pub const fn as_integer(&self) -> Option<&[Option<i64>]>
        pub const fn as_float(&self) -> Option<&[Option<f64>]>
        impl TryFrom<AsciiColumnData> for AsciiText
        impl TryFrom<AsciiColumnData> for Vec<Option<f64>>
        impl TryFrom<AsciiColumnData> for Vec<Option<i64>>

table::AsciiColumnReader
    struct AsciiColumnReader<'a>: Clone, Copy, Debug
        pub fn descriptor(&self) -> &'a AsciiColumn
        pub fn raw_into(&self, out: &mut AsciiColumnData) -> Result<()>
        pub fn physical_into<T: PhysicalSample>(&self, out: &mut Vec<T>) -> Result<()>

table::AsciiFloatFormat
    enum AsciiFloatFormat: Clone, Copy, Debug, Eq, PartialEq
        F { width: usize, decimals: usize }
        E { width: usize, decimals: usize }
        D { width: usize, decimals: usize }
        pub const fn width(self) -> usize
        pub const fn decimals(self) -> usize

table::AsciiRows
    struct AsciiRows<'a>: Clone, Debug, ExactSizeIterator, Iterator

table::AsciiSchema
    struct AsciiSchema: Clone, Debug
        pub columns: Vec<AsciiColumn>
        pub fn parse(header: &Header) -> Result<Self>
        pub const fn row_count(&self) -> usize
        pub const fn row_len(&self) -> usize
        pub fn column_index(&self, name: &str) -> Result<Option<usize>>

table::AsciiText
    struct AsciiText: Clone, Debug, Default, Eq, PartialEq, TryFrom<AsciiColumnData>
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn row(&self, r: usize) -> Option<&str>
        pub const fn iter(&self) -> AsciiRows<'_>
        impl From<AsciiText> for AsciiColumnData
        impl<'a> IntoIterator for &'a AsciiText
        impl<S: AsRef<str>> FromIterator<Option<S>> for AsciiText

table::AsciiTform
    enum AsciiTform: Clone, Copy, Debug, Display, Eq, FromStr, PartialEq
        Text { width: usize }
        Integer { width: usize }
        Float(AsciiFloatFormat)
        pub const fn width(self) -> usize

table::AsciiWriteColumn
    struct AsciiWriteColumn: Clone, Debug
        pub fn text(name: impl Into<String>, values: AsciiText, width: usize) -> Self
        pub fn integer(name: impl Into<String>, values: impl IntoIterator<Item = Option<i64>>, width: usize) -> Self
        pub fn float(name: impl Into<String>, values: impl IntoIterator<Item = Option<f64>>, format: AsciiFloatFormat) -> Self
        pub fn with_unit(self, unit: impl Into<String>) -> Self
        pub const fn with_scaling(self, tscale: f64, tzero: f64) -> Self
        pub fn with_null(self, tnull: impl Into<String>) -> Self

table::BitColumn
    struct BitColumn<'a>: Clone, Copy, Debug, Index<(usize, usize)>, Index<usize>, IntoIterator
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn row(&self, r: usize) -> &'a BitSlice<u8, Msb0>
        pub fn get(&self, row: usize, col: usize) -> Option<bool>
        pub const fn iter(&self) -> BitRows<'a>
        impl<'a> IntoIterator for &BitColumn<'a>

table::BitRows
    struct BitRows<'a>: Clone, Debug, ExactSizeIterator, Iterator

table::Builder
    struct Builder: Clone, Debug, Default
        pub const fn new() -> Self
        pub const fn with_rows(row_count: usize) -> Self
        pub fn push(&mut self, column: WriteColumn) -> Result<&mut Self>
        pub fn with_columns(row_count: usize, columns: impl IntoIterator<Item = WriteColumn>) -> Result<Self>

table::CharacterField
    struct CharacterField<'a>: Clone, Copy, Debug, Eq, PartialEq
        pub const fn new(bytes: &'a [u8]) -> Self
        pub const fn bytes(&self) -> &'a [u8]
        pub fn members(&self) -> &'a [u8]
        pub fn is_null(&self) -> bool

table::Column
    struct Column: Clone, Debug
        pub tform: Tform
        pub const fn byte_offset(&self) -> usize
        pub fn name(&self) -> Result<Option<&str>>
        pub fn unit(&self) -> Result<Option<&str>>
        pub fn tscale(&self) -> Result<f64>
        pub fn tzero(&self) -> Result<f64>
        pub fn tnull(&self) -> Result<Option<i64>>
        pub fn tdim(&self) -> Result<Option<&[usize]>>
        pub fn tdisp(&self) -> Result<Option<&str>>

table::ColumnData
    enum ColumnData: Clone, Debug, From<BitVec<u8, Msb0>>, PartialEq, RowValues
        Logical(Vec<Option<bool>>)
        Bit(BitVec<u8, Msb0>)
        U8(Vec<u8>)
        I16(Vec<i16>)
        I32(Vec<i32>)
        I64(Vec<i64>)
        F32(Vec<f32>)
        F64(Vec<f64>)
        ComplexF32(Vec<Complex<f32>>)
        ComplexF64(Vec<Complex<f64>>)
        Character(Vec<u8>)
        pub fn len(&self) -> usize
        pub fn is_empty(&self) -> bool
        pub fn view(&self) -> ColumnView<'_>
        pub const fn as_logical(&self) -> Option<&[Option<bool>]>
        pub fn as_bit(&self) -> Option<&BitSlice<u8, Msb0>>
        pub const fn as_u8(&self) -> Option<&[u8]>
        pub const fn as_i16(&self) -> Option<&[i16]>
        pub const fn as_i32(&self) -> Option<&[i32]>
        pub const fn as_i64(&self) -> Option<&[i64]>
        pub const fn as_f32(&self) -> Option<&[f32]>
        pub const fn as_f64(&self) -> Option<&[f64]>
        pub const fn as_complex_f32(&self) -> Option<&[Complex<f32>]>
        pub const fn as_complex_f64(&self) -> Option<&[Complex<f64>]>
        pub const fn as_character(&self) -> Option<&[u8]>
        impl TryFrom<ColumnData> for BitVec<u8, Msb0>
        impl<T: ColumnElement> From<Vec<T>> for ColumnData
        impl<T: ColumnElement> TryFrom<ColumnData> for Vec<T>

table::ColumnElement
    trait ColumnElement: Element

table::ColumnReader
    struct ColumnReader<'a>: Clone, Copy, Debug
        pub const fn descriptor(&self) -> &'a Column
        pub fn raw_into(&self, out: &mut ColumnData) -> Result<()>
        pub fn physical_into<T: PhysicalSample>(&self, out: &mut Vec<T>) -> Result<()>
        pub fn unsigned_into(&self, out: &mut UnsignedData) -> Result<bool>
        pub fn complex(&self) -> Result<Vec<Complex<f64>>>
        pub fn bits(&self) -> Result<BitColumn<'a>>
        pub fn vla_into(&self, out: &mut Ragged<ColumnData>) -> Result<()>
        pub fn vla_physical_into<T: PhysicalSample>(&self, out: &mut Ragged<Vec<T>>) -> Result<()>
        pub fn vla_unsigned(&self) -> Result<Option<Ragged<UnsignedData>>>
        pub fn vla_complex(&self) -> Result<Ragged<Vec<Complex<f64>>>>
        pub fn vla_bits(&self) -> Result<BitColumn<'a>>

table::ColumnSelector
    enum ColumnSelector<'a>: Clone, Copy, Debug, Eq, From<&'a String>, From<&'a str>, From<&ColumnSelector<'_>>, From<usize>, PartialEq
        Index(usize)
        Name(&'a str)

table::ColumnView
    enum ColumnView<'a>: Clone, Copy, Debug, PartialEq
        Logical(&'a [Option<bool>])
        Bit(&'a BitSlice<u8, Msb0>)
        U8(&'a [u8])
        I16(&'a [i16])
        I32(&'a [i32])
        I64(&'a [i64])
        F32(&'a [f32])
        F64(&'a [f64])
        ComplexF32(&'a [Complex<f32>])
        ComplexF64(&'a [Complex<f64>])
        Character(&'a [u8])
        pub fn len(&self) -> usize
        pub fn is_empty(&self) -> bool
        pub const fn as_logical(&self) -> Option<&'a [Option<bool>]>
        pub const fn as_bit(&self) -> Option<&'a BitSlice<u8, Msb0>>
        pub const fn as_u8(&self) -> Option<&'a [u8]>
        pub const fn as_i16(&self) -> Option<&'a [i16]>
        pub const fn as_i32(&self) -> Option<&'a [i32]>
        pub const fn as_i64(&self) -> Option<&'a [i64]>
        pub const fn as_f32(&self) -> Option<&'a [f32]>
        pub const fn as_f64(&self) -> Option<&'a [f64]>
        pub const fn as_complex_f32(&self) -> Option<&'a [Complex<f32>]>
        pub const fn as_complex_f64(&self) -> Option<&'a [Complex<f64>]>
        pub const fn as_character(&self) -> Option<&'a [u8]>
        pub fn slice(self, range: Range<usize>) -> Self
        pub fn to_data(&self) -> ColumnData
        impl<'a, T: ColumnElement> TryFrom<ColumnView<'a>> for &'a [T]

table::DescriptorWidth
    enum DescriptorWidth: Clone, Copy, Debug, Eq, PartialEq
        P
        Q

table::ElementKind
    enum ElementKind: Clone, Copy, Debug, Eq, PartialEq
        Logical
        Bit
        U8
        I16
        I32
        I64
        Character
        F32
        F64
        ComplexF32
        ComplexF64
        pub const fn code(self) -> char

table::Ragged
    struct Ragged<V>
        impl Ragged<ColumnData>
            pub fn from_rows(rows: impl IntoIterator<Item = ColumnData>) -> Result<Self>
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn range(&self, r: usize) -> Range<usize>
        pub fn ends(&self) -> &[usize]
        pub const fn values(&self) -> &V
        pub fn into_values(self) -> V
        impl<V: RowValues> Ragged<V>
            pub fn new(values: V, ends: Vec<usize>) -> Self
            pub fn row(&self, r: usize) -> V::Row<'_>
            pub const fn iter(&self) -> Rows<'_, V>
        impl<'r, V: RowValues> IntoIterator for &'r Ragged<V>
        impl<T, R: IntoIterator<Item = T>> FromIterator<R> for Ragged<Vec<T>>
        impl<V: Clone> Clone for Ragged<V>
        impl<V: Debug> Debug for Ragged<V>
        impl<V: Default> Default for Ragged<V>
        impl<V: Eq> Eq for Ragged<V>
        impl<V: PartialEq> PartialEq for Ragged<V>

table::RowValues
    trait RowValues
        type Row<'a>
        fn len(&self) -> usize
        fn is_empty(&self) -> bool { .. }
        fn row(&self, range: Range<usize>) -> Self::Row<'_>
        fn is_row_end(&self, end: usize) -> bool { .. }

table::Rows
    struct Rows<'r, V>
        impl<'r, V: Clone> Clone for Rows<'r, V>
        impl<'r, V: Debug> Debug for Rows<'r, V>
        impl<'r, V: RowValues> Iterator for Rows<'r, V>
        impl<V: RowValues> ExactSizeIterator for Rows<'_, V>

table::Schema
    struct Schema: Clone, Debug
        pub columns: Vec<Column>
        pub fn parse(header: &Header) -> Result<Self>
        pub const fn row_count(&self) -> usize
        pub const fn row_len(&self) -> usize
        pub fn column_index(&self, name: &str) -> Result<Option<usize>>

table::SelectedColumn
    struct SelectedColumn: Clone, Debug
        pub descriptor: Column
        pub data: SelectedData

table::SelectedData
    #[non_exhaustive]
    enum SelectedData: Clone, Debug, PartialEq
        Fixed(ColumnData)
        Variable(Ragged<ColumnData>)

table::Selection
    struct Selection: Clone, Debug
        pub rows: Range<usize>
        pub columns: Vec<SelectedColumn>
        pub fn column<'c>(&self, column: impl Into<ColumnSelector<'c>>) -> Result<Option<&SelectedColumn>>

table::Storage
    enum Storage: Clone, Copy, Debug, Eq, PartialEq
        Fixed
        Array(DescriptorWidth)

table::Table
    struct Table: Clone, Debug
        pub const fn schema(&self) -> &Schema
        pub fn column<'n>(&self, column: impl Into<ColumnSelector<'n>>) -> Result<ColumnReader<'_>>

table::Tform
    struct Tform: Clone, Copy, Debug, Display, Eq, FromStr, PartialEq
        pub const fn repeat(self) -> usize
        pub const fn element(self) -> ElementKind
        pub const fn storage(self) -> Storage
        pub const fn code(self) -> char
        pub const fn descriptor_width(self) -> Option<DescriptorWidth>
        pub const fn byte_width(self) -> usize

table::WriteColumn
    struct WriteColumn: Clone, Debug
        pub fn scalar(name: impl Into<String>, data: ColumnData) -> Self
        pub fn fixed(name: impl Into<String>, data: ColumnData, repeat: usize) -> Self
        pub fn characters(name: impl Into<String>, fields: impl IntoIterator<Item = impl AsRef<[u8]>>, width: usize) -> Result<Self>
        pub fn vla(name: impl Into<String>, rows: Ragged<ColumnData>, width: DescriptorWidth) -> Self
        pub fn bits(name: impl Into<String>, rows: impl IntoIterator<Item = impl AsRef<BitSlice<u8, Msb0>>>, width: usize) -> Result<Self>
        pub fn with_unit(self, unit: impl Into<String>) -> Self
        pub fn with_tdim(self, shape: impl Into<Vec<usize>>) -> Self
        pub const fn with_scaling(self, tscale: f64, tzero: f64) -> Self
        pub const fn with_null(self, tnull: i64) -> Self

time
    mod

time::Axis
    struct Axis: Clone, Debug, PartialEq
        pub const fn scale(&self) -> &Scale
        pub fn mjd(&self, value: f64) -> Result<Mjd>

time::Bounds
    struct Bounds<'a>: Clone, Copy, Debug
        pub const fn new(header: &'a Header, frame: &'a Frame) -> Self
        pub fn start(&self) -> Result<Option<Coordinate>>
        pub fn end(&self) -> Result<Option<Coordinate>>
        pub fn mid(&self) -> Result<Option<Coordinate>>
        pub fn exposure(&self) -> Result<Option<f64>>
        pub fn elapsed(&self) -> Result<Option<f64>>
        pub fn resolution(&self) -> Result<Option<f64>>
        pub fn pixel_position(&self) -> Result<f64>
        pub fn systematic_error(&self) -> Result<Option<f64>>
        pub fn random_error(&self) -> Result<Option<f64>>

time::Coordinate
    struct Coordinate: Clone, Debug, PartialEq
        pub mjd: Mjd
        pub scale: Scale
        pub fn observation(header: &Header) -> Result<Option<Self>>
        pub fn epoch(header: &Header) -> Result<Option<Self>>

time::Datetime
    struct Datetime: Clone, Copy, Debug, Display, FromStr, PartialEq
        pub fn date(year: i64, month: u32, day: u32) -> Result<Self>
        pub fn at(self, hour: u32, minute: u32, second: f64) -> Result<Self>
        pub const fn year(self) -> i64
        pub const fn month(self) -> u32
        pub const fn day(self) -> u32
        pub const fn hour(self) -> u32
        pub const fn minute(self) -> u32
        pub const fn second(self) -> u32
        pub const fn second_fraction(self) -> f64
        pub fn to_mjd(self) -> Result<Mjd>

time::Frame
    struct Frame: Clone, Debug
        pub scale: Scale
        pub reference_mjd: Mjd
        pub unit: Unit
        pub clock_offset: f64
        pub reference_position: ReferencePosition
        pub fn from_header(header: &Header) -> Result<Self>
        pub fn from_column(header: &Header, column: usize) -> Result<Self>
        pub fn unit_seconds(&self) -> Result<f64>
        pub fn relative_axis(&self) -> Result<Axis>
        pub fn time_axis(&self, ctype: &str, cunit: &str) -> Result<Option<Axis>>

time::Mjd
    struct Mjd: Clone, Copy, Debug, PartialEq, PartialOrd
        pub fn new(day: i64, fraction: f64) -> Self
        pub const fn day(self) -> i64
        pub const fn fraction(self) -> f64
        pub fn to_f64(self) -> f64

time::PhaseAxis
    struct PhaseAxis: Clone, Copy, Debug, PartialEq
        pub zero_phase: f64
        pub period: Option<f64>
        pub fn from_header(header: &Header, axis: usize, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Option<Self>>
        pub fn from_pixel_list(header: &Header, column: usize, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Option<Self>>
        pub fn from_array_column(header: &Header, axis: usize, column: usize, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Option<Self>>

time::ReferencePosition
    enum ReferencePosition: Clone, Debug, Eq, PartialEq
        Topocenter
        Geocenter
        Barycenter
        Relocatable
        Custom
        Heliocenter
        GalacticCenter
        EarthMoonBarycenter
        Mercury
        Venus
        Mars
        Jupiter
        Saturn
        Uranus
        Neptune
        Other(String)

time::Scale
    enum Scale: Clone, Debug, Display, Eq, FromStr, PartialEq
        Known { kind: ScaleKind, realization: Option<String> }
        Local(String)
        pub const fn known(kind: ScaleKind) -> Self

time::ScaleKind
    enum ScaleKind: Clone, Copy, Debug, Display, Eq, PartialEq
        Utc
        Ut1
        Tai
        Tt
        Tcg
        Tdb
        Tcb
        Gps

time::Unit
    struct Unit: Clone, Copy, Debug, Display, FromStr, PartialEq
        pub fn seconds(self, reference_mjd: Mjd, scale: &Scale) -> Result<f64>

wcs
    mod

wcs::Alternate
    struct Alternate: Clone, Copy, Debug, Display, Eq, Hash, PartialEq, TryFrom<Option<char>>, TryFrom<char>
        pub const PRIMARY: Self
        pub const fn new(letter: char) -> Self
        pub const fn letter(self) -> Option<char>

wcs::AxisDescription
    struct AxisDescription: Clone, Debug, PartialEq
        pub ctype: String
        pub cunit: String
        pub crval: f64
        pub crpix: f64
        pub spectral_frame: Option<SpectralFrame>

wcs::AxisWorld
    struct AxisWorld<'a>: Clone, Copy, Debug, PartialEq
        pub unit: &'a str
        pub value: f64

wcs::CelestialFrame
    struct CelestialFrame: Clone, Debug, PartialEq
        pub reference_frame: CelestialReferenceFrame
        pub equinox: Option<f64>

wcs::CelestialPole
    struct CelestialPole: Clone, Copy, Debug, PartialEq
        pub ra: f64
        pub dec: f64
        pub lonpole: f64

wcs::CelestialProjection
    struct CelestialProjection: Clone, Copy, Debug, PartialEq
        pub longitude_axis: usize
        pub latitude_axis: usize
        pub projection: Projection
        pub pole: CelestialPole

wcs::CelestialReferenceFrame
    enum CelestialReferenceFrame: Clone, Debug, Eq, PartialEq
        Icrs
        Fk5
        Fk4
        Fk4NoE
        Gappt
        Other(String)

wcs::Description
    struct Description<'a>: Clone, Copy, Debug
        pub axes: &'a [AxisDescription]
        pub celestial_frame: Option<&'a CelestialFrame>
        pub celestial_projection: Option<CelestialProjection>
        pub unsupported_axes: &'a [usize]
        pub distortion: Option<Distortion>

wcs::Distortion
    enum Distortion: Clone, Copy, Debug, Eq, PartialEq
        Sip
        Tpv

wcs::Projection
    enum Projection: Clone, Copy, Debug, Display, Eq, PartialEq
        Tan
        Sin
        Arc
        Stg
        Zea
        Car
        Cea
        Mer
        Sfl
        Ait
        Mol
        Zpn
        Cyp
        Par
        Cop
        Coe
        Cod
        Coo
        Bon
        Air
        Azp
        Pco
        Szp
        Tsc
        Csc
        Qsc
        Hpx
        pub const fn code(self) -> &'static str

wcs::SpectralFrame
    struct SpectralFrame: Clone, Debug, PartialEq
        pub coordinate: Option<SpectralReferenceFrame>
        pub observer: SpectralReferenceFrame
        pub rest: SpectralRest

wcs::SpectralReferenceFrame
    enum SpectralReferenceFrame: Clone, Debug, Eq, PartialEq
        Topocentric
        Geocentric
        Barycentric
        Heliocentric
        LsrKinematic
        LsrDynamic
        Galactocentric
        LocalGroup
        CmbDipole
        Source
        Other(String)

wcs::SpectralRest
    struct SpectralRest: Clone, Copy, Debug, PartialEq
        pub frequency: Option<f64>
        pub wavelength: Option<f64>

wcs::Wcs
    struct Wcs: Clone, Debug
        pub fn from_header(header: &Header, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Self>
        pub fn crop(&self, region: Region<'_>) -> Result<Self>
        pub fn bin(&self, factors: &[usize]) -> Result<Self>
        pub fn write_into(&self, header: &mut Header, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<()>
        pub fn description(&self) -> Description<'_>
        pub fn world_unit(&self, axis: usize) -> Result<&str>
        pub fn axis_world(&self, axis: usize, pixel: &[f64]) -> Result<AxisWorld<'_>>
        pub fn from_pixel_list(header: &Header, columns: &[usize], alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Self>
        pub fn from_array_column(header: &Header, column: usize, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Self>
        pub fn pixel_to_world(&self, pixel: &[f64], world: &mut [f64]) -> Result<()>
        pub fn world_to_pixel(&self, world: &[f64], pixel: &mut [f64]) -> Result<()>

```

## Public but not exported

Items declared `pub` that no path from the crate root reaches. A caller cannot name
them; each one is either dead surface or a type that leaks through a signature.

```text
trait      fits_well::reader::source::Source
```
