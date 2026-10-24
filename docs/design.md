# fits-well design

The decisions that shape the public API, each with the options refused. The code and
its doc comments show how each one works. Sources: the FITS 4.0 standard
(`docs/refs/fits_standard40.md`) and the dated facts in `docs/research/`.

## Reading

- **One handle reads an image.** `FitsReader::image` gives an `ImageReader`, which
  resolves the geometry and a compressed image's codec and tiles once, and reads a
  `Region` three ways: into the caller's words, into an owned `Image`, or physical.
  Refused: a method for each combination of whole or section, owned or borrowed, summed
  or not; a second lazy form of the same read; and a `TableReader`, as four reader
  methods cover a table, which has no checksum pass or caller's words to carry.
- **The caller owns the image-sized buffers.** A view fills the caller's words; the
  reader keeps staging the size of the largest read, and a compressed image's tile
  index; the decode's buffers live only while a read runs. Refused: a staging copy of
  the unit or the section, which never shrank.
- **A plain read lands in the buffer it returns.** A stream reads the stored samples
  straight into it and swaps them in place, and a resident source decodes from its
  borrowed bytes. Refused: a staging copy, measured 7–25 % slower from a stream.
- **A stream reads adjacent ranges as one, and does not seek between them.** A
  selection's buffer grows as a `Vec` grows. Refused: a seek and an exact reservation
  for each range, which made a selection of n rows O(n²).
- **A checksum is summed by the handle that reads, as far as its reads continue the
  sum:** a plain image's reads, a compressed image's tile index and whole reads, and a
  stream's compressed sections in heap order, so the finish reads only what the reads
  did not. A resident source's sections are summed at the finish, in place: summing
  between their decodes left the workers waiting, 16 % on a RICE frame. Refused: a
  caller-held sum, which a caller could finish against another HDU; and a stream's
  sections left unsummed, which read every compressed image twice. Research:
  `2026-10-24-read-bytes.md`.
- **A checksum report gives two verdicts,** valid and invalid, and leaves `Absent` and
  `Unknown` to the caller's policy. Refused: one verdict, which chose that policy for
  every caller.
- **`checksum` owns the one reading of `DATASUM` and `CHECKSUM` and the one running
  sum,** for the reader, the image stream and the editor. A blank value is `Unknown`, as
  cfitsio's `ffvcks` reads it. Refused: a parse in each, which read `''` two ways.
  Research: `2026-10-24-reference-library-practice.md`.
- **A physical value is one fused multiply-add,** `BSCALE.mul_add(raw, BZERO)`, for
  images, columns and groups alike. Refused: cfitsio's product then sum, which rounds
  twice, and astropy's `float32` arithmetic. Research:
  `2026-10-24-reference-library-practice.md`.
- **Every image path reads one scaling, checked when the header is read.** A float
  image ignores `BLANK` (§5.2.5), as cfitsio and astropy do. Refused: a check in
  `Image::new` alone, which made `read` refuse what `view` gave.
- **A physical read appends to the caller's `Vec`,** so chunks append to one plane, and
  random groups do the same. Refused: a slice the caller sizes and zeroes, and a new
  `Vec` for each call.
- **`Borrowed` is the borrowed image, the twin of `Image`,** and the conversions live on
  it alone. Refused: the same conversions on each image type.
- **The six sample types are one sealed trait,** and a generic caller converts with the
  std traits, each failed conversion giving its input back. Refused: a hand-written
  `match` for each operation, and a trait of its own in each caller.
- **A typed view over the `u64` scratch is a safe fn with one release `assert!` of its
  length,** as `bytemuck::cast_slice` checks it. Refused: an `unsafe fn` whose bound was
  a `debug_assert!`.
- **`Hdus` owns the HDU list** — the scan, the selection and the image list — for the
  reader and the editor alike. Refused: a free function over a slice of `Hdu` in each
  caller.
- **A read names its HDU with an `HduSelector`:** an index, or an `EXTNAME` with
  `EXTVER` and `EXTLEVEL`, a primary with none being `PRIMARY`, as in astropy. A name no
  HDU carries is `HduNotFound`. Refused: a second way to each fact about an HDU, and an
  `Option` result for a missing name.
- **`Hdu::is_image` classifies by rank alone** (§4.4.1). Refused: the kind alone, which
  missed a primary with `NAXIS = 0`, and requiring the geometry to resolve, which hid
  its error behind "not an image".
- **An image's geometry has one public source,** the metadata of the HDU or the
  handle, which gives a compressed image's own shape. Refused: public header accessors
  for `BITPIX` and the axes, which give a compressed image's container.
- **`read_data_unit` gives the data bytes without the block fill.** Refused: the padded
  unit, which every caller sliced.
- **A `-TAB` reference selects its extension as any lookup does,** and reads it plain or
  compressed. Refused: a filter on `BINTABLE`, which missed a compressed table.
- **`Ok(None)` means only that an optional value is absent.** An operation that cannot
  do what it names errors. Refused: `Ok(None)` for a miss, which a caller reads as
  absent.
- **An error is an enum of cases.** A size error carries its unit, a read of the wrong
  kind names what it asked for, and a value or text check carries a defect enum.
  Refused: one bare pair of numbers for three units, a variant for each kind, and
  formatted messages.
- **The `Source` trait is sealed and cannot be named.** Refused: exporting it before a
  caller is generic over sources.

## Names

- **A name does not repeat its module, and keeps one meaning in the crate:**
  `image::Borrowed`, `table::Ascii`, `wcs::Description` beside `image::Metadata`.
  Refused: `image::ImageView`, `AsciiTable`, and two `Axis` a caller who names both must
  alias. A leaf name shared under two parents is the norm, as `fmt::Error` and
  `io::Error`.
- **A module holds what is its own.** The root holds the reader, the writer, the editor
  and the error; `error` the enums an error carries; `compress` the codec settings.
  Refused: the helper enums at the root, and compression settings under `image`.
- **A name pairs with its counterpart:** `with_checksums` on both sides, `schema()` on
  both table forms, `view()` and `to_*()` between owned and borrowed, `with_*` for an
  option and `from_*` for a constructor, `*_count` for a count but a FITS keyword's own
  name. Refused: one spelling on each side, which a caller remembers twice.
- **A type read from text implements `FromStr`, and its `Display` is the exact inverse**
  in the canonical spelling. Refused: a `Display` that prints the input as read, which a
  parse does not keep.

## Tables

- **A table selection reads through a `Fetch`,** the file or decoded tiles, and joins
  adjacent rows and heap arrays into one fetch. Refused: a selection in the reader,
  which wrapped decoded bytes in a source of their own.
- **Both table forms parse their schema when the file opens,** and check the structure
  the data unit is sized from (§7.2.1, §7.3.1). Refused: an ASCII schema known only
  after a read.
- **A bad optional column keyword fails only the reads that need it.** Refused: a
  refusal of the whole table, and cfitsio's and astropy's silent skip, which reads the
  column with wrong values. Research: `2026-10-24-reference-library-practice.md`.
- **The characters after a `TFORMn` code are ignored,** as §7.3.1 leaves them undefined
  and cfitsio and astropy skip them. Refused: a refusal, which lost a table that
  carried a writer's suffix.
- **A `Tform` is a repeat, an element kind and a storage.** Refused: one kind enum
  with the descriptors beside the element types, which needed an `unreachable!` in each
  decode.
- **`X` and `PX` decode to bits.** Refused: the packed bytes, which gave a 3-bit and an
  8-bit row one length.
- **A tiled-compressed table reads as the table it restores** (§10.3). Refused: a
  separate read, which made a caller check the kind first, and the container, which
  reads quietly wrong.
- **A partial read decodes only the tiles its rows touch and the columns it selects,**
  through the plain selection. Its cost: a table with arrays restores its whole heap,
  as each descriptor addresses the heap absolutely. Refused: a second selection path,
  and a buffer of the whole restored table.
- **A column is named by one `ColumnSelector`,** an index or a `TTYPEn` name compared
  case-insensitively (§7.2.2). Refused: a pair of methods on each type, and an owned
  name, which allocated on each read.
- **`read_table_columns_into` refills the caller's `Selection`,** so a loop over chunks
  allocates only for the first. Refused: a request struct.
- **Every row collection gives `row`, `iter` and `IntoIterator`, with an exact count.**
  Refused: an impl for each value type, and `get`, `rows` and `iter` for one operation.
- **A table's facts live on its schema.** Refused: a second way to them on `Table`.

## Compression

- **A compressed image's tile index is read once, when its handle first opens,** and
  the reader keeps the index of each image it opens. A section then fetches only its
  tiles' heap cells, a whole read only the heap, and what a read holds is known before
  it starts. Refused: the tile rows read with each section and the parameter columns
  parsed for each read; a bound from `PCOUNT` alone, loose by the tile count; and one
  index for the image opened last, which made a caller that plans an image's
  extensions read the image's index again. The cost: a reader holds each index it read
  until it is dropped, a row of a few bytes a tile, which `view_bytes` counts.
- **A compressed table's tile layout is read once, when the file opens,** and a partial
  read of a table with no arrays fetches only its tiles' container rows. A table with
  arrays reads its whole container, as a tile's descriptors address the container heap.
  Refused: the container read whole for each cell.
- **A tile parameter resolves as its keyword, then its column, then neither,** each on
  its own. Refused: a column only, which decoded a `ZSCALE` keyword with a scale of 1.
  Source: cfitsio `imcomp_get_compressed_image_par`.
- **A float image with no `ZSCALE` and no `ZZERO`, or with `ZQUANTIZ = 'NONE'`, is
  lossless,** its cells gzip'd floats, as cfitsio reads it. Refused: the integer decode
  of those cells. Sources: §10.2; cfitsio `imcomp_init_table`.
- **A float tile decodes in the image's stored type,** a quantized one in `f64` and
  rounded once. Refused: one `f64` plane, which set a signaling NaN's quiet bit and
  doubled an f32 tile's memory.
- **An integer image's null is `BLANK`, else `ZBLANK`** (§10.1.1).
- **The compression keywords resolve once, for the handle,** so a bad keyword fails at
  `FitsReader::image`.
- **A tile set reserves its buffers before its first tile.** Each codec states what
  decoding the largest tile fills, so no buffer grows within a read. Refused: buffers
  that grow as tiles fill them, whose size follows the tiles each worker drew.
- **`ImageReader::view_bytes` bounds a run of views, by owner:** what the reader keeps,
  and what the handle holds while it reads, counting a tile set for each worker that
  may take a run of tiles. It is never below the peak, and equals it on one worker over
  a run that grows each buffer once; `finish_bytes` bounds the finish. Refused: an exact
  figure, which fixed the split of each wave and gave up rayon's balancing, and buffers
  kept between reads, which needed that split. Research: `2026-10-24-read-bytes.md`.
- **gzip reads its own member framing.** A tile set holds one raw-deflate state, made by
  its first gzip tile, and fits-well checks the header and the trailer; a tile inflates
  into a buffer a byte past its size, so a bomb shows without an allocation. Refused: a
  `GzDecoder` for each tile, 75 KB of allocation every time. Source: RFC 1952.
- **A parallel decode writes each band of the image in place,** where there are bands
  enough for the workers; other images and sections decode in waves through one
  buffer. Refused: a `Vec` for each tile, which a whole read paid 9–27 % for.
- **One tile codec carries its parameters,** and each codec decodes over its natural
  input. Refused: five codec enums and six dispatches.
- **A codec lends its output from its scratch,** which the tile decoder swaps into its
  own. Refused: the caller's output buffer as a parameter, and a copy of every tile.
- **An encoder hands its container to the writer in parts:** the header, the rows and
  the heap's cells, each compressed once. A compressed table's heap puts the arrays'
  descriptor blocks last, so their offsets are known before any block is gzip'd in
  parallel. Refused: the tiles copied into one unit, twice the compressed size at the
  peak, and a serial gzip walk.
- **A variable-length column's descriptor block holds the original descriptors, then
  the compressed arrays',** as cfitsio and nom-tam-fits 1.19.1 write it, the reverse of
  §10.3.5. Refused: the standard's order, which cfitsio misreads, and a guess for each
  tile. Sources: cfitsio `imcompress.c`; nom-tam-fits fix #557.
- **A float image's quantization is one opaque `Quantization`:** lossless, relative to
  each tile's noise, or absolute, as fpack's `-q`; HCOMPRESS's scale is the same pair.
  Refused: cfitsio's one number, where 0 is lossless and a negative value absolute, and
  a public enum anyone could build unchecked. Source: `fpackguide.md` §C.
- **`Dither` holds the method and its seed,** so a seed never stands beside no dither.
  Refused: `TryFrom<u16>` for the seed, which does not name its kind.
- **The default seed is the byte sum of every sample, mod 10000, plus 1:** the same
  image gives the same file, and frames with equal first tiles still differ. Its cost:
  one more read of the samples. Refused: the first tile's sum (fpack `-qt`), the clock
  (cfitsio and astropy), and a fixed seed, which fpack warns loses the dither's benefit.
- **The noise estimate is cfitsio's noise2/3/5,** in `float` for an f32 image, so a
  write matches fpack's `ZSCALE` and HCOMPRESS scale bit for bit. Two differences stay:
  each median runs over its own differences, and a level stays an `f64`. Refused: noise3
  alone. Sources: cfitsio `fits_quantize_float`; Stoehr et al., ST-ECF newsletter 42.
- **A quantized float image takes HCOMPRESS_1,** on the quantized integers, and refuses
  `SUBTRACTIVE_DITHER_2` with it, as fpack does. Refused: cfitsio's library change of the
  method with a warning, and `PLIO_1` for floats, whose 24 bits a tile seldom fits.
- **A lossless float write gathers the samples as bytes,** so a NaN's payload comes
  back. Refused: the quantizer's `f64` gather.
- **The float container widens its descriptors to `Q`** past the 32-bit range, as the
  integer container does.
- **A u16 image's `PLIO_1` stream holds its physical values,** stored plus 32768, as
  cfitsio writes and reads it; only the 16-bit case is offset. Refused: astropy's
  refusal of the write and its read with no offset, which misreads cfitsio's files.
  Sources: cfitsio `imcomp_convert_tile_tshort`; astropy `_tiled_compression.py`.
- **A compression error names its case, and a write refuses what it would not keep:**
  a quantization on an integer image, a tile of 0. Refused: messages, and a setting a
  write ignored with no notice.

## World coordinates

- **The `CTYPEia` grammar takes a distortion code** after any projection
  (`RA---TAN-SIP`), as wcslib excises one; an unknown code leaves the axes unsupported.
  Sources: wcslib `wcsset`; Shupe et al. 2005.
- **SIP is a prior distortion, on the primary description only,** turned on by
  `A_ORDER` and `B_ORDER` with or without the `-SIP` code, as wcslib and astropy read it.
  The inverse solves the polynomials by Newton steps with their exact Jacobian, to
  wcslib's tolerance and step limit. Refused: `AP`/`BP` alone as the inverse, an
  approximate fit; astropy's 1e-4 pixel default; and SIP only with the code.
- **TPV is a sequent distortion on the celestial intermediate coordinates,** its
  absent terms 0 but `PVi_1`, so a header with no `PV` is TAN. Refused: TNX, ZPX, TPU
  and TPD, which no darkroom input uses. Source: the TPV registration; wcslib `tpvset`.
- **`Wcs::write_into` writes a description into a header in wcslib's form,** first
  removing its old records, so `from_header` gives back the same transform bit for bit.
  Refused: a new header the caller merges, which leaves `CD` beside the new `PC`; `CD`
  output; and a `PC` and `CDELT` factored from the product, which rounds. Source:
  wcslib `wcshdo`.
- **`Wcs::crop` and `Wcs::bin` derive a new WCS exactly,** the crop from the `Region`
  an image read takes, the bin scaling `PC` and the SIP terms. Refused: astropy's way,
  which scales `CDELT` and keeps the SIP terms, right only for an equal bin with no SIP.
- **The native latitude crosses from the rotation to the projection as its sine and
  cosine,** so a point next to the pole keeps its offset to an ulp. Refused: wcslib's
  `asin` in degrees, which put every point within 8.5e-7° of the pole on it. Research:
  `2026-10-24-wcs-round-trip-precision.md`.
- **A projection's constants are built once, into its family's kernel.** Refused: the
  constants for each point, and one module whose forward and inverse lay apart.
- **An inverse solve stops at the precision of its root and errors when it does not:**
  bracketed Newton with bisection, as Boost's `newton_raphson_iterate`. Refused:
  wcslib's false position, which returns its last guess unchecked, and absolute
  residual stops, which left AIR and MOL thousands of ε off. Sources: Boost.Math
  `tools/roots.hpp`; wcslib 8.10 `prj.c`.
- **MOL solves for the co-auxiliary angle near the poles,** so the colatitude keeps its
  digits. Refused: wcslib's form, 2.3e6 ε off at 1e-5° from the pole.
- **AIT's domain is its ellipse, the poles on its edge,** as wcslib's `aitx2s` takes it.
- **A formula next to its own zero computes the difference first** — a velocity, MER,
  SFL, BON. Refused: wcslib's forms, 32 % off one ulp from the rest value.
- **A 1-D `-TAB` inverse searches the array's monotonic runs,** which the standard
  does not require to be monotonic, in the scan's order. Refused: the scan, 79 µs a
  point over 100 000 entries, and a last-hit cache, which a shared `&self` cannot hold.
  Research: `2026-10-24-tab-inverse-runs.md`.
- **One plan for each axis holds its kind, its transform and its world unit,** parsed
  once. Refused: parallel vectors indexed by axis, and `CTYPE` text compared after the
  parse.
- **A transform writes one point into the caller's slice,** as wcslib writes into the
  caller's arrays, so a point allocates nothing. Refused: a `Vec` for each point, and a
  batch API. Research: `2026-10-24-wcs-into-slice.md`.
- **The inverse linear step solves with the matrix's LU factors,** in place. Its cost:
  four coupled axes invert 34 % slower. Refused: the explicit inverse, which needs a
  buffer for each point. Source: Higham §14.1.
- **A WCS or time read selects its description with `impl TryInto<Alternate>`** (§8.2).
  Refused: `Option<char>`, which let a digit spell another keyword, and a public enum.
- **A WCS error names its case.** Refused: a formatted detail, about 35 messages a caller
  could tell apart only by their text.

## Writing

- **Every image write borrows its samples.** Refused: an owned image, which made a
  caller copy a buffer it held.
- **A data unit is written from a typed body and never staged;** with checksums, a
  first pass sums, the signed header is written, and a second pass writes, as cfitsio
  signs after its data. Refused: a copy of the unit, 96 MB for a 24 MP f32 frame.
- **A stream needs `Seek`, even without checksums.** Refused: a type state for
  checksums, a second type parameter on every method for a sink no caller has.
- **A failed write poisons the writer.** Refused: a retry, which puts the next bytes
  after a partial write and signs a broken HDU.
- **The structural cards are always generated,** and an extension that comes first
  follows an empty primary, which `write_empty_primary` writes with the caller's cards.
  Refused: an `EXTNAME` argument on each write.
- **One `CompressionConfig` holds the codec, the tiles and the quantization.** A table
  takes one tile axis and no quantization. Refused: the codec as a separate argument.
- **A table with no tile height uses cfitsio's tiles of about 10 MB of rows.** Refused:
  one row a tile, which compresses badly. Source: cfitsio `fits_compress_table`.
- **`write_compressed_table` takes the `Builder` that `write_table` takes,** and
  compresses straight from its columns. Refused: the read-side `Table`, cfitsio's
  HDU-to-HDU route, and rows encoded first, which doubled a write's time.
- **A compressed table clamps its codec to one each column's type takes:** GZIP_2 only
  on `I`, `J`, `K`, `E` and `D`, as cfitsio's reader unshuffles only those, and no
  `NOCOMPRESS`, which cfitsio refuses to read. Source: cfitsio `fits_uncompress_table`.
- **An ASCII real column declares its format, with no default,** and every cell has a
  decimal point (§7.2.5). Refused: a default of `F` with 0 decimals. Research:
  `2026-10-24-reference-library-practice.md`.
- **An ASCII text field drops its trailing blanks and keeps its leading ones,** so a
  written column reads back equal. Refused: the padded field, and cfitsio's single
  blank for an all-blank field. Sources: cfitsio `cfitsio.tex`; astropy `fitsrec.py`.
- **A bit column takes rows of bits and their width.** Refused: bytes the caller packed.
- **A variable-length column takes its descriptor width as an argument.** Refused: a
  method that panicked on a fixed-width column.
- **Table columns stay owned,** as the write's transpose dominates. Refused: borrowed
  columns, a lifetime on `Builder` for a small gain.
- **An owned image is built as the borrowed one is,** from a shape, samples and a
  scaling; four unsigned constructors stay, as a write converts to signed values in
  any case.

## Editing

- **`FitsEditor` edits a file that exists, and a header is replaced in place or not at
  all:** it must fit its blocks, keep its data unit and its kind. `replace_header_into`
  writes the edited file to a second sink, as astropy's update mode does. Refused: a
  move of every byte after the header, as cfitsio's `ffiblk` does, which leaves a broken
  file when it stops; and row and column edits, which a copy does.
- **A replaced header that holds `CHECKSUM` gets a valid one,** from its `DATASUM`, else
  from the data, read once.
- **Spare records are kept,** so a later edit fits in place, as cfitsio's `ffhdef`
  reserves space.
- **`append` refuses a file that does not end at its last HDU.** Refused: cfitsio's write
  over the trailing bytes, which loses them. Research:
  `2026-10-24-reference-library-practice.md`.

## Header

- **A keyword is a checked, inline, `Copy` `Keyword`,** and the index keys on it, so a
  lookup cannot miss on its spelling and a parse makes no `String`. Refused: `&str`
  keys, which made `get("naxis")` quietly `None`.
- **Every method that names a record takes `impl TryInto<RecordName>`,** so a keyword, a
  preset, a `HIERARCH` name or a `&str` works, and a `HIERARCH` record has every typed
  getter. Refused: `Keyword` arguments alone, a panicking `From<&str>`, and a second
  family of `*_hierarch` methods.
- **`header::keywords` holds the standard's keywords and the roots of its families.**
  Refused: macros for names, and a constant for each numbered member.
- **A name an axis or column number makes that fits no card is absent,** and one that
  fits but is no keyword panics, as only a wrong pattern makes it. Refused: an error,
  which failed a WCS with a wide column.
- **`require_*` reads a mandatory keyword, `set_optional` writes an optional one.**
  Refused: range-checked getters, and an infallible `set`, as a NaN and non-ASCII text
  are invalid in FITS.
- **A comment edit on an absent card is `MissingKeyword`.** Refused: a no-op, which loses
  a misspelled keyword without a trace.
- **A record is an enum of its kinds,** valued, commentary, `HIERARCH` or malformed, with
  a borrowed `RecordView`. Refused: one struct whose `Option` value told the kinds apart.
- **A near-valid record is repaired and reported; a record with a bad value is kept as
  malformed,** and only a read that needs it fails. Four defects are repaired as cfitsio
  and astropy read them. Refused: a lenient mode, which is modal state; a repair with
  no report; and a refusal of the file for one bad optional record. Sources: cfitsio
  `ffgcrd`, `ffpsvc`; astropy `card.py`.
- **The blank records before `END` are space, not content,** so an edited header fits
  again. Refused: keeping them as commentary.
- **HIERARCH records are typed,** names compared without ASCII case, as cfitsio and
  astropy compare them, and long strings continued on `CONTINUE`. Refused: a `Keyword`
  past 8 bytes, which ends `Copy`; astropy's silent truncation; and ESO's token rule as a
  read check, which makes cfitsio's records malformed. Source: Wicenec, Grosbøl and
  Pence 2009.
- **A commentary record's name is a `CommentaryName`,** any 8 printable bytes, and a
  push refuses a record that would read back as another. Refused: a `String` name,
  which allocated for each record, and a method for each name.
- **The null string is written as `''`,** as astropy writes it; any other pads to 8.
- **A header signs itself as it renders.** Refused: a clone with the checksum cards set,
  and a render to count the records first.
- **Each edit maintains the header's indexes.** Refused: a rebuild on each insert, which
  made a header built by `append` O(n²).

## Time

- **No time-scale conversion.** It needs leap-second, ephemeris and Earth-orientation
  data from outside the file. Refused: built-in tables, which go out of date.
- **A time is an integer MJD day and a fraction in [0, 1),** joined by error-free
  operations, as astropy's `day_frac` (§9.2.2). Refused: one `f64` MJD, 0.63 µs apart at
  MJD 60000; astropy's fraction in [−0.5, 0.5], centred on a JD's noon; and public
  fields.
- **A time unit is parsed once, with its frame, and keeps its spelling;** a year's length
  follows the epoch and the scale. Refused: the text parsed by each conversion, and the
  length alone, which `ks` and `10**3 s` share.
- **A time axis is resolved once, from its `CTYPEi` and `CUNITi`,** and `TIMEOFFS`
  stays in `TIMEUNIT` (§9.4.1). Refused: a parse on each call, and an offset scaled by
  `CUNITi`, which moved 10 s by 10 days.
- **A datetime is valid by construction** (§9.1.1), its second a whole number and a
  fraction; `DD/MM/YY` before 2000 reads as 19YY, as cfitsio's `ffs2dt`. Refused: public
  fields, and the second as one `f64`.
- **Each bound fails alone,** reading only its own keywords. Refused: one parse that
  failed whole.
- **Time fields say what they are.** Refused: the keyword spellings as field names.

## Scope

- **No section stride.** A preview reads full-resolution chunks and decimates them.
- **No partial reads of an ASCII table,** a legacy format every reader reads whole.
- **No `serde` feature.** A caller keeps its own copy of a stored enum, so a rename here
  cannot break its files.
- **No gzip'd file, non-seekable stream or async input.** They wrap the format: the
  caller inflates a `.fits.gz` and calls `from_bytes`. Refused: a dependency each, and a
  stream reader, which cannot read the HDUs lazily.
- **No HCOMPRESS `SMOOTH` on write,** a hint for the reader.
- **No general split of the header borrow from the data borrow.** The handle's header
  outlives its borrow, which covers the read path.
- **No random-groups write, public unit grammar or cfitsio tool features.** Random
  groups are deprecated (§6); a unit is parsed where an axis needs one; row filters and
  the extended file-name syntax are programs over the library.

## Crate boundary

- **No re-export of another crate's items.** A caller that names `bitvec` or
  `num_complex` types depends on that crate. Refused: re-exports, which tie callers to
  fits-well's versions.
- **fits-well maps no file.** A caller maps it and reads it with `from_bytes`. Refused:
  a safe `open_mmap`, unsound when another process truncates the file, and an `unsafe`
  one, which duplicates the caller's mapping layer.
