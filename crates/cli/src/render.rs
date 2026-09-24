//! Text rendering of a parsed field tree.
//!
//! The JSON output is the one meant for machines; this is the one meant for a
//! terminal, so it stays on one line per field and puts the byte span where the
//! eye can scan it.

use schema_runtime::{FieldNode, Value};

/// Render a whole tree, one line per field.
pub fn tree(root: &FieldNode) -> String {
    let mut out = String::new();
    node(&mut out, root, 0);
    out
}

fn node(out: &mut String, node_ref: &FieldNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let value = value(&node_ref.value);
    let shown = if value.is_empty() {
        String::new()
    } else {
        format!(" = {value}")
    };
    // A checksum verdict is the point of a `check` field, so it goes on the same
    // line rather than hiding in the JSON.
    let check = match &node_ref.check {
        None => String::new(),
        Some(c) if c.ok => format!("  [{} ok]", c.algo),
        Some(c) => format!(
            "  [{} MISMATCH: stored {:#x}, computed {:#x}]",
            c.algo, c.stored, c.computed
        ),
    };
    let desc = if node_ref.description.is_empty() {
        String::new()
    } else {
        format!("   // {}", node_ref.description)
    };
    out.push_str(&format!(
        "{indent}{}: {}{shown}   [@{} +{}]{check}{desc}\n",
        node_ref.name, node_ref.type_name, node_ref.offset, node_ref.size
    ));
    for child in &node_ref.children {
        node(out, child, depth + 1);
    }
}

/// One decoded value, as text.
pub fn value(v: &Value) -> String {
    match v {
        Value::U(n) => n.to_string(),
        Value::I(n) => n.to_string(),
        Value::F(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => format!("'{c}'"),
        Value::Str(s) => format!("{s:?}"),
        Value::Bytes(b) => hex(b),
        Value::Enum(e) => match &e.name {
            Some(name) => format!("{name} ({})", e.value),
            None => format!("{} (unknown)", e.value),
        },
        Value::Struct | Value::Array | Value::Bitfield => String::new(),
    }
}

/// Space-separated lowercase hex, the way a hex dump reads.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
