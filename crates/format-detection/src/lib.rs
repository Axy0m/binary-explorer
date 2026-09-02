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

/// The signature registry. Kept deliberately small and high-quality, per the
/// plan ("a small number of excellent implementations is better").
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
        description: "Java compiled class file",
        confidence: 95,
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
