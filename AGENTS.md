# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## What this is

`fits-well` is a Rust library to **read and write FITS** (Flexible Image Transport
System) files — the standard data format of astronomy. The two non-negotiable
goals shape every decision:

1. **Blazing fast** — zero-copy where the format allows, a borrowed read view
   (`read_image_view` → `BorrowedImage`) that byte-swaps into a caller-owned reused
   scratch so a hot read loop isn't page-fault-bound, single-pass byte-swap / scaling, tile-parallel
   (de)compression, reused read/write scratch buffers, lazy HDU access via seeking.
2. **Whole-standard coverage** — the full **FITS 4.0** standard (images, ASCII
   tables, binary tables with heap/variable-length arrays, random groups for
   read, WCS, time coordinates, tiled compression).
