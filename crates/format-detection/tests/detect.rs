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
fn detects_the_shapes_firmware_arrives_in() {
    assert_eq!(detect(b"hsqs____")[0].format, "SquashFS");
    assert_eq!(detect(b"sqsh____")[0].format, "SquashFS");
    assert_eq!(detect(&[0x27, 0x05, 0x19, 0x56, 0, 0, 0, 0])[0].format, "uImage");
    assert_eq!(detect(&[0x0A, 0x0D, 0x0D, 0x0A, 0, 0, 0, 0])[0].format, "PCAPNG");
    assert_eq!(detect(b"dex\n035\0")[0].format, "DEX");
}

#[test]
fn detects_the_modern_compressors() {
    assert_eq!(detect(&[0x28, 0xB5, 0x2F, 0xFD, 0, 0])[0].format, "ZSTD");
    assert_eq!(detect(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00])[0].format, "XZ");
    assert_eq!(detect(&[0x04, 0x22, 0x4D, 0x18, 0, 0])[0].format, "LZ4");
    assert_eq!(detect(b"BZh91AY&SY")[0].format, "BZIP2");
}

#[test]
fn an_iso_media_file_is_recognized_by_ftyp_past_its_length() {
    // The first four bytes are the box length, so the signature has to be
    // anchored at 4 rather than 0.
    let mut mp4 = vec![0x00, 0x00, 0x00, 0x20];
    mp4.extend_from_slice(b"ftypisom");
    assert_eq!(detect(&mp4)[0].format, "MP4");
    // The same bytes without `ftyp` where it belongs are not an MP4.
    assert!(detect(&[0x00, 0x00, 0x00, 0x20, 0, 0, 0, 0]).is_empty());
}

#[test]
fn a_cafebabe_file_reports_both_things_it_could_be() {
    // A Mach-O universal binary and a Java class file share these four bytes
    // exactly. Claiming either one outright would be a guess, so both are
    // listed and neither is confident.
    let hits = detect(&[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 2]);
    let names: Vec<&str> = hits.iter().map(|h| h.format).collect();
    assert!(names.contains(&"Mach-O fat"), "{names:?}");
    assert!(names.contains(&"Java class"), "{names:?}");
    assert!(hits.iter().all(|h| h.confidence < 80), "neither is certain: {hits:?}");
}

#[test]
fn a_riff_container_is_told_apart_by_its_fourth_word() {
    let riff = |kind: &[u8]| {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0x24, 0x08, 0, 0]);
        bytes.extend_from_slice(kind);
        bytes
    };
    assert_eq!(detect(&riff(b"WEBP"))[0].format, "WebP");
    assert_eq!(detect(&riff(b"WAVE"))[0].format, "WAV");
    assert_eq!(detect(&riff(b"AVI "))[0].format, "AVI");
}

#[test]
fn tiff_is_recognized_in_both_byte_orders() {
    let le = detect(&[0x49, 0x49, 0x2A, 0x00, 8, 0, 0, 0]);
    let be = detect(&[0x4D, 0x4D, 0x00, 0x2A, 0, 0, 0, 8]);
    assert_eq!(le[0].format, "TIFF");
    assert_eq!(be[0].format, "TIFF");
    assert!(le[0].description.contains("little"), "{}", le[0].description);
    assert!(be[0].description.contains("big"), "{}", be[0].description);
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
