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
//! nybble strings firmware.bin --min 6          # readable text, with offsets
//! nybble entropy firmware.bin                   # where is the packed data?
//! nybble hints firmware.bin --at 0x4000           # what shape are these bytes?
//! nybble check my.schema                          # does the schema compile?
//! ```

use std::io::Read;
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
    nybble strings <file> [options]          list the readable text in a file
    nybble entropy <file> [options]          measure how random the bytes are
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

STRINGS OPTIONS:
    --min <n>        shortest run to report, in characters (default 4)
    --at <offset>    where to start looking (decimal or 0x…; default 0)
    --len <n>        how many bytes to look at (default: to the end)
    --limit <n>      how many strings to list (default: all)
    --json           emit the strings as JSON

ENTROPY OPTIONS:
    --buckets <n>    how many slices to measure (default 64)
    --at <offset>    where to start looking (decimal or 0x…; default 0)
    --len <n>        how many bytes to look at (default: to the end)
    --json           emit the per-bucket values as JSON

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
        "strings" => strings_cmd(rest),
        "entropy" => entropy_cmd(rest),
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
    min: Option<usize>,
    buckets: Option<usize>,
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
        min: None,
        buckets: None,
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
            "--min" => {
                let raw = value("--min")?;
                out.min = Some(
                    raw.parse()
                        .map_err(|_| Fail::Usage(format!("--min wants a number, got `{raw}`")))?,
                );
            }
            "--buckets" => {
                let raw = value("--buckets")?;
                out.buckets = Some(raw.parse().map_err(|_| {
                    Fail::Usage(format!("--buckets wants a number, got `{raw}`"))
                })?);
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

/// Read at most `limit` bytes from the front of a file.
///
/// For the questions that only look at a header, this is the difference between
/// touching a page and paging in a multi-gigabyte firmware image.
fn read_head(path: &str, limit: usize) -> Result<Vec<u8>, Fail> {
    let file = std::fs::File::open(path).map_err(|e| Fail::Error(format!("{path}: {e}")))?;
    let mut head = Vec::with_capacity(limit);
    file.take(limit as u64)
        .read_to_end(&mut head)
        .map_err(|e| Fail::Error(format!("{path}: {e}")))?;
    Ok(head)
}

/// The slice `--at` and `--len` select, plus the file offset it starts at. Both
/// bounds are clamped to the file, so pointing past the end yields an empty
/// region rather than an error — the answer to "what is out there?" is "nothing".
fn region<'a>(bytes: &'a [u8], args: &Args) -> (&'a [u8], u64) {
    let total = bytes.len() as u64;
    let at = args.at.unwrap_or(0).min(total);
    let end = args.len.map_or(total, |n| at.saturating_add(n).min(total));
    (&bytes[at as usize..end as usize], at)
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
    // Every signature is anchored near the start — the deepest one ends at 262 —
    // so there is no reason to pull a whole disk image through memory to answer.
    const HEAD: usize = 512;
    let bytes = read_head(path, HEAD)?;
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

// --- strings ----------------------------------------------------------------

fn strings_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [path] = args.positional.as_slice() else {
        return Err(Fail::Usage("strings takes one file".into()));
    };
    let bytes = read_file(path)?;
    let (region, at) = region(&bytes, &args);
    // Four characters is the same floor the app's string panel uses: short
    // enough to catch a four-letter tag, long enough that binary data does not
    // fill the list with accidents.
    let hits = analysis::find_strings(region, args.min.unwrap_or(4));
    let listed = args.limit.unwrap_or(hits.len()).min(hits.len());

    if args.json {
        // Offsets are relative to the region the scanner saw, so shift them back
        // to where they are in the file before anything downstream reads them.
        let out: Vec<_> = hits
            .iter()
            .take(listed)
            .map(|h| {
                serde_json::json!({
                    "offset": at + h.offset as u64,
                    "len": h.len,
                    "encoding": h.encoding,
                    "text": h.text,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string(&out).map_err(|e| Fail::Error(e.to_string()))?
        );
    } else if !args.quiet {
        if hits.is_empty() {
            println!("no strings in {} byte(s) at {at:#x}", region.len());
        }
        for hit in hits.iter().take(listed) {
            let encoding = match hit.encoding {
                analysis::Encoding::Ascii => "ascii",
                analysis::Encoding::Utf16Le => "utf16le",
            };
            println!("{:#010x} +{:<6} {encoding:<8} {}", at + hit.offset as u64, hit.len, hit.text);
        }
        if hits.len() > listed {
            println!("  … {} more string(s)", hits.len() - listed);
        }
    }
    if hits.is_empty() {
        Err(Fail::Mismatch)
    } else {
        Ok(())
    }
}

// --- entropy ----------------------------------------------------------------

/// The eight block glyphs, so a whole file's entropy fits on one terminal line.
const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn entropy_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [path] = args.positional.as_slice() else {
        return Err(Fail::Usage("entropy takes one file".into()));
    };
    let bytes = read_file(path)?;
    let (region, at) = region(&bytes, &args);
    let buckets = args.buckets.unwrap_or(64).max(1);
    let values = analysis::entropy(region, buckets);
    if values.is_empty() {
        if !args.quiet && !args.json {
            println!("nothing to measure at {at:#x}");
        }
        return Err(Fail::Mismatch);
    }
    // `entropy` returns fewer buckets than asked for when the region is shorter
    // than that, so the width each value covers comes from what came back.
    let width = region.len() / values.len();

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "offset": at,
                "len": region.len(),
                "bucket_bytes": width,
                "values": values,
            })
        );
        return Ok(());
    }
    if args.quiet {
        return Ok(());
    }

    let strip: String = values
        .iter()
        .map(|v| SPARK[((v * 8.0) as usize).min(7)])
        .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let (peak_index, peak) = values
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .expect("values is not empty");
    println!(
        "{at:#010x} +{} — {} bucket(s) of {width} byte(s)",
        region.len(),
        values.len()
    );
    println!("{strip}");
    println!(
        "mean {mean:.2}, peak {peak:.2} at {:#x}",
        at + (peak_index * width) as u64
    );
    // Entropy this close to 1.0 is the signature of data that is already
    // compressed or encrypted, which is usually the region worth opening first.
    if *peak > 0.95 {
        println!("the peak looks compressed or encrypted");
    }
    Ok(())
}

// --- hints ------------------------------------------------------------------

fn hints_cmd(args: &[String]) -> Result<(), Fail> {
    let args = parse_args(args)?;
    let [path] = args.positional.as_slice() else {
        return Err(Fail::Usage("hints takes one file".into()));
    };
    let bytes = read_file(path)?;
    let total = bytes.len() as u64;
    let (region, at) = region(&bytes, &args);
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
