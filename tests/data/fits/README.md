# FITS test fixtures

Real-world FITS files for round-trip and decoder testing. All are public-domain
and conform to the FITS 4.0 standard (FITS carries no version tag — these
exercise features ratified in 4.0: WCS, time/celestial coordinates, multi-HDU
layouts, binary tables with vector columns, and the legacy random-groups form
that the "once FITS, always FITS" read path must keep supporting).

## NASA FITS Support Office samples
Source: <https://fits.gsfc.nasa.gov/fits_samples.html> (mostly via the MAST archive)

| File | Feature exercised |
|------|-------------------|
| `UITfuv2582gc.fits` | Plain 512×512 primary image (simplest case) |
| `WFPC2u5780205r_c0fx.fits` | 200×200×4 image cube (NAXIS=3) + table extension with WCS |
| `NICMOSn4hk12010_mos.fits` | Null primary array + 5 IMAGE extensions (sci/err/dq/samp/time) |
| `IUElwp25637mxlo.fits` | Spectrum in BINTABLE vector columns |
| `EUVEngc4151imgx.fits` | Multiple image extensions, each with an associated binary table |
| `FGSf64y0106m_a1f.fits` | 89688×7 primary time series + astrometric table extension |
| `DDTSUVDATA.fits` | **Random Groups** legacy format (NAXIS1=0, GROUPS=T) from the AIPS suite |

## WCS celestial-projection samples
Source: <https://www.atnf.csiro.au/people/mcalabre/WCS/example_data.html>
(Calabretta & Greisen "Representations of Celestial Coordinates in FITS")

Each is the same all-sky map reprojected so WCS keyword handling can be checked
against a known projection:

| File | Projection |
|------|-----------|
| `1904-66_TAN.fits` | Gnomonic (tangent plane) |
| `1904-66_SIN.fits` | Orthographic / slant orthographic |
| `1904-66_AIT.fits` | Hammer-Aitoff |
| `1904-66_CAR.fits` | Plate carrée (cylindrical) |

More projection codes (AZP, SZP, STG, NCP, ARC, ZPN, ZEA, AIR, CYP, CEA, MER,
SFL, PAR, MOL, COP, COE, COD, COO, BON, PCO, TSC, CSC, QSC, HPX) are available
at the same source if broader WCS coverage is needed.
