# WCS round-trip precision before and after the colatitude change

Date: 2026-10-24. Each row is the largest miss of `world_to_pixel(pixel_to_world(p))` in
a sweep of pixels `p` at offsets of 0.5, 1, 37, 128, 1e3, 1e4 and 1e5 pixels from
`CRPIX`, in four directions, out to 20°. The headers are `WcsHeader::celestial` with
`CDELT = (−s, s)`. Before: commit 6461e9b. After: the change that carries θ as its sine
and cosine between the rotation and the projection.

## Zenithal projections

| Projection | `CRVAL` | s (°/px) | Before (px) | After (px) | After (°) |
|---|---|---|---|---|---|
| TAN | (6.155, −2.072) | 3.4e-4 | 5.5e-6 | 7.7e-12 | 2.6e-15 |
| TAN | (6.155, −2.072) | 1e-6 | 0.5 | 7.2e-10 | 7.2e-16 |
| TAN | (150, 60) | 3.4e-4 | 5.1e-6 | 4.5e-11 | 1.5e-14 |
| TAN | (150, 60) | 1e-6 | 0.5 | 1.4e-8 | 1.4e-14 |
| TAN | (10, 89.95) | 3.4e-4 | 2.8e-6 | 5.5e-11 | 1.9e-14 |
| TAN | (10, 89.95) | 1e-6 | 0.5 | 1.9e-8 | 1.9e-14 |
| SIN | (6.155, −2.072) | 1e-6 | 0.35 | 6.1e-10 | 6.1e-16 |
| SIN | (150, 60) | 1e-6 | 0.70 | 1.9e-8 | 1.9e-14 |
| ARC | (150, 60) | 1e-6 | 0.5 | 1.1e-8 | 1.1e-14 |
| STG | (150, 60) | 1e-6 | 0.5 | 1.1e-8 | 1.1e-14 |
| ZEA | (150, 60) | 1e-6 | 0.5 | 1.3e-8 | 1.3e-14 |

Before, a world point within about 8.5e-7° (√(2ε) rad) of the native pole had
`sin θ` = 1 in `f64`, so `asin` put it on the pole: at 1e-6°/px every offset under
one pixel was lost. After, each miss is about one ulp of the world coordinates in
degrees, `ulp(α)·cos δ + ulp(δ)`: 1.4e-14° at α = 150°, δ = 60°.

## Other families

| Projection | `CRVAL` | s (°/px) | Before (px) | After (px) |
|---|---|---|---|---|
| CAR | (10, 89.95) | 1e-6 | 8.6e-6 | 1.8e-8 |
| MER | (10, 89.95) | 3.4e-4 | 1.5e-7 | 5.2e-11 |
| SFL | (10, 89.95) | 1e-6 | 8.6e-6 | 1.8e-8 |
| AIT | (10, 89.95) | 1e-6 | 8.6e-6 | 1.8e-8 |
| CAR | (6.155, −2.072) | 1e-6 | 6.4e-8 | 6.4e-8 |

The celestial pole inside the field made the native → world `asin` lose δ in the same
way; after, those rows drop to the ulp of the world coordinates too. Near the
native equator, CAR, MER, SFL and AIT keep a floor of 6.4e-14°: φ = φ_p + atan2(…) in
degrees, with φ_p = 180°, rounds at an ulp of 180°, 2.8e-14°.

## Cost

Criterion medians of `atomic/wcs/*/tan`, one row of the light's plate solution, on the
same machine, before (6461e9b) and after:

| Case | Before | After |
|---|---|---|
| `to_world/tan` | 479 µs | 461 µs |
| `to_pixel/tan` | 542 µs | 405 µs |

The rotation takes Δα's sine and cosine from one `sin_cos` of Δα/2, √(x² + y²) needs no
`hypot` for a unit vector, TAN deprojects with a square root and no `atan`, and each
projection takes θ or ζ by `atan2` only where it uses it.
