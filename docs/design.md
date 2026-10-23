# fits-well design

The decisions that shape the public API. The code and its doc comments show how
each one works. Sources: the FITS 4.0 standard (`docs/refs/fits_standard40.md`)
and the dated measurements in `docs/research/`.

## Reading

- **One handle reads an image: `FitsReader::image` gives an `ImageReader`.** A
  `Region` picks the whole image or one range per axis, and the handle reads it
  three ways: `view` into the caller's `words`, `read` into an owned `Image`, and
  `physical_into`. The handle resolves the geometry and a compressed image's codec
  and tiles once. Refused: one method per combination of whole or section, owned
  or borrowed, and summed or not (six methods, with the checksum as a suffix), and
  `Lazy`, a second form of the same read.
- **A checksum is summed by the handle that reads.** With `with_checksum`, a read
  feeds the stored bytes it fetches, as long as they continue the sum: a plain
  image's whole reads and sections, and a compressed image's whole reads. A
  compressed section's tiles are not fed. `finish` reads only what the reads did
  not feed, and without `with_checksum` it reads the whole unit. Refused: a caller-held sum passed to each read, which a caller could finish
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
  the handle is `fits_well::ImageReader`, beside `FitsReader`. Refused:
  `image::ImageView` and `image::ImageReader`.
- **A read names its HDU with an `HduSelector`:** an index, an `EXTNAME`
  (case-insensitive), or a name with `EXTVER` and `EXTLEVEL`, each 1 when absent.
  A primary with no `EXTNAME` is `PRIMARY`, as in astropy. Every reader method
  takes `impl Into<HduSelector>`, and `Hdu::matches` answers the same selector,
  so a caller does not compare `EXTNAME` by hand. A name no HDU carries is
  `HduNotFound`, and `images()` iterates the image HDUs. Refused: a separate
  `hdu_index` lookup before each read, an `Option` result for a missing name, and
  `first_image()`, which is `images().next()`.
- **`Hdu::is_image` classifies by rank alone.** An HDU is an image when it is a
  primary array, an image extension or a tiled-compressed image whose `NAXIS`
  (`ZNAXIS`) is above 0. Refused: the kind alone, which counted an image
  extension with `NAXIS = 0` but not such a primary. Also refused: requiring the
  whole geometry to resolve, which would turn a malformed image into "not an
  image" and hide the error that `Hdu::image_metadata` reports. Source: §4.4.1.
- **A checksum report gives two verdicts:** `is_valid`, both checksums present
  and verified, and `has_invalid`, either one malformed or contradicted by its
  bytes. A caller's policy for `Absent` and `Unknown` stays its own. Refused: a
  single verdict, which would choose that policy for every caller.
- **The `Source` trait is sealed and cannot be named.** A source is not an
  extension point, and no caller is generic over sources. Refused: exporting it
  before such a caller exists.

## Tables

- **A tiled-compressed table reads as the table it restores.** `read_table`,
  `read_table_rows`, `read_table_columns`, `read_table_cell` and `table_schema`
  give the logical table, as an image read gives a compressed image's pixels. A
  compressed image is `NotABinTable` to each. Refused: a separate
  `read_compressed_table`, which made a caller check the kind first, and the
  container, a table of compressed cells that reads quietly wrong. Source: §10.3.
- **A partial read decodes only the tiles its rows touch, and only the columns it
  selects,** as an image section decodes only its tiles. The touched tiles and
  the heap decode into a table of their own, which the plain selection reads.
  Refused: a second selection path over the tiles, and a buffer of the whole
  restored table. The cost: a table with variable-length columns restores its
  whole heap, because each descriptor addresses the heap by its absolute
  offset.
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
- **`ColumnData` gives its values typed:** an `as_*` accessor for each variant,
  and `TryFrom<ColumnData> for Vec<T>`, which moves the values. Refused: a
  `let … else` on the variant at each caller.
- **Table types are named for their module:** `table::Table`, `table::Ascii`,
  `table::Builder` and `table::AsciiBuilder`. A builder from a row count and its
  columns is `with_columns`, and `RandomGroups::group` gives one group. Refused:
  `AsciiTable` and `TableBuilder`, which repeat the module, as `image::ImageView`
  would.
- **One name for an element type and a count across the crate:**
  `ColumnData::U8`, as `Data::U8`, and `len` with `is_empty`, as `Data` and
  `AsciiColumnData` have. Refused: `ColumnData::Bytes` and `element_count`.

## World coordinates

- **A transform writes one point into the caller's slice.** `pixel_to_world` and
  `world_to_pixel` take the input and the output as slices of one value per
  axis, so a caller loops over its points with one buffer and a point makes no
  allocation unless an axis is `-TAB`. Refused: a new `Vec` for each point, and
  a batch API, which the loop makes unnecessary. wcslib also writes into the
  caller's arrays. Source: `docs/research/2026-10-23-wcs-into-slice.md`.
- **The inverse linear step solves with the matrix's LU factors,** in place in
  the output slice. Refused: the explicit inverse, whose product needs its input
  kept apart from its output, so a buffer for each point. Solving is as accurate
  as the product with the inverse or better (Higham, §14.1), and it keeps the
  division for each row rather than a reciprocal. The cost: four coupled linear
  axes invert 34% slower, four dependent divisions where the product was one
  pass. Source: `docs/research/2026-10-23-wcs-into-slice.md`.
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
  first. Also refused: a builder from a read `Table`; `read_data_raw` and
  `write_raw_hdu` copy an HDU as it is. Sources: cfitsio `imcompress.c`, rustfits
  `CompressedTableHDU`.
- **A table compresses straight from the builder's columns.** Each tile gathers its
  column's cells from the `Builder`, through the `TableCells` trait, and computes
  the descriptors the plain table would hold. Refused: encoding the rows first and
  gathering each column back from them, cfitsio's route, which it takes because its
  input is a file of rows; it doubled the time of a write.
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
- **The unsigned constructors stay four.** `Image::from_u16`, `from_u32`,
  `from_u64` and `from_i8` take any shape `Image::new` takes. Refused: one
  `from_unsigned` over a borrowed view; the write converts the samples to signed
  values and `BZERO` in any case, so it saves no copy.

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
- **A keyword error names the keyword.** `MissingKeyword`, `KeywordOutOfRange`,
  `KeywordNotFound`, `ReservedKeyword` and `InvalidHeaderValue` carry a `Keyword`, and a family is
  reported by its member, `TDIM3`, not `TDIMn`. Refused: `&'static str` names,
  which a run-time keyword could not use.
- **A name formatted from an axis or column number that is no keyword is
  absent.** The long spelling of a table-WCS family with a wide column (`TPV123_45`)
  can be on no card, so the WCS reads treat it as absent, as the `&str` lookup
  did. Refused: an error, which failed a WCS with a wide column.
- **`require_*` reads a mandatory keyword, `set_optional` writes an optional
  one.** `set_optional(k, None)` removes every record with `k`, so the header
  holds the value or nothing. Refused: range-checked getters, whose ranges belong
  to the caller's domain; `set_with_comment`, which is `set(…)?.set_comment(…)`;
  and an infallible `set`, as a NaN real and non-ASCII text are invalid in FITS.
- **Editing the comment of an absent card fails with `KeywordNotFound`.**
  Refused: a no-op, which loses a misspelled keyword without a trace.
- **A commentary card keeps its name as text.** The standard lets a record with
  no value indicator start with any 8 bytes, which no `Keyword` holds, so `Entry`
  gives the name as `&str`, and `history` and `comments` read the commentary
  records.

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
