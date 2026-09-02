//! Integration tests for the schema runtime. Schemas are written in the real
//! DSL (via `schema-parser`) and executed against in-memory byte buffers, so
//! these tests exercise the whole Phase 3 + Phase 4 pipeline end to end.

use binary_reader::BinaryReader;
use schema_runtime::{parse, Endian, FieldNode, Value};

fn run(src: &str, entry: &str, bytes: Vec<u8>, endian: Endian) -> FieldNode {
    let schema = schema_parser::parse(src).expect("schema should parse");
    let reader = BinaryReader::from_bytes(bytes);
    parse(&schema, &reader, entry, endian).expect("runtime should succeed")
}

/// Find a direct child by name.
fn child<'a>(node: &'a FieldNode, name: &str) -> &'a FieldNode {
    node.children
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no child named {name}"))
}

#[test]
fn plan_section_9_example() {
    // struct Header { magic char[4]  version u16  size u32 }
    // bytes:        00 01 02 03      01 00        80 00 00 00
    // -> magic bytes, version 1, size 128   (little-endian)
    let src = "struct Header { magic char[4]  version u16  size u32 }";
    let bytes = vec![0x00, 0x01, 0x02, 0x03, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00];
    let root = run(src, "Header", bytes, Endian::Little);

    assert_eq!(root.name, "Header");
    assert_eq!(root.type_name, "Header");
    assert_eq!(root.offset, 0);
    assert_eq!(root.size, 10);

    assert_eq!(child(&root, "version").value, Value::U(1));
    assert_eq!(child(&root, "size").value, Value::U(128));
}

#[test]
fn every_field_records_its_exact_byte_range() {
    // This offset/size metadata is the whole point (byte <-> field highlighting).
    let src = "struct Header { magic char[4]  version u16  size u32 }";
    let bytes = vec![0x00, 0x01, 0x02, 0x03, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00];
    let root = run(src, "Header", bytes, Endian::Little);

    let magic = child(&root, "magic");
    assert_eq!((magic.offset, magic.size), (0, 4));
    let version = child(&root, "version");
    assert_eq!((version.offset, version.size), (4, 2));
    let size = child(&root, "size");
    assert_eq!((size.offset, size.size), (6, 4));
}

#[test]
fn endianness_flips_multibyte_values() {
    let src = "struct S { n u32 }";
    let bytes = vec![0x00, 0x00, 0x00, 0x2A]; // 42 big-endian
    assert_eq!(child(&run(src, "S", bytes.clone(), Endian::Big), "n").value, Value::U(42));
    assert_eq!(
        child(&run(src, "S", bytes, Endian::Little), "n").value,
        Value::U(0x2A00_0000),
    );
}

#[test]
fn char_array_reads_back_as_a_string() {
    // \x7fELF — the ELF magic — as char[4].
    let src = "struct S { magic char[4] }";
    let bytes = vec![0x7f, b'E', b'L', b'F'];
    let root = run(src, "S", bytes, Endian::Little);
    let magic = child(&root, "magic");
    assert_eq!(magic.value, Value::Str("\u{7f}ELF".to_string()));
    assert_eq!(magic.type_name, "char[4]");
}

#[test]
fn fixed_string_stops_at_nul_padding() {
    let src = "struct S { name string[8] }";
    let bytes = vec![b'H', b'i', 0, 0, 0, 0, 0, 0];
    let root = run(src, "S", bytes, Endian::Little);
    let name = child(&root, "name");
    assert_eq!(name.value, Value::Str("Hi".to_string()));
    assert_eq!(name.size, 8); // still occupies all 8 bytes
}

#[test]
fn signed_and_float_and_bool() {
    let src = "struct S { a i8  b f32  c bool }";
    // a = -1 ; b = 1.0f32 LE (00 00 80 3F) ; c = 1 -> true
    let bytes = vec![0xFF, 0x00, 0x00, 0x80, 0x3F, 0x01];
    let root = run(src, "S", bytes, Endian::Little);
    assert_eq!(child(&root, "a").value, Value::I(-1));
    assert_eq!(child(&root, "b").value, Value::F(1.0));
    assert_eq!(child(&root, "c").value, Value::Bool(true));
}

#[test]
fn nested_structs_produce_a_tree() {
    let src = "
        struct Point { x u8  y u8 }
        struct Line  { a Point  b Point }
    ";
    let bytes = vec![1, 2, 3, 4];
    let root = run(src, "Line", bytes, Endian::Little);
    assert_eq!(root.size, 4);

    let a = child(&root, "a");
    assert_eq!(a.type_name, "Point");
    assert_eq!(a.value, Value::Struct);
    assert_eq!((a.offset, a.size), (0, 2));
    assert_eq!(child(a, "x").value, Value::U(1));
    assert_eq!(child(a, "y").value, Value::U(2));

    let b = child(&root, "b");
    assert_eq!((b.offset, b.size), (2, 2));
    assert_eq!(child(b, "x").value, Value::U(3));
}

#[test]
fn fixed_length_array_of_primitives() {
    let src = "struct S { xs u16[3] }";
    let bytes = vec![1, 0, 2, 0, 3, 0];
    let root = run(src, "S", bytes, Endian::Little);
    let xs = child(&root, "xs");
    assert_eq!(xs.type_name, "u16[3]");
    assert_eq!(xs.value, Value::Array);
    assert_eq!(xs.children.len(), 3);
    assert_eq!(xs.children[0].value, Value::U(1));
    assert_eq!(xs.children[2].value, Value::U(3));
    assert_eq!((xs.children[2].offset, xs.children[2].size), (4, 2));
}

#[test]
fn array_length_from_an_earlier_field() {
    // struct File { count u8  items u16[count] }
    let src = "struct File { count u8  items u16[count] }";
    let bytes = vec![0x03, 10, 0, 20, 0, 30, 0];
    let root = run(src, "File", bytes, Endian::Little);
    let items = child(&root, "items");
    assert_eq!(items.children.len(), 3);
    assert_eq!(items.children[1].value, Value::U(20));
    assert_eq!(root.size, 1 + 3 * 2);
}

#[test]
fn out_of_bounds_read_is_an_error_not_a_panic() {
    let schema = schema_parser::parse("struct S { n u32 }").unwrap();
    let reader = BinaryReader::from_bytes(vec![0x00, 0x01]); // only 2 bytes
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

#[test]
fn unknown_entry_struct_is_an_error() {
    let schema = schema_parser::parse("struct S { n u8 }").unwrap();
    let reader = BinaryReader::from_bytes(vec![0x00]);
    assert!(parse(&schema, &reader, "Nope", Endian::Little).is_err());
}

// --- Enums & bitfields (Phase 11) ------------------------------------------

#[test]
fn enum_decodes_known_and_unknown_values() {
    let src = "
        enum ColorType : u8 { Grayscale = 0  RGB = 2  RGBA = 6 }
        struct Img { color ColorType  other ColorType }
    ";
    // color = 6 (RGBA, known); other = 9 (no variant)
    let root = run(src, "Img", vec![6, 9], Endian::Little);

    let color = child(&root, "color");
    assert_eq!(color.type_name, "ColorType");
    assert_eq!((color.offset, color.size), (0, 1));
    assert_eq!(
        color.value,
        Value::Enum(schema_runtime::EnumValue { value: 6, name: Some("RGBA".into()) })
    );

    let other = child(&root, "other");
    assert_eq!(
        other.value,
        Value::Enum(schema_runtime::EnumValue { value: 9, name: None })
    );
}

#[test]
fn bitfield_unpacks_single_bits_and_ranges() {
    let src = "
        bitfield Flags : u8 { a 0  b 1  level 2..4 }
        struct S { f Flags }
    ";
    // 0b0001_1001 = 0x19: bit0=1 (a), bit1=0 (b), bits2..4 = 0b110 = 6
    let root = run(src, "S", vec![0x19], Endian::Little);
    let f = child(&root, "f");
    assert_eq!(f.type_name, "Flags");
    assert_eq!(f.value, Value::Bitfield);
    assert_eq!((f.offset, f.size), (0, 1));

    assert_eq!(child(f, "a").value, Value::Bool(true));
    assert_eq!(child(f, "b").value, Value::Bool(false));
    assert_eq!(child(f, "level").value, Value::U(6));
    // Members share the underlying byte, so each spans the same range.
    assert_eq!((child(f, "level").offset, child(f, "level").size), (0, 1));
}

#[test]
fn bitfield_respects_endianness() {
    // u16 bitfield: bit 8 is the low bit of the second byte.
    let src = "
        bitfield B : u16 { top 8 }
        struct S { b B }
    ";
    // LE bytes 00 01 -> value 0x0100 -> bit 8 set
    let root = run(src, "S", vec![0x00, 0x01], Endian::Little);
    assert_eq!(child(child(&root, "b"), "top").value, Value::Bool(true));
    // BE bytes 00 01 -> value 0x0001 -> bit 8 clear
    let root = run(src, "S", vec![0x00, 0x01], Endian::Big);
    assert_eq!(child(child(&root, "b"), "top").value, Value::Bool(false));
}

#[test]
fn reference_to_missing_type_is_an_error() {
    let schema = schema_parser::parse("struct S { x Nope }").unwrap();
    let reader = BinaryReader::from_bytes(vec![0x00, 0x00]);
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

// --- Conditional fields (Phase 11) -----------------------------------------

#[test]
fn conditional_field_present_when_true() {
    // present = 1, so `extra` (u32 LE) is read from the next 4 bytes.
    let src = "struct S { present u8  extra u32 if present }";
    let root = run(src, "S", vec![1, 0x2A, 0x00, 0x00, 0x00], Endian::Little);
    assert_eq!(root.children.len(), 2);
    assert_eq!(child(&root, "extra").value, Value::U(42));
    assert_eq!(root.size, 5);
}

#[test]
fn conditional_field_absent_consumes_no_bytes() {
    // present = 0, so `extra` is skipped; `tail` reads the byte right after.
    let src = "struct S { present u8  extra u32 if present  tail u8 }";
    let root = run(src, "S", vec![0, 0x99], Endian::Little);
    assert_eq!(root.children.len(), 2); // present + tail, no extra
    assert!(root.children.iter().all(|c| c.name != "extra"));
    assert_eq!(child(&root, "tail").value, Value::U(0x99));
    assert_eq!((child(&root, "tail").offset, root.size), (1, 2));
}

#[test]
fn conditional_on_bitfield_member() {
    // gzip-style: XLEN present only when the FEXTRA flag bit is set.
    let src = "
        bitfield Flags : u8 { extra 2 }
        struct Hdr { flags Flags  xlen u16 if flags.extra }
    ";
    // flags = 0x04 (bit 2 set) -> xlen present, reads 0x0010 = 16 LE
    let root = run(src, "Hdr", vec![0x04, 0x10, 0x00], Endian::Little);
    assert_eq!(child(&root, "xlen").value, Value::U(16));

    // flags = 0x00 -> xlen absent
    let root = run(src, "Hdr", vec![0x00], Endian::Little);
    assert!(root.children.iter().all(|c| c.name != "xlen"));
}

#[test]
fn conditional_comparison_operators() {
    let src = "struct S { ver u8  body u32 if ver >= 2 }";
    // ver = 1 -> body absent
    let root = run(src, "S", vec![1], Endian::Little);
    assert!(root.children.iter().all(|c| c.name != "body"));
    // ver = 2 -> body present
    let root = run(src, "S", vec![2, 0x01, 0x00, 0x00, 0x00], Endian::Little);
    assert_eq!(child(&root, "body").value, Value::U(1));
}

#[test]
fn condition_referencing_unknown_field_errors() {
    let schema = schema_parser::parse("struct S { a u8  b u8 if nope }").unwrap();
    let reader = BinaryReader::from_bytes(vec![1, 2]);
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

// --- Pointer / relative-offset fields (Phase 11) ---------------------------

#[test]
fn absolute_pointer_reads_at_target_offset() {
    // off (u32 LE) = 6 -> `target` (u16 LE) is read at byte 6, not byte 4.
    let src = "struct S { off u32  target at off u16 }";
    //             offset:  0  1  2  3   (off = 6)   ...  6  7
    let bytes = vec![0x06, 0x00, 0x00, 0x00, 0xEE, 0xEE, 0x2A, 0x00];
    let root = run(src, "S", bytes, Endian::Little);

    let target = child(&root, "target");
    assert_eq!(target.value, Value::U(0x2A));
    assert_eq!(target.offset, 6); // reads at the pointed-to offset
    // The pointer field consumes no sequential bytes: the struct spans only off.
    assert_eq!(root.size, 4);
}

#[test]
fn relative_pointer_adds_struct_start() {
    // Nested struct starts at offset 4; `at +2` -> absolute offset 4 + 2 = 6.
    let src = "
        struct Inner { rel at +2 u8 }
        struct Outer { pad u32  inner Inner }
    ";
    let bytes = vec![0, 0, 0, 0, 0x11, 0x22, 0x33, 0x44];
    let root = run(src, "Outer", bytes, Endian::Little);
    let rel = child(child(&root, "inner"), "rel");
    assert_eq!(rel.offset, 6);
    assert_eq!(rel.value, Value::U(0x33));
}

#[test]
fn pointer_to_field_length_array() {
    // ELF-style: phnum entries of u32 living at phoff.
    let src = "struct Elf { phnum u16  phoff u32  phs at phoff u32[phnum] }";
    // phnum = 2, phoff = 8; at byte 8: two u32 LE = 0x01, 0x02
    let bytes = vec![
        0x02, 0x00, // phnum = 2
        0x08, 0x00, 0x00, 0x00, // phoff = 8
        0xFF, 0xFF, // padding between header and target
        0x01, 0x00, 0x00, 0x00, // phs[0] = 1
        0x02, 0x00, 0x00, 0x00, // phs[1] = 2
    ];
    let root = run(src, "Elf", bytes, Endian::Little);
    let phs = child(&root, "phs");
    assert_eq!(phs.offset, 8);
    assert_eq!(phs.children.len(), 2);
    assert_eq!(phs.children[0].value, Value::U(1));
    assert_eq!(phs.children[1].value, Value::U(2));
}

#[test]
fn pointer_past_end_of_file_errors() {
    let schema = schema_parser::parse("struct S { off u8  t at off u32 }").unwrap();
    // off = 200, far beyond this 2-byte file.
    let reader = BinaryReader::from_bytes(vec![200, 0]);
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

// --- Discriminated unions / match fields (Phase 11) ------------------------

const TLV: &str = "struct TLV {
    tag u8
    body match tag {
        0 => u32
        1 => string[4]
        default => bytes[2]
    }
}";

#[test]
fn match_selects_arm_by_discriminant() {
    // tag = 0 -> body is a u32 (LE) read from the next 4 bytes.
    let root = run(TLV, "TLV", vec![0, 0x2A, 0x00, 0x00, 0x00], Endian::Little);
    let body = child(&root, "body");
    assert_eq!(body.value, Value::U(42));
    assert_eq!(body.type_name, "u32");
    assert_eq!((body.offset, body.size), (1, 4));
    assert_eq!(root.size, 5);
}

#[test]
fn match_selects_string_variant() {
    // tag = 1 -> body is string[4].
    let root = run(TLV, "TLV", vec![1, b'H', b'i', 0, 0], Endian::Little);
    assert_eq!(child(&root, "body").value, Value::Str("Hi".into()));
    assert_eq!(child(&root, "body").size, 4);
}

#[test]
fn match_falls_back_to_default() {
    // tag = 9 -> no arm, so `default => bytes[2]`.
    let root = run(TLV, "TLV", vec![9, 0xAB, 0xCD], Endian::Little);
    let body = child(&root, "body");
    assert_eq!(body.value, Value::Bytes(vec![0xAB, 0xCD]));
    assert_eq!(body.size, 2);
}

#[test]
fn match_without_default_errors_on_unknown_tag() {
    let src = "struct S { tag u8  body match tag { 0 => u8 } }";
    let schema = schema_parser::parse(src).unwrap();
    // tag = 7, no matching arm and no default.
    let reader = BinaryReader::from_bytes(vec![7, 0]);
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

// --- Variable-length fields (Phase 11) -------------------------------------

#[test]
fn cstring_reads_up_to_and_including_nul() {
    // "Hi\0" then a trailing byte that a following field should read.
    let src = "struct S { name cstring  after u8 }";
    let root = run(src, "S", vec![b'H', b'i', 0, 0x42], Endian::Little);
    let name = child(&root, "name");
    assert_eq!(name.value, Value::Str("Hi".into()));
    assert_eq!((name.offset, name.size), (0, 3)); // includes the NUL
    assert_eq!(child(&root, "after").value, Value::U(0x42));
    assert_eq!(child(&root, "after").offset, 3);
}

#[test]
fn cstring_without_nul_reads_to_end() {
    let src = "struct S { name cstring }";
    let root = run(src, "S", vec![b'A', b'B', b'C'], Endian::Little);
    assert_eq!(child(&root, "name").value, Value::Str("ABC".into()));
    assert_eq!(child(&root, "name").size, 3);
}

#[test]
fn rest_bytes_consume_to_end_of_file() {
    let src = "struct S { head u16  rest bytes[*] }";
    let root = run(src, "S", vec![0x01, 0x02, 0xAA, 0xBB, 0xCC], Endian::Little);
    let rest = child(&root, "rest");
    assert_eq!(rest.value, Value::Bytes(vec![0xAA, 0xBB, 0xCC]));
    assert_eq!((rest.offset, rest.size), (2, 3));
    assert_eq!(root.size, 5);
}

#[test]
fn rest_string_reads_remaining_text() {
    let src = "struct S { n u8  text string[*] }";
    let root = run(src, "S", vec![9, b'h', b'i', b'!'], Endian::Little);
    assert_eq!(child(&root, "text").value, Value::Str("hi!".into()));
}

// --- Computed fields (Phase 11) --------------------------------------------

#[test]
fn computed_field_evaluates_and_consumes_no_bytes() {
    // total = 100, header = 12  ->  body = 88, reading no bytes.
    let src = "struct S { total u32  header u32  body = total - header }";
    let bytes = vec![100, 0, 0, 0, 12, 0, 0, 0];
    let root = run(src, "S", bytes, Endian::Little);
    let body = child(&root, "body");
    assert_eq!(body.value, Value::U(88));
    assert_eq!(body.size, 0); // computed fields occupy no bytes
    assert_eq!(root.size, 8); // total + header only
}

#[test]
fn computed_respects_precedence() {
    // c = a + b * 2  with a=1, b=3  ->  1 + 6 = 7
    let src = "struct S { a u8  b u8  c = a + b * 2 }";
    let root = run(src, "S", vec![1, 3], Endian::Little);
    assert_eq!(child(&root, "c").value, Value::U(7));
    // (a + b) * 2 -> 8
    let src = "struct S { a u8  b u8  c = (a + b) * 2 }";
    let root = run(src, "S", vec![1, 3], Endian::Little);
    assert_eq!(child(&root, "c").value, Value::U(8));
}

#[test]
fn computed_field_can_drive_an_array_length() {
    // count = n - 1 = 2, so `items` reads two u8 after n.
    let src = "struct S { n u8  count = n - 1  items u8[count] }";
    let root = run(src, "S", vec![3, 0xAA, 0xBB, 0xCC], Endian::Little);
    let items = child(&root, "items");
    assert_eq!(items.children.len(), 2);
    assert_eq!(items.children[0].value, Value::U(0xAA));
    assert_eq!(items.children[1].value, Value::U(0xBB));
    // The computed field itself sits between n and items at zero width.
    assert_eq!(child(&root, "count").value, Value::U(2));
}

#[test]
fn computed_division_by_zero_errors() {
    let schema = schema_parser::parse("struct S { z u8  q = 10 / z }").unwrap();
    let reader = BinaryReader::from_bytes(vec![0]);
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

#[test]
fn computed_negative_result_is_signed() {
    let src = "struct S { a u8  b u8  d = a - b }";
    // a = 3, b = 5 -> -2
    let root = run(src, "S", vec![3, 5], Endian::Little);
    assert_eq!(child(&root, "d").value, Value::I(-2));
}

// --- Tag-dispatched unions (string match) ----------------------------------

// A four-char chunk tag chooses the body type; `default` handles the rest.
const TAGGED: &str = "struct Chunk {
    tag  char[4]
    len  u32
    body match tag {
        \"NUMS\" => u32
        \"TEXT\" => string[len]
        default => bytes[len]
    }
}";

#[test]
fn match_dispatches_on_a_string_tag() {
    // tag \"NUMS\", len 4, body = u32 = 0x2A.
    let bytes = vec![
        b'N', b'U', b'M', b'S', 0x04, 0, 0, 0, 0x2A, 0, 0, 0,
    ];
    let root = run(TAGGED, "Chunk", bytes, Endian::Little);
    let body = child(&root, "body");
    assert_eq!(body.value, Value::U(0x2A));
    assert_eq!(body.type_name, "u32");
}

#[test]
fn match_string_tag_falls_back_to_default() {
    // tag \"XZ??\", len 2 -> default => bytes[2].
    let bytes = vec![b'X', b'Z', b'?', b'?', 0x02, 0, 0, 0, 0xAB, 0xCD];
    let root = run(TAGGED, "Chunk", bytes, Endian::Little);
    let body = child(&root, "body");
    assert_eq!(body.value, Value::Bytes(vec![0xAB, 0xCD]));
}

#[test]
fn match_string_tag_without_default_errors_on_miss() {
    let src = "struct C { tag char[2]  body match tag { \"Hi\" => u8 } }";
    let schema = schema_parser::parse(src).unwrap();
    let reader = BinaryReader::from_bytes(vec![b'N', b'o', 0]);
    assert!(parse(&schema, &reader, "C", Endian::Little).is_err());
}

// --- repeat T until <sentinel> ---------------------------------------------

const CHUNKS: &str = "struct Chunk {
    tag  char[4]
    len  u32
    body bytes[len]
}
struct File {
    chunks repeat Chunk until tag == \"ENDF\"
}";

#[test]
fn repeat_until_sentinel_includes_the_terminator() {
    // Chunk \"AAAA\" len 2 body [1,2], then Chunk \"ENDF\" len 0 (stops).
    let bytes = vec![
        b'A', b'A', b'A', b'A', 0x02, 0, 0, 0, 0x01, 0x02, // AAAA
        b'E', b'N', b'D', b'F', 0x00, 0, 0, 0, // ENDF, len 0
    ];
    let root = run(CHUNKS, "File", bytes, Endian::Little);
    let chunks = child(&root, "chunks");
    assert_eq!(chunks.children.len(), 2);
    // First element decoded normally.
    assert_eq!(child(&chunks.children[0], "tag").value, Value::Str("AAAA".into()));
    // Terminator element is present (inclusive) and is what stopped the loop.
    assert_eq!(child(&chunks.children[1], "tag").value, Value::Str("ENDF".into()));
    // The array spans exactly the two chunks (10 + 8 bytes).
    assert_eq!((chunks.offset, chunks.size), (0, 18));
}

#[test]
fn repeat_with_no_until_reads_to_end_of_file() {
    let src = "struct File { nums repeat u16 }";
    let bytes = vec![0x01, 0x00, 0x02, 0x00, 0x03, 0x00];
    let root = run(src, "File", bytes, Endian::Little);
    let nums = child(&root, "nums");
    assert_eq!(nums.children.len(), 3);
    assert_eq!(nums.children[2].value, Value::U(3));
    assert_eq!(nums.size, 6);
}

#[test]
fn repeat_leaves_trailing_fields_for_later_reads() {
    // After the sentinel chunk, a trailing u32 CRC must still be readable.
    let src = "struct Chunk { tag char[4]  len u32  body bytes[len] }
        struct File {
            chunks repeat Chunk until tag == \"ENDF\"
            crc u32
        }";
    let bytes = vec![
        b'E', b'N', b'D', b'F', 0x00, 0, 0, 0, // one ENDF chunk (stops at once)
        0xEF, 0xBE, 0xAD, 0xDE, // crc = 0xDEADBEEF
    ];
    let root = run(src, "File", bytes, Endian::Little);
    assert_eq!(child(&root, "chunks").children.len(), 1);
    assert_eq!(child(&root, "crc").value, Value::U(0xDEAD_BEEF));
}

// --- Flagship: a TLV game-save (repeat + tag-dispatch end to end) -----------

// The `hollow_ascent_save` shape from the roadmap, exercised as one entry
// struct: a header, a repeat of tagged chunks terminated by an ENDF sentinel,
// then a trailing CRC. Proves loops and string-dispatched unions compose.
const SAVE_SCHEMA: &str = "
struct Header {
    magic    char[4]
    version  u16
    flags    u16
    savedAt  bytes[4]
    playtime u32
    reserved bytes[4]
}
struct Plyr {
    nameLen u16
    name    string[nameLen]
    level   u8
    hp      u16
}
struct Meta {
    titleLen u16
    title    string[titleLen]
}
struct Chunk {
    tag  char[4]
    len  u32
    body match tag {
        \"PLYR\" => Plyr
        \"META\" => Meta
        default => bytes[len]
    }
}
struct SaveFile {
    header Header
    chunks repeat Chunk until tag == \"ENDF\"
    crc    u32
}";

fn tlv_chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(tag);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
}

#[test]
fn hollow_save_decodes_loops_and_tagged_chunks() {
    // ---- header (20 bytes) ----
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"HASV");
    bytes.extend_from_slice(&3u16.to_le_bytes()); // version
    bytes.extend_from_slice(&2u16.to_le_bytes()); // flags
    bytes.extend_from_slice(&0x6655_44_33u32.to_be_bytes()); // savedAt (opaque BE)
    bytes.extend_from_slice(&3600u32.to_le_bytes()); // playtime
    bytes.extend_from_slice(&[0, 0, 0, 0]); // reserved

    // ---- PLYR ----
    let mut plyr = Vec::new();
    let name = b"Wren Ashgrave";
    plyr.extend_from_slice(&(name.len() as u16).to_le_bytes());
    plyr.extend_from_slice(name);
    plyr.push(27); // level
    plyr.extend_from_slice(&184u16.to_le_bytes()); // hp
    tlv_chunk(&mut bytes, b"PLYR", &plyr);

    // ---- META ----
    let mut meta = Vec::new();
    let title = b"Hollow Ascent";
    meta.extend_from_slice(&(title.len() as u16).to_le_bytes());
    meta.extend_from_slice(title);
    tlv_chunk(&mut bytes, b"META", &meta);

    // ---- ENDF sentinel + CRC ----
    tlv_chunk(&mut bytes, b"ENDF", &[]);
    bytes.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());

    let root = run(SAVE_SCHEMA, "SaveFile", bytes, Endian::Little);

    // Header decoded.
    assert_eq!(child(child(&root, "header"), "magic").value, Value::Str("HASV".into()));

    // Three chunks: PLYR, META, ENDF (the terminator is included).
    let chunks = child(&root, "chunks");
    assert_eq!(chunks.children.len(), 3);

    // PLYR body dispatched to the Plyr struct.
    let plyr_body = child(&chunks.children[0], "body");
    assert_eq!(plyr_body.type_name, "Plyr");
    assert_eq!(child(plyr_body, "name").value, Value::Str("Wren Ashgrave".into()));
    assert_eq!(child(plyr_body, "level").value, Value::U(27));
    assert_eq!(child(plyr_body, "hp").value, Value::U(184));

    // META body dispatched to the Meta struct.
    let meta_body = child(&chunks.children[1], "body");
    assert_eq!(child(meta_body, "title").value, Value::Str("Hollow Ascent".into()));

    // Terminator and the trailing CRC after the loop.
    assert_eq!(child(&chunks.children[2], "tag").value, Value::Str("ENDF".into()));
    assert_eq!(child(&root, "crc").value, Value::U(0xDEAD_BEEF));
}

// --- decode / transforms (Phase 13) ----------------------------------------

#[test]
fn decode_xor_deobfuscates_bytes() {
    // 4 bytes XOR 0xA5 -> known plaintext.
    let src = "struct S { data bytes[4] decode xor(0xA5) }";
    let enc: Vec<u8> = vec![0x10 ^ 0xA5, 0x20 ^ 0xA5, 0x30 ^ 0xA5, 0x40 ^ 0xA5];
    let root = run(src, "S", enc, Endian::Little);
    assert_eq!(child(&root, "data").value, Value::Bytes(vec![0x10, 0x20, 0x30, 0x40]));
    // The node still spans the original 4 encoded bytes.
    assert_eq!((child(&root, "data").offset, child(&root, "data").size), (0, 4));
}

#[test]
fn decode_add_wraps() {
    let src = "struct S { data bytes[3] decode add(1) }";
    let root = run(src, "S", vec![0x00, 0x7F, 0xFF], Endian::Little);
    assert_eq!(child(&root, "data").value, Value::Bytes(vec![0x01, 0x80, 0x00]));
}

#[test]
fn decode_base64_to_text() {
    // "Hello" base64 = "SGVsbG8="
    let src = "struct S { data bytes[8] decode base64 }";
    let root = run(src, "S", b"SGVsbG8=".to_vec(), Endian::Little);
    assert_eq!(child(&root, "data").value, Value::Bytes(b"Hello".to_vec()));
}

#[test]
fn decode_xor_as_struct_reparses() {
    // XOR-masked little-endian u32 (42) + u16 (7), then decode + reinterpret.
    let src = "struct Inner { a u32  b u16 }
        struct S { blob bytes[6] decode xor(0xFF) as Inner }";
    let mut enc = Vec::new();
    enc.extend_from_slice(&42u32.to_le_bytes());
    enc.extend_from_slice(&7u16.to_le_bytes());
    for byte in enc.iter_mut() { *byte ^= 0xFF; }
    let root = run(src, "S", enc, Endian::Little);
    let blob = child(&root, "blob");
    assert_eq!(child(blob, "a").value, Value::U(42));
    assert_eq!(child(blob, "b").value, Value::U(7));
    // Encoded span is preserved on the parent.
    assert_eq!((blob.offset, blob.size), (0, 6));
}

#[test]
fn decode_zlib_inflate_as_struct() {
    // THE launch feature: a zlib-compressed struct, inflated + parsed inline.
    // Build the plaintext (u16 count=3, u16 first=0xBEEF), compress with zlib.
    let mut plain = Vec::new();
    plain.extend_from_slice(&3u16.to_le_bytes());
    plain.extend_from_slice(&0xBEEFu16.to_le_bytes());
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write;
    enc.write_all(&plain).unwrap();
    let compressed = enc.finish().unwrap();

    let src = "struct Payload { count u16  first u16 }
        struct S { blob bytes[*] decode zlib_inflate as Payload }";
    let root = run(src, "S", compressed.clone(), Endian::Little);
    let blob = child(&root, "blob");
    assert_eq!(child(blob, "count").value, Value::U(3));
    assert_eq!(child(blob, "first").value, Value::U(0xBEEF));
    // Node still covers the compressed bytes in the file.
    assert_eq!(blob.size, compressed.len());
}

#[test]
fn decode_gunzip_roundtrip() {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(b"gzipped!").unwrap();
    let gz = enc.finish().unwrap();
    let src = "struct S { blob bytes[*] decode gunzip }";
    let root = run(src, "S", gz, Endian::Little);
    assert_eq!(child(&root, "blob").value, Value::Bytes(b"gzipped!".to_vec()));
}

#[test]
fn decode_bad_zlib_errors_not_panics() {
    let src = "struct S { blob bytes[*] decode zlib_inflate }";
    let schema = schema_parser::parse(src).unwrap();
    let reader = BinaryReader::from_bytes(vec![0, 1, 2, 3, 4]); // not zlib
    assert!(parse(&schema, &reader, "S", Endian::Little).is_err());
}

// --- varints / LEB128 (Phase 14) -------------------------------------------

#[test]
fn varint_single_and_multi_byte() {
    // a: 0x7F -> 127 (1 byte); b: 0xAC 0x02 -> 300 (2 bytes, LSB group first).
    let root = run(
        "struct S { a varint  b varint }",
        "S",
        vec![0x7F, 0xAC, 0x02],
        Endian::Little,
    );
    let a = child(&root, "a");
    let b = child(&root, "b");
    assert_eq!(a.value, Value::U(127));
    assert_eq!(a.offset, 0);
    assert_eq!(a.size, 1);
    assert_eq!(a.type_name, "varint");
    // The second varint must start right after the first's variable width.
    assert_eq!(b.value, Value::U(300));
    assert_eq!(b.offset, 1);
    assert_eq!(b.size, 2);
    assert_eq!(root.size, 3);
}

#[test]
fn svarint_sign_extends() {
    // -1 encodes as a single 0x7F byte; 64 needs two bytes (0xC0 0x00) so its
    // set bit-6 isn't misread as a sign bit.
    let root = run(
        "struct S { neg svarint  pos svarint }",
        "S",
        vec![0x7F, 0xC0, 0x00],
        Endian::Little,
    );
    assert_eq!(child(&root, "neg").value, Value::I(-1));
    assert_eq!(child(&root, "neg").size, 1);
    assert_eq!(child(&root, "pos").value, Value::I(64));
    assert_eq!(child(&root, "pos").size, 2);
}

#[test]
fn varint_can_drive_an_array_length() {
    // A varint count followed by that many bytes — a very common shape.
    let root = run(
        "struct S { n varint  data u8[n] }",
        "S",
        vec![0x03, 0xAA, 0xBB, 0xCC],
        Endian::Little,
    );
    assert_eq!(child(&root, "n").value, Value::U(3));
    let data = child(&root, "data");
    assert_eq!(data.children.len(), 3);
    assert_eq!(data.children[2].value, Value::U(0xCC));
}

#[test]
fn packed_varint_array() {
    // varint[3]: three back-to-back varints of differing widths.
    // 0x01 -> 1 | 0x96 0x01 -> 150 | 0x05 -> 5
    let root = run(
        "struct S { xs varint[3] }",
        "S",
        vec![0x01, 0x96, 0x01, 0x05],
        Endian::Little,
    );
    let xs = child(&root, "xs");
    assert_eq!(xs.children[0].value, Value::U(1));
    assert_eq!(xs.children[1].value, Value::U(150));
    assert_eq!(xs.children[2].value, Value::U(5));
    assert_eq!(xs.size, 4);
}

#[test]
fn protobuf_field_08_96_01() {
    // The canonical protobuf example: `08 96 01` is field #1 (wire type 0) = 150.
    // key = 0x08 (tag<<3 | wiretype), value = 150.
    let root = run(
        "struct Field { key varint  value varint }",
        "Field",
        vec![0x08, 0x96, 0x01],
        Endian::Little,
    );
    assert_eq!(child(&root, "key").value, Value::U(8));
    let value = child(&root, "value");
    assert_eq!(value.value, Value::U(150));
    assert_eq!(value.offset, 1);
    assert_eq!(value.size, 2);
}

#[test]
fn overlong_varint_errors_not_panics() {
    // 11 bytes that never clear the continuation bit: must error, not spin/panic.
    let schema = schema_parser::parse("struct S { a varint }").expect("schema parses");
    let reader = BinaryReader::from_bytes(vec![0x80; 11]);
    let err = parse(&schema, &reader, "S", Endian::Little).unwrap_err();
    assert!(
        err.to_string().contains("varint"),
        "expected a varint error, got: {err}"
    );
}
