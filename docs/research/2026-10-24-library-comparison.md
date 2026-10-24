# fits-well compared with cfitsio, rust-fitsio, fitsio-pure, fitsrs and astropy

From source reads on 2026-10-24, fits-well at `69f5a25`, the others at cfitsio 4.7.0
(`113781d`), rust-fitsio 0.21.10 (`ee9c3b3`), fitsio-pure 0.22.0 (`2543a9d`), fitsrs
0.4.2 (`c7e9c28`) and astropy (`6edc126`). The README's table is the summary for users;
the decisions on what fits-well refuses are in `docs/design.md`.

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

## What fits-well lacks, ranked

Important:

1. **wcsfix-style repairs:** `CUNIT = 'DEG'` or `'degree'`, the AIPS `-NCP` and `-GLS`
   projections, `FREQ-LSR`. They are `InvalidUnit` or an unsupported projection.
2. **A cfitsio differential test job and a fuzz harness.** The golden fixtures come from
   fpack, funpack and astropy, but no CI job compares with a running cfitsio, as
   fitsio-pure has. None of the libraries has a fuzz harness.

Medium:

- A NULL_PIXEL_MASK write: lossy HCOMPRESS of an image with undefined pixels is refused.
- Typed unsigned table columns on write: `ColumnData` has no U16, U32, U64 or I8.
- A streaming table write; only `stream_image` exists.
- A WCS axis subset, as wcslib's `wcssub`.
- An in-place data edit, as cfitsio has.

Niche or legacy: the extended filename syntax, row filters, histograms, templates, the
grouping convention, region files, BZIP2_1, RVKC, a byte-exact header round trip,
fitsdiff and fitsverify, IRAF `.imh`.

## Bugs in the other libraries

- **fitsio-pure:** compressed region, section and row reads return garbage; a checksum
  verify is true when the checksum is absent; `BLANK` wraps, and `30.0/seconds` loses
  its value; 16-digit reals do not round-trip; an `L` null reads as false, and the ASCII
  implied decimal is ignored; an unknown `XTENSION` ends the HDU list; three crafted
  files panic.
- **fitsrs:** RICE is always 32-bit, and GZIP assumes 4 bytes a pixel; there are no edge
  tiles, and three dither differences from cfitsio; `PCOUNT` and `GCOUNT` are ignored,
  and `X` columns break the `NAXIS1` check; the async reader loses bytes on `Pending`;
  I/O errors are swallowed.
- **rust-fitsio:** `hdu()` panics on a variable-length or complex column, nulls are
  never mapped, and `write_key` duplicates the card.
- **cfitsio:** CVE fixes in 4.6.0, 4.6.4 and 4.7.0; it repairs a near-valid header with
  no report, and moves every byte after a header that grows.
- **astropy:** refuses a u16 `PLIO_1` write and misreads cfitsio's.
