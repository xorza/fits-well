# 6. Binary Table Extension (Standard §7.3)

`XTENSION = 'BINTABLE'`. The workhorse FITS table: rows of fixed-width binary
records, columns of typed (optionally array-valued) cells, plus an optional
**heap** for variable-length arrays. This is where most FITS performance work
lives.

## 6.1 Data layout

- Main data table: `NAXIS2` rows, each `NAXIS1` bytes (`BITPIX = 8`, `NAXIS = 2`).
- Row width `NAXIS1 = Σ_n (r_n × b_n)` over the `TFIELDS` columns, where `r_n` is
  the repeat count and `b_n` the element size of column n's `TFORMn`.
- After the main table comes the **heap** (variable-length array storage),
  optionally offset by `THEAP` from the start of the data unit.
- `PCOUNT` = number of bytes in the heap (incl. any gap). `GCOUNT = 1`.
- All numeric data big-endian; same encodings as [§5](03-data-representation.md).

## 6.2 Mandatory keywords (Table 17, in order)

| Keyword | Value |
|---------|-------|
| `XTENSION` | `'BINTABLE'` |
| `BITPIX` | `8` |
| `NAXIS` | `2` |
| `NAXIS1` | row width in bytes |
| `NAXIS2` | number of rows |
| `PCOUNT` | heap size in bytes |
| `GCOUNT` | `1` |
| `TFIELDS` | number of columns (0…999) |
| `TFORMn` | n = 1…TFIELDS, format of column n |
| `END` | — |

## 6.3 `TFORMn` data types (Table 18)

Format is `rTa`: optional **repeat count** `r` (non-negative integer, default 1),
a single **type code** `T`, and optional trailing chars `a` (undefined by spec).

| Code | Description | Bytes/elem |
|:----:|-------------|:----------:|
| `L` | Logical (`T`/`F`/`0`) | 1 |
| `X` | Bit | ⌈bits/8⌉ † |
| `B` | Unsigned byte | 1 |
| `I` | 16-bit integer | 2 |
| `J` | 32-bit integer | 4 |
| `K` | 64-bit integer | 8 |
| `A` | Character | 1 |
| `E` | Single-precision float | 4 |
| `D` | Double-precision float | 8 |
| `C` | Single-precision complex | 8 |
| `M` | Double-precision complex | 16 |
| `P` | Array descriptor (32-bit) → heap | 8 |
| `Q` | Array descriptor (64-bit) → heap | 16 |

† `X`: `r` is the number of bits; storage is ⌈r/8⌉ bytes.

- `rA` is a character string of length `r` (one cell), not `r` separate strings.
- `r = 0` is allowed (empty cell). Repeat `r` applies element-wise for numerics.

## 6.4 Scaling & nulls

- Physical value: `physical = TZEROn + TSCALn × stored` (Eq. 7).
  Must **not** be applied to `A`, `L`, `X` columns.
- For `P`/`Q`, scaling applies to heap array values, not the descriptor.
- **Unsigned integers** (Table 19): `TSCALn = 1` plus `TZEROn` =

  | `TFORMn` | Native (stored) | Physical | `TZEROn` |
  |:--------:|------------------|----------|----------|
  | `B` | unsigned | signed byte | `-128` (−2⁷) |
  | `I` | signed | unsigned 16-bit | `32768` (2¹⁵) |
  | `J` | signed | unsigned 32-bit | `2147483648` (2³¹) |
  | `K` | signed | unsigned 64-bit | `9223372036854775808` (2⁶³) |

- `TNULLn` (integer columns only) = the stored integer value denoting undefined.
  Float columns use IEEE NaN for undefined.

## 6.5 Multidimensional cells — `TDIMn`

A vector cell can be reshaped into an N-D array via `TDIMn = '(d1,d2,…)'`, with
`Π di = r` (the repeat count). Fortran order (d1 fastest).

## 6.6 Variable-length arrays (§7.3.5–7.3.6)

Columns of type `P`/`Q` store a fixed-size **array descriptor** in the row and the
actual data in the heap.

- Descriptor layout: `(nelem, byte_offset)` — two 32-bit ints for `P`, two 64-bit
  ints for `Q`. `byte_offset` is relative to the start of the heap.
- `TFORMn = 'rPt(emax)'` / `'rQt(emax)'`: `t` is the element type code, `emax` is
  the maximum element count across rows (guideline, aids preallocation).
- Heap begins `THEAP` bytes from the start of the data unit (default = end of the
  main table, i.e. `NAXIS1 × NAXIS2`). Heap size counted in `PCOUNT`.
- Guidelines (§7.3.6): writers should pack the heap with no gaps and in row order;
  readers must not assume either.

## 6.7 Reserved keywords (§7.3.2)

`TTYPEn` (name; case-insensitive, recommend `[A-Za-z0-9_]`), `TUNITn`,
`TSCALn`, `TZEROn`, `TNULLn`, `TDISPn`, `TDIMn`, `THEAP`, plus
`EXTNAME`/`EXTVER`/`EXTLEVEL`, `AUTHOR`, `REFERENC`.

## Implementation notes (this library)

- Precompute per-column byte offset within a row and element size at header-parse
  time; store as a column-descriptor table. Row access is then offset arithmetic.
- **Column-oriented reads**: striding by `NAXIS1` to gather one column is
  cache-unfriendly; for analytic workloads provide a transpose/columnar
  materialization, and SIMD-gather where strides allow. Row reads are contiguous.
- Endian swap + `TSCAL/TZERO` is vectorizable per column; fast path when
  `TSCALn==1 && TZEROn==0` and types match host (raw slice).
- Unsigned detection mirrors images: integer `TFORM` + `TZEROn == 2^(n-1)` +
  `TSCALn == 1` ⇒ expose `uN`.
- Heap/VLA: parse descriptors lazily; expose per-row array slices into the heap.
  Validate `byte_offset + nelem×bytes ≤ PCOUNT`.
- `X` (bit) columns: pack/unpack MSB-first within each byte.
