//! Heuristics for length-delimited protobuf values.
//!
//! Length-delimited fields are ambiguous without a schema: the same bytes may
//! be a nested message, a string, packed scalars, or opaque bytes. This module
//! reports candidates and leaves interpretation to the caller.

use crate::wire::{Message, read_varint};

/// Candidate interpretations for a length-delimited value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LengthDelimitedHints {
    /// Present when the full payload decodes cleanly as a protobuf message.
    pub nested_message: Option<Message>,
    /// Present when the full payload is valid UTF-8 text.
    pub utf8: Option<String>,
    /// Present when the full payload decodes as two or more varints.
    pub packed_varints: Option<Vec<u64>>,
}

impl LengthDelimitedHints {
    /// Classify one length-delimited payload.
    pub fn classify(bytes: &[u8]) -> Self {
        let utf8 = classify_utf8(bytes);
        let packed_varints = if utf8.is_none() {
            classify_packed_varints(bytes)
        } else {
            None
        };

        Self {
            nested_message: classify_nested_message(bytes),
            utf8,
            packed_varints,
        }
    }
}

fn classify_nested_message(bytes: &[u8]) -> Option<Message> {
    if bytes.is_empty() {
        return None;
    }

    let message = Message::decode(bytes).ok()?;
    if message.fields.is_empty() {
        None
    } else {
        Some(message)
    }
}

fn classify_utf8(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.is_empty() || !text.chars().all(is_text_landmark) {
        return None;
    }
    Some(text.to_owned())
}

fn is_text_landmark(value: char) -> bool {
    !value.is_control() || matches!(value, '\n' | '\r' | '\t')
}

fn classify_packed_varints(bytes: &[u8]) -> Option<Vec<u64>> {
    if bytes.is_empty() {
        return None;
    }

    let mut cursor = 0;
    let mut values = Vec::new();
    while cursor < bytes.len() {
        values.push(read_varint(bytes, &mut cursor).ok()?);
    }

    if values.len() > 1 { Some(values) } else { None }
}
