# FITS 4.0 Conformance Audit

Audit of the `fits` implementation against the FITS 4.0 standard (the curated
notes in [`refs/`](refs/) and the normative [`refs/fits_standard40.pdf`]). Last
reviewed 2026-06-01, after the fix pass recorded below.

**The bar — "full compatibility, nothing more":** read every conforming FITS 4.0
file and expose its data *and* coordinate semantics correctly, and write only
conforming output that round-trips. Computing things the *format* standard does
not define (inter-frame astrometry, light-travel/ephemeris corrections) is
deliberately out of scope — see the last section.

Severity: 🔴 correctness (rejects valid files / wrong output) · 🟡 lenient or
write-side non-conformance · 🟢 missing standard feature · ⚪ out of scope.

---

## Status by section

| § | Area | Status |
|---|------|--------|
| 3 | File structure, 2880 blocking, padding, HDU sizing, special/trailing records | ✅ complete |
| 4 | Header & keyword records; CONTINUE / CHECKSUM / HIERARCH conventions | ✅ complete |
| 5 | Data representation (all `BITPIX`, big-endian, `BSCALE`/`BZERO`, `BLANK`, unsigned, NaN/Inf bit-exact) | ✅ complete |
| 6 / 7.1 | Images, random groups (incl. §6.3 addend summing) | ✅ complete |
| 7.2 | ASCII `TABLE` | ✅ read complete · 🟢 write can't emit `TNULL`/`TSCAL`/`TZERO` |
| 7.3 | Binary `TABLE` | ✅ complete · 🟢 logical-null state, `1PX` VLA bit-unpack |
| 8 | World Coordinate Systems | ⚠️ **partial** — 23/28 projections; no `CUNIT`, spectral, table WCS |
| 9 | Time coordinates | ✅ core complete (astropy-validated) · 🟢 non-`TIME` axis value computation |
| 10 | Tiled compression | ✅ all codecs **decode**; encode partial (see roadmap) |

The data-format layers (§3–§7 structure, §5 representation, §10 codec decode) are
complete. The remaining work to reach full conformance is concentrated in **§8
(WCS)** plus a handful of small §7/§9/§10 round-trip and fidelity items.

---

## Fixes applied (2026-06-01)

Each fix carries a regression test; the full gate passes (see Verification).

| # | Sev | Fix | § | Code | Test |
|---|-----|-----|---|------|------|
| 1 | 🔴 | ASCII bare-sign exponent (`3.14159-2` = 0.0314159) was rejected, erroring the column read | 7.2.5 | `ascii::parse_ascii_float` + new `split_mantissa_exponent` | `signed_exponent_without_letter_parses_as_fortran_real`, `reads_a_column_with_a_bare_sign_exponent_field` |
| 2 | 🔴 | Compressing an integer image silently dropped `BZERO`/`BSCALE`/`BLANK` | 10.2 | `compress::encode_image` | `integer_image_compression_preserves_bscale_bzero_and_blank` |
| 3 | 🔴 | `RICE_1` with `BYTEPIX=8` panicked (debug) / corrupted (release) on encode *and* decode | 10.4.1 | `compress::encode_image`/`decode_tile_cell` guards; `rice::BitReader::read` mask | `rice_rejects_64_bit_pixels` |
| 4 | 🔴 | `Q` (64-bit) VLA descriptors truncated count/offset to 32 bits | 7.3.5 | `writer` `descs: u64` + new `push_vla_descriptor` | `vla_descriptor_q_form_carries_full_64_bit_count_and_offset` |
| 5 | 🟡 | `BLANK` card emitted for float images (illegal — positive `BITPIX` only) | 4.4.2.5 | `writer::add_scaling` (gated on `is_integer()`) | `blank_is_emitted_only_for_integer_images` |
| 6 | 🟡 | `inf`/`NaN` accepted in a keyword value (read) and emitted (write) | 4.2.4 | `card::parse_real` rejects non-finite; `card::format_real` asserts | `non_finite_reals_are_rejected_on_read`, `rendering_a_non_finite_real_panics` |
| 7 | — | Dead, unreachable duplicate `SZP` block (a second, un-validated formula) | 8 | removed from `wcs` deproject | existing SZP goldens |
| 8 | 🟢 | Random-groups §6.3 addend summing (split parameters sharing a `PTYPEn`) | 6.3 | new `RandomGroups::parameter_physical` | `parameter_physical_sums_addends_sharing_a_ptype`, extended real-file test |

**Behavior change to note (fix #6):** a header card whose value field is
`inf`/`NaN`/an overflowing real (e.g. `1E400`) is now a hard `InvalidValue` parse
error rather than silently becoming `Real(inf)`. Real archives never contain
these, and it matches how `Card::parse` already rejects other malformed scalars.

---

## Remaining work for full conformance

### Required — coordinate semantics (§8 WCS, the bulk)

| Item | § | Current behavior | Effort |
|------|---|------------------|--------|
| Quad-cube projections `TSC`/`CSC`/`QSC` | 8.3 | clean error (`UnsupportedProjection`) | M |
| HEALPix projections `HPX`/`XPH` | 8.3 | clean error | M |
| **Spectral WCS** (`FREQ-F2W`, `WAVE-…`, `VELO-…`, `…-LOG`, …) | 8.4 | ⚠️ **silently linear** — `pixel_to_world` applies `CRVAL + intermediate`, no error | **L** |
| Table / pixel-list WCS (`TCTYPn`/`iCTYPn`/`TCRPXn`…, Table 22) | 8.5 | unparsed (image-header WCS only) | M–L |
| `CUNITia` axis units | 8 | ignored; non-degree axes mis-scaled | M |
| `yzLN`/`yzLT` celestial axes (planetary/solar, incl. `HPLN`/`HPLT`) | 8.2 (`may`) | silently linear | S |

> Cheap interim safety for spectral: route nonlinear spectral algorithm codes
> through the same "unsupported" error as quad-cube/HEALPix so they fail loudly
> instead of returning wrong coordinates, until the §8.4 math lands.

### Required — coordinate semantics (§9 time)

| Item | § | Current behavior | Effort |
|------|---|------------------|--------|
| `PHASE`/`TIMELAG`/`FREQUENCY` axis value computation (`CZPHSia`/`CPERIia`) | 9.6 | axis *recognized*; only `TIME` resolves to a value | S–M |

### Required — data-format round-trip / fidelity (§7, §10)

| Item | § | Current behavior | Effort |
|------|---|------------------|--------|
| `RICE_1` `BYTEPIX=8` (64-bit) — a permitted value (Table 37) | 10.4.1 | clean error (today's safety fix); needs real support | M (u128 decode accumulator + `output_nbits` rework) |
| `NULL_PIXEL_MASK` / `ZMASKCMP` lossy null mask | 10.2.2 | unhandled — lossy image's undefined pixels read as garbage; lossy write loses blank locations | M |
| Compressed-table VLA columns | 10.3.6 | rejected cleanly | M |
| `1Q` image-tile descriptors (write) | 10.1.3 | only `1P` written → can't emit a >2 GB-heap compressed image (decode of `1Q` works) | S |
| Binary-table logical `NULL` (`0x00`) state | 7.3.3 | read as `false` — the three-state `T`/`F`/null collapses | S (`Option<bool>` / null mask) |
| `1PX` bit-array VLA unpacking | 7.3 | heap bit-arrays returned as packed bytes (fixed `X` columns *do* unpack) | S |
| ASCII `TNULLn`/`TSCALn`/`TZEROn` write | 7.2 | reader supports; writer can't emit them (round-trip asymmetry) | S |

### Optional — `should`/`may`, skippable and still conformant

- Verbatim-copy reconstruction keywords (`ZSIMPLE`/`ZTENSION`/`ZPCOUNT`/`ZHECKSUM`/`ZDATASUM`, §10.1.2 — `should`): only for reconstructing a byte-identical original HDU.
- `NOCOMPRESS` encoder (§10.4): never *required* to leave a tile uncompressed.
- `DATE-AVG`/`MJD-AVG` surfacing; JEPOCH/BEPOCH fallback for `obs_mjd` (§9.5): minor semantics.

---

## Deliberately out of scope ("nothing more")

Correctly **absent** — adding them would exceed the FITS *format* standard:

- ⚪ **Inter-frame astrometry** (FK4↔FK5↔Galactic↔ICRS: precession, E-terms, frame
  bias). §8 parses `RADESYS`/`EQUINOX` and returns coordinates in the file's
  *declared* frame; transforming between frames is an astronomy library's job
  (astropy `SkyCoord`, ERFA).
- ⚪ **Light-travel / `TREFPOS`/`TREFDIR`/`PLEPHEM` corrections and ΔUT1 tables** —
  observational astronomy, not the format. (The leap-second table and TDB series
  *are* kept: they are the defining UTC↔TAI and TDB relations §9.2.1 needs to read
  a stated time, correctly bounded — no bundled observational/forecast data.)
- ⚪ **Reader strictness tightening** — rejecting control chars, the column-10
  value indicator, the 999-axis / `TFIELDS≤999` bound, or extension-keyword order.
  The standard does not require a *reader* to reject these, so enforcing them adds
  nothing to compatibility and risks rejecting readable files.
- ⚪ **Ergonomics / performance** — coordinate-index/strided image API, SIMD /
  zero-copy decode, trivial typed accessors (`DATAMIN`/`TLMINn`/`WCSNAMEa`…). Not
  part of the standard.

---

## Verification

```
cargo test                                                        → 163 passed
cargo test --features compression                                 → 190 passed, 2 ignored (fixture emitters)
cargo fmt --all                                                   → applied
cargo clippy --all-targets -- -D warnings                         → clean
cargo clippy --all-targets --features compression -- -D warnings  → clean
```

The math-heavy layers are cross-checked against external golden values: WCS
projections against `astropy.wcs` (wcslib), time scales against `astropy.time`
(ERFA), and the compression codecs against cfitsio/`fpack` and astropy outputs.
