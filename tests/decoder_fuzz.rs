use protorev::Message;

#[test]
fn decoder_handles_deterministic_byte_corpus_without_panicking() {
    for len in 0..=96 {
        for case in 0..128 {
            let bytes = generated_bytes(len, case);
            assert_decoding_is_bounded(&bytes);
        }
    }

    for first in 0u8..=u8::MAX {
        assert_decoding_is_bounded(&[first]);
        for second in [0x00, 0x01, 0x7f, 0x80, 0xff] {
            assert_decoding_is_bounded(&[first, second]);
        }
    }
}

fn generated_bytes(len: usize, case: u64) -> Vec<u8> {
    let mut state = case
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(u64::try_from(len).unwrap_or(0));
    let mut bytes = Vec::with_capacity(len);
    for index in 0..len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let mixed = state
            .wrapping_mul(0x2545_f491_4f6c_dd1d)
            .wrapping_add(u64::try_from(index).unwrap_or(0));
        bytes.push(u8::try_from(mixed & 0xff).unwrap_or(0));
    }
    bytes
}

fn assert_decoding_is_bounded(bytes: &[u8]) {
    let Ok(message) = Message::decode(bytes) else {
        return;
    };

    assert_eq!(message.len, bytes.len());
    let mut previous_end = 0;
    for field in message.fields {
        assert!(field.tag_offset >= previous_end);
        assert!(field.value_offset >= field.tag_offset);
        assert!(field.end_offset >= field.value_offset);
        assert!(field.end_offset <= bytes.len());
        previous_end = field.end_offset;
    }
}
