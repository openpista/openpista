# CLI / TUI Usage Guide

---

## Overview

The CLI channel is the **default channel** — it is always enabled. When you run `openpista` without subcommands, it opens a full-screen terminal UI (TUI) built with [ratatui](https://github.com/ratatui-org/ratatui) + crossterm.

The TUI provides a ChatGPT-style interface directly in your terminal, sharing the same session and agent as all other channels.

---

## Prerequisites

| Requirement | Description |
|---|---|
| openpista binary | Built from source (`cargo build --release`) or installed |
| Terminal | Any terminal emulator with UTF-8 support |

No additional setup is required — the CLI channel is enabled by default.

---

## Quick Start

```bash
# Open TUI (default channel, always available)
openpista

# Resume a specific session by ID
openpista -s SESSION_ID

# Run a single prompt and exit (non-interactive)
openpista run -e "list files in current directory"

# Start as background daemon (all channels)
openpista start
```

---

## Configuration Reference

### Config File (`~/.openpista/config.toml`)

```toml
[channels.cli]
# Whether the CLI/TUI adapter is enabled.
# Default: true (always on unless explicitly disabled)
enabled = true
```

The CLI channel has minimal configuration — it is controlled primarily by CLI flags and TUI commands.

---

## TUI Layout

```
┌─────────────────────────────────────────────────────┐
│ Sidebar          │ Chat Pane                         │
│                  │                                   │
│ Sessions         │ [conversation history]            │
│  • session-1     │                                   │
│  • session-2     │                                   │
│                  │                                   │
│ Runtime Metadata │                                   │
│  Status: idle    ├───────────────────────────────────┤
│  Model: ...      │ > Input area                      │
│  Session: ...    │                                   │
└─────────────────────────────────────────────────────┘
│ Status bar: model · provider · session              │
└──────────────────────────────────────────────────────┘
```

- **Left sidebar**: Session list + runtime metadata (status, model, session ID, web sharing status)
- **Main pane**: Conversation history with user messages, assistant replies, and tool call output
- **Bottom composer**: Multi-line input area with slash command autocomplete
- **Status bar**: Current model, provider, and session information

---

## Slash Commands

Type `/` in the input area and press `Tab` to open the autocomplete palette.

| Command | Description |
|---|---|
| `/help` | Show available commands |
| `/model` | Open the full-screen model browser |
| `/model list` | Print available models to chat |
| `/login` | Open the provider login/credentials browser |
| `/connection` | Same as `/login` (alias) |
| `/clear` | Clear conversation history for the current session |
| `/session` | Open the full-screen session browser |
| `/session new` | Start a new session |
| `/session load <id>` | Load a session by ID |
| `/session delete <id>` | Delete a session by ID |
| `/web` | Show web adapter status |
| `/web setup` | Configure web adapter (interactive wizard) |
| `/whatsapp` | Start WhatsApp QR pairing flow |
| `/whatsapp setup` | Same as `/whatsapp` |
| `/whatsapp status` | Show WhatsApp configuration status |
| `/telegram` | Show Telegram setup guide |
| `/telegram setup` | Same as `/telegram` |
| `/telegram status` | Show Telegram configuration status |
| `/telegram start` | Show info for starting the Telegram adapter |
| `/qr` | Show QR code for the Web UI URL |

---

## Keyboard Shortcuts

### Chat Pane

| Key | Action |
|---|---|
| `Enter` | Send message |
| `Shift+Enter` | Insert newline (multi-line input) |
| `↑` / `↓` or scroll | Scroll conversation history |
| `Tab` | Open slash command autocomplete palette |
| Mouse drag | Select text |
| `Ctrl+C` / `Cmd+C` | Copy selected text |

### Sidebar

| Key | Action |
|---|---|
| `Tab` | Toggle sidebar open/closed |
| `j` or `↓` | Move to next session |
| `k` or `↑` | Move to previous session |
| `Enter` | Load selected session |
| `d` or `Delete` | Delete selected session (shows confirmation) |
| `Esc` | Unfocus sidebar |

### Session Browser (`/session`)

| Key | Action |
|---|---|
| Text input | Filter sessions by title |
| `j` or `↓` | Move to next session |
| `k` or `↑` | Move to previous session |
| `Enter` | Load selected session |
| `n` | Create a new session |
| `d` or `Delete` | Delete selected session |
| `Esc` | Close session browser |

#### Delete Confirmation Dialog

| Key | Action |
|---|---|
| `y` or `Enter` | Confirm deletion |
| `n` or `Esc` | Cancel |

### Model Browser (`/model`)

| Key | Action |
|---|---|
| `s` or `/` | Enter search mode |
| Text input | Filter models by ID (partial match) |
| `Backspace` | Delete search character |
| `j` / `k` or `↑` / `↓` | Navigate model list |
| `PgUp` / `PgDn` | Page up / page down |
| `Enter` | Apply selected model to current session |
| `r` | Force refresh model list |
| `Esc` | Exit search mode (or close browser in normal mode) |

### Login Browser (`/login`)

| Key | Action |
|---|---|
| `↑` / `↓`, `j` / `k` | Navigate |
| `Enter` | Select / proceed to next step |
| Text input | Enter search query or credentials |
| `Backspace` | Delete input character |
| `Esc` | Go back or exit |

---

## Daemon Mode

Run all channels (including CLI, Telegram, Web, WhatsApp) as a background daemon:

```bash
# Start daemon
openpista start

# Check daemon status
openpista status

# Stop daemon
openpista stop
```

The daemon uses a PID file at `~/.openpista/openpista.pid`. On Unix systems, `stop` sends `SIGTERM` to the daemon process.

> **Note:** When running as a daemon, the TUI is not available. Use `openpista` (foreground) for TUI access, or the web channel for browser-based access.

---

## Troubleshooting

| Problem | Solution |
|---|---|
| No model configured / empty responses | Run `openpista model select` or set `[agent] model` in config |
| TUI does not display correctly | Ensure terminal supports UTF-8; try resizing the terminal window |
| TUI appears corrupted after resize | The layout redraws automatically — resize to trigger a full redraw |
| Database locked error | Only one instance of openpista can run at a time; stop other instances |
| `openpista start` daemon not responding | Check `~/.openpista/openpista.pid`; if stale, delete it and restart |
| Slash commands not autocompleting | Press `Tab` with at least `/` typed in the input area |
