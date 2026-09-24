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
fn usage_errors_are_distinguishable_from_mismatches() {
    // Exit 2 means "you held it wrong", so a script can tell a bad invocation
    // apart from a file that simply did not match.
    assert_eq!(code(&nybble(&["parse"])), 2, "missing arguments");
    assert_eq!(code(&nybble(&["wobble"])), 2, "unknown command");
    assert_eq!(code(&nybble(&[])), 2, "no command at all");
    assert_eq!(code(&nybble(&["--help"])), 0, "help is not an error");
}
