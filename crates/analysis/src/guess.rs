//! "What could the bytes at this offset be?" — a set of labelled guesses that
//! go beyond raw type interpretation (plan §11).

use serde::Serialize;

use crate::dates::format_unix;
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

    // 64-bit Unix timestamps (seconds).
    if let Some(b) = rest.get(0..8) {
        let arr: [u8; 8] = b.try_into().unwrap();
        for (order, secs) in [
            ("u64 LE", u64::from_le_bytes(arr) as i64),
            ("u64 BE", u64::from_be_bytes(arr) as i64),
        ] {
            if (PLAUSIBLE_MIN..=PLAUSIBLE_MAX_64).contains(&secs) {
                guesses.push(Guess {
                    label: format!("Unix timestamp ({order}, seconds)"),
                    detail: format_unix(secs),
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
