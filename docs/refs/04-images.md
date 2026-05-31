# 4. Images: Primary Array & IMAGE Extension (Standard §6, §7.1)

## 4.1 Data model

An image is a single N-dimensional array stored in **row-major Fortran order**:
the first axis (`NAXIS1`) varies fastest. Element type is set by `BITPIX`
(see [data representation](03-data-representation.md)); physical values via
`BZERO`/`BSCALE`.

Index mapping for an element at (i₁, i₂, …, im), 1-based as in the spec:

```
offset = i₁ + NAXIS1 × (i₂ − 1) + NAXIS1·NAXIS2 × (i₃ − 1) + …   (1-based)
```

In 0-based terms the linear index is `Σ_k idx_k · Π_{j<k} NAXISj`.

## 4.2 Primary array (§3.3.2)

- Declared by the mandatory primary keywords `SIMPLE`, `BITPIX`, `NAXIS`,
  `NAXISn`.
- `NAXIS = 0` ⇒ no primary data array (common when all science data lives in
  extensions — a "dataless" primary HDU).
- A primary array cannot use random groups *and* a normal array simultaneously.

## 4.3 IMAGE extension (§7.1)

Identical data model to the primary array, but in an extension HDU.

### Mandatory keywords (Table 13, in order)

| Keyword | Value |
|---------|-------|
| `XTENSION` | `'IMAGE   '` |
| `BITPIX` | 8, 16, 32, 64, −32, −64 |
| `NAXIS` | 0…999 |
| `NAXISn` | n = 1…NAXIS |
| `PCOUNT` | 0 (mandatory value for IMAGE) |
| `GCOUNT` | 1 (mandatory value for IMAGE) |
| `END` | — |

### Reserved keywords (§7.1.2)

`BSCALE`, `BZERO`, `BUNIT`, `BLANK`, `DATAMIN`, `DATAMAX`, plus `EXTNAME`,
`EXTVER`, `EXTLEVEL`, and the full WCS keyword set.

## 4.4 Random groups (§6) — legacy, read-only support

A historical primary-array structure (predates extensions, used by early radio
interferometry / `uv` data). Must still be *read* ("once FITS, always FITS"),
but **must not be written** by new software.

Signalled in the primary header by:
- `NAXIS1 = 0`
- `GROUPS = T`
- `PCOUNT` = number of parameters per group
- `GCOUNT` = number of groups

Data is `GCOUNT` groups, each = `PCOUNT` parameters followed by an array of
`NAXIS2 × … × NAXISm` elements. Parameter scaling uses `PSCALn`/`PZEROn` and
parameter names `PTYPEn`.

## Implementation notes (this library)

- Store image data as a flat buffer plus a shape `[NAXISn]`; expose strided /
  ndarray-style views. Fortran order means stride[0] = 1.
- Zero-copy is achievable for the no-scaling, matching-endianness case: hand back
  a typed slice over the mmap'd block. Otherwise decode into an owned buffer.
- Parallelize bulk endian-swap + scale across tiles/rows for "blazing fast" large
  images; the operation is embarrassingly parallel and SIMD-friendly.
- Random groups: implement a read path guarded behind the `GROUPS = T` flag; do
  not expose a writer for it.
