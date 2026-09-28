//! Checksum algorithms behind the schema `check` clause.
//!
//! Each one takes the covered bytes and returns the value a correct file would
//! store, widened to `u64` so the runtime can compare it against a field of any
//! integer width. All of these are small and self-contained on purpose: the
//! point of the clause is to validate (and repair) the checksums that ordinary
//! binary formats carry, which are almost always a CRC (32- or 16-bit),
//! Adler-32, or a plain additive/XOR fold.

use schema::Checksum;

/// Compute `algo` over `bytes`.
pub fn compute(algo: Checksum, bytes: &[u8]) -> u64 {
    match algo {
        Checksum::Crc32 => crc32(bytes) as u64,
        Checksum::Adler32 => adler32(bytes) as u64,
        Checksum::Crc16 => crc16_reflected(bytes, 0x0000) as u64,
        Checksum::Crc16Modbus => crc16_reflected(bytes, 0xFFFF) as u64,
        Checksum::Crc16Ccitt => crc16_ccitt(bytes, 0xFFFF) as u64,
        Checksum::Crc16Xmodem => crc16_ccitt(bytes, 0x0000) as u64,
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

/// CRC-16 over the reflected `0x8005` polynomial. Init `0x0000` is the variant
/// catalogued as CRC-16/ARC and written as plain "CRC-16" nearly everywhere;
/// init `0xFFFF` is Modbus. Nothing else separates the two.
pub fn crc16_reflected(bytes: &[u8], init: u16) -> u16 {
    let mut crc = init;
    for b in bytes {
        crc = CRC16_REFLECTED_TABLE[((crc ^ *b as u16) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// CRC-16 over the un-reflected `0x1021` polynomial, most significant bit
/// first. Init `0xFFFF` is the variant datasheets call CCITT (catalogued as
/// CRC-16/IBM-3740, or "CCITT-FALSE"); init `0x0000` is XMODEM.
pub fn crc16_ccitt(bytes: &[u8], init: u16) -> u16 {
    let mut crc = init;
    for b in bytes {
        let index = (((crc >> 8) ^ *b as u16) & 0xFF) as usize;
        crc = CRC16_CCITT_TABLE[index] ^ (crc << 8);
    }
    crc
}

static CRC32_TABLE: [u32; 256] = crc32_table();
static CRC16_REFLECTED_TABLE: [u16; 256] = crc16_reflected_table();
static CRC16_CCITT_TABLE: [u16; 256] = crc16_ccitt_table();

const fn crc16_reflected_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u16;
        let mut bit = 0;
        while bit < 8 {
            // 0xA001 is 0x8005 with its bits reversed, which is what reflecting
            // the input lets us use.
            c = if c & 1 != 0 { 0xA001 ^ (c >> 1) } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const fn crc16_ccitt_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

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

    // Every CRC-16 in the catalogue is published with its check value: the CRC
    // of the ASCII digits "123456789". Four variants, four known answers — the
    // one test that catches a wrong polynomial, a missed reflection, or an init
    // copied from the neighbouring row of the table.
    #[test]
    fn crc16_variants_match_their_published_check_values() {
        assert_eq!(compute(Checksum::Crc16, b"123456789"), 0xBB3D);
        assert_eq!(compute(Checksum::Crc16Modbus, b"123456789"), 0x4B37);
        assert_eq!(compute(Checksum::Crc16Ccitt, b"123456789"), 0x29B1);
        assert_eq!(compute(Checksum::Crc16Xmodem, b"123456789"), 0x31C3);
    }

    #[test]
    fn crc16_of_nothing_is_the_initial_value() {
        assert_eq!(compute(Checksum::Crc16, b""), 0x0000);
        assert_eq!(compute(Checksum::Crc16Modbus, b""), 0xFFFF);
        assert_eq!(compute(Checksum::Crc16Ccitt, b""), 0xFFFF);
        assert_eq!(compute(Checksum::Crc16Xmodem, b""), 0x0000);
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
