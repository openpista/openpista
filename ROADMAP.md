# Roadmap

> **openpista** — A QUIC-based autonomous AI agent that controls your OS through any messenger.

---

## v0.1.0 — Initial Autonomous Agent Release

The first public release establishes the core autonomous loop: the LLM receives a message, reasons over available tools, executes OS commands, and replies — all without manual intervention.

### Core Runtime

- [x] Agent ReAct loop (LLM → tool call → result → LLM → text response)
- [x] `LlmProvider` trait with OpenAI-compatible adapter (`async-openai`)
- [x] `ToolRegistry` — dynamic tool registration and dispatch
- [x] Configurable max tool rounds to prevent infinite loops (default: 10)
- [x] Skill context injection into the system prompt on every request

### OS Tools

- [x] `system.run` — BashTool with configurable timeout (default: 30s)
- [x] Output truncation at 10,000 characters with a clear prompt indicator
- [x] stdout + stderr capture with exit code in results
- [x] Working directory override support
- [x] `screen.capture`
- [x] `input.control` (OpenClaw-style)

### Transport & Gateway

- [x] QUIC server via `quinn` + `rustls` on port 4433
- [x] Automatic self-signed TLS certificate generation via `rcgen` (no config needed)
- [x] Length-prefixed JSON framing over bidirectional QUIC streams
- [x] Per-connection `AgentSession` lifecycle management
- [x] `ChannelRouter` — `DashMap`-based channel-to-session mapping
- [x] `CronScheduler` — scheduled message dispatch via `tokio-cron-scheduler`
- [x] In-process gateway for CLI/testing (no QUIC required)

### Memory & Persistence

- [x] SQLite conversation memory via `sqlx`
- [x] Automatic migrations on startup (`sqlx::migrate!`)
- [x] Session creation, lookup, and timestamp updates
- [x] Message store/load with role, content, and tool call metadata
- [x] Tool call JSON serialization preserved across sessions
- [x] `~` path expansion for database URL

### Channel Adapters

- [x] `ChannelAdapter` trait for pluggable channel implementations
- [x] `CliAdapter` — stdin/stdout with `/quit` exit command
- [x] `TelegramAdapter` — `teloxide` dispatcher with stable per-chat sessions
- [x] Response routing: CLI responses → stdout, Telegram responses → bot API
- [x] Error responses clearly surfaced to the user

### Skills System

- [x] `SkillLoader` — recursive `SKILL.md` discovery from workspace
- [x] Context concatenation from all discovered skills
- [x] Subprocess execution: `run.sh` → bash, `main.py` → python/python3
- [x] Non-zero exit codes surfaced as tool errors
- [x] `OPENPISTACRAB_WORKSPACE` environment variable override

### Docker Sandbox

- [x] `container.run` tool — creates an isolated Docker container per task
- [ ] Per-task ephemeral tokens: short-lived credentials injected at container start, auto-revoked on exit
- [ ] Orchestrator/worker pattern: main agent acts as orchestrator and spawns worker containers for heavy or dangerous tasks
- [ ] Worker containers report results back to the orchestrator session via QUIC stream
- [ ] Container-level resource limits: CPU quota, memory cap, no-network by default
- [ ] Workspace volume mount (read-only) so workers can read skills/files without write access to the host
- [ ] Container lifecycle: create → inject token → run task → collect results → destroy (no reuse)
- [ ] `bollard` crate for Docker API integration (not `docker` CLI shell-out)
- [ ] Configurable base image per skill (`image:` field in `SKILL.md`)
- [ ] Fallback to subprocess mode when Docker daemon is unavailable

### WASM Skill Sandbox

- [ ] `wasmtime` integration as an embedded WASM runtime
- [ ] WASI host interface: restricted filesystem (read-only workspace) + stdout/stderr
- [ ] Skill execution mode flag in `SKILL.md` (`mode: wasm` vs `mode: subprocess`)
- [ ] Host↔guest ABI: receive JSON-encoded `ToolCall` args via WASM memory, return `ToolResult`
- [ ] 30-second execution timeout enforced at the WASM fuel/epoch level
- [ ] Memory cap: 64 MB per WASM skill instance
- [ ] `cargo build --target wasm32-wasip1` build guide included in `skills/README.md`
- [ ] Example WASM skill included in the repo (`skills/hello-wasm/`)

### CLI & Configuration

- [x] `openpistacrab start` — full daemon (QUIC + all enabled channels)
- [x] `openpistacrab run -e "..."` — single-shot agent command
- [x] `openpistacrab repl` — interactive REPL with session persistence
- [x] TOML config file with documented examples (`config.toml`)
- [x] Environment variable override for all secrets
- [ ] PID file written on start, removed on exit
- [x] `SIGTERM` + `Ctrl-C` graceful shutdown

### Quality & CI

- [x] Unit + integration tests: `cargo test --workspace` (target: 90+ tests)
- [x] Zero clippy warnings: `cargo clippy --workspace -- -D warnings`
- [x] Consistent formatting: `cargo fmt --all`
- [ ] GitHub Actions CI workflow on `push` / `pull_request` to `main`
- [ ] Codecov coverage reporting

### Documentation & Release Artifacts

- [ ] `README.md` with badges (CI, codecov, Rust version, license)
- [ ] `ROADMAP.md` (this document)
- [ ] `CHANGELOG.md` with v0.1.0 entries
- [ ] `LICENSE-MIT` and `LICENSE-APACHE`
- [ ] `config.example.toml` with all options documented
- [ ] GitHub Release with pre-built binaries:
  - [ ] `x86_64-apple-darwin` (macOS Intel)
  - [ ] `aarch64-apple-darwin` (macOS Apple Silicon)
  - [ ] `x86_64-unknown-linux-gnu` (Linux x86_64)
  - [ ] `aarch64-unknown-linux-gnu` (Linux ARM64)
- [ ] `crates.io` publish for library crates (optional)

---

## v0.2.0 — Screen & Input Control

Extends the tool surface to full visual OS control, inspired by OpenClaw's approach.

- `screen.capture` — base64/file screenshot via the `screenshots` crate
- `screen.ocr` — text extraction from screen regions
- `input.click`, `input.type`, `input.scroll` — mouse and keyboard control (OpenClaw-style)
- `system.notify` — desktop notifications via `notify-rust`
- Discord adapter
- Slack adapter
- Prometheus metrics export (`metrics-exporter-prometheus`)

---

## v0.3.0 — Voice & Multi-Agent

- `voice.transcribe` — mic input via `whisper-rs`
- `voice.speak` — TTS output
- Multi-agent collaboration (agents spawning agents)
- Rate limiting and safety layers (command allowlist/blocklist)
- WebSocket gateway as an alternative transport

---

## v1.0.0 — Production Ready

- Stable public API for all crates
- Full end-to-end security audit
- Long-term support guarantee
- Packaging: `brew`, `apt`, Docker image
