//! A protobuf reverse-engineering workbench.
//!
//! `protorev` decodes raw protobuf wire streams when no `.proto` schema is
//! available. It keeps byte offsets, reports wire types, classifies
//! length-delimited fields as possible nested messages, UTF-8 strings, or
//! packed varints, and aggregates observations across a sample corpus.
//!
//! The crate reports evidence rather than truth. For example, a
//! length-delimited field that decodes cleanly as a nested message is a strong
//! candidate, but not a proof that the producer's schema used a message type.
//!
//! ```
//! use protorev::{Corpus, Message, Value, dump_message};
//!
//! let message = Message::decode(&[0x08, 0x96, 0x01])?;
//! assert_eq!(message.fields[0].value, Value::Varint(150));
//!
//! let dump = dump_message(&message, 4);
//! assert!(dump.contains("field 1 varint = 150"));
//!
//! let corpus = Corpus::from_messages(&[message], 4);
//! assert!(corpus.draft_proto().contains("uint64 field_1 = 1;"));
//! # Ok::<(), protorev::Error>(())
//! ```

mod classify;
mod dump;
mod error;
mod infer;
pub mod wire;

pub use classify::LengthDelimitedHints;
pub use dump::dump_message;
pub use error::Error;
pub use infer::Corpus;
pub use wire::{Field, Message, Value, WireType};
