# What a compressed read holds, and what it costs

Measured 2026-10-24 on an AMD Ryzen 7 6800U, rustc 1.99.1, flate2 1.1.10 with
miniz_oxide 0.9.1, with `cargo drive bench` (the `bench` profile, `--all-features`).
The cases are `src/compression/bench.rs`: the standard frame in each codec's default
tiles, read whole or ten rows a section with the checksum judged, from memory (`slice`)
or from a file (`stream`).

- **Before:** commit `3ed0326`. A section reads its tiles' container rows and heap cells
  and parses the per-tile columns again; each rayon split of a decode makes a tile set
  that grows as tiles fill it; each gzip tile makes a `GzDecoder`; a compressed section
  feeds nothing to the checksum, so the finish reads the whole data unit again.
- **After:** the working tree of this batch. The tile index is read once at open; each
  set reserves the largest tile's buffers before its first tile; one `Decompress` a set
  inflates every gzip tile; a stream's sections feed the checksum.

## Allocations

- A `flate2::read::GzDecoder` allocates a 32 768-byte read buffer
  (`BufReader::with_buf(vec![0; 32 * 1024], …)`) and the boxed miniz_oxide
  `InflateState`, 43 296 bytes, measured by the allocator of `tests/support/peak.rs`.
- The old decode did both for every gzip tile.

## Speed

The median of criterion's estimate, in ms. The RICE rows are from a run of their own: two
runs of the old tree around one of the new; the old column is their mean, whose two runs
differ by under 4%. The other rows are from one run of each tree.

| Case | Before | After | Change |
|---|---:|---:|---:|
| `read_compressed/slice/rice_1/i16` | 22.05 | 21.80 | −1.2% |
| `read_compressed/stream/rice_1/i16` | 31.52 | 31.21 | −1.0% |
| `read_compressed_section/slice/rice_1/i16` | 45.60 | 44.62 | −2.1% |
| `read_compressed_section/stream/rice_1/i16` | 58.37 | 52.95 | −9.3% |
| `read_compressed_section/stream/rice_1/f32` | 54.77 | 52.28 | −4.6% |
| `read_compressed_section/slice/gzip_1/i16` | 44.34 | 43.58 | −1.7% |
| `read_compressed_section/stream/gzip_2/i16` | 66.99 | 62.37 | −6.9% |
| `read_compressed_section/stream/plio_1/u8` | 12.45 | 11.40 | −8.5% |
| `read_compressed_section/slice/hcompress_1/i16` | 1011.8 | 1030.8 | +1.9% |
| `read_compressed/slice/gzip_1/f32` | 42.76 | 44.09 | +3.1% |
| `read_compressed/stream/gzip_1/f32` | 46.78 | 48.63 | +4.0% |
| `atomic/decompress/gzip_2/i16` | 0.038 | 0.036 | −5.0% |

- Every other image case is within ±3%.
- A stream's sections gain most: the finish no longer reads the data unit again.

## Two costs found on the way

- **Summing a resident source between section decodes.** With a resident source's
  section cells fed to the checksum, `read_compressed_section/slice/rice_1/i16` took
  53.88 ms against 44.80 and 46.39 for the old tree, and 46.49 with the feed taken out.
  A profile showed the time in rayon waking its workers (`wake_specific_thread`, 1.2%,
  absent before) and in the kernel (15.3% against 13.9%): the sum ran on the calling
  thread between waves, and the workers slept and woke for each. A resident source is
  summed at the finish in place, with no read, so its sections no longer feed it.
- **RICE's loop inlined into `TileCodec::decode_ints`.** `atomic/decompress/rice_1/i16`,
  which calls only that function and whose codec this batch does not change, took 27.6 µs
  against 25.2 µs in three alternated runs of each tree. `decode_ints` was 100 bytes
  smaller after the gzip arm changed. With `rice::Codec::decode` out of line, it took
  25.0–25.2 µs against 25.2–25.5 µs.

## The bound

`tests/view_bytes.rs` and `tests/view_bytes_parallel.rs` hold `ImageReader::view_bytes`
to the allocator over row, square and single tiles through every codec, from memory and
from a stream, in two orders of regions.

- **On four workers** the peak never passes the figure.
- **On one worker**, over the order that grows each kept buffer once, the figure is the
  peak plus, for a container with a gzip column, the room the 64 KiB inflate bound
  leaves, and, from a stream, at most the 2 880-byte block the header scan left in the
  staging buffer.
- **The finish** of a sum that holds nothing of the views stays within `finish_bytes`.
