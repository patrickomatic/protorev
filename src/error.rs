//! Error types for `protorev`.

use std::fmt::{Display, Formatter};

/// Errors produced by protobuf decoding and CLI file IO.
#[derive(Debug)]
pub enum Error {
    /// Filesystem error.
    Io(std::io::Error),
    /// Human-readable command or validation error.
    Message(String),
    /// The input ended before the current wire value was complete.
    Truncated {
        /// Value being decoded.
        context: &'static str,
        /// Byte offset where the decoder needed more input.
        offset: usize,
    },
    /// The input was not valid for the supported protobuf wire subset.
    InvalidWire {
        /// Static explanation of the invalid condition.
        reason: &'static str,
        /// Byte offset where the invalid condition was detected.
        offset: usize,
    },
}

impl Error {
    /// Create a plain message error.
    pub fn message(value: impl Into<String>) -> Self {
        Self::Message(value.into())
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Message(message) => formatter.write_str(message),
            Self::Truncated { context, offset } => {
                write!(formatter, "truncated {context} at offset {offset}")
            }
            Self::InvalidWire { reason, offset } => {
                write!(
                    formatter,
                    "invalid protobuf wire stream at offset {offset}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Message(_) | Self::Truncated { .. } | Self::InvalidWire { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
