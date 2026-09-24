//! Diff two files from the command line.
//!
//! ```sh
//! cargo run -p diff --example diff -- <a> <b> [max-regions]
//! ```
//!
//! The same aligned comparison the desktop app's compare mode runs, without the
//! UI — handy for scripting, and for checking what the app should be showing.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: diff <file-a> <file-b> [max-regions]");
        return ExitCode::from(2);
    }
    let limit: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let a = match std::fs::read(&args[0]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", args[0]);
            return ExitCode::FAILURE;
        }
    };
    let b = match std::fs::read(&args[1]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", args[1]);
            return ExitCode::FAILURE;
        }
    };

    let d = diff::compare(&a, &b);
    println!("a: {} ({} bytes)", args[0], d.a_len());
    println!("b: {} ({} bytes)", args[1], d.b_len());

    if d.is_identical() {
        println!("identical");
        return ExitCode::SUCCESS;
    }

    println!(
        "{} byte(s) differ in {} region(s){}",
        d.changed_bytes(),
        d.runs().len(),
        if d.truncated() { " (list capped)" } else { "" }
    );
    if d.a_len() != d.b_len() {
        let (longer, delta) = if d.a_len() > d.b_len() {
            ("a", d.a_len() - d.b_len())
        } else {
            ("b", d.b_len() - d.a_len())
        };
        println!("{longer} is {delta} byte(s) longer; compared the first {} byte(s)", d.common_len());
    }

    for run in d.runs().iter().take(limit) {
        let from = run.offset as usize;
        let to = (run.end() as usize).min(from + 16);
        println!(
            "  {:#010x} +{:<6} {} -> {}",
            run.offset,
            run.len,
            hex(&b[from..to]),
            hex(&a[from..to]),
        );
    }
    if d.runs().len() > limit {
        println!("  … {} more region(s)", d.runs().len() - limit);
    }
    ExitCode::SUCCESS
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
