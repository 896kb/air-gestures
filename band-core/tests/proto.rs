use band_core::proto::{field_bytes, field_int, varint, ProtoFields};

#[test]
fn builders_produce_protobuf_wire_format() {
    assert_eq!(varint(0), vec![0]);
    assert_eq!(varint(300), vec![0xac, 0x02]);
    assert_eq!(field_int(1, 1), vec![0x08, 0x01]);
    assert_eq!(field_bytes(5, &[]), vec![0x2a, 0x00]);
    assert_eq!(field_int(10, 1), vec![0x50, 0x01]);
    let long = vec![b'x'; 1777];
    assert_eq!(&field_bytes(2, &long)[..3], &[0x12, 0xf1, 0x0d]);
}

#[test]
fn parser_reads_integers_bytes_and_skips_fixed_fields() {
    let data = [
        field_int(1, 42),
        field_bytes(3, b"abc"),
        vec![0x21, 1, 2, 3, 4, 5, 6, 7, 8], // field 4, wire type 1 (fixed64)
        vec![0x2d, 1, 2, 3, 4],             // field 5, wire type 5 (fixed32)
    ]
    .concat();
    let fields = ProtoFields::parse(&data).unwrap();
    assert_eq!(fields.integer(1).unwrap(), 42);
    assert_eq!(fields.required_integer(1).unwrap(), 42);
    assert_eq!(fields.integer(2).unwrap(), 0, "absent integers default to 0");
    assert!(fields.required_integer(2).is_err());
    assert_eq!(fields.bytes(3).unwrap(), b"abc");
    assert!(fields.bytes_len(3, 4).is_err());
    assert!(fields.contains(4) && fields.contains(5));
    assert!(fields.integer(4).is_err(), "fixed fields are not integers");
    assert!(fields.bytes(2).is_err());
}

#[test]
fn parser_rejects_malformed_input() {
    assert!(ProtoFields::parse(&[0x08, 0x80]).is_err(), "truncated varint");
    assert!(
        ProtoFields::parse(&[0x08, 1, 0x08, 2]).unwrap().required_integer(1).is_err(),
        "repeated integer"
    );
    assert!(ProtoFields::parse(&[0x1a, 12, 1]).is_err(), "truncated bytes");
    assert!(ProtoFields::parse(&[0x00]).is_err(), "field number 0");
    assert!(ProtoFields::parse(&[0x0b]).is_err(), "wire type 3");
    let overflow = [0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
    assert!(ProtoFields::parse(&overflow).is_err(), "varint overflow");
}
