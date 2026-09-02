//! Golden (snapshot) tests for the *built-in* schemas.
//!
//! The DSL-feature tests in `runtime.rs` prove each language construct works in
//! isolation. These tests instead pin the end-to-end output of the eight
//! schemas that ship with the app (`schemas/*.schema`) against a crafted
//! fixture for each format. They catch the scariest regression class in this
//! project: a lexer/parser/runtime change that silently shifts a field's
//! `offset`/`size` or mis-decodes a value — nothing errors, the UI just
//! highlights the wrong bytes.
//!
//! Each test renders the whole `FieldNode` tree (name, type, value, offset,
//! size, description) to canonical text and compares it to a committed
//! `tests/golden/<name>.golden` file. To regenerate the snapshots after an
//! intentional schema/output change:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p schema-runtime --test golden
//! ```
//!
//! Review the resulting diff before committing — that's the whole point.

use std::path::Path;

use binary_reader::BinaryReader;
use schema_runtime::{parse, Endian, FieldNode, Value};

// --- Built-in schema sources (the exact files the app bundles) -------------

const PNG: &str = include_str!("../../../schemas/png.schema");
const GZIP: &str = include_str!("../../../schemas/gzip.schema");
const ZIP: &str = include_str!("../../../schemas/zip.schema");
const SQLITE: &str = include_str!("../../../schemas/sqlite.schema");
const ELF: &str = include_str!("../../../schemas/elf.schema");
const PE: &str = include_str!("../../../schemas/pe.schema");
const MACHO: &str = include_str!("../../../schemas/macho.schema");
const PCAP: &str = include_str!("../../../schemas/pcap.schema");

// --- Tiny endian-aware byte builder ----------------------------------------

/// A little fluent builder so the fixtures below read like the byte layout
/// they describe. Every write is explicit about width and endianness.
#[derive(Default)]
struct B(Vec<u8>);

impl B {
    fn new() -> Self {
        B(Vec::new())
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn u16le(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32le(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64le(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32le(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u16be(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32be(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn raw(mut self, b: &[u8]) -> Self {
        self.0.extend_from_slice(b);
        self
    }
    fn ascii(self, s: &str) -> Self {
        self.raw(s.as_bytes())
    }
    /// Append `n` zero bytes.
    fn zeros(mut self, n: usize) -> Self {
        self.0.resize(self.0.len() + n, 0);
        self
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn build(self) -> Vec<u8> {
        self.0
    }
}

// --- Golden harness ---------------------------------------------------------

fn golden(name: &str, src: &str, entry: &str, endian: Endian, bytes: Vec<u8>) {
    let schema = schema_parser::parse(src)
        .unwrap_or_else(|e| panic!("built-in schema {name} should parse: {e}"));
    let reader = BinaryReader::from_bytes(bytes);
    let tree = parse(&schema, &reader, entry, endian)
        .unwrap_or_else(|e| panic!("built-in schema {name} should run against its fixture: {e}"));
    let actual = render(&tree);

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.golden"));

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual.as_bytes()).unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden file {}\nrun `UPDATE_GOLDEN=1 cargo test -p schema-runtime --test golden` to create it",
            path.display()
        )
    });
    // Normalize line endings: git may check these out as CRLF on Windows.
    let expected = expected.replace("\r\n", "\n");
    assert_eq!(
        actual, expected,
        "golden mismatch for {name}\nrerun with UPDATE_GOLDEN=1 to update the snapshot after reviewing the change"
    );
}

fn render(node: &FieldNode) -> String {
    let mut out = String::new();
    write_node(&mut out, node, 0);
    out
}

fn write_node(out: &mut String, node: &FieldNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let value = render_value(&node.value);
    let val = if value.is_empty() {
        String::new()
    } else {
        format!(" = {value}")
    };
    let desc = if node.description.is_empty() {
        String::new()
    } else {
        format!("  // {}", node.description)
    };
    out.push_str(&format!(
        "{indent}{}: {}{val}  [@{} +{}]{desc}\n",
        node.name, node.type_name, node.offset, node.size
    ));
    for c in &node.children {
        write_node(out, c, depth + 1);
    }
}

fn render_value(v: &Value) -> String {
    match v {
        Value::U(n) => n.to_string(),
        Value::I(n) => n.to_string(),
        Value::F(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => format!("'{c}'"),
        Value::Str(s) => format!("{s:?}"),
        Value::Bytes(b) => b
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" "),
        Value::Enum(e) => match &e.name {
            Some(name) => format!("{name} ({})", e.value),
            None => format!("{} (unknown)", e.value),
        },
        Value::Struct | Value::Array | Value::Bitfield => String::new(),
    }
}

// --- Fixtures + tests -------------------------------------------------------

#[test]
fn golden_png() {
    // 8-byte signature + a full chunk stream (IHDR, IDAT, IEND) for a 1x1 RGBA
    // image. This exercises `repeat ... until chunkType == "IEND"` plus the
    // string-dispatched `match` that decodes IHDR and leaves IDAT raw.
    let bytes = B::new()
        .raw(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) // signature
        // -- IHDR chunk --
        .u32be(13) // IHDR data length
        .ascii("IHDR") // chunk type
        .u32be(1) // width
        .u32be(1) // height
        .u8(8) // bit depth
        .u8(6) // color type -> RGBA
        .u8(0) // compression
        .u8(0) // filter
        .u8(0) // interlace -> None
        .u32be(0x12345678) // crc (arbitrary)
        // -- IDAT chunk (left raw by the default arm) --
        .u32be(4) // data length
        .ascii("IDAT")
        .raw(&[0xDE, 0xAD, 0xBE, 0xEF]) // opaque data
        .u32be(0x9ABCDEF0) // crc
        // -- IEND chunk (terminator, zero-length) --
        .u32be(0)
        .ascii("IEND")
        .u32be(0xAE426082) // canonical IEND crc
        .build();
    golden("png", PNG, "PNG", Endian::Big, bytes);
}

#[test]
fn golden_macho() {
    // 64-bit Mach-O header: magic 0xFEEDFACF, an x86_64 executable. Exercises
    // the enum-typed cpuType/fileType fields.
    let bytes = B::new()
        .u32le(0xFEED_FACF) // magic
        .u32le(16777223) // cpuType -> X86_64
        .u32le(3) // cpuSubtype
        .u32le(2) // fileType -> Execute
        .u32le(16) // numCmds
        .u32le(1416) // sizeOfCmds
        .u32le(0x0020_0085) // flags
        .u32le(0) // reserved
        .build();
    golden("macho", MACHO, "MachHeader64", Endian::Little, bytes);
}

#[test]
fn golden_pcap() {
    // libpcap global header (little-endian, microsecond) + one Ethernet packet
    // record. Exercises the `bytes[inclLen]` field-driven length.
    let bytes = B::new()
        .u32le(0xA1B2_C3D4) // magic
        .u16le(2) // versionMajor
        .u16le(4) // versionMinor
        .i32le(0) // thisZone
        .u32le(0) // sigfigs
        .u32le(65535) // snaplen
        .u32le(1) // network -> Ethernet
        // first packet record
        .u32le(0x5D3C_1A00) // tsSec
        .u32le(12345) // tsUsec
        .u32le(4) // inclLen
        .u32le(60) // origLen
        .raw(&[0xDE, 0xAD, 0xBE, 0xEF]) // captured bytes
        .build();
    golden("pcap", PCAP, "Pcap", Endian::Little, bytes);
}

#[test]
fn golden_gzip() {
    // FNAME flag (bit 3) set -> the `name` cstring is present; others absent.
    let bytes = B::new()
        .raw(&[0x1F, 0x8B]) // magic
        .u8(8) // method (deflate)
        .u8(0x08) // flags: FNAME
        .u32le(0x5D3C_1A00) // mtime
        .u8(0) // xfl
        .u8(3) // os (unix)
        .ascii("hi.txt") // name ...
        .u8(0) // ... NUL terminator
        .build();
    golden("gzip", GZIP, "Gzip", Endian::Little, bytes);
}

#[test]
fn golden_zip() {
    // Local file header for a stored 5-char name, no extra field, UTF-8 flag.
    let name = "a.txt";
    let bytes = B::new()
        .raw(&[0x50, 0x4B, 0x03, 0x04]) // signature PK\x03\x04
        .u16le(20) // versionNeeded
        .u16le(0x0800) // flags: utf8 (bit 11)
        .u16le(8) // method -> Deflated
        .u16le(0x6000) // modTime
        .u16le(0x5000) // modDate
        .u32le(0x1234_5678) // crc32
        .u32le(100) // compressedSize
        .u32le(200) // uncompressedSize
        .u16le(name.len() as u16) // nameLength
        .u16le(0) // extraLength
        .ascii(name) // fileName
        .build();
    golden("zip", ZIP, "ZipLocalFileHeader", Endian::Little, bytes);
}

#[test]
fn golden_sqlite() {
    // The full 100-byte SQLite 3 header (big-endian).
    let bytes = B::new()
        .ascii("SQLite format 3")
        .u8(0) // 16-byte NUL-terminated magic
        .u16be(4096) // pageSize
        .u8(1) // writeVersion -> Legacy
        .u8(1) // readVersion -> Legacy
        .u8(0) // reservedSpace
        .u8(64) // maxPayloadFraction
        .u8(32) // minPayloadFraction
        .u8(32) // leafPayloadFraction
        .u32be(5) // changeCounter
        .u32be(2) // databaseSizePages
        .u32be(0) // firstFreelistPage
        .u32be(0) // freelistPageCount
        .u32be(1) // schemaCookie
        .u32be(4) // schemaFormat
        .u32be(0) // defaultCacheSize
        .u32be(0) // largestRootPage
        .u32be(1) // textEncoding -> Utf8
        .u32be(0) // userVersion
        .u32be(0) // incrementalVacuum
        .u32be(0) // applicationId
        .zeros(20) // reserved
        .u32be(5) // versionValidFor
        .u32be(3045000) // sqliteVersion
        .build();
    assert_eq!(bytes.len(), 100, "SQLite header fixture must be 100 bytes");
    golden("sqlite", SQLITE, "SqliteHeader", Endian::Big, bytes);
}

#[test]
fn golden_elf() {
    // Minimal ELF64: 64-byte header, one 56-byte program header at phoff=64,
    // one 64-byte section header at shoff=120. Total 184 bytes.
    let mut b = B::new()
        // e_ident (16 bytes)
        .raw(&[0x7F, b'E', b'L', b'F']) // magic
        .u8(2) // class -> ELF64
        .u8(1) // data -> LittleEndian
        .u8(1) // version
        .u8(0) // osabi -> SystemV
        .u8(0) // abiVersion
        .zeros(7) // pad
        // rest of the ELF header
        .u16le(2) // type -> Executable
        .u16le(62) // machine -> X86_64
        .u32le(1) // version
        .u64le(0x4010A0) // entry
        .u64le(64) // phoff
        .u64le(120) // shoff
        .u32le(0) // flags
        .u16le(64) // ehsize
        .u16le(56) // phentsize
        .u16le(1) // phnum
        .u16le(64) // shentsize
        .u16le(1) // shnum
        .u16le(0); // shstrndx
    assert_eq!(b.len(), 64, "ELF header must be 64 bytes");

    // Program header (56 bytes) at offset 64.
    b = b
        .u32le(1) // type -> Load
        .u32le(5) // flags -> read|execute
        .u64le(0) // offset
        .u64le(0x400000) // vaddr
        .u64le(0x400000) // paddr
        .u64le(0x200) // filesz
        .u64le(0x200) // memsz
        .u64le(0x1000); // align
    assert_eq!(b.len(), 120, "program header should end at shoff");

    // Section header (64 bytes) at offset 120.
    b = b
        .u32le(1) // name (offset into .shstrtab)
        .u32le(1) // type -> Progbits
        .u64le(6) // flags -> alloc|execinstr
        .u64le(0x401000) // addr
        .u64le(0x1000) // offset
        .u64le(0x1a4) // size
        .u32le(0) // link
        .u32le(0) // info
        .u64le(16) // addralign
        .u64le(0); // entsize
    assert_eq!(b.len(), 184, "ELF fixture should be 184 bytes");

    golden("elf", ELF, "Elf64", Endian::Little, b.build());
}

#[test]
fn golden_pe() {
    // Minimal PE32 (magic 0x10b): 64-byte DOS header, e_lfanew=64 -> PE header,
    // COFF header, a PE32 optional header with one data directory
    // (sizeOfOptionalHeader=104), and one 40-byte section at offset 192.
    let mut b = B::new()
        // IMAGE_DOS_HEADER (64 bytes)
        .ascii("MZ") // magic
        .u16le(0x90) // lastPageBytes
        .u16le(3) // pages
        .u16le(0) // relocations
        .u16le(4) // headerParagraphs
        .u16le(0) // minAlloc
        .u16le(0xFFFF) // maxAlloc
        .u16le(0) // initialSS
        .u16le(0xB8) // initialSP
        .u16le(0) // checksum
        .u16le(0) // initialIP
        .u16le(0) // initialCS
        .u16le(0x40) // relocTableOffset
        .u16le(0) // overlayNumber
        .zeros(8) // reserved
        .u16le(0) // oemId
        .u16le(0) // oemInfo
        .zeros(20) // reserved2
        .u32le(64); // lfanew -> 64
    assert_eq!(b.len(), 64, "DOS header must be 64 bytes");

    // PE header + COFF file header (24 bytes) at offset 64.
    b = b
        .raw(&[0x50, 0x45, 0x00, 0x00]) // signature "PE\0\0"
        .u16le(0x14c) // machine -> I386
        .u16le(1) // numberOfSections
        .u32le(0x5F5E_1000) // timeDateStamp
        .u32le(0) // pointerToSymbolTable
        .u32le(0) // numberOfSymbols
        .u16le(104) // sizeOfOptionalHeader
        .u16le(0x0102); // characteristics (executableImage | 32bit machine)
    assert_eq!(b.len(), 88, "COFF header should end at 88");

    // PE32 optional header (104 bytes) at offset 88.
    b = b
        .u16le(0x10b) // magic -> PE32
        .u8(14) // majorLinkerVersion
        .u8(0) // minorLinkerVersion
        .u32le(0x200) // sizeOfCode
        .u32le(0x200) // sizeOfInitializedData
        .u32le(0) // sizeOfUninitializedData
        .u32le(0x1000) // addressOfEntryPoint
        .u32le(0x1000) // baseOfCode
        .u32le(0x2000) // baseOfData (PE32)
        .u32le(0x400000) // imageBase32 (PE32)
        .u32le(0x1000) // sectionAlignment
        .u32le(0x200) // fileAlignment
        .u16le(6) // majorOsVersion
        .u16le(0) // minorOsVersion
        .u16le(0) // majorImageVersion
        .u16le(0) // minorImageVersion
        .u16le(6) // majorSubsystemVersion
        .u16le(0) // minorSubsystemVersion
        .u32le(0) // win32VersionValue
        .u32le(0x4000) // sizeOfImage
        .u32le(0x200) // sizeOfHeaders
        .u32le(0) // checkSum
        .u16le(3) // subsystem -> WindowsCui
        .u16le(0x8140) // dllCharacteristics
        .u32le(0x100000) // stackReserve32
        .u32le(0x1000) // stackCommit32
        .u32le(0x100000) // heapReserve32
        .u32le(0x1000) // heapCommit32
        .u32le(0) // loaderFlags
        .u32le(1) // numberOfRvaAndSizes -> 1
        // one DataDirectory (8 bytes)
        .u32le(0) // virtualAddress
        .u32le(0); // size
    assert_eq!(b.len(), 192, "optional header should end at the section table");

    // Section table: one 40-byte IMAGE_SECTION_HEADER at offset 192.
    b = b
        .ascii(".text")
        .zeros(3) // name char[8], NUL-padded
        .u32le(0x1A4) // virtualSize
        .u32le(0x1000) // virtualAddress
        .u32le(0x200) // sizeOfRawData
        .u32le(0x200) // pointerToRawData
        .u32le(0) // pointerToRelocations
        .u32le(0) // pointerToLinenumbers
        .u16le(0) // numberOfRelocations
        .u16le(0) // numberOfLinenumbers
        .u32le(0x6000_0020); // characteristics (code | execute | read)
    assert_eq!(b.len(), 232, "PE fixture should be 232 bytes");

    golden("pe", PE, "PE", Endian::Little, b.build());
}
