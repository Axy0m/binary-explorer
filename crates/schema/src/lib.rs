//! Schema AST / IR for Binary Explorer's DSL.
//!
//! This crate is pure data (see the "Product Architecture Principle" in the
//! plan): it defines the *shape* a schema takes once parsed. It knows nothing
//! about how the text was tokenized (that's `schema-parser`) or how the schema
//! is executed against bytes (that's the future `schema-runtime`).
//!
//! The target surface syntax lives in `docs/schema-language.md`. A schema is a
//! set of named `struct`s, each a sequence of named fields:
//!
//! ```text
//! struct Header {
//!     magic   char[4]
//!     version u16
//!     size    u32
//! }
//! ```

use serde::{Deserialize, Serialize};

/// A whole schema: an ordered set of type definitions.
///
/// Order is preserved as written so tooling can round-trip the source and so
/// the first struct can act as the document's entry point. Structs, enums, and
/// bitfields share one name space; all three are referenced by name from a
/// field's [`TypeExpr::Named`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schema {
    pub structs: Vec<StructDef>,
    #[serde(default)]
    pub enums: Vec<EnumDef>,
    #[serde(default)]
    pub bitfields: Vec<BitfieldDef>,
}

impl Schema {
    /// Build a schema of structs only (enums/bitfields empty).
    pub fn new(structs: Vec<StructDef>) -> Self {
        Self {
            structs,
            enums: Vec::new(),
            bitfields: Vec::new(),
        }
    }

    /// Look up a struct definition by name.
    pub fn struct_named(&self, name: &str) -> Option<&StructDef> {
        self.structs.iter().find(|s| s.name == name)
    }

    /// Look up an enum definition by name.
    pub fn enum_named(&self, name: &str) -> Option<&EnumDef> {
        self.enums.iter().find(|e| e.name == name)
    }

    /// Look up a bitfield definition by name.
    pub fn bitfield_named(&self, name: &str) -> Option<&BitfieldDef> {
        self.bitfields.iter().find(|b| b.name == name)
    }
}

/// A named enum: an integer whose values map to symbolic names,
/// `enum Name : <repr> { Variant = <int> ... }`. `repr` is the underlying
/// integer primitive that is actually read from the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumDef {
    pub name: String,
    pub repr: Prim,
    pub variants: Vec<EnumVariant>,
}

impl EnumDef {
    /// The symbolic name for a decoded value, if any variant matches.
    pub fn name_of(&self, value: i64) -> Option<&str> {
        self.variants
            .iter()
            .find(|v| v.value == value)
            .map(|v| v.name.as_str())
    }
}

/// One `Name = value` pair inside an enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub value: i64,
}

/// A named bitfield: an integer unpacked into named single-bit flags or
/// multi-bit sub-fields, `bitfield Name : <repr> { flag <bit>  field <lo>..<hi> }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitfieldDef {
    pub name: String,
    pub repr: Prim,
    pub members: Vec<BitMember>,
}

/// One member of a bitfield: an inclusive bit range `[lo, hi]` (a single bit
/// when `lo == hi`), counting bit 0 as the least-significant bit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitMember {
    pub name: String,
    pub lo: u8,
    pub hi: u8,
}

impl BitMember {
    /// Number of bits this member spans.
    pub fn width(&self) -> u8 {
        self.hi - self.lo + 1
    }
}

/// A named struct: `struct Name { field* }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
}

/// A single field inside a struct: `name [at src] type` with an optional `if`
/// condition and an optional trailing description string, e.g.
/// `xlen u16 if flags.extra "extra-field length"` or
/// `ifd at ifdOffset Ifd "image file directory"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub ty: TypeExpr,
    /// When present, the field is read at an offset held elsewhere (a pointer),
    /// rather than at the current sequential cursor; it consumes no bytes in the
    /// enclosing struct's layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Pointer>,
    /// When present, the field is only read if this condition holds; otherwise
    /// it is skipped entirely (consuming no bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    /// When present, the field's raw bytes are passed through a transform
    /// (de-obfuscation or decompression) after reading — `bytes[n] decode
    /// zlib_inflate as Header`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode: Option<Decode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// A post-read transform on a field's raw bytes: `decode <transform> [as <Type>]`.
///
/// The field must read a byte run (`bytes[n]` or `bytes[*]`); those bytes are
/// transformed, and either shown as the decoded bytes or — with `as <Type>` —
/// re-parsed as that type on the decoded buffer (inline decompression /
/// de-obfuscation into a structured view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decode {
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_type: Option<Box<TypeExpr>>,
}

/// A byte-buffer transform. Only inert, keyless transforms live here; keyed
/// cryptography is deliberately left to the future sandboxed plugin tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transform {
    /// Repeating-key XOR (a single byte, or a multi-byte key cycled over the input).
    Xor(Vec<u8>),
    /// Rolling XOR: the key starts at `seed` and updates `k = k*mul + add`
    /// (mod 256) after each byte — a common lightweight save-file obfuscation.
    RollingXor { seed: u8, mul: u8, add: u8 },
    /// Add a constant to every byte (mod 256); a negative constant subtracts.
    Add(i64),
    /// Standard-alphabet Base64 decode.
    Base64,
    /// zlib-stream inflate (RFC 1950 — 2-byte header + Adler-32 trailer).
    ZlibInflate,
    /// Raw DEFLATE inflate (RFC 1951 — no header or trailer).
    Inflate,
    /// gzip-member inflate (RFC 1952 — gzip header + DEFLATE + CRC/size).
    Gunzip,
}

/// A pointer-follow directive on a field: `at [+] <offset>`.
///
/// `offset` supplies the byte position to read the field's type from — either a
/// literal or an earlier field's value. When `relative` is set (`at +off`) the
/// offset is added to the enclosing struct's start; otherwise it is an absolute
/// file offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer {
    pub offset: Len,
    #[serde(default)]
    pub relative: bool,
}

/// A guard on a conditional field: `if <path> [<op> <int>]`.
///
/// `path` names an earlier field, optionally dotted into a bitfield/struct
/// member (e.g. `flags.extra`). With no operator the test is truthiness (the
/// referenced value is non-zero); with one, it compares against an integer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare: Option<Compare>,
}

/// The comparison half of a [`Condition`]: an operator and a right-hand value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compare {
    pub op: CompareOp,
    pub value: CompareValue,
}

/// The right-hand side of a [`Compare`]: either an integer (compared against a
/// numeric field) or a string literal (compared against a text field such as a
/// `char[4]` tag). Strings support only equality/inequality meaningfully; other
/// operators fall back to lexicographic order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareValue {
    Int(i64),
    Str(String),
}

/// A comparison operator usable in a conditional field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// The type of a field.
///
/// Every multi-byte primitive is read with the schema's endianness (resolved by
/// the runtime, default little-endian); a fixed-size primitive knows its own
/// width. Composite shapes (`string[N]`, `T[N]`, struct references) carry the
/// extra info needed to size and walk them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeExpr {
    /// A fixed-width numeric or boolean primitive.
    Prim(Prim),
    /// A variable-length LEB128 integer. `signed` selects unsigned LEB128
    /// (`varint`) versus signed, sign-extended SLEB128 (`svarint`). It consumes
    /// as many bytes as the encoding needs (1–10 for a 64-bit value) — its width
    /// is not known until read, unlike a [`Prim`]. Ubiquitous in protobuf, WASM,
    /// DEX, and other compact formats.
    Varint { signed: bool },
    /// A single 1-byte character.
    Char,
    /// A fixed-length text string of `len` bytes, e.g. `string[32]`.
    Str(Len),
    /// A NUL-terminated string, e.g. `cstring` (a C string). Reads up to and
    /// including the terminating NUL; the decoded value omits the NUL.
    CStr,
    /// A fixed run of `len` raw bytes, e.g. `bytes[16]`.
    Bytes(Len),
    /// A reference to another struct defined in the same schema.
    Named(String),
    /// An array of `elem` repeated `len` times, e.g. `char[4]` or
    /// `Player[player_count]`.
    Array { elem: Box<TypeExpr>, len: Len },
    /// A discriminated union: the concrete type is chosen at runtime by the
    /// value of an earlier field (`discriminant`), reading only the selected
    /// variant's bytes. `default` (if any) handles values no arm matches.
    Match {
        discriminant: Vec<String>,
        arms: Vec<MatchArm>,
        default: Option<Box<TypeExpr>>,
    },
    /// A repeated element read until a sentinel is reached: `repeat T [until
    /// <cond>]`. Each iteration decodes one `elem` at the cursor and appends it.
    /// The loop stops when `until` (evaluated against the element just read)
    /// holds — the matching element is *included* — or, with no `until`, when
    /// the file ends or an element would read past it. A built-in iteration cap
    /// keeps a runaway schema from looping forever.
    Repeat {
        elem: Box<TypeExpr>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<Condition>,
    },
    /// A computed field: its value is an expression over earlier fields, and it
    /// reads no bytes from the file (`total = size - header`).
    Computed(Box<Expr>),
}

/// An integer arithmetic expression over field values (for computed fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    /// An integer literal.
    Int(i64),
    /// A (possibly dotted) reference to an earlier field's value.
    Field(Vec<String>),
    /// A binary operation on two sub-expressions.
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

/// A binary arithmetic operator usable in a computed field's expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// One arm of a [`TypeExpr::Match`]: `key => type`, where `key` is the
/// discriminant value this arm handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchArm {
    pub key: MatchKey,
    pub ty: TypeExpr,
}

/// The value an [`MatchArm`] matches on: an integer (for numeric discriminants)
/// or a string literal (for text discriminants such as a four-char chunk tag,
/// `match tag { "PLYR" => Plyr }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchKey {
    Int(i64),
    Str(String),
}

/// A fixed-width primitive type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Prim {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
}

impl Prim {
    /// Encoded size in bytes.
    pub fn size(self) -> usize {
        match self {
            Prim::U8 | Prim::I8 | Prim::Bool => 1,
            Prim::U16 | Prim::I16 => 2,
            Prim::U32 | Prim::I32 | Prim::F32 => 4,
            Prim::U64 | Prim::I64 | Prim::F64 => 8,
        }
    }

    /// Whether this is an integer primitive (valid as an enum/bitfield repr).
    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Prim::U8 | Prim::U16 | Prim::U32 | Prim::U64 | Prim::I8 | Prim::I16 | Prim::I32 | Prim::I64
        )
    }

    /// Map a type keyword to a primitive, if it names one.
    pub fn from_keyword(word: &str) -> Option<Prim> {
        Some(match word {
            "u8" => Prim::U8,
            "u16" => Prim::U16,
            "u32" => Prim::U32,
            "u64" => Prim::U64,
            "i8" => Prim::I8,
            "i16" => Prim::I16,
            "i32" => Prim::I32,
            "i64" => Prim::I64,
            "f32" => Prim::F32,
            "f64" => Prim::F64,
            "bool" => Prim::Bool,
            _ => return None,
        })
    }
}

/// A length, as it appears inside `[...]`.
///
/// It is either a literal count or a reference to an earlier field whose value
/// supplies the count at runtime (e.g. `players Player[player_count]`). The
/// parser accepts both; resolving a [`Len::Field`] is the runtime's job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Len {
    Fixed(u64),
    Field(String),
    /// `[*]` — everything from here to the end of the file.
    Rest,
}
