# Proposal: close the gaps the library comparison found

Source: `.notes/fits-well-comparison.md` (2026-10-23) and the open issues in
`.notes/ISSUES.md`. References: the FITS 4.0 standard (`docs/refs/fits_standard40.md`),
cfitsio 4.7.0 (`imcompress.c`, `quantize.c`, `docs/fpackguide.md`), astropy.wcs,
Shupe et al. 2005 (SIP), the TPV registration at fits.gsfc.nasa.gov/registry/tpvwcs.

This proposal is temporary. When an item lands, its decisions move into
`docs/design.md` and the item is deleted here. Every batch ends with
`cargo drive check` and `cargo drive test`. Where an item changes a public type, the
darkroom callers change with it (today only `lumos/src/io/fits/decode/tests.rs` uses
the compression API).

Local reference tools: `fpack`, `funpack` and `fitsverify` (cfitsio 4.7) are in
`/usr/bin`. astropy is not installed. A fixture made with a tool records the command
line beside its test, as the current fixtures do.

## Decisions

### Compression

- **A float image without `ZSCALE` and `ZZERO` columns, or with `ZQUANTIZ = 'NONE'`,
  is lossless.** Its `COMPRESSED_DATA` cells hold GZIP_1 or GZIP_2 big-endian floats
  at `ZBITPIX` width, as the `GZIP_COMPRESSED_DATA` fallback does. Any other codec on
  such an image is `UnsupportedCompression`. Source: §10.2 ("the absence of these two
  columns … losslessly compressed"); cfitsio `imcompress.c:5307-5318`, `:985-991`,
  `:6599-6611`. Refused: the present path, which reads the float bits as quantized
  integers.
- **The float quantization is one public type with three cases:** lossless,
  a step relative to each tile's noise, and an absolute step. Refused: cfitsio's one
  number, where 0 is lossless and a negative value is an absolute step. Refused:
  fits-well's present level 0, which means "the default of 4". Both put a mode into
  a magic value. Source: `fpackguide.md` §C, `quantize.c:128-157`.
- **The default dither seed comes from the first tile's bytes**, as fpack `-qt` does:
  `ZDITHER0 = (Σ bytes of the first tile's samples mod 10000) + 1`. The same input
  gives the same file, and two images get different seeds. A caller can also give a
  fixed seed from 1 to 10000. Refused: cfitsio's default, a seed from the clock, which
  makes the output different on each run. Refused: the present fixed seed of 1, which
  gives each image the same dither pattern. fpack warns that this loses the benefit
  of dither when images are added. Source: `imcompress.c:2788-2829`,
  `fpackguide.md` lines 213-230.
- **The noise estimate is the smallest nonzero value of noise2, noise3 and noise5**,
  for the float quantization and for the HCOMPRESS scale. Source: cfitsio
  `quantize.c:141-144` (`FnNoise5_float`), `imcompress.c:2090-2098`. Refused:
  noise3 alone, which gives a larger step than fpack and so loses more data.
- **A u16 image in PLIO_1 stores its physical values.** The writer adds 32768 to the
  stored values when `ZBITPIX = 16` and `BZERO = 32768`, and the reader subtracts it.
  Source: cfitsio `imcompress.c:2343-2362`, `:6494-6497`. The cost: cfitsio also
  subtracts 32768 from a PLIO image with `ZBITPIX = 32` and `BZERO = 32768`, which
  its own writer does not offset. fits-well offsets only the 16-bit case, so it reads
  that rare file differently from cfitsio, and correctly.
- **A quantized float image takes HCOMPRESS_1**, with the HCOMPRESS scale applied to
  the quantized integers, as cfitsio does. `SUBTRACTIVE_DITHER_2` with HCOMPRESS is
  refused, as fpack refuses `-qz` with `-h`. PLIO_1 for floats stays refused: a
  quantized tile seldom fits PLIO's 24-bit range. Source: `fpackguide.md` `-qz`.
- **The float container promotes to `Q` descriptors** past the 32-bit range, as the
  integer container does. Refused: the present `DataUnitOverflow`.
- **A compressed table writes GZIP_2 only on `I`, `J`, `K`, `E` and `D` columns.**
  Other column types take GZIP_1 when GZIP_2 is requested. cfitsio's reader
  unshuffles only those five (`imcompress.c:9028-9175`). A shuffle of a 1-byte
  column does nothing. Refused: GZIP_2 on `C` and `M`, which cfitsio's writer emits
  but its own reader refuses.
- **`write_compressed_table` refuses `Compression::None`** with
  `InvalidCompressionParameter`. A table with no compression is `write_table`.
  Refused: `ZCTYPn = 'NOCOMPRESS'`, which the standard permits (Table 36) and cfitsio
  refuses to read (`imcompress.c:8870-8887`). Refused: GZIP_1 at deflate level 0,
  which writes a keyword the caller did not ask for. Chosen by the owner,
  2026-10-23.

### Header

- **A malformed record does not stop the open.** `Header::parse` keeps a record that
  does not parse as `RecordView::Malformed { text, defect }`, with its 80 bytes. The
  defect is a typed enum: non-ASCII byte, lowercase keyword, no space after `=`,
  bad value. A lookup of a valid keyword whose value did not parse returns that
  error, so a read fails only when it needs that card. A malformed record in a
  structural keyword (`SIMPLE`, `XTENSION`, `BITPIX`, `NAXISn`, `PCOUNT`, `GCOUNT`,
  `TFIELDS`, `TFORMn`, `TBCOLn`, `THEAP`) still fails the open, because the HDU's
  size is unknown without it. The writer refuses a header that holds a malformed
  record. Source: cfitsio `getkey.c:3203-3206`. Refused: a lenient mode, which is
  modal state. Refused: a silent repair, such as reading a lowercase keyword as
  uppercase. Refused: the present behaviour, where one bad optional card in any HDU
  fails the whole file.
- **HIERARCH records are typed.** `HIERARCH ESO DET CHIP1 NAME = 'x' / c` reads as
  `RecordView::Hierarch { name, value, comment }`, where `name` is the tokens joined
  by single spaces. `Header::get_hierarch(name)` looks it up with a second index
  (`HashMap<Box<str>, usize>`, built on parse; a HIERARCH header is a cold path).
  `Header::set_hierarch` writes one, and refuses a record longer than 80 bytes or a
  value that needs `CONTINUE`. Source: ESO convention as registered at
  fits.gsfc.nasa.gov/registry/hierarch_keyword.html; astropy `card.py:158-177`.
  Refused: a `Keyword` longer than 8 bytes, which would end `Keyword: Copy`.

### WCS

- **The `CTYPEia` grammar takes a distortion suffix.** `RA---TAN-SIP` is the head
  `RA`, the projection `TAN` and the distortion `SIP`. `RA---TPV` is the projection
  `TAN` with the distortion `TPV`. Source: Shupe et al. 2005 §2; TPV registration.
- **SIP sits in the linear layer, before the matrix:** `(u, v) = p − CRPIX`, then
  `u + f(u, v)`, `v + g(u, v)` with `A_p_q`/`B_p_q` for `p + q ≤ A_ORDER`/`B_ORDER`.
  No order cap past what an 8-byte keyword names (`AP_p_q` with `p, q < 100`). astropy's
  `sip.c` has none either.
  The inverse solves the forward polynomial with Newton steps, from `AP`/`BP` as the
  first guess when present, else from `(U, V)`. It stops when the step is below 1e-12
  pixel: a pixel coordinate of order 1e4 has a spacing of 1e4 · 2⁻⁵² ≈ 2e-12, so a
  smaller step is rounding. After 20 steps it returns `WcsNoConvergence`. Refused:
  `AP`/`BP` alone as the inverse, which is an approximate fit, so a round trip would
  not return the input pixel. astropy's `all_world2pix` also iterates.
- **TPV sits after the matrix, on the intermediate coordinates in degrees:** `ξ` and
  `η` are the registered 40-term polynomials in `PV1_k` and `PV2_k`. On a `-TPV`
  axis, `PVi_m` on the longitude axis are polynomial terms, not `φ₀`/`θ₀`/`LONPOLE`,
  so the celestial transform takes the TAN defaults. The inverse uses the same Newton
  stop rule, in degrees: 1e-12 pixel times the pixel scale. Refused: TNX, ZPX and TPD
  (IRAF `WAT` text and wcslib's record-valued `DPj`), which no darkroom input uses.
- **A `Wcs` writes its header.** `Wcs::to_header(alt)` gives the canonical form:
  `WCSAXES`, `CTYPE`, `CUNIT` in the unit the transform works in, `CRPIX`, `CRVAL`,
  the `CDi_j` matrix as the transform holds it, `PVi_m`, `LONPOLE`, `LATPOLE`,
  `RADESYS`, `EQUINOX`, the spectral rest and frame keywords, and SIP/TPV terms.
  A `-TAB` axis is `UnsupportedWcsTransform` here, because its arrays live in
  another HDU. Refused: `PC` with `CDELT`, which some viewers show as the pixel
  scale. To factor the matrix into the two rounds its values, so a write and a read
  would not give back the same transform.
- **`Wcs::crop(origin)` and `Wcs::bin(factors)` derive a new WCS exactly.** A crop
  subtracts the 0-based origin from `CRPIX`. A bin by `b` on an axis maps `CRPIX`
  to `(CRPIX − 0.5)/b + 0.5` and multiplies that matrix column by `b`. With bin
  factors `b₁`, `b₂` on the SIP axes, `A'_p_q = A_p_q·b₁^p·b₂^q / b₁` and
  `B'_p_q = B_p_q·b₁^p·b₂^q / b₂`, because the offsets from `CRPIX` scale by `b`
  and the focal-plane offset is divided by `b` again. Refused: astropy's `slice` with a stride on
  `Region`, which `docs/design.md` "No section stride" refuses.

### Files

- **`FitsEditor` edits a file that exists**, over `Read + Write + Seek`. It scans
  the headers as `FitsReader` does. `replace_header(hdu, &Header)` writes a header
  in place when it fits in the HDU's header blocks, and fills the free records
  before `END` with blanks. When `DATASUM` is present, it updates `CHECKSUM` from
  that `DATASUM` and the new header, with no read of the data. A header that needs
  more blocks is `HeaderDoesNotFit { needed, available }`. `append()` gives a
  `FitsWriter` at the end of the file, after it checks that the file ends on a block
  boundary. Refused: a move of every byte after the header, as cfitsio and
  fitsio-pure do, which is a large write for one keyword. A rewrite through
  `read_data_unit` and `write_raw_hdu` is the explicit form of that cost. Refused:
  row and column insert and delete, which a rewrite does.

### Time

- **A time is an integer MJD day and a fraction.** `time::Mjd { day: i64, fraction:
  f64 }`, with the fraction in [0, 1), carries `MJDREFI`/`MJDREFF` and the values
  derived from it. A fraction of one day has a spacing of 2⁻⁵³ day ≈ 1e-11 s. Source:
  §9.2.2. Refused: one `f64` MJD, which at MJD 60000 has a spacing of 2⁻³⁷ day ≈
  0.63 µs. Refused: astropy's two free doubles, whose split is not unique, so two
  equal times compare unequal.

## Not in this plan

- `.fits.gz`, non-seekable and async input: outside the format. The caller inflates
  and calls `FitsReader::from_bytes`.
- HCOMPRESS `SMOOTH` on write: a hint for the reader, which already smooths.
- PLIO_1 for float images; time-scale conversion; a public unit grammar; TNX, ZPX,
  TPD; random groups write; row filters and the other cfitsio tool features.

## Plan

### Batch 1 — compression read bugs

1. **Lossless float read.** `FloatQuantization::read` gives a `Lossless` case when
   `ZQUANTIZ = 'NONE'`, or when both the `ZSCALE` and `ZZERO` columns are absent.
   `TileDecoder::decode_float_tile_into` decodes a `Compressed` cell of a lossless
   image as GZIP_1/GZIP_2 floats (unshuffled at the element width) and refuses
   other codecs. Files: `src/compress/decode/float_quantization.rs`,
   `src/compress/decode/tile_decoder.rs`.
   Tests: `fpack -g1 -q 0` and `fpack -g2 -q 0` of a small f32 and a small f64 image,
   with NaN pixels, read bit for bit equal to the source. A hand-built table with no
   `ZSCALE`/`ZZERO` columns and no `ZQUANTIZ` (the probe's
   `scratchpad/probe/gen.py` layout) reads `[1.5, -2.25, …]`. A lossless RICE_1
   float header is `UnsupportedCompression`. Removes the first two issues in
   `.notes/ISSUES.md`.
2. **PLIO_1 u16 read.** `TileDecoder` subtracts 32768 from PLIO_1 tiles when
   `ZBITPIX = 16` and `BZERO = 32768`. Test: `fpack -p` of a u16 image with values
   0, 1, 32767, 32768 and 65535 reads back exactly.

### Batch 2 — compression write

3. **Public quantization type.** Replace `with_quantization(level, dither)` with
   `with_quantization(Quantization)`: `Quantization::Lossless`,
   `Quantization::noise(level, Dither)` (level finite and > 0), and
   `Quantization::step(step, Dither)` (step finite and > 0). `Dither` holds the
   method and the `DitherSeed` (`FromFirstTile` default, `Fixed(seed)` checked
   1..=10000). The default config is `noise(4.0)` with `SUBTRACTIVE_DITHER_1` and
   `FromFirstTile`. Files: `src/compress/mod.rs`, `src/lib.rs` (the `image` module's
   exports). Tests: each constructor refuses 0, a negative value, NaN and infinity;
   seed 0 and 10001 refused, 1 and 10000 accepted.
4. **Lossless float write.** `compress_float_image` with `Quantization::Lossless`
   writes one `COMPRESSED_DATA` column of gzip'd big-endian floats, `ZQUANTIZ =
   'NONE'`, and no `ZSCALE`/`ZZERO`/`ZDITHER0`. A codec other than GZIP_1/GZIP_2 is
   `UnsupportedCompression`. Tests: round trip of f32 and f64 with NaN, ±0.0 and
   subnormals, bit for bit; `funpack` of the output equals the source (checked once
   to make a fixture, and `fitsverify` reports no error).
5. **Seed from the first tile.** `ZDITHER0` comes from `Dither` (item 3). Test: the
   seed of a known first tile is computed by hand in the test (the byte sum shown),
   two different images give different `ZDITHER0`, and the same image twice gives
   identical bytes. A file written with `Fixed(17)` reads back with the decoder's
   dither at seed 17.
6. **noise2/noise3/noise5.** Port `FnNoise5_float` (2nd, 3rd and 5th order MAD,
   per row, with cfitsio's minimum-row-length rules) into `src/compress/quantize.rs`,
   and use the smallest nonzero value in `quantize_tile` and `hcompress_tile_scale`.
   Absolute step: `Quantization::step` sets `ZSCALE = step` with no noise estimate.
   Tests: hand-computed noise2/3/5 on an 8×8 tile with the medians shown; `fpack -q 4`
   and `fpack -q -0.01` of a fixed-seed (`-q17`-style seed via `Fixed`) noisy f32
   image give the same `ZSCALE`/`ZZERO` per tile and the same quantized integers as
   fits-well (read the fpack output's columns and compare).
7. **HCOMPRESS for floats, and Q descriptors.** `compress_float_image` accepts
   HCOMPRESS_1 (2-D only, as for integers), refuses it with `SUBTRACTIVE_DITHER_2`,
   and promotes both descriptors to `Q` with `DescriptorWidth::addressing`, as
   `compress_image` does. Remove the `unreachable!()` arm and the `s.be.clone()` in
   the same function while it changes. Tests: `fpack -h -s 2.5` of a fixed-seed f32
   image and fits-well's output decode to the same floats; a container layout test
   with a forced `Q` width (an `internals` hook for the width threshold, not a 4 GiB
   image).
8. **PLIO_1 u16 write.** `compress_image` adds 32768 to a u16 image's values for
   PLIO_1. Test: round trip of values 0, 1, 32767, 32768, 65535; `funpack` of the
   output equals the source. Removes the third issue in `.notes/ISSUES.md`.

### Batch 3 — compressed table interop

9. **ZCTYPn that cfitsio reads.** `pick_algo` maps GZIP_2 to GZIP_1 for `L`, `X`,
   `A`, `B`, `C` and `M`. Tests: the existing table tests updated with the column
   types and codecs shown; one written table with every column type unpacks with
   `funpack -F` and `fitsverify` reports no error (checked once to make a fixture).
   `write_compressed_table` with `Compression::None` is
   `InvalidCompressionParameter`, and the `Algo::NoCompress` arm of `pick_algo`
   goes away if no other path reaches it.

### Batch 4 — header

10. **Malformed records.** Add `Card::Malformed { raw: [u8; 80], defect: Defect }`,
    `RecordView::Malformed`, and `Defect` (in `src/error.rs` if a `FitsError`
    variant carries it). `ParsedRecord::parse` returns it instead of an error for a
    non-structural keyword. `Header::get*` of a malformed valued keyword returns its
    error. `HduRole`/`HduKind` classification fails on a malformed structural
    keyword, as now. The writer refuses a header with a malformed record
    (`MalformedRecord`). Tests: a file with a lowercase card, a `KEY    ='x'` card
    and a non-ASCII comment in HDU 1 opens, reads HDU 0 and HDU 2, iterates the
    three records as `Malformed` with the right `Defect`, and `get` on the bad valued
    keyword errors; a malformed `NAXIS1` still fails the open.
11. **HIERARCH.** `ParsedRecord::parse` reads a `HIERARCH` record with `=` into
    `Card::Hierarch`; one without `=` stays commentary. `Header::get_hierarch`,
    `set_hierarch`, and the render path. Tests: ESO cards with string, integer, real
    and logical values; whitespace in the name normalized to single spaces; lookup is
    case-sensitive as ESO names are uppercase; a record over 80 bytes refused on
    write; round trip of a header with HIERARCH and ordinary records in order. Update
    `src/header_model/card/tests.rs:214`.

### Batch 5 — WCS

12. **CTYPE distortion suffix.** `Ctype` gains `distortion: Option<Distortion>`
    (`Sip`, `Tpv`), parsed from `-SIP` after the projection code and from the `TPV`
    code. Tests: `RA---TAN-SIP`, `DEC--TAN-SIP`, `RA---TPV`, `GLON-CAR`, `FREQ`.
13. **SIP.** A `sip.rs` in `src/world_coordinates/linear_transform/` with the
    coefficients and the forward and Newton inverse, applied by `LinearTransform`.
    Tests: a hand-built order-2 header with the forward polynomial worked by hand;
    astropy's `irac_sip.hdr` (copied from `astropy/wcs/tests/data`, BSD-3, with its
    source named), where astropy's `test_sip` gives pixel (200, 200) → focal (72, 72)
    within 1e-3; a round trip at the image corners to 1e-10 pixel; a header with
    `AP`/`BP` gives the same inverse as one without; `WcsNoConvergence` on a
    polynomial with a fold inside the image. wcslib and astropy are not installed,
    and an install needs the user's approval, so the values come from hand work and
    from the astropy test files.
14. **TPV.** The registered 40-term polynomial in a `tpv.rs`, applied between the
    matrix and the projection, and `CelestialTransform` skipping `PV1_1`..`PV1_4`
    on a `-TPV` axis. Tests: astropy's `siponly.hdr` and `tpvonly.hdr` give one
    distortion in the two forms. astropy's `test_sip_tpv_agreement`
    (`test_wcs.py:967-1001`) makes them agree to 6 decimals at `CRPIX` only, so
    fits-well's SIP and TPV must agree there to that precision. Away from `CRPIX` the
    two headers are a fitted conversion, so measure their difference at 25 pixels
    once, write it into the test with that reason, and assert it. Also: a polynomial
    with only `PV1_1 = PV2_1 = 1` equals plain TAN, and one term is worked by hand.
    Both headers use the deprecated `RADECSYS`, so check that item 10 or the WCS
    parse does not refuse them.
15. **`Wcs::to_header`.** Tests: `from_header(to_header(w))` gives the same
    metadata and the same `pixel_to_world` at 25 pixels, bit for bit, on every
    projection fixture, a spectral cube, an alternate `A`, SIP and TPV; a `-TAB`
    axis is `UnsupportedWcsTransform`; the `CDi_j` written for a `PC` + `CDELT`
    header and for a legacy `CROTA` header, worked by hand. The metadata compare
    is on what the transform holds: the declared `CDELT` and `CROTA` keywords do not
    come back.
16. **`Wcs::crop` and `Wcs::bin`.** Tests: for TAN and TAN-SIP, a crop's
    `pixel_to_world(p − origin)` equals the parent's `pixel_to_world(p)` to 1e-12°;
    a 2×2 bin's pixel `p'` equals the parent at `2(p' − 0.5) + 0.5`; the bin of a
    SIP term worked by hand.

### Batch 6 — file edits

17. **`FitsEditor`.** `src/fits_editor/` with `open`, `hdus`, `replace_header`,
    `append`; `Hdu` gains the header's byte offset and block count. Tests: replace a
    header with one of fewer and of the same number of blocks (the file length and
    the data bytes do not change, and the rest of the file is byte-identical);
    `HeaderDoesNotFit` with the counts; `CHECKSUM` valid after a replace when
    `DATASUM` is present; `append` after a file that ends off the block grid is an
    error; append of an image to a file and `FitsReader` reads both.

### Batch 7 — time

18. **`time::Mjd`.** `Frame::reference_mjd`, `relative_to_mjd`, `time_axis_mjd`,
    `Datetime::to_mjd` and `Coordinate` move to `Mjd`. Tests: `MJDREFI = 60000`,
    `MJDREFF = 0.5` and a relative time of 1e-6 s keep the microsecond (it is lost
    in one `f64` today, shown in the test); `JDREFI`/`JDREFF` splits; normalization
    of a fraction of 1.0 to the next day.

### Batch 8 — documents

19. **Research note for the WCS decision.** Run the linear-inverse bench that
    `docs/design.md` "The inverse linear step solves with the matrix's LU factors"
    cites, and write `docs/research/2026-10-23-wcs-into-slice.md` with the numbers.
    If no bench measures it, add one under `src/world_coordinates/bench.rs`. Removes
    the fourth issue in `.notes/ISSUES.md`.
20. **Fold the decisions.** Each batch moves its decisions above into
    `docs/design.md`, updates `README.md` (SIP/TPV, lossless float, `FitsEditor`),
    and deletes its items here. The last batch deletes this file.
