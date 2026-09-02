//! Dump a binary file's structure using a schema, straight from the terminal.
//!
//! Usage:
//!   cargo run -p schema-runtime --example dump -- <schema.txt> <file.bin> [entry] [le|be]
//!
//! This is the whole `bytes -> schema -> structure` pipeline without the
//! desktop UI: handy for reverse-engineering a save file before the app's
//! structure view is wired up.

use binary_reader::BinaryReader;
use schema_runtime::{parse, Endian, FieldNode, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!(
            "usage: dump <schema.txt> <file.bin> [entry] [le|be]\n\
             (entry defaults to the first struct; endian defaults to le)"
        );
        std::process::exit(2);
    }

    let schema_path = &args[0];
    let file_path = &args[1];
    let entry = args.get(2).cloned().unwrap_or_default();
    let endian = match args.get(3).map(|s| s.as_str()) {
        Some("be") => Endian::Big,
        _ => Endian::Little,
    };

    let schema_text = std::fs::read_to_string(schema_path).unwrap_or_else(|e| {
        eprintln!("cannot read schema {schema_path}: {e}");
        std::process::exit(1);
    });
    let schema = schema_parser::parse(&schema_text).unwrap_or_else(|e| {
        eprintln!("schema error: {e}");
        std::process::exit(1);
    });
    let reader = BinaryReader::open(file_path).unwrap_or_else(|e| {
        eprintln!("cannot open {file_path}: {e}");
        std::process::exit(1);
    });

    let entry_name = if entry.trim().is_empty() {
        schema.structs.first().map(|s| s.name.clone()).unwrap_or_default()
    } else {
        entry
    };

    match parse(&schema, &reader, &entry_name, endian) {
        Ok(tree) => print_node(&tree, 0),
        Err(e) => {
            eprintln!("parse failed: {e}");
            std::process::exit(1);
        }
    }
}

fn print_node(node: &FieldNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let value = render_value(&node.value);
    let span = format!("@{} +{}", node.offset, node.size);
    let note = if node.description.is_empty() {
        String::new()
    } else {
        format!("   // {}", node.description)
    };
    if value.is_empty() {
        println!("{indent}{}: {}   [{span}]{note}", node.name, node.type_name);
    } else {
        println!("{indent}{}: {} = {value}   [{span}]{note}", node.name, node.type_name);
    }
    for child in &node.children {
        print_node(child, depth + 1);
    }
}

fn render_value(v: &Value) -> String {
    match v {
        Value::U(n) => n.to_string(),
        Value::I(n) => n.to_string(),
        Value::F(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => format!("'{c}'"),
        Value::Str(s) => format!("{s:?}"),
        Value::Bytes(b) => b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" "),
        Value::Enum(e) => match &e.name {
            Some(name) => format!("{name} ({})", e.value),
            None => format!("{} (unknown)", e.value),
        },
        Value::Struct | Value::Array | Value::Bitfield => String::new(),
    }
}
