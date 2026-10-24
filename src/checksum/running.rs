//! [`Running`]: a checksum summed from bytes as they pass.

use crate::checksum;

/// The 32-bit ones'-complement sum of bytes fed in pieces of any length: a word split
/// between two pieces waits for its rest.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Running {
    sum: u32,
    partial: [u8; 4],
    partial_len: usize,
}

impl Running {
    /// The sum of no bytes.
    pub(crate) const fn new() -> Self {
        Self {
            sum: 0,
            partial: [0; 4],
            partial_len: 0,
        }
    }

    /// Add `bytes` to the sum.
    pub(crate) fn update(&mut self, mut bytes: &[u8]) {
        if self.partial_len > 0 {
            let fill = (4 - self.partial_len).min(bytes.len());
            self.partial[self.partial_len..self.partial_len + fill].copy_from_slice(&bytes[..fill]);
            self.partial_len += fill;
            bytes = &bytes[fill..];
            if self.partial_len < 4 {
                return;
            }
            self.sum = checksum::combine(self.sum, u32::from_be_bytes(self.partial));
            self.partial_len = 0;
        }
        let whole = bytes.len() - bytes.len() % 4;
        self.sum = checksum::accumulate(&bytes[..whole], self.sum);
        let tail = &bytes[whole..];
        self.partial[..tail.len()].copy_from_slice(tail);
        self.partial_len = tail.len();
    }

    /// The sum of every byte fed, which must be a whole number of words.
    pub(crate) fn finish(&self) -> u32 {
        debug_assert_eq!(self.partial_len, 0, "a sum of whole words");
        self.sum
    }
}
