# Telegram Channel Setup Guide

---

## Overview

openpista connects to Telegram using the **Bot API polling** model — no webhook server or public IP address required. The bot runs as a long-polling client, pulling messages from Telegram's servers at regular intervals.

**No external infrastructure needed.**

---

## Prerequisites

| Requirement | Description |
|---|---|
| Telegram account | A personal Telegram account on your phone or desktop |
| @BotFather | Telegram's official bot management account |
| Bot token | Obtained from @BotFather (format: `123456789:AABBcc...`) |
| LLM model | At least one provider configured (`openpista model select`) |

---

## Quick Start

```bash
# 1. Configure and verify your bot token
openpista telegram setup --token 123456789:AABBcc...

# 2. Start the Telegram bot server
openpista telegram start
```

---

## Step-by-Step Setup

### Step 1: Create a Bot with @BotFather

1. Open Telegram and search for **@BotFather**
2. Start a chat and send `/newbot`
3. Enter a display name for your bot (e.g. `My Pista Bot`)
4. Enter a username ending in `_bot` (e.g. `mypista_bot`)
5. BotFather will reply with your token:
   ```
   123456789:AABBccDDeeFFggHH...
   ```

### Step 2: Configure the Token

Choose one of three methods:

**Method A — CLI setup (recommended)**
```bash
openpista telegram setup --token 123456789:AABBcc...
```
This validates the token against the Telegram API and saves it to your config file.

**Method B — Config file**

Edit `~/.openpista/config.toml`:
```toml
[channels.telegram]
enabled = true
token = "123456789:AABBcc..."
```

**Method C — Environment variable**
```bash
TELEGRAM_BOT_TOKEN=123456789:AABBcc... openpista telegram start
```

### Step 3: Select a Model

Telegram needs an LLM to respond to messages:
```bash
openpista model select
```

Or set it directly in `~/.openpista/config.toml`:
```toml
[agent]
provider = "anthropic"
model = "claude-sonnet-4-6"
```

### Step 4: Start the Bot

```bash
openpista telegram start
```

The output shows the active provider and model, then waits for messages:
```
Telegram Bot Server
===================

Provider: anthropic
Model   : claude-sonnet-4-6

Starting agent runtime...
Bot is running. Press Ctrl+C to stop.
```

### Step 5: Verify

Send a message to your bot in Telegram. It should reply within a few seconds.

Check status from the CLI:
```bash
openpista telegram status
```

---

## Configuration Reference

### Config File (`~/.openpista/config.toml`)

```toml
[channels.telegram]
# Whether the Telegram adapter is enabled.
enabled = true

# Bot token from @BotFather. Required to start the bot.
# Format: NUMBERS:STRING (e.g. 123456789:AABBcc...)
token = "123456789:AABBcc..."
```

### Environment Variables

| Variable | Description | Example |
|---|---|---|
| `TELEGRAM_BOT_TOKEN` | Bot token (overrides config file) | `123456789:AABBcc...` |

**Priority order:** `--token` flag → `TELEGRAM_BOT_TOKEN` env var → config file value

---

## CLI Commands

| Command | Description |
|---|---|
| `openpista telegram setup --token TOKEN` | Validate and save bot token to config |
| `openpista telegram start` | Start the Telegram bot server |
| `openpista telegram start --token TOKEN` | Start with a specific token (overrides config) |
| `openpista telegram status` | Show current Telegram configuration status |

---

## TUI Commands

From within the TUI (`openpista`):

| Command | Description |
|---|---|
| `/telegram` | Show Telegram setup guide |
| `/telegram setup` | Same as `/telegram` |
| `/telegram status` | Show Telegram configuration status |
| `/telegram start` | Show info for starting the Telegram adapter |

---

## Troubleshooting

| Problem | Solution |
|---|---|
| `Invalid token format` | Token must match `NUMBERS:STRING` — copy it exactly from BotFather |
| `Token verification failed (HTTP 401)` | Token is wrong or was revoked. Create a new one via `/revoke` in @BotFather |
| Bot does not respond to messages | Ensure `openpista telegram start` is running and the model is configured |
| `No model configured` | Run `openpista model select` or set `[agent] model` in config |
| `⚠ No model configured` prompt at start | Enter `y` to continue anyway, or configure a model first |
| Multiple messages received | Long-polling deduplication is automatic; restart if duplicates persist |
| Bot responds slowly | Normal for large LLM models — consider a faster model |

---

## Security Notes

- **Bot tokens grant full access** to your bot's messages — never commit them to version control
- Use environment variables or a secrets manager for production deployments
- Telegram bots can only receive messages sent directly to them or in groups where they are added
- Messages are processed locally — no data passes through openpista's servers
