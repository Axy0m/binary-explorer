//! Scan bytes for runs of readable text.

use serde::Serialize;

/// How a detected string was encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Ascii,
    Utf16Le,
}

/// A run of readable text found in the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StringHit {
    /// Byte offset where the run starts.
    pub offset: usize,
    /// Number of bytes the run occupies (2× the char count for UTF-16LE).
    pub len: usize,
    pub encoding: Encoding,
    pub text: String,
}

/// A byte is "printable" for string-scanning if it's a visible ASCII glyph or
/// a plain space. Tabs/newlines are excluded so runs stay on obvious text.
fn is_printable(b: u8) -> bool {
    (0x20..=0x7E).contains(&b)
}

/// If `bytes` begins with at least `min_len` printable ASCII characters, return
/// that leading run as text; otherwise `None`. Used by the offset analyzer to
/// decide whether a string starts right here.
pub(crate) fn is_printable_run(bytes: &[u8], min_len: usize) -> Option<String> {
    let run = bytes.iter().take_while(|&&b| is_printable(b)).count();
    if run >= min_len {
        Some(bytes[..run].iter().map(|&b| b as char).collect())
    } else {
        None
    }
}

/// Find readable strings of at least `min_len` characters.
///
/// Both ASCII and UTF-16LE (little-endian, the common Windows encoding) runs
/// are reported. A UTF-16LE run is recognized as `printable, 0x00` pairs. The
/// two passes are independent, so a run may appear once per encoding.
pub fn find_strings(bytes: &[u8], min_len: usize) -> Vec<StringHit> {
    let min_len = min_len.max(1);
    let mut hits = scan_ascii(bytes, min_len);
    hits.extend(scan_utf16le(bytes, min_len));
    hits.sort_by_key(|h| h.offset);
    hits
}

fn scan_ascii(bytes: &[u8], min_len: usize) -> Vec<StringHit> {
    let mut hits = Vec::new();
    let mut start = 0usize;
    let mut run = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if is_printable(b) {
            if run == 0 {
                start = i;
            }
            run += 1;
        } else {
            flush_ascii(bytes, start, run, min_len, &mut hits);
            run = 0;
        }
    }
    flush_ascii(bytes, start, run, min_len, &mut hits);
    hits
}

fn flush_ascii(bytes: &[u8], start: usize, run: usize, min_len: usize, hits: &mut Vec<StringHit>) {
    if run >= min_len {
        let text = bytes[start..start + run].iter().map(|&b| b as char).collect();
        hits.push(StringHit {
            offset: start,
            len: run,
            encoding: Encoding::Ascii,
            text,
        });
    }
}

fn scan_utf16le(bytes: &[u8], min_len: usize) -> Vec<StringHit> {
    let mut hits = Vec::new();
    let mut start = 0usize;
    let mut chars = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let printable_pair = is_printable(bytes[i]) && bytes[i + 1] == 0x00;
        if printable_pair {
            if chars == 0 {
                start = i;
            }
            chars += 1;
            i += 2;
        } else {
            flush_utf16(bytes, start, chars, min_len, &mut hits);
            chars = 0;
            i += 1;
        }
    }
    flush_utf16(bytes, start, chars, min_len, &mut hits);
    hits
}

fn flush_utf16(bytes: &[u8], start: usize, chars: usize, min_len: usize, hits: &mut Vec<StringHit>) {
    if chars >= min_len {
        let text = (0..chars).map(|k| bytes[start + k * 2] as char).collect();
        hits.push(StringHit {
            offset: start,
            len: chars * 2,
            encoding: Encoding::Utf16Le,
            text,
        });
    }
}
