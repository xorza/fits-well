# 4. Images: Primary Array & IMAGE Extension (Standard §6, §7.1)

## 4.1 Data model

An image is a single N-dimensional array stored in **FITS/Fortran order**:
the first axis (`NAXIS1`) varies fastest (often called column-major order).
Element type is set by `BITPIX`
(see [data representation](03-data-representation.md)); physical values via
`BZERO`/`BSCALE`.

The 1-based ordinal of an element at (i₁, i₂, …, im) is:

```
ordinal = i₁ + NAXIS1 × (i₂ − 1) + NAXIS1·NAXIS2 × (i₃ − 1) + …
```

The byte offset is `(ordinal − 1) × |BITPIX|/8`. In 0-based terms the linear
element index is `Σ_k idx_k · Π_{j<k} NAXISj`.

## 4.2 Primary array (§3.3.2)

- Declared by the mandatory primary keywords `SIMPLE`, `BITPIX`, `NAXIS`,
  `NAXISn`.
- `NAXIS = 0` ⇒ no primary data array (common when all science data lives in
  extensions — a "dataless" primary HDU).
- If any `NAXISn = 0`, no data follow, except for the `NAXIS1 = 0`
  random-groups signature.
- A primary array cannot use random groups *and* a normal array simultaneously.
- Elements use the §5 representation selected by `BITPIX`; the last data block
  is padded with zero bits to 2880 bytes.

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

The records from `XTENSION` through `GCOUNT` are consecutive. `NAXIS = 0` or
any zero-length `NAXISn` means no data blocks. When `NAXIS > 0` and all axes are
non-zero, the unpadded byte count is `|BITPIX|/8 × Π NAXISn`; the last block is
zero-filled.

### Reserved keywords (§7.1.2)

`BSCALE`, `BZERO`, `BUNIT`, `BLANK`, `DATAMIN`, `DATAMAX`, plus `EXTNAME`,
`EXTVER`, `EXTLEVEL`, and the full WCS keyword set.

## 4.4 Random groups (§6) — legacy, read-only support

A historical primary-array structure (predates extensions, used by early radio
interferometry / `uv` data). It remains valid and must still be read. The
Standard deprecates further use outside existing radio-interferometry practice
and says it **should not** be used for new applications; this library's stricter
policy is read-only support.

Signalled in the primary header by:
- `NAXIS1 = 0`
- `GROUPS = T`
- `PCOUNT` = number of parameters per group
- `GCOUNT` = number of groups

Here `NAXIS` is one greater than the dimensionality of each group array, so
`NAXIS2` describes its first axis. The ordered prefix remains `SIMPLE`, `BITPIX`,
`NAXIS`, `NAXIS1`, `NAXIS2...`; `GROUPS`, `PCOUNT`, and `GCOUNT` are mandatory
before `END`, but need not immediately follow the axis records.

Data is `GCOUNT` consecutive groups, each containing `PCOUNT` parameters followed
immediately by an array of `NAXIS2 × … × NAXISm` elements. The next group's first
parameter follows the preceding group's final element. Parameters and array
elements have the same `BITPIX` representation, arrays use ordinary FITS order,
and the final block is zero-filled.

When present, `PTYPEn` names parameter `n`; its physical value is
`PZEROn + PSCALn × stored` (defaults 0 and 1). If several `PTYPEn` values have the
same name, their derived physical values are **summed**; this is the standard
mechanism for representing a parameter with more precision than one array element.

## Implementation notes (this library)

- Store image data as a flat buffer plus a shape `[NAXISn]`; expose strided /
  ndarray-style views. Fortran order means stride[0] = 1.
- Zero-copy is achievable for the no-scaling, matching-endianness case: hand back
  a typed slice over the mmap'd block. Otherwise decode into an owned buffer.
- Reuse the caller's aligned swap buffer for repeated reads. This memory-bound
  path is faster serially on the measured targets; explicit SIMD and threading
  add overhead without overcoming transformed-store bandwidth.
- Random groups: implement a read path guarded behind the `GROUPS = T` flag; do
  not expose a writer for it.
