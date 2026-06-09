use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub fields: Vec<Field>,
    pub len: usize,
}

impl Message {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub number: u32,
    pub wire_type: WireType,
    pub tag_offset: usize,
    pub value_offset: usize,
    pub end_offset: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireType {
    Varint,
    Fixed64,
    LengthDelimited,
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

    pub fn name(self) -> &'static str {
        match self {
            Self::Varint => "varint",
            Self::Fixed64 => "fixed64",
            Self::LengthDelimited => "length-delimited",
            Self::Fixed32 => "fixed32",
        }
    }

    pub fn proto_scalar(self) -> &'static str {
        match self {
            Self::Varint => "uint64",
            Self::Fixed64 => "fixed64",
            Self::LengthDelimited => "bytes",
            Self::Fixed32 => "fixed32",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Varint(u64),
    Fixed64(u64),
    LengthDelimited(Vec<u8>),
    Fixed32(u32),
}

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
