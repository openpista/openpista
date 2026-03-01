# WhatsApp Channel Setup Guide
> Inspired by [OpenClaw](https://github.com/nicepkg/openclaw)'s WhatsApp Web multi-device protocol approach.

---

## Overview

openpista connects to WhatsApp using the **WhatsApp Web multi-device protocol** — the same protocol that powers WhatsApp Web/Desktop. The application acts as a linked device on your WhatsApp account.

**No Meta Business API, no developer account, no access tokens required.**
| What you need | Description |
|---|---|
| WhatsApp account | A personal WhatsApp account on your phone |
| Node.js 18+ | Required to run the Baileys bridge subprocess |
| QR code scan | One-time scan from your phone to pair |

---

## How It Works

```
┌──────────────┐   JSON lines    ┌──────────────────┐   WhatsApp Web   ┌─────────────┐
│  openpista    │ ◄──stdin/stdout──► │  whatsapp-bridge │ ◄───protocol────► │  WhatsApp   │
│  (Rust)       │                │  (Node.js/Baileys)│                 │  Servers    │
└──────────────┘                └──────────────────┘                 └─────────────┘
```

1. `openpista start` spawns a Node.js bridge process (`whatsapp-bridge/index.js`)
2. The bridge connects to WhatsApp servers using the Baileys library
3. A QR code appears in the TUI — scan it with **WhatsApp > Linked Devices > Link a Device**
4. Once paired, messages flow directly through the WhatsApp Web protocol
5. Session credentials are stored locally — you only need to scan once

---

## Step 1: Install Node.js

The WhatsApp bridge requires Node.js 18 or later.

### macOS

```bash
brew install node
```

### Ubuntu / Debian

```bash
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs
```

### Verify

```bash
node --version   # v18.0.0 or later
```

---

## Step 2: Install Bridge Dependencies

From the openpista project root:

```bash
cd whatsapp-bridge
npm install
```

This installs [@whiskeysockets/baileys](https://github.com/WhiskeySockets/Baileys) and its dependencies.

---

## Step 3: Enable WhatsApp in Config

Edit `~/.openpista/config.toml`:

```toml
[channels.whatsapp]
enabled = true
# session_dir = "~/.openpista/whatsapp"     # Where pairing credentials are stored
# bridge_path = "whatsapp-bridge/index.js"  # Path to bridge script (auto-detected)
```

---

## Step 4: Pair via QR Code

1. Launch openpista:

```bash
openpista start
```

2. In the TUI, type:

```
/whatsapp
```

3. A QR code will appear in the terminal. On your phone:
   - Open **WhatsApp**
   - Go to **Settings > Linked Devices**
   - Tap **Link a Device**
   - Scan the QR code shown in the TUI

4. Once paired, the TUI shows a success message with your phone number and display name.

---

## Step 5: Verify

Check the connection status:

```
/whatsapp status
```

This shows:
- Whether WhatsApp is enabled
- Session directory path
- Bridge script path
- Pairing status (paired / not paired)

---

## TUI Commands
| Command | Description |
|---|---|
| `/whatsapp` | Start the QR pairing flow |
| `/whatsapp setup` | Same as `/whatsapp` |
| `/whatsapp status` | Show connection status |

---

## Configuration Reference

### Config File (`config.toml`)

```toml
[channels.whatsapp]
enabled = true
# Directory for WhatsApp session credentials (auth keys, etc.)
# Default: ~/.openpista/whatsapp
session_dir = "~/.openpista/whatsapp"

# Path to the Node.js bridge script
# Default: auto-detected (whatsapp-bridge/index.js relative to binary)
# bridge_path = "/path/to/custom/bridge.js"
```

### Session Persistence

After the first QR scan, credentials are stored in `session_dir/auth/`. The session persists across restarts — you do **not** need to scan again unless:

- You manually delete the session directory
- You unlink the device from WhatsApp on your phone
- The session expires (WhatsApp may expire inactive linked devices after ~14 days)

---

## Troubleshooting
| Problem | Solution |
|---|---|
| QR code not appearing | Ensure Node.js 18+ is installed and `npm install` was run in `whatsapp-bridge/` |
| `Error: Cannot find module` | Run `cd whatsapp-bridge && npm install` |
| QR code keeps refreshing | QR codes expire after ~60 seconds. Scan quickly, or wait for the next one |
| Session expired | Delete `~/.openpista/whatsapp/auth/` and re-pair with `/whatsapp` |
| `failed to spawn bridge` | Check that `node` is on your PATH |
| Messages not arriving | Ensure your phone has an active internet connection (required for multi-device) |

---

## Security Notes

- Session credentials in `~/.openpista/whatsapp/auth/` grant full access to your WhatsApp account — protect this directory
- Add `~/.openpista/whatsapp/` to your backup exclusions if using cloud backups
- The bridge communicates with WhatsApp servers using end-to-end encryption (Signal protocol)
- No data passes through openpista servers — all communication is direct

---

## Further Reading
- [Baileys (WhiskeySockets)](https://github.com/WhiskeySockets/Baileys) — WhatsApp Web API library
- [WhatsApp Multi-Device](https://faq.whatsapp.com/general/download-and-installation/about-linked-devices/) — Official FAQ
- [OpenClaw](https://github.com/nicepkg/openclaw) — Inspiration for the WhatsApp Web protocol approach
