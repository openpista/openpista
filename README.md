# openpista

[![CI](https://github.com/openpista/openpista/actions/workflows/ci.yml/badge.svg)](https://github.com/openpista/openpista/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/openpista/openpista/graph/badge.svg)](https://codecov.io/gh/openpista/openpista)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)

**Languages:** English | [한국어](README_ko.md)

Docs: [ROADMAP](./ROADMAP.md) · [CHANGELOG (v0.1.0+)](./CHANGELOG.md)

**An OS Gateway AI Agent with browser access via Rust→WASM.** Let your LLM control your machine through any messenger.

> Inspired by [OpenClaw](https://github.com/openpista/openclaw)'s WebSocket-based agent architecture,
> rebuilt from scratch in Rust with a Rust→WASM browser client for universal access —
> a single static binary with no runtime dependencies.

---

## What is openpista?

openpista is a lightweight daemon written in Rust that bridges **messaging channels** (Telegram, WhatsApp, CLI, web browser) to your **operating system** via an AI agent loop.

- Send a message in Telegram: the LLM decides what to do, bash runs it, the reply comes back
 Single static binary, ~10 MB, minimal memory footprint
- Persistent conversation memory backed by SQLite
- Full browser automation via Chromium CDP and desktop screen capture
- Extensible **Skills** system: drop a `SKILL.md` in your workspace to add new agent capabilities

```
[ Channel Adapters ]  Telegram · WhatsApp · CLI (TUI) · Web (WASM)
        │  tokio::mpsc  ChannelEvent
        ▼
[ OS Gateway ]        in-process router · cron scheduler
        │
        ▼
[ Agent Runtime ]     ReAct loop · OpenAI / Anthropic / Responses API · SQLite memory
        │  tool_call
        ▼
[ OS Tools ]          system.run · browser.* · screen.capture · container.run
[ Skills ]            SKILL.md → system prompt + subprocess / WASM
```

---

## Features

| Feature | Status |
|---|---|
| Bash tool (`system.run`) | ✅ v0.1.0 |
| Browser tools (`browser.*`) | ✅ v0.1.0 |
| Screen capture (`screen.capture`) | ✅ v0.1.0 |
| Docker sandbox (`container.run`) | ✅ v0.1.0 |
| WASM skill sandbox | ✅ v0.1.0 |
| Telegram channel | ✅ v0.1.0 |
| Cron scheduler | ✅ v0.1.0 |
| SQLite conversation memory | ✅ v0.1.0 |
| Session management (sidebar + browser) | ✅ v0.1.0 |
| Skills (SKILL.md loader) | ✅ v0.1.0 |
| Multi-provider OAuth (PKCE) | ✅ v0.1.0 |
| Model catalog browser | ✅ v0.1.0 |
| OpenAI Responses API (SSE) | ✅ v0.1.0 |
| Anthropic Claude provider | ✅ v0.1.0 |
| Web adapter (Rust→WASM + WebSocket) | ✅ v0.1.0 |
| WhatsApp channel (Business Cloud API) | ✅ v0.1.0 |
| Discord / Slack adapters | 🔜 v0.2.0 |

---

## Providers

Six provider presets ship out of the box:

| Provider | Default Model | Auth |
|---|---|---|
| `openai` (default) | gpt-4o | OAuth PKCE, API key |
| `claude` / `anthropic` | claude-sonnet-4-6 | OAuth PKCE, Bearer |
| `together` | meta-llama/Llama-3.3-70B-Instruct-Turbo | API key |
| `ollama` | llama3.2 | None (local) |
| `openrouter` | openai/gpt-4o | OAuth PKCE, API key |
| `custom` | configurable | configurable |

The OpenAI preset supports both the standard ChatCompletions API and the Responses API (`/v1/responses`) for ChatGPT Pro subscribers. The Anthropic preset uses OAuth Bearer auth with the `anthropic-beta: oauth-2025-04-20` header and handles tool name sanitization automatically.

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

## Quick Start

After building openpista, authenticate with your LLM provider and launch the TUI:

```bash
# 1. Log in (opens browser for OAuth PKCE — recommended)
openpista auth login
# 2. Launch the TUI
openpista
```

That's it. The OAuth token is persisted to `~/.openpista/credentials.json` and auto-refreshed on expiry.

---

## Authentication

**OAuth PKCE browser login** is the recommended way to authenticate. It works with OpenAI, Anthropic, and OpenRouter out of the box — no API keys required.

```bash
# Interactive provider picker (search + arrow selection)
openpista auth login
```

From the TUI:

```txt
/login
/connection
```

For providers that don't support OAuth (Together, Ollama, Custom), supply an API key:

```bash
# API key login (stored in credential store)
openpista auth login --non-interactive --provider together --api-key "$TOGETHER_API_KEY"
# Provider with custom endpoint
openpista auth login --non-interactive \
  --provider azure-openai \
  --endpoint "https://your-resource.openai.azure.com" \
  --api-key "$AZURE_OPENAI_API_KEY"
```

### Credential Resolution Priority

openpista resolves credentials in this order (highest priority first):

| Priority | Source | Description |
|---|---|---|
| 1 | Config file / `openpista_API_KEY` | Explicit `api_key` in `config.toml` or env override |
| 2 | Credential store | Token saved by `openpista auth login` (`~/.openpista/credentials.json`) |
| 3 | Provider env var | e.g. `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `TOGETHER_API_KEY` |
| 4 | Legacy fallback | `OPENAI_API_KEY` (used when no other source matches) |

For most users, **`openpista auth login` (priority 2) is all you need.** Environment variables and config file keys are provided for CI pipelines, Docker containers, and non-interactive scripts.

---

## Configuration

Configuration is loaded from: `--config` path → `./config.toml` → `~/.openpista/config.toml` → defaults.

```bash
cp config.example.toml config.toml
```
```toml
[agent]
provider = "openai"
model = "gpt-4o"
max_tool_rounds = 10
# api_key = ""       # Optional — prefer `openpista auth login` instead
[channels.telegram]
enabled = false
token = ""
[channels.cli]
enabled = true
url = "~/.openpista/memory.db"
workspace = "~/.openpista/workspace"

[channels.whatsapp]
enabled = false
phone_number_id = ""
access_token = ""
verify_token = ""
app_secret = ""
webhook_port = 8080

[channels.web]
enabled = false
token = ""
port = 3210
static_dir = "~/.openpista/web"
```

### Environment Variable Overrides (CI / Scripts)

Environment variables override config file values. They are intended for CI pipelines, Docker, and non-interactive environments — not as the primary setup method.
| Variable | Description |
|---|---|
| `openpista_API_KEY` | API key override (takes top priority) |
| `OPENAI_API_KEY` | OpenAI API key |
| `ANTHROPIC_API_KEY` | Anthropic API key |
| `openpista_MODEL` | Model override |
| `openpista_OAUTH_CLIENT_ID` | Custom OAuth PKCE client ID |
| `openpista_WEB_TOKEN` | Web adapter auth token |
| `openpista_WEB_PORT` | Web adapter HTTP/WS port (default: 3210) |
| `openpista_WORKSPACE` | Custom skills workspace path |
| `WHATSAPP_ACCESS_TOKEN` | WhatsApp Business API access token |
| `WHATSAPP_VERIFY_TOKEN` | WhatsApp webhook verification token |
| `WHATSAPP_PHONE_NUMBER_ID` | WhatsApp Business phone number ID |
| `WHATSAPP_APP_SECRET` | WhatsApp app secret for HMAC verification |
| `TELEGRAM_BOT_TOKEN` | Telegram bot token (auto-enables Telegram) |
| `OPENCODE_API_KEY` | OpenCode Zen API key |
---

## Usage

### TUI (default)

```bash
# Launch TUI
openpista
openpista -s SESSION_ID
openpista tui -s SESSION_ID
```

### Run a single command

```bash
openpista run -e "list files in my home directory"
```

### Model catalog

```bash
openpista model list
```

From the TUI:

```txt
/model
/model list
```

Inside the model browser:

```txt
s or /: search by model ID
r:      refresh catalog from remote
Esc:    exit search mode (first press) or close browser
```

### Session management

From the TUI:

```txt
/session              - browse sessions
/session new          - start a new session
/session load ID      - load a session by ID
/session delete ID    - delete a session by ID
```

Press `Tab` to toggle the sidebar, which shows recent sessions. Navigate with `j`/`k` or arrow keys, `Enter` to open, `d`/`Delete` to request deletion, `Esc` to unfocus.

### Daemon mode (Telegram + WhatsApp + CLI + Web UI)

```bash
openpista start
```

Enable Telegram in `config.toml` or via environment:

```bash
# config.toml approach (recommended)
# [channels.telegram]
# enabled = true
# token = "123456:ABC..."

# Or via env var for CI/Docker
TELEGRAM_BOT_TOKEN=123456:ABC... openpista start
```

Enable WhatsApp in `config.toml`:

```bash
# [channels.whatsapp]
# enabled = true
# phone_number_id = "123456789"
# access_token = "EAA..."
# verify_token = "my-verify-token"
# app_secret = "abc123..."

# Or via env vars
WHATSAPP_ACCESS_TOKEN=EAA... WHATSAPP_PHONE_NUMBER_ID=123456789 openpista start
```

Enable the Web UI adapter:

```bash
# [channels.web]
# enabled = true
# token = "my-secret-token"
# port = 3210

# Or via env vars
openpista_WEB_TOKEN=my-secret-token openpista_WEB_PORT=3210 openpista start
# Then open http://localhost:3210 in your browser
```
The daemon:
 Starts all enabled channel adapters
 Writes a PID file to `~/.openpista/openpista.pid`
 Handles `SIGTERM` / `Ctrl-C` for graceful shutdown
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
│   ├── gateway/    # In-process gateway, cron scheduler
│   ├── agent/      # ReAct loop, OpenAI / Anthropic / Responses API, SQLite memory
│   ├── tools/      # Tool trait — BashTool, BrowserTool, ScreenTool, ContainerTool
│   ├── channels/   # CliAdapter, TelegramAdapter, WhatsAppAdapter, WebAdapter
│   ├── skills/     # SKILL.md loader, subprocess + WASM runner
│   ├── web/        # Rust→WASM browser client (wasm-bindgen, H5 chat UI)
│   └── cli/        # Binary entry point, clap, config, TUI (ratatui + crossterm)
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
   docs: update installation guide
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
