# COMMANDS

Frequently used commands in `openpista` categorized by `CLI` and `TUI`

## CLI Commands

### Basic Execution

```bash
# Run TUI (Default)
openpista
openpista tui

# Run Daemon
openpista start

# Execute Single Request
openpista run -e "your prompt"
```

### Model Catalog

```bash
# Default: List recommended coding models
openpista models list
```

### Authentication (Auth)

```bash
# Interactive provider picker login
openpista auth login

# Non-interactive login (Script/CI)
openpista auth login --non-interactive --provider opencode --api-key "$OPENCODE_API_KEY"

# Example of endpoint + key provider
openpista auth login --non-interactive \
  --provider azure-openai \
  --endpoint "https://your-resource.openai.azure.com" \
  --api-key "$AZURE_OPENAI_API_KEY"

# Logout
openpista auth logout --provider openai

# Check saved authentication status
openpista auth status
```

## TUI Commands

### Slash Commands

```txt
/help                    - Show available TUI commands
/login                   - Open provider picker for authentication
/connection              - Same as /login (alias)
/models                  - Open model browser (Recommended on top, All Models at bottom)
/clear                   - Clear conversation history
/quit                    - Exit TUI
/exit                    - Exit TUI (alias for /quit)
```

### Login Browser Keybinds

```txt
↑/↓, j/k: Move
Enter: Select/Next step
Type: Search or input
Backspace: Delete input
Esc: Previous step or exit
```

### Model Browser Keybinds

```txt
s or /: Enter search mode
Type/Backspace: Search for model id partial match
j/k, ↑/↓: Move
PgUp/PgDn: Move page
Enter: Apply selected model as the current session model
r: Force Refresh
Esc: (In search mode) Exit search, (In normal mode) Exit browser
```
