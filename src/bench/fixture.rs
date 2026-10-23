//! The policy every case follows: its kind and sampling, the sources a read case opens, and the
//! directory its files go to.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, process, thread};

use criterion::measurement::WallTime;
use criterion::{Bencher, BenchmarkGroup, Criterion, SamplingMode, Throughput};

use crate::bitpix::Bitpix;
use crate::reader::FitsReader;
use crate::reader::source::Source;

/// What a case times, the first part of its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// One operation on the unit the read and write paths hand it.
    Atomic,
    /// A public entry point end to end on the standard frame.
    Integration,
}

impl Kind {
    /// The group `name` of this kind, sampled as `scale` says.
    pub(crate) fn group<'a>(
        self,
        c: &'a mut Criterion,
        name: &str,
        scale: Scale,
    ) -> BenchmarkGroup<'a, WallTime> {
        let kind = match self {
            Self::Atomic => "atomic",
            Self::Integration => "integration",
        };
        let mut group = c.benchmark_group(format!("{kind}/{name}"));
        scale.apply(&mut group);
        group
    }
}

/// How long one iteration of a case takes, which sets how criterion samples it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scale {
    /// A row or a header: microseconds an iteration.
    Sample,
    /// A frame, or a row of points through a transform, up to 300 ms an iteration, so fewer and
    /// longer samples.
    Frame,
    /// Past 300 ms an iteration: sampled flat at the fewest samples criterion takes, two
    /// iterations each. `iteration` is the slowest case's as measured, which sets the
    /// measurement time.
    Set { iteration: Duration },
}

impl Scale {
    /// The samples of a [`Scale::Set`] case: the fewest criterion takes.
    const SET_SAMPLES: u32 = 10;

    /// The iterations each sample of a [`Scale::Set`] case takes at the measured iteration.
    /// Criterion's flat sampling warns of a sample of one iteration, which it takes wherever the
    /// measurement time is at most the samples times the iteration: at two, only a case that
    /// runs at twice the measured iteration or slower takes one.
    const SET_ITERATIONS: u32 = 2;

    /// Sets the sampling of the cases `group` registers from here on.
    #[expect(
        clippy::disallowed_methods,
        reason = "`Scale::apply` is the one place a group's sampling is set"
    )]
    pub(crate) fn apply(self, group: &mut BenchmarkGroup<'_, WallTime>) {
        match self {
            Self::Sample => group
                .sample_size(50)
                .sampling_mode(SamplingMode::Auto)
                .warm_up_time(Duration::from_millis(500))
                .measurement_time(Duration::from_secs(1)),
            Self::Frame => group
                .sample_size(10)
                .sampling_mode(SamplingMode::Auto)
                .warm_up_time(Duration::from_secs(1))
                .measurement_time(Duration::from_secs(3)),
            Self::Set { iteration } => group
                .sample_size(Self::SET_SAMPLES as usize)
                .sampling_mode(SamplingMode::Flat)
                .warm_up_time(Duration::from_secs(1))
                .measurement_time(iteration * Self::SET_SAMPLES * Self::SET_ITERATIONS),
        };
    }
}

/// The throughput of a case over `elements` samples or rows of `element_bytes` each, in the
/// bytes they take uncompressed.
pub(crate) const fn throughput(elements: usize, element_bytes: usize) -> Throughput {
    Throughput::ElementsAndBytes {
        elements: elements as u64,
        bytes: (elements * element_bytes) as u64,
    }
}

/// The id part of `bitpix`: `i16`, `f32`.
pub(crate) const fn bitpix_id(bitpix: Bitpix) -> &'static str {
    match bitpix {
        Bitpix::U8 => "u8",
        Bitpix::I16 => "i16",
        Bitpix::I32 => "i32",
        Bitpix::I64 => "i64",
        Bitpix::F32 => "f32",
        Bitpix::F64 => "f64",
    }
}

/// The source a read case opens, the variant part of its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    /// `FitsReader::open` over a `File`: each data unit copied into the reader's scratch.
    Stream,
    /// `FitsReader::from_bytes` over the file's bytes in memory: decoded where they lie.
    Slice,
}

impl SourceKind {
    pub(crate) const fn all() -> [Self; 2] {
        [Self::Stream, Self::Slice]
    }

    /// The variant part of a case's id.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Slice => "slice",
        }
    }
}

/// What a read case does with the reader its source opens.
pub(crate) trait Reading {
    type Output;

    /// Reads from `reader` up to the last sample the case times.
    fn read<S: Source>(&mut self, reader: FitsReader<S>) -> Self::Output;
}

impl SourceKind {
    /// Times `reading` on the file at `path`, from opening this source to the last sample: the
    /// header scan, the read, and dropping the reader. A slice's bytes are read into memory first,
    /// untimed.
    pub(crate) fn bench<R: Reading>(self, b: &mut Bencher<'_>, path: &Path, mut reading: R) {
        match self {
            Self::Stream => b.iter_with_large_drop(|| {
                reading.read(FitsReader::open(File::open(path).unwrap()).unwrap())
            }),
            Self::Slice => {
                let bytes = fs::read(path).expect("a bench file is read");
                b.iter_with_large_drop(|| reading.read(FitsReader::from_bytes(&bytes).unwrap()));
            }
        }
    }
}

/// A directory for the files the cases write, removed with them when dropped.
#[derive(Debug)]
pub(crate) struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// A new directory `name` under the system's temporary directory, unique to this process.
    pub(crate) fn new(name: &str) -> Self {
        let path = env::temp_dir().join(format!("fits-well-bench-{}-{name}", process::id()));
        fs::create_dir_all(&path).expect("the bench scratch directory is created");
        Self { path }
    }

    /// The path of the file `name` in this directory.
    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// Writes `bytes` to the file `name` in this directory, and its path.
    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.file(name);
        fs::write(&path, bytes).expect("a bench file is written");
        path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let removed = fs::remove_dir_all(&self.path);
        if !thread::panicking() {
            removed.expect("the bench scratch directory is removed");
        }
    }
}
