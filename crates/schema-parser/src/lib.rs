//! Parser for Binary Explorer's schema DSL.
//!
//! Turns schema source text into a [`schema::Schema`] AST in two stages:
//! `text → `[`lexer`]` → tokens → `[`parser`]` → AST`. This crate owns *only*
//! the front-end; executing the schema against bytes is a later crate's job.
//!
//! ```
//! let schema = schema_parser::parse(
//!     "struct Header { magic char[4] version u16 size u32 }",
//! )
//! .unwrap();
//! assert_eq!(schema.structs[0].name, "Header");
//! assert_eq!(schema.structs[0].fields.len(), 3);
//! ```

mod error;
mod lexer;
mod parser;

pub use error::{ParseError, Span};
pub use lexer::{tokenize, Token, TokenKind};

/// Parse schema source text into a [`schema::Schema`].
pub fn parse(src: &str) -> Result<schema::Schema, ParseError> {
    let tokens = lexer::tokenize(src)?;
    parser::parse(tokens)
}
