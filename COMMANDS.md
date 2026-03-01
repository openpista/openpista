# Commands Reference

CLI and TUI commands for `openpista`, organized by category.

## CLI Commands

### Authentication (Recommended — Run First)

OAuth PKCE browser login is the recommended authentication method. Works with OpenAI, Anthropic, and OpenRouter without a separate API key.

```bash
# Interactive provider picker (recommended)
openpista auth login

# Non-interactive with explicit credentials
openpista auth login --non-interactive --provider openai --api-key "$OPENAI_API_KEY"
openpista auth login --non-interactive \
  --provider azure-openai \
  --endpoint "https://your-resource.openai.azure.com" \
  --api-key "$AZURE_OPENAI_API_KEY"

# Logout / status
openpista auth logout --provider openai
openpista auth status
```

> **Credential resolution order:** config file / `openpista_API_KEY` → credential store (`auth login`) → provider env vars → `OPENAI_API_KEY` fallback
> For most users, `openpista auth login` is all you need.

### Basic Usage

```bash
# Launch TUI (default)
openpista
openpista tui
openpista -s SESSION_ID
openpista tui -s SESSION_ID

# Run as daemon (all enabled channels)
openpista start

# Run a single prompt and exit
openpista run -e "your prompt here"
```

### Web Channel

```bash
# Save web config + install static assets (crates/channels/static → static_dir)
openpista web setup --enable --port 3210

# Token management
openpista web setup --regenerate-token
openpista web setup --yes                        # auto-confirm regeneration prompt
openpista web setup --token "manual-token"

# Start web-only daemon
openpista web start

# Show config + runtime state (pid / health)
openpista web status
```

`web setup` token behavior:
- First run with no token: token is auto-generated.
- Existing token in an interactive terminal: prompt asks whether to regenerate.
- Non-interactive mode: existing token is preserved unless `--regenerate-token` is passed.

### Telegram Channel

```bash
# Validate and save bot token
openpista telegram setup --token 123456:ABC...

# Start Telegram bot server
openpista telegram start
openpista telegram start --token 123456:ABC...   # override token for this session only

# Show configuration status
openpista telegram status
```

### WhatsApp Channel

```bash
# Initiate QR pairing flow (same as /whatsapp in TUI)
openpista whatsapp
openpista whatsapp setup

# Start WhatsApp bridge in foreground mode
openpista whatsapp start

# Show connection status
openpista whatsapp status

# Send a message directly
openpista whatsapp send 821012345678 "Hello from openpista"
```

### Model Catalog

```bash
# List available models
openpista model list

# Test a specific model
openpista model -m "hello" gpt-4o

# Open interactive model browser (same as /model in TUI)
openpista model
```

### Global Flags

```bash
# Use a custom config file
openpista --config /path/to/config.toml

# Set log level (trace, debug, info, warn, error)
openpista --log-level debug

# Write debug log to ~/.openpista/debug.log
openpista --debug
```

---

## TUI Commands

### Slash Commands

Press `Tab` in the input area to open the autocomplete palette. Navigate with arrow keys and press `Enter` to select.

```
/help                    Show available TUI commands
/login                   Open provider login / credentials browser
/connection              Same as /login (alias)
/model                   Open full-screen model browser
/model list              Print available models to chat
/session                 Open full-screen session browser
/session new             Start a new session
/session load <id>       Load a session by ID
/session delete <id>     Delete a session by ID
/clear                   Clear conversation history
/quit                    Exit TUI
/exit                    Exit TUI (same as /quit)
/web                     Show web adapter status
/web setup               Configure web adapter (interactive wizard)
/whatsapp                Start WhatsApp QR pairing flow
/whatsapp setup          Same as /whatsapp
/whatsapp status         Show WhatsApp configuration status
/telegram                Show Telegram setup guide
/telegram setup          Same as /telegram
/telegram status         Show Telegram configuration status
/telegram start          Show info for starting the Telegram adapter
/qr                      Show QR code for the Web UI URL
```

---

## Sidebar Keybinds

```
Tab              Toggle sidebar open/closed
j or ↓           Move to next session
k or ↑           Move to previous session
Enter            Load selected session
d or Delete      Request deletion of selected session (shows confirmation)
Esc              Unfocus sidebar
```

---

## Session Browser Keybinds

Full-screen session browser opened with `/session`.

```
Text input       Filter sessions by title
j or ↓           Move to next session
k or ↑           Move to previous session
Enter            Load selected session
n                Create a new session
d or Delete      Request deletion of selected session
Esc              Close session browser
```

### Delete Confirmation Dialog

```
y or Enter       Confirm deletion
n or Esc         Cancel
```

---

## Login Browser Keybinds

```
↑/↓, j/k         Navigate
Enter             Select / proceed to next step
Text input        Enter search query or credentials
Backspace         Delete input character
Esc               Go back or exit
```

---

## Model Browser Keybinds

Full-screen model browser opened with `/model`.

```
s or /           Enter search mode
Text input       Filter models by ID (partial match)
Backspace        Delete search character
j/k or ↑/↓      Navigate model list
PgUp/PgDn        Page up / page down
Enter            Apply selected model to current session
r                Force refresh model list
Esc              Exit search mode (or close browser in normal mode)
```

---

## Chat Keybinds

```
Enter                    Send message
Shift+Enter              Insert newline (multi-line input)
↑/↓ or scroll            Scroll conversation history
Mouse drag               Select text
Ctrl+C or Cmd+C          Copy selected text
Tab                      Open slash command autocomplete palette
```
