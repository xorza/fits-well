# fits-well design

The decisions that shape the public API. The code and its doc comments show how
each one works. Sources: the FITS 4.0 standard (`docs/refs/fits_standard40.md`)
and the dated measurements in `docs/research/`.

## Reading

- **A random-groups physical read appends to the caller's `Vec`,** as an image's does,
  and a parameter name resolves once to the parameters that carry it; an absent
  `PTYPEn` is `None` and names nothing. Refused: a new `Vec` for each group, and `""`
  for an absent name, which summed every unnamed parameter.
- **A size error carries its unit:** `DataSizeMismatch { unit, expected, got }` counts
  bytes, samples or rows in `u64`, and a P/Q count or offset its width cannot hold is
  `DescriptorOutOfRange`. Refused: one bare pair of numbers for three units, and an
  overflow error for a descriptor that is merely too large.
- **A plain image read lands in the buffer it returns:** a stream source reads the
  stored samples straight into the caller's `words`, or the owned image, and swaps
  them in place; a resident source decodes from its borrowed bytes; the checksum sums
  the big-endian bytes as they arrive. A physical read converts a bounded chunk at a
  time, and random groups read straight into their samples. Refused: a staging copy
  of the unit or the section in the reader's scratch, which never shrank. Measured
  on the standard frame: a stream read 7–25 % faster, a slice read unchanged.
- **`Hdus` is the one owner of the HDU list:** the scan, the selection by index or
  name, and the image list, which `FitsReader` and `FitsEditor` share. A reader method
  borrows the list and the source apart. Refused: a free function over a slice of
  `Hdu` that each caller imported, and an index looked up again after a selection.
- **A `-TAB` reference selects its extension as any lookup does,** by name, version
  and level, and reads it through the table path, plain or tiled-compressed; an absent
  extension is `HduNotFound`. Refused: a filter on `BINTABLE`, which missed a compressed
  table, and an `InvalidWcs` that formatted the lookup's error.
- **A stream source tracks its position and reads adjacent ranges as one:** a read
  that starts where the last ended does not seek, the rows and heap arrays a selection
  copies join where they follow one another, and a buffer grows as a `Vec` grows, with
  no zero fill. Refused: a seek and an exact reservation for each range, which made a
  selection of n rows cost O(n²) copying.
- **`read_data_unit` gives the data bytes alone,** `data_bytes` of them, without the
  block fill; the checksum reads the fill when it needs it. Refused: the padded unit
  and its data range, which every caller sliced off.
- **A physical value is one fused multiply-add,** `BSCALE.mul_add(raw, BZERO)`, the
  correctly rounded `f64` of the exact affine, for images, table columns and random
  groups alike; the `BLANK` test and the identity are decided once, outside the loop.
  Refused: cfitsio's product then sum, which rounds twice, and astropy's `float32`
  arithmetic for `BITPIX` ≤ 16. Source:
  `docs/research/2026-10-24-reference-library-practice.md`.
- **The six sample types are one sealed trait, `image::Sample`:** its `BITPIX` public,
  and sealed the view and owned wraps and the big-endian and physical operations every
  loop shares. A generic caller converts with the std traits: `View::from`,
  `Data::from`, `<&[T]>::try_from(view)` and `Vec::<T>::try_from(data)`, each failed
  conversion giving its input back. Refused: a `match` written by hand for each
  operation; a trait of its own in each caller, as lumos wrote `PlaneSample` and
  `ExtensionSample`; and public `Sample` methods beside the std conversions.
- **Every image path reads one scaling, checked when the header is read:** a float
  image ignores `BLANK`, which §5.2.5 defines for integers, as cfitsio ignores it and
  astropy warns and ignores it, and an integer `BLANK` its `BITPIX` cannot hold is
  `KeywordOutOfRange`. `image_metadata`, `view` and `read` then agree, and a read
  builds its image without a second check. Refused: a check in `Image::new` alone,
  which made `read` refuse what `view` gave.
- **A typed view over the `u64` scratch is a safe fn with one release `assert!` of its
  length,** as `bytemuck::cast_slice` checks it. Refused: an `unsafe fn` whose bound
  was a `debug_assert!`, which a safe caller could break in a release build.
- **One handle reads an image: `FitsReader::image` gives an `ImageReader`.** A
  `Region` picks the whole image or one range per axis, and the handle reads it
  three ways: `view` into the caller's `words`, `read` into an owned `Image`, and
  `physical_into`. The handle resolves the geometry and a compressed image's codec
  and tiles once. Refused: one method per combination of whole or section, owned
  or borrowed, and summed or not (six methods, with the checksum as a suffix), and
  `Lazy`, a second form of the same read.
- **A checksum is summed by the handle that reads.** With `with_checksums`, a read
  feeds the stored bytes it fetches, as long as they continue the sum: a plain
  image's whole reads and sections, and a compressed image's whole reads. A
  compressed section's tiles are not fed. `finish` reads only what the reads did
  not feed, and without `with_checksums` it reads the whole unit. Refused: a caller-held sum passed to each read, which a caller could finish
  against another HDU.
- **A physical read appends to the caller's `Vec`.** `physical_into` extends `out`,
  so a caller can append chunk after chunk to one plane, and nothing is zeroed
  first. A plain image converts from its stored big-endian bytes in one pass.
  Refused: a `&mut [T]` that the caller must size and zero, and a new `Vec` for each
  call.
- **`Borrowed` is the borrowed image, the twin of `Image`.** Every read result is one,
  `Image::view` lends one, and the conversions (`physical_into`, `unsigned`,
  `sample_type`) live on it alone. `View` has a typed accessor for each type, and
  `Data` converts back to its `Vec`. Refused: the same conversions on `Image`, `Lazy`
  and `Borrowed`, each with its own code.
- **The caller owns the image-sized buffers.** `view` fills the caller's `words`,
  and the reader keeps only staging: a section's stored bytes, and the words a
  compressed image's physical read decodes into, both the size of the region read.
- **Names do not repeat their module.** The borrowed image is `image::Borrowed`, and
  the handles are `fits_well::ImageReader` and `fits_well::ImageWriter`, beside
  `FitsReader` and `FitsWriter`. Refused: `image::ImageView`, `image::ImageReader`
  and `image::Writer`.
- **A name pairs with its counterpart.** The reader and the writer share
  `with_checksums`; the two table forms share `schema()` (`Schema`, `AsciiSchema`)
  and the `tnull` field; a stored kind and its decoded data share variant names
  (`ElementKind::U8`/`Character` with `ColumnData::U8`/`Character`,
  `AsciiKind::Text` with `AsciiColumnData::Text`); an owned type lends with `view()`
  and its view owns again with `to_*()` (`Image`/`Borrowed::to_image`,
  `Data`/`View::to_data`); a builder option is `with_*` and a constructor `from_*`.
  A type read from text implements `FromStr` (`Scale`, `Tform`, `Datetime`,
  `time::Unit`), so it parses with `str::parse`; an inherent `parse` stays for input
  that is not text (`Header::parse(bytes)`, `Schema::parse(header)`).
  A count is `*_count` (`row_count`, `field_count`), except a FITS keyword's own name
  (`gcount`, `pcount`). Refused: one spelling on each side, which a caller must
  remember twice, and `nrows`, a third spelling of a count.
- **A read names its HDU with an `HduSelector`:** an index, an `EXTNAME`
  (case-insensitive), or a name with `EXTVER` and `EXTLEVEL`, each 1 when absent.
  A primary with no `EXTNAME` is `PRIMARY`, as in astropy. Every reader method
  takes `impl Into<HduSelector>`, and `Hdu::matches` answers the same selector,
  so a caller does not compare `EXTNAME` by hand. A name no HDU carries is
  `HduNotFound`, and `images()` iterates the image HDUs as `&Hdu`, as `hdus()`
  gives them. A fact about an HDU is read from the `Hdu` that `hdu()` gives: its
  `index`, its `table_schema()`. `extname()` errors for an `EXTNAME` that is no
  text, as `extver()` does, and a lookup passes over that HDU. Refused: `hdu_index`
  and a reader-level `table_schema`, second ways to the same fact; an `Option`
  result for a missing name; `first_image()`, which is `images().next()`; and an
  `extname()` that read a bad `EXTNAME` as none.
- **A read of the wrong kind names what it asked for.** `HduKindMismatch { expected:
  ExpectedHdu, got: HduKind }` refuses an image read of a table and a table read of an
  image, and `ColumnKindMismatch { column, expected: ExpectedColumn }` a column read its
  format does not take, the column by its zero-based index; `ImageHasGroups` stays its
  own case, which no kind names. A failed text or unit check carries an enum too:
  `AsciiValueDefect`, `UnitKind`, `DescriptorField`. A defect enum, which says why a
  value fails, is an `Error`; an enum that names a kind or a field has `Display`
  alone, as `ValueKind`. Refused: a variant for each kind
  (`NotAnImage`, `NotABitColumn`, …), nine where two say the same, and
  `TypeMismatch { expected: &'static str }`, a message. Names a FITS format defines, a
  codec's or an algorithm's, stay its spelling.
- **`Hdu::is_image` classifies by rank alone.** An HDU is an image when it is a
  primary array, an image extension or a tiled-compressed image whose `NAXIS`
  (`ZNAXIS`) is above 0. Refused: the kind alone, which counted an image
  extension with `NAXIS = 0` but not such a primary. Also refused: requiring the
  whole geometry to resolve, which would turn a malformed image into "not an
  image" and hide the error that `Hdu::image_metadata` reports. Source: §4.4.1.
- **An image's geometry has one public source:** `Hdu::image_metadata` before a
  read, `ImageReader::metadata` during one. Both give a compressed image's own
  shape and `BITPIX`. Refused: public `Header::bitpix`, `naxis`, `axes` and
  `scaling`, which read the keywords as they stand and so give a compressed
  image's container, `BITPIX = 8` and its table axes.
- **A checksum report gives two verdicts:** `is_valid`, both checksums present
  and verified, and `has_invalid`, either one malformed or contradicted by its
  bytes. A caller's policy for `Absent` and `Unknown` stays its own. Refused: a
  single verdict, which would choose that policy for every caller.
- **`checksum` owns the one reading of `DATASUM` and `CHECKSUM`, and the one running
  sum.** An empty or all-blank value is `Unknown` (§4.4.2.7), as cfitsio `ffvcks`
  reads it, and a `DATASUM` that is no 32-bit unsigned decimal is `Invalid`; the
  reader judges by it, and the editor recomputes the sum from the data where it gives
  none. The reader, the image stream and the editor sum through one `Running`, the
  editor in bounded chunks. Refused: a parse in each, which read `''` two ways and
  failed the editor on a malformed `DATASUM`, and a copy of the data unit to sum it.
  Source: research `2026-10-24-reference-library-practice.md`.
- **`Ok(None)` means only that an optional value is absent.** A header getter gives
  it for an absent keyword, and an accessor such as `Value::as_real` or
  `ColumnReader::unsigned` for a value not in that form. An operation that cannot
  do what its name says errors instead: `parameter_physical` with a name no
  parameter carries (`GroupParameterNotFound`). Refused: `Ok(None)` for a miss, which a
  caller reads as "absent" and goes on.
- **The `Source` trait is sealed and cannot be named.** A source is not an
  extension point, and no caller is generic over sources. Refused: exporting it
  before such a caller exists.

## Tables

- **A table selection reads through a `Fetch`,** the file through the reader or tiles
  decoded into memory, and joins the rows and heap arrays that follow one another
  into one fetch. `bintable::selection` owns the compaction and the `Selection` types,
  and the reader's `DataSource` only fetches. Refused: a selection in the reader, which
  wrapped decoded bytes in a source of their own to reach it.
- **Both table forms parse their schema from the header when the file opens:**
  `Hdu::table_schema` gives a `Schema` and `Hdu::ascii_schema` an `AsciiSchema`, and a
  read builds its table over the schema it holds. Both check the structure the data
  unit is sized from, `BITPIX = 8`, `NAXIS = 2` and `GCOUNT = 1`, and an ASCII table's
  `PCOUNT = 0` (§7.2.1, §7.3.1). Refused: an ASCII schema known only after a read of the
  data unit, and a table whose header gives its data unit another size than its rows.
- **A bad optional column keyword fails only its column's reads that need it.** A
  `TSCALn`, `TZEROn`, `TNULLn`, `TDIMn` or `TDISPn` whose value does not read is kept
  on its column; its accessor and each read it changes fail with
  `InvalidColumnKeyword`, which names it and holds the cause, and the stored values and
  the other columns read. Refused: a refusal of the whole table, which lost every
  column for one card, and cfitsio's and astropy's silent skip of the keyword, which
  reads the column with wrong values. Source:
  `docs/research/2026-10-24-reference-library-practice.md`.
- **A `Tform` is a repeat, an element kind and a storage:** `ElementKind` names the
  values (Table 18), and `Storage` is `Fixed` or `Array(DescriptorWidth)` for a `P`
  or `Q` column. Refused: one kind enum that held the descriptors beside the element
  types, which put an `Option` element type beside it and an `unreachable!` in each
  decode that had to refuse a descriptor.
- **`X` and `PX` decode to bits:** `ColumnData::Bit` and `ColumnView::Bit` hold the
  rows' bits with no padding between them, and a `PX` row ends at its descriptor's
  count. Refused: the packed bytes as `ColumnData::U8`, which gave a 3-bit and an
  8-bit row the same length and named `B` and `X` with one variant.
- **The characters after a `TFORMn` code are ignored,** and after a P/Q's element
  type, its `(emax)` among them, as §7.3.1 leaves them undefined and cfitsio and
  astropy skip them. Refused: a refusal, which lost every column of a table that
  carried a writer's own suffix.
- **A tiled-compressed table reads as the table it restores.** `read_table`,
  `read_table_rows`, `read_table_columns`, `read_table_cell` and `Hdu::table_schema`
  give the logical table, as an image read gives a compressed image's pixels. A
  compressed image is `HduKindMismatch` to each. Refused: a separate
  `read_compressed_table`, which made a caller check the kind first, and the
  container, a table of compressed cells that reads quietly wrong. Source: §10.3.
- **A partial read decodes only the tiles its rows touch, and only the columns it
  selects,** as an image section decodes only its tiles. The touched tiles and
  the heap decode into a table of their own, which the plain selection reads.
  Refused: a second selection path over the tiles, and a buffer of the whole
  restored table. The cost: a table with variable-length columns restores its
  whole heap, because each descriptor addresses the heap by its absolute
  offset.
- **A table's facts live on its schema.** `Schema` and `AsciiSchema` give the row
  count and row width by method, the columns as a field, and `column_index`; a
  `Column` and an `AsciiColumn` give their `byte_offset()` alike. `Table` and `Ascii`
  give the schema and read columns. A builder adds a column with `push` and is built whole with
  `with_columns`. Refused: `Table::column_index`, a second way to the schema's, and
  a consuming `Builder::column`, a third way to add a column under the name a
  table reads one by.
- **A column is named by one borrowed `ColumnSelector`,** an index or a `TTYPEn`
  name compared case-insensitively, through one `column` method on `Table`,
  `Ascii` and `Selection`, and through each reader method. Refused: a
  `column_by_idx` and `column_by_name` pair on each type, and an owned `String`
  name, which allocated on each read. Source: §7.2.2, §7.3.
- **`read_table_columns_into` refills the caller's `Selection`.** Each column
  keeps its buffers when its type is the same, so a loop over the chunks of a
  plain table allocates nothing after the first chunk. On an error, the
  selection holds no column. Refused: a new `Selection` for each call, which is
  what `read_table_columns` gives.
- **Every row collection gives `row(r)` and `iter()`, and `IntoIterator` for a
  reference,** `Ragged`, `BitColumn` and `AsciiText` alike, with an exact count.
  `Ragged<V>` gives them once, for each `V: RowValues`: a slice, a `&str`, or for a
  variable-length column a `ColumnView`, and an `UnsignedView`, the borrowed twins of
  `ColumnData` and `UnsignedData` as `View` is of `Data`. Refused: a `Ragged` impl
  for each value type in `ragged.rs`, which tied it to the table modules and left
  `Ragged<UnsignedData>` without rows, and `get`, `rows` and `iter` for one
  operation.
- **`ColumnData` gives its values typed:** an `as_*` accessor for each variant,
  and `TryFrom<ColumnData> for Vec<T>`, which moves the values. Refused: a
  `let … else` on the variant at each caller.
- **Table types are named for their module:** `table::Table`, `table::Ascii`,
  `table::Builder` and `table::AsciiBuilder`. A builder from a row count and its
  columns is `with_columns`, and `RandomGroups::group` gives one group. Refused:
  `AsciiTable` and `TableBuilder`, which repeat the module, as `image::ImageView`
  would, and `bintable::field` for `bintable::column`, which holds `Column`: a leaf
  name shared under two parents is the norm, as `fmt::Error` and `io::Error`.
- **One name for an element type and a count across the crate:**
  `ColumnData::U8`, as `Data::U8`, and `len` with `is_empty`, as `Data` and
  `AsciiColumnData` have. Refused: `ColumnData::Bytes` and `element_count`.

## Compression

- **A tiled-compressed table's tile layout is read from its header once, when the
  file opens,** and a partial read of a table with no arrays fetches only the container
  rows of the tiles it touches. A table with arrays reads the whole container: the
  compressed descriptors of a tile address the container heap, which a selection of
  its rows would move. The restored heap holds the arrays the tiles reference, row by
  row, as a plain selection lays them out. Refused: the container read whole for each
  cell, the whole restored heap allocated for a read of one row, and a second check of
  the header against the container schema parsed from it.
- **A tile parameter resolves as its keyword, then its column, then neither.**
  `ZSCALE`, `ZZERO` and `ZBLANK` each resolve on their own, and an integer image's
  stored null falls back to `BLANK`. A column of any numeric type reads as doubles.
  Refused: a column only, which decoded a `ZSCALE` keyword with a scale of 1. Source:
  cfitsio `imcomp_get_compressed_image_par`; research
  `2026-10-24-reference-library-practice.md`.
- **A float image with no `ZSCALE` and no `ZZERO`, as a keyword or a column, or with
  `ZQUANTIZ = 'NONE'`, is lossless.** Its `COMPRESSED_DATA` cells hold GZIP_1 or GZIP_2
  big-endian floats at `ZBITPIX` width, and another codec on such an image is
  `UnsupportedCompression`. cfitsio treats the absent parameters as `'NONE'` whatever
  `ZQUANTIZ` says, and allows only GZIP for it. Refused: the integer decode of those
  cells, which read the float bits as quantized integers. Sources: §10.2; cfitsio
  `imcomp_get_compressed_image_par`, `imcomp_init_table`.
- **A float tile decodes in the image's stored type.** A tile stored as floats keeps
  every bit, a signaling NaN's included, and a quantized one is computed in `f64` and
  rounded once, as cfitsio's `unquantize` does. Refused: one `f64` plane for both
  widths, which set a signaling NaN's quiet bit and doubled an f32 tile's memory.
- **An integer image's null value is `BLANK`, else `ZBLANK`.** A tile whose stored null
  differs from it reads with the image's. Source: §10.1.1, Table 36.
- **The header's compression keywords resolve once, for the image handle.** Codec
  parameters, `ZQUANTIZ`, `ZDITHER0`, `ZMASKCMP` and the parameter keywords are read by
  `FitsReader::image`, so a bad keyword fails there; a read resolves only the columns.
  `ZDITHER0` is checked only for a method that dithers.
- **A float image's quantization is one opaque type, `Quantization`:**
  `Quantization::NONE` for lossless (`ZQUANTIZ = 'NONE'`), `relative(level, dither)`
  for levels spaced at each tile's noise divided by `level`, and `absolute(step,
  dither)` for one spacing for every tile, as fpack's `-q level` and `-q -step`.
  `Hcompress::relative` and `Hcompress::absolute` are the same pair for the HCOMPRESS
  scale, as fpack's `-s`. Refused: cfitsio's one number, where 0 is lossless and a
  negative value absolute, and fits-well's former level 0 for "the default of 4":
  both put a mode into a magic value. Refused: an enum with public variants, whose
  values anyone could build unchecked, as `Alternate` refuses. Sources: cfitsio
  `fpackguide.md` §C, `fits_quantize_float`.
- **`Dither` holds the method and its seed:** `None`, `Subtractive1(seed)` and
  `Subtractive2(seed)`, so a seed never stands beside no dither. `DitherSeed` is
  opaque: `FROM_SAMPLES`, the default, or `fixed(1..=10000)`.
- **The default seed is `(Σ bytes of every sample mod 10000) + 1`.** A byte sum does not
  depend on the byte order. The same image gives the same file, and frames whose first
  tiles are equal, such as registered frames with blank edges, still get different
  seeds; the reader takes any seed, so the rule has no effect on interop. The cost: one
  more read of the samples. Refused: the first tile's byte sum (fpack `-qt`, astropy
  `DITHER_SEED_CHECKSUM`), as equal first tiles give equal seeds; the clock (cfitsio's
  and astropy's default), which writes a different file on each run; and the former
  fixed seed of 1, which gives every image one dither pattern, which fpack warns loses
  the benefit of dither when images are added. Source: `fpackguide.md`, `-qt`.
- **The noise estimate is cfitsio's:** noise3, or noise2 or noise5 where either is
  nonzero and smaller, each from the 2nd, 3rd and 5th order median absolute
  differences, in `float` arithmetic for an f32 image, for the float quantization and
  the HCOMPRESS scale. A write then matches fpack's `ZSCALE`, `ZZERO` and HCOMPRESS
  scale bit for bit. Refused: noise3 alone, which gave a larger step than fpack. Two
  differences from cfitsio stay: each median runs over its own differences, where
  cfitsio takes noise2's over the noise3 count and reads entries left from earlier rows
  when a run of three equal values drops one (`quantize.c` `FnNoise5_float`); and a
  level or a step stays an `f64`, where cfitsio takes it as a C `float`. Sources:
  cfitsio `fits_quantize_float`, `fits_img_stats_int`; Stoehr et al., ST-ECF
  newsletter 42.
- **A quantized float image takes HCOMPRESS_1**, and the HCOMPRESS scale applies to
  the quantized integers, as in cfitsio. `SUBTRACTIVE_DITHER_2` with it is
  `InvalidCompressionParameter`, as fpack refuses `-qz` with `-h`. Refused: cfitsio's
  library way, which changes the method to `SUBTRACTIVE_DITHER_1` with a warning, a
  change the caller did not ask for. `PLIO_1` for floats stays refused: a quantized
  tile seldom fits its 24-bit range. Sources: `fpackguide.md` `-qz`; cfitsio
  `imcomp_init_table`.
- **The float container widens its two descriptors to `Q`** past the 32-bit range, as
  the integer container does. Refused: the former `DataUnitOverflow` for such a heap.
- **A lossless float write gathers the samples as bytes**, so the floats come back as
  written, a NaN's payload included. Refused: the `f64` gather the quantizer uses,
  which sets a signaling NaN's quiet bit.
- **A u16 image's `PLIO_1` stream holds its physical values:** the stored value plus
  32768, as PLIO takes no negative value, when `BITPIX = 16` and `BZERO = 32768`.
  cfitsio writes and reads it so. The cost: cfitsio also subtracts 32768 from a 32-bit
  PLIO image with `BZERO = 32768`, which its own writer does not offset; fits-well
  offsets only the 16-bit case. Refused: astropy's way, which refuses the write and
  reads with no offset, so it misreads cfitsio's files. Sources: cfitsio
  `imcomp_convert_tile_tshort`, `imcomp_decompress_tile`; astropy
  `_tiled_compression.py`. cfitsio 4.7.0's `fpack -p` aborts on a u16 image, so the
  read test builds its stream; `funpack` reads fits-well's u16 PLIO file back exactly.
- **A compression error names its case, and a write refuses what it would not keep.**
  `InvalidCompressionParameter` carries a `CompressionParameter` and
  `UnsupportedCompression` a `CompressionFeature`. An integer image refuses a
  quantization, as a table does, and a tile of 0 is refused for an image and a table
  alike. A stream that does not decode is `CorruptCompressedData` with a
  `CompressedDataDefect`, the gzip error kept as its source. Refused: formatted
  messages, and a quantization an integer write ignored or a tile of 0 read as 1 with
  no notice.
- **A variable-length column's descriptor block holds the original descriptors, then
  those of the compressed arrays,** in the writer and the reader, as cfitsio and
  nom-tam-fits 1.19.1 hold them, the reverse of the order FITS 4.0 §10.3.5 lists; no
  keyword marks the order. Refused: the standard's order, which no other writer uses
  and cfitsio misreads; a guess between the two for each tile, which read a malformed
  block in the other order with no error; and nom-tam-fits's switch for the other
  order, which no caller needs. Sources: cfitsio `imcompress.c`
  (`fits_compress_table`, `fits_uncompress_table`); nom-tam-fits 1.19.1, fix #557.
- **A parallel decode writes each band of the image in place.** The tiles at one index
  along the slowest axis write one contiguous run of the plane and nothing else, so with
  as many bands as workers each worker takes whole bands from `par_chunks_mut` and
  narrows its tiles straight into them. An image of fewer bands, and a section, decode
  in waves through one buffer of a slot for each tile. Refused: a `Vec` for each tile
  and a `Vec<Vec<D>>` for each wave, scattered serially, which a whole read paid 9 % to
  27 % for, and a serial section decode.
- **An encoder hands its container to the writer in parts.** `EncodedUnit` holds the
  header, the main-table rows and the heap's cells, each compressed once; the writer sums
  and writes them in order, small cells gathered to 1 MiB writes. A compressed table's
  heap holds the fixed columns' cells, then every array, then the arrays' descriptor
  blocks, so the array offsets are prefix sums known before any block is gzip'd, and the
  blocks gzip in parallel. A table read with arrays restores each tile and its own heap
  in parallel, and the descriptors land once the heaps' bases are known. Refused: tiles
  kept and copied into one data unit, twice the compressed size at the peak, a write
  for each cell, and blocks gzip'd in a serial walk.
- **One tile codec carries its parameters.** `TileCodec` holds `GZIP_1`/`GZIP_2` with
  their level, `RICE_1` with its block size and pixel width, `PLIO_1` with its u16
  offset, `HCOMPRESS_1` with its scale and smoothing, and `NOCOMPRESS`; each codec
  module's `Codec` is its parameters, whose encode/decode pair runs over its natural
  input — integers for RICE, PLIO and HCOMPRESS, big-endian bytes for GZIP and
  NOCOMPRESS — and the codec converts what a caller holds. Image, float, null-mask and
  table tiles and the bench all go through it. Refused: five codec enums and six
  dispatches, with NOCOMPRESS inlined four times under different size checks, and
  owning and `_into` twins of each codec.
- **A codec lends its output from its scratch.** An encode lends the stream, which a
  caller that keeps it copies once at its size; a decode lends its integers as the
  scratch's `Vec`, which the tile decoder swaps into its own buffer, and the null mask
  and a float tile's quantized samples are read where they lie. Refused: the caller's
  output buffer as a parameter, which made each codec function take four to six, and
  a copy of every decoded tile into it.

## World coordinates

- **The `CTYPEia` grammar takes a distortion code.** A 12-character value with `-` in
  byte 9 carries a distortion code after any projection (`RA---TAN-SIP`), as wcslib
  excises one; `RA---TPV` is TAN with the TPV polynomial. An unknown code leaves the
  axes unsupported. Source: wcslib `wcsset`; Shupe et al. 2005; the TPV registration.
- **`Wcs::write_into(header, alt)` writes a description into a header, in wcslib's
  form.** It first removes every record of that description, then sets `WCSAXES`,
  `CRPIX`, `PCi_j` (an identity term omitted), `CDELT`, `CUNIT`, `CTYPE`, `CRVAL`, the
  pole, frame and spectral keywords and `PVi_m` as they were read, and the SIP terms.
  The `Wcs` keeps its `PC` and `CDELT` as read, a `CD` matrix as `PC = CD` with
  `CDELT = 1`, as wcslib's `wcsset` keeps it and as TPV needs; so `from_header` of the
  result gives back the same transform bit for bit. A legacy `CROTA` writes the `PC`
  it derives, whose off-diagonal terms round once. A `-TAB` axis is
  `UnsupportedWcsTransform`, as its arrays live in another HDU. Refused: a new header
  the caller merges, as astropy's `to_header` gives, where a merge over a `CD` header
  leaves `CD` beside the new `PC`, which `from_header` refuses; `CD` output of the
  product matrix, which leaves the form wcslib and astropy write; and a `PC` and
  `CDELT` factored from the product, which rounds. Source: wcslib `wcshdo`.
- **`Wcs::crop(region)` and `Wcs::bin(factors)` derive a new WCS exactly.** A crop takes
  the `Region` the image read takes, so one value crops the image and its WCS, and
  subtracts each range's start from `CRPIX`. A bin by `b` on pixel axis `j` maps
  `CRPIX` to `(CRPIX − 0.5)/b + 0.5`, the centre of the first block, as astropy's
  `slice` with a step does, and multiplies column `j` of `PC` by `b`, as a bin scales a
  pixel axis. With factors `b₁`, `b₂` on the SIP axes, `A_p_q·b₁^p·b₂^q/b₁` and
  `B_p_q·b₁^p·b₂^q/b₂`, `AP` and `BP` alike; TPV, after the matrix, stays. Both go
  through the header `write_into` writes, so a `-TAB` axis is refused as there. Refused:
  astropy's way, which multiplies `CDELT` and keeps the SIP terms, correct only for an
  equal bin on both axes with no SIP; and a stride on `Region` ("No section stride").
- **SIP is a prior distortion, from its keywords, on the primary description only.**
  `(U, V) = (u + f(u, v), v + g(u, v))` on the offsets of the first two pixel axes
  from `CRPIX`, before the matrix, up to order 9 (wcslib reads one digit for `p` and
  `q`). `A_ORDER` and `B_ORDER` turn it on with or without the `-SIP` code, as wcslib
  and astropy read it; a `-SIP` code without them is `MissingKeyword`; its keywords
  carry no alternate letter; a `-TPV` code drops them, as astropy does. The inverse
  solves the forward polynomials by Newton steps with their exact Jacobian, from
  `AP`/`BP` as the first guess when present, until each residual is within 1e-13,
  absolute below 1 and relative above, and is `WcsNoConvergence` after 30 steps:
  wcslib's rule and limit (`disx2p`, `disitermax`). Refused: `AP`/`BP` alone as the inverse, an
  approximate fit whose round trip misses the pixel (astrometry.net does this);
  astropy's `all_world2pix` default of 1e-4 pixel; a fixed absolute 1e-12 pixel, which a
  coordinate past about 4500 can never meet; and SIP only with the `-SIP` code, which
  would read astropy's SIP files without their distortion. SCAMP's `PVi_m` with `m` of
  5 or more beside SIP set no projection parameter, as astropy drops them.
- **TPV is a sequent distortion, on the celestial intermediate coordinates in degrees:**
  `ξ = P₁(x, y)` from the longitude axis's `PV` and `η = P₂(y, x)` from the latitude
  axis's, the registered 40 terms, an absent term 0 except `PVi_1`, which is 1, so a
  header with no `PV` is plain TAN, as the registration states. On a `-TPV` pair no
  celestial `PV` sets `φ₀`, `θ₀` or the pole; `LONPOLE` and `LATPOLE` still do, as
  wcslib reads it. The inverse uses the SIP stop rule. Refused: TNX, ZPX, TPU and TPD,
  which no darkroom input uses. Source: TPV registration; wcslib `wcsset`, `tpvset`.
- **The native latitude crosses from the rotation to the projection as its sine and
  cosine.** The rotation takes θ = atan2(z, √(x² + y²)) from the rotated unit vector,
  with x = sin(δ − δp) + 2 cos δ sin δp sin²(Δα/2), and a zenithal projection takes
  ζ = atan2(cos θ, sin θ) from the pair, so a point next to the native pole keeps its
  offset to an ulp. TAN is cos θ / sin θ, SIN and SZP use σ = cos²θ/(1 + sin θ), and
  the HEALPix polar cap cos θ = (σ/√K)·√(2 − σ²/K). Refused: θ = asin(sin θ) in
  degrees, wcslib's form outside its `|z| > 0.99` branch, which put every point within
  8.5e-7° of the pole on it; and ζ in radians alone, which loses θ near the equator
  that the cylindrical projections need. CSC keeps wcslib's θ = asin n of its vector,
  which has unit length only to the `f32` sum in its scale. Source:
  `docs/research/2026-10-24-wcs-round-trip-precision.md`; wcslib `sphs2x`.
- **A projection's constants are built once, into its family's kernel.**
  `Projection::parameters` validates `PVi_m` and builds the kernel of the family —
  zenithal, perspective, conic, equatorial, cube or HEALPix — whose file holds its
  forward, its inverse and its constants side by side; one table holds each code and
  family. Refused: the constants for each point, two `powf`, two `ln_1p` and six trig
  calls a COO point, and one 1200-line module whose forward and inverse of a projection
  lay 200 lines apart.
- **An inverse solve stops at the precision of its root and errors when it does
  not.** ZPN, AIR, MOL and PCO share one solve: Newton's method on the exact
  derivative inside a bracket the math gives and each residual narrows, a bisection
  where a step leaves the bracket or does not halve, as Boost's
  `newton_raphson_iterate` does, a stop when the root moves by at most four ulps, and
  `WcsNoConvergence` after 100 steps; a quadratic takes its roots as q/a and c/q.
  Refused: wcslib's `zpnx2s` false position, which returns its last ζ after 100 steps
  with no check, to 2.7e-5 of the bracket at worst; and an absolute residual stop, as
  wcslib's `airx2s`, `mols2x` and `pcox2s` have, which left AIR and MOL 2656 ε and
  1589 ε off and, with no bracket, PCO refusing half its sphere. Sources: Boost.Math
  `tools/roots.hpp`; wcslib 8.10 `prj.c`.
- **MOL solves for the co-auxiliary angle β = π/2 − |γ| more than 45° from the
  equator:** `β − sin β`, by its series below 1, against `π cos²θ/(1 + |sin θ|)`, and
  back from `1 − sin|γ| = 2 sin²(β/2)`, so the colatitude and cos γ keep their digits.
  The round trip next to a pole keeps to the change one ulp of y makes there. Refused:
  γ from `π sin θ` and `asin(y/(√2·180/π))`, as wcslib's `mols2x` and `molx2s` take it,
  which at 1e-5° from the pole was 2.3e6 ε off where an ulp of y moves θ 190 ε; the
  test `mollweide_keeps_its_digits_next_to_the_poles` holds that bound.
- **AIT's domain is its ellipse `z² ≥ 1/2`, where the native poles lie on the edge.**
  A point within rounding of the edge is on it, and a pole deprojects to φ = 0, as
  wcslib's `aitx2s` takes both. Refused: `z² ≥ 0`, which left the bounds check to refuse
  the points between the two ellipses and, as `2z² − 1` rounds below 0, the poles.
- **A formula next to its own zero computes the difference first.** A velocity is
  c(ν₀ − ν)/ν₀, c(λ − λ₀)/λ₀ or c(ν₀ − ν)(ν₀ + ν)/(ν₀² + ν²); MER is asinh(tan θ) and
  atan(sinh y); SFL and BON divide by `cosd`, exactly 0 at a pole, where φ is 0. One ulp
  from the rest value the velocity is right to 4 ulps, where 1 − ν/ν₀ was 32 % off.
  Refused: wcslib's forms, which a reference comparison at 1e-9 hid.
- **A 1-D `-TAB` inverse searches the array's monotonic runs.** The standard allows a
  coordinate array that is not monotonic, and the first interval in array order that
  holds the point wins, as wcslib's `tabx2s` scans for it. The transform splits the
  array into maximal strictly monotonic runs, broken at equal neighbours and equal index
  values, and a point binary-searches them in order: the scan's interval in
  O(R·log(K/R)). A coupled table keeps each cell's level-0 corner bounds, so the search
  passes over a cell that cannot hold the point without spending its budget. Refused:
  the scan, O(K) a point, 79 µs over a 100 000-entry index; and a rule that the array be
  monotonic, which the standard does not make; and a last-hit cache, which a `&self`
  transform shared across threads cannot hold. Source:
  `docs/research/2026-10-24-tab-inverse-runs.md`.
- **One plan for each axis holds what it is, its step and its world unit.**
  `AxisPlan` holds the axis's kind, parsed once from `CTYPEi` (celestial, `-TAB`, or
  other), its own transform with the `CRVALi` it adds, and the unit its world
  coordinates come out in, which `Wcs::world_unit` and `axis_world` report. One
  resolution, `WorldUnit::of`, gives the unit of a celestial, a spectral and a `-TAB`
  axis, so a `WAVE-TAB` axis in nm says m, the unit its table was scaled to.
  `LinearTransform` owns `CRPIX`, SIP runs once for each point, and the celestial
  transform owns its step and its latitude check. Refused: the parallel vectors of
  references, units and transforms, each indexed by axis, and the `CTYPE` text compared
  after the parse.
- **A WCS error names its case.** `InvalidWcs` carries a `WcsDefect`, with the grism
  and `-TAB` cases in `GrismDefect` and `TabularDefect`; `ConflictingWcsKeywords`
  carries a `WcsConflict` and `WcsInvalidPole` a `PoleDefect`. A case holds the header
  text that names it, a `CTYPE` or a frame value, where one does, and a projection's
  errors its `Projection`, which prints as its code. Refused: a formatted `detail`,
  about 35 messages a caller could tell apart only by their text.
- **A transform writes one point into the caller's slice.** `pixel_to_world` and
  `world_to_pixel` take the input and the output as slices of one value per
  axis, so a caller loops over its points with one buffer and a point makes no
  allocation unless an axis is `-TAB`. Refused: a new `Vec` for each point, and
  a batch API, which the loop makes unnecessary. wcslib also writes into the
  caller's arrays. An axis past the last is `IndexOutOfBounds` for every per-axis
  call, `world_unit` and `axis_world` alike. Source:
  `docs/research/2026-10-24-wcs-into-slice.md`.
- **The inverse linear step solves with the matrix's LU factors,** in place in
  the output slice. Refused: the explicit inverse, whose product needs its input
  kept apart from its output, so a buffer for each point. Solving is as accurate
  as the product with the inverse or better (Higham, §14.1), and it keeps the
  division for each row rather than a reciprocal. The cost: four coupled linear
  axes invert 34% slower, four dependent divisions where the product was one
  pass. Source: `docs/research/2026-10-24-wcs-into-slice.md`.
- **A WCS or time read selects its description with `impl TryInto<Alternate>`,**
  as a keyword argument takes `impl TryInto<Keyword>`: `None` for the primary
  description, and `'A'` or `Some('A')` for an alternate. `Alternate` holds the
  primary description or a letter `A` to `Z`, checked when it is made (§8.2).
  Refused: `Option<char>`, which let a digit spell another keyword (`CTYPE1`
  and `1` read axis 11); `Option<Alternate>`, which made each call spell
  `Some(Alternate::new('A'))`; a public enum, whose letter variant anyone
  could build unchecked; and `WcsAlt`, which repeats its module. A bare `None`
  infers its type only while `Option<char>` is the one `Option` that
  `Alternate` converts from.

## Writing

- **Every image write borrows its samples.** `write_image` and
  `write_compressed_image` take a `Borrowed`, and a stream's `write_chunk` takes a
  `View`. An owned `Image` lends one with `view`. Refused: an owned `Image` or
  `Data`, which made a caller copy a buffer it already held, as a whole plane or
  as one `Vec` for each chunk.
- **`stream_image` takes the image's `Metadata`**, the shape, `BITPIX` and scaling
  that the reader gives. Refused: the three as separate arguments.
- **A stream needs `Seek`, even without checksums.** It seeks only to write
  `CHECKSUM` again after the data. Refused: a type state for checksums, which puts
  a second type parameter on every writer method for a sink no caller has. A
  caller that writes to a pipe writes to a `Vec` and copies it.
- **One `CompressionConfig` holds the codec, the tiles and the quantization.**
  Refused: the codec as a separate argument beside `CompressionOptions`, and a
  table's tile height as a third form. A table takes one tile axis, its rows, and
  no quantization; a config with more axes fails with `RankMismatch`, and one
  with a quantization with `InvalidCompressionParameter`.
- **A table with no tile height set uses cfitsio's tiles of about 10 MB of rows**
  (`maxchunksize` in `fits_compress_table`), and one tile when its rows have no
  bytes. Refused: one row per tile, the former result of a height of 0, which
  compresses badly. Source: cfitsio `imcompress.c`.
- **`write_compressed_table` takes the `Builder` that `write_table` takes**, and the
  header last, as an `Option`, as rustfits takes the same columns for a compressed
  table as for a plain one. Refused: the read-side `Table` with its header first,
  cfitsio's HDU-to-HDU `fits_compress_table`, which is a second way to write the
  same table and made a table that a program built go through a write and a read
  first. Also refused: a builder from a read `Table`; `read_data_unit` and
  `write_raw_hdu` copy an HDU as it is. Sources: cfitsio `imcompress.c`, rustfits
  `CompressedTableHDU`.
- **A compressed table takes GZIP_2 only on `I`, `J`, `K`, `E` and `D` columns.** The
  table's `Compression` is a preference that each column clamps to a codec its type
  takes, as cfitsio's writer does; `L`, `X`, `A`, `B`, `C` and `M` take GZIP_1 when
  GZIP_2 is asked, because cfitsio's reader unshuffles only those five types, and a
  shuffle of 1-byte values does nothing. Refused: GZIP_2 on `C` and `M`, which
  cfitsio's writer emits but its own reader refuses. Source: cfitsio
  `fits_compress_table`, `fits_uncompress_table`.
- **`write_compressed_table` refuses `Compression::None`** with
  `InvalidCompressionParameter`; a table with no compression is `write_table`. The
  reader still restores `NOCOMPRESS` columns. Refused: `ZCTYPn = 'NOCOMPRESS'` on
  write, which the standard permits (Table 36) and cfitsio refuses to read. Refused: a
  clamp to GZIP_1 at deflate level 0, which compresses nothing and still writes a gzip
  stream for each tile.
- **A table compresses straight from the builder's columns.** Each tile gathers its
  column's cells from the `Builder`, through the `TableCells` trait, and computes
  the descriptors the plain table would hold. Refused: encoding the rows first and
  gathering each column back from them, cfitsio's route, which it takes because its
  input is a file of rows; it doubled the time of a write. Also refused: a `Schema`
  built from the builder's layout, a second source of the column facts that the parse
  of the rendered container header gives.
- **A bit column takes rows of bits and their width,** as a text column takes
  fields and theirs: `WriteColumn::bits(name, rows, width)`, kept as one
  `ColumnData::Bit`. A row of another length is `BitFieldLengthMismatch`: unlike a
  text field, a bit row has no padding to fill. A `PX` column is `WriteColumn::vla`
  over `ColumnData::Bit` rows, as any other array column. Refused: bytes the caller
  packed, and a `vla_bits` constructor of its own.
- **A data unit is written from a typed body and never staged.** An image's samples
  are encoded in bounded chunks of the writer's scratch; with checksums, a first pass
  sums the chunks, the signed header is written, and a second pass encodes them again
  and writes them, as cfitsio signs an HDU after its data. A raw unit is written as
  the caller holds it, its fill from a constant block. Refused: a copy of the whole
  unit, 96 MB more for a 24 MP f32 frame. Source:
  `docs/research/2026-10-24-reference-library-practice.md`.
- **A failed write poisons the writer.** Every byte passes one `emit`, which marks the
  writer failed on an I/O error, and refuses each later write, a stream's chunk and
  finish included, with `WriterFailed`. Refused: a retry after a partial write, which
  puts the next bytes after the partial ones and signs a broken HDU.
- **An extension header written first follows an empty primary,** for `write_raw_hdu`
  as for a table, so a copy of a file that drops its primary stays valid.
- **An ASCII real column declares `Fw.d`, `Ew.d` or `Dw.d`, with no default,** through
  `AsciiWriteColumn::float(name, values, AsciiFloatFormat)`; every cell has a decimal
  point (§7.2.5), as C's `%w.df` and `%w.dE` write it for cfitsio, and a value is
  rounded to `d` decimals. A format too narrow for any cell is
  `AsciiFloatFormatTooNarrow`. Refused: a default of `F` with 0 decimals, which wrote
  cells with no point, and astropy's default widths. Source:
  `docs/research/2026-10-24-reference-library-practice.md`.
- **A variable-length column takes its descriptor width as an argument:**
  `WriteColumn::vla(name, rows, DescriptorWidth::P)`. Refused:
  `with_wide_descriptors`, which panicked on a fixed-width column.
- **Table columns stay owned.** A table write transposes its columns into rows in
  the writer's scratch, and that copy dominates. Refused: borrowed columns, which
  put a lifetime on `Builder` for a small gain.
- **`write_empty_primary` writes a primary with no data that carries the caller's
  cards**, and fails with `PrimaryAlreadyWritten` after any HDU. A table or a
  compressed image that comes first gets such a primary without it. Refused: four
  structural cards that each caller writes through `write_raw_hdu`.
- **The structural cards are always generated.** A caller names an extension with
  one `EXTNAME` card in the header it passes. Refused: an `EXTNAME` argument on
  each write.
- **An owned image is built as the borrowed one is,** from a shape, samples and a
  `Scaling` (`Scaling::IDENTITY` for none): `Image::new` mirrors `Borrowed::new`.
  `shape()`, `samples()` and `scaling()` read it, `into_parts()` takes it apart into
  `image::Parts`, and a read of the whole image takes `..` as std ranges do. Refused:
  `new` and `new_scaled`, two constructors where one takes the scaling.
- **The unsigned constructors stay four.** `Image::from_u16`, `from_u32`,
  `from_u64` and `from_i8` take any shape `Image::new` takes. Refused: one
  `from_unsigned` over a borrowed view; the write converts the samples to signed
  values and `BZERO` in any case, so it saves no copy.

## Editing

- **`FitsEditor` edits a file that exists**, over `Read + Write + Seek`, beside
  `FitsReader` and `FitsWriter`. An edit in place parses the header it writes and
  reads no other, as the other HDUs do not move. Refused: a move of every byte after
  the header, as cfitsio (`ffiblk`) and fitsio-pure do, a large write that leaves a
  broken file when it stops in the middle; and row and column insert and delete, which
  a copy does.
- **`replace_header` writes in place or refuses.** The new header must fit the HDU's
  header blocks (`HeaderDoesNotFit`), give the same data unit (`DataSizeMismatch`) and
  keep the HDU's kind (`HduKindChange`), as a `TABLE` and a `BINTABLE` fill their last
  block differently (§7.2.3, §3.3.2). `replace_header_into` writes the whole edited
  file into a second sink, which the caller renames over the first, as astropy does in
  update mode.
- **A replaced header that holds `CHECKSUM` gets a valid one:** from its `DATASUM`,
  trusted as it was before the edit, else from the data unit, read once, also for a
  blank `DATASUM`, the standard's unknown sum.
- **Free records are kept.** `FitsWriter::with_spare_records(n)` writes `n` blank
  records before each `END`, so a later edit fits in place, as cfitsio's `ffhdef`
  reserves space; "spare", as "reserved" names the standard's reserved keywords. A
  header that `replace_header_into` grows keeps the free records the old one had in
  its blocks.
- **`append` refuses a file that does not end at its last HDU:**
  `TrailingContent { hdu_end, file_len }` for bytes after it, such as special records,
  where a reader's scan stops, and `UnexpectedEof` for a file that ends inside it. It
  consumes the editor, so two handles never write one file. Refused: cfitsio's write
  over the trailing bytes, which loses them. Source:
  `docs/research/2026-10-24-reference-library-practice.md`.

## Header

- **A keyword is a `Keyword`: 1 to 8 bytes of `A`–`Z`, `0`–`9`, `-` and `_`, held
  inline and `Copy`.** It is checked when it is made, so a lookup cannot miss on
  its spelling, and the header's index keys on it, so a parse makes no `String`
  for each valued card. Refused: `&str` keys, which made `get("naxis")` quietly
  `None` while `set("naxis")` failed.
- **Every method that names a keyword takes `impl TryInto<Keyword>`.** A
  `Keyword`, a `header::keywords` preset or a `&str` all work, so callers write
  `set("EXPTIME", 30.0)` or `set(keywords::NAXIS, 2)`; a `&str` that is no keyword
  is `InvalidKeyword`, so `get` and the other lookups are fallible. `Keyword`
  converts with `Infallible`, which `FitsError` takes. Refused: `Keyword`
  arguments alone, which made a call site spell every name twice, and
  `From<&str>`, whose conversion would panic on a bad name.
- **`header::keywords` holds the keywords the standard defines (§4–§10) and the
  roots of its indexed families.** `NAXIS.indexed(3)` gives `NAXIS3`, and
  `Keyword::try_from(format_args!(…))` formats a compound name on the stack.
  Refused: macros for keyword names, and a constant for each numbered member.
- **A keyword error names the keyword, and a record error the record.**
  `MissingKeyword`, `KeywordOutOfRange` and `ReservedKeyword` carry a `Keyword`, and a
  family is reported by its member, `TDIM3`, not `TDIMn`. `ValueTypeMismatch`,
  `InvalidValue`, `InvalidHeaderValue`, `HeaderCardTooLong` and `InvalidAscii` carry
  the `RecordName`, and a fixed set of causes is an enum (`ValueKind`, `NonFinite`,
  `TextOrigin`, `NumericTarget`). Refused: `&'static str` names, which a run-time
  keyword could not use, and a message string in place of a case.
- **A name formatted from an axis or column number that is no keyword is
  absent.** The long spelling of a table-WCS family with a wide column (`TPV123_45`)
  can be on no card, so the WCS reads treat it as absent, as the `&str` lookup
  did. Such a name is an `Option<Keyword>` from `Keyword::fitting`, `None` where no
  card can hold it, for that reason or because its family has no spelling in that
  description; only the error that reports a missing keyword formats its name as a
  `Result`. A name that fits but is no keyword panics: its pattern is wrong, which
  no header can cause. Refused: an
  error, which failed a WCS with a wide column, and a `Result<Keyword>` that each
  read turned back into absence.
- **`require_*` reads a mandatory keyword, `set_optional` writes an optional
  one.** `set_optional(k, None)` removes every record with `k`, so the header
  holds the value or nothing. Refused: range-checked getters, whose ranges belong
  to the caller's domain; `set_with_comment`, which is `set(…)?.set_comment(…)`;
  and an infallible `set`, as a NaN real and non-ASCII text are invalid in FITS.
- **Editing the comment of an absent card fails with `MissingKeyword`,** the one
  variant for an absent keyword, whether a file lacks a mandatory one or an edit
  names a card the header lacks.
  Refused: a no-op, which loses a misspelled keyword without a trace.
- **A header record is a `Record`, and its borrowed form a `RecordView`,** as the
  standard calls one a keyword record: a header stores `Record`, `iter()` gives
  views, `remove` the owned record, and `insert_record` and `Extend<Record>` put
  records back. Each is an enum of the kinds a record is: `Valued` with its
  `Keyword`, value and inline comment, `Commentary` with its name and text,
  `Hierarch`, and `Malformed`; a record's name is a `RecordName`. An inline comment
  reads with `get_comment`, the twin of `set_comment`. Refused: `Entry`, a name
  unrelated to its owned twin; one struct whose `Option` value told the kinds apart
  and whose `comment` meant two things; a `record(k)` lookup that could only give the
  valued kind; and a third, internal record type that repeated `Record`.
- **A near-valid record is repaired and reported; a record with a bad value is kept as
  malformed.** `Header::parse` repairs four defects, each a `Repair` that
  `Header::repairs()` lists by the record's 1-based position in the header unit as
  read: a lowercase name reads as its uppercase keyword; a value indicator `=` before
  byte 9, or in byte 9 with no blank in byte 10, reads the value that follows it; a
  non-ASCII byte in a comment or commentary text reads as `?`; an `END` record with
  other bytes after its name ends the header, as cfitsio, which compares only the
  name, and astropy, which warns, read it. A repaired record is an
  ordinary record after that, so a write gives the corrected record. A record whose
  name is no keyword, or whose value does not parse or holds a non-ASCII byte, is
  `RecordView::Malformed` with its 80 bytes and its name when it has one: a lookup of
  its name and a write of its header fail with `InvalidValue`, and `Header::set` or
  `set_hierarch` replaces it. So a read fails only
  when it needs that record; a defect in a keyword that sizes an HDU still stops the
  open, as the next HDU cannot be found. Sources: cfitsio finds a lowercase keyword,
  reads a value after a misplaced `=` and parses a value only on request
  (`ffgcrd`, `ffpsvc`, `ffgphd`); astropy uppercases the name, accepts an early `=`
  and replaces non-ASCII with `?` (`card.py`, `util.py`). Refused: a lenient mode,
  which is modal state; a repair without a report; `Malformed` for every defect, which
  leaves a lowercase `exptime` out of reach by name; and the former refusal of the
  whole file for one bad optional record.
- **The blank records just before `END` are space, not content:** `Header::parse`
  drops them, so a header read and edited fits again in the space they held. Sources:
  astropy fills trailing blanks first (`Header.append(useblanks=True)`); cfitsio
  writes the space `ffhdef` reserves as blank records before `END`. Refused: keeping
  them as commentary, which makes every edited header one record larger than its
  space.
- **HIERARCH records are typed.** `HIERARCH ESO DET CHIP1 NAME = 'x' / c` reads as
  `RecordView::Hierarch` with a checked `HierarchName`: one or more tokens of printable
  ASCII other than `=`, joined by one blank, as the blanks between tokens carry no
  meaning. Names compare without regard to ASCII case, as cfitsio and astropy compare
  them, and keep the case they were read with. `get_hierarch` and `set_hierarch` take
  `impl TryInto<HierarchName>`, as the keyword methods take `impl TryInto<Keyword>`, and
  look the name up in a second index. A long string continues on `CONTINUE` records,
  as astropy writes it; the blank before `=` goes only when the record needs that
  byte, as cfitsio and astropy write it; a comment that does not fit is
  `HeaderCardTooLong`, as is `COMMENT` or `HISTORY` text that does not fit one record.
  Refused: a `Keyword` longer than 8 bytes, which would end
  `Keyword: Copy`; astropy's silent truncation of a long comment; and ESO's token rule
  (`A`–`Z`, `0`–`9`, `-`, `_`) as a read check, which would make the HIERARCH records
  cfitsio and astropy write malformed; and cfitsio's split of long commentary text over
  records, which changes the caller's text with no notice. Sources: Wicenec, Grosbøl and Pence, "The ESO
  HIERARCH Keyword Conventions" (2009); cfitsio `ffgcrd`, `ffmkky`; astropy `card.py`.
- **A commentary record's name is a `CommentaryName`.** The standard lets a record
  with no value indicator start with any 8 bytes of printable ASCII, which no `Keyword`
  holds, so the name is its own inline, `Copy` type, while a valued record's is a
  `Keyword`; `history_records` and `comment_records` read the commentary records.
  Refused: a `String` name, which allocated for each `COMMENT` record a parse read,
  and `comments`, which gave "comment" a second meaning beside the inline comment.
- **The null string is written as `''`.** Any other string pads to 8 characters, as
  cfitsio writes it, but padding the null string would read back as one blank, so it
  stays `''`, as astropy writes it. A text value drops its trailing blanks when it is
  made, as a record holds it.
- **A header signs itself as it renders.** `render_into` takes a `Signing`, which sets
  `DATASUM` and `CHECKSUM` in their place or after the records and patches `CHECKSUM` at
  its offset, and a `Space`, spare records or an exact block count. Refused: a clone
  of the header with the checksum cards set, and a render to count the records before
  the render that fills the blocks.
- **Each edit maintains the header's indexes.** `append` adds one entry, and an
  insert or a removal shifts the positions after it; only a bulk edit rebuilds them.
  Refused: a rebuild on every insert, which made a header built by `append` O(n²);
  and `Extend<Record>` or `FromIterator<Record>`, which must panic on a record that
  does not render, where `insert_record` refuses it.

## Scope

- **No section stride.** A preview reads full-resolution chunks and decimates
  them. Refused: a stride on `Region`, which would complicate the plain and the
  tiled section paths for no caller.
- **No partial reads of an ASCII table.** ASCII tables are a legacy format, and
  every reader of them reads the whole table. Refused: the row, column and cell
  reads of a binary table for them.
- **No `serde` feature.** A caller that stores a fits-well enum keeps its own
  copy, as lumos does with `SampleType`, so a rename here cannot break its saved
  files. Refused: the feature and its dependency.
- **No general split of the header borrow from the data borrow.**
  `ImageReader::header` outlives the handle borrow, which covers the read path.
- **No gzip'd file, non-seekable stream or async input.** These wrap the format
  and are not part of it: the caller inflates a `.fits.gz` and calls
  `FitsReader::from_bytes`. Refused: a gzip reader and an async API, which would add
  a dependency each, and a stream reader, which cannot read the HDUs lazily.
- **No HCOMPRESS `SMOOTH` on write.** `SMOOTH` is a hint for the reader, which
  smooths when a file asks for it. Refused: a write option that stores the hint.
- **No random groups write, public unit grammar or cfitsio tool features.** Random
  groups are deprecated for all data but radio interferometry (§6), and read only; a
  unit is parsed only where a WCS or time axis needs one; row filters, the extended
  file-name syntax and the other cfitsio tool features are programs over the library,
  not the format.

## Time

- **No time-scale conversion.** A time keeps the scale its header declares; a
  conversion needs leap-second, ephemeris and Earth-orientation data from outside the
  file, as ERFA and astropy take them. Refused: built-in tables, which go out of date.
- **A time is an integer MJD day and a fraction:** `time::Mjd`, opaque, with
  `Mjd::new(day, fraction)` moving the fraction into [0, 1). The fraction has a
  spacing of at most 2⁻⁵³ day ≈ 1e-11 s. A relative time joins the reference epoch
  with error-free operations, two-sum, two-product and a division with its exact
  remainder, rounded once into the fraction, as astropy's `day_frac` does
  (`time/utils.py`, after Shewchuk 1997). A day past 2⁵³ in size holds no fraction
  and is refused. Source: §9.2.2. Refused: one `f64` MJD, which at MJD 60000 has a
  spacing of 2⁻³⁷ day ≈ 0.63 µs. Refused: astropy's fraction in [−0.5, 0.5], which
  centres the day on a JD's noon; an MJD day starts at 0 h, so [0, 1) keeps the day
  number of the date. Refused: a JD in one `f64`, which loses more than an MJD.
  Refused: public fields, which let a fraction outside [0, 1) exist.

- **A time unit is parsed once, with its frame.** `Frame::unit` is a
  `time::Unit`: a fixed length, or a multiple of the tropical or Besselian year,
  whose length depends on the epoch and holds only in a TDB or TT frame, so
  `Unit::seconds` evaluates it with the scale in use. A malformed `TIMEUNIT`
  refuses the frame. Refused: the `TIMEUNIT` text, parsed again by every
  conversion, so a bad unit surfaced only at the first one.

- **Time fields say what they are; the docs name their keywords.** `Frame` has
  `reference_mjd`, `unit`, `clock_offset` and `reference_position`; `Bounds` has
  `start`, `end`, `mid`, `exposure`, `elapsed`, `resolution`, `pixel_position`,
  `systematic_error` and `random_error`. Refused: the keyword spellings (`mjdref`,
  `timeoffs`, `xposure`, `timrder`), which mixed with the descriptive ones.

- **A time axis is resolved once, from its `CTYPEi` and `CUNITi`.**
  `Frame::time_axis` gives a `time::Axis`, whose `mjd` takes the world value that
  `Wcs::axis_world` gives. `TIMEOFFS` stays in `TIMEUNIT` and the value in its own unit
  (§9.4.1). Refused: `time_axis_mjd(&Wcs, axis, pixel)`, which parsed the type, the
  scale and the unit on each call and tied the time layer to the WCS layer, and the
  offset scaled by `CUNITi`, which moved `TIMEOFFS = 10` s by 10 days on a day axis.

- **A datetime is valid by construction.** `Datetime` has private fields,
  `Datetime::date(…).at(…)`, and its second as a whole number and a fraction, the
  fraction kept below 1. `to_mjd` needs no scale: a leap second needs a leap-second
  table in every scale (`ExternalTimeDataRequired`). The `DD/MM/YY` form of `DATE`
  before 2000 is read as 1900 + YY, as cfitsio's `ffs2dt`. Source: §9.1.1, §4.4.2.1.
  Refused: public fields, which let 30 February exist, and the second as one `f64`,
  which rounded `59.99999999999999999` to 60.

- **Each bound fails alone.** `Bounds` borrows the header and its `Frame`, and each
  accessor reads only its own keywords, so a leap-second `DATE-END` leaves `XPOSURE`
  readable. Refused: one `from_header` that failed whole and parsed `TIMESYS` for each
  date.

- **Time errors name their case and their keyword:** `InvalidDatetime { keyword }`,
  `InvalidTimeScale { keyword }`, `TimeOutOfRange`, and `MissingKeyword` for a phase
  axis without `CZPHSi`; a conversion that needs data from outside the file is
  `ExternalTimeDataRequired` with its `TimeConversion`. Refused: `InvalidTime
  { detail }`, a message that said `DATE` for every date keyword.

## Crate boundary

- **No re-export of another crate's items.** A caller that names `bitvec`'s
  `BitSlice`, `BitVec` or `Msb0`, or `num_complex`'s `Complex`, depends on that
  crate itself, at the version its workspace pins. Refused: re-exports, which tie
  callers to fits-well's versions. Also refused: types of fits-well's own, as
  these are the ecosystem's standard types.
- **fits-well maps no file.** A caller that wants a mapped file maps it and
  reads it with `FitsReader::from_bytes`, which decodes from the borrowed bytes
  without a copy. Refused: a safe `open_mmap`, which is unsound, because another
  process that truncates or rewrites a mapped file makes its reads undefined.
  `memmap2::Mmap::map` is `unsafe` for that reason. Also refused: an `unsafe fn
  open_mmap`, which would duplicate the caller's own mapping layer (Darkroom's
  `common::MappedFile`) and keep a feature and a dependency.
