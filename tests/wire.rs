use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use protorev::wire::push_varint;
use protorev::{
    Confidence, Corpus, Error, ExperimentManifest, LengthDelimitedHints, Message, SchemaOptions,
    Value, WireType, dump_message, dump_message_json,
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
fn json_outputs_escape_text_and_manifest_strings() -> Result<(), Box<dyn std::error::Error>> {
    let text = b"line\n\"quoted\"\\path\tend";
    let mut bytes = Vec::new();
    push_len_field(&mut bytes, 1, text);
    let message = Message::decode(&bytes)?;
    let dump_json = dump_message_json(&message, 1);

    assert!(!dump_json.contains('\n'));
    assert!(dump_json.contains(r#""text":"line\n\"quoted\"\\path\tend""#));
    let parsed_dump = parse_json_box(&dump_json)?;
    assert_eq!(
        parsed_dump["fields"][0]["value"]["text"],
        JsonValue::from("line\n\"quoted\"\\path\tend")
    );

    let messages = vec![message];
    let corpus = Corpus::from_messages(&messages, 1);
    let field = protorev::FieldPath::parse("1")
        .ok_or_else(|| protorev::Error::message("field path should parse"))?;
    let values_json = corpus
        .values_json(&messages, &field)
        .ok_or_else(|| protorev::Error::message("field should have values"))?;
    assert!(values_json.contains(r#""value":"line\n\"quoted\"\\path\tend""#));
    let parsed_values = parse_json_box(&values_json)?;
    assert_eq!(
        parsed_values["length_delimited"]["text_common"][0]["value"],
        JsonValue::from("line\n\"quoted\"\\path\tend")
    );

    let dir = temp_dir("json-escape")?;
    let before_path = dir.join("before.pb");
    let after_path = dir.join("after.pb");
    let manifest_path = dir.join("experiments.protorev");
    let mut before = Vec::new();
    push_varint_field(&mut before, 1, 1);
    let mut after = Vec::new();
    push_varint_field(&mut after, 1, 2);
    std::fs::write(&before_path, before)?;
    std::fs::write(&after_path, after)?;
    std::fs::write(
        &manifest_path,
        r#"
        [[experiment]]
        name = "quote \" slash \\"
        notes = "line\nnext\tcell"
        before = ["before.pb"]
        after = ["after.pb"]
        "#,
    )?;

    let output = run_protorev(["experiments", "--json", path_str(&manifest_path)?])?;
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains(r#""name":"quote \" slash \\""#));
    assert!(stdout.contains(r#""notes":"line\nnext\tcell""#));
    let parsed_experiments = parse_json_box(&stdout)?;
    assert_eq!(
        parsed_experiments["experiments"][0]["name"],
        JsonValue::from("quote \" slash \\")
    );
    assert_eq!(
        parsed_experiments["experiments"][0]["notes"],
        JsonValue::from("line\nnext\tcell")
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

#[test]
fn experiment_manifest_parses_named_corpora() -> Result<(), protorev::Error> {
    let manifest = ExperimentManifest::parse(
        r#"
        # comments are allowed outside strings
        [[experiment]]
        name = "add field four"
        notes = "controlled before/after run"
        before = ["before.pb"]
        after = ["after.pb", "/tmp/absolute.pb"]
        "#,
        "/tmp/protorev-manifest",
    )?;

    assert_eq!(manifest.experiments.len(), 1);
    let experiment = &manifest.experiments[0];
    assert_eq!(experiment.name, "add field four");
    assert_eq!(
        experiment.notes.as_deref(),
        Some("controlled before/after run")
    );
    assert_eq!(
        experiment.before[0],
        PathBuf::from("/tmp/protorev-manifest/before.pb")
    );
    assert_eq!(
        experiment.after[0],
        PathBuf::from("/tmp/protorev-manifest/after.pb")
    );
    assert_eq!(experiment.after[1], PathBuf::from("/tmp/absolute.pb"));

    Ok(())
}

#[test]
fn experiment_manifest_handles_escapes_and_comments_inside_strings() -> Result<(), protorev::Error>
{
    let manifest = ExperimentManifest::parse(
        r#"
        [[experiment]]
        name = "hash # quote \" slash \\"
        notes = "line\nnext\tcell"
        before = ["before#one.pb"] # real comment
        after = ["after.pb"]
        "#,
        "/tmp/protorev-manifest",
    )?;

    let experiment = &manifest.experiments[0];
    assert_eq!(experiment.name, "hash # quote \" slash \\");
    assert_eq!(experiment.notes.as_deref(), Some("line\nnext\tcell"));
    assert_eq!(
        experiment.before[0],
        PathBuf::from("/tmp/protorev-manifest/before#one.pb")
    );

    Ok(())
}

#[test]
fn experiment_manifest_reports_invalid_input() {
    assert_manifest_error(
        "name = \"missing section\"",
        "expected [[experiment]] before key-value entries",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        before = ["before.pb"]
        after = ["after.pb"]
        "#,
        "experiment is missing name",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        name = "missing after"
        before = ["before.pb"]
        "#,
        "experiment is missing after",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        name = "duplicate"
        name = "again"
        before = ["before.pb"]
        after = ["after.pb"]
        "#,
        "duplicate key \"name\"",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        name = "unknown"
        description = "nope"
        before = ["before.pb"]
        after = ["after.pb"]
        "#,
        "unknown key \"description\"",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        name = "empty before"
        before = []
        after = ["after.pb"]
        "#,
        "before must contain at least one path",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        name = "bad escape"
        notes = "\x"
        before = ["before.pb"]
        after = ["after.pb"]
        "#,
        "unsupported string escape",
    );
    assert_manifest_error(
        r#"
        [[experiment]]
        name = "trailing comma"
        before = ["before.pb",]
        after = ["after.pb"]
        "#,
        "trailing comma is not supported",
    );
}

#[test]
fn cli_dump_infer_and_diff_use_library_output() -> Result<(), Box<dyn std::error::Error>> {
    let mut first = Vec::new();
    push_varint_field(&mut first, 1, 150);
    push_len_field(&mut first, 2, b"title");

    let mut second = Vec::new();
    push_varint_field(&mut second, 1, 151);
    push_len_field(&mut second, 2, b"title");

    let first_path = write_sample("first", &first)?;
    let second_path = write_sample("second", &second)?;

    let dump = run_protorev(["dump", path_str(&first_path)?])?;
    assert_success(&dump);
    let dump_stdout = String::from_utf8(dump.stdout)?;
    assert!(dump_stdout.contains("field 1 varint = 150"));
    assert!(dump_stdout.contains("field 2 length-delimited len=5 [utf8]"));

    let dump_json = run_protorev(["dump", "--json", path_str(&first_path)?])?;
    assert_success(&dump_json);
    let dump_json_stdout = String::from_utf8(dump_json.stdout)?;
    assert!(dump_json_stdout.contains("\"number\":1"));
    assert!(dump_json_stdout.contains("\"kind\":\"varint\",\"value\":150"));

    let infer = run_protorev(["infer", path_str(&first_path)?, path_str(&second_path)?])?;
    assert_success(&infer);
    let infer_stdout = String::from_utf8(infer.stdout)?;
    assert!(infer_stdout.contains("samples: 2"));
    assert!(infer_stdout.contains("--- draft proto ---"));
    assert!(infer_stdout.contains("uint64 field_1 = 1;"));

    let schema = run_protorev(["schema", path_str(&first_path)?, path_str(&second_path)?])?;
    assert_success(&schema);
    let schema_stdout = String::from_utf8(schema.stdout)?;
    assert!(schema_stdout.contains("uint64 field_1 = 1; // confidence: high"));

    let explain = run_protorev([
        "explain",
        "--field",
        "1",
        path_str(&first_path)?,
        path_str(&second_path)?,
    ])?;
    assert_success(&explain);
    let explain_stdout = String::from_utf8(explain.stdout)?;
    assert!(explain_stdout.contains("field 1"));
    assert!(explain_stdout.contains("confidence: high"));

    let explain_json = run_protorev([
        "explain",
        "--json",
        "--field",
        "1",
        path_str(&first_path)?,
        path_str(&second_path)?,
    ])?;
    assert_success(&explain_json);
    let explain_json_stdout = String::from_utf8(explain_json.stdout)?;
    assert!(explain_json_stdout.contains("\"path\":\"1\""));
    assert!(explain_json_stdout.contains("\"confidence\":\"high\""));

    let values = run_protorev([
        "values",
        "--field",
        "1",
        path_str(&first_path)?,
        path_str(&second_path)?,
    ])?;
    assert_success(&values);
    let values_stdout = String::from_utf8(values.stdout)?;
    assert!(values_stdout.contains("field 1"));
    assert!(values_stdout.contains("min: 150"));
    assert!(values_stdout.contains("max: 151"));

    let values_json = run_protorev([
        "values",
        "--json",
        "--field",
        "1",
        path_str(&first_path)?,
        path_str(&second_path)?,
    ])?;
    assert_success(&values_json);
    let values_json_stdout = String::from_utf8(values_json.stdout)?;
    assert!(values_json_stdout.contains("\"path\":\"1\""));
    assert!(values_json_stdout.contains("\"max\":151"));

    let diff = run_protorev(["diff", path_str(&first_path)?, path_str(&second_path)?])?;
    assert_success(&diff);
    let diff_stdout = String::from_utf8(diff.stdout)?;
    assert!(diff_stdout.contains("before samples: 1"));
    assert!(diff_stdout.contains("after samples: 1"));

    let diff_json = run_protorev([
        "diff",
        "--json",
        path_str(&first_path)?,
        "--",
        path_str(&second_path)?,
    ])?;
    assert_success(&diff_json);
    let diff_json_stdout = String::from_utf8(diff_json.stdout)?;
    assert!(diff_json_stdout.contains("\"before_samples\":1"));
    assert!(diff_json_stdout.contains("\"after_samples\":1"));

    Ok(())
}

#[test]
fn cli_experiments_runs_manifest_diffs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_dir("manifest")?;
    let before_path = dir.join("before.pb");
    let after_path = dir.join("after.pb");
    let manifest_path = dir.join("experiments.protorev");

    let mut before = Vec::new();
    push_varint_field(&mut before, 1, 10);

    let mut after = Vec::new();
    push_varint_field(&mut after, 1, 10);
    push_fixed32_field(&mut after, 4, 4);

    std::fs::write(&before_path, before)?;
    std::fs::write(&after_path, after)?;
    std::fs::write(
        &manifest_path,
        r#"
        [[experiment]]
        name = "add field four"
        notes = "synthetic controlled change"
        before = ["before.pb"]
        after = ["after.pb"]
        "#,
    )?;

    let text = run_protorev(["experiments", path_str(&manifest_path)?])?;
    assert_success(&text);
    let text_stdout = String::from_utf8(text.stdout)?;
    assert!(text_stdout.contains("== add field four =="));
    assert!(text_stdout.contains("notes: synthetic controlled change"));
    assert!(text_stdout.contains("before samples: 1"));
    assert!(text_stdout.contains("after samples: 1"));
    assert!(text_stdout.contains("added:"));
    assert!(text_stdout.contains("field 4: observed 1/1 samples"));

    let json = run_protorev(["experiments", "--json", path_str(&manifest_path)?])?;
    assert_success(&json);
    let json_stdout = String::from_utf8(json.stdout)?;
    assert!(json_stdout.contains("\"experiments\":["));
    assert!(json_stdout.contains("\"name\":\"add field four\""));
    assert!(json_stdout.contains("\"notes\":\"synthetic controlled change\""));
    assert!(json_stdout.contains("\"before_samples\":1"));
    assert!(json_stdout.contains("\"path\":\"4\""));

    Ok(())
}

#[test]
fn cli_reports_usage_errors() -> Result<(), Box<dyn std::error::Error>> {
    let output = run_protorev(["dump"])?;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("required arguments were not provided"));
    assert!(stderr.contains("Usage: protorev dump <file.pb>"));

    Ok(())
}

#[test]
fn cli_reports_argument_file_and_field_errors() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    push_varint_field(&mut bytes, 1, 10);
    let sample_path = write_sample("errors", &bytes)?;

    let bad_field = run_protorev(["explain", "--field", "0", path_str(&sample_path)?])?;
    assert_failure_contains(&bad_field, "must look like 1 or 3.1")?;

    let bad_confidence = run_protorev([
        "schema",
        "--min-confidence",
        "certain",
        path_str(&sample_path)?,
    ])?;
    assert_failure_contains(&bad_confidence, "must be one of: high, medium, low")?;

    let missing_file_path = std::env::temp_dir().join(format!(
        "protorev-missing-{}-{}.pb",
        std::process::id(),
        unique_suffix()
    ));
    let missing_file = run_protorev(["dump", path_str(&missing_file_path)?])?;
    assert_failure_contains(&missing_file, "No such file")?;

    let diff_without_separator = run_protorev([
        "diff",
        path_str(&sample_path)?,
        path_str(&sample_path)?,
        path_str(&sample_path)?,
    ])?;
    assert_failure_contains(
        &diff_without_separator,
        "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
    )?;

    let unobserved_explain = run_protorev(["explain", "--field", "2", path_str(&sample_path)?])?;
    assert_failure_contains(
        &unobserved_explain,
        "field 2 was not observed in the corpus",
    )?;

    let unobserved_values = run_protorev(["values", "--field", "2", path_str(&sample_path)?])?;
    assert_failure_contains(
        &unobserved_values,
        "field 2 had no observed values in the corpus",
    )?;

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

fn parse_json_box(value: &str) -> Result<JsonValue, Box<dyn std::error::Error>> {
    serde_json::from_str(value).map_err(Into::into)
}

fn assert_manifest_error(manifest: &str, text: &str) {
    let error = ExperimentManifest::parse(manifest, "/tmp/protorev-manifest")
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(error.contains(text), "{error}");
}

fn write_sample(name: &str, bytes: &[u8]) -> Result<PathBuf, std::io::Error> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "protorev-{name}-{}-{}.pb",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn temp_dir(name: &str) -> Result<PathBuf, std::io::Error> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "protorev-{name}-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

fn unique_suffix() -> u128 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    }
}

fn path_str(path: &Path) -> Result<&str, Box<dyn std::error::Error>> {
    path.to_str()
        .ok_or_else(|| String::from("sample path was not valid UTF-8").into())
}

fn run_protorev<const N: usize>(args: [&str; N]) -> Result<std::process::Output, std::io::Error> {
    Command::new(env!("CARGO_BIN_EXE_protorev"))
        .args(args)
        .output()
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure_contains(
    output: &std::process::Output,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr.clone())?;
    assert!(stderr.contains(text), "{stderr}");
    Ok(())
}
