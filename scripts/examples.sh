#!/usr/bin/env bash
# Each example, run once. `inspect` reads a sample file. A test holds this list to `examples/`.
set -euo pipefail

cargo --locked run --example image
cargo --locked run --example compression
cargo --locked run --example table
cargo --locked run --example ascii_table
cargo --locked run --example header
cargo --locked run --example edit
cargo --locked run --example stream
cargo --locked run --example wcs
cargo --locked run --example time
cargo --locked run --example inspect -- test_resources/UITfuv2582gc.fits
