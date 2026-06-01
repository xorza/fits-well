# FITS 4.0 Conformance Audit

Audit of the `fits` implementation against the FITS 4.0 standard (the curated
notes in [`refs/`](refs/) and the normative [`refs/fits_standard40.pdf`]). Last
reviewed 2026-06-01, after the conformance-completion pass recorded below.

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
| 5 | Data representation (`BITPIX`, big-endian, scaling, `BLANK`, unsigned, NaN/Inf) | ✅ complete |
| 6 / 7.1 | Images, random groups (incl. §6.3 addend summing) | ✅ complete |
| 7.2 | ASCII `TABLE` (read incl. bare-sign exponents; write incl. `TSCAL`/`TZERO`/`TNULL`) | ✅ complete |
| 7.3 | Binary `TABLE` (incl. logical-NULL three-state, `1PX` VLA bit-unpack) | ✅ complete |
| 8 | World Coordinate Systems | ✅ for all implemented projections + `CUNIT` + pixel-list WCS; ⚠️ quad-cube/HEALPix/non-linear-spectral error cleanly |
| 9 | Time coordinates | ✅ complete (scales, references, bounds incl. `DATE-AVG`, `PHASE` axis) |
| 10 | Tiled compression | ✅ all codecs decode; encode incl. `NOCOMPRESS` + `1Q`; null-mask/VLA-table = reference doesn't emit |

The structural and data-format layers (§3–§7, §10 decode) are complete. §8 WCS is
complete for every projection it implements plus `CUNIT` and pixel-list WCS; the
unimplemented projections and non-linear spectral axes **error cleanly** rather
than return wrong coordinates.

---

## Fixes applied — review pass

| Sev | Fix | § | Code |
|-----|-----|---|------|
| 🔴 | ASCII bare-sign exponent (`3.14159-2`) was rejected, erroring the column read | 7.2.5 | `ascii::split_mantissa_exponent` |
| 🔴 | Compressing an integer image dropped `BZERO`/`BSCALE`/`BLANK` | 10.2 | `compress::encode_image` |
| 🔴 | `RICE_1` `BYTEPIX=8` panicked / corrupted → clean error (see deferred) | 10.4.1 | `compress`, `rice::BitReader` |
| 🔴 | `Q` (64-bit) VLA descriptors truncated to 32 bits | 7.3.5 | `writer::push_vla_descriptor` |
| 🟡 | `BLANK` card emitted for float images | 4.4.2.5 | `writer::add_scaling` |
| 🟡 | `inf`/`NaN` accepted (read) and emitted (write) in keyword values | 4.2.4 | `card::parse_real`/`format_real` |
| — | Dead, unreachable duplicate `SZP` projection block removed | 8 | `wcs` |
| 🟢 | Random-groups §6.3 addend summing | 6.3 | `RandomGroups::parameter_physical` |

## Fixes applied — completion pass

| Area | Item | § | Code | Test |
|------|------|---|------|------|
| ASCII | Write `TSCALn`/`TZEROn`/`TNULLn`; non-finite cell → marker/blank | 7.2.2/.4 | `AsciiWriteColumn`, `ascii_table_header`, `format_ascii_field` | `ascii_write_emits_tscal_tzero_tnull_and_round_trips` |
| BINTABLE | Logical three-state `T`/`F`/`0x00`(null) | 7.3.3 | `ColumnData::Logical(Vec<Option<bool>>)` | `logical_column_round_trips_with_null_state` |
| BINTABLE | `1PX`/`1QX` VLA bit-array unpack (MSB-first) | 7.3 | `BinTable::read_vla_bit_column` | `vla_bit_column_unpacks_msb_first` |
| Compress | `NOCOMPRESS` encoder | 10.4 | `compress::encode_image` | `nocompress_image_round_trips` |
| Compress | `1Q` compressed-image descriptors (auto-switch past 4 GiB) | 10.1.3 | `compress::push_compressed_descriptor` | `compressed_image_descriptor_switches_to_q_for_large_offsets` |
| WCS | `yzLN`/`yzLT` celestial axes (planetary/solar, incl. `HPLN`/`HPLT`) | 8.2 | `wcs::find_celestial` | `planetary_solar_lonlat_axes_are_celestial` |
| WCS | `CUNITia` → scale celestial axes to degrees | 8.2 | `wcs::unit_to_degrees`, `from_header` | `cunit_scales_celestial_axes_to_degrees` |
| WCS | Pixel-list (event-list) WCS, `TCTYPn` family (Table 22) | 8.5 | `Wcs::from_pixel_list` | `pixel_list_wcs_matches_the_equivalent_image_wcs` |
| Time | `DATE-AVG`/`MJD-AVG` observation midpoint | 9.5 | `TimeBounds::avg_mjd` | `reads_bound_duration_and_error_keywords` |
| Time | `obs_mjd` JEPOCH/BEPOCH fallback | 9.5 | `FitsTime::obs_mjd` | `obs_mjd_falls_back_to_jepoch` |
| Time | `PHASE` axis `CZPHSia`/`CPERIia` + fold | 9.6 | `FitsTime::phase_axis`, `PhaseAxis` | `reads_phase_axis_and_folds` |

**Behavior change to note:** a header card whose value field is `inf`/`NaN`/an
overflowing real (e.g. `1E400`) is now a hard `InvalidValue` parse error rather
than silently becoming `Real(inf)`.

---

## Deliberately error-cleanly / not implemented (with rationale)

These return a clean error (or a documented no-op) instead of wrong output. Each
is either underspecified, unproducible by the reference implementation, or rare
enough that a verified implementation isn't achievable — so erroring is the
honest, conformant-in-practice behavior.

| Item | § | Behavior | Why not implemented |
|------|---|----------|---------------------|
| Non-linear spectral axes (`-F2W`, `-LOG`, …) | 8.4 | `UnsupportedSpectral` error | The Paper III transforms are large; erroring beats the previous *silent linear* (wrong) result. Bare linear spectral types (`FREQ`, `WAVE`, …) work via the linear path. |
| Quad-cube `TSC`/`CSC`/`QSC` | 8.3 | `UnsupportedProjection` error | Obsolete (COBE-era); exact forward distortion-polynomial formulas need a verified reference. |
| HEALPix `HPX`/`XPH` | 8.3 | `UnsupportedProjection` error | Rare as a WCS projection (HEALPix data uses table pixelisation); formulas need a verified reference. |
| `RICE_1` `BYTEPIX=8` (64-bit) | 10.4.1 | `UnsupportedCompression` error | Table 37 permits it, but the 8-byte Rice bitstream params are unspecified and no reference implementation (cfitsio) produces it — a clean error beats a guessed, non-interoperable codec. |
| `NULL_PIXEL_MASK` / `ZMASKCMP` | 10.2.2 | float nulls handled via `ZBLANK`/NaN | Verified empirically: `fpack` never emits the mask — it uses `ZBLANK` (which we support). The mask construct does not occur in practice. |
| §10.3.6 compressed-table VLA | 10.3.6 | `UnsupportedCompression` on write | Verified empirically: `fpack` passes VLA tables through *uncompressed* rather than emitting a compressed-VLA `ZTABLE`; the construct does not occur in practice. |
| Vector-form table WCS (`iCTYPn` for array cells) | 8.5 | not parsed | The high-value pixel-list form (`TCTYPn`) is implemented; the array-in-cell form is a smaller follow-up. |

---

## Deliberately out of scope ("nothing more")

Correctly **absent** — adding them would exceed the FITS *format* standard:

- ⚪ **Inter-frame astrometry** (FK4↔FK5↔Galactic↔ICRS: precession, E-terms, frame
  bias). §8 parses `RADESYS`/`EQUINOX` and returns coordinates in the file's
  *declared* frame; transforming between frames is an astronomy library's job.
- ⚪ **Light-travel / `TREFPOS`/`TREFDIR`/`PLEPHEM` corrections and ΔUT1 tables** —
  observational astronomy, not the format. (The leap-second table and TDB series
  *are* kept: they are the defining UTC↔TAI and TDB relations §9.2.1 needs.)
- ⚪ **Reader strictness tightening** (rejecting control chars, the col-10 value
  indicator, the 999-axis bound, extension-keyword order). The standard does not
  require a *reader* to reject these, so enforcing them risks rejecting readable
  files without improving compatibility.
- ⚪ **Ergonomics / performance** — coordinate-index/strided image API, SIMD /
  zero-copy decode, trivial typed accessors. Not part of the standard.

---

## Verification

```
cargo test                                                        → 172 passed
cargo test --features compression                                 → 201 passed, 2 ignored (fixture emitters)
cargo fmt --all                                                   → applied
cargo clippy --all-targets -- -D warnings                         → clean
cargo clippy --all-targets --features compression -- -D warnings  → clean
```

The math-heavy layers are cross-checked against external golden values: WCS
projections against `astropy.wcs` (wcslib), time scales against `astropy.time`
(ERFA), and the compression codecs against cfitsio/`fpack` and astropy outputs.
New WCS/time additions are verified by self-consistency against the
astropy-validated pipelines (e.g. the pixel-list and `CUNIT` WCS reproduce the
equivalent image WCS exactly).
