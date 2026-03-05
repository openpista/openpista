//! Slash command parsing and dispatch for the TUI event loop.
#![allow(dead_code, unused_imports)]

use super::helpers::{
    ModelsCommand, SessionCommand, TelegramCommand, WebCommand, WhatsAppCommand,
    format_telegram_setup_guide, format_telegram_status,
};
use crate::config::Config;
use crate::tui::action::Action;
use crate::tui::app::TuiApp;

/// Parses a raw `/model` input into a `ModelsCommand` variant.
pub(super) fn parse_models_command(raw: &str) -> Option<ModelsCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/model" {
        return None;
    }

    match parts.next() {
        None => Some(ModelsCommand::Browse),
        Some("list") => Some(ModelsCommand::List),
        Some(_) => Some(ModelsCommand::Invalid(
            "Use /model to browse or /model list to print models.".to_string(),
        )),
    }
}

pub(super) fn parse_session_command(raw: &str) -> Option<SessionCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/session" {
        return None;
    }
    match parts.next() {
        None | Some("list") => Some(SessionCommand::List),
        Some("new") => Some(SessionCommand::New),
        Some("load") => {
            let id = parts.collect::<Vec<_>>().join(" ");
            if id.is_empty() {
                Some(SessionCommand::Invalid(
                    "Usage: /session load <id>".to_string(),
                ))
            } else {
                Some(SessionCommand::Load(id))
            }
        }
        Some("delete") | Some("del") => {
            let id = parts.collect::<Vec<_>>().join(" ");
            if id.is_empty() {
                Some(SessionCommand::Invalid(
                    "Usage: /session delete <id>".to_string(),
                ))
            } else {
                Some(SessionCommand::Delete(id))
            }
        }
        Some(_) => Some(SessionCommand::Invalid(
            "Use /session, /session list, /session new, /session load <id>, /session delete <id>"
                .to_string(),
        )),
    }
}

pub(super) fn parse_web_command(raw: &str) -> Option<WebCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/web" {
        return None;
    }
    match parts.next() {
        None | Some("status") => Some(WebCommand::Status),
        Some("setup") => Some(WebCommand::Setup),
        Some(_) => Some(WebCommand::Invalid(
            "Use /web to show status or /web setup to configure.".to_string(),
        )),
    }
}

pub(super) fn parse_whatsapp_command(raw: &str) -> Option<WhatsAppCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/whatsapp" {
        return None;
    }
    match parts.next() {
        None | Some("setup") => Some(WhatsAppCommand::Setup),
        Some("status") => Some(WhatsAppCommand::Status),
        Some(_) => Some(WhatsAppCommand::Invalid(
            "Usage: /whatsapp [setup|status]".to_string(),
        )),
    }
}

pub(super) fn parse_telegram_command(raw: &str) -> Option<TelegramCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/telegram" {
        return None;
    }
    match parts.next() {
        None | Some("setup") => Some(TelegramCommand::Setup),
        Some("start") => Some(TelegramCommand::Start),
        Some("status") => Some(TelegramCommand::Status),
        Some(_) => Some(TelegramCommand::Invalid(
            "Usage: /telegram [setup|start|status]".to_string(),
        )),
    }
}

pub(super) fn handle_telegram_command(app: &mut TuiApp, config: &Config, message: &str) -> bool {
    let Some(tg_cmd) = parse_telegram_command(message) else {
        return false;
    };

    match tg_cmd {
        TelegramCommand::Setup => {
            let guide = format_telegram_setup_guide();
            app.update(Action::PushAssistantMessage(guide));
        }
        TelegramCommand::Start => {
            let tg = &config.channels.telegram;
            if tg.enabled && !tg.token.is_empty() {
                app.update(Action::PushAssistantMessage(
                    "Telegram adapter is configured. Run `openpista start` to launch the daemon with all enabled channels.".to_string(),
                ));
            } else {
                app.update(Action::PushError(
                    "Telegram is not configured. Run /telegram setup for instructions.".to_string(),
                ));
            }
        }
        TelegramCommand::Status => {
            let status = format_telegram_status(config);
            app.update(Action::PushAssistantMessage(status));
        }
        TelegramCommand::Invalid(msg) => {
            app.update(Action::PushError(msg));
        }
    }
    app.update(Action::ScrollToBottom);
    true
}
