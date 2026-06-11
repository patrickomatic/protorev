use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use protorev::{
    Confidence, Corpus, Error, Experiment, ExperimentManifest, FieldPath, Message, SchemaOptions,
    dump_message, dump_message_json,
};

const DEFAULT_MAX_DEPTH: usize = 4;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };
    let paths = args.collect::<Vec<_>>();

    match command.as_str() {
        "dump" => dump_command(&paths),
        "infer" => infer_command(&paths),
        "schema" => schema_command(&paths),
        "explain" => explain_command(&paths),
        "values" => values_command(&paths),
        "diff" => diff_command(&paths),
        "experiments" | "experiment" => experiments_command(&paths),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        _ => Err(Error::message(format!("unknown command {command:?}"))),
    }
}

fn dump_command(args: &[String]) -> Result<(), Error> {
    let (json, paths) = parse_dump_args(args);
    if paths.len() != 1 {
        return Err(Error::message("usage: protorev dump [--json] <file.pb>"));
    }

    let message = read_message(paths[0])?;
    if json {
        println!("{}", dump_message_json(&message, DEFAULT_MAX_DEPTH));
    } else {
        print!("{}", dump_message(&message, DEFAULT_MAX_DEPTH));
    }
    Ok(())
}

fn infer_command(paths: &[String]) -> Result<(), Error> {
    if paths.is_empty() {
        return Err(Error::message("usage: protorev infer <file.pb>..."));
    }

    let mut messages = Vec::new();
    for path in paths {
        messages.push(read_message(path)?);
    }
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    print!("{}", corpus.summary());
    println!("\n--- draft proto ---\n");
    print!("{}", corpus.draft_proto());
    Ok(())
}

fn schema_command(args: &[String]) -> Result<(), Error> {
    let (options, paths) = parse_schema_args(args)?;
    if paths.is_empty() {
        return Err(Error::message(
            "usage: protorev schema [--min-confidence high|medium|low] <file.pb>...",
        ));
    }

    let mut messages = Vec::new();
    for path in paths {
        messages.push(read_message(path)?);
    }
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    print!("{}", corpus.schema(&options));
    Ok(())
}

fn explain_command(args: &[String]) -> Result<(), Error> {
    let (json, field_path, paths) = parse_explain_args(args)?;
    if paths.is_empty() {
        return Err(Error::message(
            "usage: protorev explain [--json] --field <path> <file.pb>...",
        ));
    }

    let mut messages = Vec::new();
    for path in paths {
        messages.push(read_message(path)?);
    }
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    let output = if json {
        corpus.explain_json(&field_path)
    } else {
        corpus.explain(&field_path)
    };
    match output {
        Some(output) => {
            print!("{output}");
            if json {
                println!();
            }
            Ok(())
        }
        None => Err(Error::message(format!(
            "field {field_path} was not observed in the corpus"
        ))),
    }
}

fn values_command(args: &[String]) -> Result<(), Error> {
    let (json, field_path, paths) = parse_field_path_args(
        args,
        "usage: protorev values [--json] --field <path> <file.pb>...",
    )?;
    if paths.is_empty() {
        return Err(Error::message(
            "usage: protorev values [--json] --field <path> <file.pb>...",
        ));
    }

    let mut messages = Vec::new();
    for path in paths {
        messages.push(read_message(path)?);
    }
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    let output = if json {
        corpus.values_json(&messages, &field_path)
    } else {
        corpus.values(&messages, &field_path)
    };
    match output {
        Some(output) => {
            print!("{output}");
            if json {
                println!();
            }
            Ok(())
        }
        None => Err(Error::message(format!(
            "field {field_path} had no observed values in the corpus"
        ))),
    }
}

fn diff_command(args: &[String]) -> Result<(), Error> {
    let (json, before_paths, after_paths) = parse_diff_args(args)?;
    if before_paths.is_empty() || after_paths.is_empty() {
        return Err(Error::message(
            "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
        ));
    }

    let before_messages = read_messages(&before_paths)?;
    let after_messages = read_messages(&after_paths)?;
    let before = Corpus::from_messages(&before_messages, DEFAULT_MAX_DEPTH);
    let after = Corpus::from_messages(&after_messages, DEFAULT_MAX_DEPTH);
    if json {
        println!("{}", Corpus::diff_json(&before, &after));
    } else {
        print!("{}", Corpus::diff(&before, &after));
    }
    Ok(())
}

fn experiments_command(args: &[String]) -> Result<(), Error> {
    let (json, path) = parse_experiments_args(args)?;
    let manifest = ExperimentManifest::from_file(path)?;
    if json {
        println!("{}", experiments_json(&manifest)?);
    } else {
        print!("{}", experiments_text(&manifest)?);
    }
    Ok(())
}

fn parse_schema_args(args: &[String]) -> Result<(SchemaOptions, Vec<&str>), Error> {
    let mut options = SchemaOptions::default();
    let mut paths = Vec::new();
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--min-confidence" {
            let Some(value) = args.get(index + 1) else {
                return Err(Error::message(
                    "usage: protorev schema [--min-confidence high|medium|low] <file.pb>...",
                ));
            };
            options.min_confidence = Confidence::parse(value).ok_or_else(|| {
                Error::message("min confidence must be one of: high, medium, low")
            })?;
            index += 2;
        } else {
            paths.push(arg);
            index += 1;
        }
    }

    Ok((options, paths))
}

fn parse_dump_args(args: &[String]) -> (bool, Vec<&str>) {
    let mut json = false;
    let mut paths = Vec::new();

    for arg in args {
        if arg == "--json" {
            json = true;
        } else {
            paths.push(arg.as_str());
        }
    }

    (json, paths)
}

fn parse_explain_args(args: &[String]) -> Result<(bool, FieldPath, Vec<&str>), Error> {
    parse_field_path_args(
        args,
        "usage: protorev explain [--json] --field <path> <file.pb>...",
    )
}

fn parse_field_path_args<'a>(
    args: &'a [String],
    usage: &'static str,
) -> Result<(bool, FieldPath, Vec<&'a str>), Error> {
    let mut json = false;
    let mut field_path = None;
    let mut paths = Vec::new();
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--json" {
            json = true;
            index += 1;
        } else if arg == "--field" {
            let Some(value) = args.get(index + 1) else {
                return Err(Error::message(usage));
            };
            field_path = FieldPath::parse(value);
            if field_path.is_none() {
                return Err(Error::message("field path must look like 1 or 3.1"));
            }
            index += 2;
        } else {
            paths.push(arg);
            index += 1;
        }
    }

    let Some(field_path) = field_path else {
        return Err(Error::message(usage));
    };

    Ok((json, field_path, paths))
}

fn parse_diff_args(args: &[String]) -> Result<(bool, Vec<&str>, Vec<&str>), Error> {
    let mut json = false;
    let mut separator = None;
    let mut paths = Vec::new();

    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg == "--" {
            if separator.is_some() {
                return Err(Error::message(
                    "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
                ));
            }
            separator = Some(paths.len());
        } else {
            paths.push(arg.as_str());
        }
    }

    if let Some(separator) = separator {
        let before = paths[..separator].to_vec();
        let after = paths[separator..].to_vec();
        return Ok((json, before, after));
    }

    if paths.len() == 2 {
        return Ok((json, vec![paths[0]], vec![paths[1]]));
    }

    Err(Error::message(
        "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
    ))
}

fn parse_experiments_args(args: &[String]) -> Result<(bool, &str), Error> {
    let mut json = false;
    let mut path = None;

    for arg in args {
        if arg == "--json" {
            json = true;
        } else if path.is_none() {
            path = Some(arg.as_str());
        } else {
            return Err(Error::message(
                "usage: protorev experiments [--json] <manifest>",
            ));
        }
    }

    path.map_or_else(
        || {
            Err(Error::message(
                "usage: protorev experiments [--json] <manifest>",
            ))
        },
        |path| Ok((json, path)),
    )
}

fn read_messages(paths: &[&str]) -> Result<Vec<Message>, Error> {
    let mut messages = Vec::new();
    for path in paths {
        messages.push(read_message(path)?);
    }
    Ok(messages)
}

fn read_pathbuf_messages(paths: &[PathBuf]) -> Result<Vec<Message>, Error> {
    let mut messages = Vec::new();
    for path in paths {
        messages.push(read_message(path)?);
    }
    Ok(messages)
}

fn read_message(path: impl AsRef<Path>) -> Result<Message, Error> {
    let bytes = std::fs::read(path)?;
    Message::decode(&bytes)
}

fn experiments_text(manifest: &ExperimentManifest) -> Result<String, Error> {
    let mut out = String::new();
    for (index, experiment) in manifest.experiments.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        write_experiment_header(&mut out, experiment);
        let before_messages = read_pathbuf_messages(&experiment.before)?;
        let after_messages = read_pathbuf_messages(&experiment.after)?;
        let before = Corpus::from_messages(&before_messages, DEFAULT_MAX_DEPTH);
        let after = Corpus::from_messages(&after_messages, DEFAULT_MAX_DEPTH);
        out.push('\n');
        out.push_str(&Corpus::diff(&before, &after));
    }
    Ok(out)
}

fn experiments_json(manifest: &ExperimentManifest) -> Result<String, Error> {
    let mut out = String::from("{\"experiments\":[");
    for (index, experiment) in manifest.experiments.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let before_messages = read_pathbuf_messages(&experiment.before)?;
        let after_messages = read_pathbuf_messages(&experiment.after)?;
        let before = Corpus::from_messages(&before_messages, DEFAULT_MAX_DEPTH);
        let after = Corpus::from_messages(&after_messages, DEFAULT_MAX_DEPTH);
        let _ = write!(
            out,
            "{{\"name\":\"{}\",\"notes\":",
            json_escape(&experiment.name)
        );
        if let Some(notes) = &experiment.notes {
            let _ = write!(out, "\"{}\"", json_escape(notes));
        } else {
            out.push_str("null");
        }
        out.push_str(",\"before\":");
        write_paths_json(&mut out, &experiment.before);
        out.push_str(",\"after\":");
        write_paths_json(&mut out, &experiment.after);
        out.push_str(",\"diff\":");
        out.push_str(&Corpus::diff_json(&before, &after));
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}

fn write_experiment_header(out: &mut String, experiment: &Experiment) {
    let _ = writeln!(out, "== {} ==", experiment.name);
    if let Some(notes) = &experiment.notes {
        let _ = writeln!(out, "notes: {notes}");
    }
    out.push_str("before:\n");
    for path in &experiment.before {
        let _ = writeln!(out, "  {}", path.display());
    }
    out.push_str("after:\n");
    for path in &experiment.after {
        let _ = writeln!(out, "  {}", path.display());
    }
}

fn write_paths_json(out: &mut String, paths: &[PathBuf]) {
    out.push('[');
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{}\"", json_escape(&path.display().to_string()));
    }
    out.push(']');
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

fn print_usage() {
    println!("protorev: protobuf reverse-engineering workbench");
    println!();
    println!("usage:");
    println!("  protorev dump [--json] <file.pb>");
    println!("  protorev infer <file.pb>...");
    println!("  protorev schema [--min-confidence high|medium|low] <file.pb>...");
    println!("  protorev explain [--json] --field <path> <file.pb>...");
    println!("  protorev values [--json] --field <path> <file.pb>...");
    println!("  protorev diff [--json] <before.pb>... -- <after.pb>...");
    println!("  protorev experiments [--json] <manifest>");
}
