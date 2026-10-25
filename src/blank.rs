//! The blanks of FITS text, which the standard keeps to ASCII.

/// `text` without its leading and trailing blanks: the bytes `str::trim` drops from ASCII
/// text — tab to carriage return, and space — a byte at a time, with no Unicode table.
/// Refused: `str::trim_ascii`, which keeps the vertical tab `str::trim` drops.
pub(crate) fn trim(text: &str) -> &str {
    debug_assert!(text.is_ascii(), "FITS text is ASCII");
    let blank = |byte: &u8| *byte == b' ' || (b'\t'..=b'\r').contains(byte);
    let bytes = text.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !blank(byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !blank(byte))
        .map_or(start, |last| last + 1);
    &text[start..end]
}

#[cfg(test)]
mod tests {
    use crate::blank;

    /// Every ASCII byte trims as `str::trim` trims it, at either end and alone.
    #[test]
    fn trim_drops_what_str_trim_drops_from_ascii() {
        for byte in 0u8..=0x7F {
            let c = char::from(byte);
            for text in [format!("{c}x{c}"), format!("{c}{c}"), format!("x{c}y")] {
                assert_eq!(blank::trim(&text), text.trim(), "{byte:#04x}");
            }
        }
        assert_eq!(blank::trim(""), "");
    }
}
