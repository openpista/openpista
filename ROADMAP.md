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
- [x] `WebAdapter` — Rust→WASM browser client with WebSocket transport (see Web Channel Adapter section)

### Telegram Channel Adapter

> Telegram uses the teloxide framework with long-polling for message delivery. The adapter supports MarkdownV2 formatting, automatic message splitting for long responses, and an optional user whitelist for security — critical since openpista has OS-level tool access.

- [x] `TelegramAdapter` — `teloxide` dispatcher with stable per-chat sessions
- [x] Stable session mapping: `telegram:{chat_id}` channel ID and session routing
- [x] `TelegramConfig` — `[channels.telegram]` config section: `enabled`, `token`, `allowed_users`
- [x] Environment variable overrides: `TELEGRAM_BOT_TOKEN`, `TELEGRAM_ALLOWED_USERS`
- [x] Text message parsing and `ChannelEvent` construction
- [x] Response routing: Telegram responses → Bot API `send_message`
- [x] Error responses clearly surfaced to the user (❌ prefix, consistent with other adapters)
- [x] Typing indicator (`ChatAction::Typing`) sent on message receipt
- [x] MarkdownV2 formatting with automatic plain-text fallback on parse failure
- [x] Long message splitting (4096 char limit) — splits on paragraph, line, then hard boundary
- [x] User whitelist security: `allowed_users` restricts who can interact (empty = allow all)
- [x] Unit tests: session ID, chat ID parsing, response formatting, message splitting, MarkdownV2 escaping, whitelist logic

#### Baseline Pre-requisites

> These foundational enhancements must land before the tasks below — several tasks depend on them.

- [ ] `TelegramConfig` expansion: `allowed_users: Vec<i64>`, `webhook_url: Option<String>`, `webhook_port: u16`, `webhook_secret: Option<String>`, `confirm_actions: bool`
- [ ] `TELEGRAM_ALLOWED_USERS` env var parsing (comma-separated `i64` list)
- [ ] `send_chunked(bot, chat_id, text)` helper: split → try MarkdownV2 → fallback to plain text per chunk
- [ ] Typing indicator fire-and-forget pattern (log `warn` on error, never propagate)

#### Upcoming Tasks

- [ ] **Bot commands registration** (`/start`, `/help`, `/status`) via `BotCommands` derive
  - Add `BotCmd` enum with `#[derive(BotCommands, Clone)]` and `rename_rule = "lowercase"`
  - Call `Bot::set_my_commands::<BotCmd>()` at startup — registers Telegram's "/" popup menu
  - Add command handler branch to `dptree` via `filter_command::<BotCmd>()` alongside text handler
  - `/status` calls `bot.get_me()` to display bot username and ID; fetch and store `bot_username: Arc<String>` at startup for reuse in group chat filter
  - Unit tests: static responses for `/start`, `/help`; `get_me` response for `/status`

- [ ] **Retry logic** with exponential backoff for transient Bot API failures
  - `send_with_retry<F>(f: F, max_attempts: u32)` generic wrapper around any bot API call
  - `RequestError::RetryAfter(secs)` → sleep exact duration, then retry
  - Network/5xx transient errors → exponential backoff starting at 500 ms, doubling, capped at 30 s
  - Non-retryable (`BadRequest`, `Unauthorized`) propagate immediately without retry
  - Wrap `send_message` in `send_response`; wrap `send_chat_action` typing call with max 1 retry
  - Unit tests: mock `RetryAfter` behavior, verify backoff cap

- [ ] **Group chat support**: reply-to-bot filtering, thread-based sessions
  - `is_message_for_bot(msg, bot_username)` — private chats always pass; groups require reply-to-bot or `@mention` in text
  - Fetch `bot_username` via `bot.get_me()` once at startup, store as `Arc<String>` (shared with bot commands task)
  - Thread-aware session ID: `telegram:{chat_id}:{thread_id}` for supergroup forum topics, fall back to `telegram:{chat_id}`
  - Wire filter as `dptree::filter(move |msg: Message| is_message_for_bot(&msg, &bot_username))`
  - Unit tests: private vs. group filter, forum thread session ID generation, `@mention` detection

- [ ] **Media message handling**: photo, document, voice, video note forwarding to agent context
  - Add `dptree` branches for `msg.photo()`, `msg.document()`, `msg.voice()`, `msg.video_note()`
  - Small files (< 5 MB): `bot.get_file(file_id)` → download via `reqwest` → base64-encode into `ChannelEvent.metadata`
  - Large files: describe as agent-readable text in `user_message` (filename, MIME type, size in bytes)
  - Caption forwarded as `user_message`; structured `MediaInfo` JSON stored in `metadata: Option<serde_json::Value>`
  - Apply whitelist + group bot-mention filter to all media handler branches
  - Unit tests: `format_media_description` for each variant; base64 encoding round-trip

- [ ] **Rate limiting**: respect Telegram's 30 msg/sec per chat, 20 msg/min to same group
  - Per-chat `governor::RateLimiter` stored in `DashMap<i64, Arc<RateLimiter<...>>>` on the adapter struct
  - Private chats: `Quota::per_second(nonzero!(30u32))`; groups (negative `chat_id`): additional `Quota::per_minute(nonzero!(20u32))`
  - Before each chunk send, acquire permit; if limiter prescribes a wait, `tokio::time::sleep` that duration
  - Add `governor = "0.6"` to workspace `Cargo.toml` and `crates/channels/Cargo.toml`
  - Unit tests: group vs. private chat quota selection; rate state isolation across chat IDs

- [ ] **Inline keyboard support**: interactive tool confirmations and menu navigation
  - `parse_inline_keyboard(text) -> (String, Option<InlineKeyboardMarkup>)` DSL: `[[label|callback_data]]` markers → button rows + cleaned text
  - `Update::filter_callback_query()` branch in `dptree` → `handle_callback_query` endpoint
  - `handle_callback_query`: `bot.answer_callback_query(query.id)`, emit `ChannelEvent` with `[callback:{data}]` as `user_message`
  - `confirm_actions: bool` in `TelegramConfig` gates DSL parsing in `send_response` (default `false`)
  - Unit tests: DSL parser layout, multi-row buttons, callback event construction

- [ ] **Webhook mode** (`teloxide::dispatching::update_listeners::webhooks`) for production deployments
  - `TelegramConfig` fields: `webhook_url: Option<String>`, `webhook_port: u16` (default 8443), `webhook_secret: Option<String>`
  - Env overrides: `TELEGRAM_WEBHOOK_URL`, `TELEGRAM_WEBHOOK_PORT`
  - Add `webhooks-axum` feature to teloxide in workspace `Cargo.toml`
  - `run()` branches on `config.webhook_url`: `webhooks::axum(bot, Options::new(addr, url))` vs long-polling fallback
  - Lifecycle: `set_webhook` on startup, `delete_webhook` on shutdown — allows clean revert to polling
  - Long-polling remains default; webhook is opt-in via config
  - Sub-tasks: TLS termination via reverse proxy (nginx/caddy); document deployment pattern in `TELEGRAM.md`

- [ ] **`/telegram status` TUI command** — show bot info, webhook/polling mode, connected chats
  - Follows `/whatsapp status` pattern in `crates/cli/src/tui/event.rs`
  - `parse_telegram_command(raw) -> Option<TelegramCommand>` and `format_telegram_status(config) -> String`
  - Display: enabled, token presence, polling vs. webhook mode, webhook URL, allowed users count
  - Add `"/telegram"` and `"/telegram status"` entries to `SLASH_COMMANDS` in `app.rs`
  - Implement after `TelegramConfig` is stabilised (all new fields present)

- [ ] **Integration test**: end-to-end message → `ChannelEvent` → `AgentResponse` → Telegram send flow
  - Use `Bot::new(token).set_api_url(mock_url)` to redirect HTTP calls to a `wiremock` mock server
  - Mock endpoints: `getMe`, `setMyCommands`, `sendChatAction`, `sendMessage` with canned JSON responses
  - Test cases: text → event, media → event with metadata, MarkdownV2 fallback, whitelist rejection, retry on 429, group `@mention` filter
  - Add `wiremock = "0.6"` as dev-dependency in `crates/channels/Cargo.toml`
  - Written last when implementation is frozen

#### Implementation Order

```
Baseline (config expansion → send_chunked → typing indicator)
    │
    ├── Bot Commands (S) ── fetch bot_username at startup
    │       │
    │       └── Group Chat (M) ── reuse bot_username; update session IDs
    │               │
    │               └── Media (M) ── reuse group filter closure
    │
    ├── Retry Logic (S) ── wrap send_chunked
    │       │
    │       └── Rate Limiting (M) ── wrap retry-wrapped send
    │               │
    │               └── Inline Keyboard (M) ── add callback query branch
    │
    ├── Webhook Mode (L) ── orthogonal dispatch path (add after group/command tasks)
    │
    ├── /telegram status TUI (S) ── once TelegramConfig fields are stable
    │
    └── Integration Test (M) ── written last when implementation is frozen
```

#### Reference Open-Source Projects

> **teloxide ecosystem**
>
> | Project | Description |
> |---------|-------------|
> | [`teloxide`](https://github.com/teloxide/teloxide) | Primary Telegram bot framework for Rust — dialogue system, `dptree` dispatcher, command parsing, inline queries. openpista's adapter is built on this. |
> | [`teloxide-core`](https://github.com/teloxide/teloxide-core) | Low-level Telegram Bot API bindings — all types, methods, and request builders. Used internally by teloxide. |
> | [`teloxide-macros`](https://github.com/teloxide/teloxide/tree/master/crates/teloxide-macros) | `#[derive(BotCommands)]` macro — strongly-typed command parsing with auto-generated `bot_commands()` registration vector. Key for bot commands task. |
> | [`dptree`](https://github.com/teloxide/dptree) | Dependency-injection handler tree — teloxide's message routing engine. Understanding this helps extend the adapter. |
>
> **teloxide official examples**
>
> | Example | Description |
> |---------|-------------|
> | [`dispatching_features.rs`](https://github.com/teloxide/teloxide/blob/master/crates/teloxide/examples/dispatching_features.rs) | Advanced `dptree` branching — handles both commands and `CallbackQuery` in one dispatcher. Direct reference for inline keyboard task. |
> | [`purchase.rs`](https://github.com/teloxide/teloxide/blob/master/crates/teloxide/examples/purchase.rs) | Inline keyboard + callback query in a dialogue state machine — shows `InlineKeyboardMarkup`, `answer_callback_query`, and state transitions. |
> | [`ngrok_ping_pong.rs`](https://github.com/teloxide/teloxide/blob/master/crates/teloxide/examples/ngrok_ping_pong.rs) | Minimal webhook mode example using `webhooks::axum` with an ngrok tunnel — reference for webhook task. |
> | [teloxide examples](https://github.com/teloxide/teloxide/tree/master/crates/teloxide/examples) | Full collection: dialogues, inline keyboards, webhooks (axum), purchase bot, command handlers. |
>
> **Group chat & production bots (Rust)**
>
> | Project | Description |
> |---------|-------------|
> | [`grpmr-rs`](https://github.com/dracarys18/grpmr-rs) | Production group management bot in Rust + teloxide — demonstrates group-scoped event handling, reply filtering, moderation actions, and MongoDB persistence. Reference for group chat task. |
> | [`frankenstein`](https://github.com/ayrat555/frankenstein) | Type-safe Telegram Bot API client with sync/async support — useful reference for raw API coverage. |
> | [`tbot`](https://github.com/tbot-rs/tbot) | Opinionated Telegram bot framework — interesting for its event loop and middleware patterns. |
>
> **Rate limiting & retry**
>
> | Project | Description |
> |---------|-------------|
> | [`governor`](https://github.com/antifuchs/governor) | Token-bucket / GCRA rate limiter crate — used to implement per-chat and per-group send quotas. Zero unsafe, `tokio`-compatible. |
> | [`telegram-rate-limiter`](https://github.com/mediv0/telegram-rate-limiter) | Purpose-built per-chat rate limiter for Telegram bots — reference for the quota model (30/s global, 20/min group). |
>
> **Integration testing**
>
> | Project | Description |
> |---------|-------------|
> | [`wiremock`](https://github.com/LukeMathWalker/wiremock-rs) | HTTP mock server for Rust — used with `Bot::set_api_url(mock_url)` to intercept Telegram API calls in integration tests without hitting real servers. |
>
> **AI agent + Telegram patterns**
>
> | Project | Description |
> |---------|-------------|
> | [`zeroclaw`](https://github.com/zeroclaw-labs/zeroclaw) | Trait-based `Channel` pattern nearly identical to openpista's `ChannelAdapter`. Multi-channel including Telegram. |
> | [`llm-chain`](https://github.com/sobelio/llm-chain) | LLM orchestration framework with agent patterns — reference for ReAct loop + chat adapter integration. |
>
> **Webhook + axum patterns**
>
> | Resource | Description |
> |----------|-------------|
> | [teloxide axum webhook example](https://github.com/teloxide/teloxide/blob/master/crates/teloxide/examples/axum_webhook.rs) | Official axum webhook example — reference for production webhook mode with TLS termination. |


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
- [x] WebSocket message framing: JSON `ChannelEvent` / `AgentResponse` over WS text frames
- [x] Token-based authentication on WebSocket handshake (`Sec-WebSocket-Protocol` or query param)
- [x] `WebConfig` — `[channels.web]` config section: `port`, `token`, `cors_origins`, `static_dir`
- [x] Environment variable overrides: `openpista_WEB_TOKEN`, `openpista_WEB_PORT`
- [x] Session mapping: `web:{client_id}` channel ID with stable session per authenticated client
 - [ ] Auto-reconnect support: client-side heartbeat ping/pong, server-side timeout detection
- [x] CORS configuration for cross-origin browser access
 - [ ] WSS (TLS) support via reverse proxy or built-in `axum-server` with `rustls`
- [x] Configurable static file directory for WASM bundle and H5 assets

#### Client (Rust→WASM)

- [x] Rust client crate (`crates/web/`) compiled to `wasm32-unknown-unknown` via `wasm-pack`
- [x] `wasm-bindgen` JS interop: WebSocket API, DOM manipulation, localStorage
- [x] WebSocket connection manager: connect, disconnect, is_connected
- [x] Message serialization: `serde_json` in WASM for `ChannelEvent` / `AgentResponse`
- [x] Session persistence: `localStorage` for session ID across page reloads
 - [ ] H5 chat UI: mobile-responsive chat interface (HTML/CSS/JS or Yew/Leptos framework)
 - [ ] Streaming response display: progressive text rendering as agent generates output
 - [ ] Slash command support: `/model`, `/session`, `/clear`, `/help` from web UI input
 - [ ] Media attachment support: image upload → base64 encoding → agent context
 - [ ] PWA manifest: installable as home screen app (offline shell + online WebSocket)
 - [ ] `wasm-pack build --target web` build pipeline in CI

#### Quality

- [x] Unit tests: WebSocket message parsing, token auth, session ID, CORS layer
 - [ ] Integration test: browser → WebSocket → `ChannelEvent` → `AgentResponse` → browser render
 - [ ] WASM bundle size optimization: `wasm-opt`, tree shaking, gzip/brotli serving

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

- [x] 699 unit + integration tests across all crates (`cargo test --workspace`)
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
