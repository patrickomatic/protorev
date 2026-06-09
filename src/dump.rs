use std::fmt::Write as _;

use crate::classify::LengthDelimitedHints;
use crate::wire::{Field, Message, Value};

const MAX_PREVIEW_BYTES: usize = 24;

pub fn dump_message(message: &Message, max_depth: usize) -> String {
    let mut out = String::new();
    dump_message_inner(&mut out, message, 0, max_depth);
    out
}

fn dump_message_inner(out: &mut String, message: &Message, depth: usize, max_depth: usize) {
    for field in &message.fields {
        dump_field(out, field, depth, max_depth);
    }
}

fn dump_field(out: &mut String, field: &Field, depth: usize, max_depth: usize) {
    let indent = "  ".repeat(depth);
    let _ = write!(
        out,
        "{indent}@{}..{} field {} {}",
        field.tag_offset,
        field.end_offset,
        field.number,
        field.wire_type.name()
    );

    match &field.value {
        Value::Varint(value) => {
            let _ = writeln!(out, " = {value}");
        }
        Value::Fixed64(value) => {
            let _ = writeln!(out, " = {value} / 0x{value:016x}");
        }
        Value::Fixed32(value) => {
            let _ = writeln!(out, " = {value} / 0x{value:08x}");
        }
        Value::LengthDelimited(value) => {
            let hints = LengthDelimitedHints::classify(value);
            let mut labels = Vec::new();
            if hints.nested_message.is_some() {
                labels.push("message");
            }
            if hints.utf8.is_some() {
                labels.push("utf8");
            }
            if hints.packed_varints.is_some() {
                labels.push("packed-varint");
            }

            if labels.is_empty() {
                let _ = writeln!(out, " len={} bytes={}", value.len(), hex_preview(value));
            } else {
                let _ = writeln!(
                    out,
                    " len={} [{}] bytes={}",
                    value.len(),
                    labels.join(", "),
                    hex_preview(value)
                );
            }

            if let Some(text) = hints.utf8 {
                let _ = writeln!(out, "{indent}  text {text:?}");
            }
            if let Some(values) = hints.packed_varints {
                let _ = writeln!(out, "{indent}  packed {values:?}");
            }
            if depth < max_depth
                && let Some(nested) = hints.nested_message
            {
                dump_message_inner(out, &nested, depth + 1, max_depth);
            }
        }
    }
}

fn hex_preview(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (index, byte) in bytes.iter().take(MAX_PREVIEW_BYTES).enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02x}");
    }
    if bytes.len() > MAX_PREVIEW_BYTES {
        out.push_str(" ...");
    }
    out
}
