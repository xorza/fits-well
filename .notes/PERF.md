# Performance journal

The measurements of the `perf-crate` work, 2026-10-25, and the ideas it rejected or left
open, with their numbers and reasons. Never retry a rejected idea without new evidence. A
kept idea is in its commit, with its numbers.

## Setup

- Host: AMD Ryzen 7 6800U (Zen 3+, 8 cores, 16 threads), 27 GB, Linux, governor
  `performance`, boost on (cannot be turned off without root). `target-cpu=x86-64-v3`
  from `.cargo/config.toml`.
- Runs: the bench binary pinned to CPU 2 (`taskset -c 2`), its SMT sibling CPU 3 left idle,
  so `parallel` cases run on one thread; one shared `CRITERION_HOME`, base
  `--save-baseline base`, cand `--baseline base`.

## Noise floor

A/A on 2026-10-25: base against its own baseline, 104 cases. Integration cases move by at
most 1.2 % (median 0.4 %). Atomic cases under 1 µs swing by up to 25 % between processes,
as code and data alignment change; the median is 0.5 %. A change is kept when an
integration case moves past 2 %, or an atomic one past its own A/A spread and an
integration case agrees.

## Numbers

Median time per iteration, CPU 2. Frame: 6000 × 4000. Start: `e6ad0b1`, saved as a
baseline. Tip: `a47d5b3`, run against it on 2026-10-25.

| Case | Start | Tip | Change |
|---|---:|---:|---:|
| `atomic/decode/i16` | 84.734ns | 84.232ns | −0.6 % |
| `atomic/decode/i32` | 305.57ns | 305.49ns | −0.1 % |
| `atomic/decode/i64` | 687.10ns | 690.91ns | +0.5 % |
| `atomic/decode/f32` | 305.79ns | 302.49ns | −0.9 % |
| `atomic/decode/f64` | 683.04ns | 688.18ns | +0.6 % |
| `atomic/encode/i16` | 191.80ns | 186.18ns | −3.4 % |
| `atomic/encode/i32` | 473.69ns | 433.09ns | −8.8 % |
| `atomic/encode/i64` | 1.0944µs | 1.0262µs | −6.2 % |
| `atomic/encode/f32` | 446.48ns | 455.18ns | +2.0 % |
| `atomic/encode/f64` | 1.0168µs | 1.0764µs | +5.6 % |
| `atomic/physical/f32/stored-u8` | 168.10ns | 198.31ns | +18.0 % |
| `atomic/physical/f64/stored-u8` | 548.36ns | 483.23ns | −12.1 % |
| `atomic/physical/f32/blank-u8` | 443.33ns | 450.12ns | +1.9 % |
| `atomic/physical/f64/blank-u8` | 829.51ns | 838.33ns | +0.9 % |
| `atomic/physical/f32/stored-i16` | 923.18ns | 928.70ns | +0.8 % |
| `atomic/physical/f64/stored-i16` | 736.94ns | 663.34ns | −10.3 % |
| `atomic/physical/f32/blank-i16` | 1.5621µs | 1.5733µs | +0.6 % |
| `atomic/physical/f64/blank-i16` | 1.0923µs | 1.0948µs | +0.3 % |
| `atomic/physical/f32/stored-i32` | 406.49ns | 483.98ns | +19.9 % |
| `atomic/physical/f64/stored-i32` | 614.53ns | 624.33ns | +1.0 % |
| `atomic/physical/f32/blank-i32` | 452.24ns | 447.11ns | −1.0 % |
| `atomic/physical/f64/blank-i32` | 1.0352µs | 984.73ns | −4.9 % |
| `atomic/physical/f32/stored-i64` | 1.3886µs | 1.4171µs | +2.5 % |
| `atomic/physical/f64/stored-i64` | 1.2726µs | 1.3734µs | +7.5 % |
| `atomic/physical/f32/blank-i64` | 2.5445µs | 2.6257µs | +3.7 % |
| `atomic/physical/f64/blank-i64` | 2.0286µs | 2.1003µs | +3.2 % |
| `atomic/physical/f32/stored-f32` | 485.02ns | 506.95ns | +4.8 % |
| `atomic/physical/f64/stored-f32` | 614.35ns | 715.94ns | +16.3 % |
| `atomic/physical/f32/stored-f64` | 841.20ns | 728.92ns | −11.8 % |
| `atomic/physical/f64/stored-f64` | 921.24ns | 907.30ns | −2.8 % |
| `atomic/unsigned/u8` | 50.047ns | 45.242ns | −8.5 % |
| `atomic/unsigned/i16` | 101.49ns | 90.061ns | −11.4 % |
| `atomic/unsigned/i32` | 323.45ns | 305.32ns | −5.7 % |
| `atomic/unsigned/i64` | 709.11ns | 674.51ns | −5.1 % |
| `atomic/checksum/i16` | 1.2794µs | 226.60ns | −82.2 % |
| `atomic/parse_header/light` | 12.630µs | 10.912µs | −14.7 % |
| `atomic/render_header/light` | 7.8495µs | 7.9146µs | +0.3 % |
| `atomic/read_column/fixed/i16` | 266.65µs | 158.96µs | −40.3 % |
| `atomic/read_column/fixed/i32` | 247.43µs | 148.31µs | −40.1 % |
| `atomic/read_column/fixed/f32` | 246.96µs | 138.35µs | −43.9 % |
| `atomic/read_column/fixed/f64` | 248.70µs | 142.69µs | −42.6 % |
| `atomic/read_column/vla/f32` | 1.8393ms | 1.7727ms | −3.4 % |
| `atomic/parse_ascii/catalog` | 526.97ns | 148.28ns | −71.9 % |
| `atomic/format_ascii/catalog` | 726.89ns | 723.90ns | +0.0 % |
| `atomic/compress/gzip_1/i16` | 57.098µs | 56.448µs | −1.1 % |
| `atomic/compress/gzip_1/i32` | 70.237µs | 68.350µs | −3.1 % |
| `atomic/compress/gzip_2/i16` | 57.202µs | 56.173µs | −1.5 % |
| `atomic/compress/gzip_2/i32` | 44.425µs | 43.960µs | −0.9 % |
| `atomic/compress/rice_1/i16` | 29.075µs | 15.663µs | −46.3 % |
| `atomic/compress/rice_1/i32` | 25.473µs | 15.047µs | −40.9 % |
| `atomic/compress/hcompress_1/i16` | 2.6457ms | 2.6265ms | −0.7 % |
| `atomic/compress/hcompress_1/i32` | 1.8773ms | 1.8674ms | −0.5 % |
| `atomic/compress/plio_1/mask` | 2.6291µs | 426.96ns | −83.9 % |
| `atomic/compress/gzip_2_lossless/f32` | 92.554µs | 97.250µs | +4.0 % |
| `atomic/decompress/gzip_1/i16` | 31.410µs | 32.024µs | +2.0 % |
| `atomic/decompress/gzip_1/i32` | 47.735µs | 50.740µs | +6.5 % |
| `atomic/decompress/gzip_2/i16` | 32.902µs | 28.392µs | −13.6 % |
| `atomic/decompress/gzip_2/i32` | 27.583µs | 18.176µs | −34.2 % |
| `atomic/decompress/rice_1/i16` | 23.903µs | 17.107µs | −28.4 % |
| `atomic/decompress/rice_1/i32` | 23.070µs | 16.755µs | −27.3 % |
| `atomic/decompress/hcompress_1/i16` | 1.7899ms | 1.4878ms | −16.9 % |
| `atomic/decompress/hcompress_1/i32` | 1.3878ms | 1.2259ms | −11.6 % |
| `atomic/decompress/plio_1/mask` | 389.24ns | 376.66ns | −2.3 % |
| `atomic/decompress/gzip_2_lossless/f32` | 53.977µs | 45.461µs | −15.7 % |
| `atomic/quantize/f32` | 69.330µs | 56.763µs | −18.1 % |
| `atomic/dequantize/f32` | 5.0681µs | 5.1004µs | +0.6 % |
| `atomic/parse_wcs/light` | 8.5467µs | 8.6001µs | +0.5 % |
| `atomic/to_world/tan` | 440.70µs | 440.70µs | −0.1 % |
| `atomic/to_world/sip` | 529.86µs | 529.57µs | −0.2 % |
| `atomic/to_world/tpv` | 1.0205ms | 1.1388ms | +10.0 % |
| `atomic/to_world/linear` | 74.705µs | 70.613µs | −5.6 % |
| `atomic/to_world/spectral` | 64.673µs | 60.521µs | −6.5 % |
| `atomic/to_world/tabular` | 400.33µs | 394.79µs | −1.1 % |
| `atomic/to_world/tabular_coupled` | 228.89µs | 223.29µs | −2.5 % |
| `atomic/to_pixel/tan` | 521.58µs | 521.08µs | +0.1 % |
| `atomic/to_pixel/sip` | 1.1393ms | 1.1407ms | +0.1 % |
| `atomic/to_pixel/tpv` | 2.6603ms | 2.6672ms | −0.1 % |
| `atomic/to_pixel/linear` | 281.52µs | 281.48µs | −0.2 % |
| `atomic/to_pixel/spectral` | 80.324µs | 79.927µs | −0.5 % |
| `atomic/to_pixel/tabular` | 451.93µs | 450.45µs | −0.1 % |
| `atomic/to_pixel/tabular_coupled` | 60.200ms | 53.595ms | −11.0 % |
| `atomic/to_mjd/utc` | 53.461µs | 53.487µs | −0.5 % |
| `atomic/to_mjd/tt` | 53.233µs | 53.439µs | −0.0 % |
| `atomic/to_mjd/tt_tai` | 53.214µs | 53.429µs | +0.4 % |
| `integration/open/stream/plain/i16` | 17.980µs | 15.718µs | −13.0 % |
| `integration/open/slice/plain/i16` | 14.210µs | 12.148µs | −13.6 % |
| `integration/open/stream/plain/mosaic` | 291.01µs | 261.57µs | −10.0 % |
| `integration/open/slice/plain/mosaic` | 226.77µs | 197.41µs | −12.5 % |
| `integration/read_image/stream/plain/u8` | 5.8419ms | 5.8556ms | +0.5 % |
| `integration/read_image/slice/plain/u8` | 3.6253ms | 3.5957ms | −0.9 % |
| `integration/read_image/stream/plain/i16` | 14.592ms | 12.697ms | −14.0 % |
| `integration/read_image/slice/plain/i16` | 6.5593ms | 6.5978ms | +0.8 % |
| `integration/read_image/stream/plain/i32` | 29.053ms | 24.624ms | −15.6 % |
| `integration/read_image/slice/plain/i32` | 12.464ms | 12.465ms | +0.1 % |
| `integration/read_image/stream/plain/i64` | 58.027ms | 48.652ms | −16.5 % |
| `integration/read_image/slice/plain/i64` | 24.410ms | 24.333ms | −0.5 % |
| `integration/read_image/stream/plain/f32` | 29.091ms | 22.311ms | −23.7 % |
| `integration/read_image/slice/plain/f32` | 12.492ms | 12.442ms | −0.4 % |
| `integration/read_image/stream/plain/f64` | 54.819ms | 43.626ms | −20.9 % |
| `integration/read_image/slice/plain/f64` | 24.377ms | 24.314ms | −0.9 % |
| `integration/read_image/stream/gzip_1/i16` | 160.84ms | 154.54ms | −3.9 % |
| `integration/read_image/slice/gzip_1/i16` | 154.08ms | 147.49ms | −4.3 % |
| `integration/read_image/stream/gzip_2/i16` | 216.76ms | 185.10ms | −14.6 % |
| `integration/read_image/slice/gzip_2/i16` | 210.10ms | 178.60ms | −15.0 % |
| `integration/read_image/stream/rice_1/i16` | 122.38ms | 91.769ms | −24.9 % |
| `integration/read_image/slice/rice_1/i16` | 117.30ms | 87.191ms | −25.6 % |
| `integration/read_image/stream/hcompress_1/i16` | 472.60ms | 410.14ms | −13.2 % |
| `integration/read_image/slice/hcompress_1/i16` | 468.18ms | 406.04ms | −13.3 % |
| `integration/read_image/stream/plio_1/mask` | 12.097ms | 8.8441ms | −26.5 % |
| `integration/read_image/slice/plio_1/mask` | 12.107ms | 8.8551ms | −26.2 % |
| `integration/read_image/stream/rice_1/f32` | 132.07ms | 113.90ms | −13.8 % |
| `integration/read_image/slice/rice_1/f32` | 129.89ms | 111.15ms | −14.4 % |
| `integration/read_image/stream/gzip_2_lossless/f32` | 269.81ms | 229.28ms | −15.0 % |
| `integration/read_image/slice/gzip_2_lossless/f32` | 255.21ms | 215.95ms | −15.4 % |
| `integration/read_section/stream/plain/u8` | 5.2256ms | 2.9482ms | −43.4 % |
| `integration/read_section/slice/plain/u8` | 3.0886ms | 1.5193ms | −50.8 % |
| `integration/read_section/stream/plain/i16` | 11.142ms | 6.8359ms | −38.5 % |
| `integration/read_section/slice/plain/i16` | 6.1583ms | 3.1012ms | −49.7 % |
| `integration/read_section/stream/plain/i32` | 22.422ms | 14.265ms | −36.4 % |
| `integration/read_section/slice/plain/i32` | 12.335ms | 6.4247ms | −47.7 % |
| `integration/read_section/stream/plain/i64` | 45.395ms | 28.626ms | −37.0 % |
| `integration/read_section/slice/plain/i64` | 24.833ms | 13.427ms | −46.0 % |
| `integration/read_section/stream/plain/f32` | 22.488ms | 11.916ms | −47.1 % |
| `integration/read_section/slice/plain/f32` | 12.356ms | 6.4603ms | −47.7 % |
| `integration/read_section/stream/plain/f64` | 42.750ms | 24.589ms | −42.3 % |
| `integration/read_section/slice/plain/f64` | 24.809ms | 13.466ms | −45.9 % |
| `integration/read_section/stream/gzip_1/i16` | 164.79ms | 154.24ms | −6.4 % |
| `integration/read_section/slice/gzip_1/i16` | 162.25ms | 152.87ms | −5.8 % |
| `integration/read_section/stream/gzip_2/i16` | 218.69ms | 185.38ms | −15.2 % |
| `integration/read_section/slice/gzip_2/i16` | 215.22ms | 183.26ms | −14.8 % |
| `integration/read_section/stream/rice_1/i16` | 121.98ms | 90.001ms | −26.2 % |
| `integration/read_section/slice/rice_1/i16` | 119.21ms | 88.874ms | −25.5 % |
| `integration/read_section/stream/hcompress_1/i16` | 1.1340s | 968.41ms | −14.6 % |
| `integration/read_section/slice/hcompress_1/i16` | 1.1297s | 964.45ms | −14.6 % |
| `integration/read_section/stream/plio_1/mask` | 12.917ms | 9.7308ms | −24.6 % |
| `integration/read_section/slice/plio_1/mask` | 12.879ms | 9.5347ms | −26.2 % |
| `integration/read_section/stream/rice_1/f32` | 125.03ms | 104.63ms | −16.2 % |
| `integration/read_section/slice/rice_1/f32` | 124.00ms | 103.46ms | −16.5 % |
| `integration/read_section/stream/gzip_2_lossless/f32` | 265.80ms | 220.23ms | −17.1 % |
| `integration/read_section/slice/gzip_2_lossless/f32` | 262.74ms | 218.46ms | −16.9 % |
| `integration/read_physical/stream/plain/u8` | 3.7024ms | 3.7492ms | +0.8 % |
| `integration/read_physical/slice/plain/u8` | 1.3437ms | 1.3583ms | +1.3 % |
| `integration/read_physical/stream/plain/i16` | 9.7212ms | 9.5963ms | −0.7 % |
| `integration/read_physical/slice/plain/i16` | 4.2720ms | 4.2788ms | +0.5 % |
| `integration/read_physical/stream/plain/i32` | 13.595ms | 13.547ms | −0.7 % |
| `integration/read_physical/slice/plain/i32` | 4.5181ms | 4.4810ms | −0.0 % |
| `integration/read_physical/stream/plain/i64` | 29.771ms | 29.506ms | −0.9 % |
| `integration/read_physical/slice/plain/i64` | 10.687ms | 10.544ms | −1.5 % |
| `integration/read_physical/stream/plain/f32` | 13.661ms | 11.566ms | −15.1 % |
| `integration/read_physical/slice/plain/f32` | 4.5427ms | 4.4561ms | −1.5 % |
| `integration/read_physical/stream/plain/f64` | 23.838ms | 21.883ms | −8.6 % |
| `integration/read_physical/slice/plain/f64` | 10.094ms | 9.0827ms | −7.9 % |
| `integration/read_physical/stream/gzip_1/i16` | 164.34ms | 157.28ms | −4.3 % |
| `integration/read_physical/slice/gzip_1/i16` | 161.05ms | 155.03ms | −3.7 % |
| `integration/read_physical/stream/gzip_2/i16` | 217.05ms | 188.23ms | −13.3 % |
| `integration/read_physical/slice/gzip_2/i16` | 214.75ms | 186.11ms | −13.3 % |
| `integration/read_physical/stream/rice_1/i16` | 122.47ms | 92.791ms | −24.2 % |
| `integration/read_physical/slice/rice_1/i16` | 120.24ms | 90.954ms | −24.4 % |
| `integration/read_physical/stream/hcompress_1/i16` | 1.1275s | 972.53ms | −13.7 % |
| `integration/read_physical/slice/hcompress_1/i16` | 1.1229s | 966.01ms | −14.0 % |
| `integration/read_physical/stream/plio_1/mask` | 14.715ms | 11.460ms | −22.5 % |
| `integration/read_physical/slice/plio_1/mask` | 14.280ms | 11.127ms | −22.5 % |
| `integration/read_physical/stream/rice_1/f32` | 124.72ms | 105.75ms | −15.3 % |
| `integration/read_physical/slice/rice_1/f32` | 123.77ms | 104.40ms | −15.8 % |
| `integration/read_physical/stream/gzip_2_lossless/f32` | 260.30ms | 220.15ms | −15.4 % |
| `integration/read_physical/slice/gzip_2_lossless/f32` | 254.05ms | 216.29ms | −14.9 % |
| `integration/verify_checksum/stream/plain/u8` | 4.9290ms | 2.8027ms | −43.0 % |
| `integration/verify_checksum/slice/plain/u8` | 2.7890ms | 1.1097ms | −59.2 % |
| `integration/verify_checksum/stream/plain/i16` | 10.659ms | 6.3024ms | −40.7 % |
| `integration/verify_checksum/slice/plain/i16` | 5.4115ms | 2.3448ms | −56.7 % |
| `integration/verify_checksum/stream/plain/i32` | 22.072ms | 13.475ms | −39.2 % |
| `integration/verify_checksum/slice/plain/i32` | 10.811ms | 4.7279ms | −56.4 % |
| `integration/verify_checksum/stream/plain/i64` | 44.664ms | 23.665ms | −45.8 % |
| `integration/verify_checksum/slice/plain/i64` | 21.575ms | 9.5897ms | −55.4 % |
| `integration/verify_checksum/stream/plain/f32` | 21.891ms | 10.996ms | −49.7 % |
| `integration/verify_checksum/slice/plain/f32` | 10.774ms | 4.7383ms | −55.9 % |
| `integration/verify_checksum/stream/plain/f64` | 41.230ms | 19.938ms | −50.5 % |
| `integration/verify_checksum/slice/plain/f64` | 21.569ms | 9.5899ms | −55.4 % |
| `integration/read_table/stream/plain/catalog` | 15.482ms | 2.1614ms | −86.8 % |
| `integration/read_table/slice/plain/catalog` | 15.257ms | 1.8644ms | −88.4 % |
| `integration/read_table/stream/gzip_1/catalog` | 32.052ms | 18.190ms | −43.1 % |
| `integration/read_table/slice/gzip_1/catalog` | 31.286ms | 17.588ms | −43.9 % |
| `integration/read_table/stream/gzip_2/catalog` | 31.331ms | 15.123ms | −51.5 % |
| `integration/read_table/slice/gzip_2/catalog` | 30.918ms | 14.693ms | −52.5 % |
| `integration/read_table/stream/rice_1/catalog` | 31.246ms | 15.386ms | −50.7 % |
| `integration/read_table/slice/rice_1/catalog` | 30.977ms | 14.814ms | −52.1 % |
| `integration/read_table_rows/stream/plain/catalog` | 14.607ms | 1.8386ms | −87.5 % |
| `integration/read_table_rows/slice/plain/catalog` | 14.329ms | 1.7363ms | −88.0 % |
| `integration/read_table_rows/stream/gzip_1/catalog` | 37.502ms | 24.576ms | −34.7 % |
| `integration/read_table_rows/slice/gzip_1/catalog` | 37.430ms | 24.427ms | −35.0 % |
| `integration/read_table_rows/stream/gzip_2/catalog` | 36.248ms | 20.527ms | −43.6 % |
| `integration/read_table_rows/slice/gzip_2/catalog` | 36.187ms | 20.271ms | −44.2 % |
| `integration/read_table_rows/stream/rice_1/catalog` | 36.212ms | 20.717ms | −43.1 % |
| `integration/read_table_rows/slice/rice_1/catalog` | 36.174ms | 20.603ms | −43.2 % |
| `integration/read_table_columns/stream/plain/catalog` | 1.8713ms | 1.5357ms | −17.7 % |
| `integration/read_table_columns/slice/plain/catalog` | 1.7451ms | 1.4278ms | −18.4 % |
| `integration/read_table_columns/stream/gzip_1/catalog` | 10.645ms | 10.305ms | −3.3 % |
| `integration/read_table_columns/slice/gzip_1/catalog` | 10.474ms | 10.097ms | −3.5 % |
| `integration/read_table_columns/stream/gzip_2/catalog` | 10.627ms | 9.1741ms | −13.6 % |
| `integration/read_table_columns/slice/gzip_2/catalog` | 10.470ms | 8.9462ms | −14.5 % |
| `integration/read_table_columns/stream/rice_1/catalog` | 10.573ms | 9.2797ms | −12.4 % |
| `integration/read_table_columns/slice/rice_1/catalog` | 10.493ms | 8.9667ms | −14.5 % |
| `integration/read_table_columns/stream/plain/spectra` | 53.767µs | 43.943µs | −17.5 % |
| `integration/read_table_columns/slice/plain/spectra` | 41.214µs | 34.668µs | −16.0 % |
| `integration/read_table_columns/stream/gzip_1/spectra` | 700.98µs | 686.09µs | −2.4 % |
| `integration/read_table_columns/slice/gzip_1/spectra` | 507.92µs | 507.40µs | +0.1 % |
| `integration/read_table_columns/stream/gzip_2/spectra` | 711.53µs | 690.52µs | −3.4 % |
| `integration/read_table_columns/slice/gzip_2/spectra` | 521.48µs | 508.67µs | −2.2 % |
| `integration/read_table_columns/stream/rice_1/spectra` | 502.05µs | 480.71µs | −4.3 % |
| `integration/read_table_columns/slice/rice_1/spectra` | 322.91µs | 305.61µs | −5.2 % |
| `integration/read_table_cells/stream/plain/catalog` | 105.63µs | 106.72µs | −0.4 % |
| `integration/read_table_cells/slice/plain/catalog` | 18.346µs | 17.052µs | −6.8 % |
| `integration/read_table_cells/stream/gzip_1/catalog` | 6.6066ms | 6.6415ms | +0.6 % |
| `integration/read_table_cells/slice/gzip_1/catalog` | 5.9343ms | 5.8884ms | −0.3 % |
| `integration/read_table_cells/stream/gzip_2/catalog` | 7.2462ms | 6.5774ms | −8.7 % |
| `integration/read_table_cells/slice/gzip_2/catalog` | 6.6704ms | 5.9444ms | −10.6 % |
| `integration/read_table_cells/stream/rice_1/catalog` | 7.2999ms | 6.5627ms | −10.1 % |
| `integration/read_table_cells/slice/rice_1/catalog` | 6.6455ms | 5.8954ms | −10.6 % |
| `integration/read_table_cells/stream/plain/spectra` | 271.78µs | 272.55µs | +0.6 % |
| `integration/read_table_cells/slice/plain/spectra` | 30.599µs | 30.054µs | −1.6 % |
| `integration/read_table_cells/stream/gzip_1/spectra` | 236.99ms | 239.83ms | +1.2 % |
| `integration/read_table_cells/slice/gzip_1/spectra` | 63.220ms | 63.711ms | +0.8 % |
| `integration/read_table_cells/stream/gzip_2/spectra` | 203.36ms | 204.80ms | +0.7 % |
| `integration/read_table_cells/slice/gzip_2/spectra` | 57.561ms | 49.040ms | −15.0 % |
| `integration/read_table_cells/stream/rice_1/spectra` | 202.06ms | 202.64ms | +0.3 % |
| `integration/read_table_cells/slice/rice_1/spectra` | 57.241ms | 48.750ms | −14.8 % |
| `integration/read_ascii_table/stream/plain/catalog` | 53.082ms | 18.012ms | −66.3 % |
| `integration/read_ascii_table/slice/plain/catalog` | 52.152ms | 17.472ms | −66.6 % |
| `integration/write_image/file/plain/u8` | 12.263ms | 9.8762ms | −19.3 % |
| `integration/write_image/memory/plain/u8` | 6.4653ms | 4.2640ms | −34.1 % |
| `integration/write_image/file/plain/i16` | 25.015ms | 20.586ms | −17.7 % |
| `integration/write_image/memory/plain/i16` | 14.115ms | 9.6370ms | −31.7 % |
| `integration/write_image/file/plain/i32` | 50.208ms | 41.207ms | −18.0 % |
| `integration/write_image/memory/plain/i32` | 28.604ms | 19.680ms | −31.2 % |
| `integration/write_image/file/plain/i64` | 101.73ms | 83.340ms | −18.0 % |
| `integration/write_image/memory/plain/i64` | 58.169ms | 40.293ms | −31.9 % |
| `integration/write_image/file/plain/f32` | 50.272ms | 41.234ms | −18.3 % |
| `integration/write_image/memory/plain/f32` | 28.741ms | 19.736ms | −31.3 % |
| `integration/write_image/file/plain/f64` | 101.50ms | 83.148ms | −17.7 % |
| `integration/write_image/memory/plain/f64` | 57.999ms | 39.245ms | −32.8 % |
| `integration/write_image/file/gzip_1/i16` | 260.73ms | 255.20ms | −2.1 % |
| `integration/write_image/memory/gzip_1/i16` | 254.16ms | 247.95ms | −2.4 % |
| `integration/write_image/file/gzip_2/i16` | 325.79ms | 316.22ms | −2.9 % |
| `integration/write_image/memory/gzip_2/i16` | 320.97ms | 311.30ms | −3.0 % |
| `integration/write_image/file/rice_1/i16` | 198.55ms | 88.269ms | −55.5 % |
| `integration/write_image/memory/rice_1/i16` | 193.54ms | 80.969ms | −57.8 % |
| `integration/write_image/file/hcompress_1/i16` | 696.65ms | 690.99ms | −0.8 % |
| `integration/write_image/memory/hcompress_1/i16` | 690.44ms | 683.73ms | −1.0 % |
| `integration/write_image/file/plio_1/mask` | 19.084ms | 4.6837ms | −75.6 % |
| `integration/write_image/memory/plio_1/mask` | 18.931ms | 4.6561ms | −75.6 % |
| `integration/write_image/file/rice_1/f32` | 481.84ms | 320.67ms | −33.4 % |
| `integration/write_image/memory/rice_1/f32` | 478.57ms | 315.32ms | −34.1 % |
| `integration/write_image/file/gzip_2_lossless/f32` | 434.54ms | 425.91ms | −2.0 % |
| `integration/write_image/memory/gzip_2_lossless/f32` | 426.66ms | 416.60ms | −2.4 % |
| `integration/write_image_stream/file/plain/u8` | 11.082ms | 8.8563ms | −20.2 % |
| `integration/write_image_stream/memory/plain/u8` | 5.4736ms | 3.1916ms | −41.5 % |
| `integration/write_image_stream/file/plain/i16` | 22.554ms | 18.420ms | −18.4 % |
| `integration/write_image_stream/memory/plain/i16` | 11.677ms | 6.8984ms | −41.0 % |
| `integration/write_image_stream/file/plain/i32` | 45.778ms | 36.931ms | −19.6 % |
| `integration/write_image_stream/memory/plain/i32` | 23.543ms | 14.550ms | −38.5 % |
| `integration/write_image_stream/file/plain/i64` | 90.708ms | 74.346ms | −17.5 % |
| `integration/write_image_stream/memory/plain/i64` | 46.658ms | 28.514ms | −38.3 % |
| `integration/write_image_stream/file/plain/f32` | 45.864ms | 36.921ms | −19.3 % |
| `integration/write_image_stream/memory/plain/f32` | 23.609ms | 14.495ms | −39.0 % |
| `integration/write_image_stream/file/plain/f64` | 90.513ms | 74.131ms | −17.9 % |
| `integration/write_image_stream/memory/plain/f64` | 46.602ms | 28.534ms | −38.5 % |
| `integration/write_table/file/plain/catalog` | 3.7167ms | 2.9908ms | −20.6 % |
| `integration/write_table/memory/plain/catalog` | 2.2495ms | 1.7917ms | −20.3 % |
| `integration/write_table/file/gzip_1/catalog` | 27.301ms | 26.803ms | −1.8 % |
| `integration/write_table/memory/gzip_1/catalog` | 26.238ms | 25.706ms | −2.1 % |
| `integration/write_table/file/gzip_2/catalog` | 24.589ms | 23.636ms | −3.9 % |
| `integration/write_table/memory/gzip_2/catalog` | 23.230ms | 22.669ms | −2.3 % |
| `integration/write_table/file/rice_1/catalog` | 24.153ms | 23.291ms | −3.4 % |
| `integration/write_table/memory/rice_1/catalog` | 23.013ms | 22.281ms | −3.0 % |
| `integration/write_ascii_table/file/plain/catalog` | 97.579ms | 96.783ms | −0.6 % |
| `integration/write_ascii_table/memory/plain/catalog` | 96.536ms | 92.845ms | −3.7 % |
| `integration/edit_header/file/plain/i16` | 44.495µs | 39.678µs | −10.9 % |

The tip moves no integration case past +2 %. Atomic cases under 2 µs move by up to +20 %,
within their A/A spread, and reruns of them gave −2 % to +7 %. `atomic/to_world/tpv` moves
+4 % to +10 % in each run, though no commit touches TPV; `atomic/to_pixel/tpv`, which runs
the same forward polynomial, moves −0.1 %: code placement.

## Rejected and open ideas

| Idea | Evidence | Result | Verdict |
|---|---|---|---|
| Rice v1: refill-once reader, block kind decided once | `rice::decode` 47 % | `read_image rice_1/i16` +3.8 %, `rice_1/f32` +8.1 % | rejected: `read` calls itself for reads past 56 bits, so it cannot inline |
| HCOMPRESS: skip zero words in `qtree_expand` | its byte scan for nonzero entries is the hottest loop of `qtree_decode` | `read_image hcompress_1` −1.6 %, `read_section` −2.1 % | rejected: within the noise floor |
| Plain encode: append without the zero fill of `resize` | `extend_be` fills, then overwrites | not tried: it needs `unsafe` code for about 5 % of a plain write | rejected |
| ASCII write: a faster `Fw.d`/`Ew.d` formatter | `dragon`/`grisu::format_exact` 35 % of `write_ascii_table` | not tried: an exact fixed-precision formatter is `core`'s own Grisu with Dragon fallback; a faster exact one is a large port | rejected for now |
| Noise: `u32` patterns for an f32 image's medians | selection 33 % of the quantizer; a row's `u64` patterns are 48 KB, past the 32 KB L1 | `atomic/quantize/f32` +20 %, `write_image/memory/rice_1/f32` +15 % | rejected |
| Stream reads: one swap of the whole run after its read, the code before `2c944d7` | the swap is a second pass through memory | `read_image/stream/plain/i16` 15.10 ms against 12.67 ms for a swap of each 256 KiB as it lands (16 % slower), all multi-byte types alike | rejected (`docs/design.md`, "A plain read lands in the buffer it returns") |
| Stream staging: 256 KiB chunks, not 1 MiB | the 1 MiB chunk exceeds the 512 KiB L2 | `read_physical/stream`, `verify_checksum/stream`: −2 % to +12 % | rejected |
| HCOMPRESS encode: all bit planes of a quadrant in one pass | `qtree_onebit` 34 %, `qtree_encode` 31 %: each plane reads the whole quadrant | not tried: a restructure of cfitsio's encoder for a codec writers seldom choose | rejected for now |
| Header index: a faster hasher than SipHash | `hash_one` + SipHash 13 % of `parse_header` | not tried: the keys come from untrusted files, and SipHash's random keys defend the index against crafted collisions | rejected |
| HCOMPRESS: branch-free sign bits in `dodecode` | the sign loop is `dodecode`'s 18 % of a decode | not tried: it must restore the byte reader's exact state afterwards, for a codec writers seldom choose | rejected for now |
| HCOMPRESS: a faster `hinv` | `hinv` 24 % of a decode | not tried: its rounding checks each coefficient for overflow, which the codec needs on untrusted streams | rejected |
| Quantizer: `has_null` as a fold | an `any` scan of each float tile | `atomic/quantize/f32` −3 %, `write_image rice_1/f32` −2 % to +4 % | rejected: no integration case agrees |
| Integer encode on the stored type, not a widened `i64` plane | widening `u8` to `i64` is 47 % of a PLIO write; the run scan reads 8 bytes a pixel | not tried: every integer codec encodes from the shared `i64` plane, so each would take four sample types | rejected for now |
| Null mask: its 0-or-1 check as a fold | an `any` scan of each mask tile | not tried: no bench reads a masked image | rejected for now |
| gzip: flate2's `zlib-rs` backend, approved by the owner | inflate 31–58 % of a gzip read | `read_image/slice/gzip_2/i16` −26 %, `write_image/memory/gzip_2/i16` −22 %, `read_table_cells/slice/gzip_1/spectra` 63.2 → 1.37 ms (−98 %: its runs are distance-1 matches, which `miniz_oxide` copies a byte at a time); but `read_image gzip_2_lossless/f32` +11 %, `read_table gzip_2/catalog` +10 %, `write_image gzip_1/i16` +4 %, `atomic/decompress/gzip_2/i32` +85 %; its deflate splits blocks differently from `GzEncoder`, so `encode_matches_gz_encoder_with_one_scratch` fails | rejected: it regresses as many cases as it speeds up |
| Compressed sections: keep the decoded tiles the next section needs | `read_section/slice/hcompress_1/i16` 964 ms against 406 ms for `read_image`: a 10-row section cuts the 16-row tiles, so 8 sections decode 12 tiles for each 5 | not tried: a caller aligns its sections to `ImageReader::tiles()`, as lumos's `chunk_rows` does, and each tile decodes once; the `read_section` and `read_physical` benches now align too, so their later numbers are not comparable to this table's | rejected (`docs/design.md`, "A section decodes the tiles it touches and keeps none for the next") |
