//! Heuristic analysis of unknown bytes (plan §11-12).
//!
//! Two tools for reverse-engineering a file whose layout you don't know:
//!
//! * [`find_strings`] scans for runs of readable text (ASCII and UTF-16LE) —
//!   names, paths, and tags usually jump straight out of a binary this way.
//! * [`analyze_at`] answers "what could the bytes here be?" for one offset:
//!   a string, a Unix timestamp, a UUID, and so on — always framed as guesses.
//!
//! Nothing here decodes a *known* structure (that's the schema runtime); this
//! is the discovery step that comes before you can write a schema.

mod dates;
mod entropy;
mod guess;
mod strings;

pub use entropy::entropy;
pub use guess::{analyze_at, Guess};
pub use strings::{find_strings, Encoding, StringHit};
