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
- `PCOUNT` = size of the supplemental data area (gap + heap), in bytes. `GCOUNT = 1`.
- All numeric data big-endian; same encodings as [§5](03-data-representation.md).

## 6.2 Mandatory keywords (Table 17, in order)

| Keyword | Value |
|---------|-------|
| `XTENSION` | `'BINTABLE'` |
| `BITPIX` | `8` |
| `NAXIS` | `2` |
| `NAXIS1` | row width in bytes |
| `NAXIS2` | number of rows |
| `PCOUNT` | supplemental area (gap + heap) size in bytes |
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
  May be `NUL`-terminated early; chars after the first `NUL` are undefined.
- `r = 0` is allowed (empty cell). Repeat `r` applies element-wise for numerics.
- `P`/`Q` (array-descriptor) columns permit **only** repeat count 0 or 1 — a cell
  holds at most one descriptor.

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

- `TNULLn` = the **raw stored** integer denoting undefined, for `B`/`I`/`J`/`K`
  columns (and `P`/`Q` descriptors pointing to integer arrays); forbidden on other
  types. It is matched against the stored value **before** Eq. 7, not the physical
  value — e.g. an unsigned-16 column (`TZEROn = 32768`) whose physical-0 means
  undefined needs `TNULLn = -32768`. Float/complex columns use IEEE NaN instead (no
  `TNULLn`).

## 6.5 Multidimensional cells — `TDIMn`

A vector cell can be reshaped into an N-D array via `TDIMn = '(d1,d2,…)'`. The
product `Π di` *must be ≤* `r` (the `TFORMn` repeat count; for `P`/`Q`, ≤ the
descriptor's stored array length); any trailing unused elements are undefined fill.
Fortran order (d1 fastest). String arrays use the same notation: `TFORMn = '60A'`
with `TDIMn = '(5,4,3)'` is a 4×3 array of 5-character strings.

## 6.6 Variable-length arrays (§7.3.5–7.3.6)

Columns of type `P`/`Q` store a fixed-size **array descriptor** in the row and the
actual data in the heap.

- Descriptor layout: `(nelem, byte_offset)` — two 32-bit **signed** ints for `P`,
  two 64-bit **signed** ints for `Q`. `byte_offset` is zero-indexed from the start
  of the heap. (Repeat count on the column itself is 0 or 1 only — see §6.3.)
- `TFORMn = 'rPt(emax)'` / `'rQt(emax)'`: `t` is the element type code (any type but
  `P`/`Q`), `emax` is the maximum element count across rows (guideline, aids
  preallocation); extra trailing chars after `(emax)` are allowed.
- Heap begins `THEAP` bytes from the start of the data unit (default *and minimum* =
  end of the main table, `NAXIS1 × NAXIS2`); a larger `THEAP` leaves a gap before
  the heap. `PCOUNT` counts gap + heap. `THEAP` must not appear when `PCOUNT = 0`.
- Zero-length array (`nelem = 0`): no heap data, `byte_offset` is undefined (write
  0). The referenced span `byte_offset + nelem×bytes` must lie entirely within the
  heap; negative descriptor values are undefined.
- Guidelines (§7.3.6): heap data may be stored in any row order, with gaps, and with
  pointer aliasing (two descriptors → one span); readers must assume none of these,
  and no element alignment is guaranteed.

## 6.7 Reserved keywords (§7.3.2)

`TTYPEn` (name; case-insensitive, recommend `[A-Za-z0-9_]`), `TUNITn`,
`TSCALn`, `TZEROn`, `TNULLn`, `TDISPn`, `TDIMn`, `THEAP`,
`TDMINn`/`TDMAXn` (actual min/max physical value in the column),
`TLMINn`/`TLMAXn` (legal value range, e.g. histogram bounds), plus
`EXTNAME`/`EXTVER`/`EXTLEVEL`, `AUTHOR`, `REFERENC`. All §4.4.2 reserved keywords
apply here **except** `EXTEND` and `BLOCKED`.

`TDISPn` display formats (Table 20) are Fortran-style: `Aw` `Lw` `Iw.m` `Bw.m`
`Ow.m` `Zw.m` `Fw.d` `Ew.dEe` `ENw.d` `ESw.d` `Gw.dEe` `Dw.dEe` — the ASCII-table
codes plus binary/octal/hex (`B`/`O`/`Z`) and logical (`L`). Display-only metadata;
the scaled physical value (Eq. 7) is what gets formatted.

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
  Validate `byte_offset + nelem×bytes ≤ heap length` (= `PCOUNT − gap`, where
  `gap = THEAP − NAXIS1×NAXIS2`), not against `PCOUNT` directly.
- `X` (bit) columns: pack/unpack MSB-first within each byte.
