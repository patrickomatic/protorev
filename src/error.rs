use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Message(String),
    Truncated {
        context: &'static str,
        offset: usize,
    },
    InvalidWire {
        reason: &'static str,
        offset: usize,
    },
}

impl Error {
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
