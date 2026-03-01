# Channel Guides

openpista supports multiple messaging channels. Each channel connects to the same agent and session backend.

## Available Channels

| Channel | Guide | Default | External Dependency | Auth |
|---|---|---|---|---|
| CLI / TUI | [cli.md](./cli.md) | Enabled | None | None |
| Web | [web.md](./web.md) | Disabled | None | Token (optional) |
| Telegram | [telegram.md](./telegram.md) | Disabled | None | Bot token |
| WhatsApp | [whatsapp.md](./whatsapp.md) | Disabled | Node.js 18+ | QR scan |

## Quick Comparison

| Channel | Best For | Setup Complexity |
|---|---|---|
| CLI / TUI | Local development, direct terminal access | None |
| Web | Browser access, sharing with other devices | Low |
| Telegram | Remote access via mobile/desktop Telegram | Low |
| WhatsApp | Remote access via WhatsApp | Medium |

## Getting Started

1. **CLI** — just run `openpista`. No setup needed.
2. **Web** — run `openpista web setup --enable && openpista web start`
3. **Telegram** — run `openpista telegram setup --token YOUR_TOKEN && openpista telegram start`
4. **WhatsApp** — install Node.js, enable in config, then scan QR via `/whatsapp` in TUI
