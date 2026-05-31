# 1. File Organization (Standard §3)

A FITS file is a sequence of one or more **Header/Data Units (HDUs)**, each
laid out on a strict 2880-byte grid. Everything is big-endian and built from
ASCII-text headers followed by optional binary data. In order (§3.1): the
**primary HDU**, then zero or more **conforming extensions**, then optional
**special records**. A primary-only file is a *Single Image FITS* (SIF); one
with extensions, a *Multi-Extension FITS* (MEF).

## 1.1 The 2880-byte block

The fundamental unit of layout is the **logical record / block = 2880 bytes**
(historically the least common multiple of common tape word sizes). Rules:

- Every **header unit** is an integral number of 2880-byte blocks.
- Every **data unit** is an integral number of 2880-byte blocks.
- The last block of each unit is **padded** to fill 2880 bytes:
  - **Header padding** uses ASCII space (0x20).
  - **Data padding** sets all remaining bits to zero, i.e. ASCII NUL (0x00) —
    *except* ASCII-table data, which is padded with ASCII space (0x20) (§7.2.3).
- Because every header and data unit is a whole number of blocks, a FITS file's
  total length on disk is always a multiple of 2880 bytes.

2880 bytes = exactly **36 keyword records** of 80 bytes each.

## 1.2 HDU anatomy

```
+------------------------------------------------------+
| HDU 0  (PRIMARY)                                     |
|   Header unit:  N × 2880 bytes  (ASCII, ends in END) |
|   Data unit:    M × 2880 bytes  (optional)           |
+------------------------------------------------------+
| HDU 1  (XTENSION = 'IMAGE' / 'TABLE' / 'BINTABLE')   |
|   Header unit:  ...                                  |
|   Data unit:    ...                                  |
+------------------------------------------------------+
| HDU 2 ...                                            |
+------------------------------------------------------+
```

- The **first** HDU is the **Primary HDU** (a.k.a. primary array). Its header
  begins with `SIMPLE = T`.
- Subsequent HDUs are **extensions**; their headers begin with `XTENSION = '...'`.
- A data unit may be empty (`NAXIS = 0`, or all axes present but size 0).

## 1.3 Primary HDU (§3.3)

- Header must start with the mandatory sequence `SIMPLE`, `BITPIX`, `NAXIS`,
  `NAXIS1..NAXISn`, … , `END` (see [headers](02-headers-keywords.md)).
- `SIMPLE = T` asserts the file conforms to the Standard. `SIMPLE = F` is
  permitted but means the file departs from the Standard in unspecified ways.
- The primary data array, if present, is a single contiguous array of **1 to 999
  axes** (`NAXIS`), stored in Fortran order — Axis 1 varies fastest (§3.3.2).
- `EXTEND = T` is a (reserved, advisory) flag that extensions *may* follow.

## 1.4 Extensions (§3.4)

A **conforming extension** satisfies the generic requirements of §3.4.1
(mandatory keyword order below) and has an IAUFWG-registered `XTENSION` type name
(Appendix F). A **standard extension** is one of the three types whose content is
fully specified in §7; other conforming types exist but are outside this Standard:

| `XTENSION` value | Meaning | Ref |
|------------------|---------|-----|
| `'IMAGE   '`     | N-dim array, same data model as primary array | [§7.1](04-images.md) |
| `'TABLE   '`     | ASCII table | [§7.2](05-ascii-tables.md) |
| `'BINTABLE'`     | Binary table (also carries a heap) | [§7.3](06-binary-tables.md) |

`XTENSION` values are space-padded to 8 characters inside the 80-byte record.

### Mandatory keywords in conforming extensions (Table 10)

| Position | Keyword |
|----------|---------|
| 1 | `XTENSION` |
| 2 | `BITPIX` |
| 3 | `NAXIS` |
| 4 | `NAXISn`, n = 1…NAXIS |
| 5 | `PCOUNT` |
| 6 | `GCOUNT` |
| … | (other keywords) |
| last | `END` |

- `PCOUNT`: 0 for IMAGE/TABLE; = heap byte count for BINTABLE; = parameter count
  for random groups.
- `GCOUNT`: 1 for IMAGE/TABLE/BINTABLE; = number of groups for random groups.
- The keywords above are **ordered and mandatory**; no other keyword may
  intervene between `XTENSION` and `GCOUNT` (§4.4.1.2).

### Order of extensions (§3.4.3)

An extension may follow the primary HDU or another conforming extension.
Standard and other conforming extensions **may appear in any order** in a FITS
file. (The **random-groups** structure of §6 is a primary-HDU feature, never an
extension.)

## 1.5 Special records & physical blocking (§3.5–3.6)

- **Special records** (§3.5): 2880-byte blocks after the last HDU whose structure
  is not defined by the Standard. Their first 8 bytes *must not* be `XTENSION`
  (and *should not* be `SIMPLE␣␣`), so a reader can tell them from an extension.
  Restricted use; a reader may ignore them.
- **Physical blocking** (§3.6): on sequential media, blocks of 1–10 logical
  records (i.e. 2880–28800 bytes). On disk this is irrelevant — read/write the
  byte stream directly. Bytes past the FITS end in a trailing partial physical
  block are zero-filled on write and disregarded on read; sub-2880-byte files
  (e.g. tape labels) are not part of the FITS file.

## 1.6 Sizing formulas

Data size in bits (excluding fill) has three cases, each with its own equation in
the Standard (numbers match §4.4.1 / §6.1):

```
primary array (§4.4.1.1):
  Nbits = |BITPIX| × (NAXIS1 × NAXIS2 × … × NAXISm)                    (Eq. 1)

conforming extension (§4.4.1.2):
  Nbits = |BITPIX| × GCOUNT × (PCOUNT + NAXIS1 × NAXIS2 × … × NAXISm)  (Eq. 2)

random groups (§6.1; NAXIS1 = 0 is the format signature):
  Nbits = |BITPIX| × GCOUNT × (PCOUNT + NAXIS2 × NAXIS3 × … × NAXISm)  (Eq. 4)
```

`Nbits` must be non-negative. An `IMAGE` extension uses Eq. 2 with `PCOUNT = 0`
and `GCOUNT = 1`, so it reduces to Eq. 1. **Random groups skip `NAXIS1`** (the
zero sentinel) and need Eq. 4 — applying Eq. 2 verbatim would wrongly zero the
array term.

Data-unit byte length = `ceil(Nbits / 8 / 2880) × 2880`.

## 1.7 Restrictions on changes (§3.7)

"Once *FITS*, always *FITS*": any structure valid under this Standard stays valid
forever. A later revision may *deprecate* a structure (e.g. random groups) but
never invalidate it — so a reader must keep handling legacy forms indefinitely,
even ones a writer should no longer emit.

## Implementation notes (this library)

- Treat the 2880-byte block as the I/O quantum; memory-map or read in block
  multiples to keep parsing branch-free and cache-friendly.
- A parser is essentially: locate HDU boundaries by scanning headers for `END`,
  round header length up to a block, compute data length from `BITPIX`/`NAXIS*`/
  `PCOUNT`/`GCOUNT`, round up to a block, advance. Repeat to EOF.
- HDU boundaries are computable without reading data — enables lazy/seeking access.
