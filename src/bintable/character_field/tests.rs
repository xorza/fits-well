use crate::bintable::Table;
use crate::bintable::character_field::CharacterField;
use crate::bintable::column_data::ColumnData;
use crate::bintable::internals::table_header;

#[test]
fn character_columns_preserve_members_terminators_and_null_strings() {
    let header = table_header(4, 4, &["4A"]);
    let fields: [&[u8]; 4] = [b"AB  ", b"AB\0x", b"\0xyz", b"    "];
    let data = fields.concat();
    let table = Table::from_data(&header, data.clone()).unwrap();
    let ColumnData::Character(bytes) = table.column(0).unwrap().raw().unwrap() else {
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
