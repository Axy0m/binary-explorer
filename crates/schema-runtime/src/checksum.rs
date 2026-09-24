//! Checksum algorithms behind the schema `check` clause.
//!
//! Each one takes the covered bytes and returns the value a correct file would
//! store, widened to `u64` so the runtime can compare it against a field of any
//! integer width. All of these are small and self-contained on purpose: the
//! point of the clause is to validate (and repair) the checksums that ordinary
//! binary formats carry, which are almost always CRC-32, Adler-32, or a plain
//! additive/XOR fold.

use schema::Checksum;

/// Compute `algo` over `bytes`.
pub fn compute(algo: Checksum, bytes: &[u8]) -> u64 {
    match algo {
        Checksum::Crc32 => crc32(bytes) as u64,
        Checksum::Adler32 => adler32(bytes) as u64,
        Checksum::Sum8 => sum(bytes) as u8 as u64,
        Checksum::Sum16 => sum(bytes) as u16 as u64,
        Checksum::Sum32 => sum(bytes) as u32 as u64,
        Checksum::Xor8 => bytes.iter().fold(0u8, |a, b| a ^ b) as u64,
    }
}

/// Byte sum, kept at 64 bits so the callers can truncate to their own width.
fn sum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |a, b| a.wrapping_add(*b as u64))
}

/// CRC-32 (IEEE 802.3, reflected, init and final xor `0xFFFFFFFF`) — the
/// variant PNG, ZIP, and gzip all use.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in bytes {
        crc = CRC32_TABLE[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// Adler-32 (RFC 1950) — the checksum in a zlib stream's trailer.
pub fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    // The largest run that cannot overflow `b` before the modulo is applied.
    const CHUNK: usize = 5552;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in bytes.chunks(CHUNK) {
        for byte in chunk {
            a += *byte as u32;
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

static CRC32_TABLE: [u32; 256] = crc32_table();

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values, each independently known rather than produced by this
    // code: "IEND" is the CRC every PNG on earth ends with, and the rest are the
    // standard published check values for their algorithms.
    #[test]
    fn crc32_matches_published_check_values() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }

    #[test]
    fn adler32_matches_published_check_values() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"a"), 0x0062_0062);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn adler32_survives_a_run_longer_than_one_chunk() {
        // Past 5552 bytes the accumulator has to be reduced; a version that
        // forgot to would silently drift here.
        let bytes = vec![0xFFu8; 20_000];
        // Computed by taking the definition at its word: a = 1 + 255*n, b is the
        // running total of a, both mod 65521.
        let mut a: u64 = 1;
        let mut b: u64 = 0;
        for byte in &bytes {
            a = (a + *byte as u64) % 65521;
            b = (b + a) % 65521;
        }
        assert_eq!(adler32(&bytes), ((b << 16) | a) as u32);
    }

    #[test]
    fn folds_truncate_to_their_width() {
        let bytes = [0xFFu8, 0xFF, 0x02];
        assert_eq!(compute(Checksum::Sum8, &bytes), 0x00);
        assert_eq!(compute(Checksum::Sum16, &bytes), 0x0200);
        assert_eq!(compute(Checksum::Sum32, &bytes), 0x0200);
        assert_eq!(compute(Checksum::Xor8, &bytes), 0x02);
        assert_eq!(compute(Checksum::Sum8, &[]), 0);
        assert_eq!(compute(Checksum::Xor8, &[]), 0);
    }
}
