//! Aligned byte-level comparison of two files.
//!
//! This is a leaf crate in the same spirit as `search`: it knows how to compare
//! two byte buffers and nothing about schemas, files on disk, or the UI.
//!
//! The comparison is *aligned* — byte `n` of A is compared against byte `n` of
//! B — which is the shape the real workflow needs: two saves of the same game,
//! two firmware revisions, a config before and after a settings change. Those
//! files keep their layout, so alignment tells you exactly which fields moved.
//! An edit-distance diff that resynchronizes after inserted bytes is a
//! different (and much more expensive) tool; when the two files are not the
//! same length, everything past the shorter one is reported as a size
//! difference rather than guessed at.
//!
//! The result is a list of maximal **runs** of differing bytes, which is both
//! how a reader thinks about a diff ("these three regions changed") and compact
//! enough to hand to a UI for tinting and navigation.

use serde::Serialize;

/// Bytes compared per block. Equal blocks are rejected with one slice
/// comparison (a `memcmp`), so identical regions cost almost nothing and only
/// blocks that actually differ are scanned byte by byte.
const BLOCK: usize = 8192;

/// Default cap on how many runs are recorded. A pathological comparison (two
/// unrelated large files) can differ in millions of places; past this point the
/// list stops growing and [`Diff::truncated`] is set. Differing-byte counts
/// stay exact either way.
pub const DEFAULT_MAX_RUNS: usize = 1_000_000;

/// A maximal span of consecutive differing bytes, as `[offset, offset + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Run {
    pub offset: u64,
    pub len: u64,
}

impl Run {
    /// One past the last differing byte.
    pub fn end(&self) -> u64 {
        self.offset + self.len
    }
}

/// The outcome of comparing two buffers.
///
/// Runs only ever describe the common prefix (`[0, common_len)`); a difference
/// in length is reported through [`Diff::a_len`] / [`Diff::b_len`] instead, so
/// a caller never has to wonder whether a run means "changed" or "absent".
#[derive(Debug, Clone)]
pub struct Diff {
    a_len: u64,
    b_len: u64,
    changed: u64,
    runs: Vec<Run>,
    truncated: bool,
}

/// Compare two buffers with the default run cap.
pub fn compare(a: &[u8], b: &[u8]) -> Diff {
    compare_with(a, b, DEFAULT_MAX_RUNS)
}

/// Compare two buffers, recording at most `max_runs` runs.
pub fn compare_with(a: &[u8], b: &[u8], max_runs: usize) -> Diff {
    let common = a.len().min(b.len());
    let mut runs: Vec<Run> = Vec::new();
    let mut changed: u64 = 0;
    let mut truncated = false;
    // Start of the run currently being accumulated. Held across blocks so a run
    // that straddles a block boundary stays one run.
    let mut open: Option<u64> = None;

    let mut i = 0usize;
    while i < common {
        let end = (i + BLOCK).min(common);
        if a[i..end] == b[i..end] {
            close(&mut runs, &mut open, i as u64, max_runs, &mut truncated);
            i = end;
            continue;
        }
        for k in i..end {
            if a[k] != b[k] {
                changed += 1;
                if open.is_none() {
                    open = Some(k as u64);
                }
            } else {
                close(&mut runs, &mut open, k as u64, max_runs, &mut truncated);
            }
        }
        i = end;
    }
    close(&mut runs, &mut open, common as u64, max_runs, &mut truncated);

    Diff {
        a_len: a.len() as u64,
        b_len: b.len() as u64,
        changed,
        runs,
        truncated,
    }
}

/// End the open run at `at`, if there is one.
fn close(
    runs: &mut Vec<Run>,
    open: &mut Option<u64>,
    at: u64,
    max_runs: usize,
    truncated: &mut bool,
) {
    let Some(start) = open.take() else { return };
    if runs.len() >= max_runs {
        *truncated = true;
        return;
    }
    runs.push(Run {
        offset: start,
        len: at - start,
    });
}

impl Diff {
    /// Length of the first (left-hand) file.
    pub fn a_len(&self) -> u64 {
        self.a_len
    }

    /// Length of the second (right-hand) file.
    pub fn b_len(&self) -> u64 {
        self.b_len
    }

    /// Bytes present in both files, i.e. the region the runs cover.
    pub fn common_len(&self) -> u64 {
        self.a_len.min(self.b_len)
    }

    /// How many bytes differ within the common prefix. Exact even when the run
    /// list was truncated.
    pub fn changed_bytes(&self) -> u64 {
        self.changed
    }

    /// The recorded runs, ascending by offset and never touching.
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Whether the run list hit its cap and stopped growing.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// True when the two files are byte-for-byte identical.
    pub fn is_identical(&self) -> bool {
        self.changed == 0 && self.a_len == self.b_len
    }

    /// The runs overlapping `[offset, offset + len)`.
    ///
    /// Runs are sorted and disjoint, so this is a binary search plus a walk —
    /// cheap enough for a viewer to call per visible window.
    pub fn runs_in(&self, offset: u64, len: u64) -> &[Run] {
        let end = offset.saturating_add(len);
        let from = self.runs.partition_point(|r| r.end() <= offset);
        let to = self.runs.partition_point(|r| r.offset < end);
        &self.runs[from..to.max(from)]
    }

    /// True if any byte in `[offset, offset + len)` differs. This is what maps
    /// a parsed field back onto the diff ("did this field change?").
    pub fn touches(&self, offset: u64, len: u64) -> bool {
        !self.runs_in(offset, len).is_empty()
    }

    /// The first run, if any — where "jump to the first change" lands.
    pub fn first_run(&self) -> Option<Run> {
        self.runs.first().copied()
    }

    /// The first run starting strictly after `offset` — "next change". Strict so
    /// that repeatedly stepping forward from a run walks off it instead of
    /// returning it again.
    pub fn next_run(&self, offset: u64) -> Option<Run> {
        let i = self.runs.partition_point(|r| r.offset <= offset);
        self.runs.get(i).copied()
    }

    /// The last run starting before `offset` — "previous change".
    pub fn prev_run(&self, offset: u64) -> Option<Run> {
        let i = self.runs.partition_point(|r| r.offset < offset);
        if i == 0 {
            None
        } else {
            self.runs.get(i - 1).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_buffers_have_no_runs() {
        let d = compare(b"hello world", b"hello world");
        assert!(d.is_identical());
        assert_eq!(d.changed_bytes(), 0);
        assert!(d.runs().is_empty());
        assert!(!d.truncated());
    }

    #[test]
    fn a_single_changed_byte_is_one_run() {
        let d = compare(b"hello world", b"hellO world");
        assert_eq!(d.changed_bytes(), 1);
        assert_eq!(d.runs(), [Run { offset: 4, len: 1 }]);
        assert!(!d.is_identical());
    }

    #[test]
    fn adjacent_changes_coalesce_but_a_matching_byte_splits_them() {
        //                0123456789
        let d = compare(b"aaaaaaaaaa", b"aXXaaaXXXa");
        assert_eq!(d.changed_bytes(), 5);
        assert_eq!(
            d.runs(),
            [Run { offset: 1, len: 2 }, Run { offset: 6, len: 3 }]
        );
    }

    #[test]
    fn a_run_spanning_a_block_boundary_stays_one_run() {
        // The scanner works a block at a time; a difference straddling the seam
        // must not be reported as two separate runs.
        let a = vec![0u8; BLOCK * 2];
        let mut b = a.clone();
        for byte in &mut b[(BLOCK - 2)..(BLOCK + 2)] {
            *byte = 0xFF;
        }
        let d = compare(&a, &b);
        assert_eq!(
            d.runs(),
            [Run {
                offset: (BLOCK - 2) as u64,
                len: 4
            }]
        );
        assert_eq!(d.changed_bytes(), 4);
    }

    #[test]
    fn a_run_ending_exactly_at_the_end_of_the_buffer_is_closed() {
        let d = compare(b"aaaa", b"aaXX");
        assert_eq!(d.runs(), [Run { offset: 2, len: 2 }]);
    }

    #[test]
    fn different_lengths_compare_only_the_common_prefix() {
        let d = compare(b"abcdef", b"abXd");
        assert_eq!(d.a_len(), 6);
        assert_eq!(d.b_len(), 4);
        assert_eq!(d.common_len(), 4);
        // Only the common prefix produces runs; the extra "ef" is a size
        // difference, not a changed byte.
        assert_eq!(d.runs(), [Run { offset: 2, len: 1 }]);
        assert_eq!(d.changed_bytes(), 1);
        assert!(!d.is_identical());
    }

    #[test]
    fn a_prefix_of_the_other_file_reports_no_changed_bytes() {
        let d = compare(b"abc", b"abcdef");
        assert_eq!(d.changed_bytes(), 0);
        assert!(d.runs().is_empty());
        // Equal content so far, but not the same file.
        assert!(!d.is_identical());
    }

    #[test]
    fn empty_inputs_are_handled() {
        assert!(compare(b"", b"").is_identical());
        let d = compare(b"", b"abc");
        assert_eq!(d.common_len(), 0);
        assert!(d.runs().is_empty());
    }

    #[test]
    fn the_run_cap_truncates_the_list_but_not_the_count() {
        // Alternating bytes make one run per changed byte.
        let a = vec![0u8; 64];
        let b: Vec<u8> = (0..64).map(|i| if i % 2 == 0 { 0xFF } else { 0 }).collect();
        let d = compare_with(&a, &b, 5);
        assert!(d.truncated());
        assert_eq!(d.runs().len(), 5);
        assert_eq!(d.changed_bytes(), 32, "counts stay exact past the cap");
    }

    #[test]
    fn window_queries_return_only_overlapping_runs() {
        let a = vec![0u8; 100];
        let mut b = a.clone();
        for k in [10, 11, 30, 70] {
            b[k] = 1;
        }
        let d = compare(&a, &b);
        assert_eq!(d.runs().len(), 3);
        // A window that starts inside a run still sees it.
        assert_eq!(d.runs_in(11, 1), [Run { offset: 10, len: 2 }]);
        assert_eq!(d.runs_in(0, 10), []);
        assert_eq!(
            d.runs_in(0, 40),
            [Run { offset: 10, len: 2 }, Run { offset: 30, len: 1 }]
        );
        assert_eq!(d.runs_in(80, 20), []);
        assert!(d.touches(30, 1));
        assert!(d.touches(28, 5));
        assert!(!d.touches(31, 5));
    }

    #[test]
    fn navigation_steps_between_runs() {
        let a = vec![0u8; 100];
        let mut b = a.clone();
        for k in [10, 30, 70] {
            b[k] = 1;
        }
        let d = compare(&a, &b);
        assert_eq!(d.first_run().unwrap().offset, 10);
        assert_eq!(d.next_run(0).unwrap().offset, 10);
        // `next` is strictly forward, so sitting on a run steps off it.
        assert_eq!(d.next_run(10).unwrap().offset, 30);
        assert_eq!(d.next_run(31).unwrap().offset, 70);
        assert!(d.next_run(71).is_none());
        assert!(d.prev_run(0).is_none());
        assert_eq!(d.prev_run(30).unwrap().offset, 10);
        assert_eq!(d.prev_run(100).unwrap().offset, 70);
    }
}
