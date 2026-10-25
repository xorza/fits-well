//! The edit case: a plate solution added to a light on disk, as a solver writes it.

use std::fs::{File, OpenOptions};
use std::path::Path;

use criterion::{Criterion, Throughput};

use crate::bench::policy::{Kind, Scale, ScratchDir, SinkKind};
use crate::fits_editor::FitsEditor;
use crate::header_model::Header;
use crate::image_model::bitpix::Bitpix;
use crate::internals::scene::{HEIGHT, WIDTH};
use crate::internals::{memory, scene};
use crate::reader::FitsReader;

/// `integration/edit_header/file/plain/i16`: the light, written with its capture header and
/// checksums, opened for an edit and its header replaced in place by one with the plate
/// solution. The new header fits the old one's blocks and keeps its `DATASUM`, so the edit
/// writes the header alone. Each iteration writes the same header over the last. Per header.
pub(crate) fn integration(c: &mut Criterion) {
    let mut group = Kind::Integration.group(c, "edit_header", Scale::Row);
    group.throughput(Throughput::Elements(1));
    group.bench_function(format!("{}/plain/i16", SinkKind::File.id()), |b| {
        let dir = ScratchDir::new("edit");
        let light = memory::written_by(memory::memory_writer().with_checksums(), |w| {
            let image = scene::sky(Bitpix::I16, WIDTH, HEIGHT, 1);
            w.write_image(image.view(), Some(&scene::capture_header()))
        });
        let path = dir.write("light.fits", light.bytes());
        let mut solved = FitsReader::from_bytes(light.bytes()).unwrap().hdus()[0]
            .header
            .clone();
        scene::solve(&mut solved);
        edit(&path, &solved);
        assert_solved(&path, &solved);
        b.iter(|| edit(&path, &solved));
    });
    group.finish();
}

fn edit(path: &Path, header: &Header) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    FitsEditor::open(file)
        .unwrap()
        .replace_header(0, header)
        .unwrap();
}

/// Asserts that the light at `path` holds the plate solution of `solved` under valid
/// checksums.
fn assert_solved(path: &Path, solved: &Header) {
    let mut reader = FitsReader::open(File::open(path).unwrap()).unwrap();
    assert!(
        reader.verify_checksum(0).unwrap().is_valid(),
        "the edit signs the header"
    );
    let header = &reader.hdus()[0].header;
    for name in ["CTYPE1", "CRPIX2", "CD2_2"] {
        let value = header.get(name).unwrap();
        assert!(value.is_some(), "{name} is written");
        assert_eq!(value, solved.get(name).unwrap(), "{name}");
    }
}
