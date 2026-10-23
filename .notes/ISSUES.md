# Issues


- A tile-compressed float image with no `ZSCALE`/`ZZERO` columns, which §10.2 defines as losslessly compressed, decodes its stored float bytes as quantized integers: the tile of f32 `[1.5, -2.25, …]` reads as `[1069547500.0, -1072693250.0, …]`, with no error (`src/compress/decode/tile_decoder.rs`, `src/compress/decode/float_quantization.rs`).
- A tile-compressed float image with `ZQUANTIZ = 'NONE'`, which cfitsio and fpack write for lossless float compression, fails with `UnsupportedCompression { name: "float quantization NONE" }` (`src/compress/decode/float_quantization.rs:36-43`).
- `write_compressed_image` with `Compression::Plio` on an unsigned 16-bit image fails with `PlioValueOutOfRange { value: -32768 }` for values below 32768. cfitsio stores a u16 image's PLIO stream offset by 32768 and removes the offset on read; fits-well applies no offset, so such a cfitsio file does not read back as its physical values.
- `docs/design.md` cites `docs/research/2026-10-23-wcs-into-slice.md` twice, and `docs/research/` is empty.
