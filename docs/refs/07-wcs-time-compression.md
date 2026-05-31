# 7. WCS, Time Coordinates & Compression (Standard §8, §9, §10)

These three chapters layer semantics on top of the structural format. A v1 of the
library can parse/preserve their keywords as ordinary header records and add typed
support incrementally. This file is an orientation map, not a full transcription —
consult the PDF (`fits_standard40.pdf`) §8–§10 and the WCS papers for normative
detail.

## 7.1 World Coordinate Systems (§8)

Maps array pixel indices to physical world coordinates (sky position, wavelength,
time, …). Defined across the FITS WCS papers (Greisen & Calabretta et al.),
incorporated into the Standard by reference.

Core keywords (per world axis `i`, pixel axis `j`, optional alternate version `a` ∈
`A`–`Z`; the primary version has `a` blank):

| Keyword | Meaning |
|---------|---------|
| `WCSAXES` | number of WCS axes (if present, must precede other WCS keywords; default NAXIS) |
| `CTYPEia` | axis type + projection, e.g. `'RA---TAN'`, `'DEC--TAN'`, `'FREQ'` (default blank = linear) |
| `CRPIXja` | reference pixel along pixel axis j (1-based; default 0.0) |
| `CRVALia` | world coordinate at the reference point (default 0.0) |
| `CDELTia` | coordinate increment per pixel (must be non-zero; default 1.0) |
| `CUNITia` | units string for axis i (must be degrees for celestial) |
| `PCi_ja` / `CDi_ja` | linear transform matrix (PC = rotation/skew, scaled separately by CDELT; CD folds scale in). Non-singular; **mutually exclusive — must not both appear** |
| `PVi_ma` / `PSi_ma` | numeric / string projection parameters (`m` = 0–99) |
| `CROTAi` | (legacy) rotation angle; deprecated, **must not** appear with PC |
| `LONPOLEa`, `LATPOLEa` | native↔celestial pole alignment |
| `RADESYSa` | reference frame: `'ICRS'`, `'FK5'`, `'FK4'`, `'FK4-NO-E'`, `'GAPPT'` |
| `EQUINOXa` | equinox (Besselian for FK4, Julian for FK5; n/a for ICRS) |
| `WCSNAMEa`, `CNAMEia` | name of the WCS version / of axis i |
| `CRDERia`, `CSYERia` | random / systematic error in coordinate i |

Non-linear `CTYPEia` uses **‘4–3’ form**: 4-char coordinate type, `-`, 3-char
algorithm code (e.g. `RA---TAN`); short types are hyphen-padded. Celestial types are
`RA`/`DEC` and `xLON`/`xLAT` (x = `G` galactic, `E` ecliptic, `H` helioecliptic, `S`
supergalactic).

Transform pipeline (PC convention): pixel `p_j` → subtract `CRPIX` → linear `PC` →
scale `×CDELT` → projection (`CTYPE` algorithm code + `PVi_m` params) → spherical
rotation (`LONPOLE`/`LATPOLE`) → world. With `CD`, the scale is folded into the matrix
(no separate `×CDELT`). In a BINTABLE the keywords take column-indexed forms (`TCTYPn`,
`TCRPXn`, `iCTYPn`, …; Table 22). §8.3 covers celestial projections (Table 23), §8.4
spectral (Tables 25–26), §8.5 conventional types (`'COMPLEX'`, `'STOKES'`).

## 7.2 Time coordinates (§9)

A full framework for representing time (added in 4.0). Key pieces:

- **Time scale** `TIMESYS` (default `UTC`); overridable per-axis via `CTYPEia`/`TCTYPn`.
  Recognized values (Table 30): `UTC`, `TT`, `TDT`, `ET`, `TAI`, `IAT`, `UT1`, `GMT`,
  `GPS`, `TCG`, `TCB`, `TDB`, `LOCAL`. A realization may be appended, e.g. `TT(TAI)`.
- **Reference value** `MJDREF`/`JDREF`/`DATEREF` (§9.2.2; `[M]JDREF` may be split into
  integer `[M]JDREFI` + fractional `[M]JDREFF`; precedence MJDREF > JDREF > DATEREF),
  **position** `TREFPOS`/`TRPOSn` (default `TOPOCENTER`; §9.2.3), **direction**
  `TREFDIR`/`TRDIRn` (§9.2.4).
- **Units**: `TIMEUNIT` (default `s`; Table 34): also `d`, `a` (Julian year), `cy`,
  `min`, `h`, `yr`, `ta`, `Ba`. Overridable by `CUNITia`.
- **ISO-8601 datetimes** (§9.1.1): `[±C]CCYY-MM-DD[Thh:mm:ss[.s…]]`; the time part
  and decimal seconds **may** be omitted, but **leading zeros may not**, and **no
  timezone designator** (`Z` suffix forbidden). Signed 5-digit years are allowed. In
  UTC the seconds field runs `00–60` (leap seconds), `00–59` otherwise. ISO-8601
  carries no time scale of its own — it follows `TIMESYS`.
- **Epochs** (§9.1.2): Julian `J2000.0` (implied scale TDB, keyword `JEPOCH`) and
  Besselian `B1950.0` (implied scale ET, keyword `BEPOCH`).
- **Global keywords** (§9.5, Table 35): `DATE`, `DATE-OBS`, `DATE-BEG`, `DATE-AVG`,
  `DATE-END`, the `MJD-*` equivalents, `TSTART`, `TSTOP`. **Binning** (§9.4.2):
  `TIMEDEL`, `TIMEPIXR` (default 0.5). **Durations** (§9.7): `XPOSURE` (effective
  exposure, dead-time corrected) and `TELAPSE` — note it is `XPOSURE`, **not**
  `EXPOSURE`; durations are numeric, never ISO-8601. `TIMEOFFS` (§9.4.1) is a bulk
  clock offset.
- Other time axes (§9.6): `CTYPEi` = `'TIME'`, `'PHASE'`, `'TIMELAG'`, or
  `'FREQUENCY'`. GTI tables (§9.7) carry `START`/`STOP` columns (+ optional `WEIGHT`).
  Time may also be a WCS axis or a table column.

## 7.3 Compressed data (§10)

### Tiled image compression (§10.1)

A compressed image is stored *inside a BINTABLE* (a registered convention promoted
into the Standard). The image is divided into rectangular **tiles** (default: one
image row per tile); each tile is compressed and stored as a variable-length
byte/int array in one table row, in row-major tile order.

Mandatory keywords (§10.1.1):

| Keyword | Meaning |
|---------|---------|
| `ZIMAGE = T` | this BINTABLE holds a compressed image |
| `ZCMPTYPE` | algorithm: `'RICE_1'`, `'GZIP_1'`, `'GZIP_2'`, `'PLIO_1'`, `'HCOMPRESS_1'`, `'NOCOMPRESS'` |
| `ZBITPIX` | BITPIX of the original (uncompressed) image |
| `ZNAXIS`, `ZNAXISn` | dimensions of the original image |

Other reserved keywords (§10.1.2, **optional**): `ZTILEn` (tile size per axis; default
row-by-row), `ZNAMEi`/`ZVALi` (algorithm parameters, e.g. Rice blocksize), `ZQUANTIZ`
+ `ZDITHER0` (float quantization, below), `ZMASKCMP` (null-mask codec), and `ZSIMPLE`/
`ZEXTEND`/`ZTENSION`/`ZPCOUNT`/`ZGCOUNT`/`ZHECKSUM`/`ZDATASUM` (verbatim copies of the
original image's mandatory keywords, for exact HDU reconstruction).

Table columns (§10.1.3): compressed tile bytes live in `COMPRESSED_DATA`
(`1PB`/`1PI`/`1PJ` or `1Q…` VLA — 8/16/32-bit output stream). `GZIP_COMPRESSED_DATA`
holds gzip'd raw pixels for tiles that won't quantize/compress (their
`COMPRESSED_DATA` descriptor is then a null pointer). `NULL_PIXEL_MASK` stores the
compressed undefined-pixel mask for lossy codecs. (FITS 4.0 defines **no**
`UNCOMPRESSED_DATA` column — that was a pre-standard convention form.)

### Quantization of floating-point data (§10.2)

Lossy compression of floating-point images works by quantizing each tile's floats to
scaled integers, then compressing those. Per-tile `ZSCALE`/`ZZERO` columns give the
linear map `I = round((F − ZZERO)/ZSCALE)`, i.e. `physical = ZZERO + ZSCALE × I`
(their absence ⇒ the tile was compressed losslessly, unscaled). `ZQUANTIZ` selects the
method — `'NO_DITHER'`, `'SUBTRACTIVE_DITHER_1'`, `'SUBTRACTIVE_DITHER_2'` (the last
maps exact `0.0` → reserved `−2147483647`, restored to `0.0` on read) — and `ZDITHER0`
(1–10000) seeds the **subtractive dithering** that avoids systematic bias, using the
Appendix-I PRNG (§10.2.1). NaN pixels are set to the `ZBLANK` integer (recommended
`−2147483648`); for lossy codecs the undefined-pixel locations are instead preserved
via a `NULL_PIXEL_MASK` (§10.2.2).

### Tiled table compression (§10.3)

Analogous scheme for BINTABLE columns: the table is split into row tiles, each column
within a tile is extracted and compressed separately, and every output column becomes
a `1QB` variable-length byte array (one compressed table row per tile). Required
keywords: `ZTABLE = T`, `ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT` (original table geometry),
`ZFORMn` (original `TFORMn`), `ZCTYPn` (per-column algorithm), `ZTILELEN` (rows per
tile). Permitted algorithms (§10.3.5) are the **lossless** ones only: `'RICE_1'`,
`'GZIP_1'`, `'GZIP_2'`, `'NOCOMPRESS'`. Optional `FZTILELN`/`FZALGOR`/`FZALGn`
directives request a tiling/algorithm. VLA columns (§10.3.6) are compressed per-array,
their descriptors gzip'd into the heap.

### Compression algorithms (§10.4)

| `ZCMPTYPE` | Notes |
|------------|-------|
| `RICE_1` | Rice coding; integer arrays only. Params `BLOCKSIZE` (16/**32**) + `BYTEPIX` (1/2/4/**8**, default 4) via `ZNAMEi`/`ZVALi` |
| `GZIP_1` | DEFLATE (LZ77 + Huffman); no params |
| `GZIP_2` | like `GZIP_1` but bytes shuffled most-significant-first (numeric types only) |
| `PLIO_1` | IRAF run-length mask codec; integer images 0–2²⁴ only; 16-bit list elements |
| `HCOMPRESS_1` | 2-D images only; H-transform + quantize + quadtree. Param `SCALE` (`0.0` = lossless) |
| `NOCOMPRESS` | stored uncompressed |

## Implementation notes (this library)

- v1: round-trip all §8/§9/§10 keywords losslessly as header records; expose a
  typed WCS/time API as a later layer over the parsed header.
- WCS math (projections, spherical rotations) is sizable — consider a separate
  module/feature flag; many users only need pixel I/O.
- Tiled compression is the highest-leverage performance feature for real archives
  (most modern survey data is Rice-compressed). Decode path: read BINTABLE →
  per-tile VLA → decompress (`RICE_1`/`GZIP`/`HCOMPRESS`/`PLIO`) → reassemble into
  the `ZNAXISn` image. Tiles decode independently ⇒ trivially parallel.
- Keep compression behind a feature flag so the core format crate stays dependency-light.
