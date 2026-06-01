# FITS 4.0 Conformance Audit

Audit of the `fits` implementation against the FITS 4.0 standard (the curated
notes in [`refs/`](refs/) and the normative [`refs/fits_standard40.pdf`]).

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
| 8 | World Coordinate Systems | ✅ all implemented projections + `CUNIT` + table WCS (pixel-list & vector-cell); quad-cube/HEALPix/non-linear-spectral axes decode through the linear stage and are flagged in `unsupported_axes` (read, never silent-wrong, never fail) |
| 9 | Time coordinates | ✅ complete (scales, references, bounds incl. `DATE-AVG`, `PHASE` axis) |
| 10 | Tiled compression | ✅ all codecs decode; encode incl. `NOCOMPRESS` + `1Q`; null-mask/VLA-table = reference doesn't emit |

The structural and data-format layers (§3–§7, §10 decode) are complete. §8 WCS is
complete for every projection it implements plus `CUNIT` and pixel-list WCS; the
unimplemented projections and non-linear spectral axes **error cleanly** rather
than return wrong coordinates.

---

## Read but not fully decoded (with rationale)

**Every conforming file reads.** The data unit and all header keywords are
accessible regardless of these features — the data readers (`read_image`/
`read_table`/…) never consult WCS, projection, or spectral keywords. The items
below are the only ones whose highest *semantic* layer is not fully evaluated;
none produces a silent wrong result, and none fails the whole read.

| Item | § | Behavior | Why not fully decoded |
|------|---|----------|-----------------------|
| Non-linear spectral axes (`-F2W`, `-LOG`, …) | 8.4 | axis decoded through the **linear stage** → intermediate world coordinate, listed in `Wcs::unsupported_axes`; all other axes (incl. the celestial pair) decode normally | Paper III transforms are large; the linear-stage value is a correct *partial* result, and the flag means it's never mistaken for fully decoded. A bare linear type (`FREQ`, `WAVE`, …) is fully decoded. |
| Quad-cube `TSC`/`CSC`/`QSC`, HEALPix `HPX`/`XPH` | 8.3 | celestial axes decoded through the linear stage → intermediate world coordinate, flagged in `Wcs::unsupported_axes` | Obsolete / rare; exact projection formulas need a verified reference. The linear stage (matrix → intermediate world) is still exact. |
| Conic (`COP`/`COE`/`COD`/`COO`) with its mandatory `PVi_1` (θ_a) absent or 0 | 8.3 | celestial axes decoded through the linear stage → intermediate world coordinate, flagged in `Wcs::unsupported_axes` | θ_a = 0 is a degenerate cone (`1/tan 0`); rather than return NaN, the axes pass through the linear stage and are flagged. A conic *with* a valid `PVi_1` is fully decoded — and `BON` at θ₁ = 0 is *not* degenerate (it is the sinusoidal `SFL`, decoded as such). |
| `RICE_1` `BYTEPIX=8` (64-bit) | 10.4.1 | `read_compressed_image` errors; the raw compressed `BINTABLE` still reads via `read_table` | Table 37 permits it, but the 8-byte Rice bitstream params are unspecified and no reference (cfitsio) produces it — a guessed, non-interoperable codec would be worse. |
| `NULL_PIXEL_MASK` / `ZMASKCMP` | 10.2.2 | float nulls handled via `ZBLANK`/NaN | Verified empirically: `fpack` never emits the mask — it uses `ZBLANK` (which we support). The mask construct does not occur in practice. |
| §10.3.6 compressed-table VLA | 10.3.6 | rejected on write; such tables read fine *uncompressed* | Verified empirically: `fpack` passes VLA tables through uncompressed rather than emitting a compressed-VLA `ZTABLE`; the construct does not occur in practice. |

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
  indicator, extension-keyword order). The standard does not require a *reader* to
  reject these, so enforcing them risks rejecting readable files without improving
  compatibility. (The `NAXIS`/`TFIELDS ≤ 999` cap is the one exception, now
  enforced: a value over 999 is non-conforming *and* could never name its own
  `NAXISn`/`TFORMn` keyword, so rejecting it drops no readable file while closing
  an allocation DoS — `axes()`/`from_data` size a `Vec` straight from the count.)
- ⚪ **Ergonomics / performance** — coordinate-index/strided image API, SIMD /
  zero-copy decode, trivial typed accessors. Not part of the standard.

---

## Verification

```
cargo test                                                        → 182 passed
cargo test --features compression                                 → 215 passed, 2 ignored (fixture emitters)
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
