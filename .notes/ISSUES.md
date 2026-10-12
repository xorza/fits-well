# Issues

- **HCOMPRESS_1 does not round-trip losslessly for some tile shapes.** An `i16` image
  written by `write_compressed_image` with `Compression::Hcompress(Hcompress::default())`
  (scale 0) and read back by `read_image(1).decode()` differs from the original in some
  samples, whatever the sample values: one tile of 17×1 (9 of 17 samples wrong), 17×17
  (9 of 289), 100×100 (80 of 10 000) and 6000×1 (3952 of 6000; the default row tiles of
  a 6000-wide frame) fail, while 16×1, 24×16 and 64×64 round-trip. The tile functions
  `hcompress_tile_encode` and `hcompress_tile_into` show the same on one 6000×1 tile
  (`cargo benches -- --exact atomic/decompress/hcompress_1/i16`).
