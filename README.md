# protorev

`protorev` is a small protobuf reverse-engineering workbench.

It is intentionally separate from the parent `iwork` crate: it has its own
library, binary, tests, and build target. The parent repository only exposes it
as a Cargo workspace member.

## Commands

```bash
cargo run -p protorev -- dump sample.pb
cargo run -p protorev -- infer samples/*.pb
cargo run -p protorev -- diff before.pb after.pb
```

## Scope

The first version handles raw protobuf wire streams:

- strict decoding for wire types 0, 1, 2, and 5
- byte offsets for every decoded field
- recursive inspection of length-delimited values that parse cleanly as messages
- UTF-8 and packed-varint hints for length-delimited values
- corpus aggregation with draft `.proto` output

Adapters for framed formats, such as iWork IWA/Snappy archives, should live
behind this crate boundary rather than inside the existing `iwork` parser.
