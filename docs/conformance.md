# FITS Conformance Audit

This document records the result of auditing the `fits` implementation against
the curated reference notes in [`docs/refs/`](refs/). Each section maps one
reference file to the code that implements it, flags conformance gaps (with
severity and `file:line` anchors), and assesses test coverage.

Severity legend: 🔴 correctness bug (rejects valid files or produces wrong
output) · 🟡 lenient/permissive beyond the standard (safe for a reader, but not
strictly conforming) · 🟢 missing nice-to-have / "should" clause · ⚪ deliberately
out of scope — astronomy/metrology computed *on top of* the keywords (frame
rotations, light-travel/ephemeris corrections), beyond the FITS *format* standard
and not a gap to close.

---

## §3 — File Organization (`docs/refs/01-file-structure.md`)

Audited code: `block.rs`, `bitpix.rs`, `hdu/`, `reader/`, `writer/`,
`header/` (rendering).

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 1.1 | 2880 block, 80-byte card, 36/block | `block.rs` constants | ✅ |
| 1.1 | Header unit = whole blocks, space-padded | `render_header` → `pad_to_block(SPACE_FILL)` | ✅ |
| 1.1 | Data unit = whole blocks, NUL-padded | `write_data_unit` / `write_hdu` w/ `ZERO_FILL` | ✅ |
| 1.1 | ASCII-table data padded with space | `write_ascii_table` passes `SPACE_FILL` | ✅ code / ⚠️ untested |
| 1.2 | Primary first, extensions follow, empty data unit | `HduKind`, `classify` | ✅ |
| 1.3 | Primary mandatory order on write | `primary_image_header` | ✅ |
| 1.3 | `SIMPLE=F` still readable | `classify` ignores SIMPLE value → Primary | ✅ untested |
| 1.3 | 1–999 axes | `axes()` reads `NAXIS1..n` | ⚠️ 999 upper bound not enforced |
| 1.3 | `EXTEND` advisory (read ignores it) | reader scans regardless | ✅ |
| 1.4 | XTENSION IMAGE/TABLE/BINTABLE | `classify` | ✅ |
| 1.4 | XTENSION space-padded to 8 | `pad_string` in `format_value` | ✅ |
| 1.4 | Mandatory ext. keyword **order** on write | writer emits in order | ✅ write / ❌ not validated on read |
| 1.4 | PCOUNT/GCOUNT semantics + any-order extensions | `data_extent`, reader loop | ✅ |
| 1.5 | Special records (§3.5) | `NextHeader::Trailing` → scan stops | ✅ disregarded |
| 1.6 | Trailing partial / zero-fill block (§3.6) | `fill_block` → `Partial`/`Eof` → `Trailing` | ✅ disregarded |
| 1.6 | Eq 1 / Eq 2 / Eq 4 sizing; `ceil(Nbits/8/2880)` | `data_extent`, `padded_len` | ✅ |
| 1.6 | Nbits non-negative; overflow-safe | checked arithmetic + PCOUNT/GCOUNT guards | ✅ |
| 1.7 | "Once FITS always FITS" (random groups) | `read_groups`, `classify` | ✅ |

### Gaps

1. ✅ **FIXED — special records / trailing blocks are now disregarded (§3.5–3.6).**
   `read_header_unit` returns a `NextHeader::Trailing` outcome for any post-HDU
   content carrying no `END` — special records, a trailing all-zero fill block, or
   a sub-2880 partial remnant — and `open()` stops the scan there instead of
   erroring. The same shape *before* any valid HDU is still rejected (no conforming
   primary). Covered by `trailing_special_records_and_partial_blocks_are_ignored`
   and `content_before_any_valid_hdu_is_rejected`.

2. 🟡 **Mandatory extension keyword order not validated on read (§3.4.1,
   Table 10).** The reader fetches `XTENSION`/`BITPIX`/`NAXIS`/`PCOUNT`/`GCOUNT`
   by name regardless of position and never checks that nothing intervenes
   between `XTENSION` and `GCOUNT`. This is the lenient/Postel choice and is
   arguably correct for a reader, but the library does not enforce a rule the
   doc marks mandatory. The writer *does* emit them in order.

3. 🟡 **999-axis upper bound not enforced (§1.3).** `axes()` accepts any
   `NAXIS`. Reading still works; just no rejection or boundary test.

### Test coverage

Well covered: block-rounding math (`block.rs` tests), `BITPIX`
codes/sizes/round-trip/rejection (`bitpix.rs` tests), all six `HduKind`s,
Eq 1/2/4 sizing with hand-computed sizes, the random-groups `NAXIS1` skip,
axis-product overflow + malformed `PCOUNT`/`GCOUNT` guards (`hdu/tests.rs`),
real-file boundary scans ending exactly at EOF, padded-bytes + data-range +
NUL-fill on read (`reader/tests.rs`), header render block-alignment + `END` +
space pad, and write→read round-trips (`writer/tests.rs`).

Coverage gaps (functionality present, assertion missing):

- No test that the ASCII-table data fill byte is `0x20` — the one distinguishing
  padding rule of §3.1 is implemented but never asserted on bytes.
- No write-side assertion that non-ASCII data padding is NUL (only `pad_to_block`
  in isolation + a read-side NUL check on a real file).
- No `SIMPLE=F` read test; no special-records / trailing-zero-block test
  (gap #1); no 999-axis boundary test.

---

## §4 — Headers & Keyword Records (`docs/refs/02-headers-keywords.md`)

Audited code: `header/card/mod.rs` (parse + render), `header/value.rs`
(typed `Value`), `header/mod.rs` (ordered model + index + builder).

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 2.1 | Keyword = bytes 1–8, left-justified, space-trimmed | `Card::parse` (`card/mod.rs:74`) | ✅ |
| 2.1 | Keyword chars `A–Z 0–9 - _`, uppercase only | `validate_keyword` (`card/mod.rs:346`) | ✅ (value cards) |
| 2.1 | Value indicator = `"= "` in bytes 9–10 | only checks byte 9 `=` (`card/mod.rs:125`) | 🟡 byte-10 space unchecked |
| 2.1 | Free text only for commentary (no indicator) | COMMENT/HISTORY/blank handled first | ✅ |
| 2.1 | Comment = after first `/` outside a string | `split_value_comment` tracks quote state | ✅ |
| 2.1 | Header restricted to ASCII 32–126 | `!raw.is_ascii()` rejects ≥128 only | 🟡 admits ctrl 0–31, DEL 127 |
| 2.1 | Mandatory keywords fixed-format on write; reader accepts free-format | `render` fixed; `parse_value` position-agnostic | ✅ |
| 2.2 | String: `''` escape, leading sig., trailing not | `parse_string` (`card/mod.rs:284`) | ✅ |
| 2.2 | Logical `T`/`F`; integer; complex int/real | `parse_value` / `parse_complex` | ✅ |
| 2.2 | Real, exponent `E`/`D` **upper-case** (§4.2.4) | reader accepts lower `e`/`d` (`card/mod.rs:338`) | 🟡 lenient on read |
| 2.2 | Date = ISO-8601 string | stored as `Value::Text` (time layer parses) | ✅ |
| 2.2 | **Null vs empty string distinct** (§4.2.1.1) | `parse_string` keeps one space for all-blank | ✅ |
| 2.2 | Undefined = blank value field, no quotes | `Value::Undefined` (`card/mod.rs:267`) | ✅ |
| 2.2 | ≤68 chars/record; longer ⇒ CONTINUE | `render_records` / `render_long_string` | ✅ |
| 2.2 | XTENSION padded to 8; no other min length | `pad_string` (`card/mod.rs:470`) | ✅ |
| 2.2 | Numbers fit field; no thousands separators | parse rejects separators; `format_real` uses `E` form for large reals | ✅ |
| 2.3 | Units = opaque comment text | comments stored verbatim | ✅ |
| 2.3 | Expose helper to parse `[...]` unit prefix | — | 🟢 not implemented |
| 2.4 | Primary mandatory keywords + presence errors | `bitpix()`/`naxis()`/`axes()` → `MissingKeyword` | ✅ |
| 2.4 | `NAXIS ≥ 0`, `NAXISn ≥ 0` | `usize::try_from` rejects negatives | ✅ |
| 2.4 | `END` last, no value/comment | `CardKind::End`, render `"END"`+spaces | ✅ |
| 2.5 | Reserved keywords used as defined *if present* | generic at header layer; semantics in data/WCS/table layers | ✅ (scope) |
| 2.6 | Commentary repeats, order significant, dups kept | ordered `Vec`, commentary not indexed | ✅ |
| — | Ordered model + side index, first-wins lookup | `Header` (`header/mod.rs:25`) | ✅ |
| — | Scan `END` at 80-byte strides | `block_has_end` (`reader/mod.rs:278`) | ✅ |

### Gaps

1. ✅ **FIXED — null string and empty (all-blank) string are now distinct
   (§4.2.1.1).** `parse_string` preserves one significant space when a string is
   non-empty but all-blank, so `''` → length 0 and `'   '` → length 1, comparing
   unequal. The previously-wrong test was corrected to assert this.

2. 🟡 **Restricted-ASCII range not enforced (§4.1).** Headers are limited to
   decimal 32–126, but `Card::parse` only rejects bytes ≥ 128 via
   `!raw.is_ascii()` (`card/mod.rs:70`). Control characters 0–31 (tab, NUL, …)
   and DEL (127) pass through into value/comment text. Lenient; a strict reader
   would reject them.

3. 🟡 **Value indicator only checks column 9.** The standard's indicator is the
   two bytes `"= "` (cols 9–10); the code keys solely on `raw[8] == b'='`
   (`card/mod.rs:125`) and ignores column 10. Safe in practice (commentary
   keywords are matched first) but not a strict `"= "` check.

4. ✅ **FIXED — large-magnitude reals no longer overflow the value field on
   write.** `format_real` now falls back to the §4.2.4 uppercase-`E` exponent form
   when the plain `Display` decimal would grow long (e.g. `1e300` → `1E300`), so
   the value always fits and round-trips. Covered by
   `large_magnitude_real_renders_with_exponent_and_round_trips`.

5. 🟢 **No `[...]` unit-prefix helper (§4.3).** The doc says the library should
   expose a helper to parse the bracketed unit prefix of a comment; comments are
   stored opaquely and no such helper exists.

### Test coverage

Well covered (`card/tests.rs`, `value.rs` tests, `header/tests.rs`):
logical/integer/real + Fortran `D`/`E` exponent parsing; string unescaping and
trailing-space trim; slash-inside-string; undefined (blank) value; complex
int/real; `END`/commentary/blank-keyword classification; commentary text
starting with `=` not misread; non-ASCII rejection (incl. multibyte straddling
the keyword boundary); lowercase-keyword rejection; HIERARCH parse + render
round-trip; CONTINUE parse, long-string split chain, and reassembly; orphan
CONTINUE demotion; trailing-`&`-without-CONTINUE literal; missing `END`; missing
mandatory keyword; builder set/replace/index and commentary append; render→parse
round-trips; `Value` accessor/`From` behavior.

Coverage gaps:

- **Null vs empty string** — not only untested, the existing test
  (`card/tests.rs:71`) locks in the conflated behavior (gap #1). Need a test
  asserting `''` → len 0, `'   '` → len 1, and the two compare unequal.
- No byte-position assertions for fixed-format rendering (logical/integer/real
  right-justified ending at column 30; string opening quote at column 11) — only
  model-level round-trips exist.
- No test for lowercase-exponent acceptance on read, nor for large-magnitude
  real rendering / field overflow (gap #4).
- No test for control-character rejection (restricted ASCII 32–126, gap #2).
- No `[...]` unit-prefix test (feature absent, gap #5).

---

## §5 — Data Representation (`docs/refs/03-data-representation.md`)

Audited code: `bitpix.rs`, `endian.rs`, `data/` (`Image`/`ImageData`/`Scaling`,
decode/encode, physical plane), with `ascii/` and `table/` for character data.

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 3.1 | All six `BITPIX` types + `|BITPIX|/8` size | `Bitpix` (`bitpix.rs`) | ✅ |
| 3.2 | Characters = 7-bit ASCII, high bit zero | header rejects ≥128; ASCII/`A`-cols lenient | 🟡 not enforced in table/ascii |
| 3.3 | Integers two's-complement, big-endian | `decode_be` (`endian.rs:7`) | ✅ |
| 3.3 | 8-bit unsigned; 16/32/64 signed | `Bitpix` → `ImageData` mapping | ✅ |
| 3.3 | Unsigned 16/32/64 + signed-8 via `BZERO`/`TZEROn` | `physical()` plane + typed `Image::unsigned()` → `UnsignedView` | ✅ |
| 3.4 | `-32`/`-64` IEEE-754, big-endian | `f32`/`f64::from_be_bytes` (`data/mod.rs:69`) | ✅ |
| 3.4 | NaN = blank float; no float `BLANK` | `scale_ints` for ints only; float NaN propagates | ✅ |
| 3.4 | Preserve ±Inf + signaling/quiet NaN payload on round-trip | `to_bits`/`from_bits` are bit-exact | ✅ code / ⚠️ untested |
| 3.4 | Honor `BSCALE`/`BZERO` on floats if present | `physical()` scales floats (`data/mod.rs:116`) | ✅ |
| 3.5 | `physical = BZERO + BSCALE × stored` (Eq. 3) | `scale` closure (`data/mod.rs:110`) | ✅ |
| 3.5 | Defaults `BSCALE=1.0`, `BZERO=0.0` | `from_header` `unwrap_or` (`data/mod.rs:150`) | ✅ |
| 3.5 | `BLANK` integer-only, applied *before* scaling | `scale_ints` sentinel→NaN pre-scale (`data/mod.rs:124`) | ✅ |
| 3.5 | Unsigned convention table (8/16/32/64) | `physical()` + exact typed `Image::unsigned()` | ✅ |
| 3.5 | `TZEROn`/`TSCALn` binary-table analogue | `table/` layer | ✅ (audited under §6) |
| 3.6 | Time defers to §9 | `time/` feature | ✅ (audited under §9) |
| impl | Zero-copy raw + SIMD bulk byte-swap | `decode` always allocates + converts | 🟢 TODO (perf) |
| impl | Detect + expose as `uN` | `Image::unsigned()` → `UnsignedView::{I8,U16,U32,U64}` | ✅ |
| impl | `BLANK` → `Option`/mask | NaN in physical plane | 🟢 by design |

The normative core of §5 (BITPIX types, big-endian two's-complement integers,
IEEE floats, Eq. 3 scaling with defaults, `BLANK`-before-scaling, the unsigned
`BZERO` convention) is correctly implemented. The gaps are design-principle and
edge-precision items, not wrong decoding.

### Gaps

1. ✅ **FIXED — native unsigned (`uN`) typed exposure.** `Image::unsigned()`
   returns a typed `UnsignedView` (`U16`/`U32`/`U64`/signed-byte `I8`) when the
   scaling is exactly the FITS unsigned convention (`BSCALE == 1`, no `BLANK`,
   `BZERO == 2^(n-1)`), recovering exact values by flipping the stored sign bit.
   The `Image::from_u16`/`from_u32`/`from_u64`/`from_i8` constructors are the encode
   side (signed storage + the offset `BZERO`, which the writer emits). Covered by
   `unsigned_views_round_trip_every_width_exactly` and the write→read round-trip.

2. ✅ **FIXED — `u64`/large-`i64` precision via the typed path.** `Image::unsigned()`
   yields exact integers even past 2⁵³, where the `f64` `physical()` plane rounds.
   Covered by `unsigned_u64_view_is_exact_where_physical_rounds`. (`physical()`
   itself is unchanged — `f64` by definition for the general scaled plane.)

3. 🟡 **§5.1 7-bit/high-bit-zero not enforced for character data.** The header
   parser rejects bytes ≥ 128 (but admits control 0–31, see §4 gap #2). ASCII
   tables decode fields with `from_utf8(...).unwrap_or("")` (`ascii/mod.rs:122`)
   — a high byte silently blanks the whole field — and binary-table `A`-columns
   use `from_utf8_lossy` (`table/mod.rs:440`), turning a high byte into U+FFFD.
   Neither enforces the 7-bit rule.

4. 🟢 **Zero-copy raw / SIMD bulk swap not implemented.** `ImageData::decode`
   always allocates a `Vec` and converts element-by-element via `decode_be`;
   there is no zero-copy typed-slice view for the `BSCALE=1, BZERO=0`,
   host-endian-matches case, and no SIMD bulk byte-swap. The module doc marks
   this TODO; it is a performance-principle gap, not a correctness one.

### Test coverage

Well covered (`data/tests.rs`, `endian.rs` tests): big-endian decode of all six
types from exact byte patterns; big-endian encode; encode↔decode inverse over a
table including `i32::MIN`/`i64::MAX`/`f64::MAX`; Eq. 3 scaling with
`BSCALE=2, BZERO=5` hand-computed plus `BLANK`→NaN; the unsigned-`u16` `BZERO`
offset hand-computed (`-32768→0, 0→32768, 32767→65535`); float scaling with NaN
pass-through; `bitpix()` reporting; and `Scaling::from_header` defaults / explicit
keywords / unsigned-not-identity.

Coverage gaps:

- ✅ **FIXED — NaN/Inf bit-for-bit round-trip is now tested.**
  `float_inf_and_nan_payloads_round_trip_bit_for_bit` round-trips ±Inf and
  signaling/quiet NaN payloads (the Appendix-E patterns) for both `f32` and `f64`,
  asserting `to_bits()` is identical — confirming `to_bits`/`from_bits` never
  canonicalize.
- Only the `u16` unsigned convention is tested; `u32` (`BZERO=2³¹`), `u64`
  (`BZERO=2⁶³`, which exposes gap #2), and signed-8 (`BZERO=-128`) are untested.
- `Inf` propagation through non-identity scaling is untested (only NaN is).
- No test for §5.1 high-bit-zero handling of ASCII-table / `A`-column character
  data (gap #3).

---

## §6 — Images: Primary Array & IMAGE Extension (`docs/refs/04-images.md`)

Audited code: `data/` (`Image`/`ImageData`), `reader/` (`read_image`),
`writer/` (`write_image`, `primary_image_header`, `image_extension_header`),
`groups/` (random groups), `hdu/` (classification).

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 4.1 | N-d array, flat, Fortran order (NAXIS1 fastest) | flat `samples` + `shape`, sequential decode | ✅ storage / 🟢 no indexing API |
| 4.1 | Index mapping `Σ idx_k · Π_{j<k} NAXISj` | — | 🟢 not exposed |
| 4.1 | Element type `BITPIX`; physical via `BZERO`/`BSCALE` | `ImageData` + `physical()` | ✅ |
| 4.2 | Primary declared by `SIMPLE`/`BITPIX`/`NAXIS`/`NAXISn` | `classify`, `read_image` | ✅ |
| 4.2 | `NAXIS = 0` ⇒ dataless primary | handled (read + write) | ✅ |
| 4.2 | No random groups + normal array at once | `GROUPS=T` wins in `classify` | ✅ |
| 4.3 | IMAGE = same data model in an extension | `read_image` accepts `Image` kind | ✅ |
| 4.3 | Mandatory keys in order; `PCOUNT=0`, `GCOUNT=1` | `image_extension_header` | ✅ write / ⚠️ values unasserted on read |
| 4.3 | IMAGE with `PCOUNT≠0`/`GCOUNT≠1` is malformed | `read_image` rejects up front (`WrongValueType`) | ✅ clean error |
| 4.3 | Reserved: `BUNIT`/`DATAMIN`/`DATAMAX`/`EXTNAME`/`EXTVER`/`EXTLEVEL` | readable generically; no typed accessors | 🟢 |
| 4.4 | Random groups: `NAXIS1=0`, `GROUPS=T`, `PCOUNT`/`GCOUNT` | `classify` → `RandomGroups`, `from_data` | ✅ |
| 4.4 | Group = `PCOUNT` params then `NAXIS2…m` array | `group_len`, base offsets (`groups/mod.rs:99`) | ✅ |
| 4.4 | Param scaling `PSCALn`/`PZEROn`, names `PTYPEn` | `param_scaling`, `parameter_names` | ✅ |
| 4.4 | Array scaling `BZERO`/`BSCALE` | `array_scaling`, `array_physical` | ✅ |
| 4.4 | Must read, must **not** write | read path only; no groups writer | ✅ |
| impl | Strided / ndarray-style views | — | 🟢 not implemented |
| impl | Zero-copy no-scaling matching-endian; SIMD/parallel | `decode` always allocates + converts | 🟢 TODO (shared w/ §5) |

The image data model and random-groups read are correct, and random groups are
validated against a real `uv` file. Gaps are API/robustness items, not wrong
decoding.

### Gaps

1. ✅ **FIXED — a malformed IMAGE now errors instead of panicking.** `read_image`
   rejects an image HDU with `PCOUNT ≠ 0` / `GCOUNT ≠ 1` up front
   (`WrongValueType`), and its sample-count check is now a `DataSizeMismatch`
   error rather than an `assert_eq!`. `RandomGroups::from_data`'s closing assert is
   likewise a `DataSizeMismatch` error. The `write_image` assert (a logic-error
   guard on a caller-built `Image`) is intentionally left. Covered by
   `malformed_image_pcount_is_rejected_not_panicked`.

2. 🟢 **No coordinate-indexing / strided-view API (§4.1).** `Image` stores the
   flat buffer (correctly in Fortran order) and the `shape`, but exposes no
   `at(coords)` accessor or strided/ndarray view, so the documented index mapping
   is left entirely to the caller. The impl-notes call for strided views
   (`stride[0] = 1`); not implemented.

3. 🟢 **Reserved image keywords have no typed accessors (§7.1.2).** `BUNIT`,
   `DATAMIN`, `DATAMAX`, `EXTNAME`, `EXTVER`, `EXTLEVEL` are readable only as raw
   header cards; `Scaling` covers `BSCALE`/`BZERO`/`BLANK` and the `wcs` layer
   covers WCS, but the others have no typed surface. Optional, but listed by the
   doc.

4. 🟢 **Zero-copy / SIMD / parallel decode not implemented** — same underlying
   gap as §5: `ImageData::decode` always allocates and converts, with no
   zero-copy typed-slice path for the identity-scaling host-endian case and no
   tiled parallel swap+scale.

### Test coverage

Well covered: `read_image` shape/type/length and physical length
(`reader/tests.rs`); raw samples cross-checked against a manual big-endian decode;
non-image HDUs rejected; multi-HDU image write→read; full image round-trip;
unsigned-scaling keyword emission + value preservation (`writer/tests.rs`);
dataless-primary (`NAXIS=0`) read and write. Random groups: the real
`DDTSUVDATA.fits` `uv` file decodes with the expected `GCOUNT`/`PCOUNT`/
`group_shape`/`array_len`/`BITPIX`/`PTYPEn`, and non-groups HDUs are rejected
(`groups/tests.rs`).

Coverage gaps:

- No **≥3-D image** test — all image fixtures are 1-D/2-D, so multi-axis decode
  (a data cube) and the higher-order index mapping are unexercised (decode is
  dimension-agnostic, but the Fortran-order flattening is never asserted for
  `NAXIS ≥ 3`).
- The written `IMAGE` extension's `PCOUNT = 0` / `GCOUNT = 1` values are never
  asserted on read (only `kind == Image`).
- No test for the malformed-IMAGE case (gap #1) — currently it would panic.
- Random-groups physical values are checked with a **range** assertion
  (`params[4] > 2_445_728.0 && < 2_445_730.0`, `groups/tests.rs:27`) rather than a
  hand-computed exact value or astropy cross-check, which the project's test rules
  discourage.

---

## §7.2 — ASCII Table Extension (`docs/refs/05-ascii-tables.md`)

Audited code: `ascii/` (`AsciiTable`/`AsciiColumn`/`parse_ascii_tform`,
`read_column`) and the writer's ASCII path (`write_ascii_table`,
`ascii_table_header`, `ascii_tform`, `format_ascii_field` in `writer/mod.rs`).

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 5.1 | `NAXIS2` rows of `NAXIS1` bytes, `BITPIX=8`, `NAXIS=2` | read uses `NAXIS1`/`NAXIS2`; write sets all | ✅ |
| 5.1 | Column `n` at 1-based `TBCOLn`, per `TFORMn` | `start = TBCOLn − 1` (`ascii/mod.rs:80`) | ✅ |
| 5.1 | Fields may overlap; gap bytes any 7-bit ASCII; CR/LF allowed | reads only each field slice; tolerant of gaps | ✅ |
| 5.1 | Data unit padded with **spaces** | `write_ascii_table` → `SPACE_FILL` | ✅ code / ⚠️ untested |
| 5.1 | Blank numeric field reads as **0** (§7.2.5) | `if s.is_empty() { 0 }` (`ascii/mod.rs:132,146`) | ✅ untested |
| 5.1 | Field matching `TNULLn` is **undefined** | `AsciiColumn.null`; raw → 0, physical → `NaN` | ✅ |
| 5.2 | Mandatory keys present + in order | read requires `NAXIS1/2`,`TFIELDS`,`TBCOLn`,`TFORMn`; write emits in order | ✅ |
| 5.2 | `TFIELDS` 0…999 | no upper-bound check | 🟢 (as §3 999) |
| 5.3 | `Aw`/`Iw`/`Fw.d`/`Ew.d`/`Dw.d`, upper-case only | `parse_ascii_tform` matches `A/I/F/E/D` only | ✅ |
| 5.3 | Scalar cells, no repeat/arrays | no repeat parsing | ✅ |
| 5.3 | `F`/`E`/`D` parse identically; base-ten; sign+exp | all → `Float`, `f64` parse, `D`→`E` | ✅ |
| 5.3 | Implicit decimal point (deprecated) | `parse_ascii_float` applies ×10⁻ᵈ | ✅ |
| 5.4 | `TTYPEn` name, compared **case-insensitively** | `column_index` uses `eq_ignore_ascii_case` | ✅ |
| 5.4 | `TUNITn` units | read into `unit` | ✅ |
| 5.4 | `TSCALn`/`TZEROn` scaling (not on `A`) | `read_column_physical` applies `TZERO + TSCAL·field` | ✅ |
| 5.4 | `TNULLn` (string) undefined marker | `AsciiColumn.null`; physical → `NaN` | ✅ |
| 5.4 | `TDISPn`, `TDMINn`/`TDMAXn`, `TLMINn`/`TLMAXn` | not implemented | 🟢 |
| impl | Right-justify numerics, left-justify strings, gap-fill spaces | `format_ascii_field` | ✅ |
| impl | Overflow handling | `*`-fill per §7.2.5 (`writer/mod.rs:656`) | ✅ |
| impl | Float-precision lint on write | — | 🟢 |

`TFORMn` parsing, field slicing, and the write→read round-trip are correct. The
substantive gaps are the three ASCII-table semantics the standard attaches to
columns — `TNULLn`, `TSCALn`/`TZEROn`, and implicit decimal points — none of which
are implemented.

### Gaps

1. ✅ **FIXED — `TNULLn` undefined values handled (§7.2.5).** `AsciiColumn` now
   reads `TNULLn`; a field equal to the marker is a 0 placeholder in the raw
   `read_column` plane and `NaN` in `read_column_physical`, so a table whose null
   marker is `'NULL'`/`'***'` no longer fails to read.

2. ✅ **FIXED — `TSCALn`/`TZEROn` scaling applied to ASCII columns (§7.2.2).** New
   `AsciiTable::read_column_physical` computes `TZEROn + TSCALn × field` (mirroring
   binary tables), mapping blanks to 0 and `TNULLn` to `NaN`.

3. ✅ **FIXED — implicit decimal point handled (§7.2.1).** `parse_ascii_float`
   applies the implied point `d` digits from the right (×10⁻ᵈ) when an
   `Fw.d`/`Ew.d`/`Dw.d` field carries no explicit `.`.

4. ✅ **FIXED — `column_index` is now case-insensitive (§7.2.2).** Matching uses
   `eq_ignore_ascii_case`, so `column_index("ra")` finds a `TTYPE='RA'` column.

5. 🟢 **No typed accessors** for `TDISPn`, `TDMINn`/`TDMAXn`, `TLMINn`/`TLMAXn`,
   `EXTNAME`/`EXTVER`/`EXTLEVEL`, `AUTHOR`, `REFERENC` — readable only as raw
   cards.

6. Note: `A`-format fields are trimmed on **both** ends (`field(r)` →
   `.trim()`, `ascii/mod.rs:122`), so a character value with significant leading
   spaces loses them; and a high byte blanks the whole field
   (`from_utf8().unwrap_or("")`, shared with §5 gap #3).

### Test coverage

Well covered (`ascii/tests.rs`): `TFORMn` parsing for `A8`/`I10`/`F8.2`/`E15.7`/
`D25.17` with a `Z3` rejection; a hand-built two-column row decoded to exact
values (`["abc","def"]`, `[123,-45]`) including trailing-space trim; and a full
`write_ascii_table` → `read_ascii_table` round-trip over `Text`/`I64`/`F64`.

Coverage gaps:

- No blank-numeric-field → 0 test (§7.2.5), though the code handles it.
- No `TNULLn`, `TSCALn`/`TZEROn`, or implicit-decimal-point tests (all unimplemented — gaps #1–#3).
- No case-insensitive `column_index` test (would currently fail — gap #4).
- No write-side test of overflow `*`-fill, of the space pad byte, or of
  gap/overlapping fields / trailing CR-LF tolerance on read.
- No lowercase-`TFORM` rejection test (the match is upper-case-only, so it works,
  but it is unverified).

---

## §7.3 — Binary Table Extension (`docs/refs/06-binary-tables.md`)

Audited code: `table/` (`Tform`/`TformKind`/`Column`/`BinTable`, `read_column`,
`read_column_physical`, `read_vla_column`, `decode_array`) and the writer's
binary-table path (`bintable_header`, `column_code`, `check_column`, `pack_rows`,
`pack_cell` in `writer/mod.rs`).

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 6.1 | `NAXIS2` rows × `NAXIS1` bytes, `BITPIX=8`, `NAXIS=2` | read uses `NAXIS1/2`; write sets all | ✅ |
| 6.1 | `NAXIS1 = Σ rₙ·bₙ` (row width) | offset accumulation + `RowWidthMismatch` (`table/mod.rs:243`) | ✅ |
| 6.1 | Heap after main table, `THEAP` offset; `PCOUNT`=gap+heap | `heap_offset` (`table/mod.rs:253`); `data_extent` PCOUNT | ✅ read |
| 6.2 | Mandatory keys present + in order | read requires `NAXIS1/2`,`TFIELDS`,`TFORMn`; write emits in order | ✅ |
| 6.3 | `rTa`: repeat (≥0, default 1), type code, trailing | `Tform::parse` (`table/mod.rs:113`) | ✅ |
| 6.3 | All 13 codes `LXBIJKAEDCMPQ` + byte sizes | `TformKind`, `elem_size`, `byte_width` | ✅ |
| 6.3 | `r=0` empty cell; repeat element-wise | `byte_width` 0; flat decode | ✅ |
| 6.3 | `rA` = one string; early `NUL` terminates | `trim_text` truncates at first NUL | ✅ |
| 6.3 | `P`/`Q` repeat only 0 or 1 | not validated | 🟢 |
| 6.4 | `physical = TZEROn + TSCALn × stored` (Eq. 7) | `read_column_physical` (`table/mod.rs:314`) | ✅ |
| 6.4 | Not applied to `A`/`L`/`X` | `_ ⇒ NonNumericColumn` (also rejects `C`/`M`) | ✅ (C/M over-rejected) |
| 6.4 | Unsigned `B`/`I`/`J`/`K` via `TZEROn` | `physical()` plane + exact typed `read_column_unsigned()` | ✅ |
| 6.4 | `TNULLn` matched on **stored** value before Eq. 7 | `scaled_int` checks `tnull` pre-scale (`table/mod.rs:318`) | ✅ |
| 6.4 | Scaling on `P`/`Q` heap values, not descriptor | `read_vla_column_physical` scales heap elements | ✅ |
| 6.5 | `TDIMn` multidimensional cell reshape | `Column.tdim` parsed; written from `WriteColumn::dims` | ✅ shape exposed |
| 6.6 | `P`/`Q` descriptor `(nelem, offset)`, signed; heap decode | `read_vla_column` (`table/mod.rs:345`) | ✅ |
| 6.6 | Default `THEAP` = main-table size; gap allowed | `heap_offset` default | ✅ (min not validated) |
| 6.6 | `nelem=0` ⇒ no heap data | empty slice | ✅ (garbage offset may error) |
| 6.6 | Span must lie within **heap** (not data unit) | bounds-checked vs `heap_end` (`nrows·row_len + PCOUNT`) | ✅ |
| 6.7 | `TTYPEn` name, compared case-insensitively | `column_index` uses `eq_ignore_ascii_case` | ✅ |
| 6.7 | `TUNITn`, `TSCALn`, `TZEROn`, `TNULLn`, `THEAP` | parsed | ✅ |
| 6.7 | `TDISPn`, `TDIMn`, `TDMINn`/`TDMAXn`, `TLMINn`/`TLMAXn` | not implemented | 🟡 `TDIM` / 🟢 rest |
| impl | `X` bit columns unpacked MSB-first | `read_bit_column` → `Vec<bool>`/row; `read_column` keeps packed bytes | ✅ |
| impl | Column-oriented / SIMD / zero-copy fast path | `read_column` copies via `flatten` | 🟢 perf |

Fixed-width decoding (all 13 type codes, repeat/byte-width including `X` =
⌈bits/8⌉ and the `P`/`Q` descriptor sizes), row-width validation, the
`TSCAL`/`TZERO`/`TNULL` physical plane (null matched pre-scale, `A`/`L`/`X`
rejected), and `P`/`Q` heap decode are all implemented and tested — including
against a real AIPS antenna table. The gaps cluster around column-level features
beyond plain fixed-width decode.

### Gaps

1. ✅ **`TDIMn` multidimensional cells read + write (§6.5).** `Column.tdim` parses
   `TDIMn` into a shape (`parse_tdim`) and the writer emits it from
   `WriteColumn::dims`. `read_column` returns the flat row-major buffer with the
   per-cell shape on `Column.tdim` for reshaping. (Added earlier; the prior gap
   entry was stale.)

2. ✅ **FIXED — VLA heap bounds now checked against the heap (§6.6).** `BinTable`
   carries `heap_end = nrows·row_len + PCOUNT`, and `read_vla_column` rejects any
   span past it, so trailing block fill is never decoded as array elements.
   Covered by `vla_descriptor_overrunning_the_heap_is_rejected`.

3. ✅ **`X` (bit) columns round-trip (§6.3).** The writer emits `<nbits>X` from
   packed bytes (`WriteColumn::bits`), and `read_bit_column` unpacks each row's
   bits MSB-first into `Vec<bool>`; `read_column` still returns the lossless packed
   bytes. Covered by `x_bit_column_unpacks_msb_first` and
   `writes_tdim_q_vla_and_bit_columns`. (Write added earlier; read-unpacking is new.)

4. ✅ **FIXED — VLA columns have a scaling/null/physical path (§6.4).**
   `read_vla_column_physical` applies `TZEROn + TSCALn × element` to each row's heap
   array, mapping integers equal to `TNULLn` to `NaN` (shared with the fixed-width
   `column_data_physical` helper). Covered by
   `read_vla_column_physical_scales_heap_arrays_and_nulls`.

5. ✅ **FIXED — `rA` early-NUL termination honored (§6.3).** `trim_text` truncates
   at the first NUL before stripping trailing spaces, so `AB\0CD` → `"AB"`. Covered
   by `a_column_terminates_at_the_first_nul`.

6. ✅ **FIXED — `column_index` now case-insensitive (§6.7),** via
   `eq_ignore_ascii_case`. Covered by `column_index_is_case_insensitive`.

7. ✅ **FIXED — native unsigned (`uN`) exposure for table columns.**
   `BinTable::read_column_unsigned` returns a typed `UnsignedView`
   (`U16`/`U32`/`U64`/signed-byte `I8`) when a `B`/`I`/`J`/`K` column uses exactly
   the convention (`TSCALn == 1`, no `TNULLn`, `TZEROn = 2^(n-1)`), exact past 2⁵³
   where `read_column_physical` rounds. Reuses the image `UnsignedView`. Covered by
   `read_column_unsigned_recovers_typed_values` and
   `read_column_unsigned_is_exact_for_u64_and_none_otherwise`.

8. ✅ **`Q` (64-bit) VLA write supported (§6.6).** `WriteColumn::q()` emits `1Q`
   descriptors for heaps beyond the 32-bit `1P` range; `1P` remains the default.
   (Added earlier; the prior gap entry was stale.)

9. 🟢 **Minor/unvalidated:** `P`/`Q` repeat not restricted to {0,1}; `THEAP`
   minimum (≥ main-table size) not enforced; `C`/`M` complex columns are rejected
   from `read_column_physical` (complex scaling unsupported, and `Vec<f64>` could
   not hold it anyway); a `nelem=0` descriptor with a garbage offset beyond the
   buffer raises `UnexpectedEof` instead of yielding empty; and the writer has no
   `TSCAL`/`TZERO`/`TNULL`/`X` write path.

10. 🟢 **No typed accessors** for `TDISPn`, `TDMINn`/`TDMAXn`, `TLMINn`/`TLMAXn`,
   `EXTNAME`/`EXTVER`/`EXTLEVEL`, `AUTHOR`, `REFERENC`; and no column-oriented /
   SIMD / zero-copy fast path (`read_column` strides and copies via `flatten`).

### Test coverage

Well covered (`table/tests.rs`): `TFORM` parsing (`8A`/`3D`/`0D`/`1J`/`E`/`16X`/
`1PE(5)`/`1QD`, with `9Z`/`""`/`1P` rejected); `byte_width` for arrays, bits
(`16X`,`9X`), and `P`/`Q` descriptors; hand-built fixed-width decode of `I32`/`F32`/
`A` with verified byte offsets and trailing-space trim; `0`-repeat → empty;
`TSCAL`/`TZERO`/`TNULL` physical hand-computed (`16`/NaN/`24`); non-numeric
rejection; `read_column` on a VLA → error and vice-versa; a hand-built `PE` heap
decode over two unequal-length rows; row-width mismatch; out-of-bounds column;
and the real `DDTSUVDATA.fits` AIPS antenna table (12 columns, byte offsets, the
`0D` zero-width `ORBPARM` sharing `NOSTA`'s offset, units).

Coverage gaps:

- No test of the **unsigned `TZEROn` convention** for tables (`B`/`I`/`J`/`K`
  with `TZERO=-128`/`32768`/`2³¹`/`2⁶³`) — only the generic `TSCAL=2,TZERO=10` case.
- No `X` bit-column decode test (current raw-`Bytes` behavior is unverified), no
  `Logical`(`L`) and no plain `Byte`(`B`) decode test, and no complex `C`/`M`
  decode test (those `decode_array` arms are unexercised).
- No `Q` (64-bit descriptor) **heap** decode test (only parsed, not decoded from a
  heap), no VLA bounds-overrun rejection test, no `nelem=0` VLA test, and no
  `THEAP`-with-gap test.
- No `rA` early-NUL test (gap #5), no `TDIM` test (unimplemented, gap #1), and no
  case-insensitive `column_index` test (would fail, gap #6).

---

## §8 — World Coordinate Systems (`docs/refs/07-wcs-time-compression.md` §7.1)

Audited code: `wcs/mod.rs` (`Wcs`, `Projection`, the pixel↔world pipeline,
`compute_pole`, matrix inversion). (Time §9 and compression §10 from the same
reference file are audited separately.)

The reference sets a deliberately low bar — *"a v1 can parse/preserve the
keywords as ordinary header records and add typed support incrementally"* — which
the ordered header model already satisfies for lossless round-trip. The actual
implementation goes far beyond that: a typed pixel↔world transform for **23
projections with full `PVi_m` parameters, validated against `astropy.wcs` (wcslib)
golden values**, yielding coordinates in the frame the file declares
(`RADESYS`/`EQUINOX`). Converting *between* reference frames (FK4↔FK5↔Galactic↔ICRS)
is astrometry beyond the FITS standard and is **deliberately out of scope** —
delegate it to astropy `SkyCoord` / ERFA. The gaps below are unimplemented advanced
features (most flagged TODO in the module doc), not defects in what exists.

### Conformance matrix

| Keyword / feature | Code | Status |
|---|---|---|
| `WCSAXES` (default `NAXIS`) | `from_header` (`wcs/mod.rs:223`) | ✅ |
| `CTYPEia` 4-3 form; `RA`/`DEC` + `xLON`/`xLAT` | `find_celestial` (`wcs/mod.rs:364`) | ✅ |
| `CRPIXja` (default 0), `CRVALia` (default 0), `CDELTia` (default 1) | `from_header` axis read | ✅ |
| `CDELT` non-zero | not checked (singular matrix ⇒ error) | ✅ effectively |
| `PCi_ja` × `CDELT` / `CDi_ja` linear layer | matrix build (`wcs/mod.rs:254`) | ✅ |
| `PC`/`CD` mutually exclusive | `CD` silently wins if both present | 🟢 not rejected |
| `CROTAi` legacy (only without `PC`) | `wcs/mod.rs:276` | ✅ |
| `LONPOLEa`/`LATPOLEa` + defaults | `compute_pole` (`wcs/mod.rs:415`) | ✅ |
| Pixel↔world pipeline + matrix inverse | `pixel_to_world` (`wcs/mod.rs:323`) / `world_to_pixel` (`:344`) | ✅ |
| Zenithal `TAN`/`SIN`/`ARC`/`STG`/`ZEA`/`ZPN`/`AIR` | `Projection` (`wcs/mod.rs:37`) | ✅ |
| Zenithal-perspective `AZP`/`SZP` | `Projection` (`wcs/mod.rs:78`) | ✅ |
| Cylindrical `CAR`/`CEA`/`MER`/`SFL`/`CYP` | `Projection` | ✅ |
| All-sky pseudo-cyl. `AIT`/`MOL`/`PAR` | `Projection` (`wcs/mod.rs:56`) | ✅ |
| Conic `COP`/`COE`/`COD`/`COO` + pseudoconic `BON` + polyconic `PCO` | `Projection` (`wcs/mod.rs:66`) | ✅ |
| Quad-cube `TSC`/`CSC`/`QSC`, HEALPix `HPX`/`XPH` | `unsupported_celestial_code` (`wcs/mod.rs:811`) | 🟡 clean error |
| `RADESYSa`/`EQUINOXa` parse; inter-frame conversion | preserved as header keywords; no transform | ⚪ out of scope (astrometry) |
| Alternate WCS `a ∈ A–Z` | `alt` param | ✅ (untested) |
| `PVi_ma` projection params (`PSi_ma` unused) | threaded through project/deproject | ✅ |
| `CUNITia` (esp. celestial = degrees) | not read; degrees assumed | 🟡 ignored |
| Spectral WCS §8.4 (`FREQ-F2W`, …) | non-celestial ⇒ linear only | 🟡 not implemented |
| BINTABLE column WCS (`TCTYPn`/`iCTYPn`, Table 22) | — | 🟡 not implemented |
| `WCSNAMEa`/`CNAMEia`, `CRDERia`/`CSYERia` | — | 🟢 not exposed |
| Conventional `'STOKES'`/`'COMPLEX'` | linear pass-through | ✅ (degenerate) |

### Gaps

1. 🟡 **Quad-cube and HEALPix projections not implemented.** `TSC`/`CSC`/`QSC`
   (quad-cube) and `HPX`/`XPH` (HEALPix) are recognized as celestial codes but
   unimplemented; `from_header` returns `FitsError::UnsupportedProjection`
   (`unsupported_celestial_code`, `wcs/mod.rs:811`) rather than silently
   mis-transforming. `PVi_m` parameters *are* supported across all 23 implemented
   projections (slant `SIN`, `CEA` λ, `ZPN`, `AZP`/`SZP`/`CYP`/conic params, and
   `φ₀`/`θ₀`/LONPOLE/LATPOLE overrides), each astropy-validated.

2. 🟡 **`CUNITia` is ignored.** Celestial axes are assumed to be in degrees
   (`CRVAL`/`CDELT` taken as degrees) and `CUNIT` is never read, so a celestial
   axis declared in `arcsec`/`rad`, or any linear axis with a non-default unit, is
   mis-scaled. No `CUNIT` accessor is exposed either.

3. 🟡 **Spectral WCS (§8.4) not implemented.** Only celestial pairs get
   nonlinear treatment; a spectral axis with an algorithm code
   (`FREQ-F2W`, `WAVE-…`) falls through to the linear branch
   (`world = CRVAL + intermediate`), which is correct only for a genuinely linear
   spectral axis.

4. 🟡 **BINTABLE column WCS not supported (Table 22).** Only image-header
   keywords are parsed; the column-indexed forms (`TCTYPn`, `TCRPXn`, `iCTYPn`, …)
   have no support.

5. ⚪ **Reference-frame conversion is out of scope (deliberate).** `RADESYS`/
   `EQUINOX` are parsed and preserved as header keywords, and pixel↔world returns
   coordinates in the file's *declared* frame, but transforming between frames
   (FK4↔FK5↔Galactic↔ICRS — precession, E-terms of aberration, the ICRS↔FK5 frame
   bias) is astrometry outside the FITS standard; delegate it to astropy
   `SkyCoord` / ERFA. `GAPPT` (geocentric apparent place) is likewise not
   interpreted.

6. 🟢 **Lenient on illegal combinations / unexposed metadata.** `PC`+`CD` both
   present is not rejected (`CD` wins); `CROTA`+`PC` is not rejected (`PC` wins);
   `WCSNAMEa`/`CNAMEia` and `CRDERia`/`CSYERia` are not exposed.

### Test coverage

Strong and unusually rigorous — golden values come from `astropy.wcs` (wcslib),
so the formulas (not merely forward/inverse self-consistency) are
checked (`wcs/tests.rs`): `parses_tan_header` (`:24`) + `pixel_to_world_matches_astropy`
(`:36`, six TAN points to 1e-9); `world_to_pixel_inverts_pixel_to_world` (`:54`);
`reference_pixel_maps_to_crval` (`:71`); `sin_projection_matches_astropy` (`:168`);
`legacy_crota_rotation_matches_astropy`; `allsky_projections_match_astropy`
(`AIT`+`MOL` goldens); `projections_match_astropy` (`STG`/`ZEA`/`CAR`/`CEA`/`MER`/`SFL`
goldens); `cea_lambda_pv_matches_astropy` (the `CEA` `PV2_1` λ parameter);
`parameterized_projections_match_astropy` (the broad golden table covering the
`PVi_m`-parameterized and conic/perspective/polyconic projections — `ZPN`/`AIR`/
`AZP`/`SZP`/`CYP`/`PAR`/`COP`/`COE`/`COD`/`COO`/`BON`/`PCO`);
`unimplemented_projection_codes_error_cleanly` (quad-cube/HEALPix codes →
`UnsupportedProjection`); `projections_round_trip` (every implemented projection
project→deproject); and a standalone `matrix_inverse_is_correct`.

Coverage gaps:

- No **alternate-WCS** (`alt = Some('A')`) test, though the code path exists.
- No mixed celestial + non-celestial (`NAXIS ≥ 3`, e.g. a spectral/linear third
  axis) `pixel_to_world` test.
- No explicit **`PCi_j`-matrix** astropy test (only `CD`/`CDELT`+`CROTA`/bare
  `CDELT` are exercised).
- No singular-matrix → `InvalidValue` error test, no `WCSAXES`-vs-`NAXIS` default
  test, and no all-linear (no celestial pair) `Wcs` test.
- `CUNIT`, spectral, and table-WCS paths are untested (unimplemented).

---

## §9 — Representations of Time Coordinates (`docs/refs/07-wcs-time-compression.md` §7.2)

Audited code: `time/mod.rs` (`Datetime`, `Epoch`, `TimeScale`, `FitsTime`,
`is_time_ctype`, plus the leap-second / `tdb_minus_tt` / proleptic-Gregorian
helpers) and `time/tests.rs`.

§9 layers a full time framework onto the WCS spine: a time scale (`TIMESYS` +
Table 30), a reference value (`MJDREF`/`JDREF`/`DATEREF`, optionally split into
integer + fractional parts), a reference position/direction
(`TREFPOS`/`TREFDIR`), a time unit (`TIMEUNIT` + Table 34), ISO-8601 datetime
strings (§9.1.1), Julian/Besselian epochs (§9.1.2), global bound keywords
(§9.5), offset/binning/error keywords (§9.4), durations (§9.7), and a set of
time-related coordinate axes (§9.6). The implementation covers the
*computational core* and is **validated against `astropy.time` (ERFA)**:
ISO-8601↔JD/MJD calendar math, `J`/`B` epochs→JD, a full
UTC/TAI/TT/TCG/TDB/TCB/GPS/UT1 scale-conversion lattice (UTC↔TAI via an embedded
IERS leap table, TDB via the standard periodic series, UT1 via caller-supplied
ΔUT1), and a `FitsTime` header view resolving the reference epoch/unit/scale and
relative→absolute MJD for the global keywords and a `CTYPEi='TIME'` image axis.
The remaining gaps are the metadata-only / table-context / nice-to-have parts of
§9; the former `TIMEUNIT`, split-reference, and realization-suffix bugs are fixed.

**Scope.** Time-*scale* conversion (the TT-pivot lattice with the defining
`L_G`/`L_B` relations, leap seconds, the TDB series) is in scope: FITS §9.2.1
defines the scales and their relationships, and reading a stated time correctly
requires it. *Out of scope* — the same boundary drawn for celestial frames in §8 —
is the geometry that turns a stated `TREFPOS`/`TREFDIR`/`PLEPHEM` into an actual
light-travel / reference-position correction (observatory location + solar-system
ephemeris), and maintaining IERS-observed data such as a bundled ΔUT1 table. Those
keywords are read and preserved; the position-dependent corrections are delegated
to an astronomy library (astropy `SkyCoord`/`time`, ERFA), not implemented here.

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 9.2.1 | `TIMESYS` (default `UTC`); other values allowed | `FitsTime::from_header` (`time/mod.rs:388`) | ✅ |
| 9.2.1 | Table 30 scales (`TAI/TT/TCG/TDB/TCB/UTC/UT1/GPS/…`) | `TimeScale::parse` (`time/mod.rs:215`) | ✅ |
| 9.2.1 | Aliases `TDT`/`ET`→`TT`, `IAT`→`TAI` | `parse` arms (`time/mod.rs:215`) | ✅ |
| 9.2.1 | Realization suffix `TT(TAI)`, `UTC(NIST)` | `TimeScale::parse` strips `(...)` before matching | ✅ |
| 9.2.1 | `GMT` (continuous with UTC) | `parse` aliases `GMT` → `Utc` | ✅ |
| 9.2.1 | TT-pivot lattice; `TT↔TCG` (`L_G`), `TDB↔TCB` (`L_B`) | `to_tt`/`from_tt` (`time/mod.rs:245`,`:269`) | ✅ |
| 9.2.1 | TDB periodic series | `tdb_minus_tt` (`time/mod.rs:297`) | ✅ (no `TDB_0`) |
| 9.2.1 | `UT1` via ΔUT1; `LOCAL` pass-through | `convert_dut1` (`time/mod.rs:237`) | ✅ caller ΔUT1 / 🟡 no bundled table |
| 9.1.1 | ISO-8601 `[±C]CCYY-MM-DD[Thh:mm:ss[.s…]]`; parts optional | `Datetime::parse` (`time/mod.rs:45`) | ✅ |
| 9.1.1 | Leading zeros **must not** be omitted | `parse_fixed` requires exact 2-digit fields, ≥4-digit year | ✅ |
| 9.1.1 | **No** timezone designator (`Z` forbidden) | `Datetime::parse` rejects `Z` explicitly | ✅ |
| 9.1.1 | Seconds `00–60` UTC (leap), `00–59` else | `0.0..61.0` for all scales (`time/mod.rs:93`) | 🟡 scale-agnostic |
| 9.1.2 | Julian/Besselian epoch strings → JD | `Epoch::to_jd` (`time/mod.rs:176`) | ✅ |
| 9.1.2/9.5 | `JEPOCH` (TDB) / `BEPOCH` (ET) **keywords** | `FitsTime::epoch` → `EpochTime { mjd, scale }` | ✅ |
| 9.2.2 | Reference in ISO / JD / MJD; defaults | `reference_mjd` (`time/mod.rs:454`) | ✅ |
| 9.2.2 | `[M]JDREFI`+`[M]JDREFF` integer+fraction split | summed (`time/mod.rs:459`) | ✅ |
| 9.2.2 | **Split takes precedence over single** when all present | `resolve_split_ref`: `MJDREFI+MJDREFF` win over `MJDREF` | ✅ |
| 9.2.2 | Kind precedence `MJDREF > JDREF > DATEREF` | checked in that order (`time/mod.rs:455`,`:462`,`:469`) | ✅ |
| 9.3 | `TIMEUNIT` (default `s`); Table 34 units | `unit_seconds`: `s`/`min`/`h`/`d`/`a`/`yr`/`y`/`cy`/`ta`/`Ba` | ✅ |
| 9.2.3 | `TREFPOS` keyword (Table 31); position-dependent light-travel correction | stored verbatim (`time/mod.rs:394`) | ✅ read / ⚪ correction out of scope |
| 9.2.4 | `TREFDIR`/`TRDIRn` reference direction (correction geometry) | — | ⚪ out of scope (astronomy) |
| 9.2.5 | `PLEPHEM` (default `DE405`) planetary ephemeris | — | ⚪ out of scope (astronomy) |
| 9.4.1 | `TIMEOFFS` added to reference time | `FitsTime.timeoffs` added in `relative_to_mjd` | ✅ |
| 9.4.2 | `TIMEDEL` / `TIMEPIXR` binning | `TimeBounds` (`TIMEPIXR` default 0.5) | ✅ |
| 9.4.3 | `TIMSYER` / `TIMRDER` time errors | `TimeBounds` | ✅ |
| 9.5 | `DATE-OBS` / `MJD-OBS` start time | `obs_mjd` (`time/mod.rs:419`) | ✅ |
| 9.5 | `DATE-BEG`/`-END`, `MJD-BEG`/`-END` typed | `TimeBounds.beg_mjd`/`end_mjd` (MJD else DATE→MJD) | ✅ |
| 9.5 | `TSTART`/`TSTOP` (rel. to `[M]JDREF`, in `TIMEUNIT`) | `relative_to_mjd` (incl. `TIMEOFFS`) | ✅ |
| 9.6 | `CTYPEi='TIME'` image time axis → world time | `time_axis_mjd` (`time/mod.rs:433`) | ✅ |
| 9.6 | `'PHASE'`/`'TIMELAG'`/`'FREQUENCY'`; `CZPHSia`/`CPERIia` | `time_axis_kind` classifies the axis | ✅ recognition / 🟢 no value calc |
| 9.7 | `XPOSURE` / `TELAPSE` durations; GTI `START`/`STOP` | `TimeBounds`; `gti_intervals` → `GtiInterval` | ✅ |

The normative computational core — the Table-30 scale set with the canonical
aliases, the TT-pivot conversion lattice including the defining `L_G`/`L_B`
relations, ISO-8601↔JD/MJD calendar math, the `[M]JDREF`/`JDREF`/`DATEREF`
resolution with kind-precedence, J/B epochs, and a working `CTYPEi='TIME'` axis —
is implemented and astropy-validated. The remaining gaps cluster in metadata
semantics, table-only constructs, and the non-`TIME` time axes.

### Gaps

1. ✅ **FIXED — `TIMEUNIT` table complete (§9.3, Table 34).** `unit_seconds` now
   handles `s`/`min`/`h`/`d`/`a`/`yr`/`y`/`cy` with exact factors plus the
   deprecated `ta`/`Ba` (conventional year lengths); an unknown unit still falls
   back to seconds. So `TIMEUNIT='min'` is 60×, `'h'` 3600×, `'cy'` a Julian
   century — no longer silently 1 s. Covered by
   `timeunit_minute_hour_century_scale_correctly`.

2. ✅ **FIXED — split reference parts take precedence (§9.2.2).** `resolve_split_ref`
   returns `MJDREFI + MJDREFF` when *both* are present (wins over the single
   `MJDREF`), else the single value, else a lone split part — for both the
   `MJDREF`/`MJDREFI`/`MJDREFF` and `JDREF`/`JDREFI`/`JDREFF` branches. Covered by
   `split_reference_takes_precedence_over_single_mjdref`.

3. ✅ **FIXED — time-scale realization suffix stripped (§9.2.1).** `TimeScale::parse`
   splits off the `(...)` realization before matching, so `'TT(TAI)'` → `TT`,
   `'UTC(NIST)'` → `UTC`. Covered by `time_scale_parse_strips_realization_and_aliases`.

4. ✅ **FIXED — `GMT` aliases `UTC` (§9.2.1, Table 30).** `TimeScale::parse` maps
   `GMT` (continuous with UTC) to `Utc`. Covered by
   `time_scale_parse_strips_realization_and_aliases`.

5. ✅ **FIXED — `TIMEOFFS` applied (§9.4.1).** `FitsTime.timeoffs` is read and
   `relative_to_mjd` adds it (in `TIMEUNIT`) before scaling, so `TSTART`/`TSTOP`
   and the time axis resolve correctly when a bulk clock correction is present.
   Covered by `timeoffs_shifts_relative_times`.

6. ✅ **FIXED — ISO-8601 field widths enforced (§9.1.1).** `Datetime::parse` now
   requires a ≥4-digit (optionally signed) year and exactly-2-digit month / day /
   hour / minute / integer-seconds fields, and rejects a `Z` designator explicitly.
   Covered by `iso_8601_strictness`. (One leniency remains by design: second 60 is
   accepted in any scale because `Datetime` is scale-agnostic — the "only in UTC"
   rule is the caller's, since the type can't see `TIMESYS`.)

7. ✅ **FIXED — Julian/Besselian epoch keywords are read (§9.5, §9.1.2).**
   `FitsTime::epoch` reads the numeric `JEPOCH` (implied scale TDB) / `BEPOCH`
   (ET ≈ TT) keywords and returns an `EpochTime { mjd, scale }`. Covered by
   `reads_jepoch_and_bepoch_keywords`.

8. ⚪ **`UT1`/ΔUT1 are caller-supplied by design; bundling an IERS ΔUT1 table is
   out of scope.** `TimeScale::convert` treats `UT1` as `UTC` (ΔUT1 = 0) unless the
   caller routes through `convert_dut1` with an external ΔUT1 (`time/mod.rs:231`,`:237`).
   ΔUT1 is an IERS-observed quantity, not a FITS keyword; maintaining that table is
   astronomy-library territory, so caller-supplied ΔUT1 is the deliberate boundary.

9. ✅ **Typed reading of the metadata/table-context §9 keywords.**
   `FitsTime::bounds` returns a `TimeBounds` of the global bound/duration/error
   keywords — `MJD-BEG`/`DATE-BEG`, `MJD-END`/`DATE-END`, `XPOSURE`, `TELAPSE`,
   `TIMEDEL`, `TIMEPIXR` (default 0.5), `TIMSYER`, `TIMRDER` (§9.4/§9.5/§9.7);
   `FitsTime::gti_intervals` converts GTI `START`/`STOP` column values to
   absolute-MJD `GtiInterval`s (§9.7); and `time_axis_kind` classifies a `CTYPE` as
   `TIME`/`PHASE`/`TIMELAG`/`FREQUENCY` (§9.6). Covered by
   `reads_bound_duration_and_error_keywords`, `gti_intervals_convert_to_absolute_mjd`,
   and `classifies_time_related_axes`. (Pixel→value for the non-`TIME` axes, and
   `CZPHSia`/`CPERIia`, are not computed — recognition only.)

10. ✅ **`TDB_0` constant applied (IAU 2006).** The `TDB − TT` series stays purely
   periodic (matching ERFA `dtdb`, which the suite validates against); the
   `TDB_0 = −6.55 × 10⁻⁵ s` constant lives in the `TCB ↔ TDB` relation where the
   IAU definition places it (`to_tt`/`from_tt` `Tcb` arms), still astropy-validated
   by `scale_conversions_match_astropy`.

### Test coverage

Strong on the computational core, with golden values from `astropy.time` (ERFA)
(`time/tests.rs`): `iso_to_jd_and_mjd_match_astropy` (six ISO strings → JD/MJD
within 1e-7, incl. the MJD zero point, a fractional second, a leap-second label
`…23:59:60`, and a date-only midnight); `datetime_round_trips_through_jd`;
`rejects_malformed_datetimes` (empty, too-short, out-of-range month/day/hour);
`epochs_match_astropy` (`J2000`/`J2015.5`, `B1950`/`B1900` within 1e-5);
`scale_conversions_match_astropy` (UTC→{TAI,TT,TCG,TDB,TCB,GPS} each to 1e-9 day
+ round-trip — pins the 37 s leap value, `TT−TAI`, the GPS offset, and the
`L_G`/`L_B`/TDB rates at once); `ut1_uses_explicit_dut1` (astropy ΔUT1, +
round-trip, + the ΔUT1=0 default); `leap_seconds_match_iers_table` (counts at
1972/1980/1999/2017/2024 + the step just before the 1999 insertion);
`time_axis_resolves_to_mjd` (`CTYPE3='TIME'`, pixel→MJD, and a non-time axis →
`None`); `fits_time_resolves_reference_and_relative_times` (scale/mjdref/trefpos
+ `TSTART`/`TSTOP`/`DATE-OBS`); and `fits_time_reads_split_and_day_unit_references`
(the `MJDREFI`+`MJDREFF` split alone, default UTC, `TIMEUNIT='d'`).

Coverage gaps:

- The three former 🔴 bugs are now tested: `split_reference_takes_precedence_over_single_mjdref`
  (gap #2, both single + split present), `timeunit_minute_hour_century_scale_correctly`
  (gap #1), and `time_scale_parse_strips_realization_and_aliases` (gap #3).
- `TimeScale::parse` now has a test covering the realization suffix, the
  `TDT`/`ET`/`IAT` and `GMT` aliases, and the unknown→`Local` fallback; the full
  Table-30 map is still only partially exercised.
- `iso_8601_strictness` covers signed/extended years, leading-zero omission, and
  the `Z`-suffix rejection (gap #6); `reads_jepoch_and_bepoch_keywords` covers the
  epoch keywords (gap #7). No `JDREF`/`DATEREF` resolution or kind-precedence test
  yet (only `MJDREF` and the split are exercised).
- `TIMEOFFS` is applied and tested (`timeoffs_shifts_relative_times`); the
  metadata/duration/GTI keywords and axis classification are covered by
  `reads_bound_duration_and_error_keywords`, `gti_intervals_convert_to_absolute_mjd`,
  and `classifies_time_related_axes`.

---

## §10 — Representations of Compressed Data (`docs/refs/07-wcs-time-compression.md` §7.3)

Audited code: `compress/mod.rs` (`decompress_image`/`encode_image`, tile
reassembly, `ZIMAGE` container, fallback columns), `compress/{gzip,rice,plio,
hcompress,quantize,table}.rs`, and the entry points `read_compressed_image` /
`read_compressed_table` (`reader/mod.rs:189`,`:199`) and `write_compressed_image`
/ `write_compressed_image_lossy` / `write_compressed_table` (`writer/mod.rs:257`,
`:270`,`:287`), behind the `compression` feature.

§10 stores a compressed image (or table) *inside* a `BINTABLE`: the image is
split into `ZTILEn` tiles, each compressed with `ZCMPTYPE` and stored as a VLA
cell in `COMPRESSED_DATA`, with the original geometry in `ZBITPIX`/`ZNAXIS`/
`ZNAXISn`; floating-point images are first quantized per-tile (`ZSCALE`/`ZZERO`)
with optional Appendix-I subtractive dithering. The implementation is unusually
complete: **all five image codecs (`RICE_1`, `GZIP_1`, `GZIP_2`, `PLIO_1`,
`HCOMPRESS_1`) read and write** — including lossy `HCOMPRESS` (`SCALE > 0`) write
and `SMOOTH = 1` decode; float quantization with all three `ZQUANTIZ` methods,
`ZBLANK`/NaN nulls, and a raw-gzip fallback for un-quantizable tiles; and §10.3
fixed-width table compression read and write. The codecs are ports of cfitsio's
`fits_rdecomp`/`fits_hdecompress`/`pl_l2pi`/`fits_quantize_float`, and the decode
paths are cross-checked against **astropy- and cfitsio/`fpack`-produced golden
files**. The gaps are missing optional pieces (`NOCOMPRESS`, `ZMASKCMP`/
`NULL_PIXEL_MASK`, lossy-`HCOMPRESS` *encode* smoothing, the verbatim-copy
reconstruction keywords, in-table VLA columns), not defects in the core codecs.

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 10.1 | Compressed image = `BINTABLE`; tiles row-major, one per row | `decompress_image` (`compress/mod.rs:39`), `encode_image` (`:192`) | ✅ |
| 10.1.1 | `ZIMAGE = T` mandatory | `NotCompressedImage` if absent (`compress/mod.rs:40`) | ✅ |
| 10.1.1 | `ZCMPTYPE` mandatory; Table-36 values only | `decode_tile_cell` dispatch (`compress/mod.rs:629`) | ✅ read / 🟡 value not pre-validated |
| 10.1.1 | `ZBITPIX` = original `BITPIX` | `Bitpix::from_code` (`compress/mod.rs:43`) | ✅ |
| 10.1.1 | `ZNAXIS`/`ZNAXISn` = original dims | `read_axes` (`compress/mod.rs:686`) | ✅ / 🟡 `.max(0)` not `>0` |
| 10.1.2 | `ZTILEn` tiling; default row-by-row | tile build (`compress/mod.rs`) | ✅ |
| 10.1.2 / 10.4.1 | `ZNAMEi`/`ZVALi`: Rice `BLOCKSIZE` (16/**32**), `BYTEPIX` (1/2/4/**8**, def 4) | `rice_params` (`compress/rice.rs:8`) | ✅ |
| 10.1.2 | `ZQUANTIZ` `NO_DITHER`/`SUBTRACTIVE_DITHER_1`/`_2` (def `NO_DITHER`) | `DitherMethod` (`compress/mod.rs`) | ✅ |
| 10.1.2 | `ZDITHER0` (1–10000) dither seed | read (`compress/mod.rs`) | ✅ / 🟡 range unchecked |
| 10.1.2 | `ZMASKCMP` null-mask codec | — | 🟢 not implemented |
| 10.1.2 | `ZSIMPLE`/`ZTENSION`/`ZPCOUNT`/`ZHECKSUM`/… verbatim-copy (image) | not read/written | 🟢 not implemented |
| 10.1.3 | `COMPRESSED_DATA` (`1P(B/I/J)` or `1Q…`) | `read_tiles` (`compress/mod.rs:539`); written `1P` | ✅ (P only; gap #6) |
| 10.1.3 | `GZIP_COMPRESSED_DATA` fallback (null `COMPRESSED_DATA` descr.) | read+write (`compress/mod.rs:100`) | ✅ |
| 10.1.3 | No `UNCOMPRESSED_DATA` column in 4.0 | read as 3rd fallback (`compress/mod.rs:101`) | 🟡 lenient (reads pre-standard column) |
| 10.1.3 | `NULL_PIXEL_MASK` for lossy-codec nulls | — | 🟢 not implemented |
| 10.1.3 | `ZBLANK` (column or keyword); column wins | per-tile column overrides the keyword (`read_i64_column`) | ✅ |
| 10.2 | `physical = ZZERO + ZSCALE × I` (Eq. 12) / dithered Eq. 14 | `dequantize` (`compress/quantize.rs:97`) | ✅ |
| 10.2.1 | `SUBTRACTIVE_DITHER_2`: exact `0.0` ↔ `ZERO_VALUE` | `ZERO_VALUE` (`compress/quantize.rs:19`) | ✅ |
| 10.2.1 / App. I | Park–Miller PRNG, 10000th seed = 1043618065 | `random_values` (`compress/quantize.rs:40`,`:53`) | ✅ |
| 10.4.1 | `RICE_1` (integer only) | `rice_decode`/`rice_encode` (`compress/rice.rs:26`,`:77`) | ✅ |
| 10.4.2 | `GZIP_1` DEFLATE; `GZIP_2` MSB-first shuffle | `compress/gzip.rs` | ✅ |
| 10.4.3 | `PLIO_1` IRAF mask RLE (ints 0–2²⁴) | `compress/plio.rs` | ✅ |
| 10.4.4 | `HCOMPRESS_1` 2-D; `SCALE` param; `SMOOTH` decode | `compress/hcompress.rs`; `hcompress_smooth` (`compress/mod.rs:657`) | ✅ decode / 🟡 no `SMOOTH` encode |
| 10.4 | `NOCOMPRESS` stored uncompressed | `decode_tile_cell` decodes verbatim big-endian pixels (read) | ✅ read / 🟢 no encode |
| 10.3 | `ZTABLE = T`; one row per row-tile, `1QB` columns | `uncompress_table`/`compress_table` (`compress/table.rs:234`,`:158`) | ✅ |
| 10.3.1 | `ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT`/`ZFORMn`/`ZCTYPn`/`ZTILELEN` | parsed + written (`compress/table.rs`) | ✅ |
| 10.3.5 | Tables: lossless `RICE_1`/`GZIP_1`/`GZIP_2` only | `Algo::parse` rejects others (`compress/table.rs:43`) | ✅ (no `NOCOMPRESS`) |
| 10.3.4 | `ZTHEAP`/`ZHECKSUM`/`ZDATASUM` verbatim-copy (table) | only `ZPCOUNT` preserved | 🟢 not implemented |
| 10.3.6 | VLA columns in a compressed table | rejected (`col_meta`, `compress/table.rs:122`) | 🟡 rejected, not compressed |

### Gaps

1. ✅ **`NOCOMPRESS` images now read (§10.4, Table 36).** `decode_tile_cell` has a
   `NOCOMPRESS` arm decoding the verbatim big-endian pixels of a tile (integer and
   float-quantized paths both flow through it). Covered by
   `decompresses_nocompress_tile_verbatim`. (No *encoder* — we never emit
   `NOCOMPRESS` — and table `ZCMPTYPE` still excludes it per §10.3.5.)

2. 🟡 **A pre-standard `UNCOMPRESSED_DATA` column is read (§10.1.3).** FITS 4.0
   defines **no** such column, yet `decompress_image` reads it as a third per-tile
   fallback (`compress/mod.rs:101`). Lenient/legacy-tolerant for a reader and
   harmless, but it accepts a column the current standard does not sanction.

3. 🟢 **Verbatim-copy reconstruction keywords neither read nor written (§10.1.2,
   §10.3.4).** For images, `ZSIMPLE`/`ZTENSION`/`ZPCOUNT`/`ZHECKSUM`/`ZDATASUM`
   (meant to rebuild a byte-identical original HDU) are ignored — `decompress_image`
   returns a freshly-built `Image`. For tables, only `ZPCOUNT` is preserved;
   `ZTHEAP`/`ZHECKSUM`/`ZDATASUM` are not. A compress→decompress cycle loses the
   original SIMPLE/XTENSION/PCOUNT/checksum keywords.

4. 🟢 **`NULL_PIXEL_MASK` / `ZMASKCMP` lossy-null preservation unimplemented
   (§10.1.3, §10.2.2).** For lossy codecs (e.g. `HCOMPRESS` `SCALE > 0`),
   undefined pixels must be recorded via a compressed mask. Neither keyword nor
   column is referenced, so a lossy image carrying a null mask loses its
   blank-pixel locations on decode (integer `BLANK` and float `ZBLANK`/NaN paths
   *are* handled — gap #6 — so this affects only the lossy-mask case).

5. 🟡 **VLA columns inside a compressed table are rejected, not compressed
   (§10.3.6).** `col_meta` returns `UnsupportedCompression` for any `P`/`Q` source
   column (`compress/table.rs:122`). The behavior is *clean* (the cfitsio-`fpack`
   VLA fixture errors rather than misreads), but the §10.3.6 two-stage
   descriptor-compression procedure is absent.

6. 🟡 **Image encoders write only 32-bit `1P` descriptors (§10.1.3).** Both image
   encoders emit `1P` (`i32`) descriptors; the standard requires `1Q` (64-bit) once
   the heap exceeds ~2.1 GB, which this writer cannot produce (decode of `1Q`
   works). **Open** (read side fine). ✅ **The `ZBLANK` *column* is now read** — a
   per-tile `ZBLANK` column overrides the keyword (`read_i64_column`,
   `zblank_column_overrides_keyword_per_tile`), per §10.1.3.

7. 🟡 **Mild under-validation.** `ZCMPTYPE` is not checked against Table 36 up
   front (an unknown value fails only when a tile is decoded); `read_axes` accepts
   `ZNAXISn ≤ 0` via `.max(0)` (`compress/mod.rs:686`); `ZDITHER0` is not range-
   checked (1–10000). All lenient-reader choices, none wrong on conforming files.

### Test coverage

Strong, anchored on independent golden files (astropy and cfitsio/`fpack`)
(`compress/tests.rs`). Image decode: `decompresses_{gzip_1,gzip_2,rice_1,
hcompress_1}_tiled_image` decode a 24×16 `i16` fixture asserting every pixel
equals `x·7 − y·5`; `decompresses_plio_1_mask` asserts `(x+y)%7` per pixel.
Lossy/quantized decode is pixel-exact against astropy: `decompresses_hcompress_lossy`
(SCALE=4), `decompresses_hcompress_smoothed` (SMOOTH=1, bit-for-bit),
`decompresses_subtractive_dither_2`, `decompresses_quantized_float_no_dither`,
`decompresses_unquantized_float_via_gzip_fallback` (the `ZSCALE=0` path), and
`decompresses_float_with_nan_nulls` (exactly 2 `ZBLANK`→NaN pixels). Encode is
exercised by round-trip + cross-check: `compression_write_round_trips_through_decode`
(all four integer codecs), `plio_write_round_trips_through_decode`,
`float_quantize_write_round_trips_within_tolerance` (asserts both `max_err < 0.2`
*and* that the tile actually quantized), `float_write_preserves_nan_nulls`,
`hcompress_lossy_write_round_trips_within_scale` (`|err| ≤ scale`), and
`dither2_quantize_round_trips` (exact zeros → `ZERO_VALUE` → exactly `0.0`). The
PRNG has a built-in `debug_assert_eq!(seed, 1_043_618_065)`. Table compression:
`table_compression_round_trips` (6-column table × `GZIP_1`/`GZIP_2`/`RICE_1` ×
tile heights {10,4,1}, byte-identical); `decodes_a_cfitsio_compressed_table`
(500-row `fpack -tableonly` file, mixed per-column codecs, byte-identical);
`compressed_table_with_vla_column_is_rejected_cleanly`; and both readers reject a
plain `BINTABLE`. Two `#[ignore]` emitters regenerate the write-side fixtures for
external (astropy / `funpack`) validation.

Coverage gaps:

- No `NOCOMPRESS` (gap #1), `UNCOMPRESSED_DATA`-fallback decode (gap #2), or
  `ZMASKCMP`/`NULL_PIXEL_MASK` (gap #4) test; no HDU-reconstruction test for the
  verbatim-copy keywords (gap #3).
- `RICE_1` is only tested at `BYTEPIX = 2`/`4`; `BYTEPIX = 1`/`8` and
  `BLOCKSIZE = 16` are never decoded. `GZIP_2` is exercised only on `i16` outside
  the float/table paths.
- No `i64`/`u8` (`ZBITPIX = 64`/`8`) image and no ≥3-D compressed cube — all image
  fixtures are 2-D `i16`/`i32`/`f32`, so multi-axis `ZTILEn` tiling is unexercised.
- No `ZBLANK`-as-column (gap #6), `1Q`-descriptor write (gap #6), or non-default-
  tiled image test; `ZTHEAP`/`ZHECKSUM`/`ZDATASUM` table preservation (gap #3) is
  untested.

---

## Conventions — CONTINUE / CHECKSUM / HIERARCH (`docs/refs/08-conventions.md`)

Covers §4.2.1.2 (`CONTINUE`), §4.4.2.7 + Appendix J (`CHECKSUM`/`DATASUM`), and
the registered ESO `HIERARCH` convention. Audited code: `header/card/mod.rs`
(`CONTINUE`/`HIERARCH` parse + render, `render_long_string`, `split_escaped`),
`header/mod.rs` (`fold_continuation`), `checksum.rs` (`accumulate`, `encode`),
`reader/mod.rs` (`verify_checksum`), `writer/mod.rs` (`write_hdu`,
`patch_checksum`).

The reference's bar is "read all three; write `CONTINUE` and
`CHECKSUM`/`DATASUM`". The library clears it — **all three convention are read
*and* written** (including `HIERARCH` write, which is not required), with strong
tests. Findings are minor or design-level.

### Conformance matrix

| Convention | Requirement | Code | Status |
|---|---|---|---|
| CONTINUE | Read: fold `&`-chains; comment from last record | `fold_continuation` (`header/mod.rs:191`) | ✅ |
| CONTINUE | Read: orphan `CONTINUE` → commentary; trailing `&` w/o CONTINUE is literal | `Header::parse`; tested | ✅ |
| CONTINUE | Write: substrings < 68; `''` pair kept atomic | `render_long_string`/`split_escaped` (`card/mod.rs:387`) | ✅ |
| CONTINUE | Write: `&` on all but last; `CONTINUE` has spaces (no `= `) in 9–10 | `render_long_string` | ✅ |
| CONTINUE | Not applied to mandatory/reserved keywords | not enforced (moot: those aren't long strings) | 🟢 |
| CONTINUE | Preserve original physical byte layout on round-trip | folds + canonical re-emit | 🟡 logical-only |
| CHECKSUM | 32-bit ones'-complement sum, BE words, end-around carry | `accumulate` (`checksum.rs:9`) | ✅ |
| DATASUM | Decimal string of **data-only** sum; `'0'` if no data; before CHECKSUM | `write_hdu` (`writer/mod.rs:314`) | ✅ |
| CHECKSUM | 16-char ASCII, fixed cols 11/28; whole-HDU sum = −0 | placeholder + `patch_checksum` (`writer/mod.rs:581`) | ✅ |
| CHECKSUM | Verify = sum HDU → `0xFFFFFFFF` | `verify_checksum` (`reader/mod.rs:209`) | ✅ |
| CHECKSUM | J.2 ASCII encode, alphanumeric, punctuation fix-up | `encode` (`checksum.rs:25`) | ✅ |
| CHECKSUM | Recommended timestamp comment | not written | 🟢 |
| CHECKSUM | Incremental update (J.4) | full re-sum each write | 🟢 |
| HIERARCH | Detect in bytes 1–8; compound key to `=`; normal value syntax | `Card::parse` (`card/mod.rs:95`) | ✅ |
| HIERARCH | Read + render round-trip; value-indexed | `render` (`card/mod.rs:153`) | ✅ |
| HIERARCH | Expose raw token list *and* normalized key | normalized (space-joined) only | 🟢 |
| HIERARCH | Optional / feature-flagged | always on (harmless) | 🟢 |

### Gaps

1. 🟡 **`CONTINUE` round-trip preserves the logical value, not the original byte
   layout.** `Header::parse` folds a `CONTINUE` chain into a single value card and
   the writer re-emits a *canonical* chain (`card/mod.rs:387`), so the substring
   split and record count need not match the input. The header-model doc states
   this deliberately, but it does deviate from the impl-note goal ("keep the
   physical records so round-trips reproduce the original byte layout") and from
   the crate's general byte-for-byte header round-trip principle — `CONTINUE` is
   the one documented exception.

2. ✅ **FIXED — long `HIERARCH` string values continue instead of truncating.**
   `render_records` now emits a `CONTINUE` chain for an overflowing `HIERARCH`
   string too — the first record is `HIERARCH key = '…&'` (with its prefix shrinking
   that record's substring budget), the rest are standard `CONTINUE` records.
   Covered by `long_hierarch_string_splits_into_a_continue_chain`.

3. 🟢 **Minor / optional, unimplemented:** the `CONTINUE`-on-reserved-keyword
   restriction is not enforced (moot — mandatory keywords aren't strings); no
   recommended timestamp comment on `CHECKSUM`; no incremental `CHECKSUM` update
   (J.4); only the normalized space-joined `HIERARCH` key is exposed (no raw
   token list); and `HIERARCH` is always on rather than feature-gated.

### Test coverage

Well covered. **CONTINUE** (`header/tests.rs`, `card/tests.rs`): the doc's exact
three-record `WEATHER` example reassembles; a trailing `&` with no following
`CONTINUE` stays literal; an orphan `CONTINUE` is demoted to commentary; a single
`CONTINUE` record parses; a long value splits into a chain with an embedded `''`
pair kept off the record boundary and then reassembles; a short string stays one
record. **CHECKSUM/DATASUM** (`checksum.rs`, `writer/tests.rs`): end-around-carry
fold; encoded output is alphanumeric across several sums; a write→verify
round-trip yields `datasum_ok = checksum_ok = Some(true)`; a flipped data byte
makes both `Some(false)`; absent keywords give `None`. **HIERARCH**
(`card/tests.rs`): a string-valued card with comment and a numeric card both parse
and render-round-trip, including the compound key.

Coverage gaps:

- No assertion that a written `CONTINUE` record actually has spaces (not `= `) in
  bytes 9–10, and no write→read test that a comment on the final `CONTINUE`
  record survives.
- `CHECKSUM`/`DATASUM` are verified **only on self-written files** — never against
  a real archive file carrying pre-existing keywords, so byte-level interop with a
  CFITSIO/astropy-produced `CHECKSUM` string is unproven (the `encode` output is
  checked for the alphanumeric property and via the internal sum→−0 round-trip,
  not against a known external value).
- No `Header::get` on a `HIERARCH` compound key, no long-`HIERARCH`-value test,
  and no explicit `DATASUM = '0'` (dataless HDU) test.
