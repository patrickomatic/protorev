//! Raw protobuf wire decoding.
//!
//! This module intentionally stays schema-less. It exposes the wire type and
//! exact byte offsets for every field so higher layers can reason about samples
//! without losing the evidence needed to revisit a hypothesis.

use crate::Error;

/// A decoded protobuf message.
///
/// `len` is the length of the input slice. Every field offset is relative to
/// the start of that same slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Fields in wire order.
    pub fields: Vec<Field>,
    /// Total decoded message length in bytes.
    pub len: usize,
}

impl Message {
    /// Decode one complete raw protobuf message.
    ///
    /// The decoder accepts only the wire types that carry real protobuf field
    /// payloads: varint, fixed64, length-delimited, and fixed32. Deprecated
    /// groups are rejected as unsupported wire types.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut cursor = 0;
        let mut fields = Vec::new();

        while cursor < bytes.len() {
            fields.push(Field::decode(bytes, &mut cursor)?);
        }

        Ok(Self {
            fields,
            len: bytes.len(),
        })
    }
}

/// One decoded protobuf field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Schema field number encoded in the tag.
    pub number: u32,
    /// Wire type encoded in the tag.
    pub wire_type: WireType,
    /// Offset where the tag varint begins.
    pub tag_offset: usize,
    /// Offset immediately after the tag varint.
    pub value_offset: usize,
    /// Offset immediately after the field value.
    pub end_offset: usize,
    /// Decoded raw value.
    pub value: Value,
}

impl Field {
    fn decode(bytes: &[u8], cursor: &mut usize) -> Result<Self, Error> {
        let tag_offset = *cursor;
        let tag = read_varint(bytes, cursor)?;
        if tag == 0 {
            return Err(Error::InvalidWire {
                reason: "field tag cannot be zero",
                offset: tag_offset,
            });
        }

        let number = u32::try_from(tag >> 3).map_err(|_| Error::InvalidWire {
            reason: "field number overflow",
            offset: tag_offset,
        })?;
        let wire_type = WireType::try_from(tag & 0x07, tag_offset)?;
        let value_offset = *cursor;
        let value = match wire_type {
            WireType::Varint => Value::Varint(read_varint(bytes, cursor)?),
            WireType::Fixed64 => Value::Fixed64(read_fixed64(bytes, cursor)?),
            WireType::LengthDelimited => {
                let len_offset = *cursor;
                let len = usize::try_from(read_varint(bytes, cursor)?).map_err(|_| {
                    Error::InvalidWire {
                        reason: "length-delimited field length overflow",
                        offset: len_offset,
                    }
                })?;
                let end = cursor.checked_add(len).ok_or(Error::InvalidWire {
                    reason: "length-delimited field length overflow",
                    offset: len_offset,
                })?;
                let value = bytes.get(*cursor..end).ok_or(Error::Truncated {
                    context: "length-delimited field",
                    offset: *cursor,
                })?;
                *cursor = end;
                Value::LengthDelimited(value.to_vec())
            }
            WireType::Fixed32 => Value::Fixed32(read_fixed32(bytes, cursor)?),
        };

        Ok(Self {
            number,
            wire_type,
            tag_offset,
            value_offset,
            end_offset: *cursor,
            value,
        })
    }
}

/// Supported protobuf wire types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireType {
    /// Wire type 0.
    Varint,
    /// Wire type 1.
    Fixed64,
    /// Wire type 2.
    LengthDelimited,
    /// Wire type 5.
    Fixed32,
}

impl WireType {
    fn try_from(value: u64, offset: usize) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Varint),
            1 => Ok(Self::Fixed64),
            2 => Ok(Self::LengthDelimited),
            5 => Ok(Self::Fixed32),
            _ => Err(Error::InvalidWire {
                reason: "unsupported wire type",
                offset,
            }),
        }
    }

    /// Human-readable wire type name used in dumps and summaries.
    pub fn name(self) -> &'static str {
        match self {
            Self::Varint => "varint",
            Self::Fixed64 => "fixed64",
            Self::LengthDelimited => "length-delimited",
            Self::Fixed32 => "fixed32",
        }
    }

    /// Conservative scalar type used when emitting a draft `.proto`.
    pub fn proto_scalar(self) -> &'static str {
        match self {
            Self::Varint => "uint64",
            Self::Fixed64 => "fixed64",
            Self::LengthDelimited => "bytes",
            Self::Fixed32 => "fixed32",
        }
    }
}

/// Decoded raw protobuf field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Wire type 0 value.
    Varint(u64),
    /// Wire type 1 value, little-endian on the wire.
    Fixed64(u64),
    /// Wire type 2 payload bytes, without the length prefix.
    LengthDelimited(Vec<u8>),
    /// Wire type 5 value, little-endian on the wire.
    Fixed32(u32),
}

/// Read a protobuf varint from `bytes`, advancing `cursor`.
pub fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let start = *cursor;
    let mut shift = 0u32;
    let mut value = 0u64;

    loop {
        if shift >= 64 {
            return Err(Error::InvalidWire {
                reason: "varint overflow",
                offset: start,
            });
        }

        let byte = *bytes.get(*cursor).ok_or(Error::Truncated {
            context: "varint",
            offset: *cursor,
        })?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }

        shift += 7;
    }
}

fn read_fixed32(bytes: &[u8], cursor: &mut usize) -> Result<u32, Error> {
    let end = cursor.checked_add(4).ok_or(Error::InvalidWire {
        reason: "fixed32 overflow",
        offset: *cursor,
    })?;
    let slice = bytes.get(*cursor..end).ok_or(Error::Truncated {
        context: "fixed32",
        offset: *cursor,
    })?;
    *cursor = end;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_fixed64(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let end = cursor.checked_add(8).ok_or(Error::InvalidWire {
        reason: "fixed64 overflow",
        offset: *cursor,
    })?;
    let slice = bytes.get(*cursor..end).ok_or(Error::Truncated {
        context: "fixed64",
        offset: *cursor,
    })?;
    *cursor = end;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

/// Append `value` as a protobuf varint.
///
/// This is public mostly for tests and small fixture builders.
pub fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}
