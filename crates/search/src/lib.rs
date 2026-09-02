//! Byte-pattern search over a buffer (plan §15).
//!
//! The core is a plain substring scan over `&[u8]` — fast enough for the
//! memory-mapped files the app deals with, and simple to reason about. On top
//! sit small builders that turn a user's query (a hex string, ASCII text, or
//! UTF-16LE text) into the needle bytes to search for.
//!
//! ```
//! let hay = b"the quick brown fox";
//! assert_eq!(search::find_all(hay, b"o", 10), vec![12, 17]);
//! assert_eq!(search::parse_hex("48 65").unwrap(), vec![0x48, 0x65]);
//! ```

/// All match offsets of `needle` in `haystack`, up to `limit` results.
/// Matches may overlap. An empty needle yields no matches.
pub fn find_all(haystack: &[u8], needle: &[u8], limit: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > haystack.len() {
        return out;
    }
    let mut start = 0usize;
    while start + needle.len() <= haystack.len() {
        match find_from(haystack, needle, start) {
            Some(pos) => {
                out.push(pos);
                if out.len() >= limit {
                    break;
                }
                start = pos + 1; // allow overlapping matches
            }
            None => break,
        }
    }
    out
}

/// The first match at an offset `>= start`, if any.
pub fn find_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || start > haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + start)
}

/// The last match that *starts* strictly before `before`, if any — for
/// "find previous".
pub fn find_before(haystack: &[u8], needle: &[u8], before: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    // Last valid start index to consider.
    let last = before.min(haystack.len() - needle.len() + 1);
    (0..last).rev().find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Parse a hex string like `"48 65 6C"` or `"48656c"` into bytes. Whitespace is
/// ignored; an odd number of hex digits or any non-hex character is an error.
pub fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err("empty hex".into());
    }
    if compact.len() % 2 != 0 {
        return Err("hex needs an even number of digits".into());
    }
    let mut out = Vec::with_capacity(compact.len() / 2);
    let bytes = compact.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i]).ok_or_else(|| format!("not a hex digit: '{}'", bytes[i] as char))?;
        let lo = hex_val(bytes[i + 1]).ok_or_else(|| format!("not a hex digit: '{}'", bytes[i + 1] as char))?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Encode text as ASCII/UTF-8 bytes (the natural byte form of the string).
pub fn text_bytes(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// Encode text as UTF-16LE bytes (common in Windows binaries).
pub fn text_utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// Encode an integer query as the `width`-byte needle to search for — this is
/// typed value search ("find all u32 == 42"). The query is decimal or `0x`
/// hex, with an optional leading `-`; a non-negative value is treated as
/// unsigned and a negative one as two's-complement. `width` must be 1, 2, 4,
/// or 8 bytes. Errors if the value doesn't fit the width.
///
/// ```
/// assert_eq!(search::encode_int("42", 4, false).unwrap(), vec![42, 0, 0, 0]);
/// assert_eq!(search::encode_int("42", 4, true).unwrap(), vec![0, 0, 0, 42]);
/// assert_eq!(search::encode_int("-1", 2, false).unwrap(), vec![0xff, 0xff]);
/// ```
pub fn encode_int(query: &str, width: usize, big_endian: bool) -> Result<Vec<u8>, String> {
    if !matches!(width, 1 | 2 | 4 | 8) {
        return Err("width must be 1, 2, 4, or 8 bytes".into());
    }
    let s = query.trim();
    if s.is_empty() {
        return Err("empty value".into());
    }
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest.trim()),
        None => (false, s),
    };
    let magnitude: u64 = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).map_err(|_| format!("not a valid hex integer: {s}"))?,
        None => digits.parse::<u64>().map_err(|_| format!("not a valid integer: {s}"))?,
    };
    let bits = width * 8;
    let pattern: u64 = if neg {
        // Two's complement; the magnitude may be as large as 2^(bits-1) (i*::MIN).
        let bound: u64 = 1u64 << (bits - 1);
        if magnitude > bound {
            return Err(format!("{s} doesn't fit in a signed {bits}-bit value"));
        }
        // `1 << 64` would overflow, so handle the full-width case separately.
        if width == 8 {
            0u64.wrapping_sub(magnitude)
        } else {
            (1u64 << bits) - magnitude
        }
    } else {
        if width < 8 && magnitude > (1u64 << bits) - 1 {
            return Err(format!("{s} doesn't fit in an unsigned {bits}-bit value"));
        }
        magnitude
    };
    let mut bytes = pattern.to_le_bytes()[..width].to_vec();
    if big_endian {
        bytes.reverse();
    }
    Ok(bytes)
}
