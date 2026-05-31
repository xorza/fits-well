# 8. Common Conventions: CONTINUE, CHECKSUM/DATASUM, HIERARCH

Beyond the mandatory format, real-world FITS files lean on a handful of widely
deployed conventions. They differ in normative status — know which is which:

| Convention | Status | Where |
|------------|--------|-------|
| `CONTINUE` long strings | **Normative** — part of the Standard | §4.2.1.2 |
| `CHECKSUM` / `DATASUM` | Reserved **keywords** are normative; the *algorithm* is informational | §4.4.2.7 + Appendix J |
| `HIERARCH` | **Not** in the Standard — a registered (ESO) convention | Conventions Registry |

A library that "supports the whole standard" must read all three (they appear
constantly in archive data) and should write `CONTINUE` and `CHECKSUM`/`DATASUM`.

---

## 8.1 CONTINUE — long-string keywords (§4.2.1.2, normative)

A single keyword record caps a string value at ~68 characters. The `CONTINUE`
convention chains records to express arbitrarily long string values. It was a
registered convention folded into the Standard in 4.0.

### Writing

1. Split the value into substrings of **fewer than 68 characters**. A literal
   single quote is written as two single quotes (`''`), and both halves of such a
   pair must fall within the *same* substring.
2. Append an ampersand `&` to every substring **except the last** — the `&` (the
   last non-space character inside the quotes) flags "continued on the next
   record".
3. Enclose each substring in single quotes.
4. Write the first substring as the value of the actual keyword.
5. Write each remaining substring in a record whose keyword name (bytes 1–8) is
   `CONTINUE`, with **spaces in bytes 9–10** (no `= ` value indicator). The
   quoted substring goes anywhere in bytes 11–80; an optional comment may follow
   a ` / `.

```
WEATHER = 'Partly cloudy during the evening f&'
CONTINUE  'ollowed by cloudy skies overnight.&'
CONTINUE  ' Low 21C. Winds NNE at 5 to 10 mph.' / forecast text
```

### Reading

A string value is continued when it ends (inside the quotes) with `&` **and** the
next record is a conforming `CONTINUE` record. Reconstruct by stripping each
trailing `&` and concatenating the substrings; the comment (if any) is taken from
the last record. A reader that does not implement the convention still parses the
first record as an ordinary (truncated) string and sees the rest as `CONTINUE`
commentary keywords — so the file stays readable.

### Restriction

`CONTINUE` must **not** be applied to any mandatory or reserved keyword unless
that keyword is explicitly declared to be of long-string type.

---

## 8.2 CHECKSUM / DATASUM — integrity check (§4.4.2.7 + Appendix J)

Two reserved keywords let a reader verify an HDU has not been corrupted. The
**keywords** are part of the Standard (§4.4.2.7); the **recommended algorithm**
(Appendix J) is informational but is the de-facto implementation (CFITSIO,
astropy, …), so follow it exactly to interoperate.

The checksum primitive is a **32-bit one's-complement sum**: interpret the bytes
as big-endian 32-bit unsigned integers (each 2880-byte record = 720 of them) and
sum with end-around carry.

### `DATASUM`

- Value: a **character string** holding the unsigned-integer (decimal) value of
  the 32-bit one's-complement checksum of the **data records only** (header
  excluded).
- `'0'` when the HDU has no data unit; may be omitted in that case.
- Must be updated **before** `CHECKSUM`.

### `CHECKSUM`

- Value: a **16-character ASCII string**, written in **fixed format** — opening
  quote in **column 11**, closing quote in **column 28** (placement affects the
  result, so it is fixed).
- It is constructed so that the one's-complement checksum accumulated over the
  **entire HDU** (header *and* data, including the `CHECKSUM` record itself)
  equals **negative zero** (all 32 bits set). Verification is therefore: sum the
  whole HDU; a valid HDU yields `0xFFFFFFFF`.

### Procedure (Appendix J.1)

1. Write `CHECKSUM` with the placeholder value `'0000000000000000'` (quote in
   col 11), including any final comment (a timestamp is recommended).
2. Accumulate the 32-bit one's-complement checksum over the header records.
3. Add it (one's-complement arithmetic) to the data checksum (= `DATASUM`) to get
   the whole-HDU checksum.
4. Take the bit-wise complement of that 32-bit total.
5. ASCII-encode the complement into 16 characters (Appendix J.2): the four bytes
   of the complement are spread over 16 bytes offset from ASCII `'0'` (0x30), with
   a punctuation-avoiding fix-up so every output character is alphanumeric
   (`0–9`, `A–Z`, `a–z`).
6. Replace the placeholder with the encoded string. The HDU checksum is now −0.

If every HDU carries valid keywords, the checksum over the **whole file** is also
−0. Both keywords apply only to their own HDU. Incremental update is possible
(Appendix J.4) without rescanning unchanged records.

---

## 8.3 HIERARCH — hierarchical / long keyword names (registered, non-standard)

The Standard limits keyword names to **8 characters** drawn from
`[A-Z0-9_-]`. The **ESO HIERARCH** convention (in the FITS Conventions Registry,
**not** in the Standard) escapes both limits while keeping each record a legal
80-byte FITS card.

### Form

```
HIERARCH ESO DET CHIP1 NAME = 'CCD-44' / detector chip name
└──────┘ └──────────────────┘ └──────┘
 bytes    hierarchical keyword   value (normal FITS value syntax)
 1–8      (space-separated tokens)
```

- Bytes 1–8 hold the literal keyword name `HIERARCH` (itself a valid 8-char FITS
  keyword), followed by a space.
- The effective keyword is the run of space-separated **tokens** between
  `HIERARCH ` and the `=` value indicator. This admits names longer than 8 chars,
  a namespace hierarchy, and a wider character set (printable ASCII except `=`;
  tokens are conventionally upper-case but mixed case occurs).
- Everything after `=` follows normal FITS value/comment syntax (§4.2).

### Reading / interoperability

- A `HIERARCH`-aware reader detects `HIERARCH` in bytes 1–8 and parses the token
  list as a compound key (commonly normalized by joining tokens with a space or
  `.`, e.g. `ESO DET CHIP1 NAME` or `ESO.DET.CHIP1.NAME`).
- A plain reader sees one keyword named `HIERARCH` whose "value field" is the
  rest of the card — harmless but not useful. There is no `=`/value at the fixed
  columns, so generic value parsing must not assume the §4.1 layout for these.
- `HIERARCH` cards and the `CONTINUE`/long-string convention can interact in
  vendor data; handle `HIERARCH` detection before generic keyword parsing.

---

## Implementation notes (this library)

- **CONTINUE**: model a logical keyword as possibly spanning multiple physical
  records. Parsing returns the reassembled value; writing re-splits on the
  68-char boundary, never breaking an escaped `''` pair across substrings. Keep
  the physical records so round-trips reproduce the original byte layout.
- **CHECKSUM/DATASUM**: implement the 32-bit one's-complement accumulator (with
  end-around carry) over big-endian 32-bit words; it is order-independent within
  a record, so it vectorizes. Offer `verify()` (sum HDU → expect `0xFFFFFFFF`)
  and `update()` (DATASUM first, then CHECKSUM, as the final write step). The
  fixed column placement of `CHECKSUM` must be exact.
- **HIERARCH**: detect `HIERARCH ` in bytes 1–8 and branch to a dedicated parser;
  expose the compound key both as the raw token list and a normalized string.
  Treat it as an optional, feature-flagged reader/writer — it is not part of the
  Standard.
- See the full **Registry of FITS Conventions** for these and others (Green Bank,
  column-oriented, inheritance): <https://fits.gsfc.nasa.gov/fits_registry.html>.
