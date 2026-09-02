//! Non-destructive binary editing (plan §17 / Phase 10).
//!
//! The reader layer ([`binary_reader::BinaryReader`]) is deliberately read-only:
//! it maps a file and never mutates it. Editing therefore lives here, as an
//! *overlay* on top of that immutable base — a set of per-offset byte
//! overrides. Reads consult the overlay first and fall back to the base, so the
//! original file on disk is untouched until the user explicitly saves.
//!
//! ```text
//! Original File  ──▶  Change Set (overlay)  ──▶  Validate  ──▶  New File
//! ```
//!
//! Edits are **overwrite-only**: a field's bytes change in place, never shifting
//! what follows. That keeps every offset in the parsed structure tree valid
//! after an edit, which is the whole point of the byte↔field relationship — an
//! insert/delete model would invalidate every offset past the edit. (Variable-
//! length structural editing is a later, separate concern.)
//!
//! Every change is recorded so it can be undone and redone, and the set of
//! touched offsets is exposed so the UI can highlight what has been modified.

use std::collections::BTreeMap;
use std::path::Path;

use binary_reader::Endian;

/// A single overwrite operation, with enough history to undo and redo it.
///
/// `prev` holds, for each byte written, the overlay state *before* this edit —
/// `Some(v)` if that offset already had an override, `None` if it was still the
/// base byte. That lets undo restore the exact prior state instead of assuming
/// the byte reverts to base.
#[derive(Debug, Clone)]
struct Edit {
    offset: usize,
    prev: Vec<Option<u8>>,
    new: Vec<u8>,
}

/// An in-memory overlay of overwrite edits on top of a fixed-length base file.
///
/// The buffer never holds the base bytes itself — callers pass the base slice
/// (from the [`BinaryReader`](binary_reader::BinaryReader)) to the read/apply
/// methods. This keeps the editor independent of *how* the base is stored
/// (memory map, buffer, …) and avoids duplicating a multi-GB file in RAM.
#[derive(Debug, Default, Clone)]
pub struct EditBuffer {
    base_len: usize,
    overlay: BTreeMap<usize, u8>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl EditBuffer {
    /// Create an empty buffer for a base file of `base_len` bytes.
    pub fn new(base_len: usize) -> Self {
        Self {
            base_len,
            ..Default::default()
        }
    }

    /// Whether any edits are pending (i.e. the buffer differs from the base).
    pub fn is_dirty(&self) -> bool {
        !self.overlay.is_empty()
    }

    /// Number of individual bytes that currently differ from the base.
    pub fn dirty_count(&self) -> usize {
        self.overlay.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The offsets that currently differ from the base, ascending.
    pub fn dirty_offsets(&self) -> impl Iterator<Item = usize> + '_ {
        self.overlay.keys().copied()
    }

    /// The edited byte at `offset`, if it has been overridden.
    pub fn overridden(&self, offset: usize) -> Option<u8> {
        self.overlay.get(&offset).copied()
    }

    /// Overwrite `bytes` starting at `offset`. Returns an error if the range
    /// would extend past the end of the base file (overwrite can't grow a file).
    ///
    /// Recorded as a single undoable step; doing so clears the redo stack.
    pub fn set_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), EditError> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or(EditError::OutOfBounds)?;
        if end > self.base_len {
            return Err(EditError::OutOfBounds);
        }
        if bytes.is_empty() {
            return Ok(());
        }

        let prev: Vec<Option<u8>> = (offset..end).map(|o| self.overlay.get(&o).copied()).collect();
        let edit = Edit {
            offset,
            prev,
            new: bytes.to_vec(),
        };
        self.apply(&edit);
        self.undo.push(edit);
        self.redo.clear();
        Ok(())
    }

    /// Write `edit.new` into the overlay at `edit.offset`.
    fn apply(&mut self, edit: &Edit) {
        for (i, &b) in edit.new.iter().enumerate() {
            self.overlay.insert(edit.offset + i, b);
        }
    }

    /// Restore the overlay to the state captured in `edit.prev`.
    fn unapply(&mut self, edit: &Edit) {
        for (i, prev) in edit.prev.iter().enumerate() {
            let o = edit.offset + i;
            match prev {
                Some(v) => {
                    self.overlay.insert(o, *v);
                }
                None => {
                    self.overlay.remove(&o);
                }
            }
        }
    }

    /// Undo the most recent edit. Returns `false` if there was nothing to undo.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(edit) => {
                self.unapply(&edit);
                self.redo.push(edit);
                true
            }
            None => false,
        }
    }

    /// Redo the most recently undone edit. Returns `false` if nothing to redo.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(edit) => {
                self.apply(&edit);
                self.undo.push(edit);
                true
            }
            None => false,
        }
    }

    /// Discard all edits and history, reverting to the pristine base.
    pub fn clear(&mut self) {
        self.overlay.clear();
        self.undo.clear();
        self.redo.clear();
    }

    /// Apply overrides in place onto a window read from the base file, where
    /// `window_offset` is the window's absolute start. Only the overrides that
    /// fall inside the window are touched (via a `BTreeMap` range), so this stays
    /// cheap even for a small window of a huge, heavily-edited file.
    pub fn apply_window(&self, window: &mut [u8], window_offset: usize) {
        let end = window_offset + window.len();
        for (&o, &b) in self.overlay.range(window_offset..end) {
            window[o - window_offset] = b;
        }
    }

    /// Copy `base[offset..offset+len]` with any overlay edits applied on top.
    /// The caller supplies the base slice (typically the whole mapped file).
    pub fn read_with_edits(&self, base: &[u8], offset: usize, len: usize) -> Vec<u8> {
        let end = (offset + len).min(base.len());
        let mut out = base[offset.min(base.len())..end].to_vec();
        // Apply only the overrides that fall inside this window.
        for (&o, &b) in self.overlay.range(offset..end) {
            out[o - offset] = b;
        }
        out
    }

    /// Materialize the full edited file: the base with every override applied.
    pub fn materialize(&self, base: &[u8]) -> Vec<u8> {
        let mut out = base.to_vec();
        for (&o, &b) in &self.overlay {
            if o < out.len() {
                out[o] = b;
            }
        }
        out
    }

    /// Write the edited file to `path`. If `backup` is set and `path` already
    /// exists, the current on-disk file is first copied to `path.bak`.
    ///
    /// This does not itself clear the dirty state — the caller decides whether a
    /// save-as keeps the edits pending against the original or adopts the copy.
    pub fn write_to(&self, base: &[u8], path: impl AsRef<Path>, backup: bool) -> std::io::Result<()> {
        let path = path.as_ref();
        if backup && path.exists() {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            std::fs::copy(path, bak)?;
        }
        std::fs::write(path, self.materialize(base))
    }
}

/// Errors from applying an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The edit would extend past the end of the file (overwrite can't grow it).
    OutOfBounds,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::OutOfBounds => write!(f, "edit extends past the end of the file"),
        }
    }
}

impl std::error::Error for EditError {}

/// The category of value the UI is asking to encode. Mirrors the runtime's
/// `Value` variants closely enough to turn a user-typed string back into bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Unsigned,
    Signed,
    Float,
    Bool,
    Char,
    Str,
    Bytes,
}

impl ValueKind {
    /// Parse the tag the UI sends (matches the runtime `Value` serde tags).
    pub fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "u" | "unsigned" => ValueKind::Unsigned,
            "i" | "signed" => ValueKind::Signed,
            "f" | "float" => ValueKind::Float,
            "bool" => ValueKind::Bool,
            "char" => ValueKind::Char,
            "str" | "string" => ValueKind::Str,
            "bytes" => ValueKind::Bytes,
            _ => return None,
        })
    }
}

/// Encode a user-entered value into exactly `size` bytes for the given kind and
/// endianness — the "Encode" step between a parsed value and the change set.
///
/// The result is always `size` bytes long so it can overwrite the field in
/// place. Numbers must fit the width; strings/bytes must fit the field and are
/// NUL/zero-padded to fill it.
pub fn encode_value(
    kind: ValueKind,
    size: usize,
    endian: Endian,
    input: &str,
) -> Result<Vec<u8>, String> {
    let big = endian == Endian::Big;
    let input = input.trim();
    match kind {
        ValueKind::Unsigned => encode_uint(input, size, big),
        ValueKind::Signed => encode_int(input, size, big),
        ValueKind::Float => encode_float(input, size, big),
        ValueKind::Bool => {
            let v = match input.to_ascii_lowercase().as_str() {
                "true" | "1" => 1u8,
                "false" | "0" => 0u8,
                _ => return Err("bool must be true/false or 1/0".into()),
            };
            Ok(vec![v])
        }
        ValueKind::Char => {
            let mut chars = input.chars();
            let c = chars.next().ok_or("char cannot be empty")?;
            if chars.next().is_some() {
                return Err("char must be a single character".into());
            }
            let code = c as u32;
            if code > 0xFF {
                return Err("char must be a single byte (0x00–0xFF)".into());
            }
            Ok(vec![code as u8])
        }
        ValueKind::Str => {
            let bytes = input.as_bytes();
            if bytes.len() > size {
                return Err(format!("string is {} bytes but field holds {size}", bytes.len()));
            }
            let mut out = vec![0u8; size];
            out[..bytes.len()].copy_from_slice(bytes);
            Ok(out)
        }
        ValueKind::Bytes => {
            let bytes = parse_hex(input)?;
            if bytes.len() > size {
                return Err(format!("{} bytes given but field holds {size}", bytes.len()));
            }
            let mut out = vec![0u8; size];
            out[..bytes.len()].copy_from_slice(&bytes);
            Ok(out)
        }
    }
}

/// Parse a possibly-`0x`-prefixed integer in any base the user likely means.
fn parse_i128(input: &str) -> Result<i128, String> {
    let s = input.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mag = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i128::from_str_radix(hex, 16).map_err(|e| e.to_string())?
    } else {
        s.parse::<i128>().map_err(|e| e.to_string())?
    };
    Ok(if neg { -mag } else { mag })
}

fn encode_uint(input: &str, size: usize, big: bool) -> Result<Vec<u8>, String> {
    let v = parse_i128(input)?;
    if v < 0 {
        return Err("value must be non-negative for an unsigned field".into());
    }
    let max: i128 = if size >= 16 { i128::MAX } else { (1i128 << (size * 8)) - 1 };
    if v > max {
        return Err(format!("{v} does not fit in a {size}-byte unsigned field"));
    }
    Ok(sized_bytes(v as u128, size, big))
}

fn encode_int(input: &str, size: usize, big: bool) -> Result<Vec<u8>, String> {
    let v = parse_i128(input)?;
    let bits = size * 8;
    let min = -(1i128 << (bits - 1));
    let max = (1i128 << (bits - 1)) - 1;
    if v < min || v > max {
        return Err(format!("{v} does not fit in a {size}-byte signed field"));
    }
    // Two's-complement representation in `size` bytes.
    let mask: u128 = if size >= 16 { u128::MAX } else { (1u128 << bits) - 1 };
    Ok(sized_bytes((v as u128) & mask, size, big))
}

fn encode_float(input: &str, size: usize, big: bool) -> Result<Vec<u8>, String> {
    match size {
        4 => {
            let v: f32 = input.parse().map_err(|_| "invalid f32".to_string())?;
            Ok(if big { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() })
        }
        8 => {
            let v: f64 = input.parse().map_err(|_| "invalid f64".to_string())?;
            Ok(if big { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() })
        }
        _ => Err(format!("unsupported float size {size}")),
    }
}

/// Take the low `size` bytes of `v` in the requested byte order.
fn sized_bytes(v: u128, size: usize, big: bool) -> Vec<u8> {
    let full = v.to_le_bytes(); // little-endian, 16 bytes
    let mut out: Vec<u8> = full[..size].to_vec();
    if big {
        out.reverse();
    }
    out
}

/// Parse a hex string (spaces optional, `0x` prefix allowed) into bytes.
fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = input
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if cleaned.len() % 2 != 0 {
        return Err("hex must have an even number of digits".into());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_read_back() {
        let base = vec![0u8; 8];
        let mut buf = EditBuffer::new(base.len());
        buf.set_bytes(2, &[0xAA, 0xBB]).unwrap();
        assert!(buf.is_dirty());
        assert_eq!(buf.dirty_count(), 2);
        let win = buf.read_with_edits(&base, 0, 8);
        assert_eq!(win, vec![0, 0, 0xAA, 0xBB, 0, 0, 0, 0]);
    }

    #[test]
    fn overwrite_out_of_bounds_is_rejected() {
        let mut buf = EditBuffer::new(4);
        assert_eq!(buf.set_bytes(3, &[1, 2]), Err(EditError::OutOfBounds));
        assert!(!buf.is_dirty());
    }

    #[test]
    fn undo_redo_round_trip() {
        let base = vec![0u8; 4];
        let mut buf = EditBuffer::new(4);
        buf.set_bytes(0, &[0x11]).unwrap();
        buf.set_bytes(1, &[0x22]).unwrap();
        assert!(buf.can_undo());
        assert!(buf.undo()); // revert the 0x22
        assert_eq!(buf.read_with_edits(&base, 0, 4), vec![0x11, 0, 0, 0]);
        assert!(buf.undo()); // revert the 0x11
        assert!(!buf.is_dirty());
        assert!(!buf.undo()); // nothing left
        assert!(buf.redo()); // reinstate 0x11
        assert_eq!(buf.read_with_edits(&base, 0, 4), vec![0x11, 0, 0, 0]);
    }

    #[test]
    fn overlapping_edit_undo_restores_prior_override() {
        let base = vec![0u8; 4];
        let mut buf = EditBuffer::new(4);
        buf.set_bytes(0, &[0x11, 0x22]).unwrap();
        buf.set_bytes(1, &[0x99]).unwrap(); // overwrites the 0x22
        assert_eq!(buf.read_with_edits(&base, 0, 4), vec![0x11, 0x99, 0, 0]);
        buf.undo(); // should restore 0x22, not base 0x00
        assert_eq!(buf.read_with_edits(&base, 0, 4), vec![0x11, 0x22, 0, 0]);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buf = EditBuffer::new(4);
        buf.set_bytes(0, &[1]).unwrap();
        buf.undo();
        assert!(buf.can_redo());
        buf.set_bytes(0, &[2]).unwrap();
        assert!(!buf.can_redo());
    }

    #[test]
    fn encode_unsigned_endianness() {
        let le = encode_value(ValueKind::Unsigned, 4, Endian::Little, "128").unwrap();
        assert_eq!(le, vec![0x80, 0, 0, 0]);
        let be = encode_value(ValueKind::Unsigned, 4, Endian::Big, "128").unwrap();
        assert_eq!(be, vec![0, 0, 0, 0x80]);
    }

    #[test]
    fn encode_hex_input() {
        let v = encode_value(ValueKind::Unsigned, 2, Endian::Big, "0x00FF").unwrap();
        assert_eq!(v, vec![0x00, 0xFF]);
    }

    #[test]
    fn encode_signed_negative_twos_complement() {
        let v = encode_value(ValueKind::Signed, 1, Endian::Little, "-1").unwrap();
        assert_eq!(v, vec![0xFF]);
        let v = encode_value(ValueKind::Signed, 2, Endian::Little, "-2").unwrap();
        assert_eq!(v, vec![0xFE, 0xFF]);
    }

    #[test]
    fn encode_overflow_rejected() {
        assert!(encode_value(ValueKind::Unsigned, 1, Endian::Little, "256").is_err());
        assert!(encode_value(ValueKind::Signed, 1, Endian::Little, "128").is_err());
        assert!(encode_value(ValueKind::Unsigned, 1, Endian::Little, "-1").is_err());
    }

    #[test]
    fn encode_float_roundtrips() {
        let v = encode_value(ValueKind::Float, 4, Endian::Little, "1.5").unwrap();
        assert_eq!(f32::from_le_bytes([v[0], v[1], v[2], v[3]]), 1.5);
    }

    #[test]
    fn encode_string_pads_to_field() {
        let v = encode_value(ValueKind::Str, 5, Endian::Little, "Hi").unwrap();
        assert_eq!(v, vec![b'H', b'i', 0, 0, 0]);
        assert!(encode_value(ValueKind::Str, 2, Endian::Little, "toolong").is_err());
    }

    #[test]
    fn materialize_applies_all_edits() {
        let base = vec![1u8, 2, 3, 4];
        let mut buf = EditBuffer::new(4);
        buf.set_bytes(1, &[0x20, 0x30]).unwrap();
        assert_eq!(buf.materialize(&base), vec![1, 0x20, 0x30, 4]);
    }
}
