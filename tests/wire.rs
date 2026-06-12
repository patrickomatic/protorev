use protorev::wire::push_varint;
use protorev::{
    Error, LengthDelimitedHints, Message, Value, WireType, dump_message, dump_message_json,
};
use serde_json::Value as JsonValue;

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
fn decodes_fixed_width_fields() -> Result<(), protorev::Error> {
    let mut bytes = Vec::new();
    push_field_tag(&mut bytes, 3, 1);
    bytes.extend_from_slice(&0x0102_0304_0506_0708_u64.to_le_bytes());
    push_field_tag(&mut bytes, 4, 5);
    bytes.extend_from_slice(&0x0a0b_0c0d_u32.to_le_bytes());

    let message = Message::decode(&bytes)?;

    assert_eq!(message.fields[0].wire_type, WireType::Fixed64);
    assert_eq!(
        message.fields[0].value,
        Value::Fixed64(0x0102_0304_0506_0708)
    );
    assert_eq!(message.fields[1].wire_type, WireType::Fixed32);
    assert_eq!(message.fields[1].value, Value::Fixed32(0x0a0b_0c0d));

    Ok(())
}

#[test]
fn rejects_invalid_and_truncated_wire_streams() {
    assert_invalid_wire(&[0x00], "field tag cannot be zero");
    assert_invalid_wire(&[0x0b], "unsupported wire type");
    assert_invalid_wire(
        &[
            0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
        ],
        "varint overflow",
    );
    assert_truncated(&[0x80], "truncated varint at offset 1");
    assert_truncated(&[0x0a, 0x03, b'a'], "truncated length-delimited field");
    assert_truncated(&[0x09, 1, 2], "truncated fixed64");
    assert_truncated(&[0x15, 1, 2], "truncated fixed32");
}

#[test]
fn decodes_varint_boundaries_and_rejects_field_number_overflow() -> Result<(), protorev::Error> {
    let mut max_value = Vec::new();
    push_varint_field(&mut max_value, 1, u64::MAX);
    let message = Message::decode(&max_value)?;
    assert_eq!(message.fields[0].value, Value::Varint(u64::MAX));

    let mut overflowing_tag = Vec::new();
    push_varint(&mut overflowing_tag, (u64::from(u32::MAX) + 1) << 3);
    push_varint(&mut overflowing_tag, 1);
    assert_invalid_wire(&overflowing_tag, "field number overflow");

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
fn dump_respects_recursive_depth_limit() -> Result<(), protorev::Error> {
    let deepest = message_bytes(&[(1, 0, 7)]);
    let mut middle = Vec::new();
    push_len_field(&mut middle, 1, &deepest);
    let mut outer = Vec::new();
    push_len_field(&mut outer, 1, &middle);

    let dump = dump_message(&Message::decode(&outer)?, 1);

    assert_eq!(dump.matches("field 1 length-delimited").count(), 2);
    assert!(!dump.contains("field 1 varint = 7"));

    Ok(())
}

#[test]
fn dump_json_exposes_offsets_values_and_hints() -> Result<(), protorev::Error> {
    let nested = message_bytes(&[(1, 0, 7)]);
    let mut bytes = Vec::new();
    push_varint_field(&mut bytes, 1, 150);
    push_len_field(&mut bytes, 2, b"title");
    push_len_field(&mut bytes, 3, &nested);

    let json = dump_message_json(&Message::decode(&bytes)?, 2);

    assert!(json.starts_with("{\"len\":"));
    assert!(json.contains("\"number\":1"));
    assert!(json.contains("\"kind\":\"varint\",\"value\":150"));
    assert!(json.contains("\"wire_type\":\"length-delimited\""));
    assert!(json.contains("\"text\":\"title\""));
    assert!(json.contains("\"message\":true"));
    assert!(json.contains("\"nested\":{\"len\":2"));

    let parsed = parse_json(&json)?;
    assert_eq!(parsed["len"], JsonValue::from(bytes.len()));
    assert_eq!(parsed["fields"][0]["number"], JsonValue::from(1));
    assert_eq!(
        parsed["fields"][0]["value"]["kind"],
        JsonValue::from("varint")
    );
    assert_eq!(parsed["fields"][0]["value"]["value"], JsonValue::from(150));
    assert_eq!(
        parsed["fields"][1]["value"]["text"],
        JsonValue::from("title")
    );
    assert_eq!(
        parsed["fields"][2]["value"]["hints"]["message"],
        JsonValue::from(true)
    );
    assert_eq!(
        parsed["fields"][2]["value"]["nested"]["len"],
        JsonValue::from(2)
    );

    Ok(())
}

#[test]
fn length_delimited_hints_keep_ambiguous_candidates_visible() {
    let nested = message_bytes(&[(1, 0, 7)]);
    let nested_hints = LengthDelimitedHints::classify(&nested);
    assert!(nested_hints.nested_message.is_some());
    assert_eq!(nested_hints.packed_varints, Some(vec![8, 7]));
    assert_eq!(nested_hints.utf8, None);

    let text_hints = LengthDelimitedHints::classify(b"title");
    assert_eq!(text_hints.utf8.as_deref(), Some("title"));
    assert_eq!(text_hints.packed_varints, None);

    let control_hints = LengthDelimitedHints::classify(&[0x01, 0x02]);
    assert_eq!(control_hints.utf8, None);
    assert_eq!(control_hints.packed_varints, Some(vec![1, 2]));
}

fn message_bytes(fields: &[(u32, u8, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (number, wire_type, value) in fields {
        push_field_tag(&mut out, *number, *wire_type);
        push_varint(&mut out, *value);
    }
    out
}

fn push_varint_field(out: &mut Vec<u8>, number: u32, value: u64) {
    push_field_tag(out, number, 0);
    push_varint(out, value);
}

fn push_len_field(out: &mut Vec<u8>, number: u32, value: &[u8]) {
    push_field_tag(out, number, 2);
    push_varint(out, u64::try_from(value.len()).unwrap_or(0));
    out.extend_from_slice(value);
}

fn push_field_tag(out: &mut Vec<u8>, number: u32, wire_type: u8) {
    push_varint(out, u64::from((number << 3) | u32::from(wire_type)));
}

fn assert_invalid_wire(bytes: &[u8], reason: &str) {
    let error = Message::decode(bytes).err();
    assert!(matches!(
        error,
        Some(Error::InvalidWire {
            reason: actual,
            ..
        }) if actual == reason
    ));
}

fn assert_truncated(bytes: &[u8], text: &str) {
    let error = Message::decode(bytes)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(error.contains(text), "{error}");
}

fn parse_json(value: &str) -> Result<JsonValue, protorev::Error> {
    serde_json::from_str(value).map_err(|error| protorev::Error::message(error.to_string()))
}
