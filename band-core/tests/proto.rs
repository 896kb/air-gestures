use band_core::datax::{DataXReceiver, encode_frame};
use band_core::proto::{ProtoFields, field_bytes, field_int, varint};

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
    assert_eq!(
        fields.integer(2).unwrap(),
        0,
        "absent integers default to 0"
    );
    assert!(fields.required_integer(2).is_err());
    assert_eq!(fields.bytes(3).unwrap(), b"abc");
    assert!(fields.bytes_len(3, 4).is_err());
    assert!(fields.contains(4) && fields.contains(5));
    assert!(fields.integer(4).is_err(), "fixed fields are not integers");
    assert!(fields.bytes(2).is_err());
}

#[test]
fn parser_rejects_malformed_input() {
    assert!(
        ProtoFields::parse(&[0x08, 0x80]).is_err(),
        "truncated varint"
    );
    assert!(
        ProtoFields::parse(&[0x08, 1, 0x08, 2])
            .unwrap()
            .required_integer(1)
            .is_err(),
        "repeated integer"
    );
    assert!(
        ProtoFields::parse(&[0x1a, 12, 1]).is_err(),
        "truncated bytes"
    );
    assert!(ProtoFields::parse(&[0x00]).is_err(), "field number 0");
    assert!(ProtoFields::parse(&[0x0b]).is_err(), "wire type 3");
    let overflow = [
        0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02,
    ];
    assert!(ProtoFields::parse(&overflow).is_err(), "varint overflow");
}

#[test]
fn frames_carry_a_typed_header_only_when_words_are_present() {
    let typed = encode_frame(0x8001, &[0x81000005, 0x02000001], &[0xaa]).unwrap();
    assert_eq!(hex::encode(&typed), "800980018100000502000001aa");
    let plain = encode_frame(5, &[], &[1, 2]).unwrap();
    assert_eq!(plain, vec![0x00, 0x02, 0x00, 0x05, 1, 2]);
    assert!(encode_frame(1, &[], &vec![0; 0x8000]).is_err());
}

#[test]
fn native_framing_preserves_split_messages_and_rejects_malformed_input() {
    let mut receiver = DataXReceiver::default();
    let payload: Vec<u8> = (0..40).collect();
    let frame = encode_frame(0x8005, &[0x0200020d], &payload).unwrap();
    let head = [&frame[..13], &[0xc3; 3][..]].concat();
    assert!(receiver.feed(&head).unwrap().is_empty());
    let tail = [&frame[13..], &[0xcd; 13][..]].concat();
    let frames = receiver.feed(&tail).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].channel, 0x8005);
    assert_eq!(frames[0].words, vec![0x0200020d]);
    assert_eq!(frames[0].payload, payload);
    assert!(
        DataXReceiver::default().feed(&[0; 15]).is_err(),
        "unaligned plaintext"
    );
    // A typed header whose words run past the frame end.
    let bad = [vec![0x80, 0x04, 0x00, 0x01, 0x80, 0, 0, 0], vec![0xc8; 8]].concat();
    assert!(DataXReceiver::default().feed(&bad).is_err());
}
