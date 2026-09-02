use search::{encode_int, find_all, find_before, find_from, parse_hex, text_bytes, text_utf16le};

#[test]
fn encode_int_unsigned_endianness() {
    assert_eq!(encode_int("42", 4, false).unwrap(), vec![42, 0, 0, 0]);
    assert_eq!(encode_int("42", 4, true).unwrap(), vec![0, 0, 0, 42]);
    assert_eq!(encode_int("255", 1, false).unwrap(), vec![0xff]);
    assert_eq!(encode_int("0x1234", 2, true).unwrap(), vec![0x12, 0x34]);
    assert_eq!(encode_int("0x1234", 2, false).unwrap(), vec![0x34, 0x12]);
}

#[test]
fn encode_int_signed_two_complement() {
    assert_eq!(encode_int("-1", 2, false).unwrap(), vec![0xff, 0xff]);
    assert_eq!(encode_int("-128", 1, false).unwrap(), vec![0x80]);
    assert_eq!(encode_int("-1", 8, true).unwrap(), vec![0xff; 8]);
}

#[test]
fn encode_int_rejects_overflow_and_junk() {
    assert!(encode_int("256", 1, false).is_err()); // > u8::MAX
    assert!(encode_int("-129", 1, false).is_err()); // < i8::MIN
    assert!(encode_int("nope", 4, false).is_err());
    assert!(encode_int("42", 3, false).is_err()); // bad width
}

#[test]
fn find_all_multiple_and_overlapping() {
    assert_eq!(find_all(b"banana", b"a", 10), vec![1, 3, 5]);
    assert_eq!(find_all(b"aaaa", b"aa", 10), vec![0, 1, 2]); // overlapping
}

#[test]
fn find_all_respects_limit() {
    assert_eq!(find_all(b"aaaa", b"a", 2), vec![0, 1]);
}

#[test]
fn find_all_no_match_and_edge_cases() {
    assert!(find_all(b"abc", b"z", 10).is_empty());
    assert!(find_all(b"abc", b"", 10).is_empty()); // empty needle
    assert!(find_all(b"a", b"abc", 10).is_empty()); // needle longer than hay
}

#[test]
fn find_from_starts_at_offset() {
    let hay = b"the quick brown fox";
    assert_eq!(find_from(hay, b"o", 0), Some(12));
    assert_eq!(find_from(hay, b"o", 13), Some(17));
    assert_eq!(find_from(hay, b"o", 18), None);
}

#[test]
fn find_before_walks_backwards() {
    let hay = b"banana";
    assert_eq!(find_before(hay, b"a", 6), Some(5));
    assert_eq!(find_before(hay, b"a", 5), Some(3));
    assert_eq!(find_before(hay, b"a", 1), None); // nothing starts before index 1 except index 0 ('b')
}

#[test]
fn parse_hex_spaced_and_compact() {
    assert_eq!(parse_hex("48 65 6C").unwrap(), vec![0x48, 0x65, 0x6C]);
    assert_eq!(parse_hex("48656c").unwrap(), vec![0x48, 0x65, 0x6C]);
    assert_eq!(parse_hex("89\t50\n4e 47").unwrap(), vec![0x89, 0x50, 0x4e, 0x47]); // mixed whitespace ok
}

#[test]
fn parse_hex_errors() {
    assert!(parse_hex("").is_err());
    assert!(parse_hex("abc").is_err()); // odd length
    assert!(parse_hex("gg").is_err()); // non-hex
}

#[test]
fn text_encoders() {
    assert_eq!(text_bytes("Hi"), vec![b'H', b'i']);
    assert_eq!(text_utf16le("Hi"), vec![b'H', 0, b'i', 0]);
}

#[test]
fn search_finds_encoded_text_in_bytes() {
    let mut hay = vec![0u8, 1, 2];
    hay.extend_from_slice(b"NOPEEK");
    let hits = find_all(&hay, &text_bytes("NOPEEK"), 10);
    assert_eq!(hits, vec![3]);
}
