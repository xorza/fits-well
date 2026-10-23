# Issues


- A tiled-compressed float image reads a signaling NaN back as a quiet NaN: its tiles decode through an `f64` plane (`src/compress/decode/wide_plane.rs`), and the `f32` → `f64` conversion sets the quiet bit.
- A TAN `Wcs` round trip `world_to_pixel(pixel_to_world(p))` misses `p` by about 7e-12° in world terms: 2e-8 pixel at a corner of astropy's IRAC header (`test_resources/wcs_irac_sip.hdr` without its SIP keywords), and 2.2e-6 pixel at an offset of 1e5 pixels of 1e-6°, where the spacing of a double is far smaller.
- `atomic/wcs/to_pixel/tabular` takes 457 ms a row from commit `e1dcdea` on, and 394 ms at `e1dcdea^`; that commit does not change `src/world_coordinates/tabular/`, whose `locate_one_dimensional` scan holds the time.
- A tiled-compressed float image whose `ZSCALE` or `ZZERO` is a header keyword rather than a table column decodes with a scale of 1 and a zero of 0 (`src/compress/decode/float_quantization.rs`, `Quantized::dequant`); cfitsio takes the keyword's value for every tile (`imcompress.c`, `imcomp_get_compressed_image_par`). With neither column, a lossless read ignores the keywords, where cfitsio applies them.
