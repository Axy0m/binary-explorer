#!/usr/bin/env python3
"""Regenerate the flagship WASM demo bytes.

Emits `module.wasm` — a genuine, spec-valid WebAssembly module exporting
`add(i32, i32) -> i32` — and `module.wasm.gz`, its gzip copy (for the
`decode gunzip as Module` demo). Run from anywhere:

    python demos/wasm/make_sample.py
"""

import gzip
import os

# The module, section by section. Every length/count/index byte is an LEB128
# varint (all small here, so one byte each — but the schema reads them as
# varints, so a larger module with multi-byte lengths parses just the same).
MODULE = bytes(
    [
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,  # "\0asm", version 1
        # Type section (id 1, size 7): 1 functype (i32, i32) -> i32
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F,
        # Function section (id 3, size 2): 1 func, type index 0
        0x03, 0x02, 0x01, 0x00,
        # Export section (id 7, size 7): export "add" -> func 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00,
        # Code section (id 10, size 9): 1 body, 0 locals, get 0/get 1/add/end
        0x0A, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6A, 0x0B,
    ]
)


def main() -> None:
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "module.wasm"), "wb") as f:
        f.write(MODULE)
    # mtime=0 makes the gzip bytes reproducible across runs.
    with gzip.GzipFile(os.path.join(here, "module.wasm.gz"), "wb", mtime=0) as f:
        f.write(MODULE)
    print(f"wrote module.wasm ({len(MODULE)} bytes) + module.wasm.gz")


if __name__ == "__main__":
    main()
