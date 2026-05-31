# 7. WCS, Time Coordinates & Compression (Standard §8, §9, §10)

These three chapters layer semantics on top of the structural format. A v1 of the
library can parse/preserve their keywords as ordinary header records and add typed
support incrementally. This file is an orientation map, not a full transcription —
consult the PDF (`fits_standard40.pdf`) §8–§10 and the WCS papers for normative
detail.

## 7.1 World Coordinate Systems (§8)

Maps array pixel indices to physical world coordinates (sky position, wavelength,
time, …). Defined across the FITS WCS papers (Greisen & Calabretta et al.).

Core keywords (per axis `i`, optional alternate `a` ∈ `A`–`Z`):

| Keyword | Meaning |
|---------|---------|
| `WCSAXES` | number of WCS axes |
| `CTYPEi` | axis type + projection, e.g. `'RA---TAN'`, `'DEC--TAN'`, `'FREQ'` |
| `CRPIXi` | reference pixel (1-based) along axis i |
| `CRVALi` | world coordinate at the reference pixel |
| `CDELTi` | coordinate increment per pixel |
| `CUNITi` | units string for axis i |
| `PCi_j` / `CDi_j` | linear transform matrix (rotation/skew/scale) |
| `CROTAi` | (legacy) rotation angle |
| `LONPOLE`, `LATPOLE` | native↔celestial pole alignment |
| `RADESYS` | reference frame, e.g. `'ICRS'`, `'FK5'` |
| `EQUINOX` | equinox for FK4/FK5 |

Transform pipeline: pixel → (subtract `CRPIX`) → linear (`PC`/`CD`) → (×`CDELT`)
→ projection (`CTYPE` algorithm code) → world coordinate. §8.3 covers celestial
projections; §8.4 spectral; §8.5 conventional types.

## 7.2 Time coordinates (§9)

A full framework for representing time (added in 4.0). Key pieces:

- **Time scale** `TIMESYS` / per-axis: `UTC`, `TT`, `TAI`, `TDB`, `TCG`, `TCB`, …
- **Reference value/position/direction**: `MJDREF`/`JDREF`/`DATEREF`,
  `TREFPOS`, `TREFDIR`.
- **Units**: `TIMEUNIT` (default `s`); also `d`, `a` (Julian year), etc.
- **ISO-8601 datetimes** (§9.1.1): `YYYY-MM-DDThh:mm:ss[.sss…]`; no defaulting of
  components, no omitted leading zeros; `DATE-OBS` etc. UTC unless `TIMESYS` says
  otherwise.
- **Epochs** (§9.1.2): Julian (`J2000.0`) and Besselian (`B1950.0`).
- **Global keywords** (§9.5): `MJD-OBS`, `DATE-OBS`, `DATE-BEG`, `DATE-END`,
  `DATE-AVG`, `TSTART`, `TSTOP`, `TIMEDEL`, `TIMEPIXR`, `TELAPSE`, `EXPOSURE`.
- Time may also be a WCS axis (`CTYPEi = 'TIME'`) or a table column.

## 7.3 Compressed data (§10)

### Tiled image compression (§10.1)

A compressed image is stored *inside a BINTABLE* (a registered convention promoted
into the Standard). The image is divided into rectangular **tiles**; each tile is
compressed and stored as a variable-length byte/int array in one table row.

Required keywords:

| Keyword | Meaning |
|---------|---------|
| `ZIMAGE = T` | this BINTABLE holds a compressed image |
| `ZCMPTYPE` | algorithm: `'RICE_1'`, `'GZIP_1'`, `'GZIP_2'`, `'PLIO_1'`, `'HCOMPRESS_1'`, … |
| `ZBITPIX` | BITPIX of the original (uncompressed) image |
| `ZNAXIS`, `ZNAXISn` | dimensions of the original image |
| `ZTILEn` | tile dimensions along each axis |
| `ZNAMEi`/`ZVALi` | algorithm parameters (e.g. Rice blocksize) |

Compressed tile bytes live in a column named `COMPRESSED_DATA` (`P`/`Q` VLA);
`GZIP_COMPRESSED_DATA` and `UNCOMPRESSED_DATA` columns are fallbacks for tiles
that don't compress under the chosen scheme.

### Quantization of floating-point data (§10.2)

Lossy compression of floating-point images works by quantizing each tile's
floats to scaled integers, then compressing those. Per-tile keywords/columns
`ZSCALE` and `ZZERO` give the linear map (`physical = ZZERO + ZSCALE × quantized`).
`ZQUANTIZ` selects the quantization method (e.g. `'NO_DITHER'`,
`'SUBTRACTIVE_DITHER_1/2'`) and `ZDITHER0` seeds the **subtractive dithering**
that avoids systematic bias (§10.2.1). Undefined (NaN) pixels are preserved
through lossy compression (§10.2.2).

### Tiled table compression (§10.3)

Analogous scheme for compressing BINTABLE columns (each column compressed in
row-blocks).

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
