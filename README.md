# openpista

[![CI](https://github.com/openpista/openpista/actions/workflows/ci.yml/badge.svg)](https://github.com/openpista/openpista/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/openpista/openpista/graph/badge.svg)](https://codecov.io/gh/openpista/openpista)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)

**Languages:** English | [한국어](README_ko.md)

**Control your OS through any messenger — Telegram, WhatsApp, Web, or CLI.**

A single ~10 MB Rust binary. 6 LLM providers. Zero runtime dependencies.

<!-- TODO: demo GIF here -->

---

## Why openpista?

Unlike terminal-only AI agents, openpista meets you where you are:

- **Multi-channel** — chat via Telegram, WhatsApp, a self-hosted web UI, or the built-in CLI TUI
- **Single static binary** — ~10 MB, Rust, no runtime dependencies, runs anywhere
- **6 LLM providers** — switch between OpenAI, Anthropic, Together, Ollama, OpenRouter, or custom mid-session
- **Safe execution** — risky commands run in throwaway Docker containers; skills run in wasmtime isolation
- **Extensible skills** — write skills in any language, compiled to WASM, isolated and version-controlled

---

## Quick Start

### Option 1 — Pre-built binary (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/openpista/openpista/main/install.sh | bash
```

The installer auto-detects your OS and architecture. To install to a custom path:

```bash
curl -fsSL https://raw.githubusercontent.com/openpista/openpista/main/install.sh | bash -s -- --prefix ~/.local/bin
```

Available targets: `x86_64-unknown-linux-gnu` · `x86_64-unknown-linux-musl` · `aarch64-unknown-linux-gnu` · `aarch64-unknown-linux-musl` · `aarch64-apple-darwin`

### Option 2 — Build from source

```bash
git clone https://github.com/openpista/openpista.git && cd openpista
cargo build --release && sudo cp target/release/openpista /usr/local/bin/
```

> **Prerequisites (source build):** [Rust 1.85+](https://rustup.rs) · SQLite 3 (usually pre-installed)

### First run

```bash
openpista auth login   # OAuth browser login — no API key required
openpista              # launch TUI
```

---

## Channels

```
  Telegram    WhatsApp    Web Browser    CLI TUI
      \           |            |          /
       ╰────── openpista gateway ──────╯
                     │
           AI Agent (ReAct loop)
                     │
       bash · browser · screen · docker
```

| Channel | Setup Guide |
|---------|-------------|
| Telegram | [use-channels/telegram.md](./use-channels/telegram.md) |
| WhatsApp | [use-channels/whatsapp.md](./use-channels/whatsapp.md) |
| Web UI (self-hosted) | [use-channels/web.md](./use-channels/web.md) |
| CLI TUI | [use-channels/cli.md](./use-channels/cli.md) |

---

## Features

| Category | Feature | Status |
|----------|---------|--------|
| **Channels** | Telegram, WhatsApp, Web UI, CLI TUI | ✅ v0.1.0 |
| **Tools** | Bash · Browser (CDP) · Screen capture · Docker sandbox | ✅ v0.1.0 |
| **Skills** | SKILL.md loader · subprocess · WASM isolation | ✅ v0.1.0 |
| **Auth** | OAuth PKCE (OpenAI, Anthropic, OpenRouter) · API key | ✅ v0.1.0 |
| **Memory** | SQLite conversation history · session management | ✅ v0.1.0 |
| **Platform** | Cron scheduler · Model catalog browser · SSE streaming | ✅ v0.1.0 |

---

## Providers

| Provider | Default Model | Auth |
|----------|--------------|------|
| `openai` (default) | gpt-5.3-codex | OAuth PKCE, API key |
| `claude` / `anthropic` | claude-sonnet-4-6 | OAuth PKCE, Bearer |
| `together` | meta-llama/Llama-3.3-70B-Instruct-Turbo | API key |
| `ollama` | llama3.2 | None (local) |
| `openrouter` | openai/gpt-4o | OAuth PKCE, API key |
| `custom` | configurable | configurable |

Use your existing **ChatGPT Pro** or **Claude Max** subscription via OAuth — no separate API key needed.

---

## Architecture

```
[ Channels ]   Telegram · WhatsApp · CLI TUI · Web (WASM)
      │  tokio::mpsc  ChannelEvent
      ▼
[ Gateway ]    in-process router · cron scheduler
      │
      ▼
[ Agent ]      ReAct loop · 6 LLM providers · SQLite memory
      │  tool_call
      ▼
[ Tools ]      system.run · browser.* · screen.capture · container.run
[ Skills ]     SKILL.md → subprocess / WASM
```

---

## Docs & Community

- [ROADMAP.md](./ROADMAP.md) — what's next
- [CHANGELOG.md](./CHANGELOG.md) — release notes
- [use-channels/](./use-channels/) — per-channel setup guides
- [config.example.toml](./config.example.toml) — full configuration reference
- [COMMANDS.md](./COMMANDS.md) — all CLI & TUI commands

Contributions welcome — fork, branch (`feat/...`), `cargo fmt && cargo clippy`, open a PR against `main`.

---

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option.
