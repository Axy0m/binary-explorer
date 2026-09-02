//! Tests for signature-based format detection.

use format_detection::detect;

#[test]
fn detects_png() {
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
    let hits = detect(&png);
    assert_eq!(hits[0].format, "PNG");
    assert_eq!(hits[0].confidence, 100);
    assert_eq!(hits[0].extension, "png");
}

#[test]
fn detects_elf() {
    let elf = [0x7F, b'E', b'L', b'F', 2, 1, 1, 0];
    assert_eq!(detect(&elf)[0].format, "ELF");
}

#[test]
fn detects_pdf_and_gif_and_sqlite() {
    assert_eq!(detect(b"%PDF-1.7\n...")[0].format, "PDF");
    assert_eq!(detect(b"GIF89a")[0].format, "GIF");
    assert_eq!(detect(b"SQLite format 3\0rest")[0].format, "SQLite");
}

#[test]
fn wav_needs_both_riff_and_wave_parts() {
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&[0x24, 0x08, 0x00, 0x00]); // chunk size
    wav.extend_from_slice(b"WAVE");
    assert_eq!(detect(&wav)[0].format, "WAV");

    // RIFF without WAVE at offset 8 should not report WAV.
    let mut riff_only = Vec::new();
    riff_only.extend_from_slice(b"RIFF");
    riff_only.extend_from_slice(&[0, 0, 0, 0]);
    riff_only.extend_from_slice(b"XXXX");
    assert!(detect(&riff_only).iter().all(|d| d.format != "WAV"));
}

#[test]
fn tar_signature_is_deep_at_offset_257() {
    let mut tar = vec![0u8; 300];
    tar[257..262].copy_from_slice(b"ustar");
    assert_eq!(detect(&tar)[0].format, "TAR");
}

#[test]
fn unknown_bytes_detect_nothing() {
    let noise = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    assert!(detect(&noise).is_empty());
}

#[test]
fn short_input_does_not_panic() {
    assert!(detect(&[]).is_empty());
    assert!(detect(&[0x89]).is_empty()); // truncated PNG magic
    let _ = detect(&[0xFF, 0xD8, 0xFF]); // exactly JPEG length
}

#[test]
fn jpeg_minimal_magic() {
    assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0])[0].format, "JPEG");
}

#[test]
fn results_sorted_by_confidence() {
    // A ZIP header (0x50 0x4B ...) — confidence 90, single match.
    let zip = [0x50, 0x4B, 0x03, 0x04, 0, 0];
    let hits = detect(&zip);
    assert_eq!(hits[0].format, "ZIP");
    // Confidences should be non-increasing.
    for w in hits.windows(2) {
        assert!(w[0].confidence >= w[1].confidence);
    }
}
