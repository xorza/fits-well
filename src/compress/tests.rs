use crate::compress;
use crate::compress::*;

#[test]
fn typed_compression_configuration_rejects_impossible_values() {
    assert!(Gzip::new(9).is_ok());
    assert!(Gzip::shuffled(0).is_ok());
    assert!(matches!(
        Gzip::new(10),
        Err(FitsError::InvalidValue { card }) if card == "gzip compression level 10 is outside 0..=9"
    ));
    assert!(matches!(
        Hcompress::lossy(0.0),
        Err(FitsError::InvalidValue { card }) if card == "lossy HCOMPRESS scale 0 must be positive"
    ));
    assert!(matches!(
        Hcompress::lossy(f64::NAN),
        Err(FitsError::InvalidValue { .. })
    ));
    assert!(matches!(
        CompressionOptions::default().with_quantization(f64::NAN, DitherMethod::None),
        Err(FitsError::InvalidValue { .. })
    ));
    assert_eq!(Compression::GZIP.name(), "GZIP_1");
    assert_eq!(Compression::GZIP_SHUFFLED.name(), "GZIP_2");
}

#[test]
fn compression_parameter_indices_use_the_standard_canonical_form() {
    assert_eq!(compress::parameter_index("ZNAME1"), Some(1));
    assert_eq!(compress::parameter_index("ZNAME999"), Some(999));
    for keyword in ["ZNAME", "ZNAME0", "ZNAME01", "ZNAME1000", "ZNAMEA"] {
        assert_eq!(compress::parameter_index(keyword), None, "{keyword}");
    }
}

#[test]
fn needs_wide_promotes_past_the_32_bit_descriptor_range() {
    // §10.1.3: a 32-bit `P` descriptor holds a signed 32-bit offset/count, so a heap
    // or tile element count past `i32::MAX` needs a 64-bit `Q` (the integer encoder
    // promotes; the fixed-layout float encoder errors). The threshold is shared.
    let max = i32::MAX as usize;
    assert!(!needs_wide(0, 0));
    assert!(!needs_wide(max, max)); // exactly i32::MAX still fits a 32-bit P
    assert!(needs_wide(max + 1, 0)); // heap offset past the range
    assert!(needs_wide(0, max + 1)); // a tile's element count past the range
}
