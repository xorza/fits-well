# Proposal: close the gaps the library comparison found

Source: `.notes/fits-well-comparison.md` (2026-10-23) and the open issues in
`.notes/ISSUES.md`. References: the FITS 4.0 standard (`docs/refs/fits_standard40.md`);
cfitsio 4.7.0 (`imcompress.c`, `quantize.c`, `getkey.c`, `fitscore.c`,
`docs/fpackguide.md`); astropy at 6edc126 (`io/fits`, `wcs`, `time`); wcslib as astropy
vendors it (`cextern/wcslib/C`: `dis.c`, `dis.h`, `wcs.c`, `wcshdr.c`, `wcspih.l`);
astrometry.net (`util/sip.c`); Shupe et al. 2005 (SIP); the TPV registration at
fits.gsfc.nasa.gov/registry/tpvwcs; Calabretta et al., WCS Paper IV draft (the terms
prior and sequent distortion, as wcslib uses them); and Wicenec, Grosbøl and Pence,
"The ESO HIERARCH Keyword Conventions" (2009). The sources are cloned in
`/tmp/fits-compare`, and each file:line reference is to those revisions. A clone in
`/tmp` does not survive a restart: clone again at the revisions named here.

This proposal is temporary. When an item lands, its decisions move into
`docs/design.md` without their "Chosen by the owner" notes, and the item is deleted
here. Every batch ends with `cargo drive check` and `cargo drive test`. Where an item
changes a public type, the darkroom callers change with it. Today no darkroom crate
outside fits-well calls an API this plan changes, except the compression write in
`lumos/src/io/fits/decode/tests.rs`.

Local reference tools: `fpack`, `funpack` and `fitsverify` (cfitsio 4.7) are in
`/usr/bin`. astropy and wcslib are not installed, and an install needs the owner's
approval. A fixture made with a tool records its command line beside its test, as the
current fixtures do.

Vocabulary in this plan, as in `docs/design.md`: a header holds **records** (`Card` is
only the internal type); a **blank record** has a blank name and no text; a **codec**
is a `ZCMPTYPE` or `ZCTYPn` algorithm; a writer writes to a **sink**; a SIP polynomial
is a **prior distortion** (before the matrix) and a TPV polynomial a **sequent
distortion** (after the matrix).

## Decisions

### Compression

- **A float image with no `ZSCALE` and no `ZZERO` column, or with `ZQUANTIZ = 'NONE'`,
  is lossless.** Its `COMPRESSED_DATA` cells hold GZIP_1 or GZIP_2 big-endian floats at
  `ZBITPIX` width, as the `GZIP_COMPRESSED_DATA` fallback does. Another codec on such an
  image is `UnsupportedCompression`. Source: §10.2 ("the absence of these two columns
  … losslessly compressed"); cfitsio treats the absent columns as `'NONE'` whatever
  `ZQUANTIZ` says (`imcompress.c:5307-5318`), and allows only GZIP for it (`:985-991`,
  `:6599-6611`). Refused: the present path, which reads the float bits as quantized
  integers.
- **A float image's quantization is one opaque type, `Quantization`:**
  `Quantization::NONE` for lossless (`ZQUANTIZ = 'NONE'`), `Quantization::relative(level,
  dither)` for a step of each tile's noise divided by `level`, and
  `Quantization::absolute(step, dither)` for one step for every tile. Both constructors
  refuse a value that is not finite and positive. The names follow fpack's "relative to
  the measured noise" and "absolute" (`fpackguide.md` §C), and the `NONE` constant
  follows `ZQUANTIZ`, as `Scaling::IDENTITY` and `Compression::GZIP` are constants.
  Refused: cfitsio's one number, where 0 is lossless and a negative value is an absolute
  step (`quantize.c:128-157`). Refused: fits-well's present level 0, which means "the
  default of 4". Both put a mode into a magic value. Refused: an enum with public
  variants, which anyone could build with an unchecked value, as `docs/design.md`
  refuses for `Alternate`.
- **HCOMPRESS takes the same two scales:** `Hcompress::relative(scale)` multiplies each
  tile's noise, and `Hcompress::absolute(scale)` is one scale for every tile. They
  replace `Hcompress::lossy`, which is the relative form. `Hcompress::default()` stays
  lossless. Source: fpack `-s` takes a negative value as the absolute scale
  (`fpackguide.md` lines 278-290), and cfitsio applies it (`imcompress.c:2083-2103`).
- **`Dither` holds the method and its seed, so no state is invalid:** `Dither::None`
  (`NO_DITHER`), `Dither::Subtractive1(seed)` and `Dither::Subtractive2(seed)`. It
  replaces the public `DitherMethod`, whose seed had no place. `DitherSeed` is opaque:
  `DitherSeed::FROM_SAMPLES` is the default, and `DitherSeed::fixed(n)` refuses `n`
  outside 1 to 10000. Refused: a seed beside `Dither::None`, where it means nothing.
- **The default seed comes from all the image's sample bytes:** `ZDITHER0 =
  (Σ bytes of every sample, in a u64, mod 10000) + 1`. A byte sum does not depend on the
  byte order. The same image gives the same file, and frames whose first tiles are
  equal, such as registered frames with blank edges, still get different seeds. The
  reader takes any seed from 1 to 10000, so the rule has no effect on interop. The cost:
  one more read of the samples before the tiles compress. Refused: the first tile's
  byte sum, which fpack `-qt`, astropy's `DITHER_SEED_CHECKSUM` and fitsio-pure use
  (`imcompress.c:2810-2821`, `compressed.py:657-673`), as equal first tiles give equal
  seeds. Refused: the clock, cfitsio's and astropy's default (`imcompress.c:2788-2807`,
  `settings.py:28`), which writes a different file on each run. Refused: no default,
  which makes every caller choose. Refused: the present fixed seed of 1, which gives
  each image the same dither pattern; fpack warns that this loses the benefit of dither
  when images are added (`fpackguide.md` lines 213-230). Chosen by the owner,
  2026-10-23.
- **The noise estimate is the smallest nonzero value of noise2, noise3 and noise5**,
  for the float quantization and for the HCOMPRESS scale, integer and float. Source:
  cfitsio `quantize.c:141-144` (`FnNoise5_float`) and `imcompress.c:2090-2096`
  (`fits_img_stats_int`). Refused: noise3 alone, which gives a larger step than fpack
  and so loses more data.
- **A u16 image in PLIO_1 stores its physical values.** The writer adds 32768 to the
  stored values when `ZBITPIX = 16` and `BZERO = 32768`, and the reader subtracts it.
  Source: cfitsio `imcompress.c:2343-2362`, `:2436-2466`, `:6494-6497`. The cost:
  cfitsio also subtracts 32768 from a PLIO image with `ZBITPIX = 32` and `BZERO =
  32768`, which its own writer does not offset. fits-well offsets only the 16-bit case,
  so it reads that rare file differently from cfitsio, and correctly. Refused:
  astropy's way, which refuses a u16 PLIO write and reads with no offset
  (`hdu/compressed/_tiled_compression.py:548-559`), so it misreads the u16 PLIO files
  that fpack writes.
- **A quantized float image takes HCOMPRESS_1**, and the HCOMPRESS scale applies to the
  quantized integers, as in cfitsio. `Dither::Subtractive2` with HCOMPRESS is
  `InvalidCompressionParameter`. Source: fpack refuses `-qz` with `-h`
  (`fpackguide.md` `-qz`). Refused: cfitsio's library way, which changes the method to
  `SUBTRACTIVE_DITHER_1` and prints a warning (`imcompress.c:1327-1333`), a change the
  caller did not ask for. PLIO_1 for floats stays refused: a quantized tile seldom fits
  PLIO's 24-bit range.
- **The float container promotes to `Q` descriptors** past the 32-bit range, as the
  integer container does. Refused: the present `DataUnitOverflow`.
- **A compressed table takes GZIP_2 only on `I`, `J`, `K`, `E` and `D` columns.** The
  table's `Compression` is a preference that each column clamps to a codec its type
  takes, as `pick_algo` already does for RICE_1 after cfitsio
  (`imcompress.c:8255-8282`). The clamp now gives GZIP_1 to `L`, `X`, `A`, `B`, `C` and
  `M` columns when GZIP_2 is asked, because cfitsio's reader unshuffles only those five
  types (`imcompress.c:9028-9175`), and a shuffle of 1-byte values does nothing.
  Refused: GZIP_2 on `C` and `M`, which cfitsio's writer emits but its own reader
  refuses.
- **`write_compressed_table` refuses `Compression::None`** with
  `InvalidCompressionParameter`. A table with no compression is `write_table`. Refused:
  `ZCTYPn = 'NOCOMPRESS'`, which the standard permits (Table 36) and cfitsio refuses to
  read (`imcompress.c:8870-8887`). Refused: a clamp to GZIP_1 at deflate level 0, which
  compresses nothing and still writes a gzip stream for each tile. Chosen by the owner,
  2026-10-23.

### Header

- **A near-valid record is repaired and reported; a record with a bad value is kept as
  malformed.** `Header::parse` repairs three defects. A lowercase name reads as its
  uppercase keyword. A value indicator `=` before byte 9, or in byte 9 with no blank in
  byte 10, reads the value that follows it. A non-ASCII byte in a comment or in
  commentary text reads as `?`. Each repair is a `Repair { record, kind }`, where
  `record` is the 1-based position of the record in the header unit as read, and
  `Header::repairs()` lists them; a later edit of the header does not change this
  report of the input. A repaired record is an ordinary record after the parse, so a
  write gives the corrected record. A record whose value does not parse, whose value
  holds a non-ASCII byte, or whose name is no keyword after the repair, is
  `RecordView::Malformed { raw, defect }` (and `Record::Malformed`), with its 80 bytes.
  A lookup of its keyword returns `InvalidValue`, so a read fails only when it needs
  that record, and a write of the header returns `InvalidValue` too. A defect in a
  keyword that sizes the HDU (`SIMPLE`, `XTENSION`, `BITPIX`, `NAXIS`, `NAXISn`,
  `GROUPS`, `PCOUNT`, `GCOUNT`) still fails the open, because the next HDU cannot be
  found without it; a defect in a table's `TFORMn` fails only a read of that table.
  Sources: cfitsio finds a lowercase keyword (`getkey.c:577-600`), reads a value after a
  misplaced `=` (`fitscore.c:1470-1475`), and parses values only on request
  (`getkey.c:3203-3206`); astropy uppercases the name and accepts an early `=`
  (`card.py:705-740`), replaces non-ASCII with `?` with a warning (`util.py:258`), and
  repairs on write only when asked. Refused: a lenient mode, which is modal state.
  Refused: a repair without a report. Refused: `Malformed` for every defect, which
  leaves a lowercase `exptime` out of reach by name. Refused: the present behaviour,
  where one bad optional record in any HDU fails the whole file. Chosen by the owner,
  2026-10-23.
- **The blank records just before `END` are space, not content.** `Header::parse`
  drops them, and counts them as the header's spare records. A header that a reader
  gives back to `FitsEditor::replace_header` with new records then fits in the space
  those blanks held. Source: astropy uses the trailing blank records for new records
  (`header.py:1091`, `append(useblanks=True)`), and cfitsio writes the space that
  `ffhdef` reserves as blank records before `END`, which it puts in the first record of
  the header's last block (`fitscore.c:7012-7030`). Refused: blank records kept as commentary, which make every edited header
  one record larger than its space.
- **HIERARCH records are typed.** `HIERARCH ESO DET CHIP1 NAME = 'x' / c` reads as
  `RecordView::Hierarch { name, value, comment }` (and `Record::Hierarch`). The name is a
  checked `HierarchName`: one or more tokens of printable ASCII other than `=`, joined by
  one blank, because the blanks between tokens carry no meaning. Two names are equal
  when they are equal without regard to ASCII case, as cfitsio (`getkey.c:512-545`) and
  astropy (`card.py:598-602`) compare them; the name keeps the case it was read with.
  `Header::get_hierarch` and `Header::set_hierarch` take `impl TryInto<HierarchName,
  Error: Into<FitsError>>`, as the keyword methods take `impl TryInto<Keyword>`, and look
  the name up in a second index (`HashMap<HierarchName, usize>`, built on parse; a
  HIERARCH header is a cold path). A long string value continues on `CONTINUE` records on
  read and on write, as astropy writes it (`card.py:1025-1034`). `set_hierarch` writes
  `HIERARCH name = value`, and drops the blank before `=` only when the record needs
  that byte, as cfitsio and astropy do (`fitscore.c:1116-1122`, `card.py:1016-1023`). A
  comment that does not fit is `HeaderCardTooLong`, as for every other record. The ESO
  convention restricts ESO's own tokens to `A`–`Z`, `0`–`9`, `-` and `_`, but cfitsio
  accepts any characters in a HIERARCH name (`fitscore.c:1679-1683`), and so does
  astropy; fits-well reads and writes what they do. Refused: a `Keyword` longer than 8
  bytes, which would end `Keyword: Copy`. Refused: astropy's silent truncation of a long
  comment. Refused: ESO's token rule as a read check, which would make the HIERARCH
  records that cfitsio and astropy write malformed.

### WCS

- **The `CTYPEia` grammar takes a distortion code.** A 12-character value with `-` in
  byte 9 carries a distortion code after the projection, in the 4-3-3 form: `RA---TAN-SIP`
  is the head `RA`, the projection `TAN` and the distortion `SIP`, and any projection
  can carry it. `RA---TPV` is the projection `TAN` with the distortion `TPV`. Source:
  wcslib excises the code for any 12-character type (`wcs.c:3020-3030`); Shupe et al.
  2005 §2; the TPV registration.
- **SIP is a prior distortion, from its keywords, on the primary description only.**
  `(u, v) = p − CRPIX`, then `U = u + f(u, v)` and `V = v + g(u, v)`, with `A_p_q` and
  `B_p_q` for `p + q ≤ A_ORDER` and `B_ORDER`, up to order 9, before the matrix. An order
  above 9 is `KeywordOutOfRange`: wcslib reads one digit for `p` and `q`
  (`wcspih.l:1874-1886`). The keywords `A_ORDER` and `B_ORDER` turn SIP on, with or
  without the `-SIP` code, as wcslib (`wcspih.l:1884`) and astropy (`wcs.py:1455-1478`)
  do; `Metadata` gives the distortion, so a caller sees that SIP applies. A `-SIP` code
  with no `A_ORDER` is `MissingKeyword`. SIP keywords carry no alternate letter, so an
  alternate description takes no SIP, as in wcslib (`wcspih.l:2693-2695`).
  The inverse solves the forward polynomial with Newton steps (the polynomial's
  Jacobian is exact and cheap), from `AP` and `BP` as the first guess when present, else
  from `(U, V)`. It stops when the residual `U − (u + f(u, v))` is at most 1e-13 on each
  axis, absolute below 1 and relative to `|U|` above 1, and gives `WcsNoConvergence`
  after 30 steps. These are wcslib's rule and limit (`dis.c:1260`, `:1339-1359`,
  `disitermax` 30): an absolute 1e-13 is under the spacing of a double above about 450,
  so the relative form takes over there. wcslib also uses `AP` and `BP` only as the
  first guess (`dis.h` "SIP"). Refused: `AP` and `BP` alone as the inverse, an
  approximate fit, so a round trip would not give back the input pixel; astrometry.net
  does this (`util/sip.c:256-263`). Refused: astropy's `all_world2pix` default of 1e-4
  pixel, a tolerance for speed. Refused: a fixed absolute 1e-12 pixel, which a
  coordinate past about 4500 can never meet. Refused: SIP only with the `-SIP` code,
  which would read the many files that astropy reads as SIP without their distortion.
- **TPV is a sequent distortion, on the intermediate coordinates in degrees:** `ξ` and
  `η` are the registered 40-term polynomials in `PV` on the two celestial axes. On a
  `-TPV` description, every `PVi_m` on the longitude and the latitude axis is a
  polynomial term, so the celestial transform takes the TAN defaults for `φ₀` and `θ₀`;
  `LONPOLE` and `LATPOLE` still apply. The longitude axis's `PV` give the polynomial
  whose first variable is that axis, and the latitude axis's `PV` the one with the
  variables swapped. wcslib does this, and erases the celestial `PV` before it reads
  `φ₀` and `θ₀` (`wcs.c:2599-2665`). The inverse uses the SIP stop rule on the
  intermediate coordinates. Refused: TNX, ZPX, TPU and TPD (IRAF `WAT` text, a prior
  TPV, and wcslib's record-valued `DPj`), which no darkroom input uses.
- **`Wcs::write_into(header, alt)` writes the description into a header, in wcslib's
  form.** It first removes every WCS record of that alternate from `header`, then sets
  `WCSAXES`, `CRPIX`, `PCi_j`, `CDELT`, `CUNIT`, `CTYPE`, `CRVAL`, `PVi_m`, `LONPOLE`,
  `LATPOLE`, `RADESYS`, `EQUINOX`, the spectral rest and frame keywords, and the SIP or
  TPV terms in their own form. The `Wcs` keeps the `PC`, `CDELT`, `CUNIT` and `CRVAL` it
  read, so the write gives back the same numbers. A header with `CD` is kept as `PC = CD`
  with `CDELT = 1`, as wcslib's `wcsset` does, which is also the form TPV needs (`dis.h`
  "TPV": TPV assumes `CDi_j`, so `CDELT` must be 1). A legacy `CROTA` writes the `PC` it
  derives. `wcshdo` writes the same set (`wcshdr.c:575-760`), and omits a `PC` term
  equal to the identity. A `-TAB` axis is `UnsupportedWcsTransform` here, because its
  arrays live in another HDU. The name pairs with `Wcs::from_header`, and the `_into`
  suffix means "into the caller's value", as in `physical_into`. Refused: a new header
  that the caller merges, as astropy's `to_header` gives (`wcs.py:2965`); a merge over
  a header with `CD` leaves `CD` beside the new `PC`, which `Wcs::from_header` then
  refuses as `ConflictingWcsKeywords`. Refused: `CD` output of the product matrix,
  which is exact but leaves the form that wcslib and astropy write. Refused: a new `PC`
  and `CDELT` factored from the product, which rounds.
- **`Wcs::crop(region)` and `Wcs::bin(factors)` derive a new WCS exactly.** A crop takes
  the `Region` that the image read takes, so a caller crops the image and its WCS with
  one value, and subtracts each range's start from `CRPIX`; `Region::All` changes
  nothing. A bin by `b` on pixel axis `j` maps `CRPIX` to `(CRPIX − 0.5)/b + 0.5`, the
  centre of the first block, as astropy's `slice` with a step does
  (`wcs.py:3580-3590`). It multiplies column `j` of `PC` by `b`, because a bin scales a
  pixel axis, which is a column, not a world axis's `CDELT`. With bin factors `b₁` and
  `b₂` on the SIP axes, `A'_p_q = A_p_q·b₁^p·b₂^q / b₁` and `B'_p_q = B_p_q·b₁^p·b₂^q /
  b₂`, because the offsets from `CRPIX` scale by `b` and the focal-plane offset is
  divided by `b` again; `AP` and `BP` scale the same way. TPV acts after the matrix, so
  a bin leaves it as it is. Refused: astropy's way, which multiplies `CDELT` and keeps
  the SIP terms (`wcs.py:3577-3612`), correct only for an equal bin on both axes with no
  SIP. Refused: a stride on `Region`, which `docs/design.md` "No section stride"
  refuses.

### Files

- **`FitsEditor` edits a file that exists**, over `Read + Write + Seek`, beside
  `FitsReader` and `FitsWriter`. It scans the headers as `FitsReader` does, and names an
  HDU with an `HduSelector`. `replace_header(hdu, &Header)` writes a header in place when
  it fits in the HDU's header blocks, and fills the free space before `END` with blank
  records. A header that needs more blocks is `HeaderDoesNotFit { needed, available }`.
  When the new header holds `CHECKSUM` and `DATASUM`, the write computes `CHECKSUM` from
  that `DATASUM` and the new header, with no read of the data; the stored `DATASUM` is
  trusted, as it was before the edit. `replace_header_into(hdu, &Header, sink)` writes
  the whole edited file into a second sink, which the caller renames over the first, as
  astropy does in update mode (`hdulist.py:1471-1500`). `FitsWriter::with_spare_records(n)`
  writes at least `n` blank records before each header's `END`, so a later edit fits in
  place, as cfitsio's `ffhdef` reserves space (`fitscore.c:6909-6950`); the word is
  "spare", because "reserved" already names the standard's reserved keywords.
  `FitsEditor::append(self)` gives a `FitsWriter` at the end of the file, after it checks
  that the file ends on a block boundary; it consumes the editor, so two handles never
  write one file. Refused: a move of every byte after the header, as cfitsio (`ffiblk`,
  `fitscore.c:8107`) and fitsio-pure do, a large write that leaves a broken file when it
  stops in the middle. Refused: row and column insert and delete, which a copy does.
  Chosen by the owner, 2026-10-24.

### Time

- **A time is an integer MJD day and a fraction:** `time::Mjd`, opaque, with
  `Mjd::new(day, fraction)` normalizing the fraction into [0, 1), and `day()` and
  `fraction()` to read it. It carries `MJDREFI`/`MJDREFF` and the values derived from
  them. A fraction of one day has a spacing of 2⁻⁵³ day ≈ 1e-11 s. A relative time joins
  it with error-free operations: `two_sum`, `two_product`, and a division with its
  remainder, as astropy's `day_frac` does (`time/utils.py:19-70`, after Shewchuk 1997).
  Source: §9.2.2. Refused: one `f64` MJD, which at MJD 60000 has a spacing of 2⁻³⁷ day ≈
  0.63 µs. Refused: astropy's fraction in [−0.5, 0.5], which centres the day on the
  integer for a JD's noon; an MJD day starts at 0 h, so [0, 1) keeps the day number of
  the date. Refused: an `f64` day number, which an `i64` holds exactly. Refused: public
  fields, which let a fraction outside [0, 1) exist.

## Not in this plan

- `.fits.gz`, non-seekable and async input: outside the format. The caller inflates and
  calls `FitsReader::from_bytes`.
- HCOMPRESS `SMOOTH` on write: a hint for the reader, which already smooths.
- PLIO_1 for float images; time-scale conversion; a public unit grammar; TNX, ZPX, TPU,
  TPD; random groups write; row filters and the other cfitsio tool features.

## Plan

### Batch 1 — compression read bugs

1. **Lossless float read.** `FloatQuantization::read` gives a lossless case when
   `ZQUANTIZ = 'NONE'`, or when both the `ZSCALE` and the `ZZERO` column are absent.
   `TileDecoder::decode_float_tile_into` decodes a `COMPRESSED_DATA` cell of a lossless
   image as GZIP_1 or GZIP_2 floats, unshuffled at the element width, and refuses
   another codec. Files: `src/compress/decode/float_quantization.rs`,
   `src/compress/decode/tile_decoder.rs`. Tests: `fpack -g1 -q 0` and `fpack -g2 -q 0` of
   a small f32 and a small f64 image with NaN pixels read bit for bit equal to the
   source. A hand-built `ZIMAGE` table of one 4×2 f32 GZIP_1 tile, with no `ZSCALE`,
   `ZZERO` or `ZQUANTIZ`, reads `[1.5, -2.25, 3.0, 100.125, 0.0, 7.75, -8.5, 42.0]`
   (today it reads `[1069547500.0, …]`). A lossless RICE_1 float header is
   `UnsupportedCompression`. Removes the first two issues in `.notes/ISSUES.md`.
2. **PLIO_1 u16 read.** `TileDecoder` subtracts 32768 from PLIO_1 values when
   `ZBITPIX = 16` and `BZERO = 32768`. Test: `fpack -p` of a u16 image with the values
   0, 1, 32767, 32768 and 65535 reads back exactly.

### Batch 2 — compression write

3. **Public quantization and dither types.** `CompressionConfig::with_quantization`
   takes a `Quantization`; add `Quantization`, `Dither` and `DitherSeed`, remove the
   public `DitherMethod` (the decoder keeps a private method type), and rename
   `Hcompress::lossy` to `Hcompress::relative` beside a new `Hcompress::absolute`. The
   default for a float image is `Quantization::relative(4.0,
   Dither::Subtractive1(DitherSeed::FROM_SAMPLES))`. Files: `src/compress/mod.rs`,
   `src/lib.rs` (the `image` module's exports), every caller in `src/`, `examples/`,
   `benches/` and `lumos`. Tests: `relative`, `absolute` and both `Hcompress`
   constructors refuse 0, a negative value, NaN and infinity; `DitherSeed::fixed`
   refuses 0 and 10001 and takes 1 and 10000.
4. **Lossless float write.** `compress_float_image` with `Quantization::NONE` writes one
   `COMPRESSED_DATA` column of gzip'd big-endian floats, `ZQUANTIZ = 'NONE'`, and no
   `ZSCALE`, `ZZERO` or `ZDITHER0`. A codec other than GZIP_1 or GZIP_2 is
   `UnsupportedCompression`. Tests: a round trip of f32 and f64 with NaN, ±0.0 and
   subnormals, bit for bit; `funpack` of the output equals the source, and `fitsverify`
   reports no error (each checked once to make a fixture).
5. **Seed from the samples.** `ZDITHER0` comes from the `Dither`; the byte sum runs over
   the borrowed samples before `map_tiles`. Tests: the seed of a small image computed by
   hand in the test, with the byte sum shown; two images with the same first row and
   different other rows give different `ZDITHER0`; the same image twice gives identical
   bytes; a file written with `DitherSeed::fixed(17)` carries `ZDITHER0 = 17` and reads
   back with the decoder's dither at seed 17.
6. **noise2, noise3 and noise5, and absolute steps.** Port `FnNoise5_float` and
   `fits_img_stats_int` (2nd, 3rd and 5th order MAD for each row, with cfitsio's rules
   for short rows) into `src/compress/quantize.rs`. Use the smallest nonzero value in
   `quantize_tile` and `hcompress_tile_scale`. `Quantization::absolute` sets `ZSCALE` to
   its step and `Hcompress::absolute` its scale, with no noise estimate. Tests: noise2,
   noise3 and noise5 on an 8×8 tile computed by hand with the medians shown;
   `fpack -q17 4` and `fpack -q17 -0.01` (seed 17) of a noisy f32 image give the same
   `ZSCALE` and `ZZERO` for each tile and the same quantized integers as fits-well with
   `DitherSeed::fixed(17)` (read the fpack output's columns and compare); `fpack -h
   -s 2.5` and `fpack -h -s -30` of an i16 image give the same tile scales as fits-well.
7. **HCOMPRESS for floats, and `Q` descriptors.** `compress_float_image` takes
   HCOMPRESS_1 (2-D only, as for integers), refuses it with `Dither::Subtractive2`, and
   promotes the descriptors to `Q` with `DescriptorWidth::addressing`, as
   `compress_image` does. While the function changes, remove its `unreachable!()` arm and
   its `s.be.clone()`. Tests: `fpack -q17 4 -h -s 2.5` of an f32 image and fits-well's
   output with the same seed decode to the same floats; a container layout test with a
   forced `Q` width (an `internals` hook for the width threshold, not a 4 GiB image).
8. **PLIO_1 u16 write.** `compress_image` adds 32768 to a u16 image's values for PLIO_1.
   Tests: a round trip of 0, 1, 32767, 32768 and 65535; `funpack` of the output equals
   the source. Removes the third issue in `.notes/ISSUES.md`.

### Batch 3 — compressed table interop

9. **`ZCTYPn` that cfitsio reads.** `pick_algo` clamps GZIP_2 to GZIP_1 for `L`, `X`,
   `A`, `B`, `C` and `M` columns. `write_compressed_table` with `Compression::None` is
   `InvalidCompressionParameter`, and the `Algo::NoCompress` arm of `pick_algo` goes
   away if no other path reaches it. Tests: the existing table tests updated with each
   column type and its codec; one written table with every column type unpacks with
   `funpack`, and `fitsverify` reports no error (checked once to make a fixture).

### Batch 4 — header

10. **Repaired and malformed records, and spare records.** Add `Repair` and its kinds
    (lowercase name, misplaced value indicator, non-ASCII byte in text),
    `Header::repairs()`, `Card::Malformed { raw: [u8; 80], defect: Defect }`,
    `Record::Malformed`, `RecordView::Malformed` and `Defect`. `ParsedRecord::parse`
    repairs or keeps a record instead of an error. `Header::get*` of a malformed valued
    keyword returns `InvalidValue`. `HduRole`/`HduKind` classification and
    `data_extent` fail on a defect in a keyword that sizes the HDU, as now. The writer
    refuses a header with a malformed record with `InvalidValue`. `Header::parse` drops
    the blank records before `END` and counts them. Tests: a file whose HDU 1 holds
    `exptime = 30`, `OBJECT  ='M31'`, `EXPOS= 5`, a comment with the byte 0xE9, and
    `GAIN    = 1.2.3` opens and reads HDU 0 and HDU 2; `get("EXPTIME")` is 30,
    `get("OBJECT")` is `M31`, `get("EXPOS")` is 5, the comment holds `?`, and
    `repairs()` lists the four repairs with their record positions in order;
    `get("GAIN")` is `InvalidValue`, and `iter()` gives `GAIN` as `Malformed`; a write
    of HDU 1's header renders `EXPTIME =`, `OBJECT  = 'M31'` and `EXPOS   = 5`, and is
    `InvalidValue` until `GAIN` is removed; a defect in `NAXIS1` fails the
    open and a defect in `TFORM2` fails only `read_table`; a valid header has no
    repairs; a header with 3 blank records before `END` parses with no blank record
    and 3 spare records.
11. **HIERARCH.** `ParsedRecord::parse` reads a `HIERARCH` record with `=` into
    `Card::Hierarch`, and one without `=` stays commentary. Add `HierarchName`,
    `Header::get_hierarch`, `Header::set_hierarch`, `Record::Hierarch`,
    `RecordView::Hierarch`, and the render path. Tests: ESO records with string,
    integer, real and logical values; two blanks between tokens read as one; a token
    longer than 8 bytes and a token with `.` read; `get_hierarch("eso det chip1 name")`
    finds `ESO DET CHIP1 NAME`; a name with `=` is refused by `HierarchName`; a long
    string on `CONTINUE` read and written; the blank before `=` dropped only for a record
    that needs that byte (worked to the byte); a comment that does not fit is
    `HeaderCardTooLong`; a round trip of a header with HIERARCH and ordinary records in
    order. Update `src/header_model/card/tests.rs:214`.

### Batch 5 — WCS

12. **CTYPE distortion code.** `Ctype` gains `distortion: Option<Distortion>` (`Sip`,
    `Tpv`): the code after byte 9 of a 12-character value, or the `TPV` projection code.
    `Metadata` gains the distortion that applies. Tests: `RA---TAN-SIP`,
    `DEC--SIN-SIP`, `RA---TPV`, `GLON-CAR`, `FREQ`, and a 12-character value whose code
    is unknown, which leaves the axis unsupported.
13. **SIP.** A `src/world_coordinates/distortion/` module with `sip.rs`: the
    coefficients, the forward polynomial, and the Newton inverse, applied before the
    matrix. Tests: a hand-built order-2 header with the forward polynomial worked by
    hand; astropy's `irac_sip.hdr` (copied from `astropy/wcs/tests/data`, BSD-3, with
    its source named), where astropy's `test_sip` gives pixel (200, 200) at origin 0 →
    focal (72, 72) within a relative 1e-3, read through a `#[cfg(test)]` reach-in to the
    SIP stage; a round trip at the image corners to 1e-10 pixel; a round trip at a pixel
    coordinate of 1e5, which only the relative stop rule lets converge; a header with
    `AP`/`BP` gives the same inverse as one without; `WcsNoConvergence` on a polynomial
    with a fold inside the image; SIP keywords with no `-SIP` code apply and `Metadata`
    shows them; `-SIP` with no `A_ORDER` is `MissingKeyword`; `A_ORDER = 10` is
    `KeywordOutOfRange`; an alternate `A` description takes no SIP.
14. **TPV.** `tpv.rs` in the same module: the registered 40-term polynomial, applied
    between the matrix and the projection, and `CelestialTransform` reading no `PVi_m`
    of either celestial axis on a `-TPV` description. Tests: astropy's `siponly.hdr` and
    `tpvonly.hdr` give one distortion in the two forms. astropy's
    `test_sip_tpv_agreement` (`test_wcs.py:967-1001`) makes them agree to 6 decimals at
    `CRPIX` only, so fits-well's SIP and TPV must agree there to that precision. Away
    from `CRPIX` the two headers are a fitted conversion, so measure their difference at
    25 pixels once, write it into the test with that reason, and assert it. Also: a
    polynomial with only `PV1_1 = PV2_1 = 1` equals plain TAN, and one term is worked by
    hand. Both headers use the deprecated `RADECSYS`; check that the WCS parse reads
    them.
15. **`Wcs::write_into`.** Tests: `from_header` of the header that `write_into` fills
    gives the same metadata and the same `pixel_to_world` at 25 pixels, bit for bit, on
    every projection fixture, a spectral cube, an alternate `A`, SIP and TPV; a `-TAB`
    axis is `UnsupportedWcsTransform`; a `PC` + `CDELT` header gives back its own
    values, a `CD` header gives `PC = CD` and `CDELT = 1`, and a `CROTA` header gives the
    `PC` worked by hand; an identity `PC` term is not written; a write into a header
    that holds `CD1_1` and an `A` description leaves no `CD` record and keeps the `A`
    records as they were.
16. **`Wcs::crop` and `Wcs::bin`.** Tests: for TAN and TAN-SIP, a crop's
    `pixel_to_world(p − start)` equals the parent's `pixel_to_world(p)` to 1e-12°; a
    2×2 bin's pixel `p'` equals the parent at `2(p' − 0.5) + 0.5`, and a 2×3 bin of a
    rotated `PC` with SIP the same on each axis; the bin of a SIP term worked by hand;
    `Region::All` gives an equal WCS.

### Batch 6 — file edits

17. **`FitsEditor`.** `src/fits_editor/` with `open`, `hdus`, `replace_header`,
    `replace_header_into` and `append`; `Hdu` gains `pub(crate)` fields for the header's
    byte offset and block count; `FitsWriter::with_spare_records`. Tests: a header
    replaced by one of fewer and of the same number of blocks (the file length and the
    data bytes do not change, and the rest of the file is byte-identical);
    `HeaderDoesNotFit` with the counts; a file written with `with_spare_records(40)`
    takes 40 more records in place (the block count worked by hand); a header read back
    with its 40 more records fits again in place; `replace_header_into` of a header that
    grows gives a file that `FitsReader` reads with every other HDU byte-identical;
    `CHECKSUM` valid after a replace when `DATASUM` is present; `append` after a file
    that ends off the block grid is an error; an image appended to a file, and
    `FitsReader` reads both.

### Batch 7 — time

18. **`time::Mjd`.** `Frame::reference_mjd`, `Frame::relative_to_mjd`,
    `Frame::time_axis_mjd`, `Datetime::to_mjd`, `Coordinate::mjd` and the `Bounds` fields
    `start_mjd`, `end_mjd` and `mid_mjd` move to `Mjd`. `Datetime::to_jd` goes away: a
    JD in one `f64` loses more than an MJD, and `Mjd` gives the day. Tests: `MJDREFI =
    60000`, `MJDREFF = 0.5` and a relative time of 1e-6 s keep the microsecond (lost in
    one `f64` today, shown in the test); `JDREFI`/`JDREFF` splits; `Mjd::new(5, 1.0)` is
    day 6 and fraction 0, and `Mjd::new(5, -0.25)` is day 4 and fraction 0.75.

### Batch 8 — documents

19. **Research note for the WCS decision.** Run the linear-inverse bench that
    `docs/design.md` "The inverse linear step solves with the matrix's LU factors" cites,
    write the numbers into `docs/research/<date of the run>-wcs-into-slice.md`, and
    change the two citations in `docs/design.md` to that name. If no bench measures it,
    add one in `src/world_coordinates/bench.rs`. Removes the fourth issue in
    `.notes/ISSUES.md`.
20. **Fold the decisions.** Each batch moves its decisions above into `docs/design.md`,
    updates `README.md` (SIP and TPV, lossless float, `FitsEditor`, the quantization
    API), and deletes its items here. The last batch deletes this file.
