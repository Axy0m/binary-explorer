//! "What could the bytes at this offset be?" — a set of labelled guesses that
//! go beyond raw type interpretation (plan §11).

use serde::Serialize;

use crate::dates::{format_dos, format_unix};
use crate::strings::is_printable_run;

/// A single semantic guess about the bytes at an offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Guess {
    /// Short label, e.g. `"Unix timestamp (u32 LE)"`.
    pub label: String,
    /// Decoded detail, e.g. the formatted date or the string preview.
    pub detail: String,
}

// A Unix time is "plausible" if it lands roughly between 2001 and 2038 for
// 32-bit, widened for 64-bit — enough to flag real timestamps without labelling
// every small integer as a date.
const PLAUSIBLE_MIN: i64 = 1_000_000_000; // 2001-09-09
const PLAUSIBLE_MAX_32: i64 = 2_147_483_647; // i32::MAX (2038)
const PLAUSIBLE_MAX_64: i64 = 4_102_444_800; // 2100-01-01

// A Windows FILETIME counts 100-nanosecond ticks from 1601-01-01 UTC. It is the
// stamp on every NTFS entry, PE debug directory, registry hive, and event log
// record, so a 64-bit number in a Windows-shaped file is more often one of
// these than a Unix time.
const FILETIME_TICKS_PER_SECOND: u64 = 10_000_000;
const FILETIME_EPOCH_TO_UNIX: i64 = 11_644_473_600; // seconds from 1601 to 1970

/// Produce guesses for the bytes starting at `offset`.
pub fn analyze_at(bytes: &[u8], offset: usize) -> Vec<Guess> {
    let mut guesses = Vec::new();
    let rest = match bytes.get(offset..) {
        Some(r) => r,
        None => return guesses,
    };

    // A readable string starting right here.
    if let Some(text) = is_printable_run(rest, 4) {
        guesses.push(Guess {
            label: format!("ASCII string ({} chars)", text.chars().count()),
            detail: format!("{text:?}"),
        });
    }

    // 32-bit Unix timestamps, both byte orders.
    if let Some(b) = rest.get(0..4) {
        let arr = [b[0], b[1], b[2], b[3]];
        for (order, secs) in [
            ("u32 LE", u32::from_le_bytes(arr) as i64),
            ("u32 BE", u32::from_be_bytes(arr) as i64),
        ] {
            if (PLAUSIBLE_MIN..=PLAUSIBLE_MAX_32).contains(&secs) {
                guesses.push(Guess {
                    label: format!("Unix timestamp ({order})"),
                    detail: format_unix(secs),
                });
            }
        }
    }

    // An MS-DOS packed date/time, as carried by ZIP entries and FAT directories.
    if let Some(b) = rest.get(0..4) {
        let arr = [b[0], b[1], b[2], b[3]];
        if let Some(date) = format_dos(u32::from_le_bytes(arr)) {
            guesses.push(Guess {
                label: "MS-DOS date/time (u32 LE)".into(),
                detail: date,
            });
        }
    }

    // 64-bit numbers that are a time in one of the three common encodings:
    // seconds, milliseconds, or Windows' 100-nanosecond ticks from 1601.
    if let Some(b) = rest.get(0..8) {
        let arr: [u8; 8] = b.try_into().unwrap();
        for (order, raw) in [
            ("u64 LE", u64::from_le_bytes(arr)),
            ("u64 BE", u64::from_be_bytes(arr)),
        ] {
            let readings = [
                ("seconds", raw as i64),
                ("milliseconds", (raw / 1_000) as i64),
            ];
            for (unit, secs) in readings {
                if (PLAUSIBLE_MIN..=PLAUSIBLE_MAX_64).contains(&secs) {
                    guesses.push(Guess {
                        label: format!("Unix timestamp ({order}, {unit})"),
                        detail: format_unix(secs),
                    });
                }
            }
            let filetime = (raw / FILETIME_TICKS_PER_SECOND) as i64 - FILETIME_EPOCH_TO_UNIX;
            if (PLAUSIBLE_MIN..=PLAUSIBLE_MAX_64).contains(&filetime) {
                guesses.push(Guess {
                    label: format!("Windows FILETIME ({order})"),
                    detail: format_unix(filetime),
                });
            }
        }
    }

    // A 16-byte UUID.
    if let Some(b) = rest.get(0..16) {
        // Only flag it when the version nibble is 1-5, which rules out most
        // random data and all-zero padding.
        let version = b[6] >> 4;
        if (1..=5).contains(&version) {
            guesses.push(Guess {
                label: format!("UUID (v{version})"),
                detail: format_uuid(b.try_into().unwrap()),
            });
        }
    }

    guesses
}

fn format_uuid(b: [u8; 16]) -> String {
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
