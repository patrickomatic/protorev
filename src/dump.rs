//! Text dumps for decoded protobuf messages.

use std::fmt::Write as _;

use crate::classify::LengthDelimitedHints;
use crate::wire::{Field, Message, Value};

const MAX_PREVIEW_BYTES: usize = 24;

/// Render a decoded message as a recursive text dump.
///
/// `max_depth` controls how deeply length-delimited nested-message candidates
/// are expanded.
pub fn dump_message(message: &Message, max_depth: usize) -> String {
    let mut out = String::new();
    dump_message_inner(&mut out, message, 0, max_depth);
    out
}

/// Render a decoded message as JSON.
///
/// `max_depth` controls how deeply nested-message candidates are included.
pub fn dump_message_json(message: &Message, max_depth: usize) -> String {
    let mut out = String::new();
    write_message_json(&mut out, message, 0, max_depth);
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

fn write_message_json(out: &mut String, message: &Message, depth: usize, max_depth: usize) {
    let _ = write!(out, "{{\"len\":{},\"fields\":[", message.len);
    for (index, field) in message.fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_field_json(out, field, depth, max_depth);
    }
    out.push_str("]}");
}

fn write_field_json(out: &mut String, field: &Field, depth: usize, max_depth: usize) {
    let _ = write!(
        out,
        "{{\"number\":{},\"wire_type\":\"{}\",\"tag_offset\":{},\"value_offset\":{},\"end_offset\":{},\"value\":",
        field.number,
        field.wire_type.name(),
        field.tag_offset,
        field.value_offset,
        field.end_offset
    );

    match &field.value {
        Value::Varint(value) => {
            let _ = write!(out, "{{\"kind\":\"varint\",\"value\":{value}}}");
        }
        Value::Fixed64(value) => {
            let _ = write!(out, "{{\"kind\":\"fixed64\",\"value\":{value}}}");
        }
        Value::Fixed32(value) => {
            let _ = write!(out, "{{\"kind\":\"fixed32\",\"value\":{value}}}");
        }
        Value::LengthDelimited(value) => {
            write_length_delimited_json(out, value, depth, max_depth);
        }
    }

    out.push('}');
}

fn write_length_delimited_json(out: &mut String, value: &[u8], depth: usize, max_depth: usize) {
    let hints = LengthDelimitedHints::classify(value);
    let _ = write!(
        out,
        "{{\"kind\":\"length-delimited\",\"len\":{},\"hex\":\"{}\",\"hints\":{{\"message\":{},\"utf8\":{},\"packed_varint\":{}}}",
        value.len(),
        hex_full(value),
        hints.nested_message.is_some(),
        hints.utf8.is_some(),
        hints.packed_varints.is_some()
    );

    if let Some(text) = hints.utf8 {
        let _ = write!(out, ",\"text\":\"{}\"", json_escape(&text));
    }
    if let Some(values) = hints.packed_varints {
        out.push_str(",\"packed_varints\":[");
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "{value}");
        }
        out.push(']');
    }
    if depth < max_depth
        && let Some(nested) = hints.nested_message
    {
        out.push_str(",\"nested\":");
        write_message_json(out, &nested, depth + 1, max_depth);
    }

    out.push('}');
}

fn hex_full(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(character));
            }
            character => out.push(character),
        }
    }
    out
}
