use protorev::wire::push_varint;
use protorev::{Confidence, Corpus, Message, SchemaOptions};
use serde_json::Value as JsonValue;

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

#[test]
fn corpus_marks_repeated_and_optional_fields() -> Result<(), protorev::Error> {
    let mut first = Vec::new();
    push_varint_field(&mut first, 1, 10);
    push_varint_field(&mut first, 1, 11);
    push_fixed32_field(&mut first, 2, 4);

    let mut second = Vec::new();
    push_varint_field(&mut second, 1, 12);

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let summary = corpus.summary();
    let draft = corpus.draft_proto();

    assert!(summary.contains("field 1: observed 2/2 samples"));
    assert!(summary.contains("max/sample: 2 repeated"));
    assert!(summary.contains("field 2: observed 1/2 samples"));
    assert!(draft.contains("repeated uint64 field_1 = 1;"));
    assert!(draft.contains("fixed32 field_2 = 2;"));

    Ok(())
}

#[test]
fn schema_emits_only_high_confidence_fields_by_default() -> Result<(), protorev::Error> {
    let nested = message_bytes(&[(1, 0, 7)]);
    let mut first = Vec::new();
    push_len_field(&mut first, 1, &nested);
    push_varint_field(&mut first, 2, 10);
    push_varint_field(&mut first, 3, 11);

    let mut second = Vec::new();
    push_len_field(&mut second, 1, &nested);
    push_field_tag(&mut second, 3, 5);
    second.extend_from_slice(&4_u32.to_le_bytes());

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let schema = corpus.schema(&SchemaOptions::default());

    assert!(schema.contains("Message_1 field_1 = 1; // confidence: high"));
    assert!(schema.contains("message Message_1 {"));
    assert!(schema.contains("uint64 field_1 = 1; // confidence: high"));
    assert!(!schema.contains("field_2 = 2;"));
    assert!(!schema.contains("field_3 = 3;"));

    Ok(())
}

#[test]
fn schema_threshold_controls_medium_and_low_confidence_fields() -> Result<(), protorev::Error> {
    let mut first = Vec::new();
    push_varint_field(&mut first, 1, 10);
    push_varint_field(&mut first, 2, 20);
    push_varint_field(&mut first, 4, 40);

    let mut second = Vec::new();
    push_varint_field(&mut second, 1, 11);
    push_field_tag(&mut second, 3, 5);
    second.extend_from_slice(&4_u32.to_le_bytes());
    push_field_tag(&mut second, 4, 5);
    second.extend_from_slice(&5_u32.to_le_bytes());

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let medium = corpus.schema(&SchemaOptions {
        min_confidence: Confidence::Medium,
    });
    let low = corpus.schema(&SchemaOptions {
        min_confidence: Confidence::Low,
    });

    assert!(medium.contains("uint64 field_1 = 1; // confidence: high"));
    assert!(medium.contains("uint64 field_2 = 2; // confidence: medium"));
    assert!(medium.contains("fixed32 field_3 = 3; // confidence: medium"));
    assert!(!medium.contains("field_4 = 4;"));

    assert!(low.contains("uint64 field_1 = 1; // confidence: high"));
    assert!(low.contains("uint64 field_2 = 2; // confidence: medium"));
    assert!(low.contains("fixed32 field_3 = 3; // confidence: medium"));
    assert!(low.contains("uint64 field_4 = 4; // confidence: low"));

    Ok(())
}

#[test]
fn schema_caps_single_sample_fields_at_medium_confidence() -> Result<(), protorev::Error> {
    let mut bytes = Vec::new();
    push_varint_field(&mut bytes, 1, 10);

    let message = Message::decode(&bytes)?;
    let corpus = Corpus::from_messages(&[message], 2);
    let high = corpus.schema(&SchemaOptions::default());
    let medium = corpus.schema(&SchemaOptions {
        min_confidence: Confidence::Medium,
    });

    assert!(high.contains("No fields met confidence threshold high"));
    assert!(!high.contains("field_1 = 1;"));
    assert!(medium.contains("uint64 field_1 = 1; // confidence: medium"));

    Ok(())
}

#[test]
fn schema_uses_bytes_when_length_delimited_shape_is_not_consistent() -> Result<(), protorev::Error>
{
    let nested = message_bytes(&[(1, 0, 7)]);
    let mut first = Vec::new();
    push_len_field(&mut first, 1, &nested);

    let mut second = Vec::new();
    push_len_field(&mut second, 1, b"title");

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let schema = corpus.schema(&SchemaOptions::default());

    assert!(schema.contains("bytes field_1 = 1; // confidence: high"));
    assert!(!schema.contains("Message_1 field_1 = 1;"));

    Ok(())
}

#[test]
fn explain_reports_field_evidence_by_path() -> Result<(), protorev::Error> {
    let nested = message_bytes(&[(1, 0, 7)]);
    let mut first = Vec::new();
    push_len_field(&mut first, 3, &nested);

    let mut second = Vec::new();
    push_len_field(&mut second, 3, &nested);

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let path = protorev::FieldPath::parse("3.1")
        .ok_or_else(|| protorev::Error::message("test field path should parse"))?;
    let explanation = corpus
        .explain(&path)
        .ok_or_else(|| protorev::Error::message("test field path should be observed"))?;
    let json = corpus
        .explain_json(&path)
        .ok_or_else(|| protorev::Error::message("test field path should be observed"))?;

    assert!(explanation.contains("field 3.1"));
    assert!(explanation.contains("schema type: uint64"));
    assert!(explanation.contains("confidence: high"));
    assert!(explanation.contains("included at high threshold: yes"));

    assert!(json.contains("\"path\":\"3.1\""));
    assert!(json.contains("\"schema_type\":\"uint64\""));
    assert!(json.contains("\"confidence\":\"high\""));
    assert!(json.contains("\"high\":true"));

    let parsed = parse_json(&json)?;
    assert_eq!(parsed["path"], JsonValue::from("3.1"));
    assert_eq!(parsed["name"], JsonValue::from("field_1"));
    assert_eq!(parsed["schema_type"], JsonValue::from("uint64"));
    assert_eq!(parsed["confidence"], JsonValue::from("high"));
    assert_eq!(parsed["included"]["high"], JsonValue::from(true));

    Ok(())
}

#[test]
fn values_summarizes_numeric_and_length_delimited_fields() -> Result<(), protorev::Error> {
    let mut first = Vec::new();
    push_varint_field(&mut first, 1, 0);
    push_varint_field(&mut first, 1, 1);
    push_len_field(&mut first, 2, b"title");

    let mut second = Vec::new();
    push_varint_field(&mut second, 1, 42);
    push_len_field(&mut second, 2, b"title");

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let field_one = protorev::FieldPath::parse("1")
        .ok_or_else(|| protorev::Error::message("field path should parse"))?;
    let field_two = protorev::FieldPath::parse("2")
        .ok_or_else(|| protorev::Error::message("field path should parse"))?;
    let numeric = corpus
        .values(&messages, &field_one)
        .ok_or_else(|| protorev::Error::message("field should have values"))?;
    let numeric_json = corpus
        .values_json(&messages, &field_one)
        .ok_or_else(|| protorev::Error::message("field should have values"))?;
    let text = corpus
        .values(&messages, &field_two)
        .ok_or_else(|| protorev::Error::message("field should have values"))?;

    assert!(numeric.contains("occurrences: 3"));
    assert!(numeric.contains("min: 0"));
    assert!(numeric.contains("max: 42"));
    assert!(numeric.contains("distinct: 3"));
    assert!(numeric.contains("counter_or_id: yes"));
    assert!(numeric_json.contains("\"path\":\"1\""));
    assert!(numeric_json.contains("\"max\":42"));
    assert!(numeric_json.contains("\"counter_or_id\":true"));
    let parsed_numeric = parse_json(&numeric_json)?;
    assert_eq!(parsed_numeric["path"], JsonValue::from("1"));
    assert_eq!(parsed_numeric["occurrences"], JsonValue::from(3));
    assert_eq!(parsed_numeric["varint"]["min"], JsonValue::from(0));
    assert_eq!(parsed_numeric["varint"]["max"], JsonValue::from(42));
    assert_eq!(
        parsed_numeric["varint"]["candidates"]["counter_or_id"],
        JsonValue::from(true)
    );

    assert!(text.contains("length-delimited:"));
    assert!(text.contains("utf8: 2/2"));
    assert!(text.contains("text distinct: 1"));
    assert!(text.contains("\"title\": 2"));

    Ok(())
}

#[test]
fn values_can_follow_nested_field_paths() -> Result<(), protorev::Error> {
    let nested = message_bytes(&[(1, 0, 7)]);
    let mut first = Vec::new();
    push_len_field(&mut first, 3, &nested);

    let mut second = Vec::new();
    push_len_field(&mut second, 3, &nested);

    let messages = vec![Message::decode(&first)?, Message::decode(&second)?];
    let corpus = Corpus::from_messages(&messages, 2);
    let path = protorev::FieldPath::parse("3.1")
        .ok_or_else(|| protorev::Error::message("field path should parse"))?;
    let values = corpus
        .values(&messages, &path)
        .ok_or_else(|| protorev::Error::message("field should have values"))?;

    assert!(values.contains("field 3.1"));
    assert!(values.contains("occurrences: 2"));
    assert!(values.contains("min: 7"));
    assert!(values.contains("max: 7"));

    Ok(())
}

#[test]
fn corpus_diff_reports_added_removed_and_changed_fields() -> Result<(), protorev::Error> {
    let mut before_first = Vec::new();
    push_varint_field(&mut before_first, 1, 10);
    push_varint_field(&mut before_first, 2, 20);
    push_len_field(&mut before_first, 3, b"before");

    let mut before_second = Vec::new();
    push_varint_field(&mut before_second, 1, 11);
    push_varint_field(&mut before_second, 2, 21);

    let mut after_first = Vec::new();
    push_varint_field(&mut after_first, 1, 12);
    push_varint_field(&mut after_first, 2, 22);
    push_varint_field(&mut after_first, 2, 23);
    push_fixed32_field(&mut after_first, 4, 4);

    let mut after_second = Vec::new();
    push_varint_field(&mut after_second, 1, 13);
    push_varint_field(&mut after_second, 2, 24);
    push_varint_field(&mut after_second, 2, 25);
    push_fixed32_field(&mut after_second, 4, 5);

    let before_messages = vec![
        Message::decode(&before_first)?,
        Message::decode(&before_second)?,
    ];
    let after_messages = vec![
        Message::decode(&after_first)?,
        Message::decode(&after_second)?,
    ];
    let before = Corpus::from_messages(&before_messages, 2);
    let after = Corpus::from_messages(&after_messages, 2);
    let diff = Corpus::diff(&before, &after);
    let json = Corpus::diff_json(&before, &after);

    assert!(diff.contains("before samples: 2"));
    assert!(diff.contains("after samples: 2"));
    assert!(diff.contains("added:"));
    assert!(diff.contains("field 4: observed 2/2 samples"));
    assert!(diff.contains("removed:"));
    assert!(diff.contains("field 3: observed 1/2 samples"));
    assert!(diff.contains("changed:"));
    assert!(diff.contains("field 2:"));
    assert!(diff.contains("max/sample: 1 -> 2"));
    assert!(diff.contains("repetition: singular -> repeated"));

    assert!(json.contains("\"before_samples\":2"));
    assert!(json.contains("\"path\":\"4\""));
    assert!(json.contains("\"path\":\"3\""));
    assert!(json.contains("repetition: singular -> repeated"));
    let parsed_json = parse_json(&json)?;
    assert_eq!(parsed_json["before_samples"], JsonValue::from(2));
    assert_eq!(parsed_json["after_samples"], JsonValue::from(2));
    assert_eq!(
        parsed_json["messages"][0]["message"],
        JsonValue::from("root")
    );
    assert_eq!(
        parsed_json["messages"][0]["added"][0]["path"],
        JsonValue::from("4")
    );
    assert_eq!(
        parsed_json["messages"][0]["removed"][0]["path"],
        JsonValue::from("3")
    );
    assert_eq!(
        parsed_json["messages"][0]["changed"][0]["path"],
        JsonValue::from("2")
    );

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

fn push_varint_field(out: &mut Vec<u8>, number: u32, value: u64) {
    push_field_tag(out, number, 0);
    push_varint(out, value);
}

fn push_fixed32_field(out: &mut Vec<u8>, number: u32, value: u32) {
    push_field_tag(out, number, 5);
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_len_field(out: &mut Vec<u8>, number: u32, value: &[u8]) {
    push_field_tag(out, number, 2);
    push_varint(out, u64::try_from(value.len()).unwrap_or(0));
    out.extend_from_slice(value);
}

fn push_field_tag(out: &mut Vec<u8>, number: u32, wire_type: u8) {
    push_varint(out, u64::from((number << 3) | u32::from(wire_type)));
}

fn parse_json(value: &str) -> Result<JsonValue, protorev::Error> {
    serde_json::from_str(value).map_err(|error| protorev::Error::message(error.to_string()))
}
