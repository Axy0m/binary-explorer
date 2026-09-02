//! Schema packaging metadata (plan §19 / Phase 12).
//!
//! A schema is just DSL text, but to *share* one we also need to know its entry
//! struct, its endianness, a human name, and a short description. Rather than a
//! separate manifest, that metadata rides along inside the schema file as
//! specially-formatted comment lines, so a `.schema` file stays a single,
//! self-contained, still-valid-DSL document:
//!
//! ```text
//! // @name PNG image
//! // @entry PNG
//! // @endian be
//! // @desc PNG signature + IHDR header chunk
//!
//! struct PNG { ... }
//! ```
//!
//! This crate is pure (no filesystem): it only parses and emits that header. The
//! app layer owns reading/writing files and the on-disk library directory.

use serde::{Deserialize, Serialize};

/// Portable metadata describing a shareable schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub entry: String,
    /// `"le"` or `"be"`; defaults to `"le"` when unspecified.
    pub endian: String,
    pub description: String,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            name: String::new(),
            entry: String::new(),
            endian: "le".to_string(),
            description: String::new(),
        }
    }
}

/// Extract [`Metadata`] from a schema's `// @key value` comment lines.
///
/// Lines may appear anywhere; unknown keys are ignored. Recognized keys are
/// `name`, `entry`, `endian`, and `desc`/`description`. `endian` is normalized
/// to `"be"`/`"le"` (defaulting to `"le"` for anything else).
pub fn parse_metadata(text: &str) -> Metadata {
    let mut meta = Metadata::default();
    let mut saw_endian = false;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("//") else {
            continue;
        };
        let rest = rest.trim();
        let Some(tag) = rest.strip_prefix('@') else {
            continue;
        };
        // Split "key value..." into the key and the remaining value.
        let (key, value) = match tag.split_once(char::is_whitespace) {
            Some((k, v)) => (k, v.trim()),
            None => (tag, ""),
        };
        match key {
            "name" => meta.name = value.to_string(),
            "entry" => meta.entry = value.to_string(),
            "endian" => {
                meta.endian = normalize_endian(value);
                saw_endian = true;
            }
            "desc" | "description" => meta.description = value.to_string(),
            _ => {}
        }
    }
    if !saw_endian {
        meta.endian = "le".to_string();
    }
    meta
}

/// Render [`Metadata`] as a comment header (only non-empty fields, `endian`
/// always). The result ends with a trailing newline so schema text can follow.
pub fn header(meta: &Metadata) -> String {
    let mut out = String::new();
    if !meta.name.is_empty() {
        out.push_str(&format!("// @name {}\n", meta.name));
    }
    if !meta.entry.is_empty() {
        out.push_str(&format!("// @entry {}\n", meta.entry));
    }
    out.push_str(&format!("// @endian {}\n", normalize_endian(&meta.endian)));
    if !meta.description.is_empty() {
        out.push_str(&format!("// @desc {}\n", meta.description));
    }
    out
}

/// Prepend (or replace) a metadata header on `body`, returning a shareable file.
///
/// Any leading run of `// @…` metadata lines already in `body` is dropped first
/// so re-exporting a schema doesn't stack duplicate headers.
pub fn with_header(meta: &Metadata, body: &str) -> String {
    let stripped = strip_leading_metadata(body);
    let header = header(meta);
    if stripped.is_empty() {
        header
    } else {
        format!("{header}\n{stripped}")
    }
}

/// Remove a leading block of `// @key value` lines (and the blank lines between
/// them), returning the schema body proper.
fn strip_leading_metadata(text: &str) -> String {
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.peek() {
        let t = line.trim();
        let is_meta = t
            .strip_prefix("//")
            .map(|r| r.trim_start().starts_with('@'))
            .unwrap_or(false);
        if is_meta || t.is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    lines.collect::<Vec<_>>().join("\n")
}

fn normalize_endian(v: &str) -> String {
    if v.trim().eq_ignore_ascii_case("be") {
        "be".to_string()
    } else {
        "le".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_fields() {
        let text = "// @name PNG image\n// @entry PNG\n// @endian be\n// @desc a header\n\nstruct PNG {}";
        let m = parse_metadata(text);
        assert_eq!(m.name, "PNG image");
        assert_eq!(m.entry, "PNG");
        assert_eq!(m.endian, "be");
        assert_eq!(m.description, "a header");
    }

    #[test]
    fn endian_defaults_to_le_and_normalizes() {
        assert_eq!(parse_metadata("struct S {}").endian, "le");
        assert_eq!(parse_metadata("// @endian BE").endian, "be");
        assert_eq!(parse_metadata("// @endian nonsense").endian, "le");
    }

    #[test]
    fn description_alias_and_missing_fields() {
        let m = parse_metadata("// @description hello");
        assert_eq!(m.description, "hello");
        assert_eq!(m.name, "");
        assert_eq!(m.entry, "");
    }

    #[test]
    fn header_round_trips() {
        let meta = Metadata {
            name: "My Format".into(),
            entry: "Root".into(),
            endian: "be".into(),
            description: "desc".into(),
        };
        let round = parse_metadata(&header(&meta));
        assert_eq!(round, meta);
    }

    #[test]
    fn with_header_replaces_existing_header() {
        let original = "// @name Old\n// @entry A\n// @endian le\n\nstruct A { x u8 }";
        let meta = Metadata {
            name: "New".into(),
            entry: "A".into(),
            endian: "le".into(),
            description: String::new(),
        };
        let out = with_header(&meta, original);
        assert!(out.contains("// @name New"));
        assert!(!out.contains("Old"));
        assert!(out.contains("struct A { x u8 }"));
        // Only one @name line survives.
        assert_eq!(out.matches("@name").count(), 1);
    }
}
