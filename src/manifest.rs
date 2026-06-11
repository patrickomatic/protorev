//! Repeatable reverse-engineering experiment manifests.

use std::path::{Path, PathBuf};

use crate::Error;

/// A set of named before/after protobuf corpus experiments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentManifest {
    /// Experiments in file order.
    pub experiments: Vec<Experiment>,
}

/// One named corpus comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Experiment {
    /// Human-readable experiment name.
    pub name: String,
    /// Optional note explaining the controlled change.
    pub notes: Option<String>,
    /// Samples captured before the controlled change.
    pub before: Vec<PathBuf>,
    /// Samples captured after the controlled change.
    pub after: Vec<PathBuf>,
}

impl ExperimentManifest {
    /// Parse a manifest string, resolving relative sample paths against `base_dir`.
    pub fn parse(value: &str, base_dir: impl AsRef<Path>) -> Result<Self, Error> {
        let base_dir = base_dir.as_ref();
        let mut experiments = Vec::new();
        let mut draft: Option<ExperimentDraft> = None;

        for (line_index, raw_line) in value.lines().enumerate() {
            let line_number = line_index + 1;
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            if line == "[[experiment]]" {
                if let Some(current) = draft.take() {
                    experiments.push(current.finish(base_dir, line_number)?);
                }
                draft = Some(ExperimentDraft::default());
                continue;
            }

            let Some(current) = draft.as_mut() else {
                return Err(manifest_error(
                    line_number,
                    "expected [[experiment]] before key-value entries",
                ));
            };
            parse_entry(current, line, line_number)?;
        }

        if let Some(current) = draft {
            experiments.push(current.finish(base_dir, value.lines().count())?);
        }

        if experiments.is_empty() {
            return Err(Error::message("manifest did not define any experiments"));
        }

        Ok(Self { experiments })
    }

    /// Read and parse a manifest file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)?;
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        Self::parse(&contents, base_dir)
    }
}

#[derive(Debug, Default)]
struct ExperimentDraft {
    name: Option<String>,
    notes: Option<String>,
    before: Option<Vec<String>>,
    after: Option<Vec<String>>,
}

impl ExperimentDraft {
    fn finish(self, base_dir: &Path, line_number: usize) -> Result<Experiment, Error> {
        let name = self
            .name
            .ok_or_else(|| manifest_error(line_number, "experiment is missing name"))?;
        let before = self
            .before
            .ok_or_else(|| manifest_error(line_number, "experiment is missing before"))?;
        let after = self
            .after
            .ok_or_else(|| manifest_error(line_number, "experiment is missing after"))?;

        if before.is_empty() {
            return Err(manifest_error(
                line_number,
                "before must contain at least one path",
            ));
        }
        if after.is_empty() {
            return Err(manifest_error(
                line_number,
                "after must contain at least one path",
            ));
        }

        Ok(Experiment {
            name,
            notes: self.notes,
            before: resolve_paths(base_dir, &before),
            after: resolve_paths(base_dir, &after),
        })
    }
}

fn parse_entry(draft: &mut ExperimentDraft, line: &str, line_number: usize) -> Result<(), Error> {
    let Some((key, raw_value)) = line.split_once('=') else {
        return Err(manifest_error(line_number, "expected key = value"));
    };
    let key = key.trim();
    let raw_value = raw_value.trim();

    match key {
        "name" => {
            reject_duplicate(draft.name.is_some(), line_number, "name")?;
            draft.name = Some(parse_string(raw_value, line_number)?);
        }
        "notes" => {
            reject_duplicate(draft.notes.is_some(), line_number, "notes")?;
            draft.notes = Some(parse_string(raw_value, line_number)?);
        }
        "before" => {
            reject_duplicate(draft.before.is_some(), line_number, "before")?;
            draft.before = Some(parse_string_array(raw_value, line_number)?);
        }
        "after" => {
            reject_duplicate(draft.after.is_some(), line_number, "after")?;
            draft.after = Some(parse_string_array(raw_value, line_number)?);
        }
        _ => return Err(manifest_error(line_number, format!("unknown key {key:?}"))),
    }

    Ok(())
}

fn reject_duplicate(already_seen: bool, line_number: usize, key: &str) -> Result<(), Error> {
    if already_seen {
        Err(manifest_error(
            line_number,
            format!("duplicate key {key:?}"),
        ))
    } else {
        Ok(())
    }
}

fn parse_string(value: &str, line_number: usize) -> Result<String, Error> {
    let (parsed, remainder) = parse_string_prefix(value, line_number)?;
    if !remainder.trim().is_empty() {
        return Err(manifest_error(line_number, "unexpected text after string"));
    }
    Ok(parsed)
}

fn parse_string_prefix(value: &str, line_number: usize) -> Result<(String, &str), Error> {
    if !value.starts_with('"') {
        return Err(manifest_error(line_number, "expected quoted string"));
    }

    let remainder = advance_after_string(value, line_number)?;
    let mut out = String::new();
    let mut escaped = false;
    for ch in value[1..value.len() - remainder.len()].chars() {
        if escaped {
            match ch {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                _ => return Err(manifest_error(line_number, "unsupported string escape")),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Ok((out, remainder));
        } else {
            out.push(ch);
        }
    }

    Err(manifest_error(line_number, "unterminated string"))
}

fn parse_string_array(value: &str, line_number: usize) -> Result<Vec<String>, Error> {
    let trimmed = value.trim();
    let Some(body) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return Err(manifest_error(line_number, "expected string array"));
    };
    let mut values = Vec::new();
    let mut rest = body.trim();

    while !rest.is_empty() {
        if !rest.starts_with('"') {
            return Err(manifest_error(line_number, "expected quoted array entry"));
        }
        let (entry, after_entry) = parse_string_prefix(rest, line_number)?;
        values.push(entry);
        rest = after_entry.trim_start();
        if rest.is_empty() {
            break;
        }
        let Some(after_comma) = rest.strip_prefix(',') else {
            return Err(manifest_error(
                line_number,
                "expected comma between entries",
            ));
        };
        rest = after_comma.trim_start();
        if rest.is_empty() {
            return Err(manifest_error(
                line_number,
                "trailing comma is not supported",
            ));
        }
    }

    Ok(values)
}

fn advance_after_string(value: &str, line_number: usize) -> Result<&str, Error> {
    let mut escaped = false;
    for (index, ch) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Ok(&value[index + ch.len_utf8()..]);
        }
    }

    Err(manifest_error(line_number, "unterminated string"))
}

fn strip_comment(value: &str) -> &str {
    let mut escaped = false;
    let mut quoted = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch == '#' && !quoted {
            return &value[..index];
        }
    }
    value
}

fn resolve_paths(base_dir: &Path, values: &[String]) -> Vec<PathBuf> {
    values
        .iter()
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                base_dir.join(path)
            }
        })
        .collect()
}

fn manifest_error(line_number: usize, message: impl Into<String>) -> Error {
    Error::message(format!("manifest line {line_number}: {}", message.into()))
}
