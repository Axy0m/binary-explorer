//! Error types and source positions for the schema parser.

/// A position in the schema source. `offset` is a 0-based byte index; `line`
/// and `col` are 1-based for human-facing messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub offset: usize,
    pub line: u32,
    pub col: u32,
}

/// Anything that can go wrong turning schema text into an AST.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("unexpected character '{ch}' at line {}, col {}", span.line, span.col)]
    UnexpectedChar { ch: char, span: Span },

    #[error("integer literal too large at line {}, col {}", span.line, span.col)]
    IntTooLarge { span: Span },

    #[error("unterminated string starting at line {}, col {}", span.line, span.col)]
    UnterminatedString { span: Span },

    #[error("expected {expected}, found {found} at line {}, col {}", span.line, span.col)]
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("expected {expected}, but reached end of input")]
    UnexpectedEof { expected: String },

    #[error("`{ty}` requires a length in brackets, e.g. `{ty}[16]` (line {}, col {})", span.line, span.col)]
    LengthRequired { ty: String, span: Span },

    #[error("duplicate type `{name}` at line {}, col {} (a struct, enum, or bitfield already has this name)", span.line, span.col)]
    DuplicateType { name: String, span: Span },

    #[error("`{ty}` is not an integer type; enum/bitfield underlying types must be an integer (u8..u64, i8..i64) at line {}, col {}", span.line, span.col)]
    NonIntegerRepr { ty: String, span: Span },

    #[error("bit {bit} is out of range for a {bits}-bit value at line {}, col {}", span.line, span.col)]
    BitOutOfRange { bit: u64, bits: u32, span: Span },

    #[error("bit range {lo}..{hi} is invalid (low bit must not exceed high bit) at line {}, col {}", span.line, span.col)]
    BadBitRange { lo: u64, hi: u64, span: Span },

    #[error("unknown transform `{name}` at line {}, col {} (try xor, rolling_xor, add, base64, zlib_inflate, inflate, gunzip)", span.line, span.col)]
    UnknownTransform { name: String, span: Span },

    #[error("transform `{name}` takes {expected} argument(s), got {found} at line {}, col {}", span.line, span.col)]
    BadTransformArgs {
        name: String,
        expected: usize,
        found: usize,
        span: Span,
    },

    #[error("unknown checksum `{name}` at line {}, col {} (try crc32, crc16, crc16_modbus, crc16_ccitt, crc16_xmodem, adler32, sum8, sum16, sum32, xor8)", span.line, span.col)]
    UnknownChecksum { name: String, span: Span },

    #[error("expected `over(<field>)` or `over(<first> .. <last>)` after `check {algo}` at line {}, col {}", span.line, span.col)]
    ExpectedOver { algo: String, span: Span },

    #[error("{value} is not a byte (0..255) in the pattern at line {}, col {}", span.line, span.col)]
    BadBytePattern { value: u64, span: Span },
}
