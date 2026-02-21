# Skills

This directory contains optional skill assets used by the runtime.

## WASM skill build guide

The runtime expects a `main.wasm` module under `skills/<skill-name>/` when a skill sets `mode: wasm` in `SKILL.md`.

Example build command:

```bash
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release
```

For the sample skill in this repository:

```bash
rustup target add wasm32-wasip1
cd skills/hello-wasm
cargo build --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/hello_wasm.wasm main.wasm
```

`SKILL.md` front-matter supports:

- `mode: wasm` or `mode: subprocess`
- `image: <container-image>` (optional, used by container mode)
