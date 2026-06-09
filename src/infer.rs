//! Corpus-level protobuf shape inference.
//!
//! This module aggregates decoded samples into field-presence summaries and a
//! conservative draft `.proto`. It deliberately avoids semantic scalar
//! inference: values are typed by wire type unless a length-delimited field is
//! consistently observed as a nested message candidate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::classify::LengthDelimitedHints;
use crate::wire::{Message, Value, WireType};

/// A nested field path, such as `1.4.2`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath(Vec<u32>);

impl FieldPath {
    fn root_field(number: u32) -> Self {
        Self(vec![number])
    }

    fn child(&self, number: u32) -> Self {
        let mut path = self.0.clone();
        path.push(number);
        Self(path)
    }

    fn message_name(&self) -> String {
        if self.0.is_empty() {
            return "Message".to_owned();
        }
        let suffix = self
            .0
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("_");
        format!("Message_{suffix}")
    }

    fn field_name(&self) -> String {
        match self.0.last() {
            Some(number) => format!("field_{number}"),
            None => "field".to_owned(),
        }
    }
}

/// Aggregated observations from a set of decoded messages.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    sample_count: usize,
    root: MessageStats,
    nested: BTreeMap<FieldPath, MessageStats>,
}

impl Corpus {
    /// Build a corpus from decoded sample messages.
    ///
    /// `max_depth` limits recursive nested-message candidate aggregation.
    pub fn from_messages(messages: &[Message], max_depth: usize) -> Self {
        let mut corpus = Self {
            sample_count: messages.len(),
            root: MessageStats::default(),
            nested: BTreeMap::new(),
        };

        for message in messages {
            corpus.root.observe(message);
            corpus.observe_nested_message(message, max_depth);
        }

        corpus
    }

    /// Produce a human-readable field presence summary.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "samples: {}", self.sample_count);
        out.push_str("\nroot:\n");
        self.root.write_summary(&mut out, 1, self.sample_count);

        for (path, stats) in &self.nested {
            let _ = writeln!(out, "\n{}:", path.message_name());
            stats.write_summary(&mut out, 1, self.sample_count);
        }

        out
    }

    /// Emit a conservative draft `.proto`.
    ///
    /// The result is intended as a starting point for human review, not a final
    /// schema. Field names are synthetic and comments carry the observation
    /// counts that led to each emitted line.
    pub fn draft_proto(&self) -> String {
        let mut out = String::new();
        out.push_str("syntax = \"proto3\";\n\n");
        self.write_message_proto(&mut out, "Message", &self.root, None);

        for (path, stats) in &self.nested {
            self.write_message_proto(&mut out, &path.message_name(), stats, Some(path));
        }

        out
    }

    fn observe_nested_message(&mut self, message: &Message, max_depth: usize) {
        for field in &message.fields {
            let path = FieldPath::root_field(field.number);
            self.observe_nested_field(&path, &field.value, 1, max_depth);
        }
    }

    fn observe_nested_field(
        &mut self,
        path: &FieldPath,
        value: &Value,
        depth: usize,
        max_depth: usize,
    ) {
        if depth > max_depth {
            return;
        }

        let Value::LengthDelimited(bytes) = value else {
            return;
        };
        let hints = LengthDelimitedHints::classify(bytes);
        let Some(message) = hints.nested_message else {
            return;
        };

        self.nested
            .entry(path.clone())
            .or_default()
            .observe(&message);
        for field in &message.fields {
            self.observe_nested_field(
                &path.child(field.number),
                &field.value,
                depth + 1,
                max_depth,
            );
        }
    }

    fn write_message_proto(
        &self,
        out: &mut String,
        name: &str,
        stats: &MessageStats,
        path: Option<&FieldPath>,
    ) {
        let _ = writeln!(out, "message {name} {{");
        for (number, field) in &stats.fields {
            let child_path = match path {
                Some(parent) => parent.child(*number),
                None => FieldPath::root_field(*number),
            };
            let type_name = if self.nested.contains_key(&child_path) {
                child_path.message_name()
            } else {
                field.primary_wire_type().proto_scalar().to_owned()
            };
            let label = if field.max_occurrences_per_sample > 1 {
                "repeated "
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "  {label}{type_name} {} = {}; // observed {}/{} samples; wires: {}",
                child_path.field_name(),
                number,
                field.samples_seen,
                self.sample_count,
                field.wire_summary()
            );
        }
        out.push_str("}\n\n");
    }
}

#[derive(Debug, Clone, Default)]
struct MessageStats {
    fields: BTreeMap<u32, FieldStats>,
}

impl MessageStats {
    fn observe(&mut self, message: &Message) {
        let mut counts = BTreeMap::<u32, usize>::new();
        for field in &message.fields {
            counts
                .entry(field.number)
                .and_modify(|count| *count += 1)
                .or_insert(1);
            self.fields
                .entry(field.number)
                .or_default()
                .wire_types
                .insert(field.wire_type);
        }

        for (number, count) in counts {
            let stats = self.fields.entry(number).or_default();
            stats.samples_seen += 1;
            stats.max_occurrences_per_sample = stats.max_occurrences_per_sample.max(count);
        }
    }

    fn write_summary(&self, out: &mut String, indent: usize, sample_count: usize) {
        let padding = "  ".repeat(indent);
        for (number, field) in &self.fields {
            let repeated = if field.max_occurrences_per_sample > 1 {
                " repeated"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "{padding}field {number}: observed {}/{} samples; wires: {}; max/sample: {}{}",
                field.samples_seen,
                sample_count,
                field.wire_summary(),
                field.max_occurrences_per_sample,
                repeated
            );
        }
    }
}

#[derive(Debug, Clone, Default)]
struct FieldStats {
    samples_seen: usize,
    max_occurrences_per_sample: usize,
    wire_types: BTreeSet<WireType>,
}

impl FieldStats {
    fn primary_wire_type(&self) -> WireType {
        self.wire_types
            .iter()
            .next()
            .copied()
            .unwrap_or(WireType::LengthDelimited)
    }

    fn wire_summary(&self) -> String {
        self.wire_types
            .iter()
            .map(|wire| wire.name())
            .collect::<Vec<_>>()
            .join(",")
    }
}
