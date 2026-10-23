# Issues

- **`read_table` on a tiled-compressed table returns the `BINTABLE` container
  without an error**, while `read_image` decompresses a tiled image
  transparently.
- **`Header::comment` on a missing keyword does nothing and reports nothing.**
- **`Header::get` with a lowercase keyword returns `None`**, while `set`
  refuses the same keyword.
- **The README's example indexes `image_indices()[0]`**, which panics on a
  file with no image HDU.
- **`table` re-exports items of other crates**: `bitvec`'s `BitSlice`,
  `BitVec` and `Msb0`, and `num_complex`'s `Complex`.
- **`reader::source::Source` is public but unnameable**, while every
  `FitsReader<S>` method bounds `S` on it.
