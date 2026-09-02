# Flagship demo — a real WebAssembly module, cracked open

This is the "real file just works" demo. `module.wasm` is a genuine,
spec-valid WebAssembly module — a browser/`wasmtime`/Node runtime validates it,
instantiates it, and runs its exported `add(2, 3) → 5`. Nybble decodes it with
one schema, all the way down to the instruction bytes.

WASM is the perfect showcase for two features: **every** section size, vector
count, and index in the format is an LEB128 **varint**, and the whole thing
gzips like any real payload — so `module.wasm.gz` shows **inline decompression**
(`decode gunzip`) feeding straight into the structured parse.

## The module

```wat
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
```
41 bytes raw, 73 bytes gzipped.

## Run it

Decode the raw module:

```sh
cargo run -p schema-runtime --example dump -- \
    demos/wasm/wasm.schema demos/wasm/module.wasm Module le
```

Decode the **gzipped** module — inflate + parse in one step (entry `GzModule`):

```sh
cargo run -p schema-runtime --example dump -- \
    demos/wasm/wasm.schema demos/wasm/module.wasm.gz GzModule le
```

**In the app:** the entry point defaults to the first struct, which is
`GzModule` — so loading `module.wasm.gz` and pasting `wasm.schema` with an
**empty entry field** just works. For the raw `module.wasm`, set the entry field
to **`Module`** (otherwise it tries to gunzip an already-uncompressed file).

## What you see

The gzip run is the headline: the top node spans the **73 compressed bytes**
(`@0 +73`), and its children are the entire inflated module — the function
signature `(i32, i32) → i32`, the `"add"` export, and the code body
`00 20 00 20 01 6a 0b` (locals, `local.get 0`, `local.get 1`, `i32.add`, `end`):

```
GzModule: GzModule   [@0 +73]
  module: decode gunzip as Module   [@0 +73]
    magic: bytes[4] = 00 61 73 6d   [@0 +4]
    version: u32 = 1   [@4 +4]
    sections: repeat Section   [@8 +33]
      ...
      2: Section   [@21 +9]
        id: u8 = 7
        size: varint = 7
        body: ExportSection
          count: varint = 1
          exports: Export[count]
            0: Export
              namelen: varint = 3
              name: string[namelen] = "add"
              kind: u8 = 0
              index: varint = 0
      3: Section   [@30 +11]
        id: u8 = 10
        body: CodeSection
          funcs: FuncBody[count]
            0: FuncBody
              bodysize: varint = 7
              body: bytes[bodysize] = 00 20 00 20 01 6a 0b
```

## Features exercised

- **varints (LEB128)** — section lengths, vector counts, type/func indices,
  string lengths, all read as `varint`; the variable widths keep the section
  walk aligned automatically.
- **`repeat`** — `sections repeat Section` runs to end of file.
- **tag dispatch** — `body match id { 1 => TypeSection  7 => ExportSection … }`
  decodes each section by its id, falling back to raw `bytes[size]`.
- **length-driven fields** — `bytes[size]`, `u8[nparams]`, `FuncType[count]`,
  `string[namelen]` all sized by an earlier field.
- **inline decompression** — `bytes[*] decode gunzip as Module` inflates the
  gzip stream and parses the result as a WASM module.

Regenerate the sample bytes (and the gzip copy) with `make_sample.py`.
