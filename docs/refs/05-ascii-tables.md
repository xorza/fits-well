# 5. ASCII Table Extension (Standard §7.2)

`XTENSION = 'TABLE   '`. Each row is a fixed-length line of ASCII characters;
columns occupy fixed byte ranges. Human-readable but bulky and lossy for floats —
prefer BINTABLE for new data. Data unit is padded with **spaces** (0x20), not NULs.

## 5.1 Data layout

- The data is `NAXIS2` rows of `NAXIS1` bytes each (`BITPIX = 8`, `NAXIS = 2`).
- Column *n* starts at 1-based byte `TBCOLn` and is formatted per `TFORMn`.
- Fields *may* overlap (discouraged) and a row *may* contain bytes outside any
  field — before the first, between, or after the last. Those gap bytes may hold
  **any** 7-bit ASCII; spaces are only a legibility convention, and a CR/LF after
  the last field is explicitly permitted (§7.2.4). Don't assume gaps are spaces.
- The data unit is padded to a 2880-byte block with ASCII **spaces** (§7.2.3).
- A field whose content matches the column's `TNULLn` string is **undefined**.
  Note a *blank* numeric field (`Iw`/`Fw.d`/`Ew.d`/`Dw.d`) is not undefined — it
  reads as **0** (§7.2.5).

## 5.2 Mandatory keywords (Table 14, in order)

| Keyword | Value |
|---------|-------|
| `XTENSION` | `'TABLE   '` |
| `BITPIX` | `8` |
| `NAXIS` | `2` |
| `NAXIS1` | row length in bytes |
| `NAXIS2` | number of rows |
| `PCOUNT` | `0` |
| `GCOUNT` | `1` |
| `TFIELDS` | number of columns (0…999) |
| `TBCOLn` | n = 1…TFIELDS, 1-based start byte of column n |
| `TFORMn` | n = 1…TFIELDS, Fortran format of column n |
| `END` | — |

## 5.3 `TFORMn` formats (Table 15)

Fortran-style format codes:

| Code | Meaning |
|------|---------|
| `Aw`   | Character string of width `w` |
| `Iw`   | Decimal integer in `w` columns |
| `Fw.d` | Floating-point, fixed decimal notation, width `w`, `d` decimals |
| `Ew.d` | Floating-point, exponential notation |
| `Dw.d` | Floating-point, exponential notation (double) |

- `w` = total field width in characters, `d` = digits after the decimal point.
- Each cell is scalar (no repeat counts / arrays in ASCII tables).
- Format codes *must* be upper case; only these five are legal. No repetition,
  scaling, or field-termination editing. All numeric fields are base ten — binary,
  octal, and hexadecimal are not permitted (§7.2.1).
- `F`, `E`, and `D` parse **identically**; the string content alone determines the
  value. `D` only hints the column needs more than 32-bit precision (Appendix E).
- Numbers may carry sign and exponent. Real fields *should* contain an explicit
  decimal point — implicit decimal points (the `d` is assumed) are permitted but
  deprecated. An undefined entry is the field matching `TNULLn`.

## 5.4 Reserved keywords (§7.2.2)

In addition to the §4.4.2 reserved keywords (except `EXTEND` and `BLOCKED`):

- `TTYPEn` — column name (compared case-insensitively; letters/digits/underscore
  recommended).
- `TUNITn` — physical units (of the value after `TSCALn`/`TZEROn`), per §4.3.
- `TSCALn`/`TZEROn` — linear scaling, `physical = TZEROn + TSCALn × field`
  (defaults `1.0` / `0.0`). *Must not* be used on `A`-format (character) fields.
- `TNULLn` — character string marking an undefined value in column n (a string,
  not an integer as in BINTABLE); implicitly space-filled to the field width.
- `TDISPn` — suggested display format (Table 16; e.g. `Iw.m`, `Bw.m`, `Ew.dEe`,
  `ENw.d`, `Gw.dEe`), overriding the default implied by `TFORMn`.
- `TDMINn`/`TDMAXn` — actual minimum/maximum physical value present in column n.
- `TLMINn`/`TLMAXn` — minimum/maximum *legal* (meaningful) physical value for
  column n (common when constructing histograms).

Plus `EXTNAME`/`EXTVER`/`EXTLEVEL`, `AUTHOR`, `REFERENC` from §4.4.2.

## Implementation notes (this library)

- Parse a row by slicing `[TBCOLn-1 .. TBCOLn-1+w]` for each column, then
  trim and parse per `TFORMn`. Check each field fits within `NAXIS1`; *warn* on
  overlapping fields rather than rejecting — the standard permits overlap (§7.2.4).
- Writing: format each value to its field width, right-justify numerics,
  left-justify strings, fill gaps with spaces, terminate row at `NAXIS1`.
- Floats lose precision in ASCII; surface a lint/warning when an `F`/`E`/`D`
  width can't represent the source value's precision.
- ASCII-table parsing is much slower than BINTABLE; keep it correct and simple
  rather than micro-optimized — steer users to BINTABLE for performance.
