# FITS Conformance Audit

This document records the result of auditing the `fits` implementation against
the curated reference notes in [`docs/refs/`](refs/). Each section maps one
reference file to the code that implements it, flags conformance gaps (with
severity and `file:line` anchors), and assesses test coverage.

Severity legend: 🔴 correctness bug (rejects valid files or produces wrong
output) · 🟡 lenient/permissive beyond the standard (safe for a reader, but not
strictly conforming) · 🟢 missing nice-to-have / "should" clause.

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
| 1.5 | Special records (§3.5) | — | 🔴 not handled |
| 1.6 | Trailing partial / zero-fill block (§3.6) | `fill_block` errors | 🔴 rejects file |
| 1.6 | Eq 1 / Eq 2 / Eq 4 sizing; `ceil(Nbits/8/2880)` | `data_extent`, `padded_len` | ✅ |
| 1.6 | Nbits non-negative; overflow-safe | checked arithmetic + PCOUNT/GCOUNT guards | ✅ |
| 1.7 | "Once FITS always FITS" (random groups) | `read_groups`, `classify` | ✅ |

### Gaps

1. 🔴 **Special records / trailing blocks make `open()` fail (§3.5–3.6).**
   `FitsReader::open` loops `read_header_unit`, which reads 2880-byte blocks
   until one carries an `END` card (`reader/mod.rs:238`). After the last HDU,
   *any* trailing content — special records, or even a single trailing all-zero
   padding block that some writers append — has no `END`, so blocks accumulate
   until EOF and `read_header_unit` returns `Err(UnexpectedEof)`
   (`reader/mod.rs:244`). The standard says a reader *may ignore* special
   records and *disregard* trailing partial blocks; this code rejects the whole
   file. A trailing partial (sub-2880) block likewise hits
   `Err(UnexpectedEof)` in `fill_block` (`reader/mod.rs:274`). No test exercises
   trailing content.

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
| 2.2 | **Null vs empty string distinct** (§4.2.1.1) | `parse_string` strips *all* trailing spaces | 🔴 conflated |
| 2.2 | Undefined = blank value field, no quotes | `Value::Undefined` (`card/mod.rs:267`) | ✅ |
| 2.2 | ≤68 chars/record; longer ⇒ CONTINUE | `render_records` / `render_long_string` | ✅ |
| 2.2 | XTENSION padded to 8; no other min length | `pad_string` (`card/mod.rs:470`) | ✅ |
| 2.2 | Numbers fit field; no thousands separators | `i64`/`f64` parse rejects separators | ✅ (large reals: see gap #4) |
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

1. 🔴 **Null string and empty (all-blank) string are conflated (§4.2.1.1).**
   The doc is explicit: `KEYWORD= ''` is a *null* string (length 0), while
   `KEYWORD= '   '` is an *empty* string that, because the first space is
   significant and only trailing spaces are dropped, reduces to a **single
   space (length 1)** — and that one space is exactly what distinguishes it
   from the null string. `parse_string` strips *all* trailing spaces with
   `while out.ends_with(' ') { out.pop(); }` (`card/mod.rs:305`), so `'   '`
   collapses to `""`, identical to `''`. Worse, the existing test
   **asserts the wrong behavior**: `parse("BLANKS  = '      '")` is asserted to
   equal `Value::Text(String::new())` (`card/tests.rs:71`). The all-blank case
   should yield `Value::Text(" ")` and must compare unequal to the `''` null
   case. Fix the parser to preserve one significant space when the string is
   non-empty but all-blank, and correct the test.

2. 🟡 **Restricted-ASCII range not enforced (§4.1).** Headers are limited to
   decimal 32–126, but `Card::parse` only rejects bytes ≥ 128 via
   `!raw.is_ascii()` (`card/mod.rs:70`). Control characters 0–31 (tab, NUL, …)
   and DEL (127) pass through into value/comment text. Lenient; a strict reader
   would reject them.

3. 🟡 **Value indicator only checks column 9.** The standard's indicator is the
   two bytes `"= "` (cols 9–10); the code keys solely on `raw[8] == b'='`
   (`card/mod.rs:125`) and ignores column 10. Safe in practice (commentary
   keywords are matched first) but not a strict `"= "` check.

4. 🔴 **Large-magnitude reals can overflow the value field on write.**
   `format_real` uses Rust's `{}` Display (`card/mod.rs:459`), which never emits
   exponent notation, so e.g. `1e300` renders as a 301-digit decimal. The value
   field is only ~70 bytes (cols 11–80) and `render_records` emits a CONTINUE
   chain **only for strings**, not numbers; `write_at` then silently clamps to
   80 bytes (`card/mod.rs:478`), truncating the number into a wrong value. Edge
   case, but a silent-corruption bug. (Also why the §4.2.4 "upper-case exponent"
   rule is trivially met on write — exponents are never emitted at all.)

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
| 3.3 | Unsigned 16/32/64 + signed-8 via `BZERO`/`TZEROn` | `physical()` float plane | ✅ values / 🟡 no typed `uN` |
| 3.4 | `-32`/`-64` IEEE-754, big-endian | `f32`/`f64::from_be_bytes` (`data/mod.rs:69`) | ✅ |
| 3.4 | NaN = blank float; no float `BLANK` | `scale_ints` for ints only; float NaN propagates | ✅ |
| 3.4 | Preserve ±Inf + signaling/quiet NaN payload on round-trip | `to_bits`/`from_bits` are bit-exact | ✅ code / ⚠️ untested |
| 3.4 | Honor `BSCALE`/`BZERO` on floats if present | `physical()` scales floats (`data/mod.rs:116`) | ✅ |
| 3.5 | `physical = BZERO + BSCALE × stored` (Eq. 3) | `scale` closure (`data/mod.rs:110`) | ✅ |
| 3.5 | Defaults `BSCALE=1.0`, `BZERO=0.0` | `from_header` `unwrap_or` (`data/mod.rs:150`) | ✅ |
| 3.5 | `BLANK` integer-only, applied *before* scaling | `scale_ints` sentinel→NaN pre-scale (`data/mod.rs:124`) | ✅ |
| 3.5 | Unsigned convention table (8/16/32/64) | `physical()` | ✅ values (u64: gap #2) |
| 3.5 | `TZEROn`/`TSCALn` binary-table analogue | `table/` layer | ✅ (audited under §6) |
| 3.6 | Time defers to §9 | `time/` feature | ✅ (audited under §9) |
| impl | Zero-copy raw + SIMD bulk byte-swap | `decode` always allocates + converts | 🟢 TODO (perf) |
| impl | Detect + expose as `uN` | no `U16`/`U32`/`U64` variant | 🟡 not implemented |
| impl | `BLANK` → `Option`/mask | NaN in physical plane | 🟢 by design |

The normative core of §5 (BITPIX types, big-endian two's-complement integers,
IEEE floats, Eq. 3 scaling with defaults, `BLANK`-before-scaling, the unsigned
`BZERO` convention) is correctly implemented. The gaps are design-principle and
edge-precision items, not wrong decoding.

### Gaps

1. 🟡 **No native unsigned (`uN`) typed exposure.** The design principle
   (CLAUDE.md: "detect and expose as `uN`") and the §3 impl-notes call for
   exposing `BITPIX` integer + `BZERO == 2^(n-1)` + `BSCALE == 1` as a typed
   `u16`/`u32`/`u64` (and signed-8 as `i8`). `ImageData` has only
   `U8/I16/I32/I64/F32/F64` (`data/mod.rs:16`), so unsigned 16/32/64 and signed
   bytes are readable *only* through the `f64` `physical()` plane — there is no
   zero-copy typed unsigned buffer, and no encode helper that takes a `uN` buffer
   and applies the inverse offset (the caller must pre-offset into signed storage,
   as the writer test does).

2. 🟡 **`u64`/large-`i64` physical values lose precision.** `physical()` returns
   `f64`, computed as `xi as f64` (`data/mod.rs:134`). For 64-bit integers whose
   magnitude exceeds 2⁵³ — including any `u64` unsigned value realized via
   `BZERO = 2⁶³` — the physical value is rounded. The raw sample plane is exact;
   only the derived `f64` plane is lossy. A native `uN`/`i64` path (gap #1) would
   avoid this.

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

- 🔴 **NaN/Inf bit-for-bit round-trip is the headline gap.** §3.4 *mandates*
  preserving ±Inf and signaling/quiet NaN payloads and not canonicalizing the
  quiet/signaling bit, and the project's own correctness rules require a
  float-NaN/Inf round-trip test — yet `encode_is_the_inverse_of_decode` uses only
  `1.0, -2.5, 0.0, f64::MAX`. The code *is* correct (`to_bits`/`from_bits` are
  bit-exact) but unverified. Need a test that round-trips the Appendix-E patterns
  (`+∞` = `0x7F800000` / `0x7FF0000000000000`, a signaling NaN such as
  `0x7F800001` / `0x7FF0000000000001`) and asserts `to_bits()` is identical for
  both `f32` and `f64`.
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
| 4.3 | IMAGE with `PCOUNT≠0`/`GCOUNT≠1` is malformed | `read_image` `assert_eq!` panics | 🟡 panics, not a clean error |
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

1. 🟡 **A malformed IMAGE panics instead of erroring.** `read_image` asserts
   `samples.len() == NAXISn product` (`reader/mod.rs:139`). For an `IMAGE`
   extension (or primary) with a non-conforming `PCOUNT > 0` or `GCOUNT > 1`,
   `data_extent` sizes the data unit as `elem × GCOUNT × (PCOUNT + product)`, so
   the decoded sample count exceeds the product and the assertion fires — a panic
   on untrusted file content, which the project's rules say should be a `Result`
   error, not an assert. Validate `PCOUNT == 0 && GCOUNT == 1` for image HDUs and
   return a `FitsError` instead. The same untrusted-input panic shape recurs in
   `RandomGroups::from_data`, whose closing `assert_eq!` on the decoded sample
   count (`groups/mod.rs:76`) fires on a corrupt random-groups header (the
   negative-`PCOUNT` / `GCOUNT < 1` cases *are* `Result`-guarded). By contrast the
   `write_image` `assert_eq!` (`writer/mod.rs:165`) is an intentional logic-error
   guard on a caller-built `Image`, not a read-side defect.

2. 🟢 **No coordinate-indexing / strided-view API (§4.1).** `Image` stores the
   flat buffer (correctly in Fortran order) and the `shape`, but exposes no
   `at(coords)` accessor or strided/ndarray view, so the documented index mapping
   is left entirely to the caller. The impl-notes call for strided views
   (`stride[0] = 1`); not implemented.

3. 🟢 **Reserved image keywords have no typed accessors (§7.1.2).** `BUNIT`,
   `DATAMIN`, `DATAMAX`, `EXTNAME`, `EXTVER`, `EXTLEVEL` are readable only as raw
   header cards; `Scaling` covers `BSCALE`/`BZERO`/`BLANK` and the `wcs` feature
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
| 5.1 | Field matching `TNULLn` is **undefined** | not read at all | 🟡 missing |
| 5.2 | Mandatory keys present + in order | read requires `NAXIS1/2`,`TFIELDS`,`TBCOLn`,`TFORMn`; write emits in order | ✅ |
| 5.2 | `TFIELDS` 0…999 | no upper-bound check | 🟢 (as §3 999) |
| 5.3 | `Aw`/`Iw`/`Fw.d`/`Ew.d`/`Dw.d`, upper-case only | `parse_ascii_tform` matches `A/I/F/E/D` only | ✅ |
| 5.3 | Scalar cells, no repeat/arrays | no repeat parsing | ✅ |
| 5.3 | `F`/`E`/`D` parse identically; base-ten; sign+exp | all → `Float`, `f64` parse, `D`→`E` | ✅ |
| 5.3 | Implicit decimal point (deprecated) | `decimals` ignored on read | 🟡 not handled |
| 5.4 | `TTYPEn` name, compared **case-insensitively** | stored; `column_index` is case-**sensitive** | 🟡 |
| 5.4 | `TUNITn` units | read into `unit` | ✅ |
| 5.4 | `TSCALn`/`TZEROn` scaling (not on `A`) | not implemented for ASCII | 🟡 missing |
| 5.4 | `TNULLn` (string) undefined marker | not implemented | 🟡 missing |
| 5.4 | `TDISPn`, `TDMINn`/`TDMAXn`, `TLMINn`/`TLMAXn` | not implemented | 🟢 |
| impl | Right-justify numerics, left-justify strings, gap-fill spaces | `format_ascii_field` | ✅ |
| impl | Overflow handling | `*`-fill per §7.2.5 (`writer/mod.rs:656`) | ✅ |
| impl | Float-precision lint on write | — | 🟢 |

`TFORMn` parsing, field slicing, and the write→read round-trip are correct. The
substantive gaps are the three ASCII-table semantics the standard attaches to
columns — `TNULLn`, `TSCALn`/`TZEROn`, and implicit decimal points — none of which
are implemented.

### Gaps

1. 🟡 **`TNULLn` undefined values not handled (§7.2.5).** `read_column` never
   reads `TNULLn`, and `ColumnData` (`I64`/`F64`/`Text`, dense `Vec`s) has no
   per-cell undefined representation. A conforming table whose null marker is a
   non-numeric string (e.g. `'NULL'`/`'***'`) therefore fails to read: the
   `Integer`/`Float` branch hits `s.parse()` and returns
   `FitsError::InvalidValue` (`ascii/mod.rs:135,152`). Needs both `TNULLn`
   detection and a nullable column representation.

2. 🟡 **`TSCALn`/`TZEROn` scaling not applied to ASCII columns (§7.2.2).**
   `AsciiColumn` carries no scaling and `read_column` returns the raw field value;
   `physical = TZEROn + TSCALn × field` is never computed. (Binary tables
   implement this; ASCII tables do not — an asymmetry.)

3. 🟡 **Implicit decimal point not handled (§7.2.1, deprecated).** For an
   `Fw.d`/`Ew.d`/`Dw.d` field with no explicit `.`, the decimal point is implied
   `d` digits from the right (value × 10⁻ᵈ). `read_column` ignores `decimals` and
   parses the digits as-is (`ascii/mod.rs:150`), so such a (deprecated but legal)
   field is read off by a factor of 10ᵈ.

4. 🟡 **`column_index` is case-sensitive (§7.2.2).** `TTYPEn` is to be compared
   case-insensitively, but `column_index` matches with `== Some(name)`
   (`ascii/mod.rs:101`), so `column_index("ra")` misses a `TTYPE='RA'` column.

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
| 6.3 | `rA` = one string; early `NUL` terminates | `trim_text` strips trailing sp/NUL, no early cut | 🟡 early-NUL not honored |
| 6.3 | `P`/`Q` repeat only 0 or 1 | not validated | 🟢 |
| 6.4 | `physical = TZEROn + TSCALn × stored` (Eq. 7) | `read_column_physical` (`table/mod.rs:314`) | ✅ |
| 6.4 | Not applied to `A`/`L`/`X` | `_ ⇒ NonNumericColumn` (also rejects `C`/`M`) | ✅ (C/M over-rejected) |
| 6.4 | Unsigned `B`/`I`/`J`/`K` via `TZEROn` | `physical()` f64 plane | ✅ values / 🟡 no typed `uN`, u64 precision |
| 6.4 | `TNULLn` matched on **stored** value before Eq. 7 | `scaled_int` checks `tnull` pre-scale (`table/mod.rs:318`) | ✅ |
| 6.4 | Scaling on `P`/`Q` heap values, not descriptor | `read_vla_column` returns raw, no scaling | 🟡 missing |
| 6.5 | `TDIMn` multidimensional cell reshape | not parsed | 🟡 missing |
| 6.6 | `P`/`Q` descriptor `(nelem, offset)`, signed; heap decode | `read_vla_column` (`table/mod.rs:345`) | ✅ |
| 6.6 | Default `THEAP` = main-table size; gap allowed | `heap_offset` default | ✅ (min not validated) |
| 6.6 | `nelem=0` ⇒ no heap data | empty slice | ✅ (garbage offset may error) |
| 6.6 | Span must lie within **heap** (not data unit) | bounds-checked vs whole `bytes` (incl fill) | 🟡 over-permissive |
| 6.7 | `TTYPEn` name, compared case-insensitively | stored; `column_index` case-**sensitive** | 🟡 |
| 6.7 | `TUNITn`, `TSCALn`, `TZEROn`, `TNULLn`, `THEAP` | parsed | ✅ |
| 6.7 | `TDISPn`, `TDIMn`, `TDMINn`/`TDMAXn`, `TLMINn`/`TLMAXn` | not implemented | 🟡 `TDIM` / 🟢 rest |
| impl | `X` bit columns unpacked MSB-first | returned as raw packed `Bytes` | 🟡 not unpacked |
| impl | Column-oriented / SIMD / zero-copy fast path | `read_column` copies via `flatten` | 🟢 perf |

Fixed-width decoding (all 13 type codes, repeat/byte-width including `X` =
⌈bits/8⌉ and the `P`/`Q` descriptor sizes), row-width validation, the
`TSCAL`/`TZERO`/`TNULL` physical plane (null matched pre-scale, `A`/`L`/`X`
rejected), and `P`/`Q` heap decode are all implemented and tested — including
against a real AIPS antenna table. The gaps cluster around column-level features
beyond plain fixed-width decode.

### Gaps

1. 🟡 **`TDIMn` multidimensional cells not implemented (§6.5).** No `TDIM`
   parsing exists; `Column` carries no shape and `read_column` returns a flat,
   row-flattened vector. A `60A` + `TDIM='(5,4,3)'` string array, or any reshaped
   numeric cell, is readable only as raw flat data with the dimensionality lost.

2. 🟡 **VLA heap bounds checked against the whole data unit, not the heap
   (§6.6 + impl note).** `read_vla_column` validates `start + nbytes` with
   `self.bytes.get(start..start+nbytes)` (`table/mod.rs:371`) — `self.bytes` is the
   entire data unit including trailing block fill — instead of against the heap
   extent (`PCOUNT − gap`). A descriptor overrunning the heap into the padding is
   silently accepted and decodes fill bytes as array elements.

3. 🟡 **`X` (bit) columns are not unpacked.** `decode_array` returns `X` as raw
   packed `ColumnData::Bytes` (`table/mod.rs:410`) rather than unpacking bits
   MSB-first into a bit/bool array, and the writer has no `X` encoding
   (`column_code` maps `Bytes`→`B`), so a bit column cannot round-trip as `X`.

4. 🟡 **VLA columns have no scaling/null/physical path (§6.4).**
   `read_vla_column` returns raw heap arrays; `TSCALn`/`TZEROn`/`TNULLn` on the
   heap element type are never applied (there is no `read_vla_column_physical`).

5. 🟡 **`rA` early-NUL termination not honored (§6.3).** `trim_text`
   (`table/mod.rs:435`) strips only *trailing* spaces and NULs, so a cell like
   `AB\0CD` decodes to `"AB\0CD"` instead of being truncated at the first NUL to
   `"AB"`.

6. 🟡 **`column_index` is case-sensitive (§6.7).** Same issue as ASCII tables —
   `TTYPEn` is to be matched case-insensitively but `column_index`
   (`table/mod.rs:278`) uses `== Some(name)`.

7. 🟡 **No native unsigned (`uN`) exposure / `u64` precision loss.** Mirrors the
   §5 image gap: integer `TFORM` + `TZEROn = 2^(n-1)` + `TSCALn = 1` is realized
   only through the `f64` `read_column_physical` plane, with no typed
   `u16`/`u32`/`u64` column and rounding for `u64` values > 2⁵³.

8. 🟡 **VLA (`P`) write is implemented but 32-bit-only.** `write_table` now
   builds the heap and emits `P` array descriptors (`WriteColumn::vla`,
   `writer/mod.rs:79`; round-tripped by `writes_and_reads_back_variable_length_arrays`,
   `writer/tests.rs:54`) — so the once-TODO binary-table VLA write path exists.
   But it always writes 32-bit `1P` descriptors and casts `nelem`/`offset`
   `as i32` (`writer/mod.rs:211`): a heap larger than 2 GiB, or a single row with
   more than `i32::MAX` elements, silently truncates, with no `Q` (64-bit)
   fallback. (Decode handles both `P` and `Q`.)

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
`compute_pole`, matrix inversion) and `wcs/frame.rs` (`Frame`, precession,
frame bias, Galactic + FK4 rotations), behind the `wcs` feature. (Time §9 and
compression §10 from the same reference file are audited separately.)

The reference sets a deliberately low bar — *"a v1 can parse/preserve the
keywords as ordinary header records and add typed support incrementally"* — which
the ordered header model already satisfies for lossless round-trip. The actual
implementation goes far beyond that: a typed pixel↔world transform for **eleven
projections plus four reference frames (ICRS, FK5 at any equinox, Galactic, and
FK4 B1950), all validated against `astropy.wcs` (wcslib) / `SkyCoord` golden
values**. The gaps below are unimplemented advanced features (most flagged TODO
in the module doc), not defects in what exists.

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
| Zenithal `TAN`/`SIN`/`ARC`/`STG`/`ZEA` | `Projection` (`wcs/mod.rs:32`) | ✅ |
| Cylindrical `CAR`/`CEA`/`MER` + pseudo-cyl. `SFL` | `Projection` | ✅ |
| All-sky `AIT`/`MOL` (Hammer-Aitoff, Mollweide) | `Projection` (`wcs/mod.rs:51`) | ✅ |
| `RADESYSa`/`EQUINOXa`; ICRS/FK5/Galactic | `frame.rs` (`matrix`, `wcs/frame.rs:100`) | ✅ |
| `FK4`/`FK4-NO-E` **B1950** (frame rotation + E-terms) | `to_icrs_vec` (`frame.rs:68`), `ETERMS`/`FK4_TO_FK5` (`:112`,`:132`) | ✅ B1950 / 🟡 other equinoxes error |
| Alternate WCS `a ∈ A–Z` | `alt` param | ✅ (untested) |
| `PVi_ma`/`PSi_ma` projection params | — | 🟡 not implemented |
| `CUNITia` (esp. celestial = degrees) | not read; degrees assumed | 🟡 ignored |
| Spectral WCS §8.4 (`FREQ-F2W`, …) | non-celestial ⇒ linear only | 🟡 not implemented |
| BINTABLE column WCS (`TCTYPn`/`iCTYPn`, Table 22) | — | 🟡 not implemented |
| `GAPPT` reference frame | unrecognized → equinox default | 🟡 not implemented |
| `WCSNAMEa`/`CNAMEia`, `CRDERia`/`CSYERia` | — | 🟢 not exposed |
| Conventional `'STOKES'`/`'COMPLEX'` | linear pass-through | ✅ (degenerate) |

### Gaps

1. 🟡 **`PVi_ma`/`PSi_ma` projection parameters not supported.** The transform
   uses parameter-free projection defaults, so slant `SIN` (`PV2_1`/`PV2_2`),
   `CEA` with `λ ≠ 1` (`PV2_1`), `ZPN`, `SZP`, and any `φ₀`/`θ₀` override are
   wrong or unrepresentable. The module doc flags this (`wcs/mod.rs:18`).
   Param-free `SIN`/`CEA`/`AIT`/`MOL` are correct and tested.

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

5. 🟡 **FK4 is supported only at B1950; `GAPPT` is unrecognized.** FK4 (and
   `FK4-NO-E`) at the B1950 equinox is fully transformed — frame rotation
   (`FK4_TO_FK5`, `frame.rs:132`) plus the E-terms of aberration
   (`remove_eterms`/`add_eterms`, `frame.rs:115`,`:121`) — and astropy-checked.
   FK4 at *any other* equinox returns `FitsError::UnsupportedFrame`
   (`frame.rs:71`,`:89`; it would need Newcomb pre-precession to B1950 first).
   `GAPPT` (geocentric apparent place) is not recognized and falls through to the
   equinox-based default frame.

6. 🟢 **Lenient on illegal combinations / unexposed metadata.** `PC`+`CD` both
   present is not rejected (`CD` wins); `CROTA`+`PC` is not rejected (`PC` wins);
   `WCSNAMEa`/`CNAMEia` and `CRDERia`/`CSYERia` are not exposed. (The ICRS↔FK5
   ~25 mas frame bias, formerly omitted, is now applied — `FK5_FROM_ICRS`,
   `frame.rs:171` — so FK5 J2000 matches astropy to ~1e-8°.)

7. 🟢 **FK5 uses the FITS-WCS IAU-1976 precession, not astropy's IAU-2006.**
   `precession_fk5` (`frame.rs:142`) is the Lieske IAU-1976 model (bit-identical
   to `erfa.pmat76`, the FITS-WCS convention); astropy applies the newer IAU-2006
   model to FK5, so the two diverge by ~tens of mas at equinoxes far from J2000
   (~68 mas at J1975). This is a deliberate, documented standard-conformance
   choice, not a defect — and the J2000 frame bias is exact either way.

### Test coverage

Strong and unusually rigorous — golden values come from `astropy.wcs` / astropy
`SkyCoord`, so the formulas (not merely forward/inverse self-consistency) are
checked (`wcs/tests.rs`): `parses_tan_header` (`:24`) + `pixel_to_world_matches_astropy`
(`:36`, six TAN points to 1e-9); `world_to_pixel_inverts_pixel_to_world` (`:54`);
`reference_pixel_maps_to_crval` (`:71`); `sin_projection_matches_astropy` (`:168`);
`legacy_crota_rotation_matches_astropy` (`:195`); `allsky_projections_match_astropy`
(`:222`, `AIT`+`MOL` goldens); `projections_match_astropy` (`:270`,
`STG`/`ZEA`/`CAR`/`CEA`/`MER`/`SFL` goldens, cylindrical `CRVAL` chosen so the
general pole computation runs); `projections_round_trip` (`:250`, all **eleven**
projections project→deproject); a standalone `matrix_inverse_is_correct` (`:154`);
and the frame block — `frame_transforms_match_astropy` (`:79`) now pins ICRS→FK5(J2000),
ICRS→FK5(J1975), and ICRS→Galactic to **1e-8°** (the J2000 frame bias and exact
Galactic matrix), adds an ICRS→FK4(B1950) golden, and asserts FK4 at J1975 →
`UnsupportedFrame`; plus `frame_round_trips` (`:124`) and `frame_parses_from_header`
(`:135`).

Coverage gaps:

- No **alternate-WCS** (`alt = Some('A')`) test, though the code path exists.
- No mixed celestial + non-celestial (`NAXIS ≥ 3`, e.g. a spectral/linear third
  axis) `pixel_to_world` test.
- No explicit **`PCi_j`-matrix** astropy test (only `CD`/`CDELT`+`CROTA`/bare
  `CDELT` are exercised).
- No singular-matrix → `InvalidValue` error test, no `WCSAXES`-vs-`NAXIS` default
  test, and no all-linear (no celestial pair) `Wcs` test.
- `CUNIT`, `PVi_m`, spectral, and table-WCS paths are untested (unimplemented);
  the `FK4-NO-E` (E-term-free) variant is not separately exercised from `FK4`.

---

## §9 — Representations of Time Coordinates (`docs/refs/07-wcs-time-compression.md` §7.2)

Audited code: `time/mod.rs` (`Datetime`, `Epoch`, `TimeScale`, `FitsTime`,
`is_time_ctype`, plus the leap-second / `tdb_minus_tt` / proleptic-Gregorian
helpers) and `time/tests.rs`, behind the `time` feature.

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
The gaps are the metadata-only / table-context / nice-to-have parts of §9, plus
two correctness bugs (the `TIMEUNIT` table and the split-reference precedence).

### Conformance matrix

| Doc § | Requirement | Code | Status |
|---|---|---|---|
| 9.2.1 | `TIMESYS` (default `UTC`); other values allowed | `FitsTime::from_header` (`time/mod.rs:388`) | ✅ |
| 9.2.1 | Table 30 scales (`TAI/TT/TCG/TDB/TCB/UTC/UT1/GPS/…`) | `TimeScale::parse` (`time/mod.rs:215`) | ✅ |
| 9.2.1 | Aliases `TDT`/`ET`→`TT`, `IAT`→`TAI` | `parse` arms (`time/mod.rs:215`) | ✅ |
| 9.2.1 | Realization suffix `TT(TAI)`, `UTC(NIST)` | matched whole-string ⇒ falls to `Local` | 🔴 not stripped |
| 9.2.1 | `GMT` (continuous with UTC) | no arm ⇒ `Local` | 🟡 should alias `Utc` |
| 9.2.1 | TT-pivot lattice; `TT↔TCG` (`L_G`), `TDB↔TCB` (`L_B`) | `to_tt`/`from_tt` (`time/mod.rs:245`,`:269`) | ✅ |
| 9.2.1 | TDB periodic series | `tdb_minus_tt` (`time/mod.rs:297`) | ✅ (no `TDB_0`) |
| 9.2.1 | `UT1` via ΔUT1; `LOCAL` pass-through | `convert_dut1` (`time/mod.rs:237`) | ✅ caller ΔUT1 / 🟡 no bundled table |
| 9.1.1 | ISO-8601 `[±C]CCYY-MM-DD[Thh:mm:ss[.s…]]`; parts optional | `Datetime::parse` (`time/mod.rs:45`) | ✅ |
| 9.1.1 | Leading zeros **must not** be omitted | integer parse accepts `2024-1-1` | 🟡 not enforced |
| 9.1.1 | **No** timezone designator (`Z` forbidden) | rejected only incidentally (f64 parse) | 🟡 not explicit |
| 9.1.1 | Seconds `00–60` UTC (leap), `00–59` else | `0.0..61.0` for all scales (`time/mod.rs:93`) | 🟡 scale-agnostic |
| 9.1.2 | Julian/Besselian epoch strings → JD | `Epoch::to_jd` (`time/mod.rs:176`) | ✅ |
| 9.1.2/9.5 | `JEPOCH` (TDB) / `BEPOCH` (ET) **keywords** | `Epoch` type not wired to header | 🟡 not read |
| 9.2.2 | Reference in ISO / JD / MJD; defaults | `reference_mjd` (`time/mod.rs:454`) | ✅ |
| 9.2.2 | `[M]JDREFI`+`[M]JDREFF` integer+fraction split | summed (`time/mod.rs:459`) | ✅ |
| 9.2.2 | **Split takes precedence over single** when all present | single `MJDREF` returned first (`time/mod.rs:455`) | 🔴 wrong precedence |
| 9.2.2 | Kind precedence `MJDREF > JDREF > DATEREF` | checked in that order (`time/mod.rs:455`,`:462`,`:469`) | ✅ |
| 9.3 | `TIMEUNIT` (default `s`); Table 34 units | `unit_seconds` (`time/mod.rs:404`) | 🔴 only `s`/`d`/`a`; `min`/`h`/`cy`→`1.0` |
| 9.2.3 | `TREFPOS` (default `TOPOCENTER`) + Table 31; `TRPOSn` | stored verbatim (`time/mod.rs:394`) | 🟡 no default/validation |
| 9.2.4 | `TREFDIR`/`TRDIRn` reference direction | — | 🟢 not implemented |
| 9.2.5 | `PLEPHEM` (default `DE405`) | — | 🟢 not implemented |
| 9.4.1 | `TIMEOFFS` added to reference time | not read; `relative_to_mjd` (`time/mod.rs:414`) omits it | 🟡 not applied |
| 9.4.2 | `TIMEDEL` / `TIMEPIXR` binning | — | 🟡 not implemented |
| 9.4.3 | `TIMSYER` / `TIMRDER` time errors | — | 🟢 not implemented |
| 9.5 | `DATE-OBS` / `MJD-OBS` start time | `obs_mjd` (`time/mod.rs:419`) | ✅ |
| 9.5 | `DATE-BEG`/`-END`, `MJD-BEG`/`-END` typed | — | 🟢 raw cards only |
| 9.5 | `TSTART`/`TSTOP` (rel. to `[M]JDREF`, in `TIMEUNIT`) | `relative_to_mjd` (`time/mod.rs:414`) | ✅ / 🟡 ignores `TIMEOFFS` |
| 9.6 | `CTYPEi='TIME'` image time axis → world time | `time_axis_mjd` (`time/mod.rs:433`) | ✅ |
| 9.6 | `'PHASE'`/`'TIMELAG'`/`'FREQUENCY'`; `CZPHSia`/`CPERIia` | `is_time_ctype` recognizes `'TIME'`+scales only (`time/mod.rs:447`) | 🟢 not implemented |
| 9.7 | `XPOSURE` / `TELAPSE` durations; GTI `START`/`STOP` | — | 🟢 raw cards only |

The normative computational core — the Table-30 scale set with the canonical
aliases, the TT-pivot conversion lattice including the defining `L_G`/`L_B`
relations, ISO-8601↔JD/MJD calendar math, the `[M]JDREF`/`JDREF`/`DATEREF`
resolution with kind-precedence, J/B epochs, and a working `CTYPEi='TIME'` axis —
is implemented and astropy-validated. The gaps cluster in metadata semantics,
table-only constructs, the non-`TIME` time axes, and two outright bugs.

### Gaps

1. 🔴 **`TIMEUNIT` table is incomplete — `min`/`h`/`cy`/`ta`/`Ba` silently scale
   as seconds (§9.3, Table 34).** `unit_seconds` matches only `d`/`day`,
   `a`/`yr`/`y`, and falls through to `1.0` otherwise (`time/mod.rs:404`). Table 34
   also defines `'min'` (60 s), `'h'` (3600 s), `'cy'` (Julian century), and the
   discouraged `'ta'`/`'Ba'`. `TIMEUNIT='min'` therefore makes `relative_to_mjd` /
   `time_axis_mjd` off by 60×, `'h'` by 3600×, with no error.

2. 🔴 **Split reference parts do not take precedence over the single value
   (§9.2.2).** The standard: *"If [M]JDREF and both [M]JDREFI and [M]JDREFF are
   present, the integer and fractional values shall have precedence over the
   single value."* `reference_mjd` returns `MJDREF` as soon as it is present
   (`time/mod.rs:455`), *before* looking at `MJDREFI`/`MJDREFF` (`:458`) — the
   reverse of the rule when all three are present. (The "single wins if present
   with only one part" sub-rule is met only by accident.) Same on the
   `JDREF`/`JDREFI`/`JDREFF` branch (`time/mod.rs:462`).

3. 🔴 **Time-scale realization suffix not stripped (§9.2.1).** High-precision
   values append a realization — `'TT(TAI)'`, `'UTC(NIST)'`. `TimeScale::parse`
   matches the whole upper-cased string (`time/mod.rs:215`), so `"TT(TAI)"` matches
   no arm and falls to `TimeScale::Local` — a recognized scale misread as an
   unconvertible local clock. The suffix must be split off before matching.

4. 🟡 **`GMT` maps to `LOCAL` instead of `UTC` (§9.2.1, Table 30).** `GMT` is a
   recognized value (continuous with UTC), but `TimeScale::parse` has no `GMT` arm
   so it falls to `Local` (`time/mod.rs:225`) and is treated as unconvertible.

5. 🟡 **`TIMEOFFS` (§9.4.1) not applied.** A bulk clock correction added to the
   reference time. `relative_to_mjd` (`time/mod.rs:414`) omits the `TIMEOFFS` term,
   so `TSTART`/`TSTOP` and table time-pixel values resolve to the wrong absolute
   MJD when an offset is present.

6. 🟡 **ISO-8601 syntax is lenient (§9.1.1).** `Datetime::parse` uses integer
   `.parse()` per field (`time/mod.rs:45`), so `'2024-1-1'` (leading zeros omitted,
   forbidden) is accepted; the forbidden `Z` suffix is rejected only incidentally
   (it breaks the seconds `f64` parse). The leap-second range `0.0..61.0`
   (`time/mod.rs:93`) is applied in every scale, whereas §9.1.1 permits second 60
   only in UTC.

7. 🟡 **Julian/Besselian epoch *keywords* are not read (§9.5).** `Epoch` parses
   `'J2000.0'`/`'B1950.0'` strings and computes their JD, but
   `FitsTime::from_header` never reads `JEPOCH` (implied TDB) or `BEPOCH` (implied
   ET), nor attaches the implied scales.

8. 🟡 **`UT1`/ΔUT1 are caller-supplied; no bundled IERS ΔUT1 table.**
   `TimeScale::convert` treats `UT1` as `UTC` (ΔUT1 = 0) unless the caller routes
   through `convert_dut1` with an external ΔUT1 (`time/mod.rs:231`,`:237`). The
   module doc states this.

9. 🟢 **Metadata-only / table-context §9 features unimplemented:** `TREFPOS`
   stored without default/validation/`TRPOSn` (`time/mod.rs:394`); no `TREFDIR`
   (§9.2.4), `PLEPHEM` (§9.2.5), `OBSGEO-*` location, `TIMEDEL`/`TIMEPIXR`
   binning (§9.4.2), `TIMSYER`/`TIMRDER` errors (§9.4.3); only `DATE-OBS`/`MJD-OBS`
   (no typed `DATE-BEG`/`-END`/`MJD-BEG`/`-END`); no `XPOSURE`/`TELAPSE` durations
   or GTI `START`/`STOP` (§9.7); no `'PHASE'`/`'TIMELAG'`/`'FREQUENCY'` axes or
   `CZPHSia`/`CPERIia` (§9.6) — `is_time_ctype` recognizes only `'TIME'` and
   Table-30 scale names (`time/mod.rs:447`). All remain readable as raw cards.

10. 🟢 **`TDB_0` constant offset omitted.** The §9.2.1 TCB→TDB relation includes
   `TDB_0 = −6.55 × 10⁻⁵ s`; the periodic `tdb_minus_tt` series (`time/mod.rs:297`)
   omits the constant term — below the astropy test tolerance, but not the literal
   defining equation.

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

- **The two 🔴 bugs are untested** (a test would fail today): no header with
  *both* `MJDREF` and `MJDREFI`/`MJDREFF` to expose the split-precedence inversion
  (gap #2 — the split is only ever tested *alone*), and no `TIMEUNIT='min'`/`'h'`/
  `'cy'` test to expose the silent seconds-fallback (gap #1).
- No `TimeScale::parse` test at all — the Table-30 string→variant map, the
  `TDT`/`ET`/`IAT` aliases, the `'TT(TAI)'` realization suffix (gap #3), the `GMT`
  value (gap #4), and the unknown→`Local` fallback are all unexercised.
- No signed-5-digit-year, leading-zero-omission (gap #6), or explicit `Z`-suffix
  rejection test; no `JDREF`/`DATEREF` resolution or kind-precedence test (only
  `MJDREF` and the split are exercised).
- Everything in gaps #5/#7/#9 (TIMEOFFS, epoch keywords, TREFPOS validation,
  binning, durations, GTI, PHASE/TIMELAG/FREQUENCY) is untested because
  unimplemented.

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
| 10.1.3 | `ZBLANK` (column or keyword); column wins | keyword only (`compress/mod.rs`) | 🟡 keyword only |
| 10.2 | `physical = ZZERO + ZSCALE × I` (Eq. 12) / dithered Eq. 14 | `dequantize` (`compress/quantize.rs:97`) | ✅ |
| 10.2.1 | `SUBTRACTIVE_DITHER_2`: exact `0.0` ↔ `ZERO_VALUE` | `ZERO_VALUE` (`compress/quantize.rs:19`) | ✅ |
| 10.2.1 / App. I | Park–Miller PRNG, 10000th seed = 1043618065 | `random_values` (`compress/quantize.rs:40`,`:53`) | ✅ |
| 10.4.1 | `RICE_1` (integer only) | `rice_decode`/`rice_encode` (`compress/rice.rs:26`,`:77`) | ✅ |
| 10.4.2 | `GZIP_1` DEFLATE; `GZIP_2` MSB-first shuffle | `compress/gzip.rs` | ✅ |
| 10.4.3 | `PLIO_1` IRAF mask RLE (ints 0–2²⁴) | `compress/plio.rs` | ✅ |
| 10.4.4 | `HCOMPRESS_1` 2-D; `SCALE` param; `SMOOTH` decode | `compress/hcompress.rs`; `hcompress_smooth` (`compress/mod.rs:657`) | ✅ decode / 🟡 no `SMOOTH` encode |
| 10.4 | `NOCOMPRESS` stored uncompressed | `other` arm ⇒ `UnsupportedCompression` | 🟢 not implemented |
| 10.3 | `ZTABLE = T`; one row per row-tile, `1QB` columns | `uncompress_table`/`compress_table` (`compress/table.rs:234`,`:158`) | ✅ |
| 10.3.1 | `ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT`/`ZFORMn`/`ZCTYPn`/`ZTILELEN` | parsed + written (`compress/table.rs`) | ✅ |
| 10.3.5 | Tables: lossless `RICE_1`/`GZIP_1`/`GZIP_2` only | `Algo::parse` rejects others (`compress/table.rs:43`) | ✅ (no `NOCOMPRESS`) |
| 10.3.4 | `ZTHEAP`/`ZHECKSUM`/`ZDATASUM` verbatim-copy (table) | only `ZPCOUNT` preserved | 🟢 not implemented |
| 10.3.6 | VLA columns in a compressed table | rejected (`col_meta`, `compress/table.rs:122`) | 🟡 rejected, not compressed |

### Gaps

1. 🟢 **`NOCOMPRESS` is not supported (§10.4, Table 36).** It appears in neither
   the decode dispatch (`decode_tile_cell`'s `other` arm → `UnsupportedCompression`,
   `compress/mod.rs:629`) nor the encoders nor `Algo::parse` for tables
   (`compress/table.rs:43`). A `ZCMPTYPE='NOCOMPRESS'` HDU (raw pixel bytes per
   tile) fails to read.

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

6. 🟡 **Image encoders write only 32-bit `1P` descriptors, and `ZBLANK` is read
   only as a keyword (§10.1.3).** Both image encoders emit `1P` (`i32`)
   descriptors; the standard requires `1Q` (64-bit) once the heap exceeds ~2.1 GB,
   which this writer cannot produce (decode of `1Q` works). Separately,
   `decompress_image` reads only the `ZBLANK` *keyword*, never the legal per-tile
   `ZBLANK` *column* (which the standard says wins over the keyword).

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

2. 🟡 **Long `HIERARCH` string values can truncate on write.** `render_records`
   only splits a `CardKind::Value` text into a `CONTINUE` chain; a `HIERARCH`
   card renders through `render`, where `write_at` silently clamps to 80 bytes
   (`card/mod.rs:478`). A `HIERARCH` key plus a long string value that overflows
   the card is truncated rather than continued. (Edge case; mirrors the §4
   large-real overflow.)

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
