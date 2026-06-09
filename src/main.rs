use std::path::Path;

use protorev::{Corpus, Error, Message, dump_message};

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
        "diff" => diff_command(&paths),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        _ => Err(Error::message(format!("unknown command {command:?}"))),
    }
}

fn dump_command(paths: &[String]) -> Result<(), Error> {
    if paths.len() != 1 {
        return Err(Error::message("usage: protorev dump <file.pb>"));
    }

    let message = read_message(&paths[0])?;
    print!("{}", dump_message(&message, DEFAULT_MAX_DEPTH));
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

fn read_message(path: impl AsRef<Path>) -> Result<Message, Error> {
    let bytes = std::fs::read(path)?;
    Message::decode(&bytes)
}

fn print_usage() {
    println!("protorev: protobuf reverse-engineering workbench");
    println!();
    println!("usage:");
    println!("  protorev dump <file.pb>");
    println!("  protorev infer <file.pb>...");
    println!("  protorev diff <before.pb> <after.pb>");
}
