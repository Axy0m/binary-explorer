//! Tests for the analysis heuristics: string scanning and offset guesses.

use analysis::{analyze_at, entropy, find_strings, Encoding};

#[test]
fn finds_ascii_string_among_binary() {
    let mut bytes = vec![0x00, 0x01, 0xFF];
    bytes.extend_from_slice(b"Player");
    bytes.extend_from_slice(&[0x00, 0x00]);
    let hits = find_strings(&bytes, 4);
    let ascii: Vec<_> = hits.iter().filter(|h| h.encoding == Encoding::Ascii).collect();
    assert_eq!(ascii.len(), 1);
    assert_eq!(ascii[0].text, "Player");
    assert_eq!(ascii[0].offset, 3);
    assert_eq!(ascii[0].len, 6);
}

#[test]
fn respects_min_length() {
    // "Hi" is length 2; with min_len 4 it should be ignored.
    let bytes = b"\x00Hi\x00wxyz\x00".to_vec();
    let hits = find_strings(&bytes, 4);
    let texts: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
    assert!(!texts.contains(&"Hi"));
    assert!(texts.contains(&"wxyz"));
}

#[test]
fn finds_utf16le_string() {
    // "OK" as UTF-16LE: 'O' 00 'K' 00
    let bytes = vec![b'O', 0, b'K', 0, b'!', 0];
    let hits = find_strings(&bytes, 2);
    let utf16: Vec<_> = hits.iter().filter(|h| h.encoding == Encoding::Utf16Le).collect();
    assert_eq!(utf16.len(), 1);
    assert_eq!(utf16[0].text, "OK!");
    assert_eq!(utf16[0].len, 6);
}

#[test]
fn no_strings_in_pure_binary() {
    let bytes = [0x00, 0x01, 0x02, 0xFE, 0xFF];
    assert!(find_strings(&bytes, 4).is_empty());
}

#[test]
fn guess_flags_a_string_start() {
    let bytes = b"Majd\x00\x00".to_vec();
    let guesses = analyze_at(&bytes, 0);
    assert!(guesses.iter().any(|g| g.label.contains("ASCII string") && g.detail.contains("Majd")));
}

#[test]
fn guess_flags_a_unix_timestamp() {
    // 1_700_000_000 = 2023-11-14 22:13:20 UTC, little-endian u32.
    let secs: u32 = 1_700_000_000;
    let bytes = secs.to_le_bytes().to_vec();
    let guesses = analyze_at(&bytes, 0);
    let ts = guesses.iter().find(|g| g.label.contains("Unix timestamp"));
    assert!(ts.is_some(), "expected a timestamp guess, got {guesses:?}");
    assert!(ts.unwrap().detail.starts_with("2023-11-14"), "got {:?}", ts.unwrap().detail);
}

#[test]
fn small_integers_are_not_called_timestamps() {
    let bytes = 42u32.to_le_bytes().to_vec();
    let guesses = analyze_at(&bytes, 0);
    assert!(guesses.iter().all(|g| !g.label.contains("Unix timestamp")));
}

#[test]
fn guess_flags_a_uuid() {
    // A v4 UUID: version nibble (byte 6 high) = 4.
    let bytes = [
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0x4d, 0xef, 0x81, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];
    let guesses = analyze_at(&bytes, 0);
    let uuid = guesses.iter().find(|g| g.label.starts_with("UUID"));
    assert!(uuid.is_some(), "expected a UUID guess, got {guesses:?}");
    assert_eq!(uuid.unwrap().detail, "12345678-9abc-4def-8123-456789abcdef");
}

#[test]
fn entropy_of_repeated_byte_is_zero() {
    let flat = vec![0x41u8; 1000];
    let e = entropy(&flat, 4);
    assert_eq!(e.len(), 4);
    assert!(e.iter().all(|&v| v < 0.01), "flat data should be ~0 entropy: {e:?}");
}

#[test]
fn entropy_of_uniform_bytes_is_near_one() {
    // Every byte value present equally -> maximum entropy.
    let uniform: Vec<u8> = (0..=255u8).cycle().take(256 * 8).collect();
    let e = entropy(&uniform, 1);
    assert!(e[0] > 0.99, "uniform data should be ~1.0 entropy: {e:?}");
}

#[test]
fn entropy_bucket_count_and_edges() {
    assert!(entropy(&[], 8).is_empty());
    assert!(entropy(&[1, 2, 3], 0).is_empty());
    // More buckets than bytes -> capped at byte count.
    assert_eq!(entropy(&[1, 2, 3], 10).len(), 3);
}

#[test]
fn analyze_past_end_is_empty_not_panic() {
    let bytes = [0u8; 2];
    assert!(analyze_at(&bytes, 100).is_empty());
}
