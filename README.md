# openpista

[![CI](https://github.com/openpista/openpista/actions/workflows/ci.yml/badge.svg)](https://github.com/openpista/openpista/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/openpista/openpista/graph/badge.svg)](https://codecov.io/gh/openpista/openpista)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)

**Languages:** English | [한국어](README_ko.md)

Docs: [ROADMAP](./ROADMAP.md) · [CHANGELOG (v0.1.0+)](./CHANGELOG.md)

**A QUIC-based OS Gateway AI Agent** — let your LLM control your machine through any messenger.

> Inspired by [OpenClaw](https://github.com/openpista/openclaw)'s WebSocket-based agent architecture,
> rebuilt from scratch in Rust with QUIC transport for lower latency, zero head-of-line blocking,
> and a single static binary with no runtime dependencies.

---

## What is openpista?

openpista is a lightweight daemon written in Rust that bridges **messaging channels** (Telegram, CLI,WhatApp) to your **operating system** via an AI agent loop.

- Send a message in Telegram → the LLM decides what to do → bash runs it → reply comes back
- Single static binary, ~10 MB, minimal memory footprint
- QUIC transport (0-RTT) instead of WebSocket for lower latency
- Persistent conversation memory backed by SQLite
- Extensible **Skills** system: drop a `SKILL.md` in your workspace to add new agent capabilities

```
[ Channel Adapters ]  Telegram · CLI
        │  tokio::mpsc
        ▼
[ QUIC OS Gateway ]   quinn · rustls · session · router · cron
        │  QUIC stream
        ▼
[ Agent Runtime ]     LLM loop · ToolRegistry · SQLite memory
        │  tool_call
        ▼
[ OS Tools ]          system.run (bash) · screen* · input control*
[ Skills ]            SKILL.md → system prompt + subprocess

* coming in v0.2.0
```

---

## Features

| Feature | Status |
|---|---|
| Bash tool (`system.run`) | ✅ v0.1.0 |
| Telegram channel | ✅ v0.1.0 |
| Interactive CLI / REPL | ✅ v0.1.0 |
| QUIC gateway (self-signed TLS) | ✅ v0.1.0 |
| Cron scheduler | ✅ v0.1.0 |
| SQLite conversation memory | ✅ v0.1.0 |
| Skills (SKILL.md loader) | ✅ v0.1.0 |
| Screen capture | 🔜 v0.2.0 |
| Screen & input control (OpenClaw-style) | 🔜 v0.2.0 |
| Discord / Slack adapters | 🔜 v0.2.0 |

---

## Installation

### Prerequisites

- **Rust 1.85+** — [rustup.rs](https://rustup.rs)
- **SQLite 3** — usually pre-installed on macOS/Linux

### macOS

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# (Optional) SQLite via Homebrew if missing
brew install sqlite

# Clone and build
git clone https://github.com/openpista/openpista.git
cd openpista
cargo build --release

# Move binary to PATH
sudo cp target/release/openpista /usr/local/bin/
```

### Ubuntu / Debian

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# System dependencies
sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev libsqlite3-dev

# Clone and build
git clone https://github.com/openpista/openpista.git
cd openpista
cargo build --release

sudo cp target/release/openpista /usr/local/bin/
```

### Fedora / RHEL

```bash
sudo dnf install -y gcc pkg-config openssl-devel sqlite-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

git clone https://github.com/openpista/openpista.git
cd openpista
cargo build --release
sudo cp target/release/openpista /usr/local/bin/
```


---

## Configuration

Copy the example config and edit it:

```bash
cp config.example.toml config.toml
```

```toml
[gateway]
port = 4433          # QUIC listen port
tls_cert = ""        # Leave empty to auto-generate a self-signed cert

[agent]
provider = "openai"
model = "gpt-4o"
api_key = ""         # Or set openpista_API_KEY env var
max_tool_rounds = 10

[channels.telegram]
enabled = false
token = ""           # Or set TELEGRAM_BOT_TOKEN env var

[channels.cli]
enabled = true

[database]
url = "~/.openpista/memory.db"

[skills]
workspace = "~/.openpista/workspace"
```

### Environment Variables

| Variable | Description |
|---|---|
| `openpista_API_KEY` | OpenAI-compatible API key (overrides config) |
| `OPENAI_API_KEY` | Fallback API key |
| `OPENCODE_API_KEY` | OpenCode Zen API key |
| `TELEGRAM_BOT_TOKEN` | Telegram bot token (enables Telegram channel) |
| `openpista_WORKSPACE` | Custom skills workspace path |

---

## Usage

### Run a single command

```bash
openpista_API_KEY=sk-... openpista run -e "list files in my home directory"
```

### Auth Login Picker

```bash
# Interactive provider picker (search + arrow selection)
openpista auth login

# Non-interactive mode (for scripts/CI)
openpista auth login --non-interactive --provider opencode --api-key "$OPENCODE_API_KEY"
```

TUI shortcuts:

```txt
/login
/connection
```


```bash
# Recommended coding models
openpista models list
```

TUI shortcuts:

```txt
/models
```

Inside the `/models` browser:

```txt
s or /: search by model id
r: refresh catalog from remote
Esc: exit search mode (first) or close browser
```

### Daemon mode (Telegram + CLI + QUIC gateway)

```bash
openpista_API_KEY=sk-... \
TELEGRAM_BOT_TOKEN=123456:ABC... \
openpista start
```

The daemon:
- Listens on QUIC port `4433` for remote agent connections
- Starts all enabled channel adapters
- Writes a PID file to `~/.openpista/openpista.pid`
- Handles `SIGTERM` / `Ctrl-C` for graceful shutdown

### Skills

Place a `SKILL.md` in your workspace to extend the agent's capabilities:

```
~/.openpista/workspace/skills/
├── deploy/
│   ├── SKILL.md      ← describes what this skill does
│   └── run.sh        ← executed when the agent calls this skill
└── monitor/
    ├── SKILL.md
    └── main.py
```

---

## Workspace Structure

```
openpista/
├── crates/
│   ├── proto/      # Shared types, errors (AgentMessage, ToolCall, …)
│   ├── gateway/    # QUIC server, session router, cron scheduler
│   ├── agent/      # ReAct loop, LLM provider, SQLite memory
│   ├── tools/      # Tool trait + BashTool (system.run)
│   ├── channels/   # ChannelAdapter, CliAdapter, TelegramAdapter
│   ├── skills/     # SKILL.md loader, subprocess runner
│   └── cli/        # Binary entry point, clap, config, daemon
├── migrations/     # SQLite schema migrations
├── config.example.toml
└── README.md
```

---

## Contributing

Contributions are welcome! Please follow these steps:

1. **Fork** the repository and create a feature branch:
   ```bash
   git checkout -b feat/my-feature
   ```

2. **Code style** — format and lint before committing:
   ```bash
   cargo fmt --all
   cargo clippy --workspace -- -D warnings
   ```

3. **Tests** — all existing tests must pass and new code should include tests:
   ```bash
   cargo test --workspace
   ```

4. **Commit message convention**:
   ```
   feat(tools): add screen capture tool
   fix(agent): handle empty LLM response gracefully
   docs: update installation guide for Windows
   ```
   Follows [Conventional Commits](https://www.conventionalcommits.org/).

5. **Open a Pull Request** against the `main` branch. The PR description should explain:
   - What problem it solves
   - How to test the change

6. For significant changes, **open an issue first** to discuss the approach before writing code.

### Development Setup

```bash
git clone https://github.com/openpista/openpista.git
cd openpista

# Run tests
cargo test --workspace

# Check for issues
cargo clippy --workspace -- -D warnings

# Build release binary
cargo build --release
```

## Agent Orchestration

Operational guidance for multi-agent role separation and model routing lives in:

- `docs/agent-orchestration/README.md`
- `docs/agent-orchestration/routing-rules.md`
- `docs/agent-orchestration/policies.md`

---

## License

Licensed under either of:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
