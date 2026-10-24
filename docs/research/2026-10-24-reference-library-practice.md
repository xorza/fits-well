# How cfitsio, astropy and wcslib handle the open questions of the review

Date: 2026-10-24. Sources are the clones in `/tmp/fits-compare`: cfitsio, astropy, and
the wcslib bundled at `astropy/cextern/wcslib`. Each fact is from a source read.

## Header

- **Null string on write.** cfitsio `ffs2c` (`putkey.c`) pads every string to 8
  characters, the empty string too, so `""` is written as `'        '`. Only a null
  pointer gives `''`. astropy `_format_value` (`io/fits/card.py`) writes `''` for `""`.
- **`END` with stray bytes.** cfitsio compares the first 8 bytes with `"END     "`
  (`fitscore.c`, `strncmp(rec, "END     ", 8)`), so bytes 9–80 are not checked. astropy
  accepts such a record and warns.

## Images

- **Scaling arithmetic.** cfitsio computes `dvalue = input[ii] * scale + zero` in double
  (`getcoli.c` and its siblings). A compiler that fuses `a*b+c` makes this one rounding,
  so the bits depend on the build flags. astropy (`io/fits/hdu/image.py`) calls
  `np.multiply(data, bscale, data)`, then `data += bzero`, in float32 for `BITPIX` ≤ 16
  and in float64 above. That is two roundings, in float32 for small types. No library
  gives a stable bit pattern to match.
- **Float image with `BLANK`.** cfitsio ignores it. astropy warns "The 'BLANK' keyword
  is only applicable to integer data, and will be ignored in this HDU."

## Tables

- **Bad optional column keyword.** cfitsio `ffgtbp` (`fitscore.c`) keeps the default
  when `TSCALn` or `TZEROn` is not a number, or `TNULLn` is not an integer. It writes a
  message and returns no error ("ignore this error, so don't return error status"). So
  the column reads with scale 1 and zero 0. astropy `Column._verify_keywords`
  (`io/fits/column.py`) warns "The invalid value will be ignored" for each such keyword,
  `TDIMn` and `TDISPn` included.
- **ASCII float formats.** astropy `ASCII_DEFAULT_WIDTHS` (`io/fits/column.py`): `E`
  (15, 7), `F` (16, 7), `D` (25, 17). cfitsio writes the `TFORMn` that the caller gives.

## Checksums

- **Blank `DATASUM`.** The standard (§4.4.2.7) says that a string of one or more blanks
  is an undefined or unknown value. cfitsio `ffvcks` (`checksum.c`) reads a value that
  is empty after the quotes are removed as undefined (`datastatus = 0`). It reads a
  value that is not a number through `atof`, which gives 0, and that then fails to
  match.
- **`DATASUM` on write.** cfitsio writes the data, then seeks back and updates the
  cards (`ffpcks`). astropy `_calculate_datasum` (`io/fits/hdu/image.py`) byteswaps the
  array in place, sums it, and swaps it back. For an array that is not writable, it
  sums a byteswapped copy.

## Writing

- **Append after trailing content.** cfitsio `ffcrhd` (`fitscore.c`) moves to the last
  known HDU and starts the new HDU at `headstart[maxhdu + 1]`, the end of that HDU. Any
  special records after it are written over.

## Compression

- **`ZQUANTIZ`.** cfitsio treats an absent `ZQUANTIZ` as `NO_DITHER`, and `'NONE'` as
  no quantization (`imcompress.c`).
