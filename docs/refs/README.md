# FITS Standard — Reference Notes

Curated, implementation-focused notes on the **FITS** (Flexible Image Transport
System) standard, distilled from the authoritative sources for building this
library. The canonical normative document is included verbatim as a PDF; the
markdown files are working references that paraphrase and tabulate it for quick
lookup while coding.

> These notes are **non-normative**. When in doubt, the PDF (`fits_standard40.pdf`)
> and the official text at <https://fits.gsfc.nasa.gov/fits_standard.html> win.

## Contents

| File | Covers | Standard §|
|------|--------|-----------|
| [`01-file-structure.md`](01-file-structure.md) | File organization, HDUs, 2880-byte blocking, padding | 3 |
| [`02-headers-keywords.md`](02-headers-keywords.md) | Keyword-record syntax, value types, mandatory & reserved keywords | 4 |
| [`03-data-representation.md`](03-data-representation.md) | BITPIX, integer/IEEE encodings, byte order, BSCALE/BZERO, unsigned trick | 5 |
| [`04-images.md`](04-images.md) | Primary array & IMAGE extension | 6, 7.1 |
| [`05-ascii-tables.md`](05-ascii-tables.md) | TABLE extension, Fortran TFORM codes | 7.2 |
| [`06-binary-tables.md`](06-binary-tables.md) | BINTABLE, TFORM data types, heap, variable-length arrays | 7.3 |
| [`07-wcs-time-compression.md`](07-wcs-time-compression.md) | WCS, time coordinates, tiled image compression (overview) | 8, 9, 10 |
| [`08-conventions.md`](08-conventions.md) | CONTINUE long strings, CHECKSUM/DATASUM, HIERARCH | 4.2.1.2, 4.4.2.7, App. J, registry |

## Primary sources

- **FITS Standard 4.0** (language-edited, 13 Aug 2018) — the normative definition.
  PDF: `fits_standard40.pdf` (normative source) · Markdown: `fits_standard40.md`
  (full verbatim conversion of the PDF, with reconstructed headings + TOC; handy
  for grep/linking) · Online: <https://fits.gsfc.nasa.gov/standard40/fits_standard40aa-le.pdf>
- **FITS Support Office** (NASA GSFC) — standard, documentation, conventions registry:
  <https://fits.gsfc.nasa.gov/>
- **Registry of FITS Conventions** — non-normative but widely deployed conventions
  (CHECKSUM, CONTINUE long strings, tiled compression, etc.):
  <https://fits.gsfc.nasa.gov/fits_registry.html>
- **A Primer on the FITS Data Format** — gentle introduction:
  <https://fits.gsfc.nasa.gov/fits_primer.html>
- **HEASARC FITS dictionary** — reserved-keyword dictionary:
  <https://heasarc.gsfc.nasa.gov/docs/fcg/standard_dict.html>

## Maintenance

The standard is governed by the **IAU FITS Working Group (IAUFWG)**. FITS 4.0 was
approved July 2016, language-edited version ratified 13 Aug 2018. A core promise of
FITS is **"once FITS, always FITS"** — the format is append-only and never breaks
backward compatibility with previously valid files. Implementations must keep reading
older structures (e.g. random groups) indefinitely.
