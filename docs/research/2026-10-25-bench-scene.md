# The bench scene: a light as its header describes it

Measured on 2026-10-25, on the development machine (`cargo drive bench`, profile `bench`),
before and after the scene of `src/internals/scene.rs` changed.

## Why it changed

- The old scene added white noise of σ = 0.01 of full scale: 655 ADU on the 16-bit light.
- Its header says `EGAIN = 0.25` e⁻/ADU, and the sky is 0.1 of full scale, 6553.5 ADU =
  1638.4 e⁻. Photon noise is √1638.4 = 40.5 e⁻; with a read noise of 1.5 e⁻,
  √(1638.4 + 2.25) = 40.5 e⁻ = 162 ADU = 0.00247 of full scale. The old noise was 4×
  that, about 2 bits a sample more for every codec.
- The old stars were single pixels of uniform random amplitude on 1 pixel in 50, apart from
  the catalog; the old mask marked those 2 % of scattered pixels, the worst input for
  `PLIO_1`'s runs.
- The new scene draws one star list (100 000 stars, a Euclidean `N(> F) ∝ F^−1.5` flux law
  above a 0.01 peak), renders each as a Gaussian of FWHM 2.5 px, adds photon and read noise
  per pixel, and writes the same list as the catalog. The mask marks one pixel in 2000 hot,
  three bad columns and the saturated star cores.

## Compressed sizes (bytes, with checksums and the light header)

| File | Before | After |
|---|---:|---:|
| `gzip_1/catalog` | 4,561,920 | 4,559,040 |
| `gzip_1/i16` | 40,109,760 | 35,464,320 |
| `gzip_2/catalog` | 4,034,880 | 4,032,000 |
| `gzip_2/i16` | 40,631,040 | 36,276,480 |
| `gzip_2_lossless/f32` | 73,915,200 | 68,967,360 |
| `hcompress_1/i16` | 40,582,080 | 30,617,280 |
| `plain/catalog` | 5,405,760 | 5,405,760 |
| `plain/f32` | 96,007,680 | 96,007,680 |
| `plain/f64` | 192,006,720 | 192,006,720 |
| `plain/i16` | 48,006,720 | 48,006,720 |
| `plain/i32` | 96,007,680 | 96,007,680 |
| `plain/i64` | 192,006,720 | 192,006,720 |
| `plain/u8` | 24,007,680 | 24,007,680 |
| `plio_1/mask` | 1,068,480 | 158,400 |
| `rice_1/catalog` | 4,063,680 | 4,060,800 |
| `rice_1/f32` | 17,447,040 | 14,754,240 |
| `rice_1/i16` | 38,462,400 | 30,738,240 |

`RICE_1` stores the light at 80 % of its plain size before and 64 % after; `PLIO_1` stores
the mask in 1 068 480 bytes before and 158 400 after.

## Timings (criterion medians)

| Case | Before | After |
|---|---:|---:|
| `atomic/compress/gzip_1/i16` | 72.301 µs | 72.429 µs |
| `atomic/compress/gzip_2/i16` | 75.789 µs | 67.974 µs |
| `atomic/compress/hcompress_1/i16` | 3.2316 ms | 2.8281 ms |
| `atomic/compress/plio_1/u8` | 6.6115 µs | 5.7847 µs |
| `atomic/compress/rice_1/i16` | 28.875 µs | 30.311 µs |
| `atomic/decompress/gzip_1/i16` | 31.524 µs | 31.491 µs |
| `atomic/decompress/gzip_2/i16` | 35.351 µs | 32.830 µs |
| `atomic/decompress/hcompress_1/i16` | 2.0380 ms | 1.7029 ms |
| `atomic/decompress/plio_1/u8` | 802.14 ns | 384.22 ns |
| `atomic/decompress/rice_1/i16` | 25.529 µs | 25.437 µs |
| `integration/read_image/slice/gzip_1/i16` | 20.502 ms | 20.567 ms |
| `integration/read_image/slice/gzip_2/i16` | 25.430 ms | 27.568 ms |
| `integration/read_image/slice/gzip_2_lossless/f32` | 37.039 ms | 36.352 ms |
| `integration/read_image/slice/hcompress_1/i16` | 78.547 ms | 70.295 ms |
| `integration/read_image/slice/plain/f32` | 12.658 ms | 12.550 ms |
| `integration/read_image/slice/plain/f64` | 24.634 ms | 24.441 ms |
| `integration/read_image/slice/plain/i16` | 6.5938 ms | 6.6264 ms |
| `integration/read_image/slice/plain/i32` | 12.632 ms | 12.670 ms |
| `integration/read_image/slice/plain/i64` | 24.602 ms | 24.563 ms |
| `integration/read_image/slice/plain/u8` | 3.7051 ms | 3.6348 ms |
| `integration/read_image/slice/plio_1/mask` | 5.0539 ms | 4.8775 ms |
| `integration/read_image/slice/rice_1/f32` | 28.062 ms | 27.829 ms |
| `integration/read_image/slice/rice_1/i16` | 22.839 ms | 22.617 ms |
