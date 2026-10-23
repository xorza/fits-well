# Public API surface

Every item the crate exports, rendered as its declaration from rustdoc JSON
(`cargo drive surface`), sorted by path. The surface is the union of a build with
no features and one build per public feature; a tag names the builds an item needs or
loses. `internals` is the crate's own test surface and is left out.

```text
FitsEditor
    struct FitsEditor<F>
        impl<F: Read + Write + Seek> FitsEditor<F>
            pub fn open(file: F) -> Result<Self>
            pub fn hdus(&self) -> &[Hdu]
            pub fn into_inner(self) -> F
            pub fn replace_header<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, header: &Header) -> Result<()>
            pub fn replace_header_into<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, header: &Header, sink: impl Write) -> Result<()>
            pub fn append(self) -> Result<FitsWriter<F>>
        impl<F: Debug> Debug for FitsEditor<F>

FitsError
    #[non_exhaustive]
    enum FitsError: Debug, Display, Error, From<Error>, From<Infallible>
        Io(Error)
        WriterFailed
        PrimaryAlreadyWritten
        InvalidKeyword { name: String }
        InvalidAlternate { alternate: char }
        ReservedKeyword { keyword: Keyword }
        InvalidHeaderValue { keyword: Keyword, reason: &'static str }
        HeaderCardTooLong { keyword: String, length: usize }
        InvalidValue { card: String }
        InvalidWcs { detail: String }
        InvalidTime { detail: String }
        InvalidCompressionParameter { detail: String }
        InvalidAsciiField { field: String }
        InvalidAsciiValue { reason: &'static str }
        ExternalTimeDataRequired { operation: &'static str }
        TypeMismatch { name: String, expected: &'static str }
        InvalidAscii { context: &'static str }
        AsciiFieldTooWide { column: String, row: usize, width: usize, minimum_width: usize }
        CharacterFieldTooWide { column: String, row: usize, length: usize, width: usize }
        BitFieldLengthMismatch { column: String, row: usize, length: usize, width: usize }
        InvalidBitpix { code: i64 }
        MissingEnd
        MissingKeyword { keyword: Keyword }
        KeywordOutOfRange { keyword: Keyword }
        IntegerOutOfRange { value: String, target: &'static str }
        UnexpectedEof
        DataUnitOverflow
        DataUnitTooLarge { bytes: u64 }
        DataSizeMismatch { expected: usize, got: usize }
        InvalidPqDescriptor { field: &'static str, value: i64 }
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
        FileOffBlockGrid { len: u64 }
        HduNotFound { name: String, version: Option<i64>, level: Option<i64> }
        NotAnImage
        ImageHasGroups
        NotABinTable
        NotATimeAxis { axis: usize }
        NotRandomGroups
        NotAnAsciiTable
        ConflictingWcsKeywords { detail: &'static str }
        UnsupportedWcsTransform { axes: Vec<usize> }
        WcsPixelOutOfDomain { projection: &'static str }
        WcsWorldOutOfDomain { projection: &'static str }
        WcsInvalidPole { detail: &'static str }
        InvalidUnit { unit: String, expected: &'static str }
        WcsCoordinateOutOfDomain { axis: usize, algorithm: &'static str }
        WcsNoConvergence { algorithm: &'static str }
        HcompressTileTooSmall { axis: usize, length: usize }
        UnsupportedCompression { name: String }
        CorruptCompressedData { detail: String }
        PlioValueOutOfRange { index: usize, value: i64 }
        InvalidTform { tform: String }
        NotAFixedColumn { code: char }
        NotAVla { code: char }
        NotABitColumn { code: char }
        NotAComplexColumn { code: char }
        NotANumericColumn { code: char }
        ColumnNotFound { name: String }
        GroupParameterNotFound { name: String }
        RowWidthMismatch { computed: usize, declared: usize }
        TableMetadataMismatch { name: String }

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
            pub fn images(&self) -> impl Iterator<Item = &Hdu> + '_
            pub fn read_data_unit<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<DataUnit>
            pub fn image<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<ImageReader<'_, S>>
            pub fn read_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Table>
            pub fn read_table_rows<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, rows: Range<usize>) -> Result<Table>
            pub fn read_table_columns<'h, 'c>(&mut self, hdu: impl Into<HduSelector<'h>>, rows: Range<usize>, columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>) -> Result<Selection>
            pub fn read_table_columns_into<'h, 'c>(&mut self, hdu: impl Into<HduSelector<'h>>, rows: Range<usize>, columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>, out: &mut Selection) -> Result<()>
            pub fn read_table_cell<'h, 'c>(&mut self, hdu: impl Into<HduSelector<'h>>, row: usize, column: impl Into<ColumnSelector<'c>>) -> Result<ColumnData>
            pub fn read_wcs<'h>(&mut self, hdu: impl Into<HduSelector<'h>>, alt: impl TryInto<Alternate, Error: Into<FitsError>>) -> Result<Wcs>
            pub fn read_ascii_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Ascii>
            pub fn read_groups<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<RandomGroups>
            pub fn verify_checksum<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<ChecksumReport>
        impl<S: Debug> Debug for FitsReader<S>

FitsWriter  [shape differs by feature]
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
        impl<W: Write + Seek> FitsWriter<W>
            pub fn stream_image(&mut self, layout: Metadata<'_>, header: Option<&Header>) -> Result<ImageWriter<'_, W>>
        impl<W: Debug> Debug for FitsWriter<W>

ImageReader
    struct ImageReader<'r, S>
        impl<'r, S: Source> ImageReader<'r, S>
            pub const fn header(&self) -> &'r Header
            pub const fn metadata(&self) -> Metadata<'r>
            pub const fn with_checksums(self) -> Self
            pub fn view<'a, 'g>(&'a mut self, region: impl Into<Region<'g>>, words: &'a mut Vec<u64>) -> Result<Borrowed<'a>>
            pub fn read<'g>(&mut self, region: impl Into<Region<'g>>) -> Result<Image>
            pub fn physical_into<'g, T: PhysicalSample>(&mut self, region: impl Into<Region<'g>>, out: &mut Vec<T>) -> Result<()>
            pub fn finish(self) -> Result<ChecksumReport>
        impl<'r, S: Debug> Debug for ImageReader<'r, S>

ImageWriter
    struct ImageWriter<'a, W: Write + Seek>: Drop
        pub fn write_chunk(&mut self, samples: View<'_>) -> Result<()>
        pub fn finish(self) -> Result<()>
        impl<'a, W: Debug + Write + Seek> Debug for ImageWriter<'a, W>

Indexed
    #[non_exhaustive]
    enum Indexed: Clone, Copy, Debug, Eq, PartialEq
        Hdu
        HeaderRecord
        Column
        Group
        WcsAxis

Ranked
    #[non_exhaustive]
    enum Ranked: Clone, Copy, Debug, Eq, PartialEq
        ImageRegion
        TileShape
        WcsCoordinate
        BinFactors

Result
    type Result<T> = Result<T, FitsError>

header
    mod

header::Defect
    enum Defect: Clone, Copy, Debug, Eq, PartialEq
        Name
        Value

header::FitsInteger
    struct FitsInteger: Clone, Debug, Display, Eq, From<i128>, From<i32>, From<i64>, From<u128>, From<u16>, From<u32>, From<u64>, From<usize>, Hash, PartialEq
        impl From<FitsInteger> for Value
        impl TryFrom<&FitsInteger> for i64

header::Header
    struct Header: Clone, Debug, Default
        pub fn parse(bytes: &[u8]) -> Result<Self>
        pub fn repairs(&self) -> &[Repair]
        pub fn get(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<&Value>>
        pub fn get_comment(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<&str>>
        pub fn get_logical(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<bool>>
        pub fn get_integer(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<i64>>
        pub fn get_real(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<f64>>
        pub fn get_text(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<&str>>
        pub fn require_logical(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<bool>
        pub fn require_integer(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<i64>
        pub fn require_real(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<f64>
        pub fn require_text(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<&str>
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn contains(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<bool>
        pub fn index_of(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<usize>>
        pub fn iter(&self) -> Iter<'_>
        pub fn history(&self) -> impl Iterator<Item = &str>
        pub fn comments(&self) -> impl Iterator<Item = &str>
        pub fn new() -> Self
        pub fn set(&mut self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn set_optional(&mut self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>, value: Option<impl Into<Value>>) -> Result<&mut Self>
        pub fn append(&mut self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn insert(&mut self, index: usize, keyword: impl TryInto<Keyword, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn remove(&mut self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Option<Record>>
        pub fn remove_at(&mut self, index: usize) -> Result<Record>
        pub fn remove_all(&mut self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<usize>
        pub fn get_hierarch(&self, name: impl TryInto<HierarchName, Error: Into<FitsError>>) -> Result<Option<&Value>>
        pub fn set_hierarch(&mut self, name: impl TryInto<HierarchName, Error: Into<FitsError>>, value: impl Into<Value>) -> Result<&mut Self>
        pub fn set_comment(&mut self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>, text: &str) -> Result<&mut Self>
        pub fn push_comment(&mut self, text: &str) -> Result<&mut Self>
        pub fn push_history(&mut self, text: &str) -> Result<&mut Self>
        pub fn push_blank(&mut self, text: &str) -> Result<&mut Self>
        impl<'a> IntoIterator for &'a Header

header::HierarchName
    struct HierarchName: Clone, Debug, Display, Eq, FromStr, Hash, PartialEq, TryFrom<&str>
        pub fn as_str(&self) -> &str

header::Iter
    struct Iter<'a>: Clone, Debug, ExactSizeIterator, Iterator

header::Keyword
    struct Keyword: Clone, Copy, Debug, Display, Eq, Hash, PartialEq, PartialEq<&str>, PartialEq<str>, TryFrom<&String>, TryFrom<&str>, TryFrom<Arguments<'_>>, TryFrom<String>
        pub const fn new(name: &str) -> Self
        pub fn indexed(self, index: usize) -> Result<Self>
        pub const fn as_str(&self) -> &str

header::Record
    enum Record: Clone, Debug, PartialEq
        Valued { keyword: Keyword, value: Value, comment: Option<String> }
        Commentary { name: String, text: Option<String> }
        Hierarch { name: HierarchName, value: Value, comment: Option<String> }
        Malformed { keyword: Option<Keyword>, raw: [u8; 80], defect: Defect }

header::RecordView
    enum RecordView<'a>: Clone, Copy, Debug, PartialEq
        Valued { keyword: &'a Keyword, value: &'a Value, comment: Option<&'a str> }
        Commentary { name: &'a str, text: Option<&'a str> }
        Hierarch { name: &'a HierarchName, value: &'a Value, comment: Option<&'a str> }
        Malformed { keyword: Option<&'a Keyword>, raw: &'a [u8; 80], defect: Defect }
        pub const fn name(self) -> &'a str

header::Repair
    struct Repair: Clone, Copy, Debug, Eq, PartialEq
        pub record: usize
        pub kind: RepairKind

header::RepairKind
    enum RepairKind: Clone, Copy, Debug, Eq, PartialEq
        LowercaseName
        ValueIndicator
        NonAsciiText

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

image
    mod

image::Bitpix
    enum Bitpix: Clone, Copy, Debug, Eq, Hash, PartialEq
        U8
        I16
        I32
        I64
        F32
        F64
        pub const fn from_code(code: i64) -> Result<Self>
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
        pub fn unsigned(&self) -> Option<UnsignedData>
        pub fn sample_type(&self) -> SampleType
        pub fn to_image(&self) -> Image

image::Compression  [features: compression or parallel]
    enum Compression: Clone, Copy, Debug, PartialEq
        Gzip(Gzip)
        Rice
        Plio
        Hcompress(Hcompress)
        None
        pub const GZIP: Self
        pub const GZIP_SHUFFLED: Self
        pub const fn name(self) -> &'static str

image::CompressionConfig  [features: compression or parallel]
    struct CompressionConfig: Clone, Debug, PartialEq
        pub const fn new(compression: Compression) -> Self
        pub fn with_tiles(self, tiles: impl Into<Vec<usize>>) -> Self
        pub const fn with_quantization(self, quantization: Quantization) -> Self

image::Data
    enum Data: Clone, Debug, From<Vec<f32>>, From<Vec<f64>>, From<Vec<i16>>, From<Vec<i32>>, From<Vec<i64>>, From<Vec<u8>>, PartialEq
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
        impl TryFrom<Data> for Vec<f32>
        impl TryFrom<Data> for Vec<f64>
        impl TryFrom<Data> for Vec<i16>
        impl TryFrom<Data> for Vec<i32>
        impl TryFrom<Data> for Vec<i64>
        impl TryFrom<Data> for Vec<u8>

image::Dither  [features: compression or parallel]
    enum Dither: Clone, Copy, Debug, Default, Eq, PartialEq
        None
        Subtractive1(DitherSeed)
        Subtractive2(DitherSeed)

image::DitherSeed  [features: compression or parallel]
    struct DitherSeed: Clone, Copy, Debug, Eq, PartialEq
        pub const FROM_SAMPLES: Self
        pub fn fixed(seed: u16) -> Result<Self>

image::Gzip  [features: compression or parallel]
    struct Gzip: Clone, Copy, Debug, Default, Eq, PartialEq
        pub fn new(level: u32) -> Result<Self>
        pub fn shuffled(level: u32) -> Result<Self>

image::Hcompress  [features: compression or parallel]
    struct Hcompress: Clone, Copy, Debug, Default, PartialEq
        pub fn relative(scale: f64) -> Result<Self>
        pub fn absolute(scale: f64) -> Result<Self>

image::Image
    struct Image: Clone, Debug
        pub fn new(shape: impl Into<Vec<usize>>, samples: impl Into<Data>, scaling: Scaling) -> Result<Self>
        pub fn view(&self) -> Borrowed<'_>
        pub fn into_samples(self) -> Data
        pub fn from_u16(shape: impl Into<Vec<usize>>, data: &[u16]) -> Result<Self>
        pub fn from_u32(shape: impl Into<Vec<usize>>, data: &[u32]) -> Result<Self>
        pub fn from_u64(shape: impl Into<Vec<usize>>, data: &[u64]) -> Result<Self>
        pub fn from_i8(shape: impl Into<Vec<usize>>, data: &[i8]) -> Result<Self>

image::Metadata
    struct Metadata<'a>: Clone, Copy, Debug, PartialEq
        pub shape: &'a [usize]
        pub bitpix: Bitpix
        pub scaling: Scaling

image::PhysicalSample
    trait PhysicalSample: Narrow

image::Quantization  [features: compression or parallel]
    struct Quantization: Clone, Copy, Debug, Default, PartialEq
        pub const NONE: Self
        pub fn relative(level: f64, dither: Dither) -> Result<Self>
        pub fn absolute(step: f64, dither: Dither) -> Result<Self>

image::Region
    enum Region<'a>: Clone, Copy, Debug, Eq, From<&'a [Range<usize>]>, PartialEq
        All
        Section(&'a [Range<usize>])
        impl<'a, const N: usize> From<&'a [Range<usize>; N]> for Region<'a>

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
    enum UnsignedData: Clone, Debug, Eq, PartialEq
        I8(Vec<i8>)
        U16(Vec<u16>)
        U32(Vec<u32>)
        U64(Vec<u64>)

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
        pub const fn as_u8(&self) -> Option<&'a [u8]>
        pub const fn as_i16(&self) -> Option<&'a [i16]>
        pub const fn as_i32(&self) -> Option<&'a [i32]>
        pub const fn as_i64(&self) -> Option<&'a [i64]>
        pub const fn as_f32(&self) -> Option<&'a [f32]>
        pub const fn as_f64(&self) -> Option<&'a [f64]>
        pub fn to_data(&self) -> Data

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

io::DataUnit
    struct DataUnit: Clone, Debug
        pub fn view(&self) -> DataUnitView<'_>
        pub fn data(&self) -> &[u8]
        pub fn into_data(self) -> Vec<u8>
        pub fn into_padded(self) -> Vec<u8>

io::DataUnitView
    struct DataUnitView<'a>: Clone, Debug
        pub padded: &'a [u8]
        pub data_range: Range<usize>

io::Hdu
    struct Hdu: Debug
        pub index: usize
        pub header: Header
        pub kind: HduKind
        pub data_bytes: u64
        pub fn is_image(&self) -> bool
        pub fn matches<'a>(&self, selector: impl Into<HduSelector<'a>>) -> Result<bool>
        pub const fn padded_data_bytes(&self) -> u64
        pub fn image_metadata(&self) -> Result<Metadata<'_>>
        pub fn table_schema(&self) -> Result<&Schema>

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
        pub fn parameters_physical(&self, group: usize) -> Result<Vec<f64>>
        pub fn parameter_physical(&self, group: usize, name: &str) -> Result<f64>
        pub fn array_physical(&self, group: usize) -> Result<Vec<f64>>

io::RandomGroupsMetadata
    struct RandomGroupsMetadata<'a>: Clone, Copy, Debug
        pub parameter_names: &'a [String]
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
        pub name: Option<String>
        pub unit: Option<String>
        pub kind: AsciiKind
        pub start: usize
        pub width: usize
        pub decimals: usize
        pub tscale: f64
        pub tzero: f64
        pub tnull: Option<String>

table::AsciiColumnData
    enum AsciiColumnData: Clone, Debug, PartialEq
        Text(AsciiText)
        Integer(Vec<Option<i64>>)
        Float(Vec<Option<f64>>)
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool

table::AsciiColumnReader
    struct AsciiColumnReader<'a>: Clone, Copy, Debug
        pub fn descriptor(&self) -> &'a AsciiColumn
        pub fn raw(&self) -> Result<AsciiColumnData>
        pub fn physical(&self) -> Result<Vec<f64>>

table::AsciiKind
    enum AsciiKind: Clone, Copy, Debug, Eq, PartialEq
        Text
        Integer
        Float

table::AsciiSchema
    struct AsciiSchema: Clone, Debug
        pub row_count: usize
        pub row_len: usize
        pub columns: Vec<AsciiColumn>
        pub fn column_index(&self, name: &str) -> Option<usize>

table::AsciiText
    struct AsciiText: Clone, Debug, Default, Eq, PartialEq
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn get(&self, r: usize) -> Option<&str>
        pub fn iter(&self) -> impl ExactSizeIterator<Item = Option<&str>>
        impl<S: AsRef<str>> FromIterator<Option<S>> for AsciiText

table::AsciiWriteColumn
    struct AsciiWriteColumn: Clone, Debug
        pub fn new(name: impl Into<String>, data: AsciiColumnData, width: usize) -> Self
        pub fn with_unit(self, unit: impl Into<String>) -> Self
        pub const fn with_decimals(self, decimals: usize) -> Self
        pub const fn with_scaling(self, tscale: f64, tzero: f64) -> Self
        pub fn with_null(self, tnull: impl Into<String>) -> Self

table::BitColumn
    struct BitColumn<'a>: Clone, Copy, Debug, Index<(usize, usize)>, Index<usize>
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn row(&self, r: usize) -> &'a BitSlice<u8, Msb0>
        pub fn get(&self, row: usize, col: usize) -> Option<bool>
        pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a BitSlice<u8, Msb0>> + '_

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
        pub name: Option<String>
        pub unit: Option<String>
        pub tform: Tform
        pub tscale: f64
        pub tzero: f64
        pub tnull: Option<i64>
        pub tdim: Option<Vec<usize>>
        pub tdisp: Option<String>
        pub const fn byte_offset(&self) -> usize

table::ColumnData
    enum ColumnData: Clone, Debug, From<Vec<Complex<f32>>>, From<Vec<Complex<f64>>>, From<Vec<Option<bool>>>, From<Vec<f32>>, From<Vec<f64>>, From<Vec<i16>>, From<Vec<i32>>, From<Vec<i64>>, From<Vec<u8>>, PartialEq
        Logical(Vec<Option<bool>>)
        U8(Vec<u8>)
        I16(Vec<i16>)
        I32(Vec<i32>)
        I64(Vec<i64>)
        F32(Vec<f32>)
        F64(Vec<f64>)
        ComplexF32(Vec<Complex<f32>>)
        ComplexF64(Vec<Complex<f64>>)
        Character(Vec<u8>)
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub const fn view(&self) -> ColumnView<'_>
        pub const fn as_logical(&self) -> Option<&[Option<bool>]>
        pub const fn as_u8(&self) -> Option<&[u8]>
        pub const fn as_i16(&self) -> Option<&[i16]>
        pub const fn as_i32(&self) -> Option<&[i32]>
        pub const fn as_i64(&self) -> Option<&[i64]>
        pub const fn as_f32(&self) -> Option<&[f32]>
        pub const fn as_f64(&self) -> Option<&[f64]>
        pub const fn as_complex_f32(&self) -> Option<&[Complex<f32>]>
        pub const fn as_complex_f64(&self) -> Option<&[Complex<f64>]>
        pub const fn as_character(&self) -> Option<&[u8]>
        impl TryFrom<ColumnData> for Vec<Complex<f32>>
        impl TryFrom<ColumnData> for Vec<Complex<f64>>
        impl TryFrom<ColumnData> for Vec<Option<bool>>
        impl TryFrom<ColumnData> for Vec<f32>
        impl TryFrom<ColumnData> for Vec<f64>
        impl TryFrom<ColumnData> for Vec<i16>
        impl TryFrom<ColumnData> for Vec<i32>
        impl TryFrom<ColumnData> for Vec<i64>
        impl TryFrom<ColumnData> for Vec<u8>

table::ColumnReader
    struct ColumnReader<'a>: Clone, Copy, Debug
        pub const fn descriptor(&self) -> &'a Column
        pub fn raw(&self) -> Result<ColumnData>
        pub fn physical(&self) -> Result<Vec<f64>>
        pub fn unsigned(&self) -> Result<Option<UnsignedData>>
        pub fn complex(&self) -> Result<Vec<Complex<f64>>>
        pub fn bits(&self) -> Result<BitColumn<'a>>
        pub fn vla(&self) -> Result<Ragged<ColumnData>>
        pub fn vla_physical(&self) -> Result<Ragged<Vec<f64>>>
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
        U8(&'a [u8])
        I16(&'a [i16])
        I32(&'a [i32])
        I64(&'a [i64])
        F32(&'a [f32])
        F64(&'a [f64])
        ComplexF32(&'a [Complex<f32>])
        ComplexF64(&'a [Complex<f64>])
        Character(&'a [u8])
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub const fn as_logical(&self) -> Option<&'a [Option<bool>]>
        pub const fn as_u8(&self) -> Option<&'a [u8]>
        pub const fn as_i16(&self) -> Option<&'a [i16]>
        pub const fn as_i32(&self) -> Option<&'a [i32]>
        pub const fn as_i64(&self) -> Option<&'a [i64]>
        pub const fn as_f32(&self) -> Option<&'a [f32]>
        pub const fn as_f64(&self) -> Option<&'a [f64]>
        pub const fn as_complex_f32(&self) -> Option<&'a [Complex<f32>]>
        pub const fn as_complex_f64(&self) -> Option<&'a [Complex<f64>]>
        pub const fn as_character(&self) -> Option<&'a [u8]>
        pub fn to_data(&self) -> ColumnData

table::Ragged
    struct Ragged<V>
        pub const fn len(&self) -> usize
        pub const fn is_empty(&self) -> bool
        pub fn range(&self, r: usize) -> Range<usize>
        pub fn ends(&self) -> &[usize]
        pub const fn values(&self) -> &V
        pub fn into_values(self) -> V
        impl<T> Ragged<Vec<T>>
            pub fn new(values: Vec<T>, ends: Vec<usize>) -> Self
            pub fn row(&self, r: usize) -> &[T]
            pub fn rows(&self) -> impl ExactSizeIterator<Item = &[T]>
        impl Ragged<String>
            pub fn new(values: String, ends: Vec<usize>) -> Self
            pub fn row(&self, r: usize) -> &str
            pub fn rows(&self) -> impl ExactSizeIterator<Item = &str>
        impl Ragged<BitVec<u8, Msb0>>
            pub fn new(values: BitVec<u8, Msb0>, ends: Vec<usize>) -> Self
            pub fn row(&self, r: usize) -> &BitSlice<u8, Msb0>
            pub fn rows(&self) -> impl ExactSizeIterator<Item = &BitSlice<u8, Msb0>>
        impl Ragged<ColumnData>
            pub fn row(&self, r: usize) -> ColumnView<'_>
            pub fn rows(&self) -> impl ExactSizeIterator<Item = ColumnView<'_>>
            pub fn new(values: ColumnData, ends: Vec<usize>) -> Self
            pub fn from_rows(rows: impl IntoIterator<Item = ColumnData>) -> Result<Self>
        impl<B: AsRef<BitSlice<u8, Msb0>>> FromIterator<B> for Ragged<BitVec<u8, Msb0>>
        impl<S: AsRef<str>> FromIterator<S> for Ragged<String>
        impl<T, R: IntoIterator<Item = T>> FromIterator<R> for Ragged<Vec<T>>
        impl<V: Clone> Clone for Ragged<V>
        impl<V: Debug> Debug for Ragged<V>
        impl<V: Default> Default for Ragged<V>
        impl<V: Eq> Eq for Ragged<V>
        impl<V: PartialEq> PartialEq for Ragged<V>

table::Schema
    struct Schema: Clone, Debug
        pub row_count: usize
        pub row_len: usize
        pub columns: Vec<Column>
        pub fn parse(header: &Header) -> Result<Self>
        pub fn column_index(&self, name: &str) -> Option<usize>

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
        pub fn column<'c>(&self, column: impl Into<ColumnSelector<'c>>) -> Option<&SelectedColumn>

table::Table
    struct Table: Clone, Debug
        pub const fn schema(&self) -> &Schema
        pub fn column<'n>(&self, column: impl Into<ColumnSelector<'n>>) -> Result<ColumnReader<'_>>

table::Tform
    struct Tform: Clone, Copy, Debug, Eq, FromStr, PartialEq
        pub repeat: usize
        pub kind: TformKind
        pub vla_elem: Option<TformKind>
        pub const fn byte_width(self) -> usize

table::TformKind
    enum TformKind: Clone, Copy, Debug, Eq, PartialEq
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
        ArrayDesc32
        ArrayDesc64
        pub const fn code(self) -> char

table::WriteColumn
    struct WriteColumn: Clone, Debug
        pub fn scalar(name: impl Into<String>, data: ColumnData) -> Self
        pub fn fixed(name: impl Into<String>, data: ColumnData, repeat: usize) -> Self
        pub fn characters(name: impl Into<String>, fields: impl IntoIterator<Item = impl AsRef<[u8]>>, width: usize) -> Result<Self>
        pub fn vla(name: impl Into<String>, rows: Ragged<ColumnData>) -> Self
        pub fn bits(name: impl Into<String>, rows: impl IntoIterator<Item = impl AsRef<BitSlice<u8, Msb0>>>, width: usize) -> Result<Self>
        pub fn vla_bits(name: impl Into<String>, rows: Ragged<BitVec<u8, Msb0>>) -> Self
        pub fn with_unit(self, unit: impl Into<String>) -> Self
        pub fn with_tdim(self, shape: impl Into<Vec<usize>>) -> Self
        pub fn with_wide_descriptors(self) -> Self
        pub const fn with_scaling(self, tscale: f64, tzero: f64) -> Self
        pub const fn with_null(self, tnull: i64) -> Self

time
    mod

time::Bounds
    struct Bounds: Clone, Copy, Debug, PartialEq
        pub start_mjd: Option<Mjd>
        pub end_mjd: Option<Mjd>
        pub mid_mjd: Option<Mjd>
        pub exposure: Option<f64>
        pub elapsed: Option<f64>
        pub resolution: Option<f64>
        pub pixel_position: f64
        pub systematic_error: Option<f64>
        pub random_error: Option<f64>
        pub fn from_header(header: &Header) -> Result<Self>

time::Coordinate
    struct Coordinate: Clone, Debug, PartialEq
        pub mjd: Mjd
        pub scale: Scale
        pub fn observation(header: &Header) -> Result<Option<Self>>
        pub fn epoch(header: &Header) -> Result<Option<Self>>

time::Datetime
    struct Datetime: Clone, Copy, Debug, FromStr, PartialEq
        pub year: i64
        pub month: u32
        pub day: u32
        pub hour: u32
        pub minute: u32
        pub second: f64
        pub fn to_mjd(&self, scale: &Scale) -> Result<Mjd>

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
        pub fn relative_to_mjd(&self, value: f64) -> Result<Mjd>
        pub fn time_axis_mjd(&self, wcs: &Wcs, axis: usize, pixel: &[f64]) -> Result<Coordinate>

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
    enum Scale: Clone, Debug, Eq, FromStr, PartialEq
        Known { kind: ScaleKind, realization: Option<String> }
        Local(String)
        pub const fn known(kind: ScaleKind) -> Self

time::ScaleKind
    enum ScaleKind: Clone, Copy, Debug, Eq, PartialEq
        Utc
        Ut1
        Tai
        Tt
        Tcg
        Tdb
        Tcb
        Gps

time::Unit
    enum Unit: Clone, Copy, Debug, FromStr, PartialEq
        Seconds(f64)
        TropicalYears(f64)
        BesselianYears(f64)
        pub fn seconds(self, reference_mjd: Mjd, scale: &Scale) -> Result<f64>

wcs
    mod

wcs::Alternate
    struct Alternate: Clone, Copy, Debug, Display, Eq, Hash, PartialEq, TryFrom<Option<char>>, TryFrom<char>
        pub const PRIMARY: Self
        pub const fn new(letter: char) -> Self
        pub const fn letter(self) -> Option<char>

wcs::Axis
    struct Axis: Clone, Debug, PartialEq
        pub ctype: String
        pub cunit: String
        pub crval: f64
        pub crpix: f64
        pub spectral_frame: Option<SpectralFrame>

wcs::CelestialFrame
    struct CelestialFrame: Clone, Copy, Debug, PartialEq
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
    enum CelestialReferenceFrame: Clone, Copy, Debug, Eq, PartialEq
        Icrs
        Fk5
        Fk4
        Fk4NoE
        Gappt

wcs::Distortion
    enum Distortion: Clone, Copy, Debug, Eq, PartialEq
        Sip
        Tpv

wcs::Metadata
    struct Metadata<'a>: Clone, Copy, Debug
        pub axes: &'a [Axis]
        pub celestial_frame: Option<CelestialFrame>
        pub celestial_projection: Option<CelestialProjection>
        pub unsupported_axes: &'a [usize]
        pub distortion: Option<Distortion>

wcs::Projection
    enum Projection: Clone, Copy, Debug, Eq, PartialEq
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

wcs::SpectralFrame
    struct SpectralFrame: Clone, Copy, Debug, PartialEq
        pub coordinate: Option<SpectralReferenceFrame>
        pub observer: SpectralReferenceFrame
        pub rest: SpectralRest

wcs::SpectralReferenceFrame
    enum SpectralReferenceFrame: Clone, Copy, Debug, Eq, PartialEq
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
        pub fn metadata(&self) -> Metadata<'_>
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
