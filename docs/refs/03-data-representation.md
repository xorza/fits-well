# 3. Data Representation (Standard §5)

All FITS binary data is **big-endian** (most significant byte first): the byte
containing the sign bit comes first, the byte with the ones bit last. Integers
are two's complement; floats are IEEE-754. There is no native unsigned integer
type — unsigned values are encoded via a `BZERO` offset (see §3.5 below). The
`BSCALE`/`BZERO` keywords themselves are defined in the standard's §4.4.2.

## 3.1 BITPIX — array element type (Table 8)

`BITPIX` selects the physical type of every element of an array (primary array,
IMAGE extension, or the data of a binary-table cell after `TFORM` decoding maps
to one of these encodings).

| `BITPIX` | Data represented |
|---------:|------------------|
| `8`   | Character or **unsigned** 8-bit integer |
| `16`  | 16-bit two's-complement signed integer |
| `32`  | 32-bit two's-complement signed integer |
| `64`  | 64-bit two's-complement signed integer |
| `-32` | IEEE-754 single-precision float |
| `-64` | IEEE-754 double-precision float |

Element size in bytes = `|BITPIX| / 8`. Note the asymmetry: `BITPIX = 8` is the
*only* natively unsigned integer; 16/32/64-bit are signed.

## 3.2 Characters (§5.1)

Each character is one byte: its **7-bit ASCII** (ANSI X3.4-1977) code in the
low-order seven bits, with the **high-order bit zero**. The standard has no
wider or multibyte character encoding. This applies to header keyword records,
ASCII-table fields, and the `A`-format character columns of binary tables.

## 3.3 Integers (§5.2)

- Two's complement, big-endian.
- 8-bit is unsigned (0–255); 16/32/64-bit are signed.
- **Unsigned 16/32/64-bit** and **signed 8-bit** are represented with the
  `BZERO`/`TZEROn` offset trick (§3.5 below).

## 3.4 IEEE-754 floating point (§5.3)

- `-32` ⇒ binary32, `-64` ⇒ binary64, big-endian byte order.
- **NaN** represents an undefined/blank float pixel (there is no `BLANK` for
  floats — `BLANK` applies only to integer arrays).
- The full IEEE set of number forms is allowed, including all special values.
  ±Inf and signaling/quiet NaNs are permitted and must be preserved on
  round-trip — do not canonicalize NaN payloads or the quiet/signaling bit.
- Use of `BSCALE`/`BZERO` with float arrays is *not recommended* (§5.3); they
  exist for integer storage. Decoders must still honor them if present.
- Canonical special-value byte patterns are tabulated in the standard's
  Appendix E — e.g. `+∞` is `0x7F800000` (binary32) / `0x7FF0000000000000`
  (binary64); the listed NaN starts at `0x7F800001` / `0x7FF0000000000001`.

## 3.5 Scaling: physical = BZERO + BSCALE × stored

For integer **arrays**, the true physical value is derived from the stored value:

```
physical_value = BZERO + BSCALE × array_value          (Eq. 3)
```

- `BSCALE` default `1.0`, `BZERO` default `0.0`.
- `BLANK` (integer arrays only) gives the *stored* value that denotes an
  undefined pixel; it is interpreted before scaling.

### Unsigned-integer convention

By choosing `BSCALE = 1` and a specific `BZERO`, signed storage represents
unsigned values:

| Stored `BITPIX` | Represents | `BZERO` |
|-----------------|------------|---------|
| `8`  | signed byte (−128…127) | `-128`  = −2⁷ |
| `16` | unsigned 16-bit | `32768` = 2¹⁵ |
| `32` | unsigned 32-bit | `2147483648` = 2³¹ |
| `64` | unsigned 64-bit | `9223372036854775808` = 2⁶³ |

The binary-table analogue uses `TZEROn` with `TSCALn = 1` and integer `TFORM`
codes (`B`, `I`, `J`, `K`) — see Table 19 in [binary tables](06-binary-tables.md).

## 3.6 Time (§5.4)

§5.4 defers to the full time-coordinate framework in
[§9](07-wcs-time-compression.md). Time *values* in headers/columns are ISO-8601
strings or numeric offsets in a stated time scale (`TIMESYS`, e.g. `UTC`, `TT`,
`TAI`, `TDB`).

## Implementation notes (this library)

- Decode is: read big-endian raw element → (optionally subtract `BLANK` sentinel)
  → apply `BZERO + BSCALE×x`. Keep raw and physical paths separate so callers can
  opt out of scaling (zero-copy raw access for the common `BSCALE=1, BZERO=0`).
- Fast path: when `BSCALE == 1.0 && BZERO == 0.0` and host is little-endian, the
  whole decode is a byte-swap (`u16::swap_bytes` / SIMD `bswap`) with no FP math.
- Unsigned detection: `BITPIX` integer + `BZERO == 2^(n-1)` + `BSCALE == 1` ⇒
  expose as `uN`. Encode is the inverse (subtract `BZERO`, store signed).
- Represent blanks: integer `BLANK` → `Option`/mask; float NaN → propagate NaN.
- Big-endian I/O on x86/ARM is a `bswap`; provide a SIMD bulk-swap and let
  big-endian hosts (rare) skip it via `cfg(target_endian)`.
