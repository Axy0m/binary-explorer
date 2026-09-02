//! Turns schema text into a flat stream of [`Token`]s.
//!
//! Whitespace (including newlines) is insignificant and only separates tokens;
//! `//` line comments run to end of line. Every token carries its source
//! [`Span`] so the parser can point at the exact place an error occurred.

use crate::error::{ParseError, Span};

/// A lexical token together with where it came from in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// The kinds of token the schema language has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// The `struct` keyword.
    Struct,
    /// The `enum` keyword.
    Enum,
    /// The `bitfield` keyword.
    Bitfield,
    /// The `if` keyword (guards a conditional field).
    If,
    /// The `at` keyword (follows a pointer to another offset).
    At,
    /// The `match` keyword (a discriminated-union field).
    Match,
    /// The `repeat` keyword (a repeat-until-sentinel field).
    Repeat,
    /// The `until` keyword (the sentinel condition of a `repeat`).
    Until,
    /// The `decode` keyword (a byte transform on a field).
    Decode,
    /// The `as` keyword (re-parse decoded bytes as a type).
    As,
    /// `,` — separates transform arguments, e.g. `rolling_xor(90, 31, 17)`.
    Comma,
    /// An identifier: a type name, field name, or struct name. Type keywords
    /// like `u32` arrive as `Ident`; only `struct`/`enum`/`bitfield` are reserved.
    Ident(String),
    /// An unsigned integer literal.
    Int(u64),
    /// A double-quoted string literal (used for field descriptions).
    Str(String),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    /// `:` — separates an enum/bitfield name from its underlying repr.
    Colon,
    /// `=` — assigns a value to an enum variant.
    Eq,
    /// `..` — an inclusive bit range in a bitfield member.
    DotDot,
    /// `.` — a member separator in a conditional field's path.
    Dot,
    /// `==` (also accepts a bare `=` in a condition) — equality.
    EqEq,
    /// `!=` — inequality.
    Ne,
    /// `<` — less than.
    Lt,
    /// `<=` — less than or equal.
    Le,
    /// `>` — greater than.
    Gt,
    /// `>=` — greater than or equal.
    Ge,
    /// `+` — marks a relative pointer offset (`at +off`).
    Plus,
    /// `=>` — separates a match arm's value from its type.
    FatArrow,
    /// `*` — a rest-of-file length (`bytes[*]`), or multiplication in an expression.
    Star,
    /// `-` — subtraction in a computed-field expression.
    Minus,
    /// `/` — division in a computed-field expression.
    Slash,
    /// `%` — remainder in a computed-field expression.
    Percent,
    /// `(` — opens a grouped expression.
    LParen,
    /// `)` — closes a grouped expression.
    RParen,
}

impl TokenKind {
    /// Human-readable description for error messages.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Struct => "keyword `struct`".into(),
            TokenKind::Enum => "keyword `enum`".into(),
            TokenKind::Bitfield => "keyword `bitfield`".into(),
            TokenKind::Ident(s) => format!("identifier `{s}`"),
            TokenKind::Int(n) => format!("integer `{n}`"),
            TokenKind::Str(s) => format!("string {s:?}"),
            TokenKind::LBrace => "`{`".into(),
            TokenKind::RBrace => "`}`".into(),
            TokenKind::LBracket => "`[`".into(),
            TokenKind::RBracket => "`]`".into(),
            TokenKind::Colon => "`:`".into(),
            TokenKind::Eq => "`=`".into(),
            TokenKind::DotDot => "`..`".into(),
            TokenKind::If => "keyword `if`".into(),
            TokenKind::Dot => "`.`".into(),
            TokenKind::EqEq => "`==`".into(),
            TokenKind::Ne => "`!=`".into(),
            TokenKind::Lt => "`<`".into(),
            TokenKind::Le => "`<=`".into(),
            TokenKind::Gt => "`>`".into(),
            TokenKind::Ge => "`>=`".into(),
            TokenKind::At => "keyword `at`".into(),
            TokenKind::Plus => "`+`".into(),
            TokenKind::Match => "keyword `match`".into(),
            TokenKind::Repeat => "keyword `repeat`".into(),
            TokenKind::Until => "keyword `until`".into(),
            TokenKind::Decode => "keyword `decode`".into(),
            TokenKind::As => "keyword `as`".into(),
            TokenKind::Comma => "`,`".into(),
            TokenKind::FatArrow => "`=>`".into(),
            TokenKind::Star => "`*`".into(),
            TokenKind::Minus => "`-`".into(),
            TokenKind::Slash => "`/`".into(),
            TokenKind::Percent => "`%`".into(),
            TokenKind::LParen => "`(`".into(),
            TokenKind::RParen => "`)`".into(),
        }
    }
}

/// Tokenize an entire schema source string.
pub fn tokenize(src: &str) -> Result<Vec<Token>, ParseError> {
    Lexer::new(src).run()
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn here(&self) -> Span {
        Span {
            offset: self.pos,
            line: self.line,
            col: self.col,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.pos + ahead).copied()
    }

    /// Consume one byte, tracking line/column. Only ASCII is expected in schema
    /// source; multi-byte UTF-8 in identifiers is rejected by `is_ident_*`.
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(b)
    }

    fn run(mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia();
            let start = self.here();
            let Some(b) = self.peek() else { break };
            let kind = match b {
                b'{' => {
                    self.bump();
                    TokenKind::LBrace
                }
                b'}' => {
                    self.bump();
                    TokenKind::RBrace
                }
                b'[' => {
                    self.bump();
                    TokenKind::LBracket
                }
                b']' => {
                    self.bump();
                    TokenKind::RBracket
                }
                b':' => {
                    self.bump();
                    TokenKind::Colon
                }
                b'=' => {
                    // `==` equality; `=>` a match arrow; a bare `=` assigns an
                    // enum variant (and is accepted as equality in a condition).
                    self.bump();
                    match self.peek() {
                        Some(b'=') => {
                            self.bump();
                            TokenKind::EqEq
                        }
                        Some(b'>') => {
                            self.bump();
                            TokenKind::FatArrow
                        }
                        _ => TokenKind::Eq,
                    }
                }
                b'!' => {
                    self.bump();
                    if self.peek() == Some(b'=') {
                        self.bump();
                        TokenKind::Ne
                    } else {
                        return Err(ParseError::UnexpectedChar { ch: '!', span: start });
                    }
                }
                b'<' => {
                    self.bump();
                    if self.peek() == Some(b'=') {
                        self.bump();
                        TokenKind::Le
                    } else {
                        TokenKind::Lt
                    }
                }
                b'>' => {
                    self.bump();
                    if self.peek() == Some(b'=') {
                        self.bump();
                        TokenKind::Ge
                    } else {
                        TokenKind::Gt
                    }
                }
                b'+' => {
                    self.bump();
                    TokenKind::Plus
                }
                b'*' => {
                    self.bump();
                    TokenKind::Star
                }
                // `//` comments are stripped by skip_trivia, so a `/` here is division.
                b'/' => {
                    self.bump();
                    TokenKind::Slash
                }
                b'-' => {
                    self.bump();
                    TokenKind::Minus
                }
                b'%' => {
                    self.bump();
                    TokenKind::Percent
                }
                b'(' => {
                    self.bump();
                    TokenKind::LParen
                }
                b')' => {
                    self.bump();
                    TokenKind::RParen
                }
                b',' => {
                    self.bump();
                    TokenKind::Comma
                }
                b'.' => {
                    // `..` is a bit range; a lone `.` is a member separator.
                    self.bump();
                    if self.peek() == Some(b'.') {
                        self.bump();
                        TokenKind::DotDot
                    } else {
                        TokenKind::Dot
                    }
                }
                b'"' => self.lex_string(start)?,
                b'0'..=b'9' => self.lex_number(start)?,
                _ if is_ident_start(b) => self.lex_ident(),
                other => {
                    return Err(ParseError::UnexpectedChar {
                        ch: other as char,
                        span: start,
                    })
                }
            };
            tokens.push(Token { kind, span: start });
        }
        Ok(tokens)
    }

    /// Skip whitespace and `//` line comments between tokens.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b) if b.is_ascii_whitespace() => {
                    self.bump();
                }
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'/') => {
                    while let Some(b) = self.peek() {
                        if b == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn lex_number(&mut self, start: Span) -> Result<TokenKind, ParseError> {
        // A `0x`/`0X` prefix introduces a hexadecimal literal (e.g. ELF's
        // `0x6474e550`). Otherwise the digits are plain decimal.
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x') | Some(b'X')) {
            self.bump(); // 0
            self.bump(); // x
            let begin = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9') | Some(b'a'..=b'f') | Some(b'A'..=b'F')) {
                self.bump();
            }
            if begin == self.pos {
                // `0x` with no hex digits following.
                return Err(ParseError::UnexpectedChar {
                    ch: self.peek().map(|b| b as char).unwrap_or('x'),
                    span: self.here(),
                });
            }
            if matches!(self.peek(), Some(b) if is_ident_continue(b)) {
                return Err(ParseError::UnexpectedChar {
                    ch: self.peek().unwrap() as char,
                    span: self.here(),
                });
            }
            let text = std::str::from_utf8(&self.src[begin..self.pos]).unwrap();
            let value = u64::from_str_radix(text, 16)
                .map_err(|_| ParseError::IntTooLarge { span: start })?;
            return Ok(TokenKind::Int(value));
        }

        let begin = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.bump();
        }
        // An identifier character immediately after digits (e.g. `12ab`) is a
        // malformed token, not two tokens.
        if matches!(self.peek(), Some(b) if is_ident_continue(b)) {
            return Err(ParseError::UnexpectedChar {
                ch: self.peek().unwrap() as char,
                span: self.here(),
            });
        }
        let text = std::str::from_utf8(&self.src[begin..self.pos]).unwrap();
        let value = text
            .parse::<u64>()
            .map_err(|_| ParseError::IntTooLarge { span: start })?;
        Ok(TokenKind::Int(value))
    }

    /// Lex a double-quoted string. Supports `\"`, `\\`, `\n`, `\t` escapes; an
    /// unterminated string (EOF or newline before the closing quote) is an error.
    fn lex_string(&mut self, start: Span) -> Result<TokenKind, ParseError> {
        self.bump(); // consume opening quote
        let mut s = String::new();
        loop {
            match self.bump() {
                Some(b'"') => return Ok(TokenKind::Str(s)),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => s.push('"'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'n') => s.push('\n'),
                    Some(b't') => s.push('\t'),
                    Some(other) => s.push(other as char),
                    None => return Err(ParseError::UnterminatedString { span: start }),
                },
                Some(b'\n') | None => return Err(ParseError::UnterminatedString { span: start }),
                Some(other) => s.push(other as char),
            }
        }
    }

    fn lex_ident(&mut self) -> TokenKind {
        let begin = self.pos;
        while matches!(self.peek(), Some(b) if is_ident_continue(b)) {
            self.bump();
        }
        let text = std::str::from_utf8(&self.src[begin..self.pos]).unwrap();
        match text {
            "struct" => TokenKind::Struct,
            "enum" => TokenKind::Enum,
            "bitfield" => TokenKind::Bitfield,
            "if" => TokenKind::If,
            "at" => TokenKind::At,
            "match" => TokenKind::Match,
            "repeat" => TokenKind::Repeat,
            "until" => TokenKind::Until,
            "decode" => TokenKind::Decode,
            "as" => TokenKind::As,
            _ => TokenKind::Ident(text.to_string()),
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
