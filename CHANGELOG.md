# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## v0.1.0 - TBD

### Added

#### Agent Runtime
- [agent] ReAct loop: LLM → tool call → result → LLM → text response, with configurable max tool rounds (default: 10).
- [agent] `LlmProvider` trait with dynamic dispatch across three providers.
- [agent] `ToolRegistry` — dynamic tool registration, dispatch, and skill context injection into every system prompt.

#### Agent Providers
- [agent] `OpenAiProvider` — standard ChatCompletions API via `async-openai`.
- [agent] `ResponsesApiProvider` — OpenAI Responses API (`/v1/responses`) with SSE streaming; ChatGPT Pro subscriber support (`chatgpt-account-id` extracted from JWT); tool name collision detection.
- [agent] `AnthropicProvider` — Anthropic Messages API; system message extraction; consecutive tool-result merging; tool name sanitization (dots → underscores); OAuth Bearer with `anthropic-beta: oauth-2025-04-20`.
- [agent] Six provider presets: `openai`, `claude` / `anthropic`, `together`, `ollama`, `openrouter`, `custom`.
- [agent] OAuth PKCE browser login for OpenAI, Anthropic (`claude.ai`, `platform.claude.com`), and OpenRouter.
- [agent] Credential slots for extension providers: GitHub Copilot, Google Gemini, Vercel AI Gateway, Azure OpenAI, AWS Bedrock.

#### OS Tools
- [tools] `system.run` — BashTool with configurable timeout (default: 30s), stdout + stderr capture, exit code, working directory override, and 10,000-char output truncation.
- [tools] `screen.capture` — display screenshot via the `screenshots` crate with base64 output.
- [tools] `browser.navigate`, `browser.click`, `browser.type`, `browser.screenshot` — Chromium CDP automation via `chromiumoxide`.
- [tools] `container.run` — isolated per-task Docker execution via `bollard`; ephemeral tokens; orchestrator/worker pattern; resource limits (CPU, memory, no-network); workspace volume mount (read-only); subprocess fallback when Docker is unavailable.
- [tools] WASM skill sandbox via `wasmtime` — WASI host interface, `mode: wasm` in `SKILL.md`, host↔guest ABI, 30s timeout, 64 MB memory cap.

#### Transport & Gateway
- [gateway] QUIC server via `quinn` + `rustls` on port 4433 with automatic self-signed TLS via `rcgen`.
- [gateway] Length-prefixed JSON framing over bidirectional QUIC streams.
- [gateway] Per-connection `AgentSession` lifecycle management.
- [gateway] `ChannelRouter` — `DashMap`-based channel-to-session mapping.
- [gateway] `CronScheduler` — scheduled message dispatch via `tokio-cron-scheduler`.
- [gateway] In-process gateway mode for CLI and testing (no QUIC required).

#### Memory & Persistence
- [agent] SQLite conversation memory via `sqlx` with automatic migrations on startup.
- [agent] Session creation, lookup, and timestamp updates; message store/load with role, content, and tool call metadata.
- [agent] Tool call JSON serialization preserved across sessions; `~` path expansion for database URL.

#### Channel Adapters
- [channels] `ChannelAdapter` trait for pluggable channel implementations.
- [channels] `CliAdapter` — stdin/stdout with `/quit` exit command.
- [channels] `TelegramAdapter` — `teloxide` dispatcher with stable per-chat sessions.
- [channels] `MobileAdapter` — QUIC bidirectional streams, token-based auth, self-signed TLS via `rcgen`.
- [channels] Response routing: CLI → stdout, Telegram → bot API; error responses surfaced to the user.

#### Skills System
- [skills] `SkillLoader` — recursive `SKILL.md` discovery from workspace with context concatenation.
- [skills] Subprocess execution: `run.sh` → bash, `main.py` → python/python3; non-zero exit codes surfaced as tool errors.
- [skills] `openpista_WORKSPACE` environment variable override.

#### CLI & TUI
- [cli] `openpista start` — full daemon (QUIC + all enabled channels).
- [cli] `openpista run -e "..."` — single-shot agent command.
- [cli] `openpista tui [-s SESSION_ID]` — interactive TUI with optional session resume.
- [cli] `openpista model [MODEL_OR_COMMAND]` — model catalog (list / test).
- [cli] `openpista -s SESSION_ID` — session resume shortcut.
- [cli] `openpista auth login` — browser OAuth PKCE login with persisted credentials; supports `--provider`, `--api-key`, `--endpoint`, `--port`, `--timeout`, `--non-interactive` flags.
- [cli] Elm Architecture (TEA) reactive TUI — unidirectional data flow (`Action → update() → State → view()`).
- [cli] 12 TUI slash commands: `/help`, `/login`, `/connection`, `/model`, `/model list`, `/session`, `/session new`, `/session load <id>`, `/session delete <id>`, `/clear`, `/quit`, `/exit`.
- [cli] Centralized TUI with dedicated Home, Chat, Session Browser, and Model Browser screens.
- [cli] TOML config file with documented examples; environment variable override for all secrets.
- [cli] PID file on start, `SIGTERM` + `Ctrl-C` graceful shutdown.

#### Session Management
- [cli] Sidebar with session list — `Tab` toggle, `j`/`k`/arrows navigation, `Enter` to load, `d`/`Delete` to request deletion, `Esc` to unfocus.
- [cli] `/session` browser — full-screen session browsing with search filtering, keyboard navigation, create new, delete with confirmation dialog.
- [cli] `ConfirmDelete` dialog — `y`/`Enter` to confirm, `n`/`Esc` to cancel.

#### Model Catalog
- [cli] `/model` browser — full-screen model browsing with search (`s` or `/`), remote sync (`r`), keyboard navigation.
- [cli] `/model list` — print available models to chat.

#### TUI Enhancements
- [cli] Text selection via mouse drag in chat area; `Ctrl+C` / `Cmd+C` to copy.
- [cli] Mouse support: click, drag, scroll in chat and sidebar.
- [cli] Command palette with `Tab` auto-complete for slash commands and arrow navigation.
- [cli] `AppState` variants: Idle, Thinking, ExecutingTool, AuthPrompting, AuthValidating, LoginBrowsing, ModelBrowsing, SessionBrowsing, ConfirmDelete.

#### Quality & CI
- [ci] 726 unit + integration tests across all crates (84 agent, 30 channels, 483 cli, 18 proto, 20 gateway, 14 skills, 75 tools, 2 doctests).
- [ci] Zero clippy warnings: `cargo clippy --workspace -- -D warnings`.
- [ci] Consistent formatting: `cargo fmt --all`.
- [ci] GitHub Actions CI workflow on `push` / `pull_request` to `main`.
- [ci] Linux cross-build matrix (`x86_64/aarch64` × `gnu/musl`).
- [ci] Codecov coverage reporting.

#### Documentation
- [docs] `README.md` (English) and `README_ko.md` (Korean) with badges (CI, codecov, Rust version, license).
- [docs] `ROADMAP.md` (English) and `ROADMAP_ko.md` (Korean).
- [docs] `CHANGELOG.md` (this document).
- [docs] `COMMANDS.md` — CLI and TUI command reference.
- [docs] Agent orchestration docs (`docs/agent-orchestration/`).

### Changed
- None.

### Fixed
- None.

### Known Limitations
- [channels] Advanced multi-channel routing (e.g., cross-channel message forwarding) is out of scope for v0.1.0.
- [docs] `LICENSE-MIT` and `LICENSE-APACHE` files not yet added.
- [docs] `config.example.toml` not yet fully documented.
- [release] Pre-built binaries and `crates.io` publish are pending.

---

When preparing a new release, add `## vX.Y.Z - TBD` at the top of this file.
On tag/release day, replace `TBD` with `YYYY-MM-DD`.
