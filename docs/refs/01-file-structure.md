# 1. File Organization (Standard §3)

A FITS file is a sequence of one or more **Header/Data Units (HDUs)**, each
laid out on a strict 2880-byte grid. Everything is big-endian and built from
ASCII-text headers followed by optional binary data.

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
- The primary data array, if present, is a single contiguous N-dimensional array.
- `EXTEND = T` is a (reserved, advisory) flag that extensions *may* follow.

## 1.4 Extensions (§3.4)

A **conforming extension** satisfies the generic requirements of §3.4.1
(mandatory keyword order below). A **standard extension** is one of the three
IAUFWG-registered types:

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

### Order of extensions (§3.4.3)

An extension may follow the primary HDU or another conforming extension.
Standard and other conforming extensions **may appear in any order** in a FITS
file. (The **random-groups** structure of §6 is a primary-HDU feature, never an
extension.)

## 1.5 Special records & physical blocking (§3.5–3.6)

- **Special records** (§3.5): blocks after the last standard HDU whose content is
  not defined by the Standard. Restricted use; a reader may ignore them.
- **Physical blocking** (§3.6): on sequential media, blocks of 1–10 logical
  records (i.e. 2880–28800 bytes). On disk this is irrelevant — read/write the
  byte stream directly.

## 1.6 Sizing formulas

Data-array size in bits (excluding fill), for primary array / IMAGE:

```
Nbits = |BITPIX| × (NAXIS1 × NAXIS2 × … × NAXISm)        (Eq. 1)
```

For conforming extensions (and random groups), including PCOUNT/GCOUNT:

```
Nbits = |BITPIX| × GCOUNT × (PCOUNT + NAXIS1 × … × NAXISm)   (Eq. 2)
```

Data-unit byte length = `ceil(Nbits / 8 / 2880) × 2880`.

## Implementation notes (this library)

- Treat the 2880-byte block as the I/O quantum; memory-map or read in block
  multiples to keep parsing branch-free and cache-friendly.
- A parser is essentially: locate HDU boundaries by scanning headers for `END`,
  round header length up to a block, compute data length from `BITPIX`/`NAXIS*`/
  `PCOUNT`/`GCOUNT`, round up to a block, advance. Repeat to EOF.
- HDU boundaries are computable without reading data — enables lazy/seeking access.
