# 7. WCS, Time Coordinates & Compression (Standard §8, §9, §10)

These three chapters layer semantics on top of the structural format. This is a
condensed implementation reference covering every mechanism in §8–§10. It is not
a replacement for the projection and transformation equations: the WCS papers
incorporated by reference are normative, and the Standard's text prevails if the
two conflict.

## 7.1 World Coordinate Systems (§8)

Maps array pixel indices to physical world coordinates (sky position, wavelength,
time, …). Defined across the FITS WCS papers (Greisen & Calabretta et al.),
incorporated into the Standard by reference.

Core keywords (per world axis `i`, pixel axis `j`, optional alternate version `a` ∈
`A`–`Z`; the primary version has `a` blank):

| Keyword | Meaning |
|---------|---------|
| `WCSAXESa` | number of WCS axes (if present, must precede other WCS keywords except `NAXIS`; default is the larger of `NAXIS` and every WCS axis index) |
| `CTYPEia` | axis type + projection, e.g. `'RA---TAN'`, `'DEC--TAN'`, `'FREQ'` (default blank = linear) |
| `CRPIXja` | reference pixel along pixel axis j (1-based; default 0.0) |
| `CRVALia` | world coordinate at the reference point (default 0.0) |
| `CDELTia` | coordinate increment per pixel (must be non-zero; default 1.0) |
| `CUNITia` | units string for axis i (must be degrees for celestial) |
| `PCi_ja` / `CDi_ja` | linear transform matrix (PC = rotation/skew, scaled separately by CDELT; CD folds scale in). Non-singular; **mutually exclusive — must not both appear** |
| `PVi_ma` / `PSi_ma` | numeric / string projection parameters (`m` = 0–99, no leading zero) |
| `CROTAi` | (legacy) rotation angle; deprecated, **must not** appear with PC |
| `LONPOLEa`, `LATPOLEa` | native↔celestial pole alignment |
| `RADESYSa` | reference frame: `'ICRS'`, `'FK5'`, `'FK4'`, `'FK4-NO-E'`, `'GAPPT'` |
| `EQUINOXa` | equinox (Besselian for FK4, Julian for FK5; n/a for ICRS) |
| `WCSNAMEa`, `CNAMEia` | name of the WCS version / of axis i |
| `CRDERia`, `CSYERia` | non-negative random / systematic error in coordinate i (default 0) |

Non-linear `CTYPEia` uses **‘4–3’ form**: 4-char coordinate type, `-`, 3-char
algorithm code (e.g. `RA---TAN`); short types are hyphen-padded. Celestial types are
`RA`/`DEC` and `xLON`/`xLAT` (x = `G` galactic, `E` ecliptic, `H` helioecliptic, `S`
supergalactic). An algorithm shorter than three characters is right-padded with
spaces, though three-character codes are recommended.

Transform pipeline (PC convention): pixel `p_j` → subtract `CRPIX` → linear `PC` →
scale `×CDELT` → projection (`CTYPE` algorithm code + `PVi_m` params) → spherical
rotation (`LONPOLE`/`LATPOLE`) → world. With `CD`, the scale is folded into the matrix
(no separate `×CDELT`).

- `PCi_j` defaults to the identity matrix. If any `CDi_j` is present, every
  unspecified CD element defaults to zero; otherwise the header is PC form even
  if no PC keyword is written. Either matrix must be square, non-singular, and
  span the WCS dimensionality.
- `CDELTi` and legacy `CROTAi` may coexist with CD for old readers, but CD-aware
  software must ignore them. `CROTAi` must not coexist with PC, PV, or PS.
- Pixel coordinates are floating-point generalizations of 1-based array indices;
  the reference pixel may lie outside the stored image.
- Alternative descriptions use suffix `A`–`Z`; the unsuffixed description is
  primary. An alternate must not exist without the primary, and every coordinate
  keyword for an alternate must be written even when it equals the primary value.
  Axis indices are 1–99 without leading zeros. `WCSNAMEa` names a version.
- In a BINTABLE, vector-cell and pixel-list WCS use the column-indexed keyword
  forms in Standard Table 22 (`TCTYPn`, `TCRPXn`, `iCTYPn`, `iCTYna`, etc.).
  The names and defaults differ, but the transformation semantics do not.

### Celestial coordinates (§8.3)

Celestial longitude/latitude axes occur as a paired projection followed by a
spherical rotation. Both use the same three-character projection code. The 27
standard codes in Table 23 are:

| Family | Codes |
|--------|-------|
| Zenithal | `AZP`, `SZP`, `TAN`, `STG`, `SIN`, `ARC`, `ZPN`, `ZEA`, `AIR` |
| Cylindrical | `CYP`, `CEA`, `CAR`, `MER` |
| Pseudocylindrical | `SFL`, `PAR`, `MOL`, `AIT` |
| Conic | `COP`, `COE`, `COD`, `COO` |
| Polyconic/pseudoconic | `BON`, `PCO` |
| Quad-cube | `TSC`, `CSC`, `QSC` |
| HEALPix | `HPX` |

`LONPOLEa` supplies the native longitude of the celestial pole; `LATPOLEa` is
needed when longitude alone does not select the rotation. Equatorial/ecliptic
frames use `RADESYSa`: `ICRS`, `FK5`, `FK4`, `FK4-NO-E`, or `GAPPT`.
Without `RADESYSa`, the default is FK4 for `EQUINOXa < 1984`, FK5 for a later
equinox, and ICRS if no equinox is given. `EQUINOXa` is Besselian for FK4 forms,
Julian for FK5, and inapplicable to ICRS/GAPPT. Deprecated `EPOCH` must not be
given a new meaning.

### Spectral and conventional coordinates (§8.4–§8.5)

The spectral `CTYPEia` type code is one of `FREQ`, `ENER`, `WAVN`, `VRAD`,
`WAVE`, `VOPT`, `ZOPT`, `AWAV`, `VELO`, or `BETA`. A blank algorithm suffix
means that type is linear. Non-linear Table-26 algorithms are the twelve
pairwise transforms `F2W`, `F2V`, `F2A`, `W2F`, `W2V`, `W2A`, `V2F`, `V2W`,
`V2A`, `A2F`, `A2W`, `A2V`, plus `LOG`, detector mappings `GRI`/`GRA`, and
table lookup `TAB`.

- `RESTFRQa` (Hz) or `RESTWAVa` (vacuum metres) should identify a spectral
  feature when meaningful.
- `SPECSYSa` gives the expressed frame and `SSYSOBSa` the frame held constant
  during observation. Allowed frames are `TOPOCENT`, `GEOCENTR`, `BARYCENT`,
  `HELIOCEN`, `LSRK`, `LSRD`, `GALACTOC`, `LOCALGRP`, `CMBDIPOL`, and `SOURCE`.
  `SSYSSRCa` may use any of these except `SOURCE`; with `ZSOURCEa`,
  `VELOSYSa`, and `VELANGLa` it describes a source frame, redshift,
  observer-frame radial velocity (m/s), and velocity-vector orientation.
  `SSYSOBSa` defaults to `TOPOCENT`.
- `DATE-AVG`/`MJD-AVG` and the geocentric metre triple
  `OBSGEO-X`/`OBSGEO-Y`/`OBSGEO-Z` provide the epoch and observing position
  needed for frame corrections.
- `-TAB` obtains coordinate arrays and indexing vectors from a referenced
  BINTABLE through its defined PS/PV parameters; it supports non-uniform and
  multidimensional tabular coordinates.
- Conventional `CTYPEia = 'COMPLEX'` uses coordinates 1=real, 2=imaginary,
  3=optional weight. `CTYPEia = 'STOKES'` uses Table-29 integer polarization
  codes: `1..4` for I/Q/U/V and `-1..-8` for RR/LL/RL/LR/XX/YY/XY/YX.

## 7.2 Time coordinates (§9)

A full framework for representing time (added in 4.0). Key pieces:

- Every FITS time is interpreted as elapsed time relative to a reference point,
  including representations commonly described as absolute JD, MJD, or datetime.
- **Time scale** `TIMESYS` (default `UTC`); overridable per-axis, table column, or
  random-group parameter via the forms in Table 22. Recognized values (Table 30):
  `TAI`, `TT`, deprecated `TDT`, `ET`, deprecated `IAT`, `UT1`, `UTC`, deprecated
  `GMT`, qualified `UT(...)`, `GPS`, `TCG`, `TCB`, `TDB`, and `LOCAL`. A
  realization may be appended, e.g. `TT(TAI)`. `TIME` is a backward-compatible
  axis/column value meaning the scale in `TIMESYS`; it is not a `TIMESYS` value.
  A mission clock or other local scale should be supplied as an alternate to a
  recognized scale and must not have the global reference value applied to it.
- **Reference value** `MJDREF`/`JDREF`/`DATEREF` (§9.2.2; `[M]JDREF` may be split into
  integer `[M]JDREFI` + fractional `[M]JDREFF`). When both parts and the combined
  value exist, both parts win; when only one part and the combined value exist,
  the combined value wins. Between forms, precedence is MJDREF > JDREF > DATEREF.
  If none exists and values are numeric, assume `MJDREF = 0.0`.
- **Reference position and direction keywords**: `TREFPOS`/`TRPOSn` (default
  `TOPOCENTER`; §9.2.3) and `TREFDIR`/`TRDIRn` (§9.2.4).
- **Units**: `TIMEUNIT` (default `s`; Table 34): also `d`, `a` (Julian year), `cy`,
  `min`, `h`, `yr`, `ta`, `Ba`. One §4.3 SI prefix is permitted (`ms`, `ks`, …);
  `ta` and `Ba` have epoch-dependent definitions. `CUNITia` overrides the global
  unit and must itself be a time unit for a time axis.
- **ISO-8601 datetimes** (§9.1.1): `[±C]CCYY-MM-DD[Thh:mm:ss[.s…]]`; the time part
  and decimal seconds **may** be omitted, but **leading zeros may not**, and **no
  timezone designator** (`Z` suffix forbidden). Signed 5-digit years are allowed. In
  UTC the seconds field runs `00–60` (leap seconds), `00–59` otherwise. ISO-8601
  carries no time scale of its own — it follows `TIMESYS`.
  Datetimes are forbidden in image-axis descriptions because `CRVALia` must be
  numeric. Dates before 1582 use the proleptic Gregorian calendar.
- **Epochs** (§9.1.2): Julian `J2000.0` (implied scale TDB, keyword `JEPOCH`) and
  Besselian `B1950.0` (implied scale ET, keyword `BEPOCH`).
- **Reference position and direction**: Standard positions include `TOPOCENTER`,
  `GEOCENTER`, `BARYCENTER`, `RELOCATABLE`, `CUSTOM`, `HELIOCENTER`, `GALACTIC`,
  `EMBARYCENTER`, and named planetary-system centers. Values are case-sensitive
  but only the first three characters are significant. Table 32 restricts
  meaningful scale/position pairs: in particular, barycentric TDB/TCB belong at
  `BARYCENTER`. A topocentric position should include `OBSGEO-X/Y/Z`, geodetic
  `OBSGEO-B/L/H`, or `OBSORBIT`. `TREFDIR`/`TRDIRn` names the longitude and
  latitude columns/keywords used for path-length correction. `PLEPHEM` identifies
  the Solar-System ephemeris (default `DE405`).
- **Global keywords** (§9.5, Table 35): `DATE`, `DATE-OBS`, `DATE-BEG`, `DATE-AVG`,
  `DATE-END`, the `MJD-*` equivalents, `TSTART`, `TSTOP`. Only `TSTART`/`TSTOP`
  are relative to the reference value. **Errors**: `TIMSYER` is absolute/systematic
  and `TIMRDER` relative/random; axis-specific `CSYER`/`CRDER` forms override them.
- **Offset and binning** (§9.4): table-only `TIMEOFFS` is added to the reference
  time and affects `TSTART`, `TSTOP`, and table time values. Table-only `TIMEDEL`
  gives resolution; `TIMEPIXR` (default 0.5, range 0–1) locates a timestamp within
  its bin. These three keywords must not be used for images.
- **Durations** (§9.7): `XPOSURE` is accumulated effective exposure after dead/lost
  time and `TELAPSE` is elapsed start-to-stop time. Durations are numeric in
  `TIMEUNIT`, never ISO-8601.
- **Other time axes** (§9.6): `CTYPEi` = `'TIME'`, `'PHASE'`, `'TIMELAG'`, or
  `'FREQUENCY'`. A phase axis uses `CZPHSia` and constant-period `CPERIia`, or
  their Table-22 BINTABLE forms; timelag and temporal frequency cannot be alternate
  descriptions of a time axis. GTI tables (§9.7) carry mandatory `START`/`STOP`
  columns plus optional `WEIGHT` (default 1, range 0–1); uncovered intervals have
  weight zero.
  Time may also be a WCS axis or a table column. Image time coordinates use the
  complete PC/CD row and full pixel vector; a recognized scale in `CTYPEia` and a
  `CUNITia` value override the corresponding global defaults. `CRVALia` remains a
  numeric elapsed time even when `DATEREF` supplies the zero point.

Alternate time descriptions must retain one reference position. TDB/TCB may be
mixed with each other (and ET to its precision), but not with the first nine
Table-30 Earth scales; `LOCAL` should not be mixed with other scales. Converting
between scales, applying leap-second histories, or computing ephemeris-dependent
light-time corrections requires external chronometry/astrometry data and is not
defined by the file syntax alone.
For authored data, the Standard strongly recommends `DATE`, `TIMESYS`, and one
of `MJDREF`/`JDREF`/`DATEREF`, plus every applicable context-specific keyword.

## 7.3 Compressed data (§10)

### Tiled image compression (§10.1)

A compressed image is stored *inside a BINTABLE* (a registered convention promoted
into the Standard). The image is divided into rectangular **tiles** (default: one
image row per tile); each tile is compressed and stored as a variable-length
byte/int array in one table row. Tiles are ordered by the position of their first
pixel in FITS axis-1-fastest order.

Mandatory keywords (§10.1.1):

| Keyword | Meaning |
|---------|---------|
| `ZIMAGE = T` | this BINTABLE holds a compressed image |
| `ZCMPTYPE` | algorithm: `'RICE_1'`, `'GZIP_1'`, `'GZIP_2'`, `'PLIO_1'`, `'HCOMPRESS_1'`, `'NOCOMPRESS'` |
| `ZBITPIX` | BITPIX of the original (uncompressed) image |
| `ZNAXIS`, `ZNAXISn` | dimensions of the original image |

Other reserved keywords (§10.1.2, **optional**): `ZTILEn` (positive tile size per
axis; default `ZTILE1 = ZNAXIS1`, every other dimension 1), `ZNAMEi`/`ZVALi`
(up to 999 algorithm parameters), `ZQUANTIZ` + `ZDITHER0` (float quantization,
below), and `ZMASKCMP` (null-mask codec). `ZSIMPLE`/`ZEXTEND`/`ZBLOCKED` preserve
primary-only structural records; `ZTENSION`/`ZPCOUNT`/`ZGCOUNT` preserve
IMAGE-extension records; `ZHECKSUM`/`ZDATASUM` preserve original checksums.
Their value and comment fields support exact logical-HDU reconstruction.
All other original image-header records should be copied verbatim, comments and
order included, even when they are unusual in a BINTABLE header.

Table columns (§10.1.3): compressed tile bytes live in `COMPRESSED_DATA`
(`1PB`/`1PI`/`1PJ` or `1Q…` VLA — 8/16/32-bit output stream). `GZIP_COMPRESSED_DATA`
holds gzip'd raw pixels for tiles that won't quantize/compress (their
`COMPRESSED_DATA` descriptor is then a null pointer). For lossy integer codecs,
`NULL_PIXEL_MASK` stores the compressed undefined-pixel mask. (FITS 4.0 defines **no**
`UNCOMPRESSED_DATA` column — that was a pre-standard convention form.)
P or Q descriptors may be used while the heap is below 2.1 GB; Q descriptors are
mandatory for a larger heap.

### Quantization of floating-point data (§10.2)

Lossy compression of floating-point images works by quantizing each tile's floats to
scaled integers, then compressing those. Per-tile `ZSCALE`/`ZZERO` columns give the
non-dithered map `I = round((F − ZZERO)/ZSCALE)`. Subtractive dithering instead
uses `I = round((F − ZZERO)/ZSCALE + R − 0.5)` and reconstructs
`F = (I − R + 0.5) × ZSCALE + ZZERO` with the identical random sequence.
Absence of the scale/zero columns means the image was compressed losslessly
without quantization. `ZQUANTIZ` selects the
method — `'NO_DITHER'`, `'SUBTRACTIVE_DITHER_1'`, `'SUBTRACTIVE_DITHER_2'` (the last
maps exact `0.0` → reserved `−2147483647`, restored to `0.0` on read) — and `ZDITHER0`
(1–10000) seeds the **subtractive dithering** that avoids systematic bias, using the
Appendix-I PRNG (§10.2.1). NaN pixels are set to the `ZBLANK` integer (recommended
`−2147483648`). `ZBLANK` may be one header keyword if constant across tiles or a
per-tile column; the column wins if both exist. For a lossy **integer** codec,
undefined `BLANK` pixels must instead be located through a losslessly compressed
`NULL_PIXEL_MASK` (§10.2.2), named by `ZMASKCMP`.
If `ZQUANTIZ` is absent, `NO_DITHER` is assumed.

### Tiled table compression (§10.3)

Analogous scheme for BINTABLE columns: the table is split into row tiles, each column
within a tile is extracted and compressed separately, and every output column becomes
a `1QB` variable-length byte array (one compressed table row per tile). Required
keywords: `ZTABLE = T`, `ZNAXIS1`/`ZNAXIS2`/`ZPCOUNT` (original table geometry),
`ZFORMn` (original `TFORMn`), `ZCTYPn` (per-column algorithm), `ZTILELEN` (rows per
tile). Permitted algorithms (§10.3.5) are the **lossless** ones only: `'RICE_1'`,
`'GZIP_1'`, `'GZIP_2'`, `'NOCOMPRESS'`. Optional `FZTILELN`/`FZALGOR`/`FZALGn`
directives request a tiling/algorithm; `FZALGn` overrides `FZALGOR`. All original
header records are copied verbatim and in order except structural `NAXIS1`,
`NAXIS2`, `PCOUNT`, `TFORMn`, `THEAP`, and checksum records. Their original values
are represented by `Z*` records, including optional `ZTHEAP`, `ZHECKSUM`, and
`ZDATASUM`.

VLA columns (§10.3.6) are compressed one array at a time into the compressed
table heap. The corresponding compressed-array Q descriptors and the original
P/Q descriptors are concatenated, compressed together with `GZIP_1`, and stored
in that output column. Decompression restores arrays to their original heap
offsets.

### Compression algorithms (§10.4)

| `ZCMPTYPE` | Notes |
|------------|-------|
| `RICE_1` | Rice coding; integer arrays only. Params `BLOCKSIZE` (16/**32**) + `BYTEPIX` (1/2/4/**8**, default 4) via `ZNAMEi`/`ZVALi` |
| `GZIP_1` | DEFLATE (LZ77 + Huffman); no params |
| `GZIP_2` | like `GZIP_1` but bytes shuffled most-significant-first (numeric types only) |
| `PLIO_1` | IRAF run-length mask codec; integer images 0–2²⁴ only; 16-bit list elements |
| `HCOMPRESS_1` | 2-D images only; H-transform + quantize + quadtree. Param `SCALE` (`0.0` = lossless) |
| `NOCOMPRESS` | stored uncompressed |

`GZIP_2` shuffling is forbidden for logical, bit, and character data. New
algorithm names and parameter keywords must be registered with the IAUFWG.

## Implementation notes (this library)

- The typed WCS layer implements the linear transform, all 27 standard celestial
  projections, spectral algorithms, and BINTABLE-backed `-TAB`. It reports
  declared frames but deliberately leaves inter-frame astrometry to a library
  with the required ephemeris and reference-frame data.
- The time layer validates FITS datetimes, units, reference metadata, and time WCS.
  It preserves the declared scale; inter-scale conversion needs current leap-second
  and ephemeris data and remains external.
- Tiled image/table compression is behind the default `compression` feature.
  Independent tile codec work fans across Rayon under `parallel`; ordered heap
  assembly stays serial. The no-default-features build keeps the scalar core.
