---
mode: wasm
description: Return a greeting from a WASM skill.
---

# hello-wasm

This skill demonstrates the host/guest JSON ABI used by the embedded wasmtime runtime.

- Export `alloc(i32) -> i32`
- Export `run(i32, i32) -> i64`
- Optionally export `dealloc(i32, i32)`

The host writes a JSON-encoded `ToolCall` into guest memory and expects a JSON-encoded `ToolResult` from guest memory.
