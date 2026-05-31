# 2. Headers & Keyword Records (Standard §4)

A header unit is a sequence of fixed-length **80-byte keyword records** (ASCII),
terminated by an `END` record and padded with blank records to a 2880-byte
boundary. Only restricted ASCII (decimal 32–126) is allowed in a header.

## 2.1 Keyword-record layout (§4.1)

```
 col:  1        9 10                                              80
       |        | |                                               |
       KKKKKKKK = <value>           / <comment>
       \______/\_/ \_____________________________/
        keyword  =  value field           comment (after first ' / ')
        (8 bytes)
```

- **Bytes 1–8**: keyword name, left-justified, space-padded.
  Allowed characters: `A–Z`, `0–9`, `-`, `_`. (Uppercase only.)
- **Value indicator**: bytes 9–10 are `= ` (equals + space) for a keyword that
  has a value. Commentary keywords (`COMMENT`, `HISTORY`, blank) have **no**
  value indicator — bytes 9–80 are free text.
- **Value field**: bytes 11–80 (free-format), optionally followed by a comment.
- **Comment**: everything after the first `/` outside a string literal. Optional.

### Fixed vs free format

The Standard defines **fixed-format** positions (e.g. logical/integer values
right-justified ending in byte 30) and **free-format** (value anywhere from
byte 11). Mandatory keywords **must** be fixed-format; a robust reader should
accept free-format for everything.

## 2.2 Value types (§4.2)

| Type | Syntax | Example value field |
|------|--------|---------------------|
| Character string | single-quoted; `''` escapes a literal quote; leading spaces significant, trailing not | `'Cygnus X-1'` |
| Logical | `T` or `F` (fixed: byte 30) | `T` |
| Integer | optional sign + digits | `16` |
| Real float | Fortran/`C` float; exponent letter `E` or `D`, **upper-case only** (§4.2.4) | `1.5`, `3.14E2`, `2.0D0` |
| Complex integer | `(re, im)` | `(3, 4)` |
| Complex float | `(re, im)` | `(1.0, -2.5)` |
| Date | string in ISO-8601 (`YYYY-MM-DD[Thh:mm:ss[.sss…]]`) | `'2006-10-22'` |

- Three distinct string-ish cases (§4.2.1) — do **not** conflate them:
  - `KEYWORD= ''` — a **null** (zero-length) string.
  - `KEYWORD= '   '` — an **empty** string. Per §4.2.1.1 the *first* space is
    significant and trailing spaces are not, so it reduces to a single space
    (length 1), **not** length 0 — that one significant space is exactly what
    distinguishes the empty string from the null string above.
  - `KEYWORD=` (blank value field, no quotes) — an **undefined** value.
- No minimum string length is required, *except* `XTENSION` values must be padded
  to 8 characters for backward compatibility (e.g. `'IMAGE   '`).
- A single-record string holds ≤ 68 content characters (opening quote at byte 11,
  closing quote by byte 80). Longer values use the **CONTINUE** long-string
  convention (substrings < 68 chars chained with a trailing `&`; registered
  convention, folded into 4.0).
- Numbers must fit the value field; no thousands separators.

## 2.3 Units (§4.3)

Physical units go in the **comment** field, recommended in square brackets at the
start: `/ [m/s] heliocentric velocity`. Units strings follow the conventions in
Standard Table 6 (and IAU style). Not machine-enforced; the library should treat
units as opaque comment text but expose helpers to parse the `[...]` prefix.

## 2.4 Mandatory keywords

### Primary header (§4.4.1; mandatory list Table 7, example Table 9)

In order:

| Keyword | Value | Notes |
|---------|-------|-------|
| `SIMPLE` | `T`/`F` | First record. `T` = conforms. |
| `BITPIX` | int | Data type — see [data representation](03-data-representation.md). |
| `NAXIS`  | int ≥ 0 | Number of axes (0 ⇒ no data array). |
| `NAXISn` | int ≥ 0 | n = 1…NAXIS; axis lengths. |
| `END`    | —       | Last record; no value, no comment. |

Example:
```
SIMPLE  =                    T / file does conform to FITS Standard
BITPIX  =                   16 / number of bits per data pixel
NAXIS   =                    2 / number of data axes
NAXIS1  =                  250 / length of data axis 1
NAXIS2  =                  300 / length of data axis 2
OBJECT  = 'Cygnus X-1'
DATE    = '2006-10-22'
END
```

### Conforming extension header (§3.4.1)

`XTENSION`, `BITPIX`, `NAXIS`, `NAXISn`, `PCOUNT`, `GCOUNT`, … , `END`
(see [file structure §1.4](01-file-structure.md)).

## 2.5 Reserved keywords (§4.4.2)

Optional but, *if present*, must be used as defined. Common ones:

- **General**: `DATE`, `ORIGIN`, `EXTEND`, `BLOCKED` (deprecated), `DATE-OBS`,
  `TELESCOP`, `INSTRUME`, `OBSERVER`, `OBJECT`, `AUTHOR`, `REFERENC`, `EQUINOX`.
- **Bibliographic / commentary**: `COMMENT`, `HISTORY`, blank keyword.
- **Array scaling**: `BSCALE`, `BZERO`, `BUNIT`, `BLANK`, `DATAMIN`, `DATAMAX`.
- **Extension naming**: `EXTNAME`, `EXTVER`, `EXTLEVEL`, `INHERIT`.
- **WCS**: `WCSAXES`, `CTYPEi`, `CRPIXi`, `CRVALi`, `CDELTi`, `CUNITi`, `CROTAi`,
  `PCi_j`, `CDi_j`, `CRDERi`, `CSYERi`, `LONPOLE`, `LATPOLE`, `RADESYS`,
  plus alternate-axis variants `…a` (a = `A`–`Z`). See [WCS](07-wcs-time-compression.md).
- **Table keywords**: `TFIELDS`, `TTYPEn`, `TFORMn`, `TUNITn`, `TSCALn`,
  `TZEROn`, `TNULLn`, `TDISPn`, `TBCOLn` (ASCII), `TDIMn`, `THEAP`.

## 2.6 Commentary keywords

`COMMENT`, `HISTORY`, and the **blank keyword** (8 spaces) carry free text in
bytes 9–80 with no value indicator. They may repeat arbitrarily and their order
is significant. A header model must preserve duplicates and ordering.

## Implementation notes (this library)

- Parse a record as: name = bytes[0..8] trimmed; if bytes[8..10] == `= ` it has a
  value, else commentary. Split value/comment on the first `/` that is not inside
  a string literal (track quote state).
- Keep the header an **ordered** list of records (not a map) to round-trip exactly,
  with an auxiliary index for O(1) keyword lookup. Duplicate keywords are legal
  for commentary; for valued keywords first-wins is the usual reader policy.
- Writing: emit fixed-format for mandatory keywords; pad each record to 80 bytes;
  emit `END`; pad header to a 2880 multiple with spaces.
- A blazing-fast reader can scan for `END` at 80-byte strides and only fully parse
  records on demand.
