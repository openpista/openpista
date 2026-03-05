//! Utility functions and shared types for the TUI event loop.
#![allow(dead_code, unused_imports)]

use crate::config::Config;
use crate::model_catalog;

/// Detects the local IP address by connecting a UDP socket to a public DNS server.
pub(super) fn detect_local_ip() -> String {
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "localhost".to_string())
}

/// Formats model catalog entries into a human-readable text listing for chat display.
pub(super) fn format_model_list(
    entries: &[model_catalog::ModelCatalogEntry],
    sync_statuses: &[String],
) -> String {
    use model_catalog::ModelSource;
    let recommended: Vec<_> = entries
        .iter()
        .filter(|e| e.recommended_for_coding && e.available)
        .collect();
    let other: Vec<_> = entries
        .iter()
        .filter(|e| !e.recommended_for_coding && e.available)
        .collect();

    let mut out = format!("Models — {} total\n", entries.len());
    if !recommended.is_empty() {
        out.push_str("\nRecommended:\n");
        for e in &recommended {
            let tag = if e.source == ModelSource::Api {
                " (api)"
            } else {
                ""
            };
            out.push_str(&format!("  ★  {} [{}]{}\n", e.id, e.provider, tag));
        }
    }
    if !other.is_empty() {
        out.push_str("\nOther:\n");
        for e in &other {
            let tag = if e.source == ModelSource::Api {
                " (api)"
            } else {
                ""
            };
            out.push_str(&format!("     {} [{}]{}\n", e.id, e.provider, tag));
        }
    }

    if !sync_statuses.is_empty() {
        out.push_str(&format!("\nSync: {}", sync_statuses.join("; ")));
    }
    out
}

/// Collects (provider_name, base_url, api_key) tuples for all authenticated providers.
pub(super) fn collect_authenticated_providers(
    config: &Config,
) -> Vec<(String, Option<String>, String)> {
    use crate::config::ProviderPreset;
    let mut providers = Vec::new();
    for preset in ProviderPreset::all() {
        let name = preset.name();
        if let Some(cred) = config.resolve_credential_for(name) {
            providers.push((name.to_string(), cred.base_url, cred.api_key));
        }
    }
    // Ensure the currently configured provider is always included
    let active = config.agent.provider.name().to_string();
    if !providers.iter().any(|(n, _, _)| n == &active) {
        let key = config.resolve_api_key();
        if !key.is_empty() {
            providers.push((
                active,
                config.agent.effective_base_url().map(String::from),
                key,
            ));
        }
    }
    providers
}

const MODEL_SYNC_IN_PROGRESS_MESSAGE: &str = "Model sync is already in progress. Please wait.";

pub(super) fn model_sync_in_progress_error() -> String {
    MODEL_SYNC_IN_PROGRESS_MESSAGE.to_string()
}

/// Parsed sub-command for the `/model` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ModelsCommand {
    /// Open the interactive model browser.
    Browse,
    /// Print model list to chat.
    List,
    /// Unrecognised sub-command with an error message.
    Invalid(String),
}

/// Parsed sub-command for the `/session` slash command.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SessionCommand {
    /// `/session` or `/session list` — print all sessions to chat.
    List,
    /// `/session new` — create a new session.
    New,
    /// `/session load <id>` — load a specific session (partial ID match).
    Load(String),
    /// `/session delete <id>` — delete a specific session (partial ID match).
    Delete(String),
    /// Invalid usage with hint message.
    Invalid(String),
}

/// Parsed sub-command for the `/web` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WebCommand {
    /// `/web` or `/web status` — display current web config.
    Status,
    /// `/web setup` — launch interactive configuration wizard.
    Setup,
    /// Unrecognised sub-command.
    Invalid(String),
}

/// Parsed sub-command for the `/whatsapp` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WhatsAppCommand {
    /// `/whatsapp` — open the interactive setup wizard.
    Setup,
    /// `/whatsapp status` — show current WhatsApp configuration.
    Status,
    /// Invalid sub-command.
    Invalid(String),
}

/// Parsed sub-command for the `/telegram` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TelegramCommand {
    /// `/telegram` or `/telegram setup` — show setup instructions.
    Setup,
    /// `/telegram start` — start the Telegram adapter.
    Start,
    /// `/telegram status` — show current Telegram configuration.
    Status,
    /// Invalid sub-command.
    Invalid(String),
}

pub(super) fn format_whatsapp_status(config: &Config) -> String {
    let wa = &config.channels.whatsapp;
    let mut lines = vec!["WhatsApp Configuration Status".to_string(), "".to_string()];
    lines.push(format!(
        "  Enabled:         {}",
        if wa.enabled { "Yes" } else { "No" }
    ));
    lines.push(format!("  Session Dir:     {}", wa.session_dir));
    lines.push(format!(
        "  Bridge Path:     {}",
        wa.bridge_path.as_deref().unwrap_or("(bundled default)")
    ));
    lines.push("".to_string());
    if wa.is_configured() {
        lines.push("  Status: Ready (session directory set)".to_string());
    } else {
        lines.push("  Status: Incomplete \u{2014} run /whatsapp to configure".to_string());
    }
    lines.join("\n")
}

pub(super) fn format_telegram_status(config: &Config) -> String {
    let tg = &config.channels.telegram;
    let mut lines = vec!["Telegram Configuration Status".to_string(), "".to_string()];
    lines.push(format!(
        "  Enabled:     {}",
        if tg.enabled { "Yes" } else { "No" }
    ));
    lines.push(format!(
        "  Token:       {}",
        if tg.token.is_empty() {
            "(not set)"
        } else {
            "(set)"
        }
    ));
    lines.push("".to_string());
    if tg.enabled && !tg.token.is_empty() {
        lines.push("  Status: Ready".to_string());
    } else if !tg.token.is_empty() {
        lines.push("  Status: Token set but adapter disabled".to_string());
        lines.push(
            "  → Set `enabled = true` in [channels.telegram] or run `openpista start`".to_string(),
        );
    } else {
        lines.push("  Status: Not configured".to_string());
        lines.push("  → Run /telegram setup for instructions".to_string());
    }
    lines.join("\n")
}

pub(super) fn format_telegram_setup_guide() -> String {
    let lines = vec![
        "Telegram Bot Setup Guide".to_string(),
        "".to_string(),
        "  1. Open Telegram and message @BotFather".to_string(),
        "  2. Send /newbot and follow the prompts".to_string(),
        "  3. Copy the bot token (e.g. 123456:ABC-DEF...)".to_string(),
        "  4. Add to config.toml:".to_string(),
        "".to_string(),
        "     [channels.telegram]".to_string(),
        "     enabled = true".to_string(),
        "     token = \"YOUR_BOT_TOKEN\"".to_string(),
        "".to_string(),
        "  Or set the environment variable:".to_string(),
        "     TELEGRAM_BOT_TOKEN=YOUR_BOT_TOKEN".to_string(),
        "".to_string(),
        "  5. Run `openpista start` to launch the daemon with Telegram enabled".to_string(),
    ];
    lines.join("\n")
}

/// Render a QR code as Unicode half-block text lines.
/// Uses `▀`, `▄`, `█` and space to pack two module rows per text line.
pub(crate) fn render_qr_text(url: &str) -> Option<String> {
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes()).ok()?;
    let modules = code.to_colors();
    let width = code.width();
    let height = modules.len() / width;

    // Add 1-module quiet zone on each side
    let mut lines: Vec<String> = Vec::new();

    // Top quiet-zone row (all white)
    lines.push(" ".repeat(width + 2));

    // Process two rows at a time using half-block characters
    let mut y = 0;
    while y < height {
        let mut row = String::new();
        row.push(' '); // left quiet zone
        for x in 0..width {
            let top = modules[y * width + x];
            let bottom = if y + 1 < height {
                modules[(y + 1) * width + x]
            } else {
                qrcode::Color::Light // pad with white if odd height
            };
            match (top, bottom) {
                (qrcode::Color::Dark, qrcode::Color::Dark) => row.push('\u{2588}'),
                (qrcode::Color::Dark, qrcode::Color::Light) => row.push('\u{2580}'),
                (qrcode::Color::Light, qrcode::Color::Dark) => row.push('\u{2584}'),
                (qrcode::Color::Light, qrcode::Color::Light) => row.push(' '),
            }
        }
        row.push(' '); // right quiet zone
        lines.push(row);
        y += 2;
    }

    // Bottom quiet-zone row
    lines.push(" ".repeat(width + 2));

    Some(lines.join("\n"))
}
