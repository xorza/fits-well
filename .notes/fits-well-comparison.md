# fits-well 0.3 compared with cfitsio, rust-fitsio, fitsio-pure, fitsrs and astropy

Date: 2026-10-24. fits-well at 69f5a25. The evidence is from source reads.

The other libraries are at the commits of the 2026-10-23 run, so their columns did not
change:

- cfitsio 4.7.0, 113781d;
- rust-fitsio 0.21.10, ee9c3b3;
- fitsio-pure 0.22.0, 2543a9d;
- fitsrs 0.4.2, c7e9c28;
- astropy, 6edc126.

The clones are in the session scratchpad, `fits-compare/`. In the fits-well column, "test"
names the fits-well test that covers a row in place of the 2026-10-23 probe.

## Matrix

| | fits-well | cfitsio 4.7 | rust-fitsio 0.21 | fitsio-pure 0.22 | fitsrs 0.4 | astropy |
|---|---|---|---|---|---|---|
| Pure Rust / no C | yes | C | no (C) | yes | yes | Python + C |
| no_std / wasm in CI | no / untested | no | no | yes / yes | no / stale | no |
| Image read/write, all BITPIX | yes / yes | yes | yes | yes | read | yes |
| BSCALE/BZERO, unsigned, BLANK | full | full | no null mapping | partial, BLANK wraps | none | full |
| Image sections | tile-aware | yes | yes | wrong on compressed | no | yes + stride |
| Binary tables r/w | yes | yes | yes, panics on VLA/complex | yes | read, raw | yes |
| VLA (P/Q) | r/w, all types | r/w | no | read, some types | no | r/w |
| TNULL, TDIM, TSCAL | yes | yes | no | no TNULL | no | yes |
| ASCII tables | r/w whole | r/w + edit | r/w | r/w, no implied decimal | raw bytes | r/w |
| Random groups | read | r/w | no | header only | breaks size | r/w |
| Compressed image codecs (write) | all 5 + NOCOMPRESS, parallel, HCOMPRESS of quantized floats | all + BZIP2 | via filename | RICE, GZIP 1/2 | none | all |
| Lossless float compression | r/w, bit-exact with NaN payloads | yes | via cfitsio | ? | panics | yes |
| Quantization noise and dither seed | cfitsio's noise2/3/5, seed from every sample | noise2/3/5, clock seed | via cfitsio | ? | — | noise2/3/5, clock seed |
| Compressed tables §10.3 | r/w, partial reads | whole-HDU convert only | no | no | no | no |
| NULL_PIXEL_MASK | read | no | no | no | no | ? |
| CHECKSUM/DATASUM | write + verify, summed on read, kept valid on a header edit | + update in place | no | verify says true if absent | no | yes |
| HIERARCH | typed, every keyed header method | r/w | — | compat only | read | r/w |
| In-place edit / append | header replace in place or into a copy, append | full | partial | header + compat | no | full |
| Lenient header parse | 4 repairs, each reported; a bad record kept as malformed | yes | yes | yes | partial | fix modes |
| Non-seekable / async input | no, by design | stdin | — | stream `Read` | async | fsspec |
| `.fits.gz` file input | no, by design | yes + URLs | yes | compat | yes | yes |
| WCS | 27 proj with HPX, spectral, -TAB, table WCS | 9 AIPS proj | no | no | 23 proj, 2-D, SIP | wcslib full |
| SIP / TPV distortion | both, both directions | no | no | no | SIP | yes |
| WCS header write, sub/slice | write, crop, bin; no axis subset | — | no | no | no | yes |
| Time §9 | metadata, MJD as day + fraction, no scale conversion | strings only | no | no | no | full + scale conversion |
| Hostile-input safety | checked | CVE history | inherits cfitsio | panics on 3 probes | swallows errors | — |

## fits-well bugs

The issue log `fits-well/.notes/ISSUES.md` has no open issue. These are the four bugs from
the 2026-10-23 run, now fixed:

- A lossless compressed float image read its float bits as quantized integers. Test
  `a_float_image_without_scale_columns_is_lossless`.
- `ZQUANTIZ = 'NONE'` was an error. Tests `decompresses_the_lossless_float_fixtures`
  (fixtures with `'NONE'`) and `lossless_float_compression_round_trips_bit_for_bit`.
- A PLIO_1 write of a u16 image failed, and its stream was not offset by 32768 as in
  cfitsio. Tests `plio_writes_a_u16_image` and `plio_of_a_u16_image_holds_its_physical_values`.
- docs/design.md cited a research file that did not exist. The cited file
  `docs/research/2026-10-24-reference-library-practice.md` is now there.

## Closed since 2026-10-23

These items of the 2026-10-23 list "fits-well lacks" are now done. The design document
`docs/design.md` gives each decision:

- **SIP and TPV distortion**, both directions. See "SIP is a prior distortion" and "TPV is
  a sequent distortion".
- **Lenient header parse.** A lowercase name, a misplaced `=`, a non-ASCII byte in text
  and text after `END` are repaired, and each repair is reported. A record with a bad
  value is kept as malformed and fails only the read that needs it. The open fails only
  for a bad keyword that sizes an HDU. See "A near-valid record is repaired and reported".
- **Lossless float compression write**, and `Quantization::NONE` in place of the level-0
  magic value.
- **A dither seed for each image**, from the byte sum of every sample. See "The default
  seed is…".
- **In-place header update, append and checksum update**: `FitsEditor`.
- **Typed HIERARCH** through every keyed `Header` method.
- **WCS header write, crop and bin**: `Wcs::write_into`, `Wcs::crop` and `Wcs::bin`.
- **The noise estimate** is cfitsio's noise2/3/5, so a write matches fpack's `ZSCALE`.
- **HCOMPRESS of quantized floats**, and an absolute quantization step.
- **Compressed float images over 2 GiB of heap**: the descriptors widen to `Q`.
- **The time precision**: an MJD is an integer day and a fraction, about 1e-11 s.
- **`ZSCALE`/`ZZERO` as keywords** are read, then as columns.
- **Interop**: no `ZCTYPn = 'NOCOMPRESS'` write, and GZIP_2 only on `I`, `J`, `K`, `E`
  and `D` columns, as cfitsio's reader takes them.

## fits-well lacks, ranked

Important:
1. **wcsfix-style repairs**: `CUNIT = 'DEG'` or `'degree'`, the AIPS `-NCP` and `-GLS`
   projections, and `FREQ-LSR`. Today these are `InvalidUnit` or an unsupported
   projection. The units resolve with case (`src/unit.rs`). `PC` with `CROTA` is refused
   on purpose.
2. **A cfitsio differential test job and a fuzz harness.** The golden fixtures come
   from fpack, funpack and astropy, but no CI job compares with a running cfitsio.
   fitsio-pure has such a job. None of the libraries has a fuzz harness.

Medium:
- A NULL_PIXEL_MASK write. Lossy HCOMPRESS of an image with undefined pixels is refused
  today (`LossyHcompressNulls`).
- Typed unsigned table columns on write. Read gives `ColumnReader::unsigned`, but a write
  takes the signed storage and `with_scaling`, and `ColumnData` has no U16, U32, U64 or
  I8.
- A streaming table write. Only `stream_image` exists.
- A WCS axis subset, as wcslib's `wcssub`.
- An in-place data edit, such as a rewrite of pixels or rows. cfitsio has one.

Declined by design (`docs/design.md` gives the reason):
- gzip'd files, non-seekable streams and async input ("No gzip'd file, non-seekable
  stream or async input");
- time-scale conversion ("No time-scale conversion");
- HCOMPRESS `SMOOTH` on write;
- the write of random groups, a public unit grammar, and the cfitsio tool features;
- a section stride, and partial reads of an ASCII table;
- the insert and delete of rows and columns ("`FitsEditor` edits a file that exists");
- `PLIO_1` for float images, and `ZCTYPn = 'NOCOMPRESS'` on write.

Niche or legacy:
- the extended filename syntax, the row filter and calculator, histograms, the iterator
  framework, templates, the grouping convention and region files;
- BZIP2_1, RVKC, and a byte-exact header round trip;
- the fitsdiff and fitsverify tools, IRAF .imh, the `RICE_ONE` alias, and the zlib tile
  fallback.

## fits-well does better

- **Compressed tables**: a transparent read and write, with partial reads by tile.
  cfitsio only converts a whole HDU, and astropy has none.
- **WCS breadth without wcslib**:
  - 27 projections with HPX, PC/CD/PVi_m and alternates;
  - all spectral algorithms, `-TAB` evaluated, and binary-table WCS;
  - SIP and TPV in both directions;
  - a write back in wcslib's form, and an exact crop and bin.

  This equals astropy, and it is far above every Rust crate and cfitsio's own helpers.
- **Codec coverage and correctness**:
  - bit-exact fixtures against fpack, funpack and astropy;
  - fpack's own `ZSCALE`, `ZZERO` and HCOMPRESS scale, bit for bit;
  - NULL_PIXEL_MASK read;
  - tile-parallel encode and decode;
  - section reads that decode only the tiles they touch.
- **Reproducible compressed files.** The dither seed comes from the samples, not the
  clock as in cfitsio and astropy. Thus the same image gives the same file, and frames
  with equal first tiles still get different seeds.
- **A u16 PLIO_1 stream that cfitsio reads.** astropy refuses this write and misreads
  cfitsio's files.
- **Hostile-input safety**: a NAXIS cap, an exact inflate size and checked offsets.
  fitsio-pure panics on three crafted files, fitsrs swallows I/O errors, and cfitsio had
  CVE fixes in 4.6.0, 4.6.4 and 4.7.0.
- **The header model**:
  - a typed `Keyword` and an O(1) index;
  - `HIERARCH` with every typed getter;
  - exact big integers, the shortest exact real format, NaN refused, `END` checked,
    and the `CONTINUE` `&` rules right;
  - a commentary record that would read back as another record is refused.
- **A repair report.** Each repair of a near-valid record is listed with its position.
  cfitsio repairs with no report, and astropy reports through its `verify` options.
- **Header edits that cannot break a file.** A header is replaced in place only when it
  fits, or into a copy, and `CHECKSUM` stays valid. cfitsio moves every byte after a
  header that grows.
- **Checksums summed during the read**, with `Absent` kept apart from valid.
- **Zero-copy and caller-owned buffers**: `from_bytes`, `read_table_columns_into`, and
  an allocation-free WCS transform per point.
- **Time**:
  - the §9 metadata is richer than astropy's, with PHASE axes and epoch-dependent year
    units;
  - an MJD is an integer day and a fraction, as astropy's `day_frac`.
- **Parsed values print back exactly.** `Tform`, `Datetime`, `Scale` and `time::Unit` have
  a `Display` that is the exact inverse of the parse.

## Bugs found in the other libraries (not ours, for reference)

These are from the 2026-10-23 run. The libraries did not change since then.

- **fitsio-pure:**
  - compressed region, section and row reads return garbage (probed);
  - a checksum verify is true when the checksum is absent;
  - BLANK wraps, and `30.0/seconds` loses its value;
  - 16-digit reals do not round-trip;
  - an `L` null reads as false, and the ASCII implied decimal is ignored;
  - an unknown XTENSION ends the HDU list.
- **fitsrs:**
  - RICE is always 32-bit, and GZIP assumes 4 bytes per pixel;
  - there are no edge tiles, and three dither differences from cfitsio;
  - PCOUNT and GCOUNT are ignored, and X columns break the NAXIS1 check;
  - the async reader loses bytes on `Pending`.
- **rust-fitsio:** `hdu()` panics on a VLA or complex column, nulls are never mapped, and
  `write_key` duplicates the card.
