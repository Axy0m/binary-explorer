//! The byte source backing a [`crate::BinaryReader`].
//!
//! Files are memory-mapped so that very large files (multi-GB disk images,
//! databases, firmware) can be opened without reading them into RAM. Small
//! in-memory buffers are also supported for tests and pasted data.

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

pub enum ByteSource {
    /// A memory-mapped file. The OS pages in only the regions actually read.
    Mapped(Mmap),
    /// An owned in-memory buffer.
    Owned(Vec<u8>),
}

impl ByteSource {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len == 0 {
            // mmap of a zero-length file is invalid on some platforms; use an
            // empty owned buffer instead so empty files "just work".
            return Ok(ByteSource::Owned(Vec::new()));
        }
        // SAFETY: the file is opened read-only. The usual mmap caveat applies —
        // external truncation of the file while mapped is undefined behavior.
        // Binary Explorer opens files read-only and does not expect concurrent
        // truncation, which is the standard assumption for a file viewer.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(ByteSource::Mapped(mmap))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        ByteSource::Owned(bytes)
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            ByteSource::Mapped(m) => &m[..],
            ByteSource::Owned(v) => &v[..],
        }
    }
}
