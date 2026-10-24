# Sources of the WCS test goldens

Date: 2026-10-24. Each golden below was printed by the tool named, in the commit named.
No script that generated them was kept, and neither astropy nor wcslib is installed
here, so none was run again. A value that a later change rewrote (a lint that grouped
its digits) keeps the source of the commit that first held it.

| Golden | Where | Source | First commit |
|---|---|---|---|
| `TAN_GOLDEN` | `world_coordinates/mod.rs` | `astropy.wcs` (wcslib) on `wcs_tan.fits` | aebfe12, 2026-05-31 |
| `CEA_GOLDEN` | `world_coordinates/mod.rs` | `astropy.wcs` (wcslib), CEA with `PV2_1 = 0.5` | 99777ae, 2026-06-01 |
| `CROTA_GOLDEN` | `world_coordinates/mod.rs` | `astropy.wcs` (wcslib), TAN with `CROTA2 = 25` | 4d94f57, 2026-05-31 |
| `PROJECTION_GOLDEN` | `projection/tests.rs` | `astropy.wcs` (wcslib) | 4d94f57, 2026-05-31; the parameterized rows 4f1a555, 2026-06-01 |
| `NATIVE_GOLDEN`, TSC, CSC, QSC, HPX rows | `projection/tests.rs` | wcslib 8.5 `*s2x` and `*x2s` | b3e39eb, 2026-07-17 |
| `NATIVE_GOLDEN`, slant SIN rows | `projection/tests.rs` | Paper II eqs. 61–62, by hand | — |
| `NATIVE_GOLDEN`, ARC rows | `projection/tests.rs` | R = 90° − θ, closed form | 4b71689, 2026-10-24 |
| Grism (`WAVE-GRI`, `AWAV-GRA`) | `axis_transform/tests.rs` | wcslib 8.5, the KPNO MARS parameters | 91b5e2f, 2026-07-17 |
| `spectral_algorithms_match_wcslib` | `axis_transform/tests.rs` | wcslib 8.5 | c0252ef, 2026-07-17 |
