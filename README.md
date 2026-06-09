# protorev

`protorev` is a small protobuf reverse-engineering workbench.

It is intentionally separate from the parent `iwork` crate: it has its own
library, binary, tests, and build target. The parent repository only exposes it
as a Cargo workspace member.

The goal is not to replace `protoc --decode_raw`. The goal is to keep the
evidence you need while reverse engineering unknown protobuf streams: offsets,
wire types, recursive length-delimited candidates, corpus-level field presence,
and a draft schema that stays honest about what was observed.

## Commands

```bash
cargo run -p protorev -- dump sample.pb
cargo run -p protorev -- infer samples/*.pb
cargo run -p protorev -- schema samples/*.pb
cargo run -p protorev -- diff before.pb after.pb
```

### `dump`

`dump` decodes one raw protobuf message and prints each field with byte offsets:

```text
@0..3 field 1 varint = 150
@3..10 field 2 length-delimited len=5 [utf8] bytes=74 69 74 6c 65
  text "title"
```

For length-delimited fields, `protorev` reports any candidates that match:

- `message`: the payload decodes cleanly as a nested protobuf message
- `utf8`: the payload is valid UTF-8 text without non-text control characters
- `packed-varint`: the payload decodes cleanly as two or more varints

These are candidates, not schema facts. A short byte string can legitimately
look like more than one thing.

### `infer`

`infer` reads multiple samples, aggregates field presence, marks fields as
`repeated` when a field number appears more than once in a sample, recursively
tracks nested message candidates, and emits a draft `.proto`:

```text
samples: 2

root:
  field 1: observed 2/2 samples; wires: length-delimited; max/sample: 1

--- draft proto ---

syntax = "proto3";

message Message {
  Message_1 field_1 = 1; // observed 2/2 samples; wires: length-delimited
}
```

### `diff`

`diff` is a small convenience around `infer` for two files. It gives a compact
shape comparison for controlled before/after samples.

### `schema`

`schema` emits a confidence-gated structural `.proto`. By default it includes
only high-confidence fields:

```bash
cargo run -p protorev -- schema samples/*.pb
```

High confidence currently means:

- at least two relevant samples were observed
- the field used one stable wire type
- the field appeared in every relevant sample
- if emitted as a nested message, every observed occurrence decoded cleanly as
  that nested message shape

For exploratory output, lower the threshold:

```bash
cargo run -p protorev -- schema --min-confidence medium samples/*.pb
```

The emitted schema is intentionally structural:

```proto
syntax = "proto3";

message Message {
  Message_1 field_1 = 1; // confidence: high; observed 2/2 samples; wires: length-delimited; occurrences: 2; nested: 2/2; utf8: 0/2; packed-varint: 2/2
}
```

`schema` still does not invent semantic names or scalar intent. A stable varint
is `uint64`; a stable length-delimited field is either a consistently observed
nested message or `bytes`.

## Scope

The first version handles raw protobuf wire streams:

- strict decoding for wire types 0, 1, 2, and 5
- byte offsets for every decoded field
- recursive inspection of length-delimited values that parse cleanly as messages
- UTF-8 and packed-varint hints for length-delimited values
- corpus aggregation with draft `.proto` output

It intentionally does not infer semantic scalar types yet. Varints are emitted
as `uint64`, fixed-width values as `fixed32`/`fixed64`, and opaque
length-delimited fields as `bytes`.

## Library API

The binary is a thin wrapper around the library:

```rust
use protorev::{Corpus, Message, dump_message};

fn main() -> Result<(), protorev::Error> {
    let message = Message::decode(&[0x08, 0x96, 0x01])?;
    println!("{}", dump_message(&message, 4));

    let corpus = Corpus::from_messages(&[message], 4);
    println!("{}", corpus.draft_proto());
    println!("{}", corpus.schema(&Default::default()));
    Ok(())
}
```

Use `Message::decode` when you already have raw protobuf bytes. Format-specific
adapters should decode their container/framing first, then pass the raw protobuf
payload into this crate.

## Reverse-Engineering Discipline

`protorev` reports evidence. It does not claim that:

- a clean nested-message parse proves a field is definitely a message
- printable bytes prove a field is semantically a string
- a varint's numeric value explains what the field means

Promote a hypothesis only after it survives comparison across controlled
samples. That is the line between structural knowledge and content coincidence.

Adapters for framed formats, such as iWork IWA/Snappy archives, should live
behind this crate boundary rather than inside the existing `iwork` parser.
