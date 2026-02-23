# WhatsApp Channel Setup Guide

> Inspired by [OpenClaw](https://github.com/openclaw/openclaw)'s single-token gateway model.

---

## Overview

openpista connects to WhatsApp using an **access-token-based** authentication model.
You need only two things:

| Field | Description | Example |
|---|---|---|
| `phone_number` | Your WhatsApp Business phone number (country code + number) | `15551234567` |
| `access_token` | Meta Graph API access token | `EAAxxxxxxxx...` |

---

## Step 1: Create a Meta Developer Account

1. Go to [developers.facebook.com](https://developers.facebook.com)
2. Log in with your Facebook account
3. Click **"Get Started"** and complete the developer registration

---

## Step 2: Create an App

1. Go to [My Apps](https://developers.facebook.com/apps/) and click **"Create App"**
2. Select app type: **"Business"**
3. Enter your app name and click **"Create"**

---

## Step 3: Add WhatsApp Product

1. In the App Dashboard, click **"Add Product"**
2. Find **WhatsApp** and click **"Set Up"**
3. Navigate to **WhatsApp > API Setup** in the left menu

---

## Step 4: Get Your Access Token

### Temporary Token (24-hour, for testing)

On the **API Setup** page, you will see a **Temporary Access Token** displayed.
Copy it — this token is valid for **24 hours**.

```
EAAxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx...
```

### Permanent Token (recommended for production)

Temporary tokens expire after 24 hours. For a permanent token:

1. Go to [Meta Business Suite](https://business.facebook.com/settings/)
2. Navigate to **Settings > Business Settings > System Users**
3. Click **"Add"** and create a System User (Admin role)
4. Click **"Generate Token"**
5. Select your app
6. Check the permission: **`whatsapp_business_messaging`**
7. Click **"Generate Token"** and copy it

This token **does not expire**.

### Token Comparison

| Type | Validity | Where to Get |
|---|---|---|
| Temporary | 24 hours | developers.facebook.com > API Setup |
| Permanent (System User) | Never expires | business.facebook.com > System Users |

---

## Step 5: Get Your Phone Number

On the **API Setup** page, you will also see your **WhatsApp Business phone number**.
Use the full number with country code, no spaces or dashes.

```
Example: 15551234567  (US: 1 + 555-123-4567)
```

If you don't have a phone number yet, Meta provides a **test phone number** on the API Setup page.

---

## Step 6: Configure openpista

### Option A: TUI Wizard (recommended)

Launch openpista and use the interactive setup wizard:

```bash
openpista start
```

Then type:

```
/whatsapp
```

The wizard guides you through 3 steps:

| Step | Prompt | What to Enter |
|---|---|---|
| 1 | Phone Number | Your WhatsApp number (e.g. `15551234567`) |
| 2 | Access Token | Your Meta access token (e.g. `EAA...`) |
| 3 | Confirm | Review and press Enter to save |

After setup, a **QR code** is displayed that encodes your `wa.me/{phone}` link.
Scan it to start a conversation.

### Option B: Config File

Edit `~/.openpista/config.toml`:

```toml
[channels.whatsapp]
enabled = true
phone_number = "15551234567"
access_token = "EAAxxxxxxxxxxxxxxxx"
webhook_port = 8443
```

### Option C: Environment Variables

```bash
WHATSAPP_PHONE_NUMBER=15551234567 \
WHATSAPP_ACCESS_TOKEN=EAAxxxxxxxx... \
openpista start
```

---

## Verifying Your Setup

After configuration, check the status:

```
/whatsapp status
```

This shows:
- Whether WhatsApp is enabled
- Phone number (displayed in full)
- Access token (masked for security)
- Webhook port
- A **QR code** if fully configured (scan with your phone)

---

## TUI Commands Reference

| Command | Description |
|---|---|
| `/whatsapp` | Open the interactive setup wizard |
| `/whatsapp setup` | Same as `/whatsapp` |
| `/whatsapp status` | Show current configuration and QR code |
| `/qr` | Display QR code overlay |

---

## Webhook Setup (Advanced)

openpista starts a webhook server on the configured port (default: `8443`).
For Meta to deliver messages, your webhook must be publicly accessible.

### Using ngrok (development)

```bash
# Terminal 1: start openpista
openpista start

# Terminal 2: expose webhook
ngrok http 8443
```

Then configure the webhook URL in Meta's App Dashboard:
- **Callback URL**: `https://your-ngrok-url.ngrok.io/webhook`
- No verify token is needed (Bearer token auth is used instead)

### Using a reverse proxy (production)

```nginx
# nginx example
server {
    listen 443 ssl;
    server_name whatsapp.yourdomain.com;

    location /webhook {
        proxy_pass http://127.0.0.1:8443;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

---

## Troubleshooting

| Problem | Solution |
|---|---|
| Token expired | Temporary tokens last 24h. Generate a permanent System User token |
| Messages not arriving | Check webhook URL is publicly accessible and port matches config |
| `401 Unauthorized` | Verify your access token is correct and has `whatsapp_business_messaging` permission |
| QR code not showing | Ensure both `phone_number` and `access_token` are set |
| `/whatsapp` not responding | Make sure you're in the TUI (`openpista start`) |

---

## Security Notes

- **Never commit** your access token to version control
- Use environment variables or `config.toml` (which is gitignored)
- Permanent tokens have full API access — treat them like passwords
- The webhook validates incoming requests using Bearer token authentication

---

## Further Reading

- [Meta WhatsApp Business Platform](https://developers.facebook.com/docs/whatsapp/)
- [WhatsApp Cloud API Getting Started](https://developers.facebook.com/docs/whatsapp/cloud-api/get-started)
- [System User Tokens](https://developers.facebook.com/docs/marketing-api/system-users/)
- [OpenClaw](https://github.com/openclaw/openclaw) — inspiration for the single-token auth model