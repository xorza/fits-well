# fits-well 0.3 compared with cfitsio, rust-fitsio, fitsio-pure, fitsrs and astropy

Date: 2026-10-23. Clones are in /tmp/fits-compare. fits-well at 3f021e7.
Evidence is from source reads. Items marked "probed" were run against fits-well.

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
| Compressed image codecs (write) | all 5 + NOCOMPRESS, parallel | all + BZIP2 | via filename | RICE, GZIP 1/2 | none | all |
| Lossless float compression | **no** (read bug) | yes | via cfitsio | ? | panics | yes |
| Compressed tables §10.3 | r/w, partial reads | whole-HDU convert only | no | no | no | no |
| NULL_PIXEL_MASK | read | no | no | no | no | ? |
| CHECKSUM/DATASUM | write + verify, summed on read | + update in place | no | verify says true if absent | no | yes |
| HIERARCH | opaque commentary | r/w | — | compat only | read | r/w |
| In-place edit / append | no | full | partial | header + compat | no | full |
| Lenient header parse | no, whole open fails | yes | yes | yes | partial | fix modes |
| Non-seekable / async input | no | stdin | — | stream `Read` | async | fsspec |
| `.fits.gz` file input | no | yes + URLs | yes | compat | yes | yes |
| WCS | 27 proj, spectral, -TAB, table WCS | 9 AIPS proj | no | no | 23 proj, 2-D, SIP | wcslib full |
| SIP / TPV distortion | **no** | no | no | no | SIP | yes |
| WCS header write, sub/slice | no | — | no | no | no | yes |
| Time §9 | metadata, f64 MJD | strings only | no | no | no | full + scale conversion |
| Hostile-input safety | checked | CVE history | inherits cfitsio | panics on 3 probes | swallows errors | — |

## Confirmed fits-well bugs (logged in fits-well/.notes/ISSUES.md)

- Probed: a lossless compressed float image (no ZSCALE/ZZERO columns, §10.2) reads raw float bits as quantized integers. Silently wrong.
- Probed: `ZQUANTIZ = 'NONE'` (what fpack -q 0 writes) is an error.
- Probed: PLIO_1 write of a u16 image fails. cfitsio stores u16 PLIO offset by 32768, fits-well does not, so cfitsio files differ.
- docs/design.md cites a research file that does not exist.

## fits-well lacks, ranked

Important:
1. SIP distortion (then TPV). Plate solvers (astrometry.net, ASTAP) and HST/DECam data use it. Today `RA---TAN-SIP` gives `UnsupportedWcsTransform`.
2. Lenient or repair-mode header parse. One bad card in any HDU fails `FitsReader::open` (src/reader/mod.rs:257). cfitsio skips value parse on scan (getkey.c:3203), astropy has fix/silentfix/warn, fitsio-pure accepts lowercase keywords and `=` with no space.
3. Lossless float compression write; quantize level 0 = lossless as in cfitsio (fits-well: 0 = default 4).
4. Per-image dither seed. `ZDITHER0` is fixed at 1 (src/compress/encode/mod.rs:312). fpack's guide warns a shared pattern hurts co-adds. This affects lumos stacks.
5. In-place header update (WCS write-back after a plate solve), append, checksum update in place.
6. Typed HIERARCH get/set (ESO data).
7. WCS header write and sub/slice, so a crop or bin can carry its WCS.
8. `.fits.gz` input and non-seekable `Read` input.

Medium:
- HCOMPRESS/PLIO for float writes, HCOMPRESS SMOOTH on write, absolute quantization step (negative level), noise estimate min(noise2, noise3, noise5) as cfitsio (fits-well uses noise3 only, so a larger step).
- Compressed float images over 2 GiB heap (float path fixed to 1PB).
- wcsfix-style repairs: `CUNIT='DEG'`/`'degree'`, AIPS `-NCP`/`-GLS`, `FREQ-LSR`, PC with CROTA.
- Time: two-part MJD (f64 gives ~0.63 µs at MJD 60000), exact §9 scale relations (TT/TAI/GPS/TCG/TCB) that need no external data.
- Public FITS unit grammar (§4.3). `unit.rs` is crate-private and narrow.
- ZSCALE/ZZERO as keywords (a cfitsio extension, not in §10).
- Interop: fits-well writes `ZCTYPn='NOCOMPRESS'` and GZIP_2 on L/X/A/B/C/M columns. Both are valid, but cfitsio cannot read them.
- Unsigned table columns (U16/U32/U64/I8 in `ColumnData`).
- Streaming table write.
- A cfitsio differential test job (fitsio-pure has one) and a fuzz harness (none of the libraries has one).

Niche or legacy: extended filename syntax, row filter/calculator, histograms, iterator framework, templates, grouping convention, region files, random groups write, BZIP2_1, RVKC, byte-exact header round trip, fitsdiff/fitsverify tools, IRAF .imh, `RICE_ONE` alias, zlib tile fallback.

## fits-well does better

- Compressed tables: transparent read/write with partial reads by tile. cfitsio only converts a whole HDU, astropy has none.
- WCS breadth without wcslib: 27 projections + HPX, PC/CD/PVi_m, alternates, all spectral algorithms, -TAB evaluated, binary-table WCS. Equal to astropy, far above every Rust crate and cfitsio's own helpers.
- Codec coverage and correctness: bit-exact fixtures against fpack/funpack/astropy, NULL_PIXEL_MASK, tile-parallel encode/decode, section reads decode only touched tiles.
- Hostile-input safety: NAXIS cap, exact inflate size, checked offsets. fitsio-pure panics on three crafted files, fitsrs swallows I/O errors, cfitsio had CVE fixes in 4.6.0/4.6.4/4.7.0.
- Header model: typed `Keyword`, O(1) index, exact big integers, shortest exact real formatting, NaN refused, END checked, CONTINUE `&` rules right.
- Checksums summed during the read, with `Absent` kept apart from valid.
- Zero-copy `from_bytes`, caller-owned buffers, `read_table_columns_into`, allocation-free WCS per point.
- §9 metadata richer than astropy's: PHASE axes, epoch-dependent year units.

## Bugs found in the other libraries (not ours, for reference)

- fitsio-pure: compressed region/section/row reads return garbage (probed), checksum verify true when absent, BLANK wraps, `30.0/seconds` loses value, 16-digit reals do not round-trip, L null reads false, ASCII implied decimal ignored, unknown XTENSION ends the HDU list.
- fitsrs: RICE always 32-bit, GZIP assumes 4 bytes/pixel, no edge tiles, three dither differences from cfitsio, PCOUNT/GCOUNT ignored, X columns break NAXIS1 check, async loses bytes on Pending.
- rust-fitsio: `hdu()` panics on a VLA or complex column, nulls never mapped, `write_key` duplicates the card.
