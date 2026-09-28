//! Minimal, dependency-free conversion from a Unix timestamp to a UTC date
//! string, so timestamp guesses can show a human date without pulling in a
//! calendar crate. Uses Howard Hinnant's `civil_from_days` algorithm.

/// Format a Unix timestamp (seconds since 1970-01-01 UTC) as
/// `"YYYY-MM-DD HH:MM:SS UTC"`.
pub fn format_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// Format an MS-DOS packed date/time as `"YYYY-MM-DD HH:MM:SS"`, or `None` when
/// the bit fields do not spell a real date. The low half is the time (hour,
/// minute, and seconds halved into five bits), the high half the date, with the
/// year counted from 1980 — the stamp every ZIP entry and FAT directory entry
/// carries.
///
/// This is the loosest of the date readings by some way: the fields are narrow,
/// so a fair share of arbitrary words decode to *some* valid date. The year
/// window is what keeps it from firing on nearly anything, and it is still a
/// guess to be weighed rather than an identification.
pub fn format_dos(packed: u32) -> Option<String> {
    let (time, date) = (packed & 0xFFFF, packed >> 16);
    let year = 1980 + (date >> 9);
    let month = (date >> 5) & 0xF;
    let day = date & 0x1F;
    let hour = time >> 11;
    let minute = (time >> 5) & 0x3F;
    let second = (time & 0x1F) * 2;
    let plausible = (1990..=2040).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 58;
    plausible
        .then(|| format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"))
}

/// Convert a count of days since the Unix epoch to (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}
