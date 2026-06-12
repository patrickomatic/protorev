use std::path::PathBuf;

use protorev::ExperimentManifest;

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

fn assert_manifest_error(manifest: &str, text: &str) {
    let error = ExperimentManifest::parse(manifest, "/tmp/protorev-manifest")
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(error.contains(text), "{error}");
}
