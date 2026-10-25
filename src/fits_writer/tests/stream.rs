//! Image streams: written a chunk at a time, and the writer they poison when they fail.

use crate::fits_writer::tests::*;

/// A stream gives the bytes of a whole write, signed, with a header template and without,
/// whatever its chunks, and the signed file verifies and reads back.
#[test]
fn streaming_image_matches_transactional_output_and_checksums() {
    let samples = vec![1i16, -2, 300, i16::MIN, i16::MAX, 42];
    let scaling = Scaling {
        bscale: 2.5,
        bzero: -10.0,
        blank: Some(i64::from(i16::MIN)),
    };
    let image = Image::new(vec![3, 2], samples.clone(), scaling).unwrap();
    let mut template = Header::new();
    template.set(keywords::OBJECT, "streamed").unwrap();
    for (template, chunks) in [
        (Some(&template), &[0..2, 2..5, 5..6][..]),
        (None, &[0..3, 3..6]),
    ] {
        let expected = memory::written_by(memory::memory_writer().with_checksums(), |w| {
            w.write_image(image.view(), template)
        });
        let mut streamed = memory::memory_writer().with_checksums();
        let mut stream = streamed
            .stream_image(image.view().metadata(), template)
            .unwrap();
        for chunk in chunks {
            stream
                .write_chunk(View::I16(&samples[chunk.clone()]))
                .unwrap();
        }
        stream.finish().unwrap();
        let actual = streamed.into_inner().into_inner();
        assert_eq!(actual, expected.bytes(), "{chunks:?}");

        let mut reader = FitsReader::from_bytes(&actual).unwrap();
        assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
        assert_eq!(
            memory::read_whole(&mut reader, 0).samples,
            Data::I16(samples.clone())
        );
    }
}

/// Each way a stream fails poisons its writer, which then refuses the next HDU, so no
/// signed but corrupt HDU is written: a failed write, after which a retried chunk and the
/// finish are refused too; a chunk of another type, the stream then dropped; and a finish
/// short of the samples.
#[test]
fn each_failed_image_stream_poisons_the_writer() {
    type Fail = fn(ImageWriter<'_, FailingSink>);
    let cases: [(usize, Fail); 3] = [
        (2, |mut stream| {
            assert_matches!(stream.write_chunk(View::I16(&[1])), Err(FitsError::Io(_)));
            assert_matches!(
                stream.write_chunk(View::I16(&[1])),
                Err(FitsError::WriterFailed)
            );
            assert_matches!(stream.finish(), Err(FitsError::WriterFailed));
        }),
        (usize::MAX, |mut stream| {
            assert_matches!(
                stream.write_chunk(View::U8(&[1])),
                Err(FitsError::ChunkTypeMismatch {
                    expected: Bitpix::I16,
                    got: Bitpix::U8
                })
            );
        }),
        (usize::MAX, |stream| {
            assert_matches!(
                stream.finish(),
                Err(FitsError::DataSizeMismatch {
                    unit: SizeUnit::Samples,
                    expected: 2,
                    got: 0,
                })
            );
        }),
    ];
    let next = Image::new(vec![1], vec![7u8], Scaling::IDENTITY).unwrap();
    for (case, (fail_at, fail)) in cases.into_iter().enumerate() {
        let sink = FailingSink {
            inner: Cursor::new(Vec::new()),
            writes: 0,
            fail_at,
        };
        let mut writer = FitsWriter::new(sink).with_checksums();
        let metadata = Metadata {
            shape: &[2],
            bitpix: Bitpix::I16,
            scaling: Scaling::IDENTITY,
        };
        fail(writer.stream_image(metadata, None).unwrap());
        assert_matches!(
            writer.write_image(next.view(), None),
            Err(FitsError::WriterFailed),
            "case {case}"
        );
    }
}
