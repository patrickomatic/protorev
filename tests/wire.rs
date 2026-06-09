use protorev::wire::push_varint;
use protorev::{Corpus, Message, Value, dump_message};

#[test]
fn decodes_basic_wire_fields_with_offsets() -> Result<(), protorev::Error> {
    let mut bytes = Vec::new();
    push_field_tag(&mut bytes, 1, 0);
    push_varint(&mut bytes, 150);
    push_field_tag(&mut bytes, 2, 2);
    push_varint(&mut bytes, 3);
    bytes.extend_from_slice(b"cat");

    let message = Message::decode(&bytes)?;
    assert_eq!(message.fields.len(), 2);
    assert_eq!(message.fields[0].number, 1);
    assert_eq!(message.fields[0].tag_offset, 0);
    assert_eq!(message.fields[0].end_offset, 3);
    assert_eq!(message.fields[0].value, Value::Varint(150));
    assert_eq!(message.fields[1].number, 2);
    assert_eq!(message.fields[1].end_offset, bytes.len());

    Ok(())
}

#[test]
fn dump_marks_nested_utf8_and_packed_candidates() -> Result<(), protorev::Error> {
    let nested = message_bytes(&[(1, 0, 7)]);
    let mut bytes = Vec::new();
    push_len_field(&mut bytes, 1, &nested);
    push_len_field(&mut bytes, 2, b"title");
    push_len_field(&mut bytes, 3, &[1, 2, 3]);

    let dump = dump_message(&Message::decode(&bytes)?, 2);

    assert!(dump.contains("field 1 length-delimited len=2 [message, packed-varint]"));
    assert!(dump.contains("field 2 length-delimited len=5 [utf8]"));
    assert!(dump.contains("field 3 length-delimited len=3 [packed-varint]"));
    assert!(dump.contains("field 1 varint = 7"));

    Ok(())
}

#[test]
fn corpus_emits_draft_proto_with_nested_messages() -> Result<(), protorev::Error> {
    let first_nested = message_bytes(&[(1, 0, 7)]);
    let second_nested = message_bytes(&[(1, 0, 8)]);
    let mut first = Vec::new();
    push_len_field(&mut first, 1, &first_nested);
    let mut second = Vec::new();
    push_len_field(&mut second, 1, &second_nested);

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let draft = corpus.draft_proto();

    assert!(draft.contains("message Message {"));
    assert!(draft.contains("Message_1 field_1 = 1;"));
    assert!(draft.contains("message Message_1 {"));
    assert!(draft.contains("uint64 field_1 = 1;"));

    Ok(())
}

fn message_bytes(fields: &[(u32, u8, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (number, wire_type, value) in fields {
        push_field_tag(&mut out, *number, *wire_type);
        push_varint(&mut out, *value);
    }
    out
}

fn push_len_field(out: &mut Vec<u8>, number: u32, value: &[u8]) {
    push_field_tag(out, number, 2);
    push_varint(out, u64::try_from(value.len()).unwrap_or(0));
    out.extend_from_slice(value);
}

fn push_field_tag(out: &mut Vec<u8>, number: u32, wire_type: u8) {
    push_varint(out, u64::from((number << 3) | u32::from(wire_type)));
}
