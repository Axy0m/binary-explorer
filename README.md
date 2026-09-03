# Nybble

**A visual binary structure explorer.** Point it at a file, describe the layout
in a small declarative schema, and watch the raw bytes turn into a navigable
tree — every field mapped back to the exact bytes it came from, and back again.

Nybble is built for the moment you're staring at a binary nobody has a template
for: a save file, a firmware blob, an undocumented network capture, a custom
container. Instead of a write–compile–stare loop, you edit a schema and the
parse updates live, with bidirectional byte ↔ field highlighting so you always
know what maps to what.

Native, offline, and fast — Rust + Tauri, no telemetry, your bytes never leave
your machine.

---

## See it work

`demos/wasm/` contains a real, spec-valid WebAssembly module and a schema that
decodes it end to end. WebAssembly is a good showcase: every section length,
vector count, and index in the format is an LEB128 varint, so nothing sits at a
fixed offset.

```sh
# Decode a WASM module top to bottom
cargo run -p schema-runtime --example dump -- \
    demos/wasm/wasm.schema demos/wasm/module.wasm Module le
```

The headline is the **gzipped** copy — one field inflates the compressed stream
and parses the result inline:

```sh
cargo run -p schema-runtime --example dump -- \
    demos/wasm/wasm.schema demos/wasm/module.wasm.gz GzModule le
```

```
GzModule
  module: decode gunzip as Module        # spans the 73 compressed bytes
    magic: bytes[4] = 00 61 73 6d
    version: 1
    sections
      ...
        name: "add"                       # the export, pulled from inside the gzip
        body: 00 20 00 20 01 6a 0b        # the function's actual code
```

73 bytes of gzip become a full, structured module tree in a single step.

---

## Features

**Hex view**
- Virtualized rendering (only visible rows drawn; bytes paged from Rust) — opens
  multi-gigabyte files via memory mapping
- Offset gutter, ASCII pane, jump-to-offset, search
- Bidirectional highlight: click a field → its bytes light up, and vice versa

**Schema language**
- `struct`, all fixed-width primitives (`u8`…`u64`, `i8`…`i64`, `f32`/`f64`,
  `bool`, `char`), `string[N]`, `bytes[N]`, `cstring`, `[*]` (rest of file)
- `enum` and `bitfield` with named flags and multi-bit ranges
- Arrays `T[N]` with fixed or field-driven lengths
- Pointers — read a field at an offset held elsewhere (`at`, `at +`)
- Conditional fields (`if`) and computed fields (arithmetic over earlier fields)
- Tag-dispatched unions — `match tag { 1 => Header  "PLYR" => Player }`
- Iteration — `repeat T [until <cond>]` for TLV / chunk / box formats
- Inline transforms — `bytes[n] decode <t> [as <Type>]` for
  `xor` / `rolling_xor` / `add` / `base64` / `zlib` / `inflate` / `gunzip`
- Variable-length integers — `varint` / `svarint` (LEB128)

**Editing & analysis**
- Edit bytes or typed field values in place, with undo/redo, then save
- Entropy strip, string extraction, timestamp detection, format guessing
- Automatic format detection on open

**Formats & sharing**
- Built-in schemas for common formats (PNG, ELF, PE, ZIP, gzip, BMP, WAV,
  SQLite, …)
- A plugin system for packaging and installing format definitions
- Browse and install community format packs from within the app

---

## A schema, end to end

```
struct Chunk {
    length    u32
    chunkType char[4]
    data      match chunkType {
        "IHDR"  => IhdrData
        default => bytes[length]
    }
    crc       u32
}

struct PNG {
    signature bytes[8]
    chunks    repeat Chunk until chunkType == "IEND"
}
```

Each field records its byte offset and size, so the UI can light up exactly the
bytes behind any node.

---

## Build from source

Prerequisites: a [Rust toolchain](https://rustup.rs), [Node.js](https://nodejs.org)
18+, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for
your platform.

```sh
npm install
npm run dev      # run the desktop app in development
npm run build    # produce a release installer
```

Run the engine headless (no UI) against any file:

```sh
cargo test                                   # full workspace test suite
cargo run -p schema-runtime --example dump -- <schema> <file> [entry] [le|be]
cargo run -p schema-parser  --example check -- <schema>   # validate a schema
```

---

## Layout

```
apps/desktop/        Tauri desktop app (React + TypeScript frontend, Rust backend)
crates/
  binary-reader/     endian-aware, memory-mapped byte reader
  schema/            the schema AST (pure data)
  schema-parser/     schema text -> AST
  schema-runtime/    execute a schema against bytes -> a field tree
  format-detection/  magic-byte format guessing
  analysis/          entropy, strings, timestamps
  search/            byte/string search
  file-editing/      in-place edits with undo/redo
  schema-library/    saved-schema storage
  plugin-host/       format-pack plugins
schemas/             built-in format schemas
demos/               runnable examples
```

---

## License

Nybble is dual-licensed:

- **[GNU AGPL v3](LICENSE)** — free for everyone. Use it, study it, modify it,
  fork it, and use it inside your organization at no cost. If you redistribute
  it or expose it over a network, share your source under the same terms.
- **[Commercial license](COMMERCIAL-LICENSE.md)** — for embedding Nybble in a
  closed-source product or hosted service without the AGPL's reciprocal
  obligations.

The files you analyze, and the schemas and format packs you write, are your own
work — the license covers Nybble itself, not its output.
