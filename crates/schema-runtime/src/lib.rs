//! Schema execution engine (plan §9).
//!
//! Given a [`schema::Schema`] and a [`BinaryReader`], the runtime walks the
//! bytes and produces a tree of [`FieldNode`]s. Every node records its `name`,
//! `type_name`, decoded `value`, and — crucially — its `offset` and `size` in
//! the file. That per-field byte range is what lets the UI light up the exact
//! bytes behind a field (Phase 5), so it is computed for every node, scalar or
//! composite.
//!
//! ```
//! use schema_runtime::{parse, Endian, Value};
//! let schema = schema_parser::parse("struct H { version u16  size u32 }").unwrap();
//! let reader = binary_reader::BinaryReader::from_bytes(
//!     [0x01, 0x00, 0x80, 0x00, 0x00, 0x00],
//! );
//! let tree = parse(&schema, &reader, "H", Endian::Little).unwrap();
//! assert_eq!(tree.children[0].value, Value::U(1));
//! assert_eq!(tree.children[1].value, Value::U(128));
//! ```

use std::io::Read;

use binary_reader::BinaryReader;
use schema::{
    BinOp, BitfieldDef, CompareOp, CompareValue, Condition, Decode, EnumDef, Expr, Len, MatchKey,
    Prim, Schema, StructDef, Transform, TypeExpr,
};
use serde::{Deserialize, Serialize};

pub use binary_reader::Endian;

/// Guards against a schema whose structs reference each other cyclically. Real
/// formats nest only a handful of levels; this keeps a bad schema from
/// recursing forever.
const MAX_DEPTH: usize = 256;

/// Upper bound on `repeat` iterations. A malformed file (or a sentinel that
/// never matches) must not spin forever; a million elements is far more than any
/// real format has while still catching genuine loops.
const MAX_ITERS: usize = 1_000_000;

/// A decoded field and its exact location in the file.
///
/// Composite fields (structs, arrays) carry their parts in `children` and use
/// [`Value::Struct`] / [`Value::Array`] as a placeholder scalar. `offset` and
/// `size` always describe the field's full byte span, children included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldNode {
    pub name: String,
    /// Display form of the field's type, e.g. `"u32"`, `"char[4]"`, `"Player[3]"`.
    pub type_name: String,
    pub value: Value,
    pub offset: usize,
    pub size: usize,
    /// Optional documentation carried from the schema field (empty if none).
    #[serde(default)]
    pub description: String,
    pub children: Vec<FieldNode>,
}

/// A decoded scalar value. Composite fields use [`Value::Struct`] /
/// [`Value::Array`] and put their contents in [`FieldNode::children`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Value {
    U(u64),
    I(i64),
    F(f64),
    Bool(bool),
    Char(char),
    Str(String),
    Bytes(Vec<u8>),
    Struct,
    Array,
    /// A decoded enum: the underlying integer plus the matching variant name
    /// (`None` if no variant matched the value).
    Enum(EnumValue),
    /// A decoded bitfield placeholder; the unpacked members are in `children`.
    Bitfield,
}

/// The payload of [`Value::Enum`]: the raw integer and its symbolic name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumValue {
    pub value: i64,
    pub name: Option<String>,
}

impl Value {
    /// The integer this value represents, if any — used to resolve array
    /// lengths that reference an earlier field (`items T[count]`).
    fn as_u64(&self) -> Option<u64> {
        match self {
            Value::U(n) => Some(*n),
            Value::I(n) if *n >= 0 => Some(*n as u64),
            Value::Enum(e) if e.value >= 0 => Some(e.value as u64),
            _ => None,
        }
    }

    /// The integer this value represents for a condition test — used to guard
    /// conditional fields. Bools become 0/1 and enums use their underlying value.
    fn as_i64(&self) -> Option<i64> {
        match self {
            Value::U(n) => Some(*n as i64),
            Value::I(n) => Some(*n),
            Value::Bool(b) => Some(*b as i64),
            Value::Enum(e) => Some(e.value),
            _ => None,
        }
    }

    /// The text this value represents — used to match a string discriminant or
    /// compare a text field (`char[4]`/`string`/`cstring`) against a literal.
    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// Follow a dotted path (`["flags", "extra"]`) from a struct's decoded siblings
/// down through child nodes, returning the addressed node if it exists.
fn resolve_path<'a>(path: &[String], siblings: &'a [FieldNode]) -> Option<&'a FieldNode> {
    let (first, rest) = path.split_first()?;
    let mut node = siblings.iter().find(|n| n.name == *first)?;
    for seg in rest {
        node = node.children.iter().find(|n| n.name == *seg)?;
    }
    Some(node)
}

/// Anything that can go wrong executing a schema against bytes.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Read(#[from] binary_reader::ReadError),

    #[error("schema has no struct named `{0}`")]
    UnknownStruct(String),

    #[error("schema has no type named `{0}` (not a struct, enum, or bitfield)")]
    UnknownType(String),

    #[error("array length refers to field `{0}`, which was not seen before this point")]
    UnknownLengthField(String),

    #[error("array length field `{0}` is not a non-negative integer")]
    LengthNotInteger(String),

    #[error("condition refers to `{0}`, which was not seen before this point")]
    UnknownConditionField(String),

    #[error("condition field `{0}` is not an integer/bool that can be tested")]
    ConditionNotInteger(String),

    #[error("match discriminant `{0}` was not seen before this point")]
    UnknownMatchField(String),

    #[error("match discriminant `{0}` is not an integer or string that can be matched")]
    MatchFieldNotMatchable(String),

    #[error("no match arm for discriminant value {0} (and no `default`)")]
    NoMatchingArm(String),

    #[error("`repeat` exceeded the {0}-iteration cap (a sentinel that never matched?)")]
    RepeatOverrun(usize),

    #[error("`until` compares `{0}` against a value of a different kind (number vs. string)")]
    ConditionTypeMismatch(String),

    #[error("`decode` on field `{0}` requires a byte field (bytes[n] or bytes[*])")]
    DecodeNotBytes(String),

    #[error("varint at offset {0} runs past 10 bytes with its continuation bit still set (malformed LEB128?)")]
    VarintTooLong(usize),

    #[error("base64 decode failed: {0}")]
    Base64(String),

    #[error("{0} failed: {1}")]
    Inflate(&'static str, String),

    #[error("computed field refers to `{0}`, which was not seen before this point")]
    ExprUnknownField(String),

    #[error("computed field refers to `{0}`, which is not an integer")]
    ExprNotInteger(String),

    #[error("division by zero in a computed field")]
    DivByZero,

    #[error("schema nests deeper than the {0}-level limit (cyclic struct references?)")]
    TooDeep(usize),
}

type Result<T> = std::result::Result<T, RuntimeError>;

/// Parse `entry`, a struct named in `schema`, starting at offset 0.
pub fn parse(
    schema: &Schema,
    reader: &BinaryReader,
    entry: &str,
    endian: Endian,
) -> Result<FieldNode> {
    Runtime {
        schema,
        reader,
        endian,
    }
    .parse_struct_field(entry.to_string(), entry, 0, 0)
}

struct Runtime<'a> {
    schema: &'a Schema,
    reader: &'a BinaryReader,
    endian: Endian,
}

impl Runtime<'_> {
    /// Build a node for a field of struct type `struct_name` at `offset`.
    fn parse_struct_field(
        &self,
        field_name: String,
        struct_name: &str,
        offset: usize,
        depth: usize,
    ) -> Result<FieldNode> {
        if depth >= MAX_DEPTH {
            return Err(RuntimeError::TooDeep(MAX_DEPTH));
        }
        let def: &StructDef = self
            .schema
            .struct_named(struct_name)
            .ok_or_else(|| RuntimeError::UnknownStruct(struct_name.to_string()))?;

        let mut children = Vec::with_capacity(def.fields.len());
        let mut cursor = offset;
        for field in &def.fields {
            // A conditional field whose guard is false is absent: it reads no
            // bytes and produces no node, so following fields stay put.
            if let Some(cond) = &field.condition {
                if !self.eval_condition(cond, &children)? {
                    continue;
                }
            }
            let mut node = if let Some(ptr) = &field.pointer {
                // Pointer follow: read the type at the target offset. This does
                // not advance the sequential cursor — the field's bytes live
                // elsewhere, so the enclosing struct's contiguous size is
                // unchanged.
                let raw = self.resolve_len(&ptr.offset, &children, offset)?;
                let target = if ptr.relative { offset + raw } else { raw };
                self.parse_type(&field.name, &field.ty, target, &children, depth + 1)?
            } else {
                let n = self.parse_type(&field.name, &field.ty, cursor, &children, depth + 1)?;
                cursor += n.size;
                n
            };
            // Apply a `decode` transform to the field's raw bytes, if any. This
            // keeps the node's file offset/size (the encoded span) but replaces
            // its value/children with the decoded result.
            if let Some(dec) = &field.decode {
                node = self.decode_field(node, dec, depth)?;
            }
            node.description = field.desc.clone().unwrap_or_default();
            children.push(node);
        }

        Ok(FieldNode {
            name: field_name,
            type_name: struct_name.to_string(),
            value: Value::Struct,
            offset,
            size: cursor - offset,
            description: String::new(),
            children,
        })
    }

    /// Apply a `decode` transform to a just-read byte field. The node keeps its
    /// file `offset`/`size` (the encoded region, so the hex view still
    /// highlights the compressed/obfuscated bytes); its value — and, with
    /// `as <Type>`, its children — are replaced by the decoded result.
    ///
    /// Decoded children carry offsets into the *decoded* buffer, not the file
    /// (the decoded bytes don't exist on disk), so they can't map back to
    /// source bytes — an inherent property of decompression.
    fn decode_field(&self, mut node: FieldNode, dec: &Decode, depth: usize) -> Result<FieldNode> {
        let raw = match &node.value {
            Value::Bytes(b) => b.clone(),
            // `decode` is only meaningful on a raw byte run.
            _ => return Err(RuntimeError::DecodeNotBytes(node.name.clone())),
        };
        let decoded = apply_transform(&dec.transform, &raw)?;
        let tname = transform_name(&dec.transform);

        match &dec.as_type {
            Some(as_type) => {
                // Re-parse the decoded bytes as their own little document.
                let sub = BinaryReader::from_bytes(decoded);
                let rt = Runtime {
                    schema: self.schema,
                    reader: &sub,
                    endian: self.endian,
                };
                let parsed = rt.parse_type(&node.name, as_type, 0, &[], depth + 1)?;
                node.type_name = format!("decode {tname} as {}", type_display(as_type));
                node.value = parsed.value;
                node.children = parsed.children;
            }
            None => {
                node.type_name = format!("decode {tname}");
                node.value = Value::Bytes(decoded);
                node.children = Vec::new();
            }
        }
        Ok(node)
    }

    /// Build a node for one field of a given type at `offset`. `siblings` are
    /// the fields already decoded in the enclosing struct, used to resolve
    /// length references.
    fn parse_type(
        &self,
        name: &str,
        ty: &TypeExpr,
        offset: usize,
        siblings: &[FieldNode],
        depth: usize,
    ) -> Result<FieldNode> {
        if depth >= MAX_DEPTH {
            return Err(RuntimeError::TooDeep(MAX_DEPTH));
        }
        match ty {
            TypeExpr::Prim(p) => {
                let value = self.read_prim(*p, offset)?;
                Ok(scalar(name, prim_name(*p), value, offset, p.size()))
            }
            TypeExpr::Varint { signed } => {
                let (value, size) = self.read_varint(*signed, offset)?;
                let tname = if *signed { "svarint" } else { "varint" };
                Ok(scalar(name, tname.into(), value, offset, size))
            }
            TypeExpr::Char => {
                let b = self.reader.read_u8_at(offset)?;
                Ok(scalar(name, "char".into(), Value::Char(b as char), offset, 1))
            }
            TypeExpr::Str(len) => {
                let n = self.resolve_len(len, siblings, offset)?;
                let value = Value::Str(self.read_string(offset, n)?);
                Ok(scalar(name, type_display(ty), value, offset, n))
            }
            TypeExpr::CStr => {
                let (s, size) = self.read_cstring(offset)?;
                Ok(scalar(name, "cstring".into(), Value::Str(s), offset, size))
            }
            TypeExpr::Bytes(len) => {
                let n = self.resolve_len(len, siblings, offset)?;
                let bytes = self.reader.read_bytes_at(offset, n)?.to_vec();
                Ok(scalar(name, type_display(ty), Value::Bytes(bytes), offset, n))
            }
            TypeExpr::Named(type_name) => {
                // Structs, enums, and bitfields share a name space.
                if self.schema.struct_named(type_name).is_some() {
                    self.parse_struct_field(name.to_string(), type_name, offset, depth)
                } else if let Some(def) = self.schema.enum_named(type_name) {
                    self.parse_enum(name, def, offset)
                } else if let Some(def) = self.schema.bitfield_named(type_name) {
                    self.parse_bitfield(name, def, offset)
                } else {
                    Err(RuntimeError::UnknownType(type_name.clone()))
                }
            }
            TypeExpr::Array { elem, len } => {
                let n = self.resolve_len(len, siblings, offset)?;

                // `char[N]` is conventionally a string, not N separate chars —
                // this is what makes `magic char[4]` read back as "\x7fELF".
                if matches!(**elem, TypeExpr::Char) {
                    let value = Value::Str(self.read_string(offset, n)?);
                    return Ok(scalar(name, type_display(ty), value, offset, n));
                }

                let mut children = Vec::with_capacity(n);
                let mut cursor = offset;
                for i in 0..n {
                    // Array elements can't reference sibling fields, so no
                    // siblings are passed down into the element type.
                    let node = self.parse_type(&i.to_string(), elem, cursor, &[], depth + 1)?;
                    cursor += node.size;
                    children.push(node);
                }
                Ok(FieldNode {
                    name: name.to_string(),
                    type_name: type_display(ty),
                    value: Value::Array,
                    offset,
                    size: cursor - offset,
                    description: String::new(),
                    children,
                })
            }
            TypeExpr::Match {
                discriminant,
                arms,
                default,
            } => {
                let key = discriminant.join(".");
                let disc = &resolve_path(discriminant, siblings)
                    .ok_or_else(|| RuntimeError::UnknownMatchField(key.clone()))?
                    .value;
                // The discriminant is either an integer or a text tag; each arm
                // keys on the same kind. Pick the matching arm, else the default.
                let (chosen, shown) = match (disc.as_i64(), disc.as_str()) {
                    (_, Some(s)) => {
                        let arm = arms
                            .iter()
                            .find(|a| matches!(&a.key, MatchKey::Str(v) if v == s))
                            .map(|a| &a.ty);
                        (arm, s.to_string())
                    }
                    (Some(n), None) => {
                        let arm = arms
                            .iter()
                            .find(|a| matches!(&a.key, MatchKey::Int(v) if *v == n))
                            .map(|a| &a.ty);
                        (arm, n.to_string())
                    }
                    (None, None) => return Err(RuntimeError::MatchFieldNotMatchable(key)),
                };
                match chosen.or(default.as_deref()) {
                    // The selected variant is read at this offset like any type;
                    // the node takes the field's name and the variant's shape.
                    Some(ty) => self.parse_type(name, ty, offset, siblings, depth + 1),
                    None => Err(RuntimeError::NoMatchingArm(shown)),
                }
            }
            TypeExpr::Repeat { elem, until } => {
                let mut children = Vec::new();
                let mut cursor = offset;
                let end = self.reader.len();
                loop {
                    // Stop cleanly at end of file rather than reading past it.
                    if cursor >= end {
                        break;
                    }
                    if children.len() >= MAX_ITERS {
                        return Err(RuntimeError::RepeatOverrun(MAX_ITERS));
                    }
                    let node =
                        self.parse_type(&children.len().to_string(), elem, cursor, &[], depth + 1)?;
                    // A zero-width element with no sentinel would loop forever;
                    // bail rather than spin.
                    let empty = node.size == 0;
                    cursor += node.size;
                    let stop = match until {
                        // The condition sees the just-read element's own fields as
                        // its scope, so `until tag == "ENDF"` tests that element's
                        // `tag` field.
                        Some(cond) => self.eval_condition(cond, &node.children)?,
                        None => false,
                    };
                    children.push(node);
                    if stop || (until.is_none() && empty) {
                        break;
                    }
                }
                Ok(FieldNode {
                    name: name.to_string(),
                    type_name: type_display(ty),
                    value: Value::Array,
                    offset,
                    size: cursor - offset,
                    description: String::new(),
                    children,
                })
            }
            TypeExpr::Computed(expr) => {
                // Evaluated, not read: the field occupies zero bytes. A
                // non-negative result is shown unsigned so it can serve as a
                // length for a later field.
                let v = self.eval_expr(expr, siblings)?;
                let value = if v >= 0 { Value::U(v as u64) } else { Value::I(v) };
                Ok(scalar(name, "computed".into(), value, offset, 0))
            }
        }
    }

    /// Evaluate a computed field's expression over the decoded siblings.
    fn eval_expr(&self, expr: &Expr, siblings: &[FieldNode]) -> Result<i64> {
        match expr {
            Expr::Int(n) => Ok(*n),
            Expr::Field(path) => {
                let key = path.join(".");
                resolve_path(path, siblings)
                    .ok_or_else(|| RuntimeError::ExprUnknownField(key.clone()))?
                    .value
                    .as_i64()
                    .ok_or(RuntimeError::ExprNotInteger(key))
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(lhs, siblings)?;
                let r = self.eval_expr(rhs, siblings)?;
                Ok(match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div => {
                        if r == 0 {
                            return Err(RuntimeError::DivByZero);
                        }
                        l.wrapping_div(r)
                    }
                    BinOp::Rem => {
                        if r == 0 {
                            return Err(RuntimeError::DivByZero);
                        }
                        l.wrapping_rem(r)
                    }
                })
            }
        }
    }

    /// Resolve a length to a concrete byte count. `at` is the offset the length
    /// is measured from, used by [`Len::Rest`] (everything to end of file).
    fn resolve_len(&self, len: &Len, siblings: &[FieldNode], at: usize) -> Result<usize> {
        match len {
            Len::Fixed(n) => Ok(*n as usize),
            Len::Field(field) => {
                let node = siblings
                    .iter()
                    .find(|s| s.name == *field)
                    .ok_or_else(|| RuntimeError::UnknownLengthField(field.clone()))?;
                let n = node
                    .value
                    .as_u64()
                    .ok_or_else(|| RuntimeError::LengthNotInteger(field.clone()))?;
                Ok(n as usize)
            }
            Len::Rest => Ok(self.reader.len().saturating_sub(at)),
        }
    }

    /// Read a NUL-terminated string at `offset`, returning the decoded text and
    /// the number of bytes consumed (including the terminating NUL). The scan is
    /// capped so a missing NUL in a huge file doesn't read the whole thing.
    fn read_cstring(&self, offset: usize) -> Result<(String, usize)> {
        const CAP: usize = 4096;
        let remaining = self.reader.len().saturating_sub(offset);
        let scan = remaining.min(CAP);
        let bytes = self.reader.read_bytes_at(offset, scan)?;
        match bytes.iter().position(|&b| b == 0) {
            Some(idx) => Ok((String::from_utf8_lossy(&bytes[..idx]).into_owned(), idx + 1)),
            None => Ok((String::from_utf8_lossy(bytes).into_owned(), scan)),
        }
    }

    /// Evaluate a conditional field's guard against the already-decoded
    /// siblings in the enclosing struct.
    fn eval_condition(&self, cond: &Condition, siblings: &[FieldNode]) -> Result<bool> {
        let key = cond.path.join(".");
        let node = resolve_path(&cond.path, siblings)
            .ok_or_else(|| RuntimeError::UnknownConditionField(key.clone()))?;
        match &cond.compare {
            // No operator: a truthiness test on an integer field (non-zero / true).
            None => {
                let lhs = node
                    .value
                    .as_i64()
                    .ok_or(RuntimeError::ConditionNotInteger(key))?;
                Ok(lhs != 0)
            }
            // Numeric comparison.
            Some(c) => match &c.value {
                CompareValue::Int(rhs) => {
                    let lhs = node
                        .value
                        .as_i64()
                        .ok_or(RuntimeError::ConditionTypeMismatch(key))?;
                    Ok(apply_ord(c.op, lhs.cmp(rhs)))
                }
                // Text comparison against a string field (`tag == "ENDF"`).
                CompareValue::Str(rhs) => {
                    let lhs = node
                        .value
                        .as_str()
                        .ok_or(RuntimeError::ConditionTypeMismatch(key))?;
                    Ok(apply_ord(c.op, lhs.cmp(rhs.as_str())))
                }
            },
        }
    }

    /// Read a LEB128 variable-length integer at `offset`, returning its decoded
    /// value and the number of bytes it spanned. Each byte carries 7 payload
    /// bits (little-endian group order); the high bit (0x80) is the "another
    /// byte follows" flag. Unsigned (`varint`) yields a [`Value::U`]; signed
    /// SLEB128 (`svarint`) sign-extends the final group into a [`Value::I`].
    ///
    /// At most 10 bytes are read — enough for any 64-bit value — after which a
    /// still-set continuation bit is rejected as a malformed encoding rather
    /// than read forever. Payload bits past 64 are dropped (a 64-bit reader).
    fn read_varint(&self, signed: bool, offset: usize) -> Result<(Value, usize)> {
        const MAX_BYTES: usize = 10;
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        let mut consumed = 0usize;
        // The only fall-through exit from the loop is the `break`, which runs
        // after `last` is assigned, so it is always initialized before its use.
        let mut last: u8;
        loop {
            if consumed >= MAX_BYTES {
                return Err(RuntimeError::VarintTooLong(offset));
            }
            let byte = self.reader.read_u8_at(offset + consumed)?;
            consumed += 1;
            last = byte;
            // shift maxes at 63 on the 10th byte, so this never over-shifts.
            result |= ((byte & 0x7f) as u64) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        let value = if signed {
            // SLEB128: if the terminating group's sign bit is set and the value
            // didn't fill all 64 bits, extend the sign into the high bits.
            let mut v = result as i64;
            if shift < 64 && (last & 0x40) != 0 {
                v |= -1i64 << shift;
            }
            Value::I(v)
        } else {
            Value::U(result)
        };
        Ok((value, consumed))
    }

    fn read_prim(&self, p: Prim, offset: usize) -> Result<Value> {
        let r = self.reader;
        let le = self.endian == Endian::Little;
        let v = match p {
            Prim::U8 => Value::U(r.read_u8_at(offset)? as u64),
            Prim::I8 => Value::I(r.read_i8_at(offset)? as i64),
            Prim::Bool => Value::Bool(r.read_u8_at(offset)? != 0),
            Prim::U16 => Value::U(pick(le, || r.read_u16_le_at(offset), || r.read_u16_be_at(offset))? as u64),
            Prim::I16 => Value::I(pick(le, || r.read_i16_le_at(offset), || r.read_i16_be_at(offset))? as i64),
            Prim::U32 => Value::U(pick(le, || r.read_u32_le_at(offset), || r.read_u32_be_at(offset))? as u64),
            Prim::I32 => Value::I(pick(le, || r.read_i32_le_at(offset), || r.read_i32_be_at(offset))? as i64),
            Prim::U64 => Value::U(pick(le, || r.read_u64_le_at(offset), || r.read_u64_be_at(offset))?),
            Prim::I64 => Value::I(pick(le, || r.read_i64_le_at(offset), || r.read_i64_be_at(offset))?),
            Prim::F32 => Value::F(pick(le, || r.read_f32_le_at(offset), || r.read_f32_be_at(offset))? as f64),
            Prim::F64 => Value::F(pick(le, || r.read_f64_le_at(offset), || r.read_f64_be_at(offset))?),
        };
        Ok(v)
    }

    /// Decode an enum field: read its integer repr and match a variant name.
    fn parse_enum(&self, name: &str, def: &EnumDef, offset: usize) -> Result<FieldNode> {
        let raw = self.read_prim(def.repr, offset)?;
        let value = match raw {
            Value::U(n) => n as i64,
            Value::I(n) => n,
            // read_prim only returns U/I for integer prims, and an enum repr is
            // always an integer, so other values are unreachable here.
            _ => 0,
        };
        let matched = def.name_of(value).map(|s| s.to_string());
        Ok(scalar(
            name,
            def.name.clone(),
            Value::Enum(EnumValue { value, name: matched }),
            offset,
            def.repr.size(),
        ))
    }

    /// Decode a bitfield: read its integer repr, then unpack each member from
    /// the bit pattern. Members share the underlying bytes, so each member node
    /// spans the same byte range as the whole field.
    fn parse_bitfield(&self, name: &str, def: &BitfieldDef, offset: usize) -> Result<FieldNode> {
        let size = def.repr.size();
        let raw = self.read_uint(size, offset)?;
        let children = def
            .members
            .iter()
            .map(|m| {
                let width = m.width();
                let mask = if width >= 64 { u64::MAX } else { (1u64 << width) - 1 };
                let bits = (raw >> m.lo) & mask;
                // A single bit reads as a bool; a wider range as its integer value.
                let (type_name, value) = if width == 1 {
                    (format!("bit {}", m.lo), Value::Bool(bits != 0))
                } else {
                    (format!("bits {}..{}", m.lo, m.hi), Value::U(bits))
                };
                scalar(&m.name, type_name, value, offset, size)
            })
            .collect();
        Ok(FieldNode {
            name: name.to_string(),
            type_name: def.name.clone(),
            value: Value::Bitfield,
            offset,
            size,
            description: String::new(),
            children,
        })
    }

    /// Read an unsigned integer of `size` bytes (1/2/4/8) with the schema's
    /// endianness — used for bitfield bit extraction regardless of repr sign.
    fn read_uint(&self, size: usize, offset: usize) -> Result<u64> {
        let r = self.reader;
        let le = self.endian == Endian::Little;
        Ok(match size {
            1 => r.read_u8_at(offset)? as u64,
            2 => pick(le, || r.read_u16_le_at(offset), || r.read_u16_be_at(offset))? as u64,
            4 => pick(le, || r.read_u32_le_at(offset), || r.read_u32_be_at(offset))? as u64,
            _ => pick(le, || r.read_u64_le_at(offset), || r.read_u64_be_at(offset))?,
        })
    }

    /// Read `n` bytes and decode as text: UTF-8 (lossy), stopping at the first
    /// NUL so fixed-width, NUL-padded strings read back cleanly.
    fn read_string(&self, offset: usize, n: usize) -> Result<String> {
        let bytes = self.reader.read_bytes_at(offset, n)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

/// Choose the little- or big-endian reader closure.
fn pick<T>(
    le: bool,
    read_le: impl FnOnce() -> binary_reader::Result<T>,
    read_be: impl FnOnce() -> binary_reader::Result<T>,
) -> binary_reader::Result<T> {
    if le {
        read_le()
    } else {
        read_be()
    }
}

/// Run a byte buffer through a [`Transform`], producing the decoded bytes.
fn apply_transform(t: &Transform, input: &[u8]) -> Result<Vec<u8>> {
    Ok(match t {
        Transform::Xor(key) => {
            if key.is_empty() {
                input.to_vec()
            } else {
                input
                    .iter()
                    .enumerate()
                    .map(|(i, b)| b ^ key[i % key.len()])
                    .collect()
            }
        }
        Transform::RollingXor { seed, mul, add } => {
            let mut k = *seed;
            let mut out = Vec::with_capacity(input.len());
            for &b in input {
                out.push(b ^ k);
                k = k.wrapping_mul(*mul).wrapping_add(*add);
            }
            out
        }
        Transform::Add(k) => {
            let k = (k.rem_euclid(256)) as u8;
            input.iter().map(|b| b.wrapping_add(k)).collect()
        }
        Transform::Base64 => base64_decode(input)?,
        Transform::ZlibInflate => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(input)
                .read_to_end(&mut out)
                .map_err(|e| RuntimeError::Inflate("zlib_inflate", e.to_string()))?;
            out
        }
        Transform::Inflate => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(input)
                .read_to_end(&mut out)
                .map_err(|e| RuntimeError::Inflate("inflate", e.to_string()))?;
            out
        }
        Transform::Gunzip => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(input)
                .read_to_end(&mut out)
                .map_err(|e| RuntimeError::Inflate("gunzip", e.to_string()))?;
            out
        }
    })
}

/// Short display name for a transform, used in a decoded node's `type_name`.
fn transform_name(t: &Transform) -> &'static str {
    match t {
        Transform::Xor(_) => "xor",
        Transform::RollingXor { .. } => "rolling_xor",
        Transform::Add(_) => "add",
        Transform::Base64 => "base64",
        Transform::ZlibInflate => "zlib_inflate",
        Transform::Inflate => "inflate",
        Transform::Gunzip => "gunzip",
    }
}

/// Decode standard-alphabet Base64 (padding optional; ASCII whitespace ignored).
fn base64_decode(input: &[u8]) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for &c in input {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c).ok_or_else(|| RuntimeError::Base64(format!("bad character {:?}", c as char)))?;
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// Turn an `Ordering` into a bool under a comparison operator. Shared by the
/// numeric and string paths of [`Runtime::eval_condition`].
fn apply_ord(op: CompareOp, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        CompareOp::Eq => ord == Equal,
        CompareOp::Ne => ord != Equal,
        CompareOp::Lt => ord == Less,
        CompareOp::Le => ord != Greater,
        CompareOp::Gt => ord == Greater,
        CompareOp::Ge => ord != Less,
    }
}

fn scalar(name: &str, type_name: String, value: Value, offset: usize, size: usize) -> FieldNode {
    FieldNode {
        name: name.to_string(),
        type_name,
        value,
        offset,
        size,
        description: String::new(),
        children: Vec::new(),
    }
}

fn prim_name(p: Prim) -> String {
    match p {
        Prim::U8 => "u8",
        Prim::U16 => "u16",
        Prim::U32 => "u32",
        Prim::U64 => "u64",
        Prim::I8 => "i8",
        Prim::I16 => "i16",
        Prim::I32 => "i32",
        Prim::I64 => "i64",
        Prim::F32 => "f32",
        Prim::F64 => "f64",
        Prim::Bool => "bool",
    }
    .to_string()
}

/// Render a type the way the plan shows it, e.g. `char[4]`, `Player[count]`.
fn type_display(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Prim(p) => prim_name(*p),
        TypeExpr::Varint { signed } => {
            if *signed { "svarint" } else { "varint" }.to_string()
        }
        TypeExpr::Char => "char".to_string(),
        TypeExpr::Str(len) => format!("string[{}]", len_display(len)),
        TypeExpr::CStr => "cstring".to_string(),
        TypeExpr::Bytes(len) => format!("bytes[{}]", len_display(len)),
        TypeExpr::Named(n) => n.clone(),
        TypeExpr::Array { elem, len } => format!("{}[{}]", type_display(elem), len_display(len)),
        TypeExpr::Match { discriminant, .. } => format!("match {}", discriminant.join(".")),
        TypeExpr::Repeat { elem, .. } => format!("repeat {}", type_display(elem)),
        TypeExpr::Computed(_) => "computed".to_string(),
    }
}

fn len_display(len: &Len) -> String {
    match len {
        Len::Fixed(n) => n.to_string(),
        Len::Field(f) => f.clone(),
        Len::Rest => "*".to_string(),
    }
}
