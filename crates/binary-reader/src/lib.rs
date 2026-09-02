//! Endian-aware, seekable binary reader.
//!
//! This is the lowest layer of Binary Explorer (see the "Product Architecture
//! Principle" in the plan): it knows how to pull typed values out of a byte
//! source and nothing else. It has no knowledge of schemas, the UI, or React.
//!
//! Large files are memory-mapped rather than read fully into RAM, so opening an
//! 8 GB image is cheap — only the pages actually touched are faulted in by the OS.

use std::path::Path;

mod source;
pub use source::ByteSource;

/// Byte order for multi-byte reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

/// Text encoding understood by [`BinaryReader::read_string`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Bytes 0x00-0x7F map directly to chars; anything else is an error.
    Ascii,
    Utf8,
    Utf16Le,
    Utf16Be,
}

/// Errors produced by the reader.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("i/o error opening file: {0}")]
    Io(#[from] std::io::Error),

    #[error("out of bounds: requested {requested} byte(s) at offset {offset}, but source is {len} byte(s)")]
    OutOfBounds {
        offset: usize,
        requested: usize,
        len: usize,
    },

    #[error("invalid {encoding:?} text at offset {offset}")]
    InvalidText { offset: usize, encoding: Encoding },
}

pub type Result<T> = std::result::Result<T, ReadError>;

/// A cursor-based reader over a byte source.
///
/// Two styles of access are provided:
/// * cursor style (`read_u32_le`, `seek`, `position`) that advances an internal
///   position — convenient for walking a structure front to back;
/// * random access (`read_u32_le_at`, `read_bytes_at`) that takes an explicit
///   offset and leaves the cursor untouched — convenient for the UI, which jumps
///   around a file.
pub struct BinaryReader {
    source: ByteSource,
    pos: usize,
}

impl BinaryReader {
    /// Open a file, memory-mapping its contents. Empty files are supported.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::new(ByteSource::open(path)?))
    }

    /// Build a reader over an in-memory buffer (used in tests and small blobs).
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(ByteSource::from_bytes(bytes.into()))
    }

    fn new(source: ByteSource) -> Self {
        Self { source, pos: 0 }
    }

    /// Total length of the underlying source in bytes.
    pub fn len(&self) -> usize {
        self.source.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Current cursor position.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Move the cursor to an absolute offset. The offset may equal `len()`
    /// (end of file) but not exceed it.
    pub fn seek(&mut self, offset: usize) -> Result<()> {
        if offset > self.len() {
            return Err(ReadError::OutOfBounds {
                offset,
                requested: 0,
                len: self.len(),
            });
        }
        self.pos = offset;
        Ok(())
    }

    /// Borrow `len` bytes at an absolute offset without moving the cursor.
    pub fn read_bytes_at(&self, offset: usize, len: usize) -> Result<&[u8]> {
        let slice = self.source.as_slice();
        let end = offset.checked_add(len).ok_or(ReadError::OutOfBounds {
            offset,
            requested: len,
            len: slice.len(),
        })?;
        if end > slice.len() {
            return Err(ReadError::OutOfBounds {
                offset,
                requested: len,
                len: slice.len(),
            });
        }
        Ok(&slice[offset..end])
    }

    /// Read `len` bytes at the cursor, advancing it.
    pub fn read_bytes(&mut self, len: usize) -> Result<&[u8]> {
        let start = self.pos;
        // Bounds-check via read_bytes_at, then advance.
        self.read_bytes_at(start, len)?;
        self.pos = start + len;
        Ok(&self.source.as_slice()[start..start + len])
    }

    /// Decode `len` bytes at the cursor as text, advancing the cursor.
    pub fn read_string(&mut self, len: usize, encoding: Encoding) -> Result<String> {
        let start = self.pos;
        let s = self.read_string_at(start, len, encoding)?;
        self.pos = start + len;
        Ok(s)
    }

    /// Decode `len` bytes at an absolute offset as text, cursor untouched.
    pub fn read_string_at(&self, offset: usize, len: usize, encoding: Encoding) -> Result<String> {
        let bytes = self.read_bytes_at(offset, len)?;
        match encoding {
            Encoding::Ascii => {
                if bytes.iter().all(|b| b.is_ascii()) {
                    Ok(bytes.iter().map(|&b| b as char).collect())
                } else {
                    Err(ReadError::InvalidText { offset, encoding })
                }
            }
            Encoding::Utf8 => String::from_utf8(bytes.to_vec())
                .map_err(|_| ReadError::InvalidText { offset, encoding }),
            Encoding::Utf16Le | Encoding::Utf16Be => {
                if bytes.len() % 2 != 0 {
                    return Err(ReadError::InvalidText { offset, encoding });
                }
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|c| match encoding {
                        Encoding::Utf16Le => u16::from_le_bytes([c[0], c[1]]),
                        _ => u16::from_be_bytes([c[0], c[1]]),
                    })
                    .collect();
                String::from_utf16(&units)
                    .map_err(|_| ReadError::InvalidText { offset, encoding })
            }
        }
    }
}

/// Generate cursor + random-access read methods for a primitive integer/float type.
///
/// For each type `t` this emits, e.g. for `u32`:
/// `read_u32_le`, `read_u32_be`, `read_u32_le_at`, `read_u32_be_at`.
macro_rules! impl_read_num {
    ($($t:ty => ($le:ident, $be:ident, $le_at:ident, $be_at:ident)),+ $(,)?) => {
        impl BinaryReader {
            $(
                #[doc = concat!("Read a little-endian `", stringify!($t), "` at an absolute offset.")]
                pub fn $le_at(&self, offset: usize) -> Result<$t> {
                    const N: usize = std::mem::size_of::<$t>();
                    let b = self.read_bytes_at(offset, N)?;
                    let mut arr = [0u8; N];
                    arr.copy_from_slice(b);
                    Ok(<$t>::from_le_bytes(arr))
                }

                #[doc = concat!("Read a big-endian `", stringify!($t), "` at an absolute offset.")]
                pub fn $be_at(&self, offset: usize) -> Result<$t> {
                    const N: usize = std::mem::size_of::<$t>();
                    let b = self.read_bytes_at(offset, N)?;
                    let mut arr = [0u8; N];
                    arr.copy_from_slice(b);
                    Ok(<$t>::from_be_bytes(arr))
                }

                #[doc = concat!("Read a little-endian `", stringify!($t), "` at the cursor, advancing it.")]
                pub fn $le(&mut self) -> Result<$t> {
                    let v = self.$le_at(self.pos)?;
                    self.pos += std::mem::size_of::<$t>();
                    Ok(v)
                }

                #[doc = concat!("Read a big-endian `", stringify!($t), "` at the cursor, advancing it.")]
                pub fn $be(&mut self) -> Result<$t> {
                    let v = self.$be_at(self.pos)?;
                    self.pos += std::mem::size_of::<$t>();
                    Ok(v)
                }
            )+
        }
    };
}

impl_read_num! {
    u16 => (read_u16_le, read_u16_be, read_u16_le_at, read_u16_be_at),
    u32 => (read_u32_le, read_u32_be, read_u32_le_at, read_u32_be_at),
    u64 => (read_u64_le, read_u64_be, read_u64_le_at, read_u64_be_at),
    i16 => (read_i16_le, read_i16_be, read_i16_le_at, read_i16_be_at),
    i32 => (read_i32_le, read_i32_be, read_i32_le_at, read_i32_be_at),
    i64 => (read_i64_le, read_i64_be, read_i64_le_at, read_i64_be_at),
    f32 => (read_f32_le, read_f32_be, read_f32_le_at, read_f32_be_at),
    f64 => (read_f64_le, read_f64_be, read_f64_le_at, read_f64_be_at),
}

// Single-byte reads don't need endianness, so they're written by hand.
impl BinaryReader {
    pub fn read_u8_at(&self, offset: usize) -> Result<u8> {
        Ok(self.read_bytes_at(offset, 1)?[0])
    }

    pub fn read_i8_at(&self, offset: usize) -> Result<i8> {
        Ok(self.read_u8_at(offset)? as i8)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        let v = self.read_u8_at(self.pos)?;
        self.pos += 1;
        Ok(v)
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        let v = self.read_i8_at(self.pos)?;
        self.pos += 1;
        Ok(v)
    }
}
