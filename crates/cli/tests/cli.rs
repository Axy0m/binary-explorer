//! End-to-end tests for the `nybble` binary.
//!
//! These run the real executable and assert on its exit status, because the
//! status is the part scripts depend on: 0 matched, 1 did not, 2 you held it
//! wrong. A regression there breaks every pipeline using the tool, silently.

use std::path::PathBuf;
use std::process::{Command, Output};

fn nybble(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nybble"))
        .args(args)
        .output()
        .expect("the nybble binary should run")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited normally")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Write a fixture into the temp dir, named after the test that wants it.
fn fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("nybble_cli_{name}"));
    std::fs::write(&path, bytes).expect("fixture should be writable");
    path
}

/// A schema and a file that matches it: two little-endian u16s.
fn pair(name: &str) -> (PathBuf, PathBuf) {
    let schema = fixture(
        &format!("{name}.schema"),
        b"// @entry S\n// @endian le\nstruct S { a u16  b u16 }\n",
    );
    let data = fixture(&format!("{name}.bin"), &[1, 0, 2, 0]);
    (schema, data)
}

#[test]
fn check_accepts_a_valid_schema_and_rejects_a_broken_one() {
    let good = fixture("check_good.schema", b"struct S { a u8 }\n");
    let out = nybble(&["check", good.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "a valid schema should exit 0");
    assert!(stdout(&out).contains("1 struct"), "{}", stdout(&out));

    let bad = fixture("check_bad.schema", b"struct S { this is not valid\n");
    assert_eq!(
        code(&nybble(&["check", bad.to_str().unwrap()])),
        1,
        "a schema that does not compile should exit 1"
    );
}

#[test]
fn parse_uses_the_schemas_own_entry_and_endianness() {
    // No --entry or --endian: both come from the `// @` header.
    let (schema, data) = pair("parse_meta");
    let out = nybble(&["parse", schema.to_str().unwrap(), data.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("a: u16 = 1"), "{text}");
    assert!(text.contains("b: u16 = 2"), "{text}");
}

#[test]
fn parse_reports_a_file_that_does_not_match_as_a_failure() {
    // The schema wants four bytes; the file has two.
    let (schema, _) = pair("parse_short");
    let short = fixture("parse_short_trunc.bin", &[1, 0]);
    let out = nybble(&[
        "parse",
        schema.to_str().unwrap(),
        short.to_str().unwrap(),
        "--quiet",
    ]);
    assert_eq!(code(&out), 1, "a parse fault should exit 1");
    assert!(stdout(&out).is_empty(), "--quiet should print nothing");
}

#[test]
fn parse_json_is_machine_readable() {
    let (schema, data) = pair("parse_json");
    let out = nybble(&[
        "parse",
        schema.to_str().unwrap(),
        data.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    // Shape, not exact bytes: the point is that it is JSON carrying the tree.
    assert!(text.trim_start().starts_with('{'), "{text}");
    assert!(text.contains("\"tree\""), "{text}");
    assert!(text.contains("\"offset\""), "{text}");
}

#[test]
fn a_failing_checksum_fails_the_parse_even_though_the_bytes_decode() {
    // Every field reads fine; the file is simply no longer self-consistent,
    // which is exactly what a CI check wants to catch.
    let schema = fixture(
        "check_sum.schema",
        b"// @entry S\nstruct S { data u16  sum u8 check sum8 over(data) }\n",
    );
    let ok = fixture("check_sum_ok.bin", &[3, 4, 7]);
    assert_eq!(
        code(&nybble(&["parse", schema.to_str().unwrap(), ok.to_str().unwrap(), "--quiet"])),
        0
    );
    let bad = fixture("check_sum_bad.bin", &[3, 4, 99]);
    assert_eq!(
        code(&nybble(&["parse", schema.to_str().unwrap(), bad.to_str().unwrap(), "--quiet"])),
        1,
        "a wrong checksum should exit 1"
    );
}

#[test]
fn min_coverage_catches_a_schema_that_stopped_explaining_the_file() {
    // The schema reads four bytes and the file has eight. Every field parses,
    // so nothing else here would call it a failure — but half the file is
    // unaccounted for, which is what a format that grew a section looks like.
    let (schema, _) = pair("coverage");
    let long = fixture("coverage_long.bin", &[1, 0, 2, 0, 9, 9, 9, 9]);
    let path = long.to_str().unwrap();

    assert_eq!(
        code(&nybble(&["parse", schema.to_str().unwrap(), path, "--quiet"])),
        0,
        "without a floor, a clean parse is still a pass"
    );
    let out = nybble(&[
        "parse",
        schema.to_str().unwrap(),
        path,
        "--min-coverage",
        "90",
    ]);
    assert_eq!(code(&out), 1, "50% coverage should not clear a 90% floor");

    assert_eq!(
        code(&nybble(&["parse", schema.to_str().unwrap(), path, "--min-coverage", "50"])),
        0,
        "and exactly meeting the floor passes"
    );
    assert_eq!(
        code(&nybble(&["parse", schema.to_str().unwrap(), path, "--min-coverage", "several"])),
        2,
        "a floor that is not a percentage is a usage error"
    );
}

#[test]
fn diff_exits_zero_only_when_the_files_are_identical() {
    let a = fixture("diff_a.bin", b"hello world");
    let same = fixture("diff_same.bin", b"hello world");
    let other = fixture("diff_b.bin", b"hellO world");

    let out = nybble(&["diff", a.to_str().unwrap(), same.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("identical"));

    let out = nybble(&["diff", a.to_str().unwrap(), other.to_str().unwrap()]);
    assert_eq!(code(&out), 1, "a difference should exit 1");
    assert!(stdout(&out).contains("1 byte(s) differ"), "{}", stdout(&out));
}

#[test]
fn a_changed_run_is_listed_from_the_first_file_to_the_second() {
    // `diff a b` reads "a became b", the same direction the parse tree shows a
    // changed field in. Printing it the other way round is silently wrong: both
    // sides are plausible hex, so nothing about the output looks off.
    let before = fixture("diff_dir_a.bin", b"name=alpha");
    let after = fixture("diff_dir_b.bin", b"name=OMEGA");
    let out = nybble(&["diff", before.to_str().unwrap(), after.to_str().unwrap()]);
    let row = stdout(&out)
        .lines()
        .find(|l| l.trim_start().starts_with("0x"))
        .expect("a changed region should be listed")
        .to_string();
    let (left, right) = row.split_once("->").expect("a row shows both sides");
    assert!(left.contains("|alpha|"), "the left side is the first file: {row}");
    assert!(right.contains("|OMEGA|"), "the right side is the second: {row}");
}

#[test]
fn detect_identifies_a_png_and_gives_up_on_noise() {
    let png = fixture(
        "detect.png",
        &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13],
    );
    let out = nybble(&["detect", png.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("PNG"), "{}", stdout(&out));

    let noise = fixture("detect_noise.bin", &[0x11, 0x22, 0x33, 0x44]);
    assert_eq!(
        code(&nybble(&["detect", noise.to_str().unwrap()])),
        1,
        "an unrecognized file should exit 1"
    );
}

#[test]
fn strings_lists_readable_runs_at_their_file_offsets() {
    // Two words buried in binary noise, the second past a `--at` boundary, so
    // the offsets have to come back as file positions and not region ones.
    let mut bytes = vec![0x00u8; 16];
    bytes.extend_from_slice(b"version");
    bytes.extend_from_slice(&[0xFF; 8]);
    bytes.extend_from_slice(b"payload");
    let file = fixture("strings.bin", &bytes);
    let path = file.to_str().unwrap();

    let out = nybble(&["strings", path]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("0x00000010"), "{text}");
    assert!(text.contains("version"), "{text}");
    assert!(text.contains("payload"), "{text}");

    // Skipping the first run drops it from the listing but keeps the offsets.
    let out = nybble(&["strings", path, "--at", "0x17"]);
    let text = stdout(&out);
    assert!(!text.contains("version"), "{text}");
    assert!(text.contains("0x0000001f"), "{text}");

    // A floor above the longest run leaves nothing to report.
    assert_eq!(
        code(&nybble(&["strings", path, "--min", "20"])),
        1,
        "a file with no strings that long should exit 1"
    );
}

#[test]
fn entropy_separates_a_packed_region_from_a_flat_one() {
    // Half padding, half a spread of every byte value: the strip should be low
    // on the left and high on the right, and the peak should land in the noise.
    let mut bytes = vec![0x00u8; 4096];
    bytes.extend((0..4096).map(|i| (i * 7 % 256) as u8));
    let file = fixture("entropy.bin", &bytes);
    let path = file.to_str().unwrap();

    let out = nybble(&["entropy", path, "--buckets", "8"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("8 bucket(s) of 1024 byte(s)"), "{text}");
    assert!(text.contains("▁▁▁▁████"), "flat first, packed second: {text}");
    // Every bucket in the noise half scores 1.00, so only the half is pinned.
    assert!(text.contains("peak 1.00 at 0x1"), "{text}");

    let out = nybble(&["entropy", path, "--buckets", "4", "--json"]);
    let text = stdout(&out);
    let values: Vec<f32> = text
        .split("\"values\":[")
        .nth(1)
        .and_then(|t| t.split(']').next())
        .expect("json should carry the values")
        .split(',')
        .map(|v| v.parse().expect("each value should be a number"))
        .collect();
    assert_eq!(values.len(), 4);
    assert!(values[0] < 0.1, "padding is not random: {values:?}");
    assert!(values[3] > 0.9, "a full byte spread is: {values:?}");

    // Nothing to measure is a mismatch, not an error.
    let empty = fixture("entropy_empty.bin", &[]);
    assert_eq!(code(&nybble(&["entropy", empty.to_str().unwrap()])), 1);
}

#[test]
fn detect_only_needs_the_head_of_a_file() {
    // A tar's signature sits at offset 257 and nothing is anchored deeper, so
    // a file with megabytes of body after it must still be identified from the
    // first few hundred bytes alone.
    let mut tar = vec![0u8; 300];
    tar[257..262].copy_from_slice(b"ustar");
    tar.extend(std::iter::repeat_n(0xAB, 4 * 1024 * 1024));
    let file = fixture("detect_big.tar", &tar);
    let out = nybble(&["detect", file.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("TAR"), "{}", stdout(&out));
}

#[test]
fn usage_errors_are_distinguishable_from_mismatches() {
    // Exit 2 means "you held it wrong", so a script can tell a bad invocation
    // apart from a file that simply did not match.
    assert_eq!(code(&nybble(&["parse"])), 2, "missing arguments");
    assert_eq!(code(&nybble(&["wobble"])), 2, "unknown command");
    assert_eq!(code(&nybble(&[])), 2, "no command at all");
    assert_eq!(code(&nybble(&["--help"])), 0, "help is not an error");
}
