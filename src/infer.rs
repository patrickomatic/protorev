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
use std::fmt::{Display, Formatter};

use crate::classify::LengthDelimitedHints;
use crate::wire::{Message, Value, WireType};

/// A nested field path, such as `1.4.2`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath(Vec<u32>);

impl FieldPath {
    /// Parse a dotted field path, such as `1` or `3.1`.
    pub fn parse(value: &str) -> Option<Self> {
        let mut path = Vec::new();
        for part in value.split('.') {
            if part.is_empty() {
                return None;
            }
            let number = part.parse::<u32>().ok()?;
            if number == 0 {
                return None;
            }
            path.push(number);
        }

        if path.is_empty() {
            None
        } else {
            Some(Self(path))
        }
    }

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

impl Display for FieldPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        for (index, number) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(".")?;
            }
            write!(formatter, "{number}")?;
        }
        Ok(())
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

    /// Explain the evidence for one field path.
    pub fn explain(&self, path: &FieldPath) -> Option<String> {
        let (parent_stats, field) = self.field_stats(path)?;
        let mut out = String::new();
        let confidence = field.confidence(parent_stats.message_observations);
        let type_name = self.schema_type_name(field, path);

        let _ = writeln!(out, "field {path}");
        let _ = writeln!(out, "  name: {}", path.field_name());
        let _ = writeln!(out, "  schema type: {type_name}");
        let _ = writeln!(out, "  confidence: {}", confidence.label());
        let _ = writeln!(
            out,
            "  observed: {}/{} relevant messages",
            field.samples_seen, parent_stats.message_observations
        );
        let _ = writeln!(out, "  occurrences: {}", field.occurrence_count);
        let _ = writeln!(
            out,
            "  max occurrences per message: {}",
            field.max_occurrences_per_sample
        );
        let _ = writeln!(out, "  wire types: {}", field.wire_summary());
        let _ = writeln!(out, "  evidence: {}", field.evidence_summary());
        let _ = writeln!(
            out,
            "  included at high threshold: {}",
            yes_no(confidence >= Confidence::High)
        );
        let _ = writeln!(
            out,
            "  included at medium threshold: {}",
            yes_no(confidence >= Confidence::Medium)
        );
        let _ = writeln!(
            out,
            "  included at low threshold: {}",
            yes_no(confidence >= Confidence::Low)
        );

        Some(out)
    }

    /// Explain one field path as JSON.
    pub fn explain_json(&self, path: &FieldPath) -> Option<String> {
        let (parent_stats, field) = self.field_stats(path)?;
        let confidence = field.confidence(parent_stats.message_observations);
        let type_name = self.schema_type_name(field, path);
        let mut out = String::new();

        let _ = write!(
            out,
            "{{\"path\":\"{path}\",\"name\":\"{}\",\"schema_type\":\"{}\",\"confidence\":\"{}\",\"observed_messages\":{},\"relevant_messages\":{},\"occurrences\":{},\"max_occurrences_per_message\":{},\"wire_types\":[",
            path.field_name(),
            type_name,
            confidence.label(),
            field.samples_seen,
            parent_stats.message_observations,
            field.occurrence_count,
            field.max_occurrences_per_sample
        );
        for (index, wire_type) in field.wire_types.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{}\"", wire_type.name());
        }
        let _ = write!(
            out,
            "],\"length_delimited\":{{\"nested_message_occurrences\":{},\"utf8_occurrences\":{},\"packed_varint_occurrences\":{}}},\"included\":{{\"high\":{},\"medium\":{},\"low\":{}}}}}",
            field.nested_message_occurrences,
            field.utf8_occurrences,
            field.packed_varint_occurrences,
            confidence >= Confidence::High,
            confidence >= Confidence::Medium,
            confidence >= Confidence::Low
        );

        Some(out)
    }

    /// Summarize observed values for one field path.
    pub fn values(&self, messages: &[Message], path: &FieldPath) -> Option<String> {
        let observations = collect_value_observations(messages, path);
        if observations.is_empty() {
            return None;
        }

        let mut out = String::new();
        let _ = writeln!(out, "field {path}");
        let _ = writeln!(out, "  occurrences: {}", observations.len());
        write_value_summary(&mut out, &observations, 1);
        Some(out)
    }

    /// Summarize observed values for one field path as JSON.
    pub fn values_json(&self, messages: &[Message], path: &FieldPath) -> Option<String> {
        let observations = collect_value_observations(messages, path);
        if observations.is_empty() {
            return None;
        }

        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"path\":\"{path}\",\"occurrences\":{}",
            observations.len()
        );
        write_value_summary_json(&mut out, &observations);
        out.push('}');
        Some(out)
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

    fn field_stats(&self, path: &FieldPath) -> Option<(&MessageStats, &FieldStats)> {
        let number = *path.0.last()?;
        let parent = if path.0.len() == 1 {
            &self.root
        } else {
            let parent_path = FieldPath(path.0[..path.0.len() - 1].to_vec());
            self.nested.get(&parent_path)?
        };
        parent.fields.get(&number).map(|field| (parent, field))
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[derive(Debug, Clone)]
struct ValueObservation {
    wire_type: WireType,
    value: Value,
}

fn collect_value_observations(messages: &[Message], path: &FieldPath) -> Vec<ValueObservation> {
    let mut observations = Vec::new();
    for message in messages {
        collect_value_observations_from_message(message, &path.0, &mut observations);
    }
    observations
}

fn collect_value_observations_from_message(
    message: &Message,
    path: &[u32],
    observations: &mut Vec<ValueObservation>,
) {
    let Some((number, rest)) = path.split_first() else {
        return;
    };

    for field in message
        .fields
        .iter()
        .filter(|field| field.number == *number)
    {
        if rest.is_empty() {
            observations.push(ValueObservation {
                wire_type: field.wire_type,
                value: field.value.clone(),
            });
            continue;
        }

        let Value::LengthDelimited(bytes) = &field.value else {
            continue;
        };
        let Ok(nested) = Message::decode(bytes) else {
            continue;
        };
        collect_value_observations_from_message(&nested, rest, observations);
    }
}

fn write_value_summary(out: &mut String, observations: &[ValueObservation], indent: usize) {
    let padding = "  ".repeat(indent);
    let mut wire_types = BTreeSet::new();
    for observation in observations {
        wire_types.insert(observation.wire_type);
    }
    let wire_summary = wire_types
        .iter()
        .map(|wire| wire.name())
        .collect::<Vec<_>>()
        .join(",");
    let _ = writeln!(out, "{padding}wire types: {wire_summary}");

    if let Some(values) = collect_varints(observations) {
        write_integer_summary(out, &padding, &values, "varint");
    }
    if let Some(values) = collect_fixed32(observations) {
        write_integer_summary(out, &padding, &values, "fixed32");
    }
    if let Some(values) = collect_fixed64(observations) {
        write_integer_summary(out, &padding, &values, "fixed64");
    }

    let length_values = collect_length_delimited(observations);
    if !length_values.is_empty() {
        write_length_delimited_summary(out, &padding, &length_values);
    }
}

fn write_value_summary_json(out: &mut String, observations: &[ValueObservation]) {
    let mut wire_types = BTreeSet::new();
    for observation in observations {
        wire_types.insert(observation.wire_type);
    }

    out.push_str(",\"wire_types\":[");
    for (index, wire_type) in wire_types.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{}\"", wire_type.name());
    }
    out.push(']');

    if let Some(values) = collect_varints(observations) {
        write_integer_summary_json(out, &values, "varint");
    }
    if let Some(values) = collect_fixed32(observations) {
        write_integer_summary_json(out, &values, "fixed32");
    }
    if let Some(values) = collect_fixed64(observations) {
        write_integer_summary_json(out, &values, "fixed64");
    }

    let length_values = collect_length_delimited(observations);
    if !length_values.is_empty() {
        write_length_delimited_summary_json(out, &length_values);
    }
}

fn collect_varints(observations: &[ValueObservation]) -> Option<Vec<u64>> {
    let values = observations
        .iter()
        .filter_map(|observation| match observation.value {
            Value::Varint(value) => Some(value),
            Value::Fixed64(_) | Value::LengthDelimited(_) | Value::Fixed32(_) => None,
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn collect_fixed32(observations: &[ValueObservation]) -> Option<Vec<u64>> {
    let values = observations
        .iter()
        .filter_map(|observation| match observation.value {
            Value::Fixed32(value) => Some(u64::from(value)),
            Value::Varint(_) | Value::Fixed64(_) | Value::LengthDelimited(_) => None,
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn collect_fixed64(observations: &[ValueObservation]) -> Option<Vec<u64>> {
    let values = observations
        .iter()
        .filter_map(|observation| match observation.value {
            Value::Fixed64(value) => Some(value),
            Value::Varint(_) | Value::LengthDelimited(_) | Value::Fixed32(_) => None,
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn collect_length_delimited(observations: &[ValueObservation]) -> Vec<&[u8]> {
    observations
        .iter()
        .filter_map(|observation| match &observation.value {
            Value::LengthDelimited(bytes) => Some(bytes.as_slice()),
            Value::Varint(_) | Value::Fixed64(_) | Value::Fixed32(_) => None,
        })
        .collect()
}

fn write_integer_summary(out: &mut String, padding: &str, values: &[u64], label: &str) {
    let Some(min) = values.iter().min() else {
        return;
    };
    let Some(max) = values.iter().max() else {
        return;
    };
    let counts = value_counts(values);

    let _ = writeln!(out, "{padding}{label}:");
    let _ = writeln!(out, "{padding}  min: {min}");
    let _ = writeln!(out, "{padding}  max: {max}");
    let _ = writeln!(out, "{padding}  distinct: {}", counts.len());
    let _ = writeln!(out, "{padding}  common:");
    for (value, count) in counts.iter().take(5) {
        let _ = writeln!(out, "{padding}    {value}: {count}");
    }
    let _ = writeln!(out, "{padding}  candidates:");
    let _ = writeln!(
        out,
        "{padding}    bool: {}",
        yes_no(is_bool_candidate(values))
    );
    let _ = writeln!(
        out,
        "{padding}    enum: {}",
        yes_no(is_enum_candidate(values))
    );
    let _ = writeln!(
        out,
        "{padding}    counter_or_id: {}",
        yes_no(is_counter_or_id_candidate(values))
    );
}

fn write_integer_summary_json(out: &mut String, values: &[u64], label: &str) {
    let Some(min) = values.iter().min() else {
        return;
    };
    let Some(max) = values.iter().max() else {
        return;
    };
    let counts = value_counts(values);

    let _ = write!(
        out,
        ",\"{label}\":{{\"min\":{min},\"max\":{max},\"distinct\":{},\"common\":[",
        counts.len()
    );
    for (index, (value, count)) in counts.iter().take(5).enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"value\":{value},\"count\":{count}}}");
    }
    let _ = write!(
        out,
        "],\"candidates\":{{\"bool\":{},\"enum\":{},\"counter_or_id\":{}}}}}",
        is_bool_candidate(values),
        is_enum_candidate(values),
        is_counter_or_id_candidate(values)
    );
}

fn write_length_delimited_summary(out: &mut String, padding: &str, values: &[&[u8]]) {
    let lengths = values
        .iter()
        .map(|value| value.len() as u64)
        .collect::<Vec<_>>();
    let Some(min_len) = lengths.iter().min() else {
        return;
    };
    let Some(max_len) = lengths.iter().max() else {
        return;
    };

    let mut nested = 0usize;
    let mut utf8 = 0usize;
    let mut packed = 0usize;
    let mut texts = BTreeMap::<String, usize>::new();
    for value in values {
        let hints = LengthDelimitedHints::classify(value);
        if hints.nested_message.is_some() {
            nested += 1;
        }
        if let Some(text) = hints.utf8 {
            utf8 += 1;
            texts
                .entry(text)
                .and_modify(|count| *count += 1)
                .or_insert(1);
        }
        if hints.packed_varints.is_some() {
            packed += 1;
        }
    }

    let _ = writeln!(out, "{padding}length-delimited:");
    let _ = writeln!(out, "{padding}  lengths:");
    let _ = writeln!(out, "{padding}    min: {min_len}");
    let _ = writeln!(out, "{padding}    max: {max_len}");
    let _ = writeln!(out, "{padding}  utf8: {utf8}/{}", values.len());
    let _ = writeln!(out, "{padding}  nested message: {nested}/{}", values.len());
    let _ = writeln!(out, "{padding}  packed varint: {packed}/{}", values.len());
    if !texts.is_empty() {
        let _ = writeln!(out, "{padding}  text distinct: {}", texts.len());
        let _ = writeln!(out, "{padding}  text common:");
        for (text, count) in sorted_text_counts(&texts).into_iter().take(5) {
            let _ = writeln!(out, "{padding}    {text:?}: {count}");
        }
    }
}

fn write_length_delimited_summary_json(out: &mut String, values: &[&[u8]]) {
    let lengths = values
        .iter()
        .map(|value| value.len() as u64)
        .collect::<Vec<_>>();
    let Some(min_len) = lengths.iter().min() else {
        return;
    };
    let Some(max_len) = lengths.iter().max() else {
        return;
    };

    let mut nested = 0usize;
    let mut utf8 = 0usize;
    let mut packed = 0usize;
    let mut texts = BTreeMap::<String, usize>::new();
    for value in values {
        let hints = LengthDelimitedHints::classify(value);
        if hints.nested_message.is_some() {
            nested += 1;
        }
        if let Some(text) = hints.utf8 {
            utf8 += 1;
            texts
                .entry(text)
                .and_modify(|count| *count += 1)
                .or_insert(1);
        }
        if hints.packed_varints.is_some() {
            packed += 1;
        }
    }

    let _ = write!(
        out,
        ",\"length_delimited\":{{\"min_len\":{min_len},\"max_len\":{max_len},\"utf8_occurrences\":{utf8},\"nested_message_occurrences\":{nested},\"packed_varint_occurrences\":{packed},\"text_distinct\":{},\"text_common\":[",
        texts.len()
    );
    for (index, (text, count)) in sorted_text_counts(&texts).into_iter().take(5).enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"value\":\"{}\",\"count\":{count}}}",
            json_escape(text)
        );
    }
    out.push_str("]}");
}

fn value_counts(values: &[u64]) -> Vec<(u64, usize)> {
    let mut counts = BTreeMap::<u64, usize>::new();
    for value in values {
        counts
            .entry(*value)
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    counts
}

fn sorted_text_counts(counts: &BTreeMap<String, usize>) -> Vec<(&str, usize)> {
    let mut counts = counts
        .iter()
        .map(|(value, count)| (value.as_str(), *count))
        .collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    counts
}

fn is_bool_candidate(values: &[u64]) -> bool {
    values.iter().all(|value| matches!(value, 0 | 1))
}

fn is_enum_candidate(values: &[u64]) -> bool {
    let counts = value_counts(values);
    counts.len() <= 16 && counts.iter().all(|(value, _)| *value <= 1024)
}

fn is_counter_or_id_candidate(values: &[u64]) -> bool {
    let Some(min) = values.iter().min() else {
        return false;
    };
    let Some(max) = values.iter().max() else {
        return false;
    };
    *max > 16 && *max > *min
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
