use binary_reader::{BinaryReader, Encoding, ReadError};

// Sample bytes reused across tests:
//   offset 0: 00 01 00 00 00 00 00 2A  48 65 6C 6C 6F
// This mirrors the example in the product plan.
fn sample() -> BinaryReader {
    BinaryReader::from_bytes(vec![
        0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, // version u16=?, then bytes
        0x48, 0x65, 0x6C, 0x6C, 0x6F, // "Hello"
    ])
}

#[test]
fn len_and_empty() {
    assert_eq!(sample().len(), 13);
    assert!(BinaryReader::from_bytes(vec![]).is_empty());
}

#[test]
fn cursor_advances_on_read() {
    let mut r = sample();
    assert_eq!(r.position(), 0);
    let _ = r.read_u8().unwrap();
    assert_eq!(r.position(), 1);
    let _ = r.read_u16_le().unwrap();
    assert_eq!(r.position(), 3);
}

#[test]
fn endianness_u16() {
    let r = BinaryReader::from_bytes(vec![0x01, 0x00]);
    assert_eq!(r.read_u16_le_at(0).unwrap(), 1);
    assert_eq!(r.read_u16_be_at(0).unwrap(), 256);
}

#[test]
fn endianness_u32_matches_plan_example() {
    // Plan section 11: bytes 00 00 00 2A -> uint32 BE = 42, uint32 LE = 704643072
    let r = BinaryReader::from_bytes(vec![0x00, 0x00, 0x00, 0x2A]);
    assert_eq!(r.read_u32_be_at(0).unwrap(), 42);
    assert_eq!(r.read_u32_le_at(0).unwrap(), 704643072);
}

#[test]
fn signed_integers() {
    let r = BinaryReader::from_bytes(vec![0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(r.read_i8_at(0).unwrap(), -1);
    assert_eq!(r.read_i16_le_at(0).unwrap(), -1);
    assert_eq!(r.read_i32_le_at(0).unwrap(), -1);
}

#[test]
fn floats() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1.5f32.to_le_bytes());
    buf.extend_from_slice(&(-2.25f64).to_be_bytes());
    let r = BinaryReader::from_bytes(buf);
    assert_eq!(r.read_f32_le_at(0).unwrap(), 1.5);
    assert_eq!(r.read_f64_be_at(4).unwrap(), -2.25);
}

#[test]
fn u64_roundtrip() {
    let v: u64 = 0x0102_0304_0506_0708;
    let r = BinaryReader::from_bytes(v.to_le_bytes().to_vec());
    assert_eq!(r.read_u64_le_at(0).unwrap(), v);
    let r = BinaryReader::from_bytes(v.to_be_bytes().to_vec());
    assert_eq!(r.read_u64_be_at(0).unwrap(), v);
}

#[test]
fn read_bytes_and_ascii_string() {
    let mut r = sample();
    r.seek(8).unwrap();
    let s = r.read_string(5, Encoding::Ascii).unwrap();
    assert_eq!(s, "Hello");
    assert_eq!(r.position(), 13);
}

#[test]
fn utf16_string() {
    // "Hi" in UTF-16LE
    let bytes = vec![0x48, 0x00, 0x69, 0x00];
    let r = BinaryReader::from_bytes(bytes);
    assert_eq!(r.read_string_at(0, 4, Encoding::Utf16Le).unwrap(), "Hi");
}

#[test]
fn non_ascii_bytes_rejected_as_ascii() {
    let r = BinaryReader::from_bytes(vec![0xFF, 0x00]);
    assert!(matches!(
        r.read_string_at(0, 2, Encoding::Ascii),
        Err(ReadError::InvalidText { .. })
    ));
}

#[test]
fn out_of_bounds_is_error_not_panic() {
    let r = sample();
    assert!(matches!(
        r.read_u32_le_at(11), // only 2 bytes remain
        Err(ReadError::OutOfBounds { .. })
    ));
    assert!(matches!(
        r.read_bytes_at(100, 1),
        Err(ReadError::OutOfBounds { .. })
    ));
}

#[test]
fn seek_to_eof_ok_but_past_eof_errors() {
    let mut r = sample();
    assert!(r.seek(13).is_ok()); // exactly len() is allowed
    assert!(r.seek(14).is_err());
}

#[test]
fn overflow_offset_len_does_not_panic() {
    let r = sample();
    assert!(r.read_bytes_at(usize::MAX, 8).is_err());
}

#[test]
fn open_real_file_via_mmap() {
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
    f.flush().unwrap();
    let r = BinaryReader::open(f.path()).unwrap();
    assert_eq!(r.len(), 4);
    assert_eq!(r.read_u32_be_at(0).unwrap(), 0xDEAD_BEEF);
}

#[test]
fn open_empty_file() {
    let f = tempfile::NamedTempFile::new().unwrap();
    let r = BinaryReader::open(f.path()).unwrap();
    assert!(r.is_empty());
}
