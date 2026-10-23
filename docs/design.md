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
  (`TformKind::U8`/`Character` with `ColumnData::U8`/`Character`,
  `AsciiKind::Text` with `AsciiColumnData::Text`); an owned type lends with `view()`
  and its view owns again with `to_*()` (`Image`/`Borrowed::to_image`,
  `Data`/`View::to_data`); a builder option is `with_*` and a constructor `from_*`.
  A type read from text implements `FromStr` (`Scale`, `Tform`, `Datetime`,
  `time::Unit`), so it parses with `str::parse`; an inherent `parse` stays for input
  that is not text (`Header::parse(bytes)`, `Schema::parse(header)`).
  A count is `*_count` (`row_count`, `bit_count`), except a FITS keyword's own name
  (`gcount`, `pcount`). Refused: one spelling on each side, which a caller must
  remember twice, and `nrows`, a third spelling of a count.
- **A read names its HDU with an `HduSelector`:** an index, an `EXTNAME`
  (case-insensitive), or a name with `EXTVER` and `EXTLEVEL`, each 1 when absent.
  A primary with no `EXTNAME` is `PRIMARY`, as in astropy. Every reader method
  takes `impl Into<HduSelector>`, and `Hdu::matches` answers the same selector,
  so a caller does not compare `EXTNAME` by hand. A name no HDU carries is
  `HduNotFound`, and `images()` iterates the image HDUs as `&Hdu`, as `hdus()`
  gives them. A fact about an HDU is read from the `Hdu` that `hdu()` gives: its
  `index`, its `table_schema()`. Refused: `hdu_index` and a reader-level
  `table_schema`, second ways to the same fact; an `Option` result for a missing
  name; and `first_image()`, which is `images().next()`.
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
- **`Ok(None)` means only that an optional value is absent.** A header getter gives
  it for an absent keyword, and an accessor such as `Value::as_real` or
  `ColumnReader::unsigned` for a value not in that form. An operation that cannot
  do what its name says errors instead: `parameter_physical` with a name no
  parameter carries (`GroupParameterNotFound`), and `time_axis_mjd` on an axis
  that is no time axis (`NotATimeAxis`). Refused: `Ok(None)` for a miss, which a
  caller reads as "absent" and goes on.
- **The `Source` trait is sealed and cannot be named.** A source is not an
  extension point, and no caller is generic over sources. Refused: exporting it
  before such a caller exists.

## Tables

- **A tiled-compressed table reads as the table it restores.** `read_table`,
  `read_table_rows`, `read_table_columns`, `read_table_cell` and `Hdu::table_schema`
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
- **A table's facts live on its schema.** `Schema` and `AsciiSchema` give the row
  count, row width, columns and `column_index`; `Table` and `Ascii` give the schema
  and read columns. A builder adds a column with `push` and is built whole with
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
- **Every `Ragged` gives `row(r)` and `rows()`,** whatever its values: a slice, a
  `&str`, a `&BitSlice`, or for a variable-length column a `ColumnView`, the
  borrowed twin of `ColumnData` as `View` is of `Data`. Refused: `range(r)` and
  `values()` as the only way to a variable-length row.
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

## Compression

- **A float image with no `ZSCALE` and no `ZZERO` column, or with `ZQUANTIZ =
  'NONE'`, is lossless.** Its `COMPRESSED_DATA` cells hold GZIP_1 or GZIP_2 big-endian
  floats at `ZBITPIX` width, and another codec on such an image is
  `UnsupportedCompression`. cfitsio treats the absent columns as `'NONE'` whatever
  `ZQUANTIZ` says, and allows only GZIP for it. Refused: the integer decode of those
  cells, which read the float bits as quantized integers. Sources: §10.2; cfitsio
  `imcomp_get_compressed_image_par`, `imcomp_init_table`.
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
- **A transform writes one point into the caller's slice.** `pixel_to_world` and
  `world_to_pixel` take the input and the output as slices of one value per
  axis, so a caller loops over its points with one buffer and a point makes no
  allocation unless an axis is `-TAB`. Refused: a new `Vec` for each point, and
  a batch API, which the loop makes unnecessary. wcslib also writes into the
  caller's arrays. Source: `docs/research/2026-10-24-wcs-into-slice.md`.
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
  input is a file of rows; it doubled the time of a write.
- **A bit column takes rows of bits and their width,** as a text column takes
  fields and theirs: `WriteColumn::bits(name, rows, width)`, packed inside. A row
  of another length is `BitFieldLengthMismatch`: unlike a text field, a bit row has
  no padding to fill. Refused: bytes the caller packed, which `vla_bits` never asked
  for.
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
  Refused: `new` and `new_scaled`, two constructors where one takes the scaling.
- **The unsigned constructors stay four.** `Image::from_u16`, `from_u32`,
  `from_u64` and `from_i8` take any shape `Image::new` takes. Refused: one
  `from_unsigned` over a borrowed view; the write converts the samples to signed
  values and `BZERO` in any case, so it saves no copy.

## Editing

- **`FitsEditor` edits a file that exists**, over `Read + Write + Seek`, beside
  `FitsReader` and `FitsWriter`. An edit in place parses the header it writes and
  reads no other, as the other HDUs do not move.
  `replace_header` writes a header in place when it fits the HDU's header blocks, the
  free space before `END` blank records, and is `HeaderDoesNotFit` otherwise; the new
  header must give the same data unit. A header that holds `CHECKSUM` gets a valid one:
  from its `DATASUM`, trusted as it was before the edit, else from the data unit, read
  once, also for a blank `DATASUM`, the standard's unknown sum. `replace_header_into`
  writes the whole edited file into a second sink, which the caller renames over the
  first, as astropy does in update mode.
  `FitsWriter::with_spare_records(n)` writes `n` blank records before each `END`, so a
  later edit fits in place, as cfitsio's `ffhdef` reserves space; "spare", as
  "reserved" names the standard's reserved keywords. `append` consumes the editor and
  gives a `FitsWriter` at the end of a file that ends on a block boundary, so two
  handles never write one file. Refused: a move of every byte after the header, as
  cfitsio (`ffiblk`) and fitsio-pure do, a large write that leaves a broken file when
  it stops in the middle; and row and column insert and delete, which a copy does.

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
  `ReservedKeyword` and `InvalidHeaderValue` carry a `Keyword`, and a family is
  reported by its member, `TDIM3`, not `TDIMn`. Refused: `&'static str` names,
  which a run-time keyword could not use.
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
  standard calls one a keyword record: `iter()` gives views, `remove` the owned
  record. Each is an enum of the two kinds a record is: `Valued` with its
  `Keyword`, value and inline comment, and `Commentary` with its name and text.
  An inline comment reads with `get_comment`, the twin of `set_comment`. Refused:
  `Entry`, a name unrelated to its owned twin; one struct whose `Option` value
  told the kinds apart and whose `comment` meant two things; and a `record(k)`
  lookup that could only give the valued kind.
- **A near-valid record is repaired and reported; a record with a bad value is kept as
  malformed.** `Header::parse` repairs three defects, each a `Repair` that
  `Header::repairs()` lists by the record's 1-based position in the header unit as
  read: a lowercase name reads as its uppercase keyword; a value indicator `=` before
  byte 9, or in byte 9 with no blank in byte 10, reads the value that follows it; a
  non-ASCII byte in a comment or commentary text reads as `?`. A repaired record is an
  ordinary record after that, so a write gives the corrected record. A record whose
  name is no keyword, or whose value does not parse or holds a non-ASCII byte, is
  `RecordView::Malformed` with its 80 bytes: a lookup of its keyword and a write of its
  header fail with `InvalidValue`, and `Header::set` replaces it. So a read fails only
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
  `HeaderCardTooLong`. Refused: a `Keyword` longer than 8 bytes, which would end
  `Keyword: Copy`; astropy's silent truncation of a long comment; and ESO's token rule
  (`A`–`Z`, `0`–`9`, `-`, `_`) as a read check, which would make the HIERARCH records
  cfitsio and astropy write malformed. Sources: Wicenec, Grosbøl and Pence, "The ESO
  HIERARCH Keyword Conventions" (2009); cfitsio `ffgcrd`, `ffmkky`; astropy `card.py`.
- **A commentary card keeps its name as text.** The standard lets a record with
  no value indicator start with any 8 bytes, which no `Keyword` holds, so
  `RecordView::Commentary` gives the name as `&str`, while a valued record's is a
  `Keyword`; `history` and `comments` read the commentary records.

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
  `start_mjd`, `end_mjd`, `mid_mjd`, `exposure`, `elapsed`, `resolution`,
  `pixel_position`, `systematic_error` and `random_error`. Refused: the keyword
  spellings (`mjdref`, `timeoffs`, `xposure`, `timrder`), which mixed with the
  descriptive ones.

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
