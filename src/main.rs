use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args, CommandFactory, Parser, Subcommand};
use protorev::{
    Confidence, Corpus, Error, Experiment, ExperimentManifest, FieldPath, Message, SchemaOptions,
    dump_message, dump_message_json,
};

const DEFAULT_MAX_DEPTH: usize = 4;

#[derive(Debug, Parser)]
#[command(
    name = "protorev",
    version,
    about = "A protobuf reverse-engineering workbench"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Decode one raw protobuf message and print observed fields.
    Dump(DumpArgs),
    /// Aggregate field observations across a corpus and emit a draft proto.
    Infer(FilesArgs),
    /// Emit a confidence-gated structural proto.
    Schema(SchemaArgs),
    /// Explain the evidence behind one field path.
    Explain(FieldCommandArgs),
    /// Summarize observed values for one field path.
    Values(FieldCommandArgs),
    /// Compare two corpora and report structural changes.
    Diff(DiffArgs),
    /// Run named before/after corpus comparisons from a manifest.
    #[command(alias = "experiment")]
    Experiments(ExperimentsArgs),
}

#[derive(Debug, Args)]
struct DumpArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Raw protobuf message to decode.
    #[arg(value_name = "file.pb")]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct FilesArgs {
    /// Raw protobuf messages to analyze.
    #[arg(value_name = "file.pb", num_args = 1..)]
    files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct SchemaArgs {
    /// Lowest confidence level to include.
    #[arg(long, value_name = "high|medium|low", value_parser = parse_confidence)]
    min_confidence: Option<Confidence>,
    /// Raw protobuf messages to analyze.
    #[arg(value_name = "file.pb", num_args = 1..)]
    files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct FieldCommandArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Dotted field path, such as 1 or 3.1.
    #[arg(long, value_name = "path", value_parser = parse_field_path)]
    field: FieldPath,
    /// Raw protobuf messages to analyze.
    #[arg(value_name = "file.pb", num_args = 1..)]
    files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct DiffArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Before/after files. Use `--` between multi-file corpora.
    #[arg(value_name = "PATH", num_args = 2.., trailing_var_arg = true, allow_hyphen_values = true)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct ExperimentsArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Experiment manifest to run.
    #[arg(value_name = "manifest")]
    manifest: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };

    match cli.command {
        Some(Commands::Dump(args)) => dump_command(&args),
        Some(Commands::Infer(args)) => infer_command(&args.files),
        Some(Commands::Schema(args)) => schema_command(&args),
        Some(Commands::Explain(args)) => explain_command(&args),
        Some(Commands::Values(args)) => values_command(&args),
        Some(Commands::Diff(args)) => diff_command(&args),
        Some(Commands::Experiments(args)) => experiments_command(&args),
        None => {
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

fn dump_command(args: &DumpArgs) -> Result<(), Error> {
    let message = read_message(&args.file)?;
    if args.json {
        println!("{}", dump_message_json(&message, DEFAULT_MAX_DEPTH));
    } else {
        print!("{}", dump_message(&message, DEFAULT_MAX_DEPTH));
    }
    Ok(())
}

fn infer_command(paths: &[PathBuf]) -> Result<(), Error> {
    let messages = read_messages(paths)?;
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    print!("{}", corpus.summary());
    println!("\n--- draft proto ---\n");
    print!("{}", corpus.draft_proto());
    Ok(())
}

fn schema_command(args: &SchemaArgs) -> Result<(), Error> {
    let mut options = SchemaOptions::default();
    if let Some(min_confidence) = args.min_confidence {
        options.min_confidence = min_confidence;
    }
    let messages = read_messages(&args.files)?;
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    print!("{}", corpus.schema(&options));
    Ok(())
}

fn explain_command(args: &FieldCommandArgs) -> Result<(), Error> {
    let messages = read_messages(&args.files)?;
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    let output = if args.json {
        corpus.explain_json(&args.field)
    } else {
        corpus.explain(&args.field)
    };
    match output {
        Some(output) => {
            print!("{output}");
            if args.json {
                println!();
            }
            Ok(())
        }
        None => Err(Error::message(format!(
            "field {} was not observed in the corpus",
            args.field
        ))),
    }
}

fn values_command(args: &FieldCommandArgs) -> Result<(), Error> {
    let messages = read_messages(&args.files)?;
    let corpus = Corpus::from_messages(&messages, DEFAULT_MAX_DEPTH);
    let output = if args.json {
        corpus.values_json(&messages, &args.field)
    } else {
        corpus.values(&messages, &args.field)
    };
    match output {
        Some(output) => {
            print!("{output}");
            if args.json {
                println!();
            }
            Ok(())
        }
        None => Err(Error::message(format!(
            "field {} had no observed values in the corpus",
            args.field
        ))),
    }
}

fn diff_command(args: &DiffArgs) -> Result<(), Error> {
    let (before_paths, after_paths) = split_diff_paths(&args.paths)?;
    let before_messages = read_messages(&before_paths)?;
    let after_messages = read_messages(&after_paths)?;
    let before = Corpus::from_messages(&before_messages, DEFAULT_MAX_DEPTH);
    let after = Corpus::from_messages(&after_messages, DEFAULT_MAX_DEPTH);
    if args.json {
        println!("{}", Corpus::diff_json(&before, &after));
    } else {
        print!("{}", Corpus::diff(&before, &after));
    }
    Ok(())
}

fn experiments_command(args: &ExperimentsArgs) -> Result<(), Error> {
    let manifest = ExperimentManifest::from_file(&args.manifest)?;
    if args.json {
        println!("{}", experiments_json(&manifest)?);
    } else {
        print!("{}", experiments_text(&manifest)?);
    }
    Ok(())
}

fn split_diff_paths(paths: &[PathBuf]) -> Result<(Vec<PathBuf>, Vec<PathBuf>), Error> {
    let mut separator = None;

    for (index, path) in paths.iter().enumerate() {
        if path.as_os_str().to_str() == Some("--") {
            if separator.is_some() {
                return Err(Error::message(
                    "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
                ));
            }
            separator = Some(index);
        }
    }

    if let Some(separator) = separator {
        let before = paths[..separator].to_vec();
        let after = paths[separator + 1..].to_vec();
        if before.is_empty() || after.is_empty() {
            return Err(Error::message(
                "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
            ));
        }
        return Ok((before, after));
    }

    if paths.len() == 2 {
        return Ok((vec![paths[0].clone()], vec![paths[1].clone()]));
    }

    Err(Error::message(
        "usage: protorev diff [--json] <before.pb>... -- <after.pb>...",
    ))
}

fn parse_confidence(value: &str) -> Result<Confidence, String> {
    Confidence::parse(value).ok_or_else(|| String::from("must be one of: high, medium, low"))
}

fn parse_field_path(value: &str) -> Result<FieldPath, String> {
    FieldPath::parse(value).ok_or_else(|| String::from("must look like 1 or 3.1"))
}

fn read_messages(paths: &[PathBuf]) -> Result<Vec<Message>, Error> {
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
        let before_messages = read_messages(&experiment.before)?;
        let after_messages = read_messages(&experiment.after)?;
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
        let before_messages = read_messages(&experiment.before)?;
        let after_messages = read_messages(&experiment.after)?;
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
