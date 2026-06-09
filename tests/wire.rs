use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use protorev::wire::push_varint;
use protorev::{
    Confidence, Corpus, Error, LengthDelimitedHints, Message, SchemaOptions, Value, WireType,
    dump_message, dump_message_json,
};

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
    assert_truncated(&[0x80], "truncated varint at offset 1");
    assert_truncated(&[0x0a, 0x03, b'a'], "truncated length-delimited field");
    assert_truncated(&[0x09, 1, 2], "truncated fixed64");
    assert_truncated(&[0x15, 1, 2], "truncated fixed32");
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

    Ok(())
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

    let diff = run_protorev(["diff", path_str(&first_path)?, path_str(&second_path)?])?;
    assert_success(&diff);
    let diff_stdout = String::from_utf8(diff.stdout)?;
    assert!(diff_stdout.contains("field 1: observed 2/2 samples"));

    Ok(())
}

#[test]
fn cli_reports_usage_errors() -> Result<(), Box<dyn std::error::Error>> {
    let output = run_protorev(["dump"])?;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("usage: protorev dump [--json] <file.pb>"));

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
