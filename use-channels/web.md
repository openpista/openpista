# Web Channel Usage Guide

This guide explains how to use the web channel with the current CLI implementation.

## Overview

`openpista web setup` does the following:

1. Saves `[channels.web]` config.
2. Installs static web assets from `crates/channels/static` to `channels.web.static_dir`.
3. Optionally updates/generates a WebSocket auth token.

Default values:

- `port`: `3210`
- `static_dir`: `~/.openpista/web`
- `enabled`: `false`

## Prerequisites

| Requirement | Description |
|---|---|
| openpista binary | Built from source (`cargo build --release`) or installed |
| Modern browser | For accessing the web UI (Chrome, Firefox, Safari, Edge) |

## Quick start

```bash
# optional: choose model shown in web UI
./target/release/openpista model select

# setup web config + static install
./target/release/openpista web setup --enable --port 3210

# start web daemon mode
./target/release/openpista web start
```

At startup, CLI prints:

- `http: http://127.0.0.1:<port>`
- `ws: ws://127.0.0.1:<port>/ws`
- `health: http://127.0.0.1:<port>/health`

Open the web page. The login modal appears first. Enter token and click `Connect`.

## UI behavior

- Layout: ChatGPT-style two-column shell
  - Left sidebar: session list + runtime metadata
  - Main pane: message area + bottom composer
- Theme mode: system auto (`prefers-color-scheme`)
  - dark and light are switched by OS/browser preference
- Runtime metadata shown in sidebar:
  - `status`
  - `Model`
  - `Session`
  - `Shared with TUI`

## Authentication modal flow

- Not authenticated: login modal is shown and chat interaction is blocked.
- Auth success: modal hides, chat becomes active.
- Auth failure: modal stays visible and error message is shown.
- Connection closed before authentication: modal reappears.

## Check status

```bash
./target/release/openpista web status
```

`web status` shows config and runtime state:

- config (`enabled`, `token set`, `port`, `cors_origins`, `static_dir`, `shared_session_id`)
- runtime (`pid`, process alive, health)
- overall (`running`, `partial`, `stopped`)

## Token behavior

- If `channels.web.token` is non-empty, clients must provide that token in the web login modal.
- If token is empty, WebSocket auth is effectively open.
- `web setup` can set or regenerate token:

  ```bash
  # set token manually
  ./target/release/openpista web setup --token "your-token"

  # force token regeneration
  ./target/release/openpista web setup --regenerate-token

  # auto-confirm regenerate prompt
  ./target/release/openpista web setup --yes
  ```

`web setup` token behavior summary:

- On first run with no token: token is auto-generated.
- With existing token in an interactive terminal: prompt asks whether to regenerate.
- In non-interactive mode: existing token is preserved unless `--regenerate-token` is passed.

## Model display and change

The web page displays the currently selected model in sidebar metadata.

- If you run `openpista model select`, the selected provider/model is reflected in the sidebar.
- Example: `Model: anthropic/claude-sonnet-4-6`

You can also change the model from the web UI at runtime without restarting the daemon:

1. Open the model selector in the web UI.
2. Select a provider and model from the list.
3. The sidebar updates immediately.

The change takes effect for all subsequent messages in the current session.

## Provider authentication from web UI

Providers that require authentication (OpenAI, Anthropic, OpenRouter, etc.) can be
configured directly from the web UI:

- **OAuth (redirect)**: Server sends an auth URL; browser opens it; callback completes auth.
- **OAuth (code display)**: Server sends an auth URL; user opens it and copies the code back.
- **API key**: User pastes an API key in the web UI.

The provider auth flow is driven by WebSocket messages (`provider_login`, `provider_auth_url`,
`provider_auth_completed`). No CLI interaction is required after the web session is open.

## Session list behavior

- Session list is synced from server-side session snapshot.
- Server list is preferred over local recent cache when available.
- Clicking a session in the sidebar switches the active session.

## Shared session with TUI

The web channel can share a session ID with the TUI so that both interfaces operate on
the same conversation:

```bash
# bind web and TUI to the same session
./target/release/openpista web setup --shared-session-id "my-shared-session"
```

When shared, the sidebar shows `Shared with TUI: yes`.

## Stop / restart operations

```bash
# stop current web daemon using pid file
./target/release/openpista web stop

# restart (stop then start)
./target/release/openpista web restart
```

Notes:

- `web stop` uses the PID file at `~/.openpista/openpista.pid`.
- If the pid file is stale, it is cleaned automatically.
- On Unix, stop sends `SIGTERM` to the daemon process.

## REST endpoints

| Path | Method | Description |
|------|--------|-------------|
| `/` | GET | Serves the web UI (`index.html`) |
| `/health` | GET | Returns `{"status":"ok"}` when the server is up |
| `/auth` | GET/POST | OAuth callback endpoint for redirect-based provider auth flows |
| `/s/{session_id}` | GET | Deep-link to a specific session; opens the web UI with that session pre-selected |
| `/ws` | GET (Upgrade) | WebSocket endpoint for chat and control messages |

## WebSocket message reference

All messages are JSON with a `type` field. Client → server and server → client messages share
the same enum.

| `type` | Direction | Description |
|--------|-----------|-------------|
| `auth` | C→S | Send token to authenticate |
| `auth_result` | S→C | Authentication result (`success`, `client_id`, `provider`, `model`, `session_id`) |
| `message` | C→S | Send a chat message to the agent |
| `response` | S→C | Agent response chunk or final reply |
| `message_ack` | S→C | Message accepted; returns assigned `message_id` |
| `message_error` | S→C | Message processing failed |
| `sessions_request` | C→S | Request the list of sessions |
| `sessions_list` | S→C | Server sends session list |
| `session_history_request` | C→S | Request history for a session |
| `session_history` | S→C | Server sends session message history |
| `model_list_request` | C→S | Request available models |
| `model_list` | S→C | Server sends model list grouped by provider |
| `model_change` | C→S | Request a model change (`provider`, `model`) |
| `model_changed` | S→C | Confirms new active model |
| `provider_auth_request` | C→S | Request provider auth status |
| `provider_auth_status` | S→C | Auth status list per provider |
| `provider_login` | C→S | Initiate provider login (`api_key`, `endpoint`, or `auth_code`) |
| `provider_auth_url` | S→C | OAuth URL to open (`flow_type`: `redirect` or `code_display`) |
| `provider_auth_completed` | S→C | OAuth or key setup result |
| `cancel_generation` | C→S | Cancel current agent processing |
| `generation_cancelled` | S→C | Confirms cancellation |
| `ping` | C→S | Keepalive ping |
| `pong` | S→C | Keepalive pong |

## Runtime path

- Web UI runs on the JS client path (`app.js`) only.
- WASM loader path is not used in the static web entrypoint.

## Troubleshooting

- `Address already in use`: another process is already listening on the same port.
- `health` is error in status: verify daemon is running and the configured port is reachable.
- Login modal keeps showing: verify token value in `~/.openpista/config.toml` (`channels.web.token`).
- UI looks old after update: run `web setup` again to reinstall static files.
- `web stop` does nothing: check that `~/.openpista/openpista.pid` exists; if missing, kill the process manually.
