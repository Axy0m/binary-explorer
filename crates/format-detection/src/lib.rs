//! Format detection by signature (plan §13).
//!
//! A registry of known byte signatures ("magic numbers"). Given the head of a
//! file, [`detect`] returns the formats whose signatures match, most-confident
//! first. It does **not** parse the file — it only recognizes it, so the UI can
//! say "this looks like a PNG" and (later) auto-load a matching schema.
//!
//! ```
//! let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
//! let hits = format_detection::detect(&png);
//! assert_eq!(hits[0].format, "PNG");
//! ```

use serde::Serialize;

/// One recognized format and how sure we are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detection {
    /// Short format name, e.g. `"PNG"`.
    pub format: &'static str,
    /// Usual file extension without the dot, e.g. `"png"`.
    pub extension: &'static str,
    /// One-line human description.
    pub description: &'static str,
    /// Confidence 0-100. Longer / more specific signatures score higher.
    pub confidence: u8,
}

/// A signature: every `(offset, bytes)` part must match for it to fire.
/// Multiple parts let us express e.g. RIFF containers (`RIFF....WAVE`).
struct Signature {
    format: &'static str,
    extension: &'static str,
    description: &'static str,
    confidence: u8,
    parts: &'static [(usize, &'static [u8])],
}

/// The signature registry. Kept to signatures that are exact and anchored: a
/// magic number at a known offset, never a heuristic over the body. A format
/// whose only tell is statistical belongs in the analysis crate, not here.
const SIGNATURES: &[Signature] = &[
    Signature {
        format: "PNG",
        extension: "png",
        description: "Portable Network Graphics image",
        confidence: 100,
        parts: &[(0, &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])],
    },
    Signature {
        format: "JPEG",
        extension: "jpg",
        description: "JPEG image",
        confidence: 95,
        parts: &[(0, &[0xFF, 0xD8, 0xFF])],
    },
    Signature {
        format: "GIF",
        extension: "gif",
        description: "GIF image",
        confidence: 100,
        parts: &[(0, b"GIF8")],
    },
    Signature {
        format: "BMP",
        extension: "bmp",
        description: "Windows bitmap image",
        confidence: 80,
        parts: &[(0, b"BM")],
    },
    Signature {
        format: "PDF",
        extension: "pdf",
        description: "Portable Document Format",
        confidence: 100,
        parts: &[(0, b"%PDF-")],
    },
    Signature {
        format: "ELF",
        extension: "elf",
        description: "ELF executable / object (Unix)",
        confidence: 100,
        parts: &[(0, &[0x7F, 0x45, 0x4C, 0x46])],
    },
    Signature {
        format: "PE",
        extension: "exe",
        description: "DOS/Windows executable (MZ header)",
        confidence: 70,
        parts: &[(0, b"MZ")],
    },
    Signature {
        format: "Mach-O",
        extension: "dylib",
        description: "Mach-O executable (64-bit, macOS/iOS)",
        confidence: 100,
        // 0xFEEDFACF stored little-endian.
        parts: &[(0, &[0xCF, 0xFA, 0xED, 0xFE])],
    },
    Signature {
        format: "Mach-O",
        extension: "dylib",
        description: "Mach-O executable (32-bit, macOS/iOS)",
        confidence: 100,
        // 0xFEEDFACE stored little-endian.
        parts: &[(0, &[0xCE, 0xFA, 0xED, 0xFE])],
    },
    Signature {
        format: "Mach-O fat",
        extension: "dylib",
        // Byte-for-byte the same magic as a Java class file; both are reported
        // and neither claims certainty, because at four bytes there is nothing
        // to tell them apart. The next word decides: a slice count in one, a
        // class-file version in the other.
        description: "Mach-O universal binary — or a Java class (same magic)",
        confidence: 60,
        parts: &[(0, &[0xCA, 0xFE, 0xBA, 0xBE])],
    },
    Signature {
        format: "PCAPNG",
        extension: "pcapng",
        description: "pcapng packet capture (Section Header Block)",
        confidence: 100,
        parts: &[(0, &[0x0A, 0x0D, 0x0D, 0x0A])],
    },
    Signature {
        format: "PCAP",
        extension: "pcap",
        description: "libpcap / tcpdump packet capture",
        confidence: 100,
        // 0xA1B2C3D4 stored little-endian (microsecond timestamps).
        parts: &[(0, &[0xD4, 0xC3, 0xB2, 0xA1])],
    },
    Signature {
        format: "ZIP",
        extension: "zip",
        description: "ZIP archive (also .jar/.docx/.xlsx/.apk)",
        confidence: 90,
        parts: &[(0, &[0x50, 0x4B, 0x03, 0x04])],
    },
    Signature {
        format: "GZIP",
        extension: "gz",
        description: "gzip-compressed data",
        confidence: 95,
        parts: &[(0, &[0x1F, 0x8B])],
    },
    Signature {
        format: "XZ",
        extension: "xz",
        description: "xz-compressed data",
        confidence: 100,
        parts: &[(0, &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00])],
    },
    Signature {
        format: "BZIP2",
        extension: "bz2",
        description: "bzip2-compressed data",
        confidence: 85,
        parts: &[(0, b"BZh")],
    },
    Signature {
        format: "ZSTD",
        extension: "zst",
        description: "Zstandard-compressed data",
        confidence: 100,
        // 0xFD2FB528 stored little-endian.
        parts: &[(0, &[0x28, 0xB5, 0x2F, 0xFD])],
    },
    Signature {
        format: "LZ4",
        extension: "lz4",
        description: "LZ4 frame",
        confidence: 95,
        // 0x184D2204 stored little-endian.
        parts: &[(0, &[0x04, 0x22, 0x4D, 0x18])],
    },
    Signature {
        format: "CAB",
        extension: "cab",
        description: "Microsoft Cabinet archive",
        confidence: 95,
        parts: &[(0, b"MSCF")],
    },
    Signature {
        format: "7-Zip",
        extension: "7z",
        description: "7-Zip archive",
        confidence: 100,
        parts: &[(0, &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])],
    },
    Signature {
        format: "RAR",
        extension: "rar",
        description: "RAR archive",
        confidence: 100,
        parts: &[(0, &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07])],
    },
    Signature {
        format: "TAR",
        extension: "tar",
        description: "POSIX tar archive",
        confidence: 90,
        parts: &[(257, b"ustar")],
    },
    Signature {
        format: "WAV",
        extension: "wav",
        description: "WAVE audio (RIFF)",
        confidence: 100,
        parts: &[(0, b"RIFF"), (8, b"WAVE")],
    },
    Signature {
        format: "AVI",
        extension: "avi",
        description: "AVI video (RIFF)",
        confidence: 100,
        parts: &[(0, b"RIFF"), (8, b"AVI ")],
    },
    Signature {
        format: "WebP",
        extension: "webp",
        description: "WebP image (RIFF)",
        confidence: 100,
        parts: &[(0, b"RIFF"), (8, b"WEBP")],
    },
    Signature {
        format: "MP4",
        extension: "mp4",
        description: "ISO base media file (MP4/MOV/HEIF)",
        confidence: 90,
        // The box length comes first; `ftyp` is what actually identifies it.
        parts: &[(4, b"ftyp")],
    },
    Signature {
        format: "Matroska",
        extension: "mkv",
        description: "Matroska / WebM container (EBML)",
        confidence: 90,
        parts: &[(0, &[0x1A, 0x45, 0xDF, 0xA3])],
    },
    Signature {
        format: "TIFF",
        extension: "tif",
        description: "TIFF image (little-endian)",
        confidence: 90,
        parts: &[(0, &[0x49, 0x49, 0x2A, 0x00])],
    },
    Signature {
        format: "TIFF",
        extension: "tif",
        description: "TIFF image (big-endian)",
        confidence: 90,
        parts: &[(0, &[0x4D, 0x4D, 0x00, 0x2A])],
    },
    Signature {
        format: "MP3",
        extension: "mp3",
        description: "MP3 audio (ID3-tagged)",
        confidence: 85,
        parts: &[(0, b"ID3")],
    },
    Signature {
        format: "OGG",
        extension: "ogg",
        description: "Ogg container",
        confidence: 100,
        parts: &[(0, b"OggS")],
    },
    Signature {
        format: "FLAC",
        extension: "flac",
        description: "FLAC audio",
        confidence: 100,
        parts: &[(0, b"fLaC")],
    },
    Signature {
        format: "SquashFS",
        extension: "squashfs",
        description: "SquashFS filesystem image (little-endian)",
        confidence: 100,
        parts: &[(0, b"hsqs")],
    },
    Signature {
        format: "SquashFS",
        extension: "squashfs",
        description: "SquashFS filesystem image (big-endian)",
        confidence: 100,
        parts: &[(0, b"sqsh")],
    },
    Signature {
        format: "uImage",
        extension: "img",
        description: "Das U-Boot kernel image header",
        confidence: 100,
        // 0x27051956, stored big-endian as the header always is.
        parts: &[(0, &[0x27, 0x05, 0x19, 0x56])],
    },
    Signature {
        format: "DEX",
        extension: "dex",
        description: "Android Dalvik executable",
        confidence: 100,
        parts: &[(0, b"dex\n")],
    },
    Signature {
        format: "SQLite",
        extension: "sqlite",
        description: "SQLite 3 database",
        confidence: 100,
        parts: &[(0, b"SQLite format 3\0")],
    },
    Signature {
        format: "WASM",
        extension: "wasm",
        description: "WebAssembly binary module",
        confidence: 100,
        parts: &[(0, &[0x00, 0x61, 0x73, 0x6D])],
    },
    Signature {
        format: "Java class",
        extension: "class",
        description: "Java compiled class file — or a Mach-O fat binary (same magic)",
        confidence: 60,
        parts: &[(0, &[0xCA, 0xFE, 0xBA, 0xBE])],
    },
];

/// Detect which known formats the given bytes match.
///
/// `bytes` should be the head of the file (a few hundred bytes is plenty; the
/// deepest signature we check ends at offset 262). Results are sorted most
/// confident first, then by signature specificity.
pub fn detect(bytes: &[u8]) -> Vec<Detection> {
    let mut hits: Vec<(usize, Detection)> = SIGNATURES
        .iter()
        .filter(|sig| sig.matches(bytes))
        .map(|sig| {
            (
                sig.magic_len(),
                Detection {
                    format: sig.format,
                    extension: sig.extension,
                    description: sig.description,
                    confidence: sig.confidence,
                },
            )
        })
        .collect();

    // Most confident first; break ties by longer (more specific) signature.
    hits.sort_by(|a, b| {
        b.1.confidence
            .cmp(&a.1.confidence)
            .then(b.0.cmp(&a.0))
    });
    hits.into_iter().map(|(_, d)| d).collect()
}

impl Signature {
    fn matches(&self, bytes: &[u8]) -> bool {
        self.parts.iter().all(|(offset, magic)| {
            let end = offset + magic.len();
            end <= bytes.len() && &bytes[*offset..end] == *magic
        })
    }

    /// Total signature bytes, used to rank specificity.
    fn magic_len(&self) -> usize {
        self.parts.iter().map(|(_, m)| m.len()).sum()
    }
}
