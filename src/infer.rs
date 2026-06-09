//! Corpus-level protobuf shape inference.
//!
//! This module aggregates decoded samples into field-presence summaries and a
//! conservative draft `.proto`. It can also emit a stricter schema view that
//! includes only fields that meet a requested confidence threshold. It
//! deliberately avoids semantic scalar inference: values are typed by wire type
//! unless a length-delimited field is consistently observed as a nested message
//! candidate.

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

/// Confidence threshold for schema emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Include unstable or sparsely observed fields.
    Low,
    /// Include fields with stable wire types but incomplete sample coverage.
    Medium,
    /// Include fields with stable wire types observed in every relevant sample.
    High,
}

impl Confidence {
    /// Parse a CLI/API confidence label.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Options controlling conservative schema emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaOptions {
    /// Lowest field confidence included in the emitted schema.
    pub min_confidence: Confidence,
}

impl Default for SchemaOptions {
    fn default() -> Self {
        Self {
            min_confidence: Confidence::High,
        }
    }
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
        self.root.write_summary(&mut out, 1);

        for (path, stats) in &self.nested {
            let _ = writeln!(out, "\n{}:", path.message_name());
            stats.write_summary(&mut out, 1);
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

    /// Emit a confidence-gated `.proto` schema.
    ///
    /// Unlike [`Corpus::draft_proto`], this omits fields whose observed shape
    /// does not meet `options.min_confidence`. The result is still a structural
    /// schema, not a reconstruction of the producer's original `.proto`.
    pub fn schema(&self, options: &SchemaOptions) -> String {
        let mut out = String::new();
        out.push_str("syntax = \"proto3\";\n\n");
        self.write_schema_message(&mut out, "Message", &self.root, None, *options);

        for (path, stats) in &self.nested {
            if self.message_has_schema_fields(stats, Some(path), *options) {
                self.write_schema_message(
                    &mut out,
                    &path.message_name(),
                    stats,
                    Some(path),
                    *options,
                );
            }
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

    fn write_schema_message(
        &self,
        out: &mut String,
        name: &str,
        stats: &MessageStats,
        path: Option<&FieldPath>,
        options: SchemaOptions,
    ) {
        let _ = writeln!(out, "message {name} {{");
        let mut emitted = false;
        for (number, field) in &stats.fields {
            let confidence = field.confidence(stats.message_observations);
            if confidence < options.min_confidence {
                continue;
            }

            let child_path = match path {
                Some(parent) => parent.child(*number),
                None => FieldPath::root_field(*number),
            };
            let type_name = self.schema_type_name(field, &child_path);
            let label = if field.max_occurrences_per_sample > 1 {
                "repeated "
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "  {label}{type_name} {} = {}; // confidence: {}; observed {}/{} samples; wires: {}; {}",
                child_path.field_name(),
                number,
                confidence.label(),
                field.samples_seen,
                stats.message_observations,
                field.wire_summary(),
                field.evidence_summary()
            );
            emitted = true;
        }

        if !emitted {
            let _ = writeln!(
                out,
                "  // No fields met confidence threshold {}.",
                options.min_confidence.label()
            );
        }

        out.push_str("}\n\n");
    }

    fn message_has_schema_fields(
        &self,
        stats: &MessageStats,
        path: Option<&FieldPath>,
        options: SchemaOptions,
    ) -> bool {
        stats.fields.iter().any(|(number, field)| {
            let child_path = match path {
                Some(parent) => parent.child(*number),
                None => FieldPath::root_field(*number),
            };
            field.confidence(stats.message_observations) >= options.min_confidence
                || self.nested.get(&child_path).is_some_and(|nested| {
                    self.message_has_schema_fields(nested, Some(&child_path), options)
                })
        })
    }

    fn schema_type_name(&self, field: &FieldStats, child_path: &FieldPath) -> String {
        if field.is_consistent_nested_message() && self.nested.contains_key(child_path) {
            child_path.message_name()
        } else {
            field.primary_wire_type().proto_scalar().to_owned()
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MessageStats {
    message_observations: usize,
    fields: BTreeMap<u32, FieldStats>,
}

impl MessageStats {
    fn observe(&mut self, message: &Message) {
        self.message_observations += 1;
        let mut counts = BTreeMap::<u32, usize>::new();
        for field in &message.fields {
            counts
                .entry(field.number)
                .and_modify(|count| *count += 1)
                .or_insert(1);
            self.fields
                .entry(field.number)
                .or_default()
                .observe_field(field);
        }

        for (number, count) in counts {
            let stats = self.fields.entry(number).or_default();
            stats.samples_seen += 1;
            stats.max_occurrences_per_sample = stats.max_occurrences_per_sample.max(count);
        }
    }

    fn write_summary(&self, out: &mut String, indent: usize) {
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
                self.message_observations,
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
    occurrence_count: usize,
    max_occurrences_per_sample: usize,
    wire_types: BTreeSet<WireType>,
    nested_message_occurrences: usize,
    utf8_occurrences: usize,
    packed_varint_occurrences: usize,
}

impl FieldStats {
    fn observe_field(&mut self, field: &crate::wire::Field) {
        self.occurrence_count += 1;
        self.wire_types.insert(field.wire_type);

        let Value::LengthDelimited(bytes) = &field.value else {
            return;
        };
        let hints = LengthDelimitedHints::classify(bytes);
        if hints.nested_message.is_some() {
            self.nested_message_occurrences += 1;
        }
        if hints.utf8.is_some() {
            self.utf8_occurrences += 1;
        }
        if hints.packed_varints.is_some() {
            self.packed_varint_occurrences += 1;
        }
    }

    fn confidence(&self, message_observations: usize) -> Confidence {
        if self.samples_seen == 0 || self.wire_types.len() != 1 {
            return Confidence::Low;
        }

        if message_observations < 2 {
            return Confidence::Medium;
        }

        if self.samples_seen == message_observations {
            Confidence::High
        } else {
            Confidence::Medium
        }
    }

    fn is_consistent_nested_message(&self) -> bool {
        self.primary_wire_type() == WireType::LengthDelimited
            && self.occurrence_count > 0
            && self.nested_message_occurrences == self.occurrence_count
    }

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

    fn evidence_summary(&self) -> String {
        if self.primary_wire_type() != WireType::LengthDelimited {
            return format!("occurrences: {}", self.occurrence_count);
        }

        format!(
            "occurrences: {}; nested: {}/{}; utf8: {}/{}; packed-varint: {}/{}",
            self.occurrence_count,
            self.nested_message_occurrences,
            self.occurrence_count,
            self.utf8_occurrences,
            self.occurrence_count,
            self.packed_varint_occurrences,
            self.occurrence_count
        )
    }
}
