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
  A positive integer suffix on an indexed keyword has no leading zeros.
- **Value indicator**: bytes 9–10 are `= ` (equals + space) for a keyword that
  has a value. Commentary keywords (`COMMENT`, `HISTORY`, blank) have **no**
  value indicator — bytes 9–80 are free text.
- **Value field**: bytes 11–80 (free-format), optionally followed by a comment.
- **Comment**: everything after the first `/` outside a string literal. Optional.

### Fixed vs free format

The Standard defines **fixed-format** positions (e.g. logical/integer values
right-justified ending in byte 30) and **free-format** (value anywhere from
byte 11). Mandatory keywords **must** be fixed-format; fixed format is recommended
for all other values. Accepting free-format mandatory values is a deliberate
tolerance for non-conforming input, not part of the format definition.

## 2.2 Value types (§4.2)

| Type | Syntax | Example value field |
|------|--------|---------------------|
| Character string | single-quoted; `''` escapes a literal quote; leading spaces significant, trailing not | `'Cygnus X-1'` |
| Logical | `T` or `F` (fixed: byte 30) | `T` |
| Integer | optional sign + digits | `16` |
| Real float | Fortran/`C` float; exponent letter `E` or `D`, **upper-case only** (§4.2.4) | `1.5`, `3.14E2`, `2.0D0` |
| Complex integer | `(re, im)` | `(3, 4)` |
| Complex float | `(re, im)` | `(1.0, -2.5)` |
| Datetime pseudo-type | ISO-8601 subset stored as a character string (`[±C]CCYY-MM-DD[Thh:mm:ss[.sss…]]`) | `'2006-10-22'` |

- Three distinct string-ish cases (§4.2.1) — do **not** conflate them:
  - `KEYWORD= ''` — a **null** (zero-length) string.
  - `KEYWORD= '   '` — an **empty** string. Per §4.2.1.1 the *first* space is
    significant and trailing spaces are not, so it reduces to a single space
    (length 1), **not** length 0 — that one significant space is exactly what
    distinguishes the empty string from the null string above.
  - `KEYWORD=` (blank value field, no quotes) — an **undefined** value.
- No minimum string length is required, *except* `XTENSION` values must be padded
  to 8 characters for backward compatibility (e.g. `'IMAGE   '`).
- A fixed-format single-record string holds at most 68 content characters
  (opening quote at byte 11, closing quote by byte 80). Longer values use the
  normative **CONTINUE** long-string form (substrings shorter than 68 characters
  chained with a trailing `&`).
- Numbers must fit the value field; no thousands separators.

## 2.3 Units (§4.3)

Units may be keyword values (`BUNIT`, `TUNITn`), table entries, or text in a
keyword comment. The Standard recommends putting comment-field units in square
brackets at the start: `/ [m/s] heliocentric velocity`. Software must not assume
that every bracketed comment is a valid unit or that units are always present
there.

- Unit names and prefixes are case-sensitive: `m` is metre, `M` is mega, and
  `Hz` is hertz. A single SI prefix (two characters for `da`) may precede a base
  unit; compound prefixes are forbidden.
- Standard Tables 3–4 define the IAU basic and additional astronomy units.
  Reserved angular keyword values should use degrees, and any keyword-specific
  required unit overrides the general recommendations.
- Compound strings use space, `*`, or `.` for multiplication; `/` for division;
  `**`, `^`, or an immediately following expression for powers; and
  `log(...)`, `ln(...)`, `exp(...)`, or `sqrt(...)`. An optional leading power
  of ten may be written as `10**k`, `10^k`, or `10±k`.
- Parentheses should remove precedence ambiguity. Multiple `/` characters are
  legal but discouraged; spaces as multiplication operators are also discouraged.

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

No other record may intervene from `SIMPLE` through the last `NAXISn`. If
`NAXIS = 0`, no `NAXISn` record may occur. `END` has spaces in bytes 9–80 and
must occur in the header's final 2880-byte block.

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
- **Integrity**: `DATASUM`, `CHECKSUM`.
- **WCS**: `WCSAXES`, `CTYPEi`, `CRPIXi`, `CRVALi`, `CDELTi`, `CUNITi`, `CROTAi`,
  `PCi_j`, `CDi_j`, `CRDERi`, `CSYERi`, `LONPOLE`, `LATPOLE`, `RADESYS`,
  plus alternate-axis variants `…a` (a = `A`–`Z`). See [WCS](07-wcs-time-compression.md).
- **Table keywords**: `TFIELDS`, `TTYPEn`, `TFORMn`, `TUNITn`, `TSCALn`,
  `TZEROn`, `TNULLn`, `TDISPn`, `TBCOLn` (ASCII), `TDIMn`, `THEAP`.

Important reserved-keyword rules:

- `DATE` records HDU creation time and, for Earth-created data in the modern
  format, is UTC. The legacy `DD/MM/YY` form is permitted only for files written
  before 2000 and implies a year in 1900–1999. `DATE-OBS` normally means the
  observation start; its scale should be made explicit when ambiguous.
- `EXTEND` is primary-only and advisory; absence does not prohibit extensions.
  `BLOCKED` is deprecated and should not be authored.
- `EXTVER` and `EXTLEVEL` each default to 1 when absent. The combination of
  extension name/version/level should be unique. `INHERIT`, when present, is an
  extension-only logical record immediately after the mandatory sequence; its
  informational merge convention is summarized in
  [conventions](08-conventions.md).
- `DATAMIN`/`DATAMAX` are floating-point physical extrema even for integer
  arrays and exclude undefined and IEEE special values.
- `DATASUM`/`CHECKSUM` describe only their own HDU. Their exact semantics and
  recommended algorithm are in [conventions](08-conventions.md).

## 2.6 Commentary keywords

`COMMENT`, `HISTORY`, and the **blank keyword** (8 spaces) carry free text in
bytes 9–80 with no value, even if bytes 9–10 happen to contain `= `. They may
repeat arbitrarily and their order is significant. A sequence of fully blank
records immediately before `END` may be treated as preallocated space for future
keywords. A header model must preserve duplicates and ordering.

## 2.7 Additional keywords (§4.4.3)

Applications may define new keywords if they obey the general syntax and do not
conflict with mandatory or reserved names. References to particular HDUs in this
or another file should be used cautiously because those HDUs may not persist.

## Implementation notes (this library)

- Parse a record as: name = bytes[0..8] trimmed; if bytes[8..10] == `= ` it has a
  value, else commentary. Split value/comment on the first `/` that is not inside
  a string literal (track quote state).
- Keep the header an **ordered** list of logical records (not a map), with an
  auxiliary index for O(1) keyword lookup. Commentary records may repeat.
  Mandatory keywords must not repeat; other valued keywords should not repeat,
  and conflicting duplicates have an **indeterminate** value under the Standard.
  Do not silently give a first or last occurrence normative precedence.
  Rendering may normalize physical value layout and `CONTINUE` splits.
- Writing: emit fixed-format for mandatory keywords; pad each record to 80 bytes;
  emit `END`; pad header to a 2880 multiple with spaces.
- A blazing-fast reader can scan for `END` at 80-byte strides and only fully parse
  records on demand.
