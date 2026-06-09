# Public protobuf wire fixtures

These fixtures are raw protobuf wire streams represented as lowercase hex.
They are intentionally independent of the `iwork` examples.

Source: the official Protocol Buffers encoding guide:

<https://protobuf.dev/programming-guides/encoding/>

Fixtures:

- `test1_varint_150.pb.hex`
  - schema: `message Test1 { int32 a = 1; }`
  - value: `a = 150`
  - documented bytes: `08 96 01`
- `test2_string_testing.pb.hex`
  - schema: `message Test2 { string b = 2; }`
  - value: `b = "testing"`
  - documented bytes: `12 07 74 65 73 74 69 6e 67`
- `test3_submessage_150.pb.hex`
  - schema: `message Test3 { Test1 c = 3; }`
  - value: `c.a = 150`
  - documented bytes: `1a 03 08 96 01`
- `test4_packed_repeated.pb.hex`
  - schema:
    `message Test4 { string d = 4; repeated int32 e = 5; }`
  - value: `d = "hello"; e = [1, 2, 3]`
  - documented bytes: `22 05 68 65 6c 6c 6f 2a 03 01 02 03`
- `test4_mixed_order_equivalent.pb.hex`
  - schema:
    `message Test4 { string d = 4; repeated int32 e = 5; }`
  - value: same logical field values, accepted as multiple packed segments
  - documented Protoscope:
    `5: {1 2}` / `4: {"hello"}` / `5: {3}`
  - equivalent bytes: `2a 02 01 02 22 05 68 65 6c 6c 6f 2a 01 03`

The fixtures encode structure only. `protorev` should not infer the exact
semantic scalar type (`int32` vs `uint64`, for example) from the bytes alone.
