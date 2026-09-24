//! How much of a file a schema actually accounts for.
//!
//! The question a half-finished schema raises is "what have I not explained
//! yet?", and a parse tree answers it implicitly: every field records the bytes
//! it read, so the bytes no field claims are exactly the part still unknown.
//! This turns that into a number and a list of holes to go and look at.
//!
//! Only fields that *read bytes themselves* are counted. A struct's span
//! covers its children, so counting containers as well would paper over a hole
//! between two of their fields — which is the interesting case, not a detail.

use serde::{Deserialize, Serialize};

use crate::FieldNode;

/// A byte range, as `[offset, offset + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub offset: u64,
    pub len: u64,
}

/// Cap on how many gaps are listed. A schema that explains almost nothing of a
/// large file can produce an enormous list; the counts stay exact.
const MAX_GAPS: usize = 10_000;

/// What a parse explained, and what it left untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Size of the file the parse ran against.
    pub total: u64,
    /// Bytes claimed by at least one field.
    pub covered: u64,
    /// Runs of bytes no field claims, ascending.
    pub gaps: Vec<Span>,
    /// True when the gap list hit its cap; `covered` is still exact.
    pub truncated: bool,
}

impl Coverage {
    /// Fraction of the file explained, in `0.0..=1.0`. An empty file counts as
    /// fully explained: there is nothing left to account for.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.covered as f64 / self.total as f64
        }
    }
}

/// Measure what `root` explains of a `total`-byte file.
pub fn coverage(root: &FieldNode, total: usize) -> Coverage {
    let total = total as u64;
    let mut spans = Vec::new();
    claim(root, total, &mut spans);
    spans.sort_unstable_by_key(|s| (s.offset, s.len));

    // Merge, then invert: walking the merged spans once yields both the covered
    // total and the holes between them.
    let mut covered = 0u64;
    let mut gaps = Vec::new();
    let mut truncated = false;
    let mut cursor = 0u64; // first byte not yet accounted for
    for span in &spans {
        if span.offset > cursor {
            if gaps.len() < MAX_GAPS {
                gaps.push(Span {
                    offset: cursor,
                    len: span.offset - cursor,
                });
            } else {
                truncated = true;
            }
            cursor = span.offset;
        }
        let end = span.offset + span.len;
        if end > cursor {
            covered += end - cursor;
            cursor = end;
        }
    }
    if cursor < total {
        if gaps.len() < MAX_GAPS {
            gaps.push(Span {
                offset: cursor,
                len: total - cursor,
            });
        } else {
            truncated = true;
        }
    }

    Coverage {
        total,
        covered,
        gaps,
        truncated,
    }
}

/// Collect the spans of every node that reads bytes of the file directly.
fn claim(node: &FieldNode, total: u64, out: &mut Vec<Span>) {
    // A decoded node's children are positioned in the decoded buffer, not the
    // file, so the encoded span it holds is the last thing here that is a real
    // file position. Descending past it would claim unrelated bytes.
    if node.children.is_empty() || node.decoded {
        let offset = node.offset as u64;
        let len = node.size as u64;
        // A pointer can send a field past the end of a truncated file; clamp
        // rather than report coverage the file cannot contain.
        if len > 0 && offset < total {
            out.push(Span {
                offset,
                len: len.min(total - offset),
            });
        }
        return;
    }
    for child in &node.children {
        claim(child, total, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn node(name: &str, offset: usize, size: usize, children: Vec<FieldNode>) -> FieldNode {
        FieldNode {
            name: name.to_string(),
            type_name: "t".into(),
            value: if children.is_empty() {
                Value::U(0)
            } else {
                Value::Struct
            },
            offset,
            size,
            description: String::new(),
            check: None,
            decoded: false,
            children,
        }
    }

    #[test]
    fn a_schema_covering_the_whole_file_leaves_no_gaps() {
        let root = node("r", 0, 8, vec![node("a", 0, 4, vec![]), node("b", 4, 4, vec![])]);
        let c = coverage(&root, 8);
        assert_eq!(c.covered, 8);
        assert!(c.gaps.is_empty());
        assert_eq!(c.fraction(), 1.0);
    }

    #[test]
    fn unparsed_bytes_at_the_end_are_a_gap() {
        let root = node("r", 0, 4, vec![node("a", 0, 4, vec![])]);
        let c = coverage(&root, 16);
        assert_eq!(c.covered, 4);
        assert_eq!(c.gaps, [Span { offset: 4, len: 12 }]);
        assert_eq!(c.fraction(), 0.25);
    }

    #[test]
    fn a_hole_between_two_fields_is_reported() {
        // The case counting containers would hide: the struct's own span spans
        // the hole, but no field explains the bytes in it.
        let root = node(
            "r",
            0,
            16,
            vec![node("a", 0, 4, vec![]), node("b", 12, 4, vec![])],
        );
        let c = coverage(&root, 16);
        assert_eq!(c.covered, 8);
        assert_eq!(c.gaps, [Span { offset: 4, len: 8 }]);
    }

    #[test]
    fn a_gap_before_the_first_field_is_reported() {
        let root = node("r", 8, 4, vec![node("a", 8, 4, vec![])]);
        let c = coverage(&root, 12);
        assert_eq!(c.gaps, [Span { offset: 0, len: 8 }]);
    }

    #[test]
    fn overlapping_fields_are_counted_once() {
        // A pointer can make two fields read the same bytes; coverage is about
        // the file, so those bytes are explained once, not twice.
        let root = node(
            "r",
            0,
            8,
            vec![node("a", 0, 8, vec![]), node("ptr", 4, 4, vec![])],
        );
        let c = coverage(&root, 8);
        assert_eq!(c.covered, 8);
        assert!(c.gaps.is_empty());
    }

    #[test]
    fn a_decoded_subtree_counts_its_encoded_span_and_stops() {
        // The children carry decoded-buffer offsets; counting them would claim
        // bytes at the start of the file that nothing actually read.
        let mut blob = node("blob", 40, 20, vec![node("inner", 0, 12, vec![])]);
        blob.decoded = true;
        let root = node("r", 0, 60, vec![node("head", 0, 40, vec![]), blob]);
        let c = coverage(&root, 60);
        assert_eq!(c.covered, 60);
        assert!(c.gaps.is_empty(), "the decoded children must not be counted");
    }

    #[test]
    fn a_field_reaching_past_the_end_is_clamped() {
        let root = node("r", 0, 4, vec![node("a", 0, 100, vec![])]);
        let c = coverage(&root, 10);
        assert_eq!(c.covered, 10);
        assert!(c.gaps.is_empty());
    }

    #[test]
    fn an_empty_file_is_fully_explained() {
        let root = node("r", 0, 0, vec![]);
        let c = coverage(&root, 0);
        assert_eq!(c.fraction(), 1.0);
        assert!(c.gaps.is_empty());
    }

    #[test]
    fn zero_width_fields_claim_nothing() {
        let root = node("r", 0, 0, vec![node("a", 0, 0, vec![])]);
        let c = coverage(&root, 4);
        assert_eq!(c.covered, 0);
        assert_eq!(c.gaps, [Span { offset: 0, len: 4 }]);
    }
}
