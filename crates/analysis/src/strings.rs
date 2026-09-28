//! Scan bytes for runs of readable text.

use std::collections::HashSet;

use serde::Serialize;

/// How a detected string was encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Ascii,
    Utf16Le,
    Utf16Be,
}

/// A run of readable text found in the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StringHit {
    /// Byte offset where the run starts.
    pub offset: usize,
    /// Number of bytes the run occupies (2× the char count for UTF-16).
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
/// ASCII, UTF-16LE (the Windows encoding) and UTF-16BE (what Java, and most
/// formats that write UTF-16 over a wire, use) are all reported: a UTF-16 run is
/// `printable, 0x00` pairs in one order or `0x00, printable` in the other. The
/// ASCII pass is independent of the other two, so a run may appear twice.
pub fn find_strings(bytes: &[u8], min_len: usize) -> Vec<StringHit> {
    let min_len = min_len.max(1);
    let mut hits = scan_ascii(bytes, min_len);
    hits.extend(scan_utf16(bytes, min_len, Encoding::Utf16Le));
    hits.extend(scan_utf16(bytes, min_len, Encoding::Utf16Be));
    drop_shadowed(&mut hits);
    hits.sort_by_key(|h| h.offset);
    hits
}

/// Drop the phantom each UTF-16 run leaves in the opposite byte order.
///
/// `41 00 42 00 43 00` is the UTF-16LE string "ABC"; read from one byte later,
/// the same bytes are the UTF-16BE string "BC". Every UTF-16 string does this,
/// so without pruning, adding the second order would have doubled the list
/// rather than found anything new. The true reading is the one that starts first
/// and covers two more bytes, which is exactly the shadow's signature.
fn drop_shadowed(hits: &mut Vec<StringHit>) {
    let spans: HashSet<(usize, usize)> = hits
        .iter()
        .filter(|h| h.encoding != Encoding::Ascii)
        .map(|h| (h.offset, h.len))
        .collect();
    hits.retain(|h| {
        h.encoding == Encoding::Ascii
            || h.offset == 0
            || !spans.contains(&(h.offset - 1, h.len + 2))
    });
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

/// Scan for UTF-16 runs in one byte order. The orders differ only in which half
/// of each pair carries the character, so `lead` picks the byte to read.
fn scan_utf16(bytes: &[u8], min_len: usize, encoding: Encoding) -> Vec<StringHit> {
    let lead = usize::from(encoding == Encoding::Utf16Be);
    let mut hits = Vec::new();
    let mut start = 0usize;
    let mut chars = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let printable_pair = is_printable(bytes[i + lead]) && bytes[i + 1 - lead] == 0x00;
        if printable_pair {
            if chars == 0 {
                start = i;
            }
            chars += 1;
            i += 2;
        } else {
            flush_utf16(bytes, start, chars, min_len, encoding, &mut hits);
            chars = 0;
            i += 1;
        }
    }
    flush_utf16(bytes, start, chars, min_len, encoding, &mut hits);
    hits
}

fn flush_utf16(
    bytes: &[u8],
    start: usize,
    chars: usize,
    min_len: usize,
    encoding: Encoding,
    hits: &mut Vec<StringHit>,
) {
    if chars >= min_len {
        let lead = usize::from(encoding == Encoding::Utf16Be);
        let text = (0..chars).map(|k| bytes[start + k * 2 + lead] as char).collect();
        hits.push(StringHit {
            offset: start,
            len: chars * 2,
            encoding,
            text,
        });
    }
}
