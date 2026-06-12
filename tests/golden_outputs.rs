use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use protorev::wire::push_varint;

#[test]
fn cli_outputs_match_golden_files() -> Result<(), Box<dyn std::error::Error>> {
    let first = sample_message(150, true);
    let second = sample_message(151, true);
    let after = sample_with_added_fixed32();

    let first_path = write_sample("golden-first", &first)?;
    let second_path = write_sample("golden-second", &second)?;
    let after_path = write_sample("golden-after", &after)?;

    assert_golden("dump.txt", run_protorev(["dump", path_str(&first_path)?])?)?;
    assert_golden(
        "dump.json",
        run_protorev(["dump", "--json", path_str(&first_path)?])?,
    )?;
    assert_golden(
        "schema.txt",
        run_protorev(["schema", path_str(&first_path)?, path_str(&second_path)?])?,
    )?;
    assert_golden(
        "diff.txt",
        run_protorev(["diff", path_str(&first_path)?, path_str(&after_path)?])?,
    )?;
    assert_golden(
        "diff.json",
        run_protorev([
            "diff",
            "--json",
            path_str(&first_path)?,
            "--",
            path_str(&after_path)?,
        ])?,
    )?;

    let manifest_dir = temp_dir("golden-manifest")?;
    let before_manifest_path = manifest_dir.join("before.pb");
    let after_manifest_path = manifest_dir.join("after.pb");
    let manifest_path = manifest_dir.join("experiments.protorev");
    std::fs::write(&before_manifest_path, &first)?;
    std::fs::write(&after_manifest_path, &after)?;
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

    let experiments = run_protorev(["experiments", "--json", path_str(&manifest_path)?])?;
    assert_success(&experiments);
    let experiments_stdout = String::from_utf8(experiments.stdout)?
        .replace(path_str(&before_manifest_path)?, "<BEFORE>")
        .replace(path_str(&after_manifest_path)?, "<AFTER>");
    assert_eq!(experiments_stdout, include_str!("golden/experiments.json"));

    Ok(())
}

fn sample_message(value: u64, include_title: bool) -> Vec<u8> {
    let mut out = Vec::new();
    push_varint_field(&mut out, 1, value);
    if include_title {
        push_len_field(&mut out, 2, b"title");
    }
    out
}

fn sample_with_added_fixed32() -> Vec<u8> {
    let mut out = sample_message(150, true);
    push_fixed32_field(&mut out, 4, 4);
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

fn assert_golden(
    name: &str,
    output: std::process::Output,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout, golden(name)?);
    Ok(())
}

fn golden(name: &str) -> Result<&'static str, Box<dyn std::error::Error>> {
    match name {
        "dump.txt" => Ok(include_str!("golden/dump.txt")),
        "dump.json" => Ok(include_str!("golden/dump.json")),
        "schema.txt" => Ok(include_str!("golden/schema.txt")),
        "diff.txt" => Ok(include_str!("golden/diff.txt")),
        "diff.json" => Ok(include_str!("golden/diff.json")),
        _ => Err(format!("unknown golden file {name:?}").into()),
    }
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
