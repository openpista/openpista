# Roadmap

> **openpista** — An autonomous AI agent that controls your OS through any messenger.

---

## v0.1.0 — Initial Autonomous Agent Release

The first public release establishes the core autonomous loop: the LLM receives a message, reasons over available tools, executes OS commands, and replies — all without manual intervention.

### Core Runtime

- [x] Agent ReAct loop (LLM → tool call → result → LLM → text response)
- [x] `LlmProvider` trait with OpenAI-compatible adapter (`async-openai`)
- [x] `ToolRegistry` — dynamic tool registration and dispatch
 - [x] Configurable max tool rounds to prevent infinite loops (default: 30)
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
> All channel adapters use their native protocols (stdin, HTTP polling, WebSocket) and bridge into the in-process gateway through `tokio::mpsc` channels.
>
> ```
> CliAdapter ─── stdin/stdout ──→ mpsc ──→ Gateway
> TelegramAdapter ── HTTP poll ──→ mpsc ──→ Gateway
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
 - [x] `WebAdapter` — axum WebSocket server + static H5 chat UI (`static/`) serving; Rust→WASM client in progress (see Web Channel Adapter section)


### WhatsApp Channel Adapter

> WhatsApp uses a subprocess bridge pattern: openpista spawns a Node.js process (Baileys) that connects to WhatsApp Web. Communication is via JSON lines over stdin/stdout. Users pair by scanning a QR code — no API keys needed.

 [x] `WhatsAppAdapter` — WhatsApp Web multi-device protocol via Node.js Baileys bridge subprocess
 [x] QR code pairing flow in TUI (`/whatsapp` command) — no API keys needed
 [x] Subprocess bridge protocol (JSON lines over stdin/stdout) for Rust ↔ Node.js communication
 [x] Session persistence (`session_dir/auth/`) — reconnects automatically after restart
 [x] Stable per-conversation sessions: `whatsapp:{sender_phone}` channel ID and session mapping
 [x] `WhatsAppConfig` — `[channels.whatsapp]` config section: `enabled`, `session_dir`, `bridge_path`
 [x] Environment variable overrides: `WHATSAPP_SESSION_DIR`, `WHATSAPP_BRIDGE_PATH`
 [x] Incoming message parsing: text (image, audio, video, document — future)
 [x] `/whatsapp status` TUI command — shows pairing status and config
 - [ ] Media message download and forwarding (incoming media → base64 or local path for agent context)
 - [ ] Interactive message support: reply buttons, list messages, quick replies
 - [ ] Retry logic with exponential backoff for transient connection failures
- [x] Error responses clearly surfaced to the user (consistent with other adapters)
- [x] Response routing integration: WhatsApp responses → Graph API `send_message`
 - [ ] Multi-number support: configurable phone number IDs for business accounts with multiple numbers
- [x] Unit tests: webhook verification, message parsing, session ID generation, response formatting, signature validation
 - [ ] Integration test: end-to-end webhook → `ChannelEvent` → `AgentResponse` → WhatsApp send flow

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

 - [x] `WebAdapter` — axum HTTP server: WebSocket upgrade + static file serving for WASM bundle
 - [x] WebSocket message framing: JSON `WsMessage` envelope (`UserMessage`, `AgentReply`, `Ping`, `Pong`, `Auth`, `AuthResult`, `ModelChange`, `ModelChanged`, `ModelList`, `ProviderAuthRequest`, `ProviderAuthStatus`, `ProviderLogin`) over WS text frames
 - [x] Two-phase authentication: `POST /auth` with session tokens + WebSocket upgrade with `?session_token=`; legacy `?token=` query param fallback
 - [x] `WebConfig` — `[channels.web]` config section: `port`, `token`, `cors_origins`, `static_dir`
 - [x] Environment variable overrides: `openpista_WEB_TOKEN`, `openpista_WEB_PORT`
 - [x] Session mapping: `web:{client_id}` channel ID with stable session per authenticated client
 - [x] Auto-reconnect support: `Ping`/`Pong` keepalive with client-side exponential backoff reconnect loop; stale token recovery
 - [x] URL-based session routing: `GET /s/{session_id}` serves `index.html` for direct session access
 - [x] CORS configuration for cross-origin browser access
 - [x] Web OAuth PKCE provider authentication: browser-initiated OAuth for OpenAI, Anthropic, OpenRouter, GitHub Copilot; API key input for Together, Ollama, custom providers
 - [x] Runtime model switching via WebSocket: `model_change` → `model_changed` without session disconnect
 - [x] Agent processing timeout: 120-second `tokio::time::timeout` wrapper on `runtime.process()` to prevent silent response drops
 - [x] Per-client response routing via `DashMap` with cloned sender pattern (no lock-across-await)
 - [x] Configurable static file directory for WASM bundle and H5 assets
 - [x] Tool call approval system: inline approval UI in chat for all channels; `ToolApprovalRequest` / `ToolApprovalResponse` WebSocket message pair; per-session "allow all" toggle
 - [x] Conversation history validation: strip orphaned `tool_use` blocks when switching providers mid-conversation; empty output ≠ auth error when tool history exists
 - [x] Screen capture sanitization: strip multi-MB `data_b64` fields from tool output before sending to LLM; full data preserved in frontend for inline image rendering
 - [ ] WSS (TLS) support via reverse proxy or built-in `axum-server` with `rustls`

#### Client (Rust→WASM)

 - [x] Rust client crate (`crates/web/`) compiled to `wasm32-unknown-unknown` via `wasm-pack`
 - [x] `wasm-bindgen` JS interop: WebSocket API, DOM manipulation, localStorage
 - [x] WebSocket connection manager: connect, auto-reconnect with exponential backoff (1s–30s, max 10 attempts), `Ping`/`Pong` heartbeat
 - [x] Message serialization: `serde_json` in WASM for `ChannelEvent` / `AgentResponse`
 - [x] Session persistence: `localStorage` for client ID, auth token, session token across page reloads
 - [x] Token persistence: authenticate once per device; saved token auto-connects on page load
 - [x] H5 chat UI: mobile-responsive chat interface (`static/index.html` + `style.css` + `app.js`; vanilla JS, not yet Rust→WASM)
 - [x] Session management: sidebar session list with create (New Chat), Claude-style ⋯ context menu (Rename / Delete with confirmation dialog)
 - [x] Session name customization: editable inline with localStorage persistence; defaults to truncated session ID
 - [x] URL-based session access: `/s/{session_id}` path routing with automatic session load
 - [x] Model selector: dropdown in navigation bar showing models grouped by provider; filtered to only show models from authenticated providers
 - [x] Dynamic model switching: change model within active session without reconnecting; server-side model swap preserves conversation
 - [x] Markdown rendering: agent responses rendered as HTML (headings, code blocks, lists, tables, blockquotes, links, emphasis)
 - [x] Responsive scrollable chat layout: auto-scroll to bottom on new messages and during agent thinking
 - [x] Thinking indicator: animated bouncing dots shown while agent is processing
 - [x] Toast notifications: non-intrusive system event messages (connection status, model changes, auth results)
 - [x] Provider authentication modal: full-screen modal showing all 11 providers with auth status dots; supports OAuth login, API key input, endpoint+key configuration, and authorization code input
 - [x] Auth-filtered model selector: server-side filtering to show only models from authenticated providers; frontend re-requests model list on auth status change
 - [x] Stop generating: Claude-style circular stop button + ESC key cancellation; backend drops client response channel to halt agent; `CancelGeneration` / `GenerationCancelled` WebSocket message pair
 - [x] Security hardening: credential file `chmod 600` on Unix; `noopener,noreferrer` on OAuth popups; 10-minute TTL on pending PKCE flows
 - [x] Inline tool approval: tool call approval rendered inside chat flow (not modal overlay); Allow / Deny / Allow All buttons with soft styling; pending timeouts paused during approval wait
 - [x] Inline image rendering: `screen.capture` base64 data rendered as `<img>` tags in chat; sanitized tool output prevents base64 flooding
 - [x] Automated Trunk build pipeline: `scripts/build-web.sh` — sync static → JS validation (`node -c`) → `trunk build --release` → deploy to `~/.openpista/web/`; optional `--restart` flag
 - [x] Legacy model cleanup: removed deprecated model entries from catalog; all models configured with maximum reasoning effort and thinking mode enabled
 - [x] Cross-provider conversation fix: conversation history validated on provider switch to prevent `tool_use` ID mismatch errors; empty LLM output correctly handled when tool history exists
 - [ ] Streaming response display: progressive text rendering as agent generates output
 - [ ] Slash command support: `/model`, `/session`, `/clear`, `/help` from web UI input
 - [ ] Media attachment support: image upload → base64 encoding → agent context
 - [ ] PWA manifest: installable as home screen app (offline shell + online WebSocket)
 - [ ] `wasm-pack build --target web` build pipeline in CI

#### Quality

 - [x] Unit tests: WebSocket handshake, session token auth, message framing, ping/pong, CORS, session mapping, response routing, broadcast fallback — 25+ tests (`channels/src/web.rs`)
 - [ ] Integration test: browser → WebSocket → `ChannelEvent` → `AgentResponse` → browser render
 - [ ] WASM bundle size optimization: `wasm-opt`, tree shaking, gzip/brotli serving

#### Reference Open-Source Projects

> **Axum WebSocket server patterns**
>
> | Project | Description |
> |---------|-------------|
> | [axum — chat example](https://github.com/tokio-rs/axum/blob/main/examples/chat/src/main.rs) | Official Axum broadcast-based WebSocket chat example. Best starting point for `WebSocketUpgrade` + `tokio::sync::broadcast`. |
> | [axum — websockets example](https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs) | Official Axum WebSocket example demonstrating general WS handling, ping/pong, and connection lifecycle. |
> | [0xLaurens/chatr](https://github.com/0xLaurens/chatr) | Chat room with WebSocket and Axum, demonstrating room-based session architecture. |
> | [kumanote/axum-chat-example-rs](https://github.com/kumanote/axum-chat-example-rs) | Axum WebSocket chat with Dragonfly (Redis-compatible) pub/sub for multi-instance scaling. |
> | [`danielclough/fireside-chat`](https://github.com/danielclough/fireside-chat) | LLM chat bot: Axum WebSocket backend + Leptos WASM frontend — closest architecture match to openpista's target Web adapter design. |
> | [`dawnchan030920/axum-ycrdt-websocket`](https://github.com/dawnchan030920/axum-ycrdt-websocket) | Axum WebSocket middleware with per-connection state and multi-client broadcast — good reference for room-aware WebSocket handler patterns. |
>
> **Rust→WASM WebSocket client libraries**
>
> | Crate | Description |
> |-------|-------------|
> | [wasm-bindgen — WebSocket example](https://github.com/rustwasm/wasm-bindgen/tree/main/examples/websockets) | Canonical `web-sys` WebSocket example from wasm-bindgen. Foundation for WASM WS clients. |
> | [`ewebsock`](https://github.com/rerun-io/ewebsock) | Cross-platform (native + WASM) WebSocket client with a single unified API. By the Rerun team. |
> | [`tokio-tungstenite-wasm`](https://github.com/TannerRogalsky/tokio-tungstenite-wasm) | Wraps `web-sys` (WASM) and `tokio-tungstenite` (native) behind one cross-platform API. ~343k crates.io downloads. |
> | [`ws_stream_wasm`](https://github.com/najamelan/ws_stream_wasm) | WASM-only WebSocket library providing `AsyncRead`/`AsyncWrite` via `WsMeta`/`WsStream`. Most ergonomic for stream-based patterns. |
> | [`cunarist/tokio-with-wasm`](https://github.com/cunarist/tokio-with-wasm) | Tokio runtime emulation in the browser — enables `tokio::spawn`, `spawn_blocking` in WASM contexts. |
> | [`gloo-net`](https://github.com/rustwasm/gloo) | Ergonomic `web-sys` wrappers from the official `rustwasm` org; `gloo_net::websocket` gives a clean `Stream`/`Sink` WebSocket API — simpler than raw `web-sys`. |
>
> **Rust WASM frontend frameworks (for H5 chat UI)**
>
> | Framework | Description |
> |-----------|-------------|
> | [`yew`](https://github.com/yewstack/yew) (~30.5k stars) | Most mature Rust/WASM frontend framework. React-like component model with JSX-style `html!` macros. |
> | [`leptos`](https://github.com/leptos-rs/leptos) (~20k stars) | Full-stack isomorphic Rust framework with fine-grained reactivity. Native WebSocket support for server functions via `Stream`. |
> | [`dioxus`](https://github.com/DioxusLabs/dioxus) (~24.5k stars) | Cross-platform (web + desktop + mobile) app framework. Deep Axum integration for full-stack Rust. |
> | [`leptos-use` — `use_websocket`](https://github.com/Synphonyte/leptos-use) | Reactive WebSocket hook for Leptos with codec support. Inspired by VueUse. |
> | [`leptos_server_signal`](https://github.com/tqwewe/leptos_server_signal) | Leptos signals synced with server through WebSocket. Supports Axum and Actix backends. |
> | [`security-union/yew-websocket`](https://github.com/security-union/yew-websocket) | Standalone Yew WebSocket service crate (extracted from Yew core). |
>
> **Full-stack Rust WebSocket chat references**
>
> | Project | Description |
> |---------|-------------|
> | [`YewChat`](https://github.com/jtordgeman/YewChat) | Complete WebSocket chat app built with Yew — routing, agents, GIF support. Companion to a popular tutorial series. |
> | [`ztm-project-uchat`](https://github.com/jayson-lennon/ztm-project-uchat) | Full-stack chat clone entirely in Rust: WASM frontend (Trunk), Diesel ORM, PostgreSQL, Tailwind CSS. |
> | [`fullstack-rust-axum-dioxus-rwa`](https://github.com/dxps/fullstack-rust-axum-dioxus-rwa) | RealWorld app with Axum backend + Dioxus WASM frontend — auth, routing, CRUD. |
> | [`rust-axum-leptos-wasm`](https://github.com/hvalfangst/rust-axum-leptos-wasm) | Full-stack Axum + Leptos WASM with JWT-protected endpoints. |
> | [`veklov/rust-chat`](https://github.com/veklov/rust-chat) | Web chat with Warp backend + Yew/WASM frontend, includes end-to-end WebDriver tests. |
> | [`ProstoyVadila/ws_chat`](https://github.com/ProstoyVadila/ws_chat) | Backend and frontend both in Rust — demonstrates the full-stack Rust WebSocket approach with separate server/client crates. |
> | [`bestia-dev/mem6_game`](https://github.com/bestia-dev/mem6_game) | Multi-player browser game in Rust WASM with real-time WebSocket and PWA service worker — the only reference covering WASM + WebSocket + PWA simultaneously. |
>
> **WASM build tooling & PWA**
>
> | Tool | Description |
> |------|-------------|
> | [`trunk`](https://github.com/trunk-rs/trunk) (~4.2k stars) | Leading WASM web application bundler for Rust. Built-in dev server with hot reload. Works with Yew, Leptos, Dioxus. |
> | [`wasm-pack`](https://github.com/rustwasm/wasm-pack) (~6.5k stars) | Classic Rust→WASM workflow tool. Note: `rustwasm` org archived mid-2025; community fork at [drager/wasm-pack](https://github.com/drager/wasm-pack). |
> | [`yew-wasm-pack-template`](https://github.com/nickkos/nickkos/yew-wasm-pack-template) | Full-stack PWA template: Yew frontend + Actix backend, with Workbox service worker. |
> | [`woz`](https://github.com/nickkos/nickkos/alexkehayias/woz) | Progressive WebAssembly App (PWAA) generator for Rust — PWA + WASM tooling in a single CLI. |
> | [`wasm-bindgen-service-worker`](https://github.com/justinrubek/wasm-bindgen-service-worker) | Service worker written entirely in Rust via wasm-bindgen. Minimal JS glue. |

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
- [x] Multi-provider OAuth PKCE: OpenAI, Anthropic, OpenRouter, GitHub Copilot
- [x] GitHub Copilot PKCE OAuth — subscription-based auth via GitHub OAuth → Copilot session token exchange
- [x] Provider login picker with search, OAuth/API-key method chooser, and credential status dots
- [x] Internal TUI slash commands (`/help`, `/login`, `/clear`, `/quit`, `/exit`)
- [x] Centralized, Landing Page-style TUI with dedicated Home and Chat screens
- [x] TOML config file with documented examples (`config.toml`)
- [x] Environment variable override for all secrets
- [x] PID file written on start, removed on exit
- [x] `SIGTERM` + `Ctrl-C` graceful shutdown
- [x] Elm Architecture (TEA) reactive TUI — unidirectional data flow (`Action → update() → State → view()`)

### Multi-Provider Authentication

- [x] OAuth 2.0 PKCE browser login for OpenAI (ChatGPT Plus/Pro subscription)
- [x] OAuth 2.0 PKCE code-display flow for Anthropic (Claude Max subscription)
- [x] GitHub Copilot PKCE OAuth: GitHub OAuth → `copilot_internal/v2/token` session token exchange
- [x] OpenAI `id_token` → API key exchange for subscription-billing Responses API
- [x] Automatic token refresh with 5-minute pre-expiry window
- [x] Credential store in `~/.openpista/credentials.toml` with per-provider tokens
- [x] Extension provider slots: GitHub Copilot, Google, Vercel AI Gateway, Azure OpenAI, AWS Bedrock
- [x] `openpista auth status` — show all stored provider credentials and expiry
- [x] `openpista auth logout` — per-provider credential removal

### Quality & CI

- [x] 755 unit + integration tests across all crates (`cargo test --workspace`)
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
