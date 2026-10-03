//! One binary-table `A` field, read in place.

/// One binary-table `A` field, its stored bytes preserved exactly.
///
/// [`CharacterField::members`] stops at the first NUL, while
/// [`CharacterField::bytes`] keeps the terminator, undefined bytes after it, and
/// all trailing spaces. A NUL in the first byte is therefore distinguishable from
/// an empty or all-space field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterField<'a> {
    bytes: &'a [u8],
}

impl<'a> CharacterField<'a> {
    /// The field stored as `bytes` — a fixed column's `repeat` bytes for one row,
    /// or one row of a `P`/`Q` character column.
    pub const fn new(bytes: &'a [u8]) -> CharacterField<'a> {
        CharacterField { bytes }
    }

    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The defined character members, ending immediately before the first NUL.
    pub fn members(&self) -> &'a [u8] {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.bytes.len());
        &self.bytes[..end]
    }

    /// Whether this is the FITS null string, identified by an initial NUL.
    pub fn is_null(&self) -> bool {
        self.bytes.first() == Some(&0)
    }
}

#[cfg(test)]
mod tests {
    use crate::bintable::BinTable;
    use crate::bintable::character_field::CharacterField;
    use crate::bintable::column_data::ColumnData;
    use crate::bintable::internals::table_header;

    #[test]
    fn character_columns_preserve_members_terminators_and_null_strings() {
        let header = table_header(4, 4, &["4A"]);
        let fields: [&[u8]; 4] = [b"AB  ", b"AB\0x", b"\0xyz", b"    "];
        let data = fields.concat();
        let table = BinTable::from_data(&header, data.clone()).unwrap();
        let ColumnData::Character(bytes) = table.column_by_idx(0).unwrap().raw().unwrap() else {
            panic!("expected exact binary character bytes");
        };
        assert_eq!(bytes, data);
        let read: Vec<_> = bytes.chunks(4).map(CharacterField::new).collect();
        assert_eq!(read[0].members(), b"AB  ");
        assert_eq!(read[1].members(), b"AB");
        assert!(!read[1].is_null());
        assert_eq!(read[2].members(), b"");
        assert!(read[2].is_null());
        assert_eq!(read[3].members(), b"    ");
        assert!(!read[3].is_null());
        assert_eq!(read[1].bytes(), b"AB\0x");
    }
}
