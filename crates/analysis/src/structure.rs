//! Guesses about the *shape* of a region of unknown bytes.
//!
//! The other analysers answer "what is the value at this offset?". This one
//! answers the question you actually have when a file opens and nothing is
//! known yet: is there a repeating record here, is this a table of offsets, is
//! this a string pool, is this just padding? Each answer is a starting point
//! for a schema, not a claim — so every hint says what it measured, and the
//! person reading it decides.
//!
//! Everything here is local arithmetic over the bytes. No model, no network:
//! the file never leaves the machine, which is the whole promise of the tool.

use serde::Serialize;

/// One observation about a region, with the span it applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hint {
    /// Short kind, e.g. `"records"`, `"offset table"`.
    pub label: String,
    /// What was measured, in words — the evidence, not just the verdict.
    pub detail: String,
    /// Absolute file offset the hint applies to.
    pub offset: u64,
    pub len: u64,
}

/// Bytes actually examined for periodicity. The scan is O(sample × strides), so
/// this bounds the work on a multi-gigabyte file without changing the answer:
/// a record layout that is not visible in 32 KB is not one this can find.
const SAMPLE: usize = 32 * 1024;

/// Longest record length considered.
const MAX_STRIDE: usize = 256;

/// Look at `bytes` (which start at `base` in the file) and say what shape they
/// appear to have. `total_len` is the file's size, used to judge whether a
/// number could plausibly be an offset into it.
pub fn infer(bytes: &[u8], base: u64, total_len: u64) -> Vec<Hint> {
    let mut hints = Vec::new();
    if bytes.is_empty() {
        return hints;
    }

    if let Some(hint) = constant_fill(bytes, base) {
        // Nothing else is worth saying about a run of one repeated byte.
        hints.push(hint);
        return hints;
    }
    if let Some(hint) = size_prefix(bytes, base, total_len) {
        hints.push(hint);
    }
    if let Some(hint) = offset_table(bytes, base, total_len) {
        hints.push(hint);
    }
    if let Some(hint) = string_pool(bytes, base) {
        hints.push(hint);
    }
    if let Some(hint) = records(bytes, base) {
        hints.push(hint);
    }
    hints
}

/// A region of one repeated byte: padding, an erased flash block, a hole.
fn constant_fill(bytes: &[u8], base: u64) -> Option<Hint> {
    if bytes.len() < 8 {
        return None;
    }
    let first = bytes[0];
    if !bytes.iter().all(|b| *b == first) {
        return None;
    }
    Some(Hint {
        label: "fill".into(),
        detail: format!("{} bytes, all {first:#04x} — padding or erased space", bytes.len()),
        offset: base,
        len: bytes.len() as u64,
    })
}

/// An integer at the start of the region that matches something's length.
///
/// Checked against what follows it rather than against arbitrary numbers, so a
/// match is evidence rather than coincidence — though a small value can still
/// line up by luck, which is why the detail says what it matched.
fn size_prefix(bytes: &[u8], base: u64, total_len: u64) -> Option<Hint> {
    for width in [4usize, 2] {
        if bytes.len() < width {
            continue;
        }
        for big in [false, true] {
            let value = read_uint(&bytes[..width], big);
            let after_in_region = (bytes.len() - width) as u64;
            let after_in_file = total_len.saturating_sub(base + width as u64);
            let order = if big { "big-endian" } else { "little-endian" };
            let what = if value == after_in_region {
                "the bytes that follow it in this selection"
            } else if value == after_in_file {
                "the bytes that follow it in the file"
            } else if value == total_len {
                "the whole file"
            } else {
                continue;
            };
            return Some(Hint {
                label: "size field".into(),
                detail: format!(
                    "the {order} u{} here is {value}, which is exactly {what}",
                    width * 8
                ),
                offset: base,
                len: width as u64,
            });
        }
    }
    None
}

/// A run of 32-bit values that increase and stay inside the file — the shape of
/// a table of offsets into the rest of it.
fn offset_table(bytes: &[u8], base: u64, total_len: u64) -> Option<Hint> {
    const MIN_ENTRIES: usize = 4;
    for big in [false, true] {
        let mut best: Option<(usize, usize)> = None; // (start index, entry count)
        let mut run_start = 0usize;
        let mut run = 0usize;
        let mut previous: Option<u64> = None;

        let count = bytes.len() / 4;
        for i in 0..count {
            let value = read_uint(&bytes[i * 4..i * 4 + 4], big);
            // Plausible as an offset: inside the file, and larger than the one
            // before it. Equal values break the run — a real table of pointers
            // into distinct things climbs.
            let ok = value <= total_len && previous.is_none_or(|p| value > p);
            if ok {
                if run == 0 {
                    run_start = i;
                }
                run += 1;
                previous = Some(value);
            } else {
                if run >= MIN_ENTRIES && best.is_none_or(|(_, n)| run > n) {
                    best = Some((run_start, run));
                }
                // The value that broke the run may still start the next one.
                run = 1;
                run_start = i;
                previous = Some(value);
            }
        }
        if run >= MIN_ENTRIES && best.is_none_or(|(_, n)| run > n) {
            best = Some((run_start, run));
        }

        if let Some((start, n)) = best {
            let order = if big { "big-endian" } else { "little-endian" };
            return Some(Hint {
                label: "offset table".into(),
                detail: format!(
                    "{n} {order} u32 values in a row, each larger than the last and inside the file"
                ),
                offset: base + (start * 4) as u64,
                len: (n * 4) as u64,
            });
        }
    }
    None
}

/// NUL-terminated printable runs packed together — a string pool.
fn string_pool(bytes: &[u8], base: u64) -> Option<Hint> {
    const MIN_STRINGS: usize = 3;
    const MIN_LEN: usize = 4;

    let mut strings = 0usize;
    let mut in_strings = 0usize;
    let mut run = 0usize;
    for b in bytes {
        match b {
            0 if run >= MIN_LEN => {
                strings += 1;
                in_strings += run + 1;
                run = 0;
            }
            0 => run = 0,
            0x20..=0x7E => run += 1,
            _ => run = 0,
        }
    }
    // Most of the region has to actually be strings; a few words inside binary
    // data are the string scanner's job, not a claim about the region's shape.
    if strings >= MIN_STRINGS && in_strings * 2 >= bytes.len() {
        Some(Hint {
            label: "string pool".into(),
            detail: format!(
                "{strings} NUL-terminated strings covering {}% of the region",
                in_strings * 100 / bytes.len()
            ),
            offset: base,
            len: bytes.len() as u64,
        })
    } else {
        None
    }
}

/// A repeating period in the bytes — the signature of fixed-size records.
///
/// Scored by how often a byte equals the one a stride away. Real records repeat
/// their structure (the same tag byte, the same zero padding, the same high
/// bytes of a small integer) even when their contents differ, so the score
/// climbs at the record size and at multiples of it; the smallest stride that
/// scores near the best is the record, the rest are its multiples.
fn records(bytes: &[u8], base: u64) -> Option<Hint> {
    let sample = &bytes[..bytes.len().min(SAMPLE)];
    // Too few distinct values and everything correlates with everything.
    let mut seen = [false; 256];
    let mut distinct = 0;
    for b in sample {
        if !seen[*b as usize] {
            seen[*b as usize] = true;
            distinct += 1;
        }
    }
    if distinct < 4 || sample.len() < 32 {
        return None;
    }

    // How often two bytes picked at random from this region would match anyway.
    // A mostly-zero region does that ~all the time, so without subtracting it a
    // run of padding looks like a record of every size at once.
    let baseline: f32 = seen
        .iter()
        .enumerate()
        .filter(|(_, present)| **present)
        .map(|(value, _)| {
            let p = sample.iter().filter(|b| **b as usize == value).count() as f32
                / sample.len() as f32;
            p * p
        })
        .sum();

    let max_stride = MAX_STRIDE.min(sample.len() / 4);
    let mut best = (0usize, 0f32);
    let mut scores = vec![0f32; max_stride + 1];
    for stride in 2..=max_stride {
        let pairs = sample.len() - stride;
        let same = sample[..pairs]
            .iter()
            .zip(&sample[stride..])
            .filter(|(a, b)| a == b)
            .count();
        let score = same as f32 / pairs as f32;
        scores[stride] = score;
        if score > best.1 {
            best = (stride, score);
        }
    }

    // A period has to beat chance by a clear margin, not merely be high: 82% of
    // bytes matching means nothing in a region that is 80% zeroes.
    let excess = best.1 - baseline;
    if best.1 < 0.35 || excess < 0.15 {
        return None;
    }
    // Prefer the smallest stride that is nearly as good: 48 scoring like 24
    // means the record is 24 bytes and 48 is two of them. Compared on the
    // excess, so a small stride inflated by padding does not win.
    let (stride, score) = (2..=best.0)
        .find(|s| scores[*s] - baseline >= excess * 0.95)
        .map(|s| (s, scores[s]))
        .unwrap_or(best);

    Some(Hint {
        label: "records".into(),
        detail: format!(
            "the bytes repeat every {stride} — {:.0}% of them match the byte {stride} later, \
             so this may be {} records of {stride} bytes",
            score * 100.0,
            sample.len() / stride
        ),
        offset: base,
        len: bytes.len() as u64,
    })
}

/// Read a 1–8 byte unsigned integer in the given byte order.
fn read_uint(bytes: &[u8], big: bool) -> u64 {
    let mut value = 0u64;
    if big {
        for b in bytes {
            value = (value << 8) | *b as u64;
        }
    } else {
        for b in bytes.iter().rev() {
            value = (value << 8) | *b as u64;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(hints: &[Hint]) -> Vec<&str> {
        hints.iter().map(|h| h.label.as_str()).collect()
    }

    #[test]
    fn a_run_of_one_byte_reads_as_padding_and_nothing_else() {
        let hints = infer(&[0xFF; 64], 0, 64);
        assert_eq!(labels(&hints), ["fill"]);
        assert!(hints[0].detail.contains("0xff"), "{}", hints[0].detail);
    }

    #[test]
    fn short_or_empty_regions_produce_nothing_rather_than_noise() {
        assert!(infer(&[], 0, 0).is_empty());
        assert!(infer(&[1, 2, 3], 0, 3).is_empty());
    }

    #[test]
    fn a_length_prefix_is_matched_against_what_follows_it() {
        // u32 = 12, then exactly 12 bytes.
        let mut bytes = 12u32.to_le_bytes().to_vec();
        bytes.extend((0..12u8).map(|i| i.wrapping_mul(37)));
        let hints = infer(&bytes, 0, bytes.len() as u64);
        let hint = hints.iter().find(|h| h.label == "size field").expect("size field");
        assert!(hint.detail.contains("little-endian"), "{}", hint.detail);
        assert_eq!(hint.len, 4);
    }

    #[test]
    fn a_big_endian_length_is_recognized_too() {
        let mut bytes = 8u32.to_be_bytes().to_vec();
        bytes.extend([9u8, 8, 7, 6, 5, 4, 3, 2]);
        let hints = infer(&bytes, 0, bytes.len() as u64);
        let hint = hints.iter().find(|h| h.label == "size field").expect("size field");
        assert!(hint.detail.contains("big-endian"), "{}", hint.detail);
    }

    #[test]
    fn an_ascending_run_of_u32s_reads_as_an_offset_table() {
        let mut bytes = Vec::new();
        for off in [0x20u32, 0x40, 0x80, 0x100, 0x180] {
            bytes.extend_from_slice(&off.to_le_bytes());
        }
        let hints = infer(&bytes, 0, 0x200);
        let hint = hints.iter().find(|h| h.label == "offset table").expect("offset table");
        assert!(hint.detail.contains('5'), "{}", hint.detail);
        assert_eq!(hint.len, 20);
    }

    #[test]
    fn values_past_the_end_of_the_file_are_not_offsets() {
        let mut bytes = Vec::new();
        for value in [0xDEAD_BEEFu32, 0xFEED_FACE, 0xCAFE_BABE, 0xBAAD_F00D] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        // The file is far too small for any of these to point into it.
        let hints = infer(&bytes, 0, 64);
        assert!(!labels(&hints).contains(&"offset table"), "{hints:?}");
    }

    #[test]
    fn packed_nul_terminated_strings_read_as_a_string_pool() {
        let mut bytes = Vec::new();
        for name in ["alpha", "bravo", "charlie", "delta"] {
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(0);
        }
        let hints = infer(&bytes, 0, bytes.len() as u64);
        let hint = hints.iter().find(|h| h.label == "string pool").expect("string pool");
        assert!(hint.detail.contains('4'), "{}", hint.detail);
    }

    #[test]
    fn a_stray_word_inside_binary_data_is_not_a_string_pool() {
        let mut bytes: Vec<u8> = (0..200u8).map(|i| i.wrapping_mul(97).wrapping_add(13)).collect();
        bytes[8..16].copy_from_slice(b"version\0");
        let hints = infer(&bytes, 0, bytes.len() as u64);
        assert!(!labels(&hints).contains(&"string pool"), "{hints:?}");
    }

    #[test]
    fn fixed_size_records_reveal_their_stride() {
        // 40 records of 16 bytes: a constant tag, a counter, and fixed padding —
        // the shape of a real record table, where only some fields vary.
        let mut bytes = Vec::new();
        for i in 0..40u8 {
            bytes.extend_from_slice(&[0x5A, 0x00, i, 0x00]);
            bytes.extend_from_slice(&[0xC3, 0x11, 0x00, 0x00]);
            bytes.extend_from_slice(&(i as u32 * 7).to_le_bytes());
            bytes.extend_from_slice(&[0, 0, 0, 0]);
        }
        let hints = infer(&bytes, 0, bytes.len() as u64);
        let hint = hints.iter().find(|h| h.label == "records").expect("records");
        assert!(hint.detail.contains("every 16"), "{}", hint.detail);
    }

    #[test]
    fn random_looking_bytes_claim_no_record_structure() {
        // A deterministic pseudo-random run: no period to find.
        let mut state = 0x1234_5678u32;
        let bytes: Vec<u8> = (0..2048)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect();
        let hints = infer(&bytes, 0, bytes.len() as u64);
        assert!(!labels(&hints).contains(&"records"), "{hints:?}");
    }

    #[test]
    fn padding_does_not_masquerade_as_a_record_layout() {
        // A sparse header: a handful of meaningful bytes in a sea of zeroes. Most
        // bytes do match the one two positions later, but only because most
        // bytes are zero — which is why the score is judged against chance and
        // not on its own. A real ELF header tripped this before the guard.
        let mut bytes = vec![0u8; 184];
        bytes[..20].copy_from_slice(&[
            0x7F, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0x3E, 0,
        ]);
        bytes[24] = 0x40;
        bytes[32] = 0x40;
        bytes[64] = 0x38;
        let hints = infer(&bytes, 0, bytes.len() as u64);
        assert!(
            !labels(&hints).contains(&"records"),
            "a mostly-zero region has no record layout: {hints:?}"
        );
    }

    #[test]
    fn hints_are_reported_at_their_absolute_offset() {
        let mut bytes = Vec::new();
        for off in [0x10u32, 0x20, 0x30, 0x40] {
            bytes.extend_from_slice(&off.to_le_bytes());
        }
        let hints = infer(&bytes, 0x1000, 0x2000);
        let hint = hints.iter().find(|h| h.label == "offset table").expect("offset table");
        assert_eq!(hint.offset, 0x1000, "offsets are file positions, not region ones");
    }
}
