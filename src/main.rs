use std::path::Path;

use protorev::{
    Confidence, Corpus, Error, FieldPath, Message, SchemaOptions, dump_message, dump_message_json,
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
        "diff" => diff_command(&paths),
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

fn diff_command(paths: &[String]) -> Result<(), Error> {
    if paths.len() != 2 {
        return Err(Error::message(
            "usage: protorev diff <before.pb> <after.pb>",
        ));
    }

    let before = read_message(&paths[0])?;
    let after = read_message(&paths[1])?;
    let corpus = Corpus::from_messages(&[before, after], DEFAULT_MAX_DEPTH);
    print!("{}", corpus.summary());
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
                return Err(Error::message(
                    "usage: protorev explain [--json] --field <path> <file.pb>...",
                ));
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
        return Err(Error::message(
            "usage: protorev explain [--json] --field <path> <file.pb>...",
        ));
    };

    Ok((json, field_path, paths))
}

fn read_message(path: impl AsRef<Path>) -> Result<Message, Error> {
    let bytes = std::fs::read(path)?;
    Message::decode(&bytes)
}

fn print_usage() {
    println!("protorev: protobuf reverse-engineering workbench");
    println!();
    println!("usage:");
    println!("  protorev dump [--json] <file.pb>");
    println!("  protorev infer <file.pb>...");
    println!("  protorev schema [--min-confidence high|medium|low] <file.pb>...");
    println!("  protorev explain [--json] --field <path> <file.pb>...");
    println!("  protorev diff <before.pb> <after.pb>");
}
