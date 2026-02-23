# Roadmap

> **openpista** — An autonomous AI agent that controls your OS through any messenger.

---

## v0.1.0 — Initial Autonomous Agent Release

The first public release establishes the core autonomous loop: the LLM receives a message, reasons over available tools, executes OS commands, and replies — all without manual intervention.

### Core Runtime

- [x] Agent ReAct loop (LLM → tool call → result → LLM → text response)
- [x] `LlmProvider` trait with OpenAI-compatible adapter (`async-openai`)
- [x] `ToolRegistry` — dynamic tool registration and dispatch
- [x] Configurable max tool rounds to prevent infinite loops (default: 10)
- [x] Skill context injection into the system prompt on every request

### Agent Providers

- [x] `OpenAiProvider` — standard OpenAI chat completions via `async-openai`
- [x] `ResponsesApiProvider` — OpenAI Responses API (`/v1/responses`) with SSE streaming; ChatGPT Pro subscriber support (`chatgpt-account-id` from JWT); tool name collision detection
- [x] `AnthropicProvider` — Anthropic Messages API; system message extraction; consecutive tool-result merging; tool name sanitization (dots to underscores); OAuth Bearer with `anthropic-beta: oauth-2025-04-20`
- [x] Six provider presets: `openai`, `claude` / `anthropic`, `together`, `ollama`, `openrouter`, `custom`
- [x] OAuth PKCE support for OpenAI, Anthropic, and OpenRouter
- [x] Credential slots for extension providers: GitHub Copilot, Google Gemini, Vercel AI Gateway, Azure OpenAI, AWS Bedrock

### OS Tools

- [x] `system.run` — BashTool with configurable timeout (default: 30s)
- [x] Output truncation at 10,000 characters with a clear prompt indicator
- [x] stdout + stderr capture with exit code in results
- [x] Working directory override support
- [x] `screen.capture` — display screenshot via the `screenshots` crate, base64 output
- [x] `browser.navigate`, `browser.click`, `browser.type`, `browser.screenshot` — Chromium CDP via `chromiumoxide`

### Gateway

- [x] `ChannelRouter` — `DashMap`-based channel-to-session mapping with in-process gateway
- [x] `CronScheduler` — scheduled message dispatch via `tokio-cron-scheduler`

> **Architecture note**
>
> All channel adapters use their native protocols (stdin, HTTP polling, HTTP webhooks, WebSocket) and bridge into the in-process gateway through `tokio::mpsc` channels.
>
> ```
> CliAdapter ─── stdin/stdout ──→ mpsc ──→ Gateway
> TelegramAdapter ── HTTP poll ──→ mpsc ──→ Gateway
> WhatsAppAdapter ── HTTP webhook → mpsc ──→ Gateway
> WebAdapter ───── WebSocket ────→ mpsc ──→ Gateway  ← Rust→WASM client in browser
> ```

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
 [ ] `WebAdapter` — Rust→WASM browser client with WebSocket transport (see Web Channel Adapter section)


### WhatsApp Channel Adapter

> WhatsApp follows the same HTTP-to-mpsc bridge pattern as Telegram. The adapter receives webhook events over HTTP (via `axum`), converts them to `ChannelEvent`, and forwards through `tokio::mpsc`.

 [ ] `WhatsAppAdapter` — WhatsApp Business Cloud API integration via `reqwest`
 [ ] Webhook HTTP server (via `axum`) for incoming messages: GET verification challenge + POST message handler
 [ ] HMAC-SHA256 webhook payload signature verification (`X-Hub-Signature-256` header)
 [ ] Text message sending via Meta Graph API (`POST /v21.0/{phone_number_id}/messages`)
 [ ] Stable per-conversation sessions: `whatsapp:{sender_phone}` channel ID and session mapping
 [ ] `WhatsAppConfig` — `[channels.whatsapp]` config section: `phone_number_id`, `access_token`, `verify_token`, `app_secret`, `webhook_port`
 [ ] Environment variable overrides: `WHATSAPP_ACCESS_TOKEN`, `WHATSAPP_VERIFY_TOKEN`, `WHATSAPP_PHONE_NUMBER_ID`, `WHATSAPP_APP_SECRET`
 [ ] Incoming message parsing: text, image, audio, video, document, location, contacts
 [ ] Message status webhook callback handling (sent → delivered → read)
 [ ] Media message download and forwarding (incoming media → base64 or local path for agent context)
 [ ] Interactive message support: reply buttons, list messages, quick replies
 [ ] Message template rendering for outbound notifications (HSM templates required by WhatsApp 24h policy)
 [ ] Rate limiting compliance with WhatsApp Business API tiers (messaging limits, throughput)
 [ ] Retry logic with exponential backoff for transient API failures (429, 500)
 [ ] Error responses clearly surfaced to the user (consistent with other adapters)
 [ ] Response routing integration: WhatsApp responses → Graph API `send_message`
 [ ] Multi-number support: configurable phone number IDs for business accounts with multiple numbers
 [ ] Unit tests: webhook verification, message parsing, session ID generation, response formatting, signature validation
 [ ] Integration test: end-to-end webhook → `ChannelEvent` → `AgentResponse` → WhatsApp send flow

#### Reference Open-Source Projects

> **Rust crates**
>
> | Crate | Description |
> |-------|-------------|
> | [`whatsapp-business-rs`](https://github.com/veecore/whatsapp-business-rs) | Full WhatsApp Business Cloud API SDK — axum webhook server, HMAC-SHA256 verification, message send/receive. Primary candidate. |
> | [`whatsapp-cloud-api`](https://github.com/sajuthankappan/whatsapp-cloud-api-rs) | Lightweight API client for Meta Graph API (30k+ downloads). No webhook server — pair with custom axum handler. |
> | [`whatsapp_handler`](https://github.com/bambby-plus/whatsapp_handler) | Webhook message processing + media/interactive message sending. |
>
> **Similar-architecture Rust AI agents**
>
> | Project | Description |
> |---------|-------------|
> | [`zeroclaw`](https://github.com/zeroclaw-labs/zeroclaw) | Trait-based `Channel` pattern nearly identical to openpista's `ChannelAdapter`. Multi-channel including WhatsApp. |
> | [`opencrust`](https://github.com/opencrust-org/opencrust) | Same `crates/` workspace layout. Separate `whatsapp/webhook.rs` + `api.rs` module structure. |
> | [`localgpt`](https://github.com/localgpt-app/localgpt) | `bridges/whatsapp/` bridge pattern for WhatsApp integration. |
> | [`loom`](https://github.com/ghuntley/loom) | Axum-based `routes/whatsapp.rs` route handler in a Rust workspace. |
>
> **API spec references (TypeScript)**
>
> | Project | Description |
> |---------|-------------|
> | [`WhatsApp-Nodejs-SDK`](https://github.com/WhatsApp/WhatsApp-Nodejs-SDK) | Official Meta SDK — authoritative webhook payload schemas and API endpoint specs. |
> | [`whatsapp-business-sdk`](https://github.com/MarcosNicolau/whatsapp-business-sdk) | Clean TypeScript types and good test coverage for Business Cloud API. |
>
> **Axum webhook HMAC-SHA256 patterns**
>
> | Resource | Description |
> |----------|-------------|
> | [pg3.dev — GitHub Webhooks in Rust with Axum](https://pg3.dev/post/github_webhooks_rust) | Complete HMAC-SHA256 + axum tutorial. `X-Hub-Signature-256` format identical to WhatsApp. |
> | [`axum-github-hooks`](https://github.com/rustunit/axum-github-hooks) | Axum extractor pattern for webhook signature verification — adaptable to `WhatsAppWebhookPayload`. |


### Web Channel Adapter (Rust→WASM + WebSocket)

> The Web adapter brings openpista to any phone or desktop browser — no native app required. The client is written in Rust, compiled to WASM, and served alongside an H5 chat UI. Communication uses standard WebSocket, which is universally supported in all browsers.

#### Server (axum)

 [ ] `WebAdapter` — axum HTTP server: WebSocket upgrade + static file serving for WASM bundle
 [ ] WebSocket message framing: JSON `ChannelEvent` / `AgentResponse` over WS text frames
 [ ] Token-based authentication on WebSocket handshake (`Sec-WebSocket-Protocol` or query param)
 [ ] `WebConfig` — `[channels.web]` config section: `port`, `token`, `cors_origins`, `static_dir`
 [ ] Environment variable overrides: `openpista_WEB_TOKEN`, `openpista_WEB_PORT`
 [ ] Session mapping: `web:{client_id}` channel ID with stable session per authenticated client
 [ ] Auto-reconnect support: client-side heartbeat ping/pong, server-side timeout detection
 [ ] CORS configuration for cross-origin browser access
 [ ] WSS (TLS) support via reverse proxy or built-in `axum-server` with `rustls`
 [ ] Configurable static file directory for WASM bundle and H5 assets

#### Client (Rust→WASM)

 [ ] Rust client crate (`crates/web/`) compiled to `wasm32-unknown-unknown` via `wasm-pack`
 [ ] `wasm-bindgen` JS interop: WebSocket API, DOM manipulation, localStorage
 [ ] WebSocket connection manager: connect, reconnect, heartbeat, buffered send queue
 [ ] Message serialization: `serde_json` in WASM for `ChannelEvent` / `AgentResponse`
 [ ] Session persistence: `localStorage` for session ID and auth token across page reloads
 [ ] H5 chat UI: mobile-responsive chat interface (HTML/CSS/JS or Yew/Leptos framework)
 [ ] Streaming response display: progressive text rendering as agent generates output
 [ ] Slash command support: `/model`, `/session`, `/clear`, `/help` from web UI input
 [ ] Media attachment support: image upload → base64 encoding → agent context
 [ ] PWA manifest: installable as home screen app (offline shell + online WebSocket)
 [ ] `wasm-pack build --target web` build pipeline in CI

#### Quality

 [ ] Unit tests: WebSocket handshake, token auth, message framing, reconnect logic
 [ ] Integration test: browser → WebSocket → `ChannelEvent` → `AgentResponse` → browser render
 [ ] WASM bundle size optimization: `wasm-opt`, tree shaking, gzip/brotli serving

### Skills System

- [x] `SkillLoader` — recursive `SKILL.md` discovery from workspace
- [x] Context concatenation from all discovered skills
- [x] Subprocess execution: `run.sh` → bash, `main.py` → python/python3
- [x] Non-zero exit codes surfaced as tool errors
- [x] `openpista_WORKSPACE` environment variable override

### Docker Sandbox

- [x] `container.run` tool — creates an isolated Docker container per task
- [x] Per-task ephemeral tokens: short-lived credentials injected at container start, auto-revoked on exit
- [x] Orchestrator/worker pattern: main agent acts as orchestrator and spawns worker containers for heavy or dangerous tasks
- [x] Worker containers report results back to the orchestrator session
- [x] Container-level resource limits: CPU quota, memory cap, no-network by default
- [x] Workspace volume mount (read-only) so workers can read skills/files without write access to the host
- [x] Container lifecycle: create → inject token → run task → collect results → destroy (no reuse)
- [x] `bollard` crate for Docker API integration (not `docker` CLI shell-out)
- [x] Configurable base image per skill (`image:` field in `SKILL.md`)
- [x] Fallback to subprocess mode when Docker daemon is unavailable

### WASM Skill Sandbox

- [x] `wasmtime` integration as an embedded WASM runtime
- [x] WASI host interface: restricted filesystem (read-only workspace) + stdout/stderr
- [x] Skill execution mode flag in `SKILL.md` (`mode: wasm` vs `mode: subprocess`)
- [x] Host↔guest ABI: receive JSON-encoded `ToolCall` args via WASM memory, return `ToolResult`
- [x] 30-second execution timeout enforced at the WASM fuel/epoch level
- [x] Memory cap: 64 MB per WASM skill instance
- [x] `cargo build --target wasm32-wasip1` build guide included in `skills/README.md`
- [x] Example WASM skill included in the repo (`skills/hello-wasm/`)

### CLI & Configuration

- [x] `openpista start` — full daemon (all enabled channels)
- [x] `openpista run -e "..."` — single-shot agent command
- [x] `openpista tui [-s SESSION_ID]` — TUI with optional session resume
- [x] `openpista model [MODEL_OR_COMMAND]` — model catalog (list / test)
- [x] `openpista -s SESSION_ID` — resume session shortcut
- [x] `openpista auth login` — browser OAuth PKCE login with persisted credentials
- [x] TUI slash commands: `/help`, `/login`, `/connection`, `/model`, `/model list`, `/session`, `/session new`, `/session load <id>`, `/session delete <id>`, `/clear`, `/quit`, `/exit`
- [x] Centralized TUI with dedicated Home, Chat, Session Browser, and Model Browser screens
- [x] TOML config file with documented examples (`config.toml`)
- [x] Environment variable override for all secrets
- [x] PID file written on start, removed on exit
- [x] `SIGTERM` + `Ctrl-C` graceful shutdown
- [x] Elm Architecture (TEA) reactive TUI — unidirectional data flow (`Action → update() → State → view()`)

### Session Management

- [x] Sidebar with session list — toggle with `Tab`, keyboard nav (`j`/`k`, arrows), `Enter` to load, `d`/`Delete` to request deletion, `Esc` to unfocus
- [x] `/session` browser — full-screen session browsing with search filtering, keyboard navigation, create new, delete with confirmation dialog
- [x] `/session new`, `/session load <id>`, `/session delete <id>` slash commands
- [x] `openpista tui -s SESSION_ID` — resume a session from the command line
- [x] `ConfirmDelete` dialog — `y`/`Enter` to confirm, `n`/`Esc` to cancel

### Model Catalog

- [x] `/model` browser — full-screen model browsing with search (`s` or `/`), remote sync (`r`), keyboard navigation
- [x] `/model list` — print available models to chat
- [x] `openpista model [MODEL_OR_COMMAND]` — model catalog from CLI

### TUI Enhancements

- [x] Text selection via mouse drag in chat area; `Ctrl+C` / `Cmd+C` to copy
- [x] Mouse support: click, drag, scroll in chat and sidebar
- [x] Command palette with `Tab` auto-complete for slash commands and arrow navigation
- [x] `AppState` variants: Idle, Thinking, ExecutingTool, AuthPrompting, AuthValidating, LoginBrowsing, ModelBrowsing, SessionBrowsing, ConfirmDelete

### Quality & CI

- [x] 726 unit + integration tests across all crates (`cargo test --workspace`)
- [x] Zero clippy warnings: `cargo clippy --workspace -- -D warnings`
- [x] Consistent formatting: `cargo fmt --all`
- [x] GitHub Actions CI workflow on `push` / `pull_request` to `main`
- [x] Codecov coverage reporting

### Documentation & Release Artifacts

- [x] `README.md` with badges (CI, codecov, Rust version, license)
- [x] `ROADMAP.md` (this document)
- [x] `CHANGELOG.md` with v0.1.0 entries
- [ ] `LICENSE-MIT` and `LICENSE-APACHE`
- [ ] `config.example.toml` with all options documented
- [ ] GitHub Release with pre-built binaries:
  - [ ] `aarch64-apple-darwin` (macOS Apple Silicon)
  - [ ] `x86_64-unknown-linux-gnu` (Linux x86_64)
  - [ ] `aarch64-unknown-linux-gnu` (Linux ARM64)
  - [ ] `x86_64-unknown-linux-musl` (Linux x86_64 static)
  - [ ] `aarch64-unknown-linux-musl` (Linux ARM64 static)
- [ ] `crates.io` publish for library crates (optional)

---

## v0.2.0 — Screen & Browser

Extends the tool surface to visual OS control.

- `screen.capture` — base64/file screenshot via `screenshots` crate
- `screen.ocr` — text extraction from screen regions
- `browser.navigate` — Chromium CDP via `chromiumoxide`
- `browser.click`, `browser.type`, `browser.screenshot`
- `system.notify` — desktop notifications via `notify-rust`
- Discord adapter
- Slack adapter
- Prometheus metrics export (`metrics-exporter-prometheus`)

---

## v0.3.0 — Voice & Multi-Agent

- [ ] MCP (Model Context Protocol) client — connect openpista to any MCP-compatible tool server
- [ ] MCP tool discovery and dynamic registration into `ToolRegistry`
- [ ] MCP resource and prompt support
- [ ] Configuration: `[mcp]` section in `config.toml` with server URLs

### Plugin System

- [ ] Plugin trait for third-party tool extensions
- [ ] Dynamic loading via shared library (`.dylib` / `.so`) or WASM
- [ ] Plugin manifest format and discovery from `~/.openpista/plugins/`
- [ ] Plugin lifecycle: load → register tools → unload

### Additional Channel Adapters

- [ ] `DiscordAdapter` — Discord bot via `serenity` crate, slash commands, thread-based sessions
- [ ] `SlackAdapter` — Slack bot via Bolt-style HTTP events API, channel/thread sessions

### Observability

- [ ] Prometheus metrics export via `metrics-exporter-prometheus`
- [ ] Key metrics: request latency, tool call count, error rate, active sessions, memory usage
- [ ] `/metrics` HTTP endpoint on configurable port
- [ ] OpenTelemetry tracing integration (optional)
- [ ] Structured logging with `tracing-subscriber` JSON output

### Worker Report System

> Worker containers currently collect results in-process. This section tracks future enhancements for worker reporting, monitoring, and history.

 [ ] Worker report receiver endpoint — HTTP POST route (`/api/worker-report`) via `axum` for accepting worker execution results
 [ ] Report authentication — validate worker task tokens on report submission (reuse existing `TaskCredential` from ContainerTool)
 [ ] Report acknowledgement protocol — reliable delivery with structured ACK/NACK responses
 [ ] Retry with exponential backoff — transient failure handling in ContainerTool HTTP client
 [ ] Offline report buffer — queue failed reports to local disk (`~/.openpista/report-queue/`), replay on connectivity restored
 [ ] Worker status WebSocket feed — live progress updates pushed to TUI/Web UI for active container executions
 [ ] Worker execution history API — query past worker reports via REST endpoint or TUI `/worker` command
 [ ] Worker dashboard in TUI — dedicated screen showing active/completed/failed worker executions with logs

---

## v1.0.0 — Production Ready

- Stable public API for all crates
- Full end-to-end security review
- Long-term support guarantee
- Packaging: `brew`, `apt`, `winget`, Docker image
