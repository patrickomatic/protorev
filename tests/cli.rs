use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use protorev::wire::push_varint;

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
