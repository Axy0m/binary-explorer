//! `nybble` — the engine on the command line.
//!
//! The desktop app is for exploring one file interactively. This is for the
//! other half of the job: running a finished schema over a hundred files,
//! diffing two captures in a script, or asserting in CI that a format still
//! parses. Same crates, same results, no UI.
//!
//! ```sh
//! nybble parse schemas/png.schema shot.png        # tree, entry from @entry
//! nybble parse schemas/png.schema shot.png --json # pipe it into jq
//! nybble diff before.sav after.sav                # changed regions
//! nybble detect firmware.bin                      # what is this?
//! nybble hints firmware.bin --at 0x4000           # what shape are these bytes?
//! nybble check my.schema                          # does the schema compile?
//! ```

use std::process::ExitCode;

use binary_reader::{BinaryReader, Endian};
use schema_runtime::{CheckResult, FieldNode};

mod render;

const USAGE: &str = "\
nybble — inspect binary files with a schema

USAGE:
    nybble parse <schema> <file> [options]   run a schema over a file
    nybble diff <a> <b> [options]            compare two files, byte-aligned
    nybble detect <file>                     identify a format from its magic bytes
    nybble hints <file> [options]            guess the shape of a region of bytes
    nybble check <schema>                    validate that a schema compiles

PARSE OPTIONS:
    --entry <name>   entry struct (default: the schema's @entry, else the first)
    --endian le|be   byte order (default: the schema's @endian, else le)
    --json           emit the field tree as JSON
    --quiet          print nothing; report the outcome as the exit status

DIFF OPTIONS:
    --limit <n>      how many changed regions to list (default 20)
    --json           emit the summary and regions as JSON

HINTS OPTIONS:
    --at <offset>    where to start looking (decimal or 0x…; default 0)
    --len <n>        how many bytes to look at (default: to the end)
    --json           emit the hints as JSON

Exit status is 0 on success, 1 when a file does not match (a parse fault, a
difference, a schema that does not compile), and 2 for a usage error.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &args[1..];

    let result = match command.as_str() {
        "parse" => parse_cmd(rest),
        "diff" => diff_cmd(rest),
        "detect" => detect_cmd(rest),
        "hints" => hints_cmd(rest),
        "check" => check_cmd(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        "-V" | "--version" | "version" => {
            println!("nybble {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        other => Err(Fail::Usage(format!("unknown command `{other}`"))),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail::Mismatch) => ExitCode::FAILURE,
        Err(Fail::Error(msg)) => {
            eprintln!("nybble: {msg}");
            ExitCode::FAILURE
        }
        Err(Fail::Usage(msg)) => {
            eprintln!("nybble: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// How a command ended. `Mismatch` has already printed whatever it had to say —
/// it means "this file did not match", not "something went wrong".
enum Fail {
    Mismatch,
    Error(String),
    Usage(String),
}

/// A parsed argument list: positionals in order, plus the flags that were set.
struct Args {
    positional: Vec<String>,
    json: bool,
    quiet: bool,
    entry: Option<String>,
    endian: Option<String>,
    limit: Option<usize>,
    at: Option<u64>,
    len: Option<u64>,
}

fn parse_args(args: &[String]) -> Result<Args, Fail> {
    let mut out = Args {
        positional: Vec::new(),
        json: false,
        quiet: false,
        entry: None,
        endian: None,
        limit: None,
        at: None,
        len: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        // A flag that takes a value consumes the next argument, so a missing
        // value is reported here rather than silently swallowing a file name.
        let mut value = |name: &str| -> Result<String, Fail> {
            it.next()
                .cloned()
                .ok_or_else(|| Fail::Usage(format!("{name} needs a value")))
        };
        match arg.as_str() {
            "--json" => out.json = true,
            "--quiet" | "-q" => out.quiet = true,
            "--entry" => out.entry = Some(value("--entry")?),
            "--endian" => out.endian = Some(value("--endian")?),
            "--limit" => {
                let raw = value("--limit")?;
                out.limit = Some(
                    raw.parse()
                        .map_err(|_| Fail::Usage(format!("--limit wants a number, got `{raw}`")))?,
                );
            }
            "--at" => out.at = Some(number(&value("--at")?, "--at")?),
            "--len" => out.len = Some(number(&value("--len")?, "--len")?),
            other if other.starts_with('-') && other != "-" => {
                return Err(Fail::Usage(format!("unknown option `{other}`")));
            }
            other => out.positional.push(other.to_string()),
        }
    }
    Ok(out)
}

/// Accept an offset the way a person writes one: `4096` or `0x1000`.
fn number(raw: &str, flag: &str) -> Result<u64, Fail> {
    let text = raw.trim();
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| Fail::Usage(format!("{flag} wants a number, got `{raw}`")))
}

fn read_file(path: &str) -> Result<Vec<u8>, Fail> {
    std::fs::read(path).map_err(|e| Fail::Error(format!("{path}: {e}")))
}

// --- parse ------------------------------------------------------------------

fn parse_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [schema_path, file_path] = args.positional.as_slice() else {
        return Err(Fail::Usage("parse takes <schema> and <file>".into()));
    };

    let text = std::fs::read_to_string(schema_path)
        .map_err(|e| Fail::Error(format!("{schema_path}: {e}")))?;
    let schema = schema_parser::parse(&text).map_err(|e| Fail::Error(e.to_string()))?;

    // A schema carries its own entry point and byte order in its `// @` header,
    // so the common case needs no flags at all.
    let meta = schema_library::parse_metadata(&text);
    let entry = args
        .entry
        .filter(|e| !e.trim().is_empty())
        .or_else(|| Some(meta.entry.clone()).filter(|e| !e.is_empty()))
        .or_else(|| schema.structs.first().map(|s| s.name.clone()))
        .ok_or_else(|| Fail::Error("schema defines no structs".into()))?;
    let endian = match args.endian.unwrap_or(meta.endian).to_ascii_lowercase().as_str() {
        "be" | "big" => Endian::Big,
        "le" | "little" | "" => Endian::Little,
        other => return Err(Fail::Usage(format!("--endian wants le or be, got `{other}`"))),
    };

    let reader = BinaryReader::open(file_path).map_err(|e| Fail::Error(e.to_string()))?;
    let outcome = schema_runtime::parse_partial(&schema, &reader, &entry, endian)
        .map_err(|e| Fail::Error(e.to_string()))?;

    if !args.quiet {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&outcome)
                    .map_err(|e| Fail::Error(e.to_string()))?
            );
        } else {
            print!("{}", render::tree(&outcome.tree));
            // What the schema did not explain is the useful half of the answer
            // while a format is still being worked out.
            let c = &outcome.coverage;
            if c.gaps.is_empty() {
                println!("
{} of {} bytes explained (all of it)", c.covered, c.total);
            } else {
                println!(
                    "
{} of {} bytes explained ({:.0}%) — {} unexplained region(s), first at {:#x}",
                    c.covered,
                    c.total,
                    c.fraction() * 100.0,
                    c.gaps.len(),
                    c.gaps[0].offset,
                );
            }
        }
    }

    // A fault, or a checksum that no longer matches, means the file did not
    // match the schema — worth an exit status a script can branch on.
    let bad_checks = count_bad_checks(&outcome.tree);
    if let Some(fault) = &outcome.fault {
        if !args.quiet && !args.json {
            eprintln!(
                "fault at {}{:#x} in {} — {}",
                if fault.decoded { "+" } else { "" },
                fault.offset,
                fault.path,
                fault.message
            );
        }
        return Err(Fail::Mismatch);
    }
    if bad_checks > 0 {
        if !args.quiet && !args.json {
            eprintln!("{bad_checks} checksum(s) do not match");
        }
        return Err(Fail::Mismatch);
    }
    Ok(())
}

fn count_bad_checks(node: &FieldNode) -> usize {
    let mine = usize::from(matches!(&node.check, Some(CheckResult { ok: false, .. })));
    mine + node.children.iter().map(count_bad_checks).sum::<usize>()
}

// --- diff -------------------------------------------------------------------

fn diff_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [a_path, b_path] = args.positional.as_slice() else {
        return Err(Fail::Usage("diff takes two files".into()));
    };
    let limit = args.limit.unwrap_or(20);
    let a = read_file(a_path)?;
    let b = read_file(b_path)?;
    let d = diff::compare(&a, &b);

    if args.json {
        let regions: Vec<_> = d
            .runs()
            .iter()
            .take(limit)
            .map(|r| serde_json::json!({ "offset": r.offset, "len": r.len }))
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "a": { "path": a_path, "len": d.a_len() },
                "b": { "path": b_path, "len": d.b_len() },
                "identical": d.is_identical(),
                "changed_bytes": d.changed_bytes(),
                "regions": d.runs().len(),
                "truncated": d.truncated(),
                "listed": regions,
            })
        );
    } else if !args.quiet {
        println!("a: {a_path} ({} bytes)", d.a_len());
        println!("b: {b_path} ({} bytes)", d.b_len());
        if d.is_identical() {
            println!("identical");
        } else {
            println!(
                "{} byte(s) differ in {} region(s){}",
                d.changed_bytes(),
                d.runs().len(),
                if d.truncated() { " (list capped)" } else { "" }
            );
            if d.a_len() != d.b_len() {
                let (longer, delta) = if d.a_len() > d.b_len() {
                    (a_path, d.a_len() - d.b_len())
                } else {
                    (b_path, d.b_len() - d.a_len())
                };
                println!(
                    "{longer} is {delta} byte(s) longer; compared the first {}",
                    d.common_len()
                );
            }
            for run in d.runs().iter().take(limit) {
                let from = run.offset as usize;
                let to = (run.end() as usize).min(from + 16);
                println!(
                    "  {:#010x} +{:<6} {} -> {}",
                    run.offset,
                    run.len,
                    render::hex(&b[from..to]),
                    render::hex(&a[from..to]),
                );
            }
            if d.runs().len() > limit {
                println!("  … {} more region(s)", d.runs().len() - limit);
            }
        }
    }

    if d.is_identical() {
        Ok(())
    } else {
        Err(Fail::Mismatch)
    }
}

// --- detect -----------------------------------------------------------------

fn detect_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [path] = args.positional.as_slice() else {
        return Err(Fail::Usage("detect takes one file".into()));
    };
    // Only the head is needed: every signature is anchored near the start.
    let bytes = read_file(path)?;
    let hits = format_detection::detect(&bytes);

    if args.json {
        println!(
            "{}",
            serde_json::to_string(&hits).map_err(|e| Fail::Error(e.to_string()))?
        );
    } else if !args.quiet {
        if hits.is_empty() {
            println!("unknown format");
        }
        for hit in &hits {
            println!(
                "{:<8} {:>3}%  {}  ({})",
                hit.format, hit.confidence, hit.description, hit.extension
            );
        }
    }
    if hits.is_empty() {
        Err(Fail::Mismatch)
    } else {
        Ok(())
    }
}

// --- hints ------------------------------------------------------------------

fn hints_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [path] = args.positional.as_slice() else {
        return Err(Fail::Usage("hints takes one file".into()));
    };
    let bytes = read_file(path)?;
    let total = bytes.len() as u64;
    let at = args.at.unwrap_or(0).min(total);
    let end = args
        .len
        .map(|n| (at + n).min(total))
        .unwrap_or(total);
    let region = &bytes[at as usize..end as usize];
    let hints = analysis::infer(region, at, total);

    if args.json {
        println!(
            "{}",
            serde_json::to_string(&hints).map_err(|e| Fail::Error(e.to_string()))?
        );
    } else if !args.quiet {
        if hints.is_empty() {
            println!("no shape found in {} byte(s) at {at:#x}", region.len());
        }
        for hint in &hints {
            println!("{:#010x} +{:<8} {:<12} {}", hint.offset, hint.len, hint.label, hint.detail);
        }
    }
    if hints.is_empty() {
        Err(Fail::Mismatch)
    } else {
        Ok(())
    }
}

// --- check ------------------------------------------------------------------

fn check_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [path] = args.positional.as_slice() else {
        return Err(Fail::Usage("check takes one schema".into()));
    };
    let text =
        std::fs::read_to_string(path).map_err(|e| Fail::Error(format!("{path}: {e}")))?;
    match schema_parser::parse(&text) {
        Ok(schema) => {
            if !args.quiet {
                let meta = schema_library::parse_metadata(&text);
                let entry = if meta.entry.is_empty() {
                    schema
                        .structs
                        .first()
                        .map(|s| s.name.as_str())
                        .unwrap_or("(none)")
                } else {
                    &meta.entry
                };
                println!(
                    "ok — {} struct(s), {} enum(s), {} bitfield(s); entry {entry}",
                    schema.structs.len(),
                    schema.enums.len(),
                    schema.bitfields.len(),
                );
            }
            Ok(())
        }
        Err(e) => {
            if !args.quiet {
                eprintln!("{path}: {e}");
            }
            Err(Fail::Mismatch)
        }
    }
}
