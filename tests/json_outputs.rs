use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use protorev::wire::push_varint;
use protorev::{Corpus, Message, dump_message_json};
use serde_json::Value as JsonValue;

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

fn parse_json_box(value: &str) -> Result<JsonValue, Box<dyn std::error::Error>> {
    serde_json::from_str(value).map_err(Into::into)
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
