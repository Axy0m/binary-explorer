//! Recursive-descent parser: [`Token`] stream → [`schema::Schema`].
//!
//! Grammar (whitespace/comments already removed by the lexer):
//!
//! ```text
//! schema       := type_def*
//! type_def     := struct_def | enum_def | bitfield_def
//! struct_def   := 'struct' IDENT '{' field* '}'
//! enum_def     := 'enum' IDENT ':' repr '{' variant* '}'
//! bitfield_def := 'bitfield' IDENT ':' repr '{' member* '}'
//! field        := IDENT type
//! variant      := IDENT '=' INT
//! member       := IDENT INT ( '..' INT )?
//! type         := base ( '[' len ']' )?
//! base         := IDENT           // a type keyword, or a struct/enum/bitfield name
//! repr         := IDENT           // an integer primitive keyword (u8..u64, i8..i64)
//! len          := INT | IDENT     // fixed count, or an earlier field's name
//! ```
//!
//! Fields, variants, and members need no separators: the parser reads them until
//! it sees the closing `}`.

use std::collections::HashMap;

use crate::error::{ParseError, Span};
use crate::lexer::{Token, TokenKind};
use schema::{
    BinOp, BitMember, BitfieldDef, Compare, CompareOp, CompareValue, Condition, Decode, EnumDef,
    EnumVariant, Expr, Field, Len, MatchArm, MatchKey, Pointer, Prim, Schema, StructDef, Transform,
    TypeExpr,
};

/// Type keywords that require a bracketed length (they have no natural size).
const SIZED_KEYWORDS: [&str; 2] = ["string", "bytes"];

pub fn parse(tokens: Vec<Token>) -> Result<Schema, ParseError> {
    Parser { tokens, pos: 0 }.parse_schema()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, want: &TokenKind, expected: &str) -> Result<Token, ParseError> {
        match self.advance() {
            Some(t) if &t.kind == want => Ok(t),
            Some(t) => Err(ParseError::UnexpectedToken {
                expected: expected.to_string(),
                found: t.kind.describe(),
                span: t.span,
            }),
            None => Err(ParseError::UnexpectedEof {
                expected: expected.to_string(),
            }),
        }
    }

    fn expect_ident(&mut self, expected: &str) -> Result<(String, Span), ParseError> {
        match self.advance() {
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) => Ok((name, span)),
            Some(t) => Err(ParseError::UnexpectedToken {
                expected: expected.to_string(),
                found: t.kind.describe(),
                span: t.span,
            }),
            None => Err(ParseError::UnexpectedEof {
                expected: expected.to_string(),
            }),
        }
    }

    fn expect_int(&mut self, expected: &str) -> Result<(u64, Span), ParseError> {
        match self.advance() {
            Some(Token {
                kind: TokenKind::Int(n),
                span,
            }) => Ok((n, span)),
            Some(t) => Err(ParseError::UnexpectedToken {
                expected: expected.to_string(),
                found: t.kind.describe(),
                span: t.span,
            }),
            None => Err(ParseError::UnexpectedEof {
                expected: expected.to_string(),
            }),
        }
    }

    fn parse_schema(&mut self) -> Result<Schema, ParseError> {
        let mut structs: Vec<StructDef> = Vec::new();
        let mut enums: Vec<EnumDef> = Vec::new();
        let mut bitfields: Vec<BitfieldDef> = Vec::new();
        // Structs, enums, and bitfields share one name space; a name and the
        // span where it was defined lets us report duplicates precisely.
        let mut names: HashMap<String, ()> = HashMap::new();

        while let Some(tok) = self.peek() {
            let span = tok.span;
            match &tok.kind {
                TokenKind::Struct => {
                    let def = self.parse_struct()?;
                    self.claim_name(&mut names, &def.name, span)?;
                    structs.push(def);
                }
                TokenKind::Enum => {
                    let def = self.parse_enum()?;
                    self.claim_name(&mut names, &def.name, span)?;
                    enums.push(def);
                }
                TokenKind::Bitfield => {
                    let def = self.parse_bitfield()?;
                    self.claim_name(&mut names, &def.name, span)?;
                    bitfields.push(def);
                }
                other => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "keyword `struct`, `enum`, or `bitfield`".to_string(),
                        found: other.describe(),
                        span,
                    })
                }
            }
        }
        Ok(Schema {
            structs,
            enums,
            bitfields,
        })
    }

    /// Register a newly-defined type name, rejecting collisions with any earlier
    /// struct, enum, or bitfield. `span` points at the definition keyword.
    fn claim_name(
        &self,
        names: &mut HashMap<String, ()>,
        name: &str,
        span: Span,
    ) -> Result<(), ParseError> {
        if names.insert(name.to_string(), ()).is_some() {
            return Err(ParseError::DuplicateType {
                name: name.to_string(),
                span,
            });
        }
        Ok(())
    }

    fn parse_struct(&mut self) -> Result<StructDef, ParseError> {
        self.expect(&TokenKind::Struct, "keyword `struct`")?;
        let (name, _span) = self.expect_ident("a struct name")?;
        self.expect(&TokenKind::LBrace, "`{`")?;

        let mut fields = Vec::new();
        loop {
            match self.peek() {
                Some(Token {
                    kind: TokenKind::RBrace,
                    ..
                }) => {
                    self.advance();
                    break;
                }
                Some(_) => fields.push(self.parse_field()?),
                None => {
                    return Err(ParseError::UnexpectedEof {
                        expected: "a field or `}`".to_string(),
                    })
                }
            }
        }
        Ok(StructDef { name, fields })
    }

    fn parse_enum(&mut self) -> Result<EnumDef, ParseError> {
        self.expect(&TokenKind::Enum, "keyword `enum`")?;
        let (name, _) = self.expect_ident("an enum name")?;
        let repr = self.parse_repr()?;
        self.expect(&TokenKind::LBrace, "`{`")?;

        let mut variants = Vec::new();
        loop {
            match self.peek() {
                Some(Token { kind: TokenKind::RBrace, .. }) => {
                    self.advance();
                    break;
                }
                Some(_) => {
                    let (vname, _) = self.expect_ident("a variant name")?;
                    self.expect(&TokenKind::Eq, "`=`")?;
                    let (value, _) = self.expect_int("a variant value")?;
                    variants.push(EnumVariant {
                        name: vname,
                        value: value as i64,
                    });
                }
                None => {
                    return Err(ParseError::UnexpectedEof {
                        expected: "a variant or `}`".to_string(),
                    })
                }
            }
        }
        Ok(EnumDef { name, repr, variants })
    }

    fn parse_bitfield(&mut self) -> Result<BitfieldDef, ParseError> {
        self.expect(&TokenKind::Bitfield, "keyword `bitfield`")?;
        let (name, _) = self.expect_ident("a bitfield name")?;
        let repr = self.parse_repr()?;
        let bits = (repr.size() * 8) as u32;
        self.expect(&TokenKind::LBrace, "`{`")?;

        let mut members = Vec::new();
        loop {
            match self.peek() {
                Some(Token { kind: TokenKind::RBrace, .. }) => {
                    self.advance();
                    break;
                }
                Some(_) => {
                    let (mname, _) = self.expect_ident("a member name")?;
                    let (lo, lo_span) = self.expect_int("a bit index")?;
                    // Optional `..hi` for a multi-bit member; otherwise single bit.
                    let hi = if matches!(self.peek(), Some(t) if t.kind == TokenKind::DotDot) {
                        self.advance();
                        let (hi, hi_span) = self.expect_int("a high bit index")?;
                        if hi < lo {
                            return Err(ParseError::BadBitRange { lo, hi, span: hi_span });
                        }
                        hi
                    } else {
                        lo
                    };
                    if hi >= bits as u64 {
                        return Err(ParseError::BitOutOfRange { bit: hi, bits, span: lo_span });
                    }
                    members.push(BitMember {
                        name: mname,
                        lo: lo as u8,
                        hi: hi as u8,
                    });
                }
                None => {
                    return Err(ParseError::UnexpectedEof {
                        expected: "a member or `}`".to_string(),
                    })
                }
            }
        }
        Ok(BitfieldDef { name, repr, members })
    }

    /// Parse `: <int-prim>` — the underlying integer type of an enum/bitfield.
    fn parse_repr(&mut self) -> Result<Prim, ParseError> {
        self.expect(&TokenKind::Colon, "`:`")?;
        let (word, span) = self.expect_ident("an integer type (u8..u64, i8..i64)")?;
        match Prim::from_keyword(&word) {
            Some(p) if p.is_integer() => Ok(p),
            _ => Err(ParseError::NonIntegerRepr { ty: word, span }),
        }
    }

    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let (name, _) = self.expect_ident("a field name")?;

        // `name = <expr>` is a computed field: no type, reads no bytes.
        if matches!(self.peek(), Some(t) if t.kind == TokenKind::Eq) {
            self.advance();
            let expr = self.parse_expr()?;
            return Ok(Field {
                name,
                ty: TypeExpr::Computed(Box::new(expr)),
                pointer: None,
                condition: None,
                decode: None,
                desc: self.parse_optional_desc(),
            });
        }

        // An optional `at [+] <offset>` makes the field a pointer follow: it is
        // read at another offset rather than at the sequential cursor.
        let pointer = if matches!(self.peek(), Some(t) if t.kind == TokenKind::At) {
            self.advance();
            let relative = matches!(self.peek(), Some(t) if t.kind == TokenKind::Plus);
            if relative {
                self.advance();
            }
            let offset = self.parse_len()?;
            Some(Pointer { offset, relative })
        } else {
            None
        };
        let ty = self.parse_type()?;
        // An optional `decode <transform> [as <Type>]` transforms the field's
        // raw bytes (de-obfuscation / decompression) after reading.
        let decode = if matches!(self.peek(), Some(t) if t.kind == TokenKind::Decode) {
            self.advance();
            Some(self.parse_decode()?)
        } else {
            None
        };
        // An optional `if <condition>` makes the field conditional.
        let condition = if matches!(self.peek(), Some(t) if t.kind == TokenKind::If) {
            self.advance();
            Some(self.parse_condition()?)
        } else {
            None
        };
        Ok(Field {
            name,
            ty,
            pointer,
            condition,
            decode,
            desc: self.parse_optional_desc(),
        })
    }

    /// Parse a `decode` clause (the `decode` keyword already consumed):
    /// `<transform>[(args)] [as <type>]`.
    fn parse_decode(&mut self) -> Result<Decode, ParseError> {
        let (name, span) = self.expect_ident("a transform name")?;
        // Optional parenthesized integer arguments.
        let mut args: Vec<i64> = Vec::new();
        if matches!(self.peek(), Some(t) if t.kind == TokenKind::LParen) {
            self.advance();
            if !matches!(self.peek(), Some(t) if t.kind == TokenKind::RParen) {
                loop {
                    let (n, _) = self.expect_int("a transform argument")?;
                    args.push(n as i64);
                    if matches!(self.peek(), Some(t) if t.kind == TokenKind::Comma) {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
        }

        let transform = self.build_transform(&name, &args, span)?;

        let as_type = if matches!(self.peek(), Some(t) if t.kind == TokenKind::As) {
            self.advance();
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        Ok(Decode { transform, as_type })
    }

    /// Map a transform name + integer args to a [`Transform`], validating arity.
    fn build_transform(
        &self,
        name: &str,
        args: &[i64],
        span: Span,
    ) -> Result<Transform, ParseError> {
        let want = |n: usize| -> Result<(), ParseError> {
            if args.len() == n {
                Ok(())
            } else {
                Err(ParseError::BadTransformArgs {
                    name: name.to_string(),
                    expected: n,
                    found: args.len(),
                    span,
                })
            }
        };
        Ok(match name {
            "xor" => {
                if args.is_empty() {
                    return Err(ParseError::BadTransformArgs {
                        name: name.to_string(),
                        expected: 1,
                        found: 0,
                        span,
                    });
                }
                Transform::Xor(args.iter().map(|a| *a as u8).collect())
            }
            "rolling_xor" => {
                want(3)?;
                Transform::RollingXor {
                    seed: args[0] as u8,
                    mul: args[1] as u8,
                    add: args[2] as u8,
                }
            }
            "add" => {
                want(1)?;
                Transform::Add(args[0])
            }
            "sub" => {
                want(1)?;
                Transform::Add(-args[0])
            }
            "base64" => {
                want(0)?;
                Transform::Base64
            }
            "zlib_inflate" | "zlib" => {
                want(0)?;
                Transform::ZlibInflate
            }
            "inflate" | "deflate_raw" => {
                want(0)?;
                Transform::Inflate
            }
            "gunzip" | "gzip" => {
                want(0)?;
                Transform::Gunzip
            }
            other => {
                return Err(ParseError::UnknownTransform {
                    name: other.to_string(),
                    span,
                })
            }
        })
    }

    /// Consume a trailing string literal as the field's description, if present.
    fn parse_optional_desc(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token { kind: TokenKind::Str(_), .. }) => match self.advance() {
                Some(Token { kind: TokenKind::Str(s), .. }) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    // Expression grammar for computed fields (standard precedence):
    //   expr   := term   (('+' | '-') term)*
    //   term   := factor (('*' | '/' | '%') factor)*
    //   factor := INT | path | '(' expr ')'
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_term()?;
        loop {
            let op = match self.peek().map(|t| &t.kind) {
                Some(TokenKind::Plus) => BinOp::Add,
                Some(TokenKind::Minus) => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_term()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_factor()?;
        loop {
            let op = match self.peek().map(|t| &t.kind) {
                Some(TokenKind::Star) => BinOp::Mul,
                Some(TokenKind::Slash) => BinOp::Div,
                Some(TokenKind::Percent) => BinOp::Rem,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_factor()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Token { kind: TokenKind::Int(n), .. }) => {
                let n = *n;
                self.advance();
                Ok(Expr::Int(n as i64))
            }
            Some(Token { kind: TokenKind::Ident(_), .. }) => Ok(Expr::Field(self.parse_path()?)),
            Some(Token { kind: TokenKind::LParen, .. }) => {
                self.advance();
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(e)
            }
            Some(t) => Err(ParseError::UnexpectedToken {
                expected: "a number, field name, or `(`".to_string(),
                found: t.kind.describe(),
                span: t.span,
            }),
            None => Err(ParseError::UnexpectedEof {
                expected: "an expression".to_string(),
            }),
        }
    }

    /// Parse a dotted field reference: `IDENT ('.' IDENT)*`, e.g. `flags.extra`.
    fn parse_path(&mut self) -> Result<Vec<String>, ParseError> {
        let (first, _) = self.expect_ident("a field name")?;
        let mut path = vec![first];
        while matches!(self.peek(), Some(t) if t.kind == TokenKind::Dot) {
            self.advance();
            let (seg, _) = self.expect_ident("a member name after `.`")?;
            path.push(seg);
        }
        Ok(path)
    }

    /// Parse a discriminated union:
    /// `match <path> { <int> => type ...  [default => type] }`.
    fn parse_match(&mut self) -> Result<TypeExpr, ParseError> {
        self.expect(&TokenKind::Match, "keyword `match`")?;
        let discriminant = self.parse_path()?;
        self.expect(&TokenKind::LBrace, "`{`")?;

        let mut arms = Vec::new();
        let mut default = None;
        loop {
            match self.peek() {
                Some(Token { kind: TokenKind::RBrace, .. }) => {
                    self.advance();
                    break;
                }
                Some(Token { kind: TokenKind::Int(_), .. }) => {
                    let (value, _) = self.expect_int("a discriminant value")?;
                    self.expect(&TokenKind::FatArrow, "`=>`")?;
                    let ty = self.parse_type()?;
                    arms.push(MatchArm {
                        key: MatchKey::Int(value as i64),
                        ty,
                    });
                }
                // A quoted arm keys on a text discriminant (`"PLYR" => Plyr`).
                Some(Token { kind: TokenKind::Str(_), .. }) => {
                    let s = match self.advance() {
                        Some(Token { kind: TokenKind::Str(s), .. }) => s,
                        _ => unreachable!("peeked a string literal"),
                    };
                    self.expect(&TokenKind::FatArrow, "`=>`")?;
                    let ty = self.parse_type()?;
                    arms.push(MatchArm {
                        key: MatchKey::Str(s),
                        ty,
                    });
                }
                // `default => type` is the catch-all arm.
                Some(Token { kind: TokenKind::Ident(id), .. }) if id == "default" => {
                    self.advance();
                    self.expect(&TokenKind::FatArrow, "`=>`")?;
                    default = Some(Box::new(self.parse_type()?));
                }
                Some(t) => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "a discriminant value, a quoted tag, `default`, or `}`"
                            .to_string(),
                        found: t.kind.describe(),
                        span: t.span,
                    })
                }
                None => {
                    return Err(ParseError::UnexpectedEof {
                        expected: "a match arm or `}`".to_string(),
                    })
                }
            }
        }
        Ok(TypeExpr::Match {
            discriminant,
            arms,
            default,
        })
    }

    /// Parse a repeat-until-sentinel field:
    /// `repeat <type> [until <condition>]`.
    ///
    /// The element type is any ordinary type (usually a struct reference). The
    /// optional `until` condition is evaluated against each element as it is
    /// read; without it the loop runs to end of file.
    fn parse_repeat(&mut self) -> Result<TypeExpr, ParseError> {
        self.expect(&TokenKind::Repeat, "keyword `repeat`")?;
        let elem = self.parse_type()?;
        let until = if matches!(self.peek(), Some(t) if t.kind == TokenKind::Until) {
            self.advance();
            Some(self.parse_condition()?)
        } else {
            None
        };
        Ok(TypeExpr::Repeat {
            elem: Box::new(elem),
            until,
        })
    }

    /// Parse a conditional guard: `<path> [<op> <int>]`, where `path` is a
    /// dotted field reference like `flags.extra`.
    fn parse_condition(&mut self) -> Result<Condition, ParseError> {
        let path = self.parse_path()?;

        let compare = match self.peek_compare_op() {
            Some(op) => {
                self.advance();
                // The right-hand side is a number or a quoted string (for
                // matching a text field, e.g. `tag == "ENDF"`).
                let value = match self.peek() {
                    Some(Token { kind: TokenKind::Str(_), .. }) => match self.advance() {
                        Some(Token { kind: TokenKind::Str(s), .. }) => CompareValue::Str(s),
                        _ => unreachable!("peeked a string literal"),
                    },
                    _ => {
                        let (n, _) = self.expect_int("an integer or string to compare against")?;
                        CompareValue::Int(n as i64)
                    }
                };
                Some(Compare { op, value })
            }
            None => None,
        };
        Ok(Condition { path, compare })
    }

    /// If the next token is a comparison operator, map it to a [`CompareOp`].
    /// A bare `=` is accepted as equality for forgiveness.
    fn peek_compare_op(&self) -> Option<CompareOp> {
        match self.peek()?.kind {
            TokenKind::EqEq | TokenKind::Eq => Some(CompareOp::Eq),
            TokenKind::Ne => Some(CompareOp::Ne),
            TokenKind::Lt => Some(CompareOp::Lt),
            TokenKind::Le => Some(CompareOp::Le),
            TokenKind::Gt => Some(CompareOp::Gt),
            TokenKind::Ge => Some(CompareOp::Ge),
            _ => None,
        }
    }

    fn parse_type(&mut self) -> Result<TypeExpr, ParseError> {
        // A `match` field is a discriminated union rather than a named type.
        if matches!(self.peek(), Some(t) if t.kind == TokenKind::Match) {
            return self.parse_match();
        }
        // A `repeat` field reads its element type over and over until a sentinel.
        if matches!(self.peek(), Some(t) if t.kind == TokenKind::Repeat) {
            return self.parse_repeat();
        }

        let (base, span) = self.expect_ident("a type")?;

        // Is a length attached?
        let len = if matches!(self.peek(), Some(t) if t.kind == TokenKind::LBracket) {
            self.advance(); // consume '['
            let len = self.parse_len()?;
            self.expect(&TokenKind::RBracket, "`]`")?;
            Some(len)
        } else {
            None
        };

        self.resolve_type(base, span, len)
    }

    /// Combine a base type name with an optional `[len]` into a [`TypeExpr`].
    fn resolve_type(
        &self,
        base: String,
        span: Span,
        len: Option<Len>,
    ) -> Result<TypeExpr, ParseError> {
        // `string`/`bytes` are sized-only: they mean nothing without a length.
        if SIZED_KEYWORDS.contains(&base.as_str()) {
            let Some(len) = len else {
                return Err(ParseError::LengthRequired { ty: base, span });
            };
            return Ok(match base.as_str() {
                "string" => TypeExpr::Str(len),
                "bytes" => TypeExpr::Bytes(len),
                _ => unreachable!("SIZED_KEYWORDS is exhaustive here"),
            });
        }

        // Everything else: a primitive keyword, `char`, `cstring`, or a
        // struct/enum/bitfield reference.
        let element = if let Some(prim) = Prim::from_keyword(&base) {
            TypeExpr::Prim(prim)
        } else if base == "varint" {
            TypeExpr::Varint { signed: false }
        } else if base == "svarint" {
            TypeExpr::Varint { signed: true }
        } else if base == "char" {
            TypeExpr::Char
        } else if base == "cstring" {
            TypeExpr::CStr
        } else {
            TypeExpr::Named(base)
        };

        Ok(match len {
            Some(len) => TypeExpr::Array {
                elem: Box::new(element),
                len,
            },
            None => element,
        })
    }

    fn parse_len(&mut self) -> Result<Len, ParseError> {
        match self.advance() {
            Some(Token {
                kind: TokenKind::Int(n),
                ..
            }) => Ok(Len::Fixed(n)),
            Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) => Ok(Len::Field(name)),
            // `*` means "to the end of the file".
            Some(Token {
                kind: TokenKind::Star,
                ..
            }) => Ok(Len::Rest),
            Some(t) => Err(ParseError::UnexpectedToken {
                expected: "an integer, field name, or `*`".to_string(),
                found: t.kind.describe(),
                span: t.span,
            }),
            None => Err(ParseError::UnexpectedEof {
                expected: "an integer, field name, or `*`".to_string(),
            }),
        }
    }
}
