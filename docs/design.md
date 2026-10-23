# fits-well design

The decisions that shape the public API. The code and its doc comments show how
each one works. Sources: the FITS 4.0 standard (`docs/refs/fits_standard40.md`)
and the dated review in `.notes/API_REVIEW.md`.

## Reading

- **One handle reads an image: `FitsReader::image` gives an `ImageReader`.** A
  `Region` picks the whole image or one range per axis, and the handle reads it
  three ways: `view` into the caller's `words`, `read` into an owned `Image`, and
  `physical_into`. The handle resolves the geometry and a compressed image's codec
  and tiles once. Refused: one method per combination of whole or section, owned
  or borrowed, and summed or not (six methods, with the checksum as a suffix), and
  `Lazy`, a second form of the same read.
- **A checksum is summed by the handle that reads.** With `with_checksum`, a read
  feeds the stored bytes it fetches, as long as they continue the sum: a plain
  image's whole reads and sections, and a compressed image's whole reads. A
  compressed section's tiles are not fed. `finish` reads only what the reads did
  not feed, and without `with_checksum` it reads the whole unit. Refused: a caller-held sum passed to each read, which a caller could finish
  against another HDU.
- **A physical read appends to the caller's `Vec`.** `physical_into` extends `out`,
  so a caller can append chunk after chunk to one plane, and nothing is zeroed
  first. A plain image converts from its stored big-endian bytes in one pass.
  Refused: a `&mut [T]` that the caller must size and zero, and a new `Vec` for each
  call.
- **`Borrowed` is the borrowed image, the twin of `Image`.** Every read result is one,
  `Image::view` lends one, and the conversions (`physical_into`, `unsigned`,
  `sample_type`) live on it alone. `View` has a typed accessor for each type, and
  `Data` converts back to its `Vec`. Refused: the same conversions on `Image`, `Lazy`
  and `Borrowed`, each with its own code.
- **The caller owns the image-sized buffers.** `view` fills the caller's `words`,
  and the reader keeps only staging: a section's stored bytes, and the words a
  compressed image's physical read decodes into, both the size of the region read.
- **Names do not repeat their module.** The borrowed image is `image::Borrowed`, and
  the handle is `fits_well::ImageReader`, beside `FitsReader`. Refused:
  `image::ImageView` and `image::ImageReader`.
- **A binary-table read never gives a compressed container.** A tiled-compressed
  image or table is stored as a `BINTABLE`, but `read_table`, `read_table_rows`,
  `read_table_columns`, `read_table_cell` and `table_schema` refuse it:
  `NotABinTable` for an image, `TableIsCompressed` for a table. Refused: handing
  back the container, which reads as a table of compressed cells and is quietly
  wrong. A compressed table reads whole through `read_compressed_table`. Source:
  §10.1, §10.3.
- **`Hdu::is_image` classifies by rank alone.** An HDU is an image when it is a
  primary array, an image extension or a tiled-compressed image whose `NAXIS`
  (`ZNAXIS`) is above 0. Refused: the kind alone, which counted an image
  extension with `NAXIS = 0` but not such a primary. Also refused: requiring the
  whole geometry to resolve, which would turn a malformed image into "not an
  image" and hide the error that `Hdu::image` reports. Source: §4.4.1.
- **The `Source` trait is sealed and cannot be named.** A source is not an
  extension point, and no caller is generic over sources. Refused: exporting it
  before such a caller exists.

## Header

- **Editing the comment of an absent card fails with `KeywordNotFound`.**
  Refused: a no-op, which loses a misspelled keyword without a trace.

## Crate boundary

- **No re-export of another crate's items.** A caller that names `bitvec`'s
  `BitSlice`, `BitVec` or `Msb0`, or `num_complex`'s `Complex`, depends on that
  crate itself, at the version its workspace pins. Refused: re-exports, which tie
  callers to fits-well's versions. Also refused: types of fits-well's own, as
  these are the ecosystem's standard types.
- **fits-well maps no file.** A caller that wants a mapped file maps it and
  reads it with `FitsReader::from_bytes`, which decodes from the borrowed bytes
  without a copy. Refused: a safe `open_mmap`, which is unsound, because another
  process that truncates or rewrites a mapped file makes its reads undefined.
  `memmap2::Mmap::map` is `unsafe` for that reason. Also refused: an `unsafe fn
  open_mmap`, which would duplicate the caller's own mapping layer (Darkroom's
  `common::MappedFile`) and keep a feature and a dependency.
