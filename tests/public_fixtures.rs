use std::path::{Path, PathBuf};
use std::process::Command;

use protorev::{Confidence, Corpus, Message, SchemaOptions, Value, dump_message};

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/public");

#[test]
fn official_encoding_fixtures_decode_to_expected_wire_shapes() -> Result<(), TestError> {
    let test1 = fixture_message("test1_varint_150.pb.hex")?;
    assert_eq!(test1.fields.len(), 1);
    assert_eq!(test1.fields[0].number, 1);
    assert_eq!(test1.fields[0].value, Value::Varint(150));

    let test2 = fixture_message("test2_string_testing.pb.hex")?;
    assert_eq!(test2.fields.len(), 1);
    assert_eq!(test2.fields[0].number, 2);
    assert_eq!(
        test2.fields[0].value,
        Value::LengthDelimited(b"testing".to_vec())
    );

    let test3 = fixture_message("test3_submessage_150.pb.hex")?;
    assert_eq!(test3.fields.len(), 1);
    assert_eq!(test3.fields[0].number, 3);
    let Value::LengthDelimited(bytes) = &test3.fields[0].value else {
        return Err(TestError::message("expected length-delimited submessage"));
    };
    let nested = Message::decode(bytes)?;
    assert_eq!(nested.fields[0].value, Value::Varint(150));

    let test4 = fixture_message("test4_packed_repeated.pb.hex")?;
    assert_eq!(test4.fields.len(), 2);
    assert_eq!(test4.fields[0].number, 4);
    assert_eq!(
        test4.fields[0].value,
        Value::LengthDelimited(b"hello".to_vec())
    );
    assert_eq!(test4.fields[1].number, 5);

    Ok(())
}

#[test]
fn fixture_dumps_show_public_examples_without_schema_knowledge() -> Result<(), TestError> {
    let string_dump = dump_message(&fixture_message("test2_string_testing.pb.hex")?, 4);
    assert!(string_dump.contains("field 2 length-delimited len=7 [utf8]"));
    assert!(string_dump.contains("text \"testing\""));

    let submessage_dump = dump_message(&fixture_message("test3_submessage_150.pb.hex")?, 4);
    assert!(submessage_dump.contains("field 3 length-delimited len=3 [message, packed-varint]"));
    assert!(submessage_dump.contains("field 1 varint = 150"));

    let packed_dump = dump_message(&fixture_message("test4_packed_repeated.pb.hex")?, 4);
    assert!(packed_dump.contains("field 4 length-delimited len=5 [utf8]"));
    assert!(packed_dump.contains("text \"hello\""));
    assert!(packed_dump.contains("field 5 length-delimited len=3 [packed-varint]"));
    assert!(packed_dump.contains("packed [1, 2, 3]"));

    Ok(())
}

#[test]
fn schema_uses_public_corpus_confidence_conservatively() -> Result<(), TestError> {
    let messages = vec![
        fixture_message("test4_packed_repeated.pb.hex")?,
        fixture_message("test4_mixed_order_equivalent.pb.hex")?,
    ];
    let corpus = Corpus::from_messages(&messages, 4);

    let high = corpus.schema(&SchemaOptions::default());
    assert!(high.contains("bytes field_4 = 4; // confidence: high"));
    assert!(high.contains("repeated bytes field_5 = 5; // confidence: high"));
    assert!(high.contains("observed 2/2 samples"));
    assert!(high.contains("packed-varint: 2/3"));

    let single = Corpus::from_messages(&[fixture_message("test1_varint_150.pb.hex")?], 4);
    let single_high = single.schema(&SchemaOptions::default());
    assert!(single_high.contains("No fields met confidence threshold high"));

    let single_medium = single.schema(&SchemaOptions {
        min_confidence: Confidence::Medium,
    });
    assert!(single_medium.contains("uint64 field_1 = 1; // confidence: medium"));

    Ok(())
}

#[test]
fn cli_schema_reads_public_hex_fixture_bytes() -> Result<(), TestError> {
    let first = materialize_fixture("test4_packed_repeated.pb.hex")?;
    let second = materialize_fixture("test4_mixed_order_equivalent.pb.hex")?;

    let output = Command::new(env!("CARGO_BIN_EXE_protorev"))
        .args(["schema", path_str(&first)?, path_str(&second)?])
        .output()?;

    if !output.status.success() {
        return Err(TestError::message(format!(
            "protorev schema failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("bytes field_4 = 4; // confidence: high"));
    assert!(stdout.contains("repeated bytes field_5 = 5; // confidence: high"));
    assert!(stdout.contains("packed-varint: 2/3"));

    Ok(())
}

fn fixture_message(name: &str) -> Result<Message, TestError> {
    Message::decode(&fixture_bytes(name)?).map_err(TestError::from)
}

fn fixture_bytes(name: &str) -> Result<Vec<u8>, TestError> {
    let path = Path::new(FIXTURE_DIR).join(name);
    let hex = std::fs::read_to_string(path)?;
    decode_hex(&hex)
}

fn materialize_fixture(name: &str) -> Result<PathBuf, TestError> {
    let bytes = fixture_bytes(name)?;
    let mut path = std::env::temp_dir();
    path.push(format!("protorev-public-{}-{name}.pb", std::process::id()));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, TestError> {
    let mut cleaned = String::new();
    for character in hex.chars() {
        if character.is_ascii_whitespace() {
            continue;
        }
        if !character.is_ascii_hexdigit() {
            return Err(TestError::message(format!(
                "invalid hex character {character:?}"
            )));
        }
        cleaned.push(character);
    }

    if !cleaned.len().is_multiple_of(2) {
        return Err(TestError::message("hex fixture had odd length"));
    }

    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for index in (0..cleaned.len()).step_by(2) {
        let byte = u8::from_str_radix(&cleaned[index..index + 2], 16)?;
        out.push(byte);
    }
    Ok(out)
}

fn path_str(path: &Path) -> Result<&str, TestError> {
    path.to_str()
        .ok_or_else(|| TestError::message("fixture path was not valid UTF-8"))
}

#[derive(Debug)]
enum TestError {
    Message(String),
    Io(std::io::Error),
    Utf8(std::string::FromUtf8Error),
    ParseInt(std::num::ParseIntError),
    Proto(protorev::Error),
}

impl TestError {
    fn message(value: impl Into<String>) -> Self {
        Self::Message(value.into())
    }
}

impl std::fmt::Display for TestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Utf8(error) => write!(formatter, "{error}"),
            Self::ParseInt(error) => write!(formatter, "{error}"),
            Self::Proto(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for TestError {}

impl From<std::io::Error> for TestError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<std::string::FromUtf8Error> for TestError {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<std::num::ParseIntError> for TestError {
    fn from(value: std::num::ParseIntError) -> Self {
        Self::ParseInt(value)
    }
}

impl From<protorev::Error> for TestError {
    fn from(value: protorev::Error) -> Self {
        Self::Proto(value)
    }
}
