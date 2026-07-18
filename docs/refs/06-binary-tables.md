# 6. Binary Table Extension (Standard §7.3)

`XTENSION = 'BINTABLE'`. The workhorse FITS table: rows of fixed-width binary
records, columns of typed (optionally array-valued) cells, plus an optional
**heap** for variable-length arrays. This is where most FITS performance work
lives.

## 6.1 Data layout

- Main data table: `NAXIS2` rows, each `NAXIS1` bytes (`BITPIX = 8`, `NAXIS = 2`).
- Row width `NAXIS1 = Σ_n (r_n × b_n)` over the `TFIELDS` columns, where `r_n` is
  the repeat count and `b_n` the element size of column n's `TFORMn` (`X` uses
  `ceil(r/8)`). Fields occur contiguously in increasing column order with no
  gaps, padding, or word-alignment requirement.
- After the main table comes the **heap** (variable-length array storage),
  optionally offset by `THEAP` from the start of the data unit.
- `PCOUNT` = size of the supplemental data area (gap + heap), in bytes. `GCOUNT = 1`.
- All numeric data big-endian; same encodings as [§5](03-data-representation.md).
- The last block after the supplemental area's last byte is filled with zero bits.

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

The first eight records, from `XTENSION` through `TFIELDS`, must be consecutive
and in exactly that order. `TFORMn` is then required for every column but is not
part of that fixed prefix; `TTYPEn` is optional but strongly recommended.
`NAXIS1`, `NAXIS2`, and `PCOUNT` are non-negative; `TFIELDS` is 0–999.

## 6.3 `TFORMn` data types (Table 18)

Format is `rTa`: optional **repeat count** `r` (non-negative integer, default 1),
a single **type code** `T`, and optional trailing chars `a` (undefined by spec).

| Code | Description | Bytes/elem |
|:----:|-------------|:----------:|
| `L` | Logical (ASCII `T`, ASCII `F`, or NUL byte `0x00`) | 1 |
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
- In an `X` cell, bits are MSB-first. Unused low-order bits in its final byte
  must be zero; bit arrays have no null representation.
- In `C`/`M`, each real component precedes its imaginary component. If either is
  NaN, the complete complex element is null.

## 6.4 Scaling & nulls

- Physical value: `physical = TZEROn + TSCALn × stored` (Eq. 7).
  Must **not** be applied to `A`, `L`, `X` columns.
- For complex `C`/`M`, `TSCALn` has zero imaginary part and scales both stored
  components, while `TZEROn` has zero imaginary part and offsets only the real
  component: `(TZERO + TSCAL×re) + i(TSCAL×im)`.
- For `P`/`Q`, scaling applies to heap array values, not the descriptor.
- **Unsigned integers** (Table 19): `TSCALn = 1` plus `TZEROn` =

  | `TFORMn` | Native (stored) | Physical | `TZEROn` |
  |:--------:|------------------|----------|----------|
  | `B` | unsigned | signed byte | `-128` (−2⁷) |
  | `I` | signed | unsigned 16-bit | `32768` (2¹⁵) |
  | `J` | signed | unsigned 32-bit | `2147483648` (2³¹) |
  | `K` | signed | unsigned 64-bit | `9223372036854775808` (2⁶³) |

- `TNULLn` = the **raw stored** integer denoting undefined, for `B`/`I`/`J`/`K`
  columns and for integer array elements pointed to by `P`/`Q`; it does not null
  the descriptor itself and is forbidden on other types. It is matched before
  Eq. 7, not against the physical value — e.g. an unsigned-16 column
  (`TZEROn = 32768`) whose physical zero means undefined needs
  `TNULLn = -32768`. Float/complex columns use IEEE NaN instead.

## 6.5 Multidimensional cells — `TDIMn`

A vector cell can be reshaped into an N-D array via `TDIMn = '(d1,d2,…)'`. The
product `Π di` *must be ≤* `r` (the `TFORMn` repeat count; for `P`/`Q`, ≤ the
descriptor's stored array length); any trailing unused elements are undefined fill.
The shape is not applicable to a `P`/`Q` descriptor whose stored array length is
zero. Fortran order (d1 fastest). String arrays use the same notation:
`TFORMn = '60A'` with `TDIMn = '(5,4,3)'` is a 4×3 array of 5-character strings.

## 6.6 Variable-length arrays (§7.3.5–7.3.6)

Columns of type `P`/`Q` store a fixed-size **array descriptor** in the row and the
actual data in the heap.

- Descriptor layout: `(nelem, byte_offset)` — two 32-bit **signed** ints for `P`,
  two 64-bit **signed** ints for `Q`. `byte_offset` is zero-indexed from the start
  of the heap. (Repeat count on the column itself is 0 or 1 only — see §6.3.)
- `TFORMn = 'rPt(emax)'` / `'rQt(emax)'`: `t` is the element type code (any type but
  `P`/`Q`). The parenthesized `emax` may be omitted; when present it is guaranteed
  to be at least the maximum element count actually stored in any row (it aids
  preallocation but imposes no additional storage limit). Extra trailing chars
  after `(emax)` are allowed.
- Heap begins `THEAP` bytes from the start of the data unit (default *and minimum* =
  end of the main table, `NAXIS1 × NAXIS2`); a larger `THEAP` leaves a gap before
  the heap. `PCOUNT` counts gap + heap. `THEAP` must not appear when `PCOUNT = 0`.
- Zero-length array (`nelem = 0`): no heap data, `byte_offset` is undefined and
  should be written as 0. Negative values have undefined meaning; negative
  offsets are expressly forbidden. Storage is contiguous and every non-empty
  referenced span must lie entirely within the heap.
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
Undefined entries and IEEE special values are excluded from actual
`TDMINn`/`TDMAXn` extrema.

`TDISPn` display formats (Table 20) are Fortran-style: `Aw` `Lw` `Iw.m` `Bw.m`
`Ow.m` `Zw.m` `Fw.d` `Ew.dEe` `ENw.d` `ESw.d` `Gw.dEe` `Dw.dEe` — the ASCII-table
codes plus binary/octal/hex (`B`/`O`/`Z`) and logical (`L`). Display-only metadata;
the scaled physical value (Eq. 7) is what gets formatted. Every byte of an `X`
or `B` array is treated as unsigned for display; `P`/`Q` formats describe the
pointed-to data, not the descriptor. Complex real and imaginary parts use the
same real format and should be comma-separated in parentheses.

## Implementation notes (this library)

- Precompute per-column byte offset within a row and element size at header-parse
  time; store as a column-descriptor table. Row access is then offset arithmetic.
- **Column-oriented reads**: striding by `NAXIS1` to gather one column is
  cache-unfriendly; for analytic workloads provide a transpose/columnar
  materialization, and SIMD-gather where strides allow. Row reads are contiguous.
- Endian swap + `TSCAL/TZERO` has a raw fast path when
  `TSCALn==1 && TZEROn==0` and types match host. Scaling and swapping are
  memory-bound; reuse destination buffers instead of assuming SIMD or threading
  will improve them.
- Unsigned detection mirrors images: integer `TFORM` + `TZEROn == 2^(n-1)` +
  `TSCALn == 1` ⇒ expose exact `uN` values through `unsigned` or, for P/Q heap
  arrays, one jagged `UnsignedData` per row through `vla_unsigned`.
- Heap/VLA: parse descriptors lazily; expose per-row array slices into the heap.
  Validate `byte_offset + nelem×bytes ≤ heap length` (= `PCOUNT − gap`, where
  `gap = THEAP − NAXIS1×NAXIS2`), not against `PCOUNT` directly.
- Physical VLA access is type-specific: `vla_physical` for real numeric arrays,
  `vla_complex` for complex arrays with `TZEROn` applied only to the real
  component, and `vla_bits` for packed jagged bit arrays.
- `X` (bit) columns: pack/unpack MSB-first within each byte. `vla_bits` writing
  accepts one exact-length `BitVec` per row, stores descriptor counts in bits,
  advances heap offsets in bytes, and clears unused low bits in the final byte.
