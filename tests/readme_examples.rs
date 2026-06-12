use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use protorev::wire::push_varint;

#[test]
fn readme_command_examples_use_installed_binary_and_still_run()
-> Result<(), Box<dyn std::error::Error>> {
    let readme = std::fs::read_to_string("README.md")?;
    assert!(!readme.contains("cargo run -p protorev"));

    for example in [
        "protorev dump sample.pb",
        "protorev dump --json sample.pb",
        "protorev infer samples/*.pb",
        "protorev schema samples/*.pb",
        "protorev explain --field 3.1 samples/*.pb",
        "protorev values --field 1 samples/*.pb",
        "protorev diff before/*.pb -- after/*.pb",
        "protorev experiments experiments.protorev",
    ] {
        assert!(readme.contains(example), "README missing {example:?}");
    }

    let dir = temp_dir("readme")?;
    let sample_path = dir.join("sample.pb");
    let samples_dir = dir.join("samples");
    let before_dir = dir.join("before");
    let after_dir = dir.join("after");
    std::fs::create_dir_all(&samples_dir)?;
    std::fs::create_dir_all(&before_dir)?;
    std::fs::create_dir_all(&after_dir)?;

    let first = sample_message(150, true);
    let second = sample_message(151, true);
    let nested = nested_sample_message();
    let after = sample_with_added_fixed32();

    std::fs::write(&sample_path, &first)?;
    std::fs::write(samples_dir.join("first.pb"), &first)?;
    std::fs::write(samples_dir.join("second.pb"), &second)?;
    std::fs::write(samples_dir.join("nested.pb"), &nested)?;
    std::fs::write(before_dir.join("sample.pb"), &first)?;
    std::fs::write(after_dir.join("sample.pb"), &after)?;

    let manifest_path = dir.join("experiments.protorev");
    std::fs::write(
        &manifest_path,
        r#"
        [[experiment]]
        name = "readme smoke"
        before = ["before/sample.pb"]
        after = ["after/sample.pb"]
        "#,
    )?;

    assert_success(&run_protorev(["dump", path_str(&sample_path)?])?);
    assert_success(&run_protorev(["dump", "--json", path_str(&sample_path)?])?);
    assert_success(&run_protorev([
        "infer",
        path_str(&samples_dir.join("first.pb"))?,
        path_str(&samples_dir.join("second.pb"))?,
    ])?);
    assert_success(&run_protorev([
        "schema",
        path_str(&samples_dir.join("first.pb"))?,
        path_str(&samples_dir.join("second.pb"))?,
    ])?);
    assert_success(&run_protorev([
        "explain",
        "--field",
        "3.1",
        path_str(&samples_dir.join("nested.pb"))?,
        path_str(&samples_dir.join("nested.pb"))?,
    ])?);
    assert_success(&run_protorev([
        "values",
        "--field",
        "1",
        path_str(&samples_dir.join("first.pb"))?,
        path_str(&samples_dir.join("second.pb"))?,
    ])?);
    assert_success(&run_protorev([
        "diff",
        path_str(&before_dir.join("sample.pb"))?,
        "--",
        path_str(&after_dir.join("sample.pb"))?,
    ])?);
    assert_success(&run_protorev(["experiments", path_str(&manifest_path)?])?);

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

fn nested_sample_message() -> Vec<u8> {
    let nested = sample_message(7, false);
    let mut out = Vec::new();
    push_len_field(&mut out, 3, &nested);
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
