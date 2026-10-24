# How cfitsio, astropy and wcslib read four cases the review left open

Date: 2026-10-25. Sources read: cfitsio 4.7.0 (commit `113781d`), astropy (commit
`6edc126`) and the wcslib it bundles (`astropy/cextern/wcslib`), the FITS 4.0 standard
(`docs/refs/fits_standard40.md`), and Calabretta & Greisen 2002 (Paper II,
arXiv astro-ph/0207413).

## ASCII implied decimal point

- **The standard.** §7.2.5, real fields, rule 2: "If the string contains no explicit
  decimal point, then the implicit decimal point is taken as immediately preceding the
  rightmost d digits of the string". Rule 3 then reads the exponent, and the value is
  "numeric string × 10^(exponent string)". The implied point applies to the numeric
  string with or without an exponent, so `1E5` in `E10.2` is 0.01 × 10⁵ = 1000.
- **cfitsio.** `fffstrr8` (`getcold.c:1710-1713`) sets `power = implipower` whenever
  there is no point, then computes `(sign * val / power) * pow(10., esign * exponent)`.
  `implipower` is 10^d from `ffasfm` for every real format (`getcold.c:776-780`). It reads
  `1E5` in `E10.2` as 1000, as the standard does. It accepts only `E` and `D` as exponent
  letters, skips embedded blanks, and takes `,` as a decimal point.
- **astropy.** `_convert_ascii` (`io/fits/fitsrec.py:990-1031`) replaces `D` with `E` and
  casts the field text with numpy. It ignores the implied point altogether: `123` in
  `F8.2` reads 123.0, not 1.23. The numpy cast accepts `nan` and `inf`.

## HCOMPRESS on an image of more than two axes

- **The standard.** §10.4.4: the algorithm "can be applied only to images with two
  dimensions". It does not say whether that is the image or the tile.
- **cfitsio, write.** `imcomp_init_table` (`imcompress.c:1037-1191`) accepts any `NAXIS ≥
  2`. Exactly two tile axes may exceed 1, each at least 4 pixels, on any two axes; every
  other tile extent is 1. A default fpack tiling of a cube is (`NAXIS1`, 16, 1, …). The
  codec gets `nx` as the first tile extent above 1 and `ny` as the product of the rest
  (`fits_calc_tile_rows`, `imcompress.c:9912-9951`).
- **cfitsio, read.** `fits_hdecompress` takes `nx` and `ny` from the stream
  (`imcompress.c:6362-6373`), with no check on `ZNAXIS`.
- **astropy.** `_tiled_compression.py:98-106` drops the tile extents of 1 and requires two
  that remain, as `nx` and `ny`; `utils.py:28-48` refuses more than two above 1 on write.

## A bad header after the primary

- **cfitsio.** `ffopen` reads the primary alone. `ffthdu` (`fitscore.c:8037-8066`) counts
  HDUs by moving to each one until a move fails, and ignores that failure's status. The
  count is the HDUs before the bad one. A move to the bad HDU itself returns an error.
- **astropy.** `HDUList._read_next_hdu` (`io/fits/hdu/hdulist.py:1301-1389`) reads HDUs
  lazily. A `VerifyError` or `ValueError` warns "There may be extra bytes after the last
  HDU or the file is corrupted" and ends the list. A header with no `END` card raises
  `OSError("Header missing END card.")` (`header.py:615`) when that HDU is reached; the
  HDUs before it stay readable.

## `PC`, `CD` and `CROTA` in one header

- **Paper II §6.2:** "Modern FITS-writers must not attempt to help older interpreters by
  including CROTAi together with the new keyword values" (`PCi_ja` or `CDi_ja`). The paper
  says nothing about how a reader treats such a header.
- **wcslib.** `wcsset` (`wcs.c:2846-2885`) uses `PCi_ja` when one is present (`altlin & 1`),
  else `CDi_ja`, else `CROTA` of the latitude axis. A `CROTA` on any other axis is never
  read. A mixed header gives no error.
- **wcslib, the pole.** `wcs.h:1295-1302`: `LONPOLEa` and `LATPOLEa`, or the `PVi_ma` of
  the longitude axis, "which takes precedence if defined".
