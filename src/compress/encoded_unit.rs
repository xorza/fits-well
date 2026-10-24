//! [`EncodedUnit`]: a compressed container's header and its data unit in parts.

use crate::header_model::Header;

/// A compressed container as an encoder hands it to the writer: its header, the rows
/// of its main table, and its heap as the encoded cells in order, each kept once and
/// written as it is, with no copy into one buffer.
#[derive(Debug)]
pub(crate) struct EncodedUnit {
    pub(crate) header: Header,
    pub(crate) rows: Vec<u8>,
    pub(crate) heap: Vec<Vec<u8>>,
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::compress::encoded_unit::EncodedUnit;

    impl EncodedUnit {
        /// The data unit as one buffer, as the writer emits it.
        pub(crate) fn data(&self) -> Vec<u8> {
            let mut data = self.rows.clone();
            for cell in &self.heap {
                data.extend_from_slice(cell);
            }
            data
        }
    }
}
