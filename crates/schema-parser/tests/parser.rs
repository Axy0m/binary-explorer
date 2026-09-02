//! Integration tests for the schema parser, driven mostly by the examples in
//! the product plan and `docs/schema-language.md`.

use schema::{Len, Prim, TypeExpr};
use schema_parser::{parse, ParseError};

/// Convenience: parse and return the single struct's fields as (name, type).
fn fields_of(src: &str) -> Vec<(String, TypeExpr)> {
    let schema = parse(src).expect("should parse");
    assert_eq!(schema.structs.len(), 1, "expected exactly one struct");
    schema.structs[0]
        .fields
        .iter()
        .map(|f| (f.name.clone(), f.ty.clone()))
        .collect()
}

#[test]
fn header_example_from_the_plan() {
    let src = "
        struct Header {
            magic   char[4]
            version u16
            flags   u16
            size    u32
        }
    ";
    let fields = fields_of(src);
    assert_eq!(
        fields,
        vec![
            (
                "magic".to_string(),
                TypeExpr::Array {
                    elem: Box::new(TypeExpr::Char),
                    len: Len::Fixed(4),
                }
            ),
            ("version".to_string(), TypeExpr::Prim(Prim::U16)),
            ("flags".to_string(), TypeExpr::Prim(Prim::U16)),
            ("size".to_string(), TypeExpr::Prim(Prim::U32)),
        ]
    );
}

#[test]
fn player_example_with_string_and_floats() {
    let src = "
        struct Player {
            id     u32
            health f32
            mana   f32
            name   string[32]
        }
    ";
    let fields = fields_of(src);
    assert_eq!(fields[0].1, TypeExpr::Prim(Prim::U32));
    assert_eq!(fields[1].1, TypeExpr::Prim(Prim::F32));
    assert_eq!(fields[2].1, TypeExpr::Prim(Prim::F32));
    assert_eq!(fields[3].1, TypeExpr::Str(Len::Fixed(32)));
}

#[test]
fn all_primitive_keywords_map_correctly() {
    let src = "
        struct P {
            a u8  b u16 c u32 d u64
            e i8  f i16 g i32 h i64
            i f32 j f64 k bool
        }
    ";
    let fields = fields_of(src);
    let types: Vec<TypeExpr> = fields.into_iter().map(|(_, t)| t).collect();
    use Prim::*;
    let want = [
        U8, U16, U32, U64, I8, I16, I32, I64, F32, F64, Bool,
    ];
    assert_eq!(types.len(), want.len());
    for (got, w) in types.iter().zip(want) {
        assert_eq!(*got, TypeExpr::Prim(w));
    }
}

#[test]
fn field_reference_array_length() {
    // The forward-looking case: an array whose length is an earlier field.
    let src = "
        struct File {
            header       Header
            player_count u32
            players      Player[player_count]
        }
    ";
    let fields = fields_of(src);
    assert_eq!(fields[0].1, TypeExpr::Named("Header".to_string()));
    assert_eq!(fields[1].1, TypeExpr::Prim(Prim::U32));
    assert_eq!(
        fields[2].1,
        TypeExpr::Array {
            elem: Box::new(TypeExpr::Named("Player".to_string())),
            len: Len::Field("player_count".to_string()),
        }
    );
}

#[test]
fn bytes_type_needs_a_length() {
    assert_eq!(
        fields_of("struct B { blob bytes[16] }")[0].1,
        TypeExpr::Bytes(Len::Fixed(16)),
    );
}

#[test]
fn multiple_structs_preserve_order() {
    let schema = parse("struct A { x u8 } struct B { y u8 }").unwrap();
    let names: Vec<&str> = schema.structs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["A", "B"]);
    assert!(schema.struct_named("B").is_some());
    assert!(schema.struct_named("C").is_none());
}

#[test]
fn comments_and_irregular_whitespace_are_ignored() {
    let src = "
        // a leading comment
        struct Tiny {
            only u8 // trailing comment
        }
    ";
    let fields = fields_of(src);
    assert_eq!(fields, vec![("only".to_string(), TypeExpr::Prim(Prim::U8))]);
}

#[test]
fn field_descriptions_are_parsed() {
    let schema = parse("struct H { version u16 \"schema revision\"  size u32 }").unwrap();
    let fields = &schema.structs[0].fields;
    assert_eq!(fields[0].desc.as_deref(), Some("schema revision"));
    assert_eq!(fields[1].desc, None); // no description on size
}

#[test]
fn description_escapes_and_unterminated() {
    let ok = parse("struct H { x u8 \"a \\\"quoted\\\" word\" }").unwrap();
    assert_eq!(ok.structs[0].fields[0].desc.as_deref(), Some("a \"quoted\" word"));
    assert!(matches!(
        parse("struct H { x u8 \"oops }").unwrap_err(),
        ParseError::UnterminatedString { .. }
    ));
}

#[test]
fn empty_source_is_an_empty_schema() {
    let schema = parse("   \n // nothing here \n  ").unwrap();
    assert!(schema.structs.is_empty());
}

// --- error cases -----------------------------------------------------------

#[test]
fn string_without_length_is_rejected() {
    let err = parse("struct S { name string }").unwrap_err();
    assert!(
        matches!(err, ParseError::LengthRequired { ref ty, .. } if ty == "string"),
        "got {err:?}"
    );
}

#[test]
fn missing_closing_brace_is_eof_error() {
    let err = parse("struct S { x u8 ").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }), "got {err:?}");
}

#[test]
fn unexpected_top_level_token() {
    let err = parse("u32 x").unwrap_err();
    assert!(
        matches!(err, ParseError::UnexpectedToken { .. }),
        "got {err:?}"
    );
}

#[test]
fn duplicate_struct_is_rejected() {
    let err = parse("struct A { x u8 } struct A { y u8 }").unwrap_err();
    assert!(
        matches!(err, ParseError::DuplicateType { ref name, .. } if name == "A"),
        "got {err:?}"
    );
}

#[test]
fn stray_character_reports_line_and_column() {
    let err = parse("struct S {\n  x u8 $\n}").unwrap_err();
    match err {
        ParseError::UnexpectedChar { ch, span } => {
            assert_eq!(ch, '$');
            assert_eq!(span.line, 2);
        }
        other => panic!("expected UnexpectedChar, got {other:?}"),
    }
}

#[test]
fn number_glued_to_letters_is_rejected() {
    let err = parse("struct S { x u8[12ab] }").unwrap_err();
    assert!(
        matches!(err, ParseError::UnexpectedChar { .. }),
        "got {err:?}"
    );
}

// --- Enums & bitfields (Phase 11) ------------------------------------------

#[test]
fn enum_parses_repr_and_variants() {
    let schema = parse(
        "enum ColorType : u8 {
            Grayscale = 0
            RGB = 2
            RGBA = 6
        }",
    )
    .expect("should parse");
    assert_eq!(schema.enums.len(), 1);
    let e = &schema.enums[0];
    assert_eq!(e.name, "ColorType");
    assert_eq!(e.repr, Prim::U8);
    assert_eq!(e.variants.len(), 3);
    assert_eq!(e.name_of(6), Some("RGBA"));
    assert_eq!(e.name_of(1), None);
}

#[test]
fn bitfield_single_and_range_members() {
    let schema = parse(
        "bitfield Flags : u16 {
            compressed 0
            encrypted 1
            level 2..4
        }",
    )
    .expect("should parse");
    assert_eq!(schema.bitfields.len(), 1);
    let b = &schema.bitfields[0];
    assert_eq!(b.repr, Prim::U16);
    assert_eq!(b.members.len(), 3);
    assert_eq!((b.members[0].lo, b.members[0].hi), (0, 0));
    assert_eq!((b.members[2].lo, b.members[2].hi), (2, 4));
    assert_eq!(b.members[2].width(), 3);
}

#[test]
fn enum_and_struct_can_reference_each_other() {
    let schema = parse(
        "enum Kind : u8 { A = 1  B = 2 }
         struct Rec { kind Kind  n u32 }",
    )
    .expect("should parse");
    assert_eq!(schema.enums.len(), 1);
    assert_eq!(schema.structs.len(), 1);
    assert_eq!(schema.structs[0].fields[0].ty, TypeExpr::Named("Kind".into()));
}

#[test]
fn hex_literals_are_accepted() {
    // ELF's GNU segment types use hex enum values like 0x6474e550.
    let schema = parse("enum E : u32 { EhFrame = 0x6474e550  Stack = 0X6474E551 }")
        .expect("should parse");
    assert_eq!(schema.enums[0].variants[0].value, 0x6474e550);
    assert_eq!(schema.enums[0].variants[1].value, 0x6474e551);
}

#[test]
fn bare_hex_prefix_is_rejected() {
    assert!(parse("enum E : u8 { A = 0x }").is_err());
}

/// Every schema the app bundles must parse — this guards the DSL against
/// regressions that would break format auto-loading (it caught the missing
/// hex-literal support when the ELF schema was added).
#[test]
fn all_bundled_schemas_parse() {
    let bundled: &[(&str, &str)] = &[
        ("png", include_str!("../../../schemas/png.schema")),
        ("gzip", include_str!("../../../schemas/gzip.schema")),
        ("elf", include_str!("../../../schemas/elf.schema")),
        ("zip", include_str!("../../../schemas/zip.schema")),
        ("pe", include_str!("../../../schemas/pe.schema")),
        ("sqlite", include_str!("../../../schemas/sqlite.schema")),
        ("macho", include_str!("../../../schemas/macho.schema")),
        ("pcap", include_str!("../../../schemas/pcap.schema")),
    ];
    for (name, text) in bundled {
        parse(text).unwrap_or_else(|e| panic!("bundled schema {name} failed to parse: {e:?}"));
    }
}

#[test]
fn non_integer_repr_is_rejected() {
    let err = parse("enum E : f32 { A = 0 }").unwrap_err();
    assert!(matches!(err, ParseError::NonIntegerRepr { .. }), "got {err:?}");
}

#[test]
fn bit_out_of_range_is_rejected() {
    let err = parse("bitfield B : u8 { high 8 }").unwrap_err();
    assert!(matches!(err, ParseError::BitOutOfRange { bit: 8, bits: 8, .. }), "got {err:?}");
}

#[test]
fn duplicate_name_across_kinds_is_rejected() {
    let err = parse("struct X { a u8 } enum X : u8 { A = 0 }").unwrap_err();
    assert!(
        matches!(err, ParseError::DuplicateType { ref name, .. } if name == "X"),
        "got {err:?}"
    );
}

// --- Conditional fields (Phase 11) -----------------------------------------

#[test]
fn conditional_field_truthiness() {
    let schema = parse("struct S { present u8  extra u32 if present }").expect("should parse");
    let f = &schema.structs[0].fields[1];
    assert_eq!(f.name, "extra");
    let cond = f.condition.as_ref().expect("has condition");
    assert_eq!(cond.path, vec!["present".to_string()]);
    assert!(cond.compare.is_none());
}

#[test]
fn conditional_field_with_comparison_and_dotted_path() {
    use schema::CompareOp;
    let schema = parse(
        "bitfield Fl : u8 { extra 2 }
         struct S { flags Fl  xlen u16 if flags.extra  ver u8  body u32 if ver >= 2 }",
    )
    .expect("should parse");
    let fields = &schema.structs[0].fields;

    let xlen = fields.iter().find(|f| f.name == "xlen").unwrap();
    let c = xlen.condition.as_ref().unwrap();
    assert_eq!(c.path, vec!["flags".to_string(), "extra".to_string()]);
    assert!(c.compare.is_none());

    let body = fields.iter().find(|f| f.name == "body").unwrap();
    let c = body.condition.as_ref().unwrap();
    assert_eq!(c.path, vec!["ver".to_string()]);
    let cmp = c.compare.as_ref().unwrap();
    assert_eq!(cmp.op, CompareOp::Ge);
    assert_eq!(cmp.value, schema::CompareValue::Int(2));
}

#[test]
fn condition_and_description_coexist() {
    let schema = parse(r#"struct S { n u8  v u32 if n == 1 "the payload" }"#).expect("should parse");
    let v = &schema.structs[0].fields[1];
    assert!(v.condition.is_some());
    assert_eq!(v.desc.as_deref(), Some("the payload"));
}

// --- Pointer / relative-offset fields (Phase 11) ---------------------------

#[test]
fn absolute_pointer_field() {
    let schema = parse("struct S { off u32  ifd at off u16 }").expect("should parse");
    let f = &schema.structs[0].fields[1];
    let p = f.pointer.as_ref().expect("has pointer");
    assert_eq!(p.offset, Len::Field("off".into()));
    assert!(!p.relative);
}

#[test]
fn relative_pointer_and_literal_offset() {
    let schema = parse("struct S { hdr at +16 u32  fixed at 100 u8 }").expect("should parse");
    let fields = &schema.structs[0].fields;
    let hdr = fields.iter().find(|f| f.name == "hdr").unwrap();
    let p = hdr.pointer.as_ref().unwrap();
    assert_eq!(p.offset, Len::Fixed(16));
    assert!(p.relative);
    let fixed = fields.iter().find(|f| f.name == "fixed").unwrap();
    assert_eq!(fixed.pointer.as_ref().unwrap().offset, Len::Fixed(100));
    assert!(!fixed.pointer.as_ref().unwrap().relative);
}

#[test]
fn pointer_to_array_with_field_length() {
    // A pointer whose target is a field-length array (ELF-style program headers).
    let schema = parse("struct Elf { phnum u16  phoff u32  phs at phoff u32[phnum] }")
        .expect("should parse");
    let phs = &schema.structs[0].fields[2];
    assert!(phs.pointer.is_some());
    assert!(matches!(phs.ty, TypeExpr::Array { .. }));
}

// --- Discriminated unions / match fields (Phase 11) ------------------------

#[test]
fn match_field_with_arms_and_default() {
    let schema = parse(
        "struct TLV {
            tag u8
            body match tag {
                0 => u32
                1 => string[16]
                default => bytes[8]
            }
        }",
    )
    .expect("should parse");
    let body = &schema.structs[0].fields[1];
    match &body.ty {
        TypeExpr::Match { discriminant, arms, default } => {
            assert_eq!(discriminant, &vec!["tag".to_string()]);
            assert_eq!(arms.len(), 2);
            assert_eq!(arms[0].key, schema::MatchKey::Int(0));
            assert_eq!(arms[0].ty, TypeExpr::Prim(Prim::U32));
            assert!(default.is_some());
        }
        other => panic!("expected a match type, got {other:?}"),
    }
}

#[test]
fn match_on_dotted_discriminant() {
    let schema = parse(
        "struct Rec { kind u8  body match kind { 5 => u16 } }",
    )
    .expect("should parse");
    assert!(matches!(schema.structs[0].fields[1].ty, TypeExpr::Match { .. }));
}

#[test]
fn match_missing_arrow_is_rejected() {
    let err = parse("struct S { t u8  b match t { 0 u32 } }").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { .. }), "got {err:?}");
}

// --- Variable-length fields (Phase 11) -------------------------------------

#[test]
fn cstring_and_rest_lengths_parse() {
    let schema = parse("struct S { name cstring  tail bytes[*]  text string[*] }")
        .expect("should parse");
    let f = &schema.structs[0].fields;
    assert_eq!(f[0].ty, TypeExpr::CStr);
    assert_eq!(f[1].ty, TypeExpr::Bytes(Len::Rest));
    assert_eq!(f[2].ty, TypeExpr::Str(Len::Rest));
}

#[test]
fn array_of_cstrings_parses() {
    let schema = parse("struct S { n u8  names cstring[n] }").expect("should parse");
    match &schema.structs[0].fields[1].ty {
        TypeExpr::Array { elem, len } => {
            assert_eq!(**elem, TypeExpr::CStr);
            assert_eq!(*len, Len::Field("n".into()));
        }
        other => panic!("expected array of cstring, got {other:?}"),
    }
}

// --- Computed fields (Phase 11) --------------------------------------------

#[test]
fn computed_field_parses_expression() {
    use schema::{BinOp, Expr};
    let schema = parse("struct S { total u32  header u32  body = total - header }")
        .expect("should parse");
    let body = &schema.structs[0].fields[2];
    match &body.ty {
        TypeExpr::Computed(expr) => match &**expr {
            Expr::Binary { op, lhs, rhs } => {
                assert_eq!(*op, BinOp::Sub);
                assert_eq!(**lhs, Expr::Field(vec!["total".into()]));
                assert_eq!(**rhs, Expr::Field(vec!["header".into()]));
            }
            other => panic!("expected a binary expr, got {other:?}"),
        },
        other => panic!("expected a computed field, got {other:?}"),
    }
}

#[test]
fn computed_precedence_and_parens() {
    use schema::{BinOp, Expr};
    // a + b * 2  parses as  a + (b * 2)
    let schema = parse("struct S { a u8  b u8  c = a + b * 2 }").expect("should parse");
    match &schema.structs[0].fields[2].ty {
        TypeExpr::Computed(expr) => match &**expr {
            Expr::Binary { op: BinOp::Add, rhs, .. } => {
                assert!(matches!(&**rhs, Expr::Binary { op: BinOp::Mul, .. }));
            }
            other => panic!("expected add at the root, got {other:?}"),
        },
        _ => panic!("expected computed"),
    }
    // Parentheses override precedence: (a + b) * 2
    let schema = parse("struct S { a u8  b u8  c = (a + b) * 2 }").expect("should parse");
    match &schema.structs[0].fields[2].ty {
        TypeExpr::Computed(expr) => {
            assert!(matches!(&**expr, Expr::Binary { op: BinOp::Mul, .. }));
        }
        _ => panic!("expected computed"),
    }
}

// --- Tag-dispatch + repeat (Phase 12) --------------------------------------

#[test]
fn match_accepts_quoted_string_arms() {
    let schema = parse(
        "struct C {
            tag char[4]
            body match tag {
                \"PLYR\" => u32
                \"ENDF\" => u8
                default => bytes[4]
            }
        }",
    )
    .expect("should parse");
    match &schema.structs[0].fields[1].ty {
        TypeExpr::Match { arms, default, .. } => {
            assert_eq!(arms.len(), 2);
            assert_eq!(arms[0].key, schema::MatchKey::Str("PLYR".into()));
            assert_eq!(arms[1].key, schema::MatchKey::Str("ENDF".into()));
            assert!(default.is_some());
        }
        other => panic!("expected match, got {other:?}"),
    }
}

#[test]
fn repeat_until_string_sentinel_parses() {
    let schema = parse(
        "struct Chunk { tag char[4]  len u32  body bytes[len] }
         struct File { chunks repeat Chunk until tag == \"ENDF\" }",
    )
    .expect("should parse");
    match &schema.structs[1].fields[0].ty {
        TypeExpr::Repeat { elem, until } => {
            assert_eq!(**elem, TypeExpr::Named("Chunk".into()));
            let cond = until.as_ref().expect("has an until clause");
            assert_eq!(cond.path, vec!["tag".to_string()]);
            let cmp = cond.compare.as_ref().unwrap();
            assert_eq!(cmp.value, schema::CompareValue::Str("ENDF".into()));
        }
        other => panic!("expected repeat, got {other:?}"),
    }
}

#[test]
fn repeat_without_until_parses() {
    let schema = parse("struct File { nums repeat u16 }").expect("should parse");
    match &schema.structs[0].fields[0].ty {
        TypeExpr::Repeat { elem, until } => {
            assert_eq!(**elem, TypeExpr::Prim(schema::Prim::U16));
            assert!(until.is_none());
        }
        other => panic!("expected repeat, got {other:?}"),
    }
}

// --- decode / transforms (Phase 13) ----------------------------------------

#[test]
fn decode_clause_parses_with_args_and_as_type() {
    let schema = parse(
        "struct Inner { a u32 }
         struct S { blob bytes[8] decode rolling_xor(90, 31, 17) as Inner }",
    )
    .expect("should parse");
    let f = &schema.structs[1].fields[0];
    let d = f.decode.as_ref().expect("has decode");
    assert_eq!(d.transform, schema::Transform::RollingXor { seed: 90, mul: 31, add: 17 });
    assert_eq!(d.as_type.as_deref(), Some(&TypeExpr::Named("Inner".into())));
}

#[test]
fn decode_zlib_without_as_parses() {
    let schema = parse("struct S { blob bytes[*] decode zlib_inflate }").expect("should parse");
    let d = schema.structs[0].fields[0].decode.as_ref().unwrap();
    assert_eq!(d.transform, schema::Transform::ZlibInflate);
    assert!(d.as_type.is_none());
}

#[test]
fn decode_unknown_transform_is_rejected() {
    let err = parse("struct S { x bytes[4] decode wobble }").unwrap_err();
    assert!(matches!(err, ParseError::UnknownTransform { .. }), "got {err:?}");
}

#[test]
fn decode_wrong_arity_is_rejected() {
    let err = parse("struct S { x bytes[4] decode rolling_xor(1, 2) }").unwrap_err();
    assert!(matches!(err, ParseError::BadTransformArgs { .. }), "got {err:?}");
}

// --- varints / LEB128 (Phase 14) -------------------------------------------

#[test]
fn varint_and_svarint_parse_as_varint_types() {
    let fields = fields_of("struct S { a varint  b svarint }");
    assert_eq!(fields[0].1, TypeExpr::Varint { signed: false });
    assert_eq!(fields[1].1, TypeExpr::Varint { signed: true });
}

#[test]
fn varint_array_parses() {
    // Packed repeated varints: `varint[n]` is an array of varints.
    let fields = fields_of("struct S { count u8  tags varint[count] }");
    match &fields[1].1 {
        TypeExpr::Array { elem, len } => {
            assert_eq!(**elem, TypeExpr::Varint { signed: false });
            assert_eq!(*len, Len::Field("count".into()));
        }
        other => panic!("expected array of varint, got {other:?}"),
    }
}
