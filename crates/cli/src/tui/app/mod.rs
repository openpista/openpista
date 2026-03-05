//! TUI application state, rendering, and input handling.
#![allow(dead_code)]

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
pub use state::*;

use unicode_width::UnicodeWidthStr;

use super::theme::THEME;
use crate::auth_picker::{self, AuthLoginIntent, AuthMethodChoice, LoginBrowseStep};
use crate::config::LoginAuthMode;
use crate::model_catalog;
use proto::{ChannelId, ProgressEvent, SessionId};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::collections::HashSet;

use state::{SLASH_COMMANDS, SPINNER, SlashCommand};

impl TuiApp {
    /// Create a new TUI application state.
    pub fn new(
        model_name: impl Into<String>,
        session_id: SessionId,
        channel_id: ChannelId,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            chat: ChatState {
                messages: Vec::new(),
                input: String::new(),
                cursor_pos: 0,
                history_scroll: 0,
                text_selection: super::selection::TextSelection::new(),
                chat_area: None,
                chat_text_grid: Vec::new(),
                chat_scroll_clamped: 0,
                pending_approval: None,
            },
            session: SessionState {
                session_id,
                channel_id,
                session_list: Vec::new(),
                pending_sidebar_selection: None,
                confirmed_delete: None,
                session_browser_new_requested: false,
            },
            model: ModelState {
                model_name: model_name.into(),
                model_entries: Vec::new(),
                model_provider: "openai".to_string(),
                model_refresh_requested: false,
                pending_model_change: None,
                provider_name: provider_name.into(),
                pending_auth_intent: None,
            },
            sidebar: SidebarState {
                hover: None,
                scroll: 0,
                visible: true,
                focused: false,
            },
            state: AppState::Idle,
            screen: Screen::Home,
            spinner_tick: 0,
            should_quit: false,
            command_palette_cursor: 0,
            workspace_name: "~/openpista".into(),
            branch_name: "main".into(),
            mcp_count: 0,
            version: env!("CARGO_PKG_VERSION").into(),
            pending_web_config: None,
        }
    }

    /// Returns `true` if the configured provider has a valid (non-expired) stored credential.
    pub fn is_authenticated(&self) -> bool {
        let creds = crate::auth::Credentials::load();
        creds
            .get(&self.model.provider_name)
            .is_some_and(|c| !c.is_expired())
    }

    /// Takes the pending model change set by the model browser on selection.
    pub fn take_pending_model_change(&mut self) -> Option<(String, String)> {
        self.model.pending_model_change.take()
    }

    /// Returns the sidebar `Rect` for the given full-frame area, or `None` if the sidebar is hidden.
    pub fn compute_sidebar_area(&self, full_area: Rect) -> Option<Rect> {
        if !self.sidebar.visible || self.screen != Screen::Chat {
            return None;
        }
        let h_chunks = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(crate::tui::sidebar::sidebar_width()),
        ])
        .split(full_area);
        Some(h_chunks[1])
    }

    // ── Command palette ──────────────────────────────────────

    pub(crate) fn is_palette_active(&self) -> bool {
        self.state == AppState::Idle && self.chat.input.starts_with('/')
    }

    fn palette_filtered_commands(&self) -> Vec<&'static SlashCommand> {
        let q = self.chat.input.to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(q.as_str()))
            .collect()
    }

    /// Resolves the palette selection into `self.chat.input` and closes the palette.
    /// Returns the command name so the caller can process it through the normal
    /// command pipeline (e.g. `/models` needs async model loading in event.rs).
    pub fn take_palette_command(&mut self) -> Option<String> {
        if !self.is_palette_active() {
            return None;
        }
        let name = self
            .palette_filtered_commands()
            .get(self.command_palette_cursor)
            .map(|c| c.name.to_string())?;
        self.chat.input = name.clone();
        self.chat.cursor_pos = self.chat.input.len();
        self.command_palette_cursor = 0;
        self.screen = Screen::Chat;
        Some(name)
    }

    // ── State mutations ──────────────────────────────────────

    /// Push a user message to the history.
    pub fn push_user(&mut self, text: String) {
        self.chat.messages.push(TuiMessage::User(text));
    }

    /// Push an assistant response to the history.
    pub fn push_assistant(&mut self, text: String) {
        self.chat.messages.push(TuiMessage::Assistant(text));
    }

    /// Push an error message to the history.
    pub fn push_error(&mut self, err: String) {
        self.chat.messages.push(TuiMessage::Error(err));
    }

    /// Take the current input and reset it.
    pub fn take_input(&mut self) -> String {
        self.chat.cursor_pos = 0;
        std::mem::take(&mut self.chat.input)
    }

    /// Parses and executes a local slash command.
    pub fn handle_slash_command(&mut self, raw: &str) -> bool {
        let trimmed = raw.trim();
        if !trimmed.starts_with('/') {
            return false;
        }

        let mut parts = trimmed.split_whitespace();
        let command = parts.next().unwrap_or(trimmed);
        match command {
            "/quit" | "/exit" => {
                self.should_quit = true;
            }
            "/clear" => {
                self.chat.messages.clear();
                self.chat.history_scroll = 0;
            }
            "/help" => {
                self.push_assistant(
                    "TUI commands:\n/help - show this help\n/login - open credential picker\n/connection - open credential picker\n/model - browse model catalog (search with s, refresh with r)\n/model list - print available models to chat\n/session - list sessions\n/session new - start a new session\n/session load <id> - load a session\n/session delete <id> - delete a session\n/web - show web adapter status\n/web setup - configure web adapter\n/whatsapp - configure WhatsApp channel\n/whatsapp status - show WhatsApp config status\n/telegram - Telegram bot setup guide\n/telegram status - show Telegram config status\n/telegram start - start Telegram adapter info\n/qr - show QR code for Web UI URL\n/clear - clear history\n/quit or /exit - leave TUI"
                        .to_string(),
                );
            }
            "/login" | "/connection" => {
                let seed = parts.collect::<Vec<_>>().join(" ");
                self.open_login_browser(if seed.trim().is_empty() {
                    None
                } else {
                    Some(seed)
                });
            }
            "/model" => {
                self.push_assistant(
                    "Loading model catalog... (inside browser: s or / to search, r to refresh)"
                        .to_string(),
                );
            }
            "/whatsapp" => {
                // "status" subcommand is handled in event.rs; bare /whatsapp opens wizard
            }
            "/telegram" => {
                // Subcommands are handled in event.rs
            }
            "/qr" => {
                // QR code generation is handled in event.rs (needs config access)
            }
            other => {
                self.push_error(format!(
                    "Unknown command: {other}. Try /help for available commands."
                ));
            }
        }

        true
    }

    /// Converts current auth input state into a submission payload.
    pub fn take_auth_submission(&mut self) -> Option<AuthSubmission> {
        let (provider, env_name, endpoint) = match &self.state {
            AppState::AuthPrompting {
                provider,
                env_name,
                endpoint,
                endpoint_env: _,
            } => (provider.clone(), env_name.clone(), endpoint.clone()),
            _ => return None,
        };

        if self.chat.input.trim().is_empty() {
            return None;
        }

        let api_key = self.take_input().trim().to_string();
        self.state = AppState::AuthValidating {
            provider: provider.clone(),
        };

        Some(AuthSubmission {
            provider,
            env_name,
            endpoint,
            api_key,
        })
    }

    /// Finalises the auth-validation flow and pushes a success or failure message to chat.
    pub fn complete_auth_validation(
        &mut self,
        provider: String,
        env_name: String,
        result: Result<(), String>,
    ) {
        match result {
            Ok(()) => self.push_assistant(format!(
                "Saved API key for '{provider}'. It will be used on the next launch (equivalent to setting {env_name})."
            )),
            Err(err) => self.push_error(format!("Failed to save API key for '{provider}': {err}")),
        }
        self.state = AppState::Idle;
    }

    fn cancel_auth_prompt(&mut self) {
        self.chat.input.clear();
        self.chat.cursor_pos = 0;
        self.state = AppState::Idle;
        self.push_assistant("Login cancelled.".to_string());
    }

    /// Transitions to the `LoginBrowsing` state, optionally pre-filtering by `seed` provider name.
    pub fn open_login_browser(&mut self, seed: Option<String>) {
        self.chat.input.clear();
        self.chat.cursor_pos = 0;
        self.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: auth_picker::parse_provider_seed(seed.as_deref()),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::SelectProvider,
            selected_provider: None,
            selected_method: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
    }

    /// Launches the step-by-step web adapter configuration wizard.
    pub fn start_web_config_wizard(
        &mut self,
        enabled: bool,
        token: String,
        port: u16,
        cors_origins: &str,
        static_dir: &str,
    ) {
        self.chat.input.clear();
        self.chat.cursor_pos = 0;
        self.screen = Screen::Chat;
        self.push_assistant(
            "Web Adapter Setup Wizard\nStep 1/6: Enable web adapter? (y/n)".to_string(),
        );
        self.state = AppState::WebConfiguring(WebConfiguringState {
            step: WebConfigStep::Enable,
            enabled,
            token,
            port: port.to_string(),
            cors_origins: cors_origins.to_string(),
            static_dir: static_dir.to_string(),
            input_buffer: String::new(),
        });
    }

    /// Takes the pending web config set when the wizard completes.
    pub fn take_pending_web_config(&mut self) -> Option<crate::config::WebConfig> {
        self.pending_web_config.take()
    }

    /// Takes the pending `AuthLoginIntent` that was set during the login browser flow.
    pub fn take_pending_auth_intent(&mut self) -> Option<AuthLoginIntent> {
        self.model.pending_auth_intent.take()
    }

    /// Re-opens the openai method selector and displays `message` as an error.
    pub fn reopen_openai_method_with_error(&mut self, message: String) {
        self.reopen_method_selector_with_error("openai", message);
    }

    /// Re-opens the method-selector step for `provider`, showing `message` as the last error.
    pub fn reopen_method_selector_with_error(&mut self, provider: &str, message: String) {
        self.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: provider.to_string(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::SelectMethod,
            selected_provider: Some(provider.to_string()),
            selected_method: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: Some(message),
            endpoint: None,
        });
    }

    /// Re-opens the provider-selection step, showing `message` as an error banner.
    pub fn reopen_provider_selection_with_error(&mut self, message: String) {
        self.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::SelectProvider,
            selected_provider: None,
            selected_method: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: Some(message),
            endpoint: None,
        });
    }

    /// Opens/updates model browser with new catalog data.
    pub fn open_model_browser(
        &mut self,
        provider: String,
        entries: Vec<model_catalog::ModelCatalogEntry>,
        query: String,
        sync_status: String,
    ) {
        self.model.model_provider = provider;
        self.model.model_entries = entries;
        self.state = AppState::ModelBrowsing {
            query,
            cursor: 0,
            scroll: 0,
            last_sync_status: sync_status,
            search_active: false,
        };
    }

    /// Updates only model entries and sync status while keeping browse options.
    pub fn update_model_browser_catalog(
        &mut self,
        provider: String,
        entries: Vec<model_catalog::ModelCatalogEntry>,
        sync_status: String,
    ) {
        self.model.model_provider = provider;
        self.model.model_entries = entries;
        if let AppState::ModelBrowsing {
            cursor,
            scroll,
            last_sync_status,
            ..
        } = &mut self.state
        {
            *last_sync_status = sync_status;
            *cursor = 0;
            *scroll = 0;
        }
    }

    /// Returns the current model-browser search query, or `None` if the browser is not active.
    pub fn model_browser_query(&self) -> Option<String> {
        match &self.state {
            AppState::ModelBrowsing { query, .. } => Some(query.clone()),
            _ => None,
        }
    }

    /// Updates the model-browser sync-status label to indicate a refresh is in progress.
    pub fn mark_model_refreshing(&mut self) {
        if let AppState::ModelBrowsing {
            last_sync_status, ..
        } = &mut self.state
        {
            *last_sync_status = "Refreshing model...".to_string();
        }
    }

    /// Returns `true` once if the user pressed `r` to request a model-catalog refresh, then resets the flag.
    pub fn take_model_refresh_request(&mut self) -> bool {
        let requested = self.model.model_refresh_requested;
        self.model.model_refresh_requested = false;
        requested
    }

    fn visible_model_entries(&self, query: &str) -> Vec<model_catalog::ModelCatalogEntry> {
        let recommended = model_catalog::filtered_entries(&self.model.model_entries, query, false);
        let all_models = model_catalog::filtered_entries(&self.model.model_entries, query, true);
        let recommended_keys: HashSet<&str> =
            recommended.iter().map(|entry| entry.id.as_str()).collect();
        let other: Vec<_> = all_models
            .into_iter()
            .filter(|entry| !recommended_keys.contains(entry.id.as_str()))
            .collect();
        let mut rows = Vec::new();
        rows.extend(recommended);
        rows.extend(other);
        // Only show model that are available.
        rows.retain(|entry| entry.available);
        rows
    }

    fn clamp_model_cursor(&mut self) {
        let query = match &self.state {
            AppState::ModelBrowsing { query, .. } => query.clone(),
            _ => return,
        };

        let visible_len = self.visible_model_entries(&query).len();
        if let AppState::ModelBrowsing { cursor, scroll, .. } = &mut self.state {
            if visible_len == 0 {
                *cursor = 0;
                *scroll = 0;
                return;
            }
            *cursor = (*cursor).min(visible_len.saturating_sub(1));
            if (*cursor as u16) < *scroll {
                *scroll = *cursor as u16;
            } else {
                *scroll = (*cursor as u16).saturating_sub(2);
            }
        }
    }

    // ── Session browser ──────────────────────────────────────

    /// Opens the session browser view.
    pub fn open_session_browser(&mut self) {
        self.state = AppState::SessionBrowsing {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            search_active: false,
        };
    }

    /// Returns visible sessions filtered by the given query string.
    pub fn visible_sessions(&self, query: &str) -> Vec<&SessionEntry> {
        if query.trim().is_empty() {
            self.session.session_list.iter().collect()
        } else {
            let q = query.to_lowercase();
            self.session
                .session_list
                .iter()
                .filter(|e| {
                    e.preview.to_lowercase().contains(&q)
                        || e.id.as_str().to_lowercase().contains(&q)
                })
                .collect()
        }
    }

    fn clamp_session_cursor(&mut self) {
        let query = match &self.state {
            AppState::SessionBrowsing { query, .. } => query.clone(),
            _ => return,
        };
        let visible_len = self.visible_sessions(&query).len();
        if let AppState::SessionBrowsing { cursor, scroll, .. } = &mut self.state {
            if visible_len == 0 {
                *cursor = 0;
                *scroll = 0;
                return;
            }
            *cursor = (*cursor).min(visible_len.saturating_sub(1));
            if (*cursor as u16) < *scroll {
                *scroll = *cursor as u16;
            } else {
                *scroll = (*cursor as u16).saturating_sub(2);
            }
        }
    }

    fn visible_login_provider_entries(
        &self,
        query: &str,
    ) -> Vec<crate::config::ProviderRegistryEntry> {
        auth_picker::filtered_provider_entries(query)
    }

    fn clamp_login_cursor(&mut self) {
        if let AppState::LoginBrowsing(LoginBrowsingState {
            query,
            cursor,
            scroll,
            step,
            ..
        }) = &mut self.state
        {
            match step {
                LoginBrowseStep::SelectProvider => {
                    let visible_len = auth_picker::filtered_provider_entries(query).len();
                    if visible_len == 0 {
                        *cursor = 0;
                        *scroll = 0;
                        return;
                    }
                    *cursor = (*cursor).min(visible_len.saturating_sub(1));
                }
                LoginBrowseStep::SelectMethod => {
                    *cursor = (*cursor).min(1);
                }
                LoginBrowseStep::InputEndpoint | LoginBrowseStep::InputApiKey => {
                    *cursor = 0;
                }
            }
            if (*cursor as u16) < *scroll {
                *scroll = *cursor as u16;
            } else {
                *scroll = (*cursor as u16).saturating_sub(3);
            }
        }
    }

    /// Apply a progress event from the agent runtime.
    pub fn apply_progress(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::LlmThinking { round } => {
                self.state = AppState::Thinking { round };
            }
            ProgressEvent::ToolCallStarted {
                tool_name, args, ..
            } => {
                let args_str = args.to_string();
                let preview = if args_str.len() > 80 {
                    format!("{}…", &args_str[..80])
                } else {
                    args_str
                };
                self.state = AppState::ExecutingTool {
                    tool_name: tool_name.clone(),
                };
                self.chat.messages.push(TuiMessage::ToolCall {
                    tool_name,
                    args_preview: preview,
                    done: false,
                });
            }
            ProgressEvent::ToolCallFinished {
                tool_name,
                output,
                is_error,
                ..
            } => {
                // Mark the last matching ToolCall as done
                for msg in self.chat.messages.iter_mut().rev() {
                    if let TuiMessage::ToolCall {
                        tool_name: name,
                        done,
                        ..
                    } = msg
                        && *name == tool_name
                        && !*done
                    {
                        *done = true;
                        break;
                    }
                }
                let preview = if output.len() > 120 {
                    format!("{}…", &output[..120])
                } else {
                    output
                };
                self.chat.messages.push(TuiMessage::ToolResult {
                    tool_name,
                    output_preview: preview,
                    is_error,
                });
            }
        }
    }

    /// Apply the final result from the agent runtime.
    pub fn apply_completion(&mut self, result: Result<String, proto::Error>) {
        match result {
            Ok(text) => {
                self.push_assistant(text);
            }
            Err(e) => {
                self.push_error(format!("{e}"));
            }
        }
        self.state = AppState::Idle;
    }

    // ── Session management ─────────────────────────────────

    /// Toggle keyboard focus between the sidebar and the main input area.
    pub fn toggle_sidebar_focus(&mut self) {
        if !self.sidebar.visible {
            return;
        }
        self.sidebar.focused = !self.sidebar.focused;
        // When focusing sidebar, select the first item if nothing is hovered
        if self.sidebar.focused
            && self.sidebar.hover.is_none()
            && !self.session.session_list.is_empty()
        {
            self.sidebar.hover = Some(0);
        }
    }

    /// Select the currently hovered sidebar session for loading.
    /// Returns the `SessionId` if a valid entry was hovered, and stores it
    /// in `pending_sidebar_selection` for the event loop to consume.
    pub fn select_sidebar_session(&mut self) -> Option<SessionId> {
        let idx = self.sidebar.hover?;
        let entry = self.session.session_list.get(idx)?;
        let id = entry.id.clone();
        self.session.pending_sidebar_selection = Some(id.clone());
        self.sidebar.focused = false;
        Some(id)
    }

    /// Consume the pending sidebar selection (set by `select_sidebar_session`).
    pub fn take_pending_sidebar_selection(&mut self) -> Option<SessionId> {
        self.session.pending_sidebar_selection.take()
    }

    /// Request deletion of the currently hovered sidebar session.
    /// Transitions to `ConfirmDelete` state and returns the session id.
    pub fn request_delete_session(&mut self) -> Option<SessionId> {
        let idx = self.sidebar.hover?;
        let entry = self.session.session_list.get(idx)?;
        let id = entry.id.clone();
        let preview = if entry.preview.is_empty() {
            "(empty session)".to_string()
        } else {
            let first_line = entry.preview.lines().next().unwrap_or(&entry.preview);
            if first_line.chars().count() > 40 {
                format!("{}…", first_line.chars().take(39).collect::<String>())
            } else {
                first_line.to_string()
            }
        };
        self.state = AppState::ConfirmDelete {
            session_id: id.as_str().to_string(),
            session_preview: preview,
        };
        Some(id)
    }

    /// Remove a session from the sidebar list by id.
    pub fn remove_session_from_list(&mut self, session_id: &SessionId) {
        self.session
            .session_list
            .retain(|e| e.id.as_str() != session_id.as_str());
        // Reset hover if it's now out of bounds
        if let Some(hover) = self.sidebar.hover
            && hover >= self.session.session_list.len()
        {
            self.sidebar.hover = if self.session.session_list.is_empty() {
                None
            } else {
                Some(self.session.session_list.len() - 1)
            };
        }
    }

    /// Load messages from a previous session into the TUI conversation history.
    /// Converts `AgentMessage` records into `TuiMessage` variants.
    pub fn load_session_messages(
        &mut self,
        session_id: SessionId,
        messages: Vec<proto::AgentMessage>,
    ) {
        self.session.session_id = session_id;
        self.chat.messages.clear();
        self.chat.history_scroll = 0;
        self.screen = Screen::Chat;

        for msg in messages {
            match msg.role {
                proto::Role::User => {
                    self.chat.messages.push(TuiMessage::User(msg.content));
                }
                proto::Role::Assistant => {
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            self.chat.messages.push(TuiMessage::ToolCall {
                                tool_name: tc.name.clone(),
                                args_preview: tc.arguments.to_string(),
                                done: true,
                            });
                        }
                    }
                    if !msg.content.is_empty() {
                        self.chat.messages.push(TuiMessage::Assistant(msg.content));
                    }
                }
                proto::Role::Tool => {
                    self.chat.messages.push(TuiMessage::ToolResult {
                        tool_name: msg.tool_name.unwrap_or_default(),
                        output_preview: msg.content,
                        is_error: false,
                    });
                }
                proto::Role::System => {
                    // Skip system messages in TUI display
                }
            }
        }
        self.scroll_to_bottom();
    }

    /// Replace the sidebar session list with a fresh list.
    pub fn refresh_session_list(&mut self, sessions: Vec<SessionEntry>) {
        self.session.session_list = sessions;
    }

    /// Consume the confirmed delete session id (set by ConfirmDelete y/Enter).
    pub fn take_confirmed_delete(&mut self) -> Option<SessionId> {
        self.session.confirmed_delete.take()
    }

    /// Sets a pending sidebar session selection (used by mouse click handler in event loop).
    pub fn set_pending_sidebar_selection(&mut self, session_id: SessionId) {
        self.session.pending_sidebar_selection = Some(session_id);
    }
    // ── TEA: update (central state transition) ─────────────────

    pub fn update(&mut self, action: super::action::Action) -> super::action::Command {
        use super::action::{Action, Command};
        match action {
            // ── Input ────────────────────────────────────────────
            Action::InsertChar(c) => {
                let is_input_active =
                    matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });
                if is_input_active {
                    self.chat.input.insert(self.chat.cursor_pos, c);
                    self.chat.cursor_pos += c.len_utf8();
                    self.command_palette_cursor = 0;
                }
                Command::None
            }
            Action::DeleteChar => {
                let is_input_active =
                    matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });
                if is_input_active && self.chat.cursor_pos > 0 {
                    let prev = self.chat.input[..self.chat.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.chat.input.drain(prev..self.chat.cursor_pos);
                    self.chat.cursor_pos = prev;
                    self.command_palette_cursor = 0;
                }
                Command::None
            }
            Action::MoveCursorLeft => {
                let is_input_active =
                    matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });
                if is_input_active && self.chat.cursor_pos > 0 {
                    self.chat.cursor_pos = self.chat.input[..self.chat.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
                Command::None
            }
            Action::MoveCursorRight => {
                let is_input_active =
                    matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });
                if is_input_active && self.chat.cursor_pos < self.chat.input.len() {
                    self.chat.cursor_pos = self.chat.input[self.chat.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.chat.cursor_pos + i)
                        .unwrap_or(self.chat.input.len());
                }
                Command::None
            }
            Action::SubmitInput => {
                if self.screen == Screen::Home {
                    self.screen = Screen::Chat;
                }
                Command::None
            }

            // ── Navigation ──────────────────────────────────────
            Action::ScrollUp(n) => {
                self.chat.history_scroll = self.chat.history_scroll.saturating_sub(n);
                Command::None
            }
            Action::ScrollDown(n) => {
                self.chat.history_scroll = self.chat.history_scroll.saturating_add(n);
                Command::None
            }
            Action::ScrollToBottom => {
                self.scroll_to_bottom();
                Command::None
            }
            Action::SwitchScreen(screen) => {
                self.screen = screen;
                Command::None
            }
            Action::ToggleSidebarFocus => {
                self.toggle_sidebar_focus();
                Command::None
            }

            // ── Chat / agent lifecycle ──────────────────────────
            Action::PushUserMessage(text) => {
                self.push_user(text);
                Command::None
            }
            Action::PushAssistantMessage(text) => {
                self.push_assistant(text);
                Command::None
            }
            Action::PushError(err) => {
                self.push_error(err);
                Command::None
            }
            Action::ApplyProgress(event) => {
                self.apply_progress(event);
                self.scroll_to_bottom();
                Command::None
            }
            Action::ApplyCompletion(result) => {
                match result {
                    Ok(text) => self.push_assistant(text),
                    Err(e) => self.push_error(e),
                }
                self.state = AppState::Idle;
                self.scroll_to_bottom();
                Command::None
            }

            // ── Sidebar ─────────────────────────────────────────
            Action::SidebarHover(idx) => {
                self.sidebar.hover = idx;
                Command::None
            }
            Action::SidebarScroll(delta) => {
                if delta > 0 {
                    self.sidebar.scroll = self.sidebar.scroll.saturating_add(delta as u16);
                } else {
                    self.sidebar.scroll = self.sidebar.scroll.saturating_sub((-delta) as u16);
                }
                Command::None
            }
            Action::SelectSidebarSession => {
                let sid = self.select_sidebar_session();
                match sid {
                    Some(id) => Command::LoadSessionFromDb(id),
                    None => Command::None,
                }
            }
            Action::RequestDeleteSession => {
                self.request_delete_session();
                Command::None
            }
            Action::ConfirmDelete => {
                if let AppState::ConfirmDelete { session_id, .. } = &self.state {
                    let id = SessionId::from(session_id.clone());
                    self.session.confirmed_delete = Some(id.clone());
                    self.state = AppState::Idle;
                    Command::DeleteSession(id)
                } else {
                    Command::None
                }
            }
            Action::CancelDelete => {
                self.state = AppState::Idle;
                Command::None
            }

            // ── Auth / login browser ────────────────────────────
            Action::OpenLoginBrowser(seed) => {
                self.open_login_browser(seed);
                Command::None
            }
            Action::CancelAuth => {
                self.cancel_auth_prompt();
                Command::None
            }
            Action::LoginBrowserKey(key) => {
                self.handle_key(key);
                Command::None
            }
            Action::SetOAuthCodeDisplayState { provider } => {
                self.state = AppState::LoginBrowsing(LoginBrowsingState {
                    query: provider.clone(),
                    cursor: 0,
                    scroll: 0,
                    step: LoginBrowseStep::InputApiKey,
                    selected_provider: Some(provider),
                    selected_method: Some(AuthMethodChoice::OAuth),
                    input_buffer: String::new(),
                    masked_buffer: String::new(),
                    last_error: None,
                    endpoint: None,
                });
                Command::None
            }
            Action::SetAuthValidating(provider) => {
                self.state = AppState::AuthValidating { provider };
                Command::None
            }

            // ── Model browser ───────────────────────────────────
            Action::OpenModelBrowser {
                provider,
                entries,
                query,
                sync_status,
            } => {
                self.open_model_browser(provider, entries, query, sync_status);
                Command::None
            }
            Action::CloseModelBrowser => {
                self.state = AppState::Idle;
                Command::None
            }
            Action::ModelBrowserKey(key) => {
                self.handle_key(key);
                Command::None
            }
            Action::MarkModelRefreshing => {
                self.mark_model_refreshing();
                Command::None
            }
            Action::UpdateModelCatalog {
                provider,
                entries,
                sync_status,
            } => {
                self.update_model_browser_catalog(provider, entries, sync_status);
                Command::None
            }

            // ── Session browser ─────────────────────────────────
            Action::OpenSessionBrowser => {
                self.open_session_browser();
                Command::None
            }
            Action::CloseSessionBrowser => {
                self.state = AppState::Idle;
                Command::None
            }
            Action::SessionBrowserKey(key) => {
                self.handle_key(key);
                Command::None
            }

            // ── Web config wizard ──────────────────────────────
            Action::WebConfigKey(key) => {
                use crossterm::event::{KeyCode, KeyModifiers};
                if let AppState::WebConfiguring(WebConfiguringState {
                    ref mut step,
                    ref mut enabled,
                    ref mut token,
                    ref mut port,
                    ref mut cors_origins,
                    ref mut static_dir,
                    ref mut input_buffer,
                }) = self.state
                {
                    // Esc cancels the wizard from any step
                    if key.code == KeyCode::Esc
                        || (key.modifiers == KeyModifiers::CONTROL
                            && key.code == KeyCode::Char('c'))
                    {
                        self.push_assistant("Web setup cancelled.".to_string());
                        self.state = AppState::Idle;
                        return Command::None;
                    }

                    match step {
                        WebConfigStep::Enable => match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                *enabled = true;
                                *step = WebConfigStep::Token;
                                *input_buffer = token.clone();
                                self.push_assistant(
                                    "Step 2/6: Auth token (leave empty for none):".to_string(),
                                );
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                *enabled = false;
                                *step = WebConfigStep::Token;
                                *input_buffer = token.clone();
                                self.push_assistant(
                                    "Step 2/6: Auth token (leave empty for none):".to_string(),
                                );
                            }
                            _ => {}
                        },
                        WebConfigStep::Token => match key.code {
                            KeyCode::Enter => {
                                *token = input_buffer.clone();
                                *step = WebConfigStep::Port;
                                *input_buffer = port.clone();
                                self.push_assistant(
                                    "Step 3/6: HTTP/WS port (default 3210):".to_string(),
                                );
                            }
                            KeyCode::Char(c) => {
                                input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                input_buffer.pop();
                            }
                            _ => {}
                        },
                        WebConfigStep::Port => match key.code {
                            KeyCode::Enter => {
                                *port = input_buffer.clone();
                                *step = WebConfigStep::CorsOrigins;
                                *input_buffer = cors_origins.clone();
                                self.push_assistant(
                                    "Step 4/6: CORS origins (comma-separated, or \"*\"):"
                                        .to_string(),
                                );
                            }
                            KeyCode::Char(c) => {
                                input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                input_buffer.pop();
                            }
                            _ => {}
                        },
                        WebConfigStep::CorsOrigins => match key.code {
                            KeyCode::Enter => {
                                *cors_origins = input_buffer.clone();
                                *step = WebConfigStep::StaticDir;
                                *input_buffer = static_dir.clone();
                                self.push_assistant("Step 5/6: Static file directory:".to_string());
                            }
                            KeyCode::Char(c) => {
                                input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                input_buffer.pop();
                            }
                            _ => {}
                        },
                        WebConfigStep::StaticDir => match key.code {
                            KeyCode::Enter => {
                                *static_dir = input_buffer.clone();
                                let summary = format!(
                                    "Step 6/6: Confirm settings?\n\
                                     enabled: {}\n\
                                     token: {}\n\
                                     port: {}\n\
                                     cors_origins: {}\n\
                                     static_dir: {}\n\
                                     Save? (y/n)",
                                    enabled,
                                    if token.is_empty() {
                                        "(none)"
                                    } else {
                                        token.as_str()
                                    },
                                    port,
                                    cors_origins,
                                    static_dir,
                                );
                                *step = WebConfigStep::Confirm;
                                *input_buffer = String::new();
                                self.push_assistant(summary);
                            }
                            KeyCode::Char(c) => {
                                input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                input_buffer.pop();
                            }
                            _ => {}
                        },
                        WebConfigStep::Confirm => match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                let web_cfg = crate::config::WebConfig {
                                    enabled: *enabled,
                                    token: token.clone(),
                                    port: port.parse::<u16>().unwrap_or(3210),
                                    cors_origins: cors_origins.clone(),
                                    static_dir: static_dir.clone(),
                                    shared_session_id: crate::config::WebConfig::default()
                                        .shared_session_id,
                                };
                                self.pending_web_config = Some(web_cfg);
                                self.push_assistant("Web config saved.".to_string());
                                self.state = AppState::Idle;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                self.push_assistant("Web setup cancelled.".to_string());
                                self.state = AppState::Idle;
                            }
                            _ => {}
                        },
                    }
                }
                Command::None
            }
            // ── Command palette ─────────────────────────────────
            Action::PaletteMoveUp => {
                self.command_palette_cursor = self.command_palette_cursor.saturating_sub(1);
                Command::None
            }
            Action::PaletteMoveDown => {
                let max = self.palette_filtered_commands().len().saturating_sub(1);
                self.command_palette_cursor = (self.command_palette_cursor + 1).min(max);
                Command::None
            }
            Action::PaletteSelect => {
                self.take_palette_command();
                Command::None
            }
            Action::PaletteClose => {
                self.chat.input.clear();
                self.chat.cursor_pos = 0;
                self.command_palette_cursor = 0;
                Command::None
            }
            Action::PaletteTabComplete => {
                let cmd_name = self
                    .palette_filtered_commands()
                    .get(self.command_palette_cursor)
                    .map(|c| c.name.to_string());
                if let Some(name) = cmd_name {
                    self.chat.input = name.clone();
                    self.chat.cursor_pos = name.len();
                    self.command_palette_cursor = 0;
                }
                Command::None
            }

            // ── Text selection ──────────────────────────────────
            Action::TextSelectionStart { row, col } => {
                self.chat.text_selection.anchor = Some((row, col));
                self.chat.text_selection.endpoint = Some((row, col));
                self.chat.text_selection.dragging = true;
                Command::None
            }
            Action::TextSelectionDrag { row, col } => {
                if self.chat.text_selection.dragging {
                    self.chat.text_selection.endpoint = Some((row, col));
                }
                Command::None
            }
            Action::TextSelectionEnd { row, col } => {
                if self.chat.text_selection.dragging {
                    self.chat.text_selection.endpoint = Some((row, col));
                    self.chat.text_selection.dragging = false;
                    if self.chat.text_selection.is_active()
                        && let Some((start, end)) = self.chat.text_selection.ordered_range()
                    {
                        let grid = self.chat.chat_text_grid.clone();
                        let scroll = self.chat.chat_scroll_clamped;
                        if let Some(text) =
                            crate::tui::selection::extract_selected_text(&grid, start, end, scroll)
                        {
                            return Command::CopyToClipboard(text);
                        }
                    }
                }
                Command::None
            }
            Action::TextSelectionCopy => {
                if let Some((start, end)) = self.chat.text_selection.ordered_range() {
                    let grid = self.chat.chat_text_grid.clone();
                    let scroll = self.chat.chat_scroll_clamped;
                    if let Some(text) =
                        super::selection::extract_selected_text(&grid, start, end, scroll)
                    {
                        self.chat.text_selection.clear();
                        return Command::CopyToClipboard(text);
                    }
                }
                self.chat.text_selection.clear();
                Command::None
            }
            Action::TextSelectionClear => {
                self.chat.text_selection.clear();
                Command::None
            }

            // ── System ──────────────────────────────────────────
            Action::Tick => {
                self.spinner_tick = self.spinner_tick.wrapping_add(1);
                Command::None
            }
            Action::Quit => {
                self.should_quit = true;
                Command::None
            }
            Action::Resize => Command::None,

            Action::SetThinking => {
                self.state = AppState::Thinking { round: 0 };
                Command::None
            }
            Action::SetIdle => {
                self.state = AppState::Idle;
                Command::None
            }

            // ── Session management ──────────────────────────────
            Action::LoadSession {
                session_id,
                messages,
            } => {
                self.load_session_messages(session_id, messages);
                Command::None
            }
            Action::RefreshSessionList(sessions) => {
                self.refresh_session_list(sessions);
                Command::None
            }
            Action::NewSession(sid) => {
                self.load_session_messages(sid.clone(), Vec::new());
                self.push_assistant(format!("New session created: `{}`", sid.as_str()));
                Command::None
            }
            Action::RemoveSession(sid) => {
                self.remove_session_from_list(&sid);
                Command::None
            }

            // ── Model / provider ────────────────────────────────
            Action::SetModel(model) => {
                self.model.model_name = model;
                Command::None
            }
            Action::SetProviderName(name) => {
                self.model.provider_name = name;
                Command::None
            }

            // ── Slash commands ──────────────────────────────────
            Action::SlashCommand(raw) => {
                self.handle_slash_command(&raw);
                self.scroll_to_bottom();
                Command::None
            }

            // ── WhatsApp pairing ────────────────────────
            Action::OpenWhatsAppSetup => {
                self.chat.input.clear();
                self.chat.cursor_pos = 0;
                self.state = AppState::WhatsAppSetup {
                    step: WhatsAppSetupStep::CheckingPrereqs,
                };
                self.screen = Screen::Chat;
                Command::CheckWhatsAppPrereqs
            }
            Action::WhatsAppSetupCancel => {
                self.state = AppState::Idle;
                self.push_assistant("WhatsApp pairing cancelled.".to_string());
                Command::None
            }
            Action::WhatsAppPrereqsChecked {
                node_ok,
                bridge_installed,
            } => {
                if !node_ok {
                    self.state = AppState::Idle;
                    self.push_error(
                        "Node.js is required for WhatsApp bridge. Install from https://nodejs.org/"
                            .to_string(),
                    );
                    Command::None
                } else if !bridge_installed {
                    self.state = AppState::WhatsAppSetup {
                        step: WhatsAppSetupStep::InstallingBridge,
                    };
                    Command::InstallWhatsAppBridge
                } else {
                    self.state = AppState::WhatsAppSetup {
                        step: WhatsAppSetupStep::WaitingForQr,
                    };
                    Command::SpawnWhatsAppBridge
                }
            }
            Action::WhatsAppBridgeInstalled(result) => match result {
                Ok(()) => {
                    self.state = AppState::WhatsAppSetup {
                        step: WhatsAppSetupStep::WaitingForQr,
                    };
                    Command::SpawnWhatsAppBridge
                }
                Err(msg) => {
                    self.state = AppState::Idle;
                    self.push_error(format!("Failed to install WhatsApp bridge: {msg}"));
                    Command::None
                }
            },
            Action::WhatsAppQrReceived(qr_data) => {
                if let AppState::WhatsAppSetup { step } = &mut self.state {
                    match generate_qr_lines(&qr_data) {
                        Ok(qr_lines) => {
                            *step = WhatsAppSetupStep::DisplayQr { qr_data, qr_lines };
                        }
                        Err(e) => {
                            self.push_error(format!("Failed to render QR code: {e}"));
                        }
                    }
                }
                Command::None
            }
            Action::WhatsAppConnected { phone, name } => {
                if let AppState::WhatsAppSetup { step } = &mut self.state {
                    *step = WhatsAppSetupStep::Connected {
                        phone: phone.clone(),
                        name: name.clone(),
                    };
                }
                self.push_assistant(format!("WhatsApp connected! Phone: {phone}, Name: {name}"));
                Command::None
            }
            Action::OpenQrCode { url, qr_lines } => {
                self.state = AppState::QrCodeDisplay { url, qr_lines };
                Command::None
            }
            Action::CloseQrCode => {
                self.state = AppState::Idle;
                Command::None
            }
            Action::WhatsAppSetupKey(key) => {
                self.handle_key(key);
                Command::None
            }
        }
    }
    /// Returns the number of user/assistant message pairs in the conversation history.
    pub fn conversation_count(&self) -> usize {
        self.chat
            .messages
            .iter()
            .filter(|m| matches!(m, TuiMessage::User(_) | TuiMessage::Assistant(_)))
            .count()
    }

    /// Sets `history_scroll` to its maximum value so the next render shows the latest messages.
    pub fn scroll_to_bottom(&mut self) {
        // Set to a large value; render_history clamps it to max_scroll.
        self.chat.history_scroll = u16::MAX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::action::{Action, Command};
    use ratatui::{Terminal, backend::TestBackend};

    fn make_app() -> TuiApp {
        TuiApp::new(
            "gpt-4o",
            SessionId::new(),
            ChannelId::from("cli:tui"),
            "openai",
        )
    }

    fn sample_models() -> Vec<model_catalog::ModelCatalogEntry> {
        vec![
            model_catalog::ModelCatalogEntry {
                id: "gpt-5-codex".to_string(),
                provider: String::new(),
                recommended_for_coding: true,
                status: model_catalog::ModelStatus::Stable,
                source: model_catalog::ModelSource::Docs,
                available: true,
            },
            model_catalog::ModelCatalogEntry {
                id: "claude-sonnet-4.6".to_string(),
                provider: String::new(),
                recommended_for_coding: true,
                status: model_catalog::ModelStatus::Stable,
                source: model_catalog::ModelSource::Docs,
                available: true,
            },
        ]
    }

    #[test]
    fn apply_progress_tool_started_updates_state() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command":"ls"}),
        });
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::ToolCall { .. }));
        assert_eq!(
            app.state,
            AppState::ExecutingTool {
                tool_name: "system.run".into()
            }
        );
    }

    #[test]
    fn apply_progress_tool_finished_adds_result() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({}),
        });
        app.apply_progress(ProgressEvent::ToolCallFinished {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            output: "file1.rs\nfile2.rs".into(),
            is_error: false,
        });
        assert_eq!(app.chat.messages.len(), 2);
        assert!(matches!(
            &app.chat.messages[1],
            TuiMessage::ToolResult {
                is_error: false,
                ..
            }
        ));
    }

    #[test]
    fn apply_completion_ok_pushes_assistant() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.apply_completion(Ok("hello world".into()));
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Assistant(t) if t == "hello world"));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn apply_completion_err_pushes_error() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.apply_completion(Err(proto::Error::Llm(proto::LlmError::RateLimit)));
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Error(_)));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn handle_key_inserts_chars() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.chat.input, "ab");
        assert_eq!(app.chat.cursor_pos, 2);
    }

    #[test]
    fn handle_key_backspace_deletes() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.chat.input, "");
        assert_eq!(app.chat.cursor_pos, 0);
    }

    #[test]
    fn handle_key_ignores_input_when_thinking() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.chat.input, "");
    }

    #[test]
    fn handle_key_accepts_input_when_auth_prompting() {
        let mut app = make_app();
        app.state = AppState::AuthPrompting {
            provider: "together".to_string(),
            env_name: "TOGETHER_API_KEY".to_string(),
            endpoint: None,
            endpoint_env: None,
        };

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(app.chat.input, "sk");
    }

    #[test]
    fn handle_key_escape_cancels_auth_prompt() {
        let mut app = make_app();
        app.state = AppState::AuthPrompting {
            provider: "together".to_string(),
            env_name: "TOGETHER_API_KEY".to_string(),
            endpoint: None,
            endpoint_env: None,
        };
        app.chat.input = "secret".to_string();
        app.chat.cursor_pos = app.chat.input.len();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(app.state, AppState::Idle);
        assert_eq!(app.chat.input, "");
        assert_eq!(app.chat.cursor_pos, 0);
        assert!(
            matches!(app.chat.messages.last(), Some(TuiMessage::Assistant(text)) if text.contains("cancelled"))
        );
    }

    #[test]
    fn take_auth_submission_moves_to_validating() {
        let mut app = make_app();
        app.state = AppState::AuthPrompting {
            provider: "together".to_string(),
            env_name: "TOGETHER_API_KEY".to_string(),
            endpoint: None,
            endpoint_env: None,
        };
        app.chat.input = "  sk-test  ".to_string();
        app.chat.cursor_pos = app.chat.input.len();

        let submission = app.take_auth_submission().expect("submission expected");

        assert_eq!(submission.provider, "together");
        assert_eq!(submission.env_name, "TOGETHER_API_KEY");
        assert_eq!(submission.endpoint, None);
        assert_eq!(submission.api_key, "sk-test");
        assert_eq!(app.chat.input, "");
        assert_eq!(
            app.state,
            AppState::AuthValidating {
                provider: "together".to_string()
            }
        );
    }

    #[test]
    fn apply_progress_llm_thinking_sets_state_round() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::LlmThinking { round: 3 });
        assert_eq!(app.state, AppState::Thinking { round: 3 });
    }

    #[test]
    fn apply_progress_marks_latest_matching_tool_call_done() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command":"echo 1"}),
        });
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c2".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command":"echo 2"}),
        });
        app.apply_progress(ProgressEvent::ToolCallFinished {
            call_id: "c2".into(),
            tool_name: "system.run".into(),
            output: "ok".into(),
            is_error: false,
        });

        assert!(matches!(
            &app.chat.messages[0],
            TuiMessage::ToolCall { done: false, .. }
        ));
        assert!(matches!(
            &app.chat.messages[1],
            TuiMessage::ToolCall { done: true, .. }
        ));
    }

    #[test]
    fn handle_key_moves_cursor_left_and_right_with_utf8() {
        let mut app = make_app();
        app.chat.input = "a한b".into();
        app.chat.cursor_pos = app.chat.input.len();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.chat.cursor_pos, "a한".len());

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.chat.cursor_pos, "a".len());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.chat.cursor_pos, "a한".len());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.chat.cursor_pos, "a한b".len());
    }

    #[test]
    fn handle_key_scroll_shortcuts_update_history_scroll() {
        let mut app = make_app();
        app.chat.history_scroll = 5;

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.chat.history_scroll, 4);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.chat.history_scroll, 5);

        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.chat.history_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.chat.history_scroll, 10);
    }

    #[test]
    fn handle_key_quit_shortcuts_only_when_idle() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.should_quit);

        app.should_quit = false;
        app.state = AppState::Thinking { round: 1 };
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.should_quit);

        app.state = AppState::Idle;
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn model_browser_enter_selects_model_for_session() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced from remote".to_string(),
        );

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.model.model_name, "gpt-5-codex");
        assert_eq!(app.state, AppState::Idle);
        assert!(matches!(
            app.chat.messages.last(),
            Some(TuiMessage::Assistant(text)) if text.contains("Selected model")
        ));
    }

    #[test]
    fn model_browser_refresh_hotkey_sets_request_flag() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced from remote".to_string(),
        );

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert!(app.take_model_refresh_request());
        assert!(!app.take_model_refresh_request());
    }

    #[test]
    fn model_browser_search_mode_esc_then_close() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced from remote".to_string(),
        );

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::ModelBrowsing {
                search_active: true,
                ..
            }
        ));

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::ModelBrowsing {
                search_active: false,
                ..
            }
        ));

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn model_browser_slash_starts_search_mode() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced from remote".to_string(),
        );

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::ModelBrowsing {
                search_active: true,
                ..
            }
        ));
    }

    #[test]
    fn render_draws_all_message_variants_without_mutating_state() {
        let mut app = make_app();
        app.push_user("user line".into());
        app.push_assistant("first\nsecond".into());
        app.chat.messages.push(TuiMessage::ToolCall {
            tool_name: "system.run".into(),
            args_preview: "{\"command\":\"echo ok\"}".into(),
            done: false,
        });
        app.chat.messages.push(TuiMessage::ToolResult {
            tool_name: "system.run".into(),
            output_preview: "ok".into(),
            is_error: false,
        });
        app.push_error("boom".into());
        app.chat.input = "typed".into();
        app.chat.cursor_pos = 2;
        app.state = AppState::ExecutingTool {
            tool_name: "system.run".into(),
        };
        app.spinner_tick = 3;
        app.chat.history_scroll = 7;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.chat.input, "typed");
        assert_eq!(app.chat.cursor_pos, 2);
        assert_eq!(app.chat.history_scroll, 7);
        assert_eq!(app.chat.messages.len(), 5);
    }

    #[test]
    fn render_idle_placeholder_path_executes() {
        let mut app = make_app();
        app.state = AppState::Idle;
        app.chat.input.clear();
        app.chat.cursor_pos = 0;

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.state, AppState::Idle);
        assert_eq!(app.chat.input, "");
    }

    #[test]
    fn take_input_resets() {
        let mut app = make_app();
        app.chat.input = "hello".into();
        app.chat.cursor_pos = 5;
        let taken = app.take_input();
        assert_eq!(taken, "hello");
        assert_eq!(app.chat.input, "");
        assert_eq!(app.chat.cursor_pos, 0);
    }

    #[test]
    fn handle_slash_command_help_pushes_local_message() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/help");
        assert!(handled);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Assistant(_)));
    }

    #[test]
    fn handle_slash_command_login_opens_login_browser_with_seed() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/login openai");
        assert!(handled);
        assert!(matches!(
            &app.state,
            AppState::LoginBrowsing(LoginBrowsingState { query, step, .. }) if query == "openai" && *step == LoginBrowseStep::SelectProvider
        ));
        assert!(app.chat.messages.is_empty());
    }

    #[test]
    fn handle_slash_command_login_without_provider_opens_browser() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/login");
        assert!(handled);
        assert!(
            matches!(&app.state, AppState::LoginBrowsing(LoginBrowsingState { query, .. }) if query.is_empty())
        );
    }

    #[test]
    fn handle_slash_command_connection_alias_opens_login_browser() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/connection copilot");
        assert!(handled);
        assert!(matches!(
            &app.state,
            AppState::LoginBrowsing(LoginBrowsingState { query, .. }) if query == "copilot"
        ));
    }

    #[test]
    fn handle_key_login_browser_enter_opens_method_step_for_openai() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(
            &app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step,
                selected_provider,
                ..
            }) if *step == LoginBrowseStep::SelectMethod
                && selected_provider.as_deref() == Some("openai")
        ));
    }

    #[test]
    fn handle_slash_command_clear_clears_history() {
        let mut app = make_app();
        app.push_user("hi".into());
        app.push_assistant("hello".into());

        let handled = app.handle_slash_command("/clear");
        assert!(handled);
        assert!(app.chat.messages.is_empty());
        assert_eq!(app.chat.history_scroll, 0);
    }

    #[test]
    fn handle_slash_command_quit_sets_should_quit() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/quit");
        assert!(handled);
        assert!(app.should_quit);
    }

    #[test]
    fn handle_slash_command_unknown_adds_error() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/nope");
        assert!(handled);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Error(_)));
    }

    #[test]
    fn handle_slash_command_whatsapp_is_consumed_without_local_side_effect() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/whatsapp");
        assert!(handled);
        assert!(app.chat.messages.is_empty());
        assert!(!app.should_quit);
    }

    #[test]
    fn handle_slash_command_telegram_is_consumed_without_local_side_effect() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/telegram");
        assert!(handled);
        assert!(app.chat.messages.is_empty());
        assert!(!app.should_quit);
    }

    #[test]
    fn handle_slash_command_returns_false_for_plain_message() {
        let mut app = make_app();
        let handled = app.handle_slash_command("hello");
        assert!(!handled);
        assert!(app.chat.messages.is_empty());
    }

    #[test]
    fn app_state_equality() {
        assert_eq!(AppState::Idle, AppState::Idle);
        assert_eq!(
            AppState::Thinking { round: 1 },
            AppState::Thinking { round: 1 }
        );
        assert_ne!(AppState::Idle, AppState::Thinking { round: 1 });
    }

    #[test]
    fn scroll_to_bottom_sets_max() {
        let mut app = make_app();
        app.scroll_to_bottom();
        assert_eq!(app.chat.history_scroll, u16::MAX);
    }

    // ── Command picker tests ────────────────────────────────

    #[test]
    fn command_picker_activates_on_slash() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        assert!(!app.is_palette_active());
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(app.is_palette_active());
        assert!(app.chat.input.starts_with('/'));
    }

    #[test]
    fn command_picker_filters_by_query() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(app.is_palette_active());
        let cmds = app.palette_filtered_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "/help");
    }

    #[test]
    fn command_picker_cursor_navigation() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(app.command_palette_cursor, 0);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.command_palette_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.command_palette_cursor, 2);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.command_palette_cursor, 1);

        // Up at 0 stays at 0
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.command_palette_cursor, 0);
    }

    #[test]
    fn command_picker_enter_selects() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        // Type "/he" to filter to /help, then select via take_palette_command
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(app.is_palette_active());
        let cmd = app.take_palette_command();
        assert_eq!(cmd, Some("/help".to_string()));
        assert_eq!(app.chat.input, "/help");
        assert_eq!(app.command_palette_cursor, 0);
    }

    #[test]
    fn command_picker_esc_cancels() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(app.is_palette_active());

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.is_palette_active());
        assert_eq!(app.chat.input, "");
        assert_eq!(app.chat.cursor_pos, 0);
        assert_eq!(app.command_palette_cursor, 0);
    }

    #[test]
    fn command_picker_deactivates_on_non_slash() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(app.is_palette_active());

        // Backspace removes the "/" → picker deactivates
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(!app.is_palette_active());
        assert_eq!(app.chat.input, "");
    }

    // ── Session management tests ──────────────────────────────

    fn make_session_entry(id: &str, preview: &str) -> SessionEntry {
        SessionEntry {
            id: SessionId::from(id),
            channel_id: "cli:tui".to_string(),
            updated_at: chrono::Utc::now(),
            preview: preview.to_string(),
        }
    }

    #[test]
    fn toggle_sidebar_focus_flips_state() {
        let mut app = make_app();
        assert!(!app.sidebar.focused);
        app.toggle_sidebar_focus();
        assert!(app.sidebar.focused);
        app.toggle_sidebar_focus();
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn select_sidebar_session_returns_hovered_id() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "hello"),
            make_session_entry("s2", "world"),
        ];
        app.sidebar.hover = Some(1);
        let selected = app.select_sidebar_session();
        assert_eq!(selected.as_ref().map(|s| s.as_str()), Some("s2"));
        // pending_sidebar_selection should be set
        assert_eq!(
            app.session
                .pending_sidebar_selection
                .as_ref()
                .map(|s| s.as_str()),
            Some("s2")
        );
        // sidebar should be unfocused
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn select_sidebar_session_returns_none_when_no_hover() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.sidebar.hover = None;
        assert!(app.select_sidebar_session().is_none());
    }

    #[test]
    fn request_delete_session_transitions_to_confirm_delete() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello world")];
        app.sidebar.hover = Some(0);
        let del_id = app.request_delete_session();
        assert_eq!(del_id.as_ref().map(|s| s.as_str()), Some("s1"));
        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
    }

    #[test]
    fn request_delete_session_returns_none_when_no_hover() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.sidebar.hover = None;
        assert!(app.request_delete_session().is_none());
    }

    #[test]
    fn remove_session_from_list_removes_entry() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "hello"),
            make_session_entry("s2", "world"),
            make_session_entry("s3", "foo"),
        ];
        app.sidebar.hover = Some(2);
        let id = SessionId::from("s2");
        app.remove_session_from_list(&id);
        assert_eq!(app.session.session_list.len(), 2);
        assert_eq!(app.session.session_list[0].id.as_str(), "s1");
        assert_eq!(app.session.session_list[1].id.as_str(), "s3");
        // hover should be clamped
        assert!(app.sidebar.hover.unwrap() < app.session.session_list.len());
    }

    #[test]
    fn remove_session_from_list_clears_hover_when_empty() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.sidebar.hover = Some(0);
        let id = SessionId::from("s1");
        app.remove_session_from_list(&id);
        assert!(app.session.session_list.is_empty());
        assert!(app.sidebar.hover.is_none());
    }

    #[test]
    fn load_session_messages_converts_agent_messages() {
        let mut app = make_app();
        let sid = SessionId::from("test-session");
        let messages = vec![
            proto::AgentMessage::new(sid.clone(), proto::Role::User, "hello"),
            proto::AgentMessage::new(sid.clone(), proto::Role::Assistant, "hi there"),
        ];
        app.load_session_messages(sid.clone(), messages);
        assert_eq!(app.chat.messages.len(), 2);
        assert!(matches!(&app.chat.messages[0], TuiMessage::User(t) if t == "hello"));
        assert!(matches!(&app.chat.messages[1], TuiMessage::Assistant(t) if t == "hi there"));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn load_session_messages_handles_tool_calls() {
        let mut app = make_app();
        let sid = SessionId::from("test-session");
        let mut assistant = proto::AgentMessage::new(sid.clone(), proto::Role::Assistant, "");
        assistant.tool_calls = Some(vec![proto::ToolCall {
            id: "call-1".to_string(),
            name: "system.run".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
        }]);
        let tool = proto::AgentMessage::tool_result(sid.clone(), "call-1", "system.run", "file.rs");
        app.load_session_messages(sid, vec![assistant, tool]);
        assert_eq!(app.chat.messages.len(), 2);
        assert!(
            matches!(&app.chat.messages[0], TuiMessage::ToolCall { tool_name, .. } if tool_name == "system.run")
        );
        assert!(
            matches!(&app.chat.messages[1], TuiMessage::ToolResult { tool_name, .. } if tool_name == "system.run")
        );
    }

    #[test]
    fn refresh_session_list_replaces_entries() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("old", "old")];
        let new_sessions = vec![
            make_session_entry("new1", "first"),
            make_session_entry("new2", "second"),
        ];
        app.refresh_session_list(new_sessions);
        assert_eq!(app.session.session_list.len(), 2);
        assert_eq!(app.session.session_list[0].id.as_str(), "new1");
    }

    #[test]
    fn take_confirmed_delete_consumes_value() {
        let mut app = make_app();
        app.session.confirmed_delete = Some(SessionId::from("del-me"));
        let taken = app.take_confirmed_delete();
        assert_eq!(taken.as_ref().map(|s| s.as_str()), Some("del-me"));
        assert!(app.take_confirmed_delete().is_none());
    }

    #[test]
    fn toggle_sidebar_focus_noop_when_hidden() {
        let mut app = make_app();
        app.sidebar.visible = false;
        app.sidebar.focused = false;

        app.toggle_sidebar_focus();
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn confirm_delete_dialog_y_confirms() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "hello"),
            make_session_entry("s2", "world"),
        ];
        app.sidebar.hover = Some(1);
        app.request_delete_session();

        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        assert_eq!(app.state, AppState::Idle);
        let del = app.take_confirmed_delete();
        assert_eq!(del.as_ref().map(|s| s.as_str()), Some("s2"));
    }

    #[test]
    fn confirm_delete_dialog_n_cancels() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "hello"),
            make_session_entry("s2", "world"),
        ];
        app.sidebar.hover = Some(1);
        app.request_delete_session();

        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        assert_eq!(app.state, AppState::Idle);
        assert!(app.take_confirmed_delete().is_none());
    }

    #[test]
    fn sidebar_focused_navigation_down() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "a"),
            make_session_entry("s2", "b"),
            make_session_entry("s3", "c"),
            make_session_entry("s4", "d"),
            make_session_entry("s5", "e"),
        ];
        app.sidebar.visible = true;
        app.sidebar.focused = true;
        app.sidebar.hover = Some(0);

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.sidebar.hover, Some(1));

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.sidebar.hover, Some(2));

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(app.sidebar.hover, Some(1));

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.sidebar.hover, Some(0));
    }

    #[test]
    fn sidebar_focused_enter_sets_pending_selection() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "a"),
            make_session_entry("s2", "b"),
            make_session_entry("s3", "c"),
        ];
        app.sidebar.visible = true;
        app.sidebar.focused = true;
        app.sidebar.hover = Some(1);

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let pending = app.take_pending_sidebar_selection();
        assert_eq!(pending.as_ref().map(|s| s.as_str()), Some("s2"));
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn sidebar_focused_d_triggers_confirm_delete() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "a"),
            make_session_entry("s2", "b"),
            make_session_entry("s3", "c"),
        ];
        app.sidebar.visible = true;
        app.sidebar.focused = true;
        app.sidebar.hover = Some(2);

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));

        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
        if let AppState::ConfirmDelete { session_id, .. } = &app.state {
            assert_eq!(session_id, "s3");
        }
    }

    // ── Session browser tests ──────────────────────────────

    #[test]
    fn open_session_browser_sets_state() {
        let mut app = make_app();
        app.open_session_browser();
        assert!(matches!(
            app.state,
            AppState::SessionBrowsing {
                search_active: false,
                ..
            }
        ));
    }

    #[test]
    fn session_browser_esc_closes() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn session_browser_search_mode_esc_then_close() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        // 's' activates search
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::SessionBrowsing {
                search_active: true,
                ..
            }
        ));

        // first Esc deactivates search
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::SessionBrowsing {
                search_active: false,
                ..
            }
        ));

        // second Esc closes browser
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn session_browser_slash_starts_search_mode() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::SessionBrowsing {
                search_active: true,
                ..
            }
        ));
    }

    #[test]
    fn session_browser_navigation_j_k() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "first"),
            make_session_entry("s2", "second"),
            make_session_entry("s3", "third"),
        ];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if let AppState::SessionBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 1);
        } else {
            panic!("expected SessionBrowsing");
        }

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        if let AppState::SessionBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 0);
        } else {
            panic!("expected SessionBrowsing");
        }
    }

    #[test]
    fn session_browser_enter_loads_session() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "first"),
            make_session_entry("s2", "second"),
        ];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        // Move to second entry
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.state, AppState::Idle);
        let pending = app.take_pending_sidebar_selection();
        assert_eq!(pending.as_ref().map(|s| s.as_str()), Some("s2"));
    }

    #[test]
    fn session_browser_n_creates_new() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        assert_eq!(app.state, AppState::Idle);
        assert!(app.session.session_browser_new_requested);
    }

    #[test]
    fn session_browser_d_triggers_delete() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "first"),
            make_session_entry("s2", "second"),
        ];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        // Move to second entry and press 'd'
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));

        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
        if let AppState::ConfirmDelete { session_id, .. } = &app.state {
            assert_eq!(session_id, "s2");
        }
    }

    #[test]
    fn session_browser_ctrl_c_closes() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn visible_sessions_filters_by_query() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("abc123", "deploy script"),
            make_session_entry("def456", "test runner"),
            make_session_entry("ghi789", "deploy fix"),
        ];

        let all = app.visible_sessions("");
        assert_eq!(all.len(), 3);

        let deploy = app.visible_sessions("deploy");
        assert_eq!(deploy.len(), 2);
        assert_eq!(deploy[0].id.as_str(), "abc123");
        assert_eq!(deploy[1].id.as_str(), "ghi789");

        let by_id = app.visible_sessions("def");
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].id.as_str(), "def456");
    }

    #[test]
    fn session_browser_search_typing_filters() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "deploy script"),
            make_session_entry("s2", "test runner"),
        ];
        app.open_session_browser();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        // Enter search mode and type 'dep'
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));

        if let AppState::SessionBrowsing { query, cursor, .. } = &app.state {
            assert_eq!(query, "dep");
            assert_eq!(*cursor, 0);
            let visible = app.visible_sessions(query);
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0].preview, "deploy script");
        } else {
            panic!("expected SessionBrowsing");
        }
    }

    #[test]
    fn render_session_browser_no_panic() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "hello world"),
            make_session_entry("s2", "another session"),
        ];
        app.open_session_browser();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        // State should be preserved after render
        assert!(matches!(app.state, AppState::SessionBrowsing { .. }));
    }

    #[test]
    fn render_session_browser_empty_no_panic() {
        let mut app = make_app();
        app.open_session_browser();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert!(matches!(app.state, AppState::SessionBrowsing { .. }));
    }

    // ── Model browser extended tests ──────────────────────

    #[test]
    fn render_model_browser_no_panic() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced from remote".to_string(),
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert!(matches!(app.state, AppState::ModelBrowsing { .. }));
    }

    #[test]
    fn render_model_browser_with_search_active() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced from remote".to_string(),
        );

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        if let AppState::ModelBrowsing {
            query,
            search_active,
            ..
        } = &app.state
        {
            assert!(query.contains("gpt"));
            assert!(*search_active);
        } else {
            panic!("expected ModelBrowsing");
        }
    }

    #[test]
    fn render_model_browser_empty_entries() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            vec![],
            String::new(),
            "Offline (no cache)".to_string(),
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_model_browser_empty_with_query() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            "nonexistent_xyz".to_string(),
            "Synced".to_string(),
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn model_browser_navigation_j_k_up_down_page() {
        let mut app = make_app();
        let mut models = sample_models();
        for i in 0..15 {
            models.push(model_catalog::ModelCatalogEntry {
                id: format!("extra-model-{i}"),
                provider: "openai".to_string(),
                recommended_for_coding: true,
                status: model_catalog::ModelStatus::Stable,
                source: model_catalog::ModelSource::Docs,
                available: true,
            });
        }
        app.open_model_browser(
            "openai".to_string(),
            models,
            String::new(),
            "Synced".to_string(),
        );

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if let AppState::ModelBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 1);
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        if let AppState::ModelBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 0);
        }
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        if let AppState::ModelBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 1);
        }
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        if let AppState::ModelBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 0);
        }
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        if let AppState::ModelBrowsing { cursor, .. } = &app.state {
            assert!(*cursor > 0);
        }
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        if let AppState::ModelBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 0);
        }
    }

    #[test]
    fn model_browser_search_type_and_backspace() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced".to_string(),
        );
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        if let AppState::ModelBrowsing { query, .. } = &app.state {
            assert_eq!(query, "gp");
        }
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        if let AppState::ModelBrowsing { query, .. } = &app.state {
            assert_eq!(query, "g");
        }
    }

    #[test]
    fn model_browser_ctrl_c_closes() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced".to_string(),
        );
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn model_browser_enter_with_empty_entries() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            vec![],
            String::new(),
            "Synced".to_string(),
        );
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
        assert!(app.take_pending_model_change().is_none());
    }

    #[test]
    fn update_model_browser_catalog_preserves_browsing() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced".to_string(),
        );
        app.update_model_browser_catalog(
            "openai".to_string(),
            sample_models(),
            "Refreshed".to_string(),
        );
        if let AppState::ModelBrowsing {
            last_sync_status,
            cursor,
            ..
        } = &app.state
        {
            assert_eq!(last_sync_status, "Refreshed");
            assert_eq!(*cursor, 0);
        } else {
            panic!("expected ModelBrowsing");
        }
    }

    #[test]
    fn model_browser_query_returns_none_when_not_browsing() {
        let app = make_app();
        assert!(app.model_browser_query().is_none());
    }

    #[test]
    fn model_browser_query_returns_query_when_browsing() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            "test-query".to_string(),
            "Synced".to_string(),
        );
        assert_eq!(app.model_browser_query(), Some("test-query".to_string()));
    }

    #[test]
    fn mark_model_refreshing_updates_sync_status() {
        let mut app = make_app();
        app.open_model_browser(
            "openai".to_string(),
            sample_models(),
            String::new(),
            "Synced".to_string(),
        );
        app.mark_model_refreshing();
        if let AppState::ModelBrowsing {
            last_sync_status, ..
        } = &app.state
        {
            assert_eq!(last_sync_status, "Refreshing model...");
        }
    }

    // ── Login browser extended tests ──────────────────────

    #[test]
    fn render_login_browser_provider_step_no_panic() {
        let mut app = make_app();
        app.open_login_browser(None);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(matches!(app.state, AppState::LoginBrowsing(_)));
    }

    #[test]
    fn render_login_browser_method_step_no_panic() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::SelectMethod,
                ..
            })
        ));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_login_browser_api_key_step_no_panic() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::InputApiKey,
                ..
            })
        ));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn login_browser_provider_navigation_and_search() {
        let mut app = make_app();
        app.open_login_browser(None);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { query, .. }) = &app.state {
            assert_eq!(query, "op");
        }
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { query, .. }) = &app.state {
            assert_eq!(query, "o");
        }
    }

    #[test]
    fn login_browser_provider_j_k_navigation() {
        let mut app = make_app();
        app.open_login_browser(None);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { cursor, .. }) = &app.state {
            assert_eq!(*cursor, 1);
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { cursor, .. }) = &app.state {
            assert_eq!(*cursor, 0);
        }
    }

    #[test]
    fn login_browser_provider_page_navigation() {
        let mut app = make_app();
        app.open_login_browser(None);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { cursor, .. }) = &app.state {
            assert!(*cursor > 0);
        }
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { cursor, .. }) = &app.state {
            assert_eq!(*cursor, 0);
        }
    }

    #[test]
    fn login_browser_ctrl_c_closes_from_provider() {
        let mut app = make_app();
        app.open_login_browser(None);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.state, AppState::Idle);
        assert!(
            matches!(app.chat.messages.last(), Some(TuiMessage::Assistant(t)) if t.contains("cancelled"))
        );
    }

    #[test]
    fn login_browser_esc_from_provider_closes() {
        let mut app = make_app();
        app.open_login_browser(None);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn login_browser_enter_no_match_shows_error() {
        let mut app = make_app();
        app.open_login_browser(Some("zzz_nonexistent_provider".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { last_error, .. }) = &app.state {
            assert!(last_error.is_some());
        } else {
            panic!("expected LoginBrowsing with error");
        }
    }

    #[test]
    fn login_browser_method_step_esc_goes_back() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::SelectMethod,
                ..
            })
        ));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::SelectProvider,
                ..
            })
        ));
    }

    #[test]
    fn login_browser_method_j_k_navigation() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { cursor, .. }) = &app.state {
            assert_eq!(*cursor, 1);
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { cursor, .. }) = &app.state {
            assert_eq!(*cursor, 0);
        }
    }

    #[test]
    fn login_browser_method_ctrl_c_closes() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn login_browser_oauth_triggers_intent() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(app.state, AppState::AuthValidating { .. }));
        let intent = app.take_pending_auth_intent();
        assert!(intent.is_some());
        assert_eq!(intent.unwrap().auth_method, AuthMethodChoice::OAuth);
    }

    #[test]
    fn login_browser_api_key_type_and_submit() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for c in "sk-test123".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        if let AppState::LoginBrowsing(LoginBrowsingState {
            masked_buffer,
            input_buffer,
            ..
        }) = &app.state
        {
            assert_eq!(input_buffer, "sk-test123");
            assert_eq!(masked_buffer.len(), 10);
            assert!(masked_buffer.chars().all(|c| c == '*'));
        } else {
            panic!("expected LoginBrowsing at InputApiKey");
        }
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState {
            input_buffer,
            masked_buffer,
            ..
        }) = &app.state
        {
            assert_eq!(input_buffer, "sk-test12");
            assert_eq!(masked_buffer.len(), 9);
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(app.state, AppState::AuthValidating { .. }));
        let intent = app.take_pending_auth_intent();
        assert!(intent.is_some());
        assert_eq!(intent.unwrap().api_key.as_deref(), Some("sk-test12"));
    }

    #[test]
    fn login_browser_api_key_empty_shows_error() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { last_error, .. }) = &app.state {
            assert!(last_error.as_deref().unwrap_or("").contains("required"));
        }
    }

    #[test]
    fn login_browser_api_key_esc_goes_back() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::InputApiKey,
                ..
            })
        ));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::SelectMethod,
                ..
            })
        ));
    }

    #[test]
    fn login_browser_api_key_ctrl_c_closes() {
        let mut app = make_app();
        app.open_login_browser(Some("openai".to_string()));
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn render_login_browser_with_error_no_panic() {
        let mut app = make_app();
        app.reopen_provider_selection_with_error("Something went wrong".to_string());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        if let AppState::LoginBrowsing(LoginBrowsingState { last_error, .. }) = &app.state {
            assert_eq!(last_error.as_deref(), Some("Something went wrong"));
        }
    }

    #[test]
    fn render_login_browser_endpoint_step_no_panic() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: "custom".to_string(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputEndpoint,
            selected_provider: Some("custom".to_string()),
            selected_method: None,
            input_buffer: "https://api.example.com".to_string(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_login_browser_api_key_with_endpoint_no_panic() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: "custom".to_string(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputApiKey,
            selected_provider: Some("custom".to_string()),
            selected_method: Some(AuthMethodChoice::ApiKey),
            input_buffer: "sk-test".to_string(),
            masked_buffer: "*******".to_string(),
            last_error: None,
            endpoint: Some("https://api.example.com".to_string()),
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_login_browser_oauth_code_step_no_panic() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: "openai".to_string(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputApiKey,
            selected_provider: Some("openai".to_string()),
            selected_method: Some(AuthMethodChoice::OAuth),
            input_buffer: "auth-code-123".to_string(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn login_browser_endpoint_type_backspace_submit() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputEndpoint,
            selected_provider: Some("custom".to_string()),
            selected_method: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for c in "https://test.com".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { input_buffer, .. }) = &app.state {
            assert_eq!(input_buffer, "https://test.co");
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::InputApiKey,
                ..
            })
        ));
    }

    #[test]
    fn login_browser_endpoint_empty_shows_error() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputEndpoint,
            selected_provider: Some("custom".to_string()),
            selected_method: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        if let AppState::LoginBrowsing(LoginBrowsingState { last_error, .. }) = &app.state {
            assert!(last_error.as_deref().unwrap_or("").contains("required"));
        }
    }

    #[test]
    fn login_browser_endpoint_esc_goes_back() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputEndpoint,
            selected_provider: Some("custom".to_string()),
            selected_method: None,
            input_buffer: "test".to_string(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::SelectProvider,
                ..
            })
        ));
    }

    #[test]
    fn login_browser_endpoint_ctrl_c_closes() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing(LoginBrowsingState {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            step: LoginBrowseStep::InputEndpoint,
            selected_provider: Some("custom".to_string()),
            selected_method: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: None,
            endpoint: None,
        });
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.state, AppState::Idle);
    }

    // ── Render tests for Chat screen states ──────────────────

    #[test]
    fn render_chat_screen_with_messages() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.push_user("hello".into());
        app.push_assistant("world".into());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_chat_screen_with_sidebar() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = true;
        app.session.session_list = vec![
            make_session_entry("s1", "session one"),
            make_session_entry("s2", "session two"),
        ];
        app.push_user("test".into());
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_chat_screen_without_sidebar() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_chat_with_confirm_delete_overlay() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.session.session_list = vec![make_session_entry("s1", "session one")];
        app.sidebar.hover = Some(0);
        app.request_delete_session();
        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_home_screen() {
        let mut app = make_app();
        app.screen = Screen::Home;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn render_thinking_state() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.state = AppState::Thinking { round: 2 };
        app.spinner_tick = 5;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_auth_prompting_state() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.state = AppState::AuthPrompting {
            provider: "together".to_string(),
            env_name: "TOGETHER_API_KEY".to_string(),
            endpoint: None,
            endpoint_env: None,
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_auth_prompting_with_endpoint() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.state = AppState::AuthPrompting {
            provider: "custom".to_string(),
            env_name: "CUSTOM_API_KEY".to_string(),
            endpoint: Some("https://api.example.com".to_string()),
            endpoint_env: Some("CUSTOM_ENDPOINT".to_string()),
        };
        app.chat.input = "sk-secret".to_string();
        app.chat.cursor_pos = 9;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_auth_validating_state() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.state = AppState::AuthValidating {
            provider: "openai".to_string(),
        };
        app.spinner_tick = 2;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_executing_tool_state() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.state = AppState::ExecutingTool {
            tool_name: "system.run".to_string(),
        };
        app.spinner_tick = 4;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    // ── Command palette render tests ──────────────────

    #[test]
    fn render_command_palette() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.chat.input = "/".to_string();
        app.chat.cursor_pos = 1;
        assert!(app.is_palette_active());
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn command_palette_tab_auto_completes() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.chat.input, "/help");
    }

    #[test]
    fn tab_toggles_sidebar_when_not_palette() {
        let mut app = make_app();
        app.sidebar.visible = true;
        app.sidebar.focused = false;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(app.sidebar.focused);
    }

    #[test]
    fn enter_on_home_screen_switches_to_chat() {
        let mut app = make_app();
        app.screen = Screen::Home;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Chat);
    }

    // ── Auth flow tests ────────────────────────────────

    #[test]
    fn complete_auth_validation_success() {
        let mut app = make_app();
        app.state = AppState::AuthValidating {
            provider: "openai".to_string(),
        };
        app.complete_auth_validation("openai".to_string(), "OPENAI_API_KEY".to_string(), Ok(()));
        assert_eq!(app.state, AppState::Idle);
        assert!(
            matches!(app.chat.messages.last(), Some(TuiMessage::Assistant(t)) if t.contains("Saved"))
        );
    }

    #[test]
    fn complete_auth_validation_failure() {
        let mut app = make_app();
        app.state = AppState::AuthValidating {
            provider: "openai".to_string(),
        };
        app.complete_auth_validation(
            "openai".to_string(),
            "OPENAI_API_KEY".to_string(),
            Err("invalid key".to_string()),
        );
        assert_eq!(app.state, AppState::Idle);
        assert!(
            matches!(app.chat.messages.last(), Some(TuiMessage::Error(t)) if t.contains("Failed"))
        );
    }

    #[test]
    fn reopen_method_selector_with_error_sets_state() {
        let mut app = make_app();
        app.reopen_method_selector_with_error("openai", "something failed".to_string());
        if let AppState::LoginBrowsing(LoginBrowsingState {
            step,
            last_error,
            selected_provider,
            ..
        }) = &app.state
        {
            assert_eq!(*step, LoginBrowseStep::SelectMethod);
            assert_eq!(last_error.as_deref(), Some("something failed"));
            assert_eq!(selected_provider.as_deref(), Some("openai"));
        } else {
            panic!("expected LoginBrowsing");
        }
    }

    #[test]
    fn reopen_openai_method_with_error_delegates() {
        let mut app = make_app();
        app.reopen_openai_method_with_error("test error".to_string());
        assert!(matches!(
            app.state,
            AppState::LoginBrowsing(LoginBrowsingState {
                step: LoginBrowseStep::SelectMethod,
                ..
            })
        ));
    }

    #[test]
    fn take_auth_submission_returns_none_when_idle() {
        let mut app = make_app();
        assert!(app.take_auth_submission().is_none());
    }

    #[test]
    fn take_auth_submission_returns_none_when_input_empty() {
        let mut app = make_app();
        app.state = AppState::AuthPrompting {
            provider: "openai".to_string(),
            env_name: "OPENAI_API_KEY".to_string(),
            endpoint: None,
            endpoint_env: None,
        };
        app.chat.input = "   ".to_string();
        assert!(app.take_auth_submission().is_none());
    }

    #[test]
    fn take_pending_auth_intent_consume() {
        let mut app = make_app();
        app.model.pending_auth_intent = Some(AuthLoginIntent {
            provider: "openai".to_string(),
            auth_method: AuthMethodChoice::OAuth,
            endpoint: None,
            api_key: None,
        });
        assert!(app.take_pending_auth_intent().is_some());
        assert!(app.take_pending_auth_intent().is_none());
    }

    #[test]
    fn take_pending_model_change_consume() {
        let mut app = make_app();
        app.model.pending_model_change = Some(("gpt-4o".to_string(), "openai".to_string()));
        let change = app.take_pending_model_change();
        assert_eq!(change, Some(("gpt-4o".to_string(), "openai".to_string())));
        assert!(app.take_pending_model_change().is_none());
    }

    #[test]
    fn compute_sidebar_area_none_when_hidden() {
        let mut app = make_app();
        app.sidebar.visible = false;
        assert!(app.compute_sidebar_area(Rect::new(0, 0, 120, 40)).is_none());
    }

    #[test]
    fn compute_sidebar_area_none_on_home() {
        let mut app = make_app();
        app.screen = Screen::Home;
        app.sidebar.visible = true;
        assert!(app.compute_sidebar_area(Rect::new(0, 0, 120, 40)).is_none());
    }

    #[test]
    fn compute_sidebar_area_some_on_chat() {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = true;
        let sidebar = app.compute_sidebar_area(Rect::new(0, 0, 120, 40));
        assert!(sidebar.is_some());
        assert!(sidebar.unwrap().width > 0);
    }

    #[test]
    fn conversation_count_counts_user_and_assistant() {
        let mut app = make_app();
        app.push_user("hi".into());
        app.push_assistant("hello".into());
        app.chat.messages.push(TuiMessage::ToolCall {
            tool_name: "t".into(),
            args_preview: "{}".into(),
            done: false,
        });
        app.push_error("err".into());
        assert_eq!(app.conversation_count(), 2);
    }

    #[test]
    fn set_pending_sidebar_selection_works() {
        let mut app = make_app();
        app.set_pending_sidebar_selection(SessionId::from("test-sid"));
        assert_eq!(
            app.take_pending_sidebar_selection()
                .as_ref()
                .map(|s| s.as_str()),
            Some("test-sid")
        );
    }

    #[test]
    fn session_browser_up_down_arrows() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "a"),
            make_session_entry("s2", "b"),
            make_session_entry("s3", "c"),
        ];
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        if let AppState::SessionBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 1);
        }
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        if let AppState::SessionBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 0);
        }
    }

    #[test]
    fn session_browser_page_up_down() {
        let mut app = make_app();
        for i in 0..20 {
            app.session.session_list.push(make_session_entry(
                &format!("s{i}"),
                &format!("session {i}"),
            ));
        }
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        if let AppState::SessionBrowsing { cursor, .. } = &app.state {
            assert!(*cursor > 0);
        }
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        if let AppState::SessionBrowsing { cursor, .. } = &app.state {
            assert_eq!(*cursor, 0);
        }
    }

    #[test]
    fn session_browser_search_backspace() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        if let AppState::SessionBrowsing { query, .. } = &app.state {
            assert_eq!(query, "hi");
        }
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        if let AppState::SessionBrowsing { query, .. } = &app.state {
            assert_eq!(query, "h");
        }
    }

    #[test]
    fn session_browser_enter_with_empty_sessions() {
        let mut app = make_app();
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
        assert!(app.take_pending_sidebar_selection().is_none());
    }

    #[test]
    fn session_browser_d_with_empty_sessions() {
        let mut app = make_app();
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(matches!(app.state, AppState::SessionBrowsing { .. }));
    }

    #[test]
    fn session_browser_delete_key_triggers_delete() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
    }

    #[test]
    fn render_session_browser_with_search_active() {
        let mut app = make_app();
        app.session.session_list = vec![
            make_session_entry("s1", "deploy script"),
            make_session_entry("s2", "test runner"),
        ];
        app.open_session_browser();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn render_session_browser_no_match_query() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.state = AppState::SessionBrowsing {
            query: "nonexistent_xyz".to_string(),
            cursor: 0,
            scroll: 0,
            search_active: true,
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn confirm_delete_enter_confirms() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.sidebar.hover = Some(0);
        app.request_delete_session();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
        assert_eq!(
            app.take_confirmed_delete().as_ref().map(|s| s.as_str()),
            Some("s1")
        );
    }

    #[test]
    fn confirm_delete_esc_cancels() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.sidebar.hover = Some(0);
        app.request_delete_session();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Idle);
        assert!(app.take_confirmed_delete().is_none());
    }

    #[test]
    fn confirm_delete_ignores_other_keys() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        app.sidebar.hover = Some(0);
        app.request_delete_session();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
    }

    #[test]
    fn sidebar_focused_esc_unfocuses() {
        let mut app = make_app();
        app.sidebar.visible = true;
        app.sidebar.focused = true;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn sidebar_focused_tab_unfocuses() {
        let mut app = make_app();
        app.sidebar.visible = true;
        app.sidebar.focused = true;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn sidebar_focused_delete_triggers_confirm() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "a")];
        app.sidebar.visible = true;
        app.sidebar.focused = true;
        app.sidebar.hover = Some(0);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(matches!(app.state, AppState::ConfirmDelete { .. }));
    }

    #[test]
    fn request_delete_session_with_empty_preview() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("s1", "")];
        app.sidebar.hover = Some(0);
        app.request_delete_session();
        if let AppState::ConfirmDelete {
            session_preview, ..
        } = &app.state
        {
            assert_eq!(session_preview, "(empty session)");
        }
    }

    #[test]
    fn request_delete_session_with_long_preview_truncates() {
        let mut app = make_app();
        let long_preview = "A".repeat(60);
        app.session.session_list = vec![make_session_entry("s1", &long_preview)];
        app.sidebar.hover = Some(0);
        app.request_delete_session();
        if let AppState::ConfirmDelete {
            session_preview, ..
        } = &app.state
        {
            assert!(session_preview.ends_with('\u{2026}'));
            assert!(session_preview.chars().count() <= 40);
        }
    }

    #[test]
    fn toggle_sidebar_focus_initializes_hover() {
        let mut app = make_app();
        app.sidebar.visible = true;
        app.sidebar.focused = false;
        app.sidebar.hover = None;
        app.session.session_list = vec![make_session_entry("s1", "a")];
        app.toggle_sidebar_focus();
        assert!(app.sidebar.focused);
        assert_eq!(app.sidebar.hover, Some(0));
    }

    #[test]
    fn load_session_messages_skips_system() {
        let mut app = make_app();
        let sid = SessionId::from("test");
        let sys = proto::AgentMessage::new(sid.clone(), proto::Role::System, "system prompt");
        app.load_session_messages(sid, vec![sys]);
        assert!(app.chat.messages.is_empty());
    }

    #[test]
    fn handle_slash_command_exit_sets_quit() {
        let mut app = make_app();
        assert!(app.handle_slash_command("/exit"));
        assert!(app.should_quit);
    }

    #[test]
    fn handle_slash_command_model_pushes_loading() {
        let mut app = make_app();
        assert!(app.handle_slash_command("/model"));
        assert!(
            matches!(app.chat.messages.last(), Some(TuiMessage::Assistant(t)) if t.contains("Loading"))
        );
    }

    #[test]
    fn visible_model_entries_filters_and_sorts() {
        let mut app = make_app();
        app.model.model_entries = sample_models();
        assert!(!app.visible_model_entries("").is_empty());
        let filtered = app.visible_model_entries("gpt");
        assert!(filtered.iter().all(|e| e.id.to_lowercase().contains("gpt")));
        assert!(app.visible_model_entries("nonexistent_xyz").is_empty());
    }

    #[test]
    fn apply_progress_tool_error_result() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command": "bad"}),
        });
        app.apply_progress(ProgressEvent::ToolCallFinished {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            output: "error: not found".into(),
            is_error: true,
        });
        assert!(matches!(
            &app.chat.messages[1],
            TuiMessage::ToolResult { is_error: true, .. }
        ));
    }

    #[test]
    fn apply_progress_long_args_truncated() {
        let mut app = make_app();
        let long_args = serde_json::json!({ "command": "x".repeat(200) });
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: long_args,
        });
        if let TuiMessage::ToolCall { args_preview, .. } = &app.chat.messages[0] {
            assert!(args_preview.ends_with('\u{2026}'));
            assert!(args_preview.len() <= 83);
        }
    }

    #[test]
    fn apply_progress_long_output_truncated() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({}),
        });
        app.apply_progress(ProgressEvent::ToolCallFinished {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            output: "x".repeat(200),
            is_error: false,
        });
        if let TuiMessage::ToolResult { output_preview, .. } = &app.chat.messages[1] {
            assert!(output_preview.ends_with('\u{2026}'));
            assert!(output_preview.len() <= 123);
        }
    }

    // ═══ Reactive TEA tests: update() ═══════════════════════════

    #[test]
    fn update_insert_char_idle() {
        let mut app = make_app();
        let cmd = app.update(Action::InsertChar('a'));
        assert!(matches!(cmd, Command::None));
        assert_eq!(app.chat.input, "a");
        assert_eq!(app.chat.cursor_pos, 1);
    }

    #[test]
    fn update_insert_char_non_idle_ignored() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        let cmd = app.update(Action::InsertChar('x'));
        assert!(matches!(cmd, Command::None));
        assert!(app.chat.input.is_empty());
    }

    #[test]
    fn update_delete_char() {
        let mut app = make_app();
        app.update(Action::InsertChar('h'));
        app.update(Action::InsertChar('i'));
        assert_eq!(app.chat.input, "hi");
        assert_eq!(app.chat.cursor_pos, 2);
        let cmd = app.update(Action::DeleteChar);
        assert!(matches!(cmd, Command::None));
        assert_eq!(app.chat.input, "h");
        assert_eq!(app.chat.cursor_pos, 1);
    }

    #[test]
    fn update_move_cursor_left_right() {
        let mut app = make_app();
        app.update(Action::InsertChar('a'));
        app.update(Action::InsertChar('b'));
        assert_eq!(app.chat.cursor_pos, 2);
        app.update(Action::MoveCursorLeft);
        assert_eq!(app.chat.cursor_pos, 1);
        app.update(Action::MoveCursorRight);
        assert_eq!(app.chat.cursor_pos, 2);
        // Left at 0 stays at 0
        app.update(Action::MoveCursorLeft);
        app.update(Action::MoveCursorLeft);
        let cmd = app.update(Action::MoveCursorLeft);
        assert!(matches!(cmd, Command::None));
        assert_eq!(app.chat.cursor_pos, 0);
    }

    #[test]
    fn update_submit_input_on_home_screen() {
        let mut app = make_app();
        app.screen = Screen::Home;
        app.update(Action::SubmitInput);
        assert_eq!(app.screen, Screen::Chat);
    }

    #[test]
    fn update_scroll_up_down() {
        let mut app = make_app();
        app.chat.history_scroll = 10;
        app.update(Action::ScrollUp(3));
        assert_eq!(app.chat.history_scroll, 7);
        app.update(Action::ScrollDown(5));
        assert_eq!(app.chat.history_scroll, 12);
        // ScrollUp saturates at 0
        app.update(Action::ScrollUp(100));
        assert_eq!(app.chat.history_scroll, 0);
    }

    #[test]
    fn update_switch_screen() {
        let mut app = make_app();
        assert_eq!(app.screen, Screen::Home);
        app.update(Action::SwitchScreen(Screen::Chat));
        assert_eq!(app.screen, Screen::Chat);
    }

    #[test]
    fn update_push_user_message() {
        let mut app = make_app();
        app.update(Action::PushUserMessage("hello".to_string()));
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::User(t) if t == "hello"));
    }

    #[test]
    fn update_push_assistant_message() {
        let mut app = make_app();
        app.update(Action::PushAssistantMessage("world".to_string()));
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Assistant(t) if t == "world"));
    }

    #[test]
    fn update_push_error() {
        let mut app = make_app();
        app.update(Action::PushError("oops".to_string()));
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Error(t) if t == "oops"));
    }

    #[test]
    fn update_apply_completion_ok_via_action() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.update(Action::ApplyCompletion(Ok("done".to_string())));
        assert_eq!(app.state, AppState::Idle);
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Assistant(t) if t == "done"));
    }

    #[test]
    fn update_apply_completion_err_via_action() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.update(Action::ApplyCompletion(Err("fail".to_string())));
        assert_eq!(app.state, AppState::Idle);
        assert_eq!(app.chat.messages.len(), 1);
        assert!(matches!(&app.chat.messages[0], TuiMessage::Error(t) if t == "fail"));
    }

    #[test]
    fn update_sidebar_hover() {
        let mut app = make_app();
        assert_eq!(app.sidebar.hover, None);
        app.update(Action::SidebarHover(Some(3)));
        assert_eq!(app.sidebar.hover, Some(3));
        app.update(Action::SidebarHover(None));
        assert_eq!(app.sidebar.hover, None);
    }

    #[test]
    fn update_sidebar_scroll() {
        let mut app = make_app();
        app.update(Action::SidebarScroll(5));
        assert_eq!(app.sidebar.scroll, 5);
        app.update(Action::SidebarScroll(-2));
        assert_eq!(app.sidebar.scroll, 3);
        // Saturates at 0
        app.update(Action::SidebarScroll(-100));
        assert_eq!(app.sidebar.scroll, 0);
    }

    #[test]
    fn update_confirm_delete_in_confirm_state() {
        let mut app = make_app();
        app.state = AppState::ConfirmDelete {
            session_id: "ses_test".to_string(),
            session_preview: "preview".to_string(),
        };
        app.update(Action::ConfirmDelete);
        assert!(app.session.confirmed_delete.is_some());
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn update_cancel_delete() {
        let mut app = make_app();
        app.state = AppState::ConfirmDelete {
            session_id: "ses_test".to_string(),
            session_preview: "preview".to_string(),
        };
        app.update(Action::CancelDelete);
        assert_eq!(app.state, AppState::Idle);
        assert!(app.session.confirmed_delete.is_none());
    }

    #[test]
    fn update_palette_move_up_down() {
        let mut app = make_app();
        app.command_palette_cursor = 5;
        app.update(Action::PaletteMoveUp);
        assert_eq!(app.command_palette_cursor, 4);
        // Saturates at 0
        app.command_palette_cursor = 0;
        app.update(Action::PaletteMoveUp);
        assert_eq!(app.command_palette_cursor, 0);
    }

    #[test]
    fn update_palette_close() {
        let mut app = make_app();
        app.chat.input = "/help".to_string();
        app.chat.cursor_pos = 5;
        app.command_palette_cursor = 2;
        app.update(Action::PaletteClose);
        assert!(app.chat.input.is_empty());
        assert_eq!(app.chat.cursor_pos, 0);
        assert_eq!(app.command_palette_cursor, 0);
    }

    #[test]
    fn update_tick() {
        let mut app = make_app();
        let before = app.spinner_tick;
        app.update(Action::Tick);
        assert_eq!(app.spinner_tick, before.wrapping_add(1));
    }

    #[test]
    fn update_quit() {
        let mut app = make_app();
        assert!(!app.should_quit);
        app.update(Action::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn update_resize_returns_none() {
        let mut app = make_app();
        let cmd = app.update(Action::Resize);
        assert!(matches!(cmd, Command::None));
    }

    #[test]
    fn update_set_model() {
        let mut app = make_app();
        app.update(Action::SetModel("claude-4".to_string()));
        assert_eq!(app.model.model_name, "claude-4");
    }

    #[test]
    fn update_set_provider_name() {
        let mut app = make_app();
        app.update(Action::SetProviderName("anthropic".to_string()));
        assert_eq!(app.model.provider_name, "anthropic");
    }

    #[test]
    fn update_new_session() {
        let mut app = make_app();
        let sid = SessionId::new();
        app.update(Action::NewSession(sid.clone()));
        assert_eq!(app.session.session_id, sid);
        assert!(
            app.chat
                .messages
                .iter()
                .any(|m| matches!(m, TuiMessage::Assistant(t) if t.contains("New session")))
        );
    }

    #[test]
    fn update_set_thinking() {
        let mut app = make_app();
        assert_eq!(app.state, AppState::Idle);
        app.update(Action::SetThinking);
        assert_eq!(app.state, AppState::Thinking { round: 0 });
    }

    #[test]
    fn update_text_selection_start() {
        let mut app = make_app();
        app.update(Action::TextSelectionStart { row: 5, col: 10 });
        assert_eq!(app.chat.text_selection.anchor, Some((5, 10)));
        assert_eq!(app.chat.text_selection.endpoint, Some((5, 10)));
        assert!(app.chat.text_selection.dragging);
    }

    #[test]
    fn update_text_selection_drag() {
        let mut app = make_app();
        app.chat.text_selection.dragging = true;
        app.update(Action::TextSelectionDrag { row: 7, col: 15 });
        assert_eq!(app.chat.text_selection.endpoint, Some((7, 15)));
    }

    #[test]
    fn update_text_selection_drag_not_dragging() {
        let mut app = make_app();
        assert!(!app.chat.text_selection.dragging);
        app.update(Action::TextSelectionDrag { row: 7, col: 15 });
        assert_eq!(app.chat.text_selection.endpoint, None);
    }

    #[test]
    fn update_text_selection_clear() {
        let mut app = make_app();
        app.chat.text_selection.anchor = Some((0, 0));
        app.chat.text_selection.endpoint = Some((5, 10));
        app.chat.text_selection.dragging = true;
        app.update(Action::TextSelectionClear);
        assert_eq!(app.chat.text_selection.anchor, None);
        assert_eq!(app.chat.text_selection.endpoint, None);
        assert!(!app.chat.text_selection.dragging);
    }

    // ═══ Reactive TEA tests: map_key_event() ════════════════════

    #[test]
    fn map_key_confirm_delete_y() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        app.state = AppState::ConfirmDelete {
            session_id: "s1".to_string(),
            session_preview: "p".to_string(),
        };
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::ConfirmDelete));
    }

    #[test]
    fn map_key_confirm_delete_esc() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        app.state = AppState::ConfirmDelete {
            session_id: "s1".to_string(),
            session_preview: "p".to_string(),
        };
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::CancelDelete));
    }

    #[test]
    fn map_key_idle_char() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let app = make_app();
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::InsertChar('z')));
    }

    #[test]
    fn map_key_idle_backspace() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let app = make_app();
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::DeleteChar));
    }

    #[test]
    fn map_key_idle_left_right() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let app = make_app();
        let left = app.map_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(left.len(), 1);
        assert!(matches!(left[0], Action::MoveCursorLeft));
        let right = app.map_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(right.len(), 1);
        assert!(matches!(right[0], Action::MoveCursorRight));
    }

    #[test]
    fn map_key_ctrl_c_quits_in_idle() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let app = make_app();
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::Quit));
    }

    #[test]
    fn map_key_esc_quits_in_idle() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let app = make_app();
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::Quit));
    }

    #[test]
    fn map_key_scroll_up_down() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        // Up arrow maps to ScrollUp(1) when not in palette and not input-active for nav
        // In Idle state, Up maps to ScrollUp since palette is not active
        app.state = AppState::Thinking { round: 0 };
        let up = app.map_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(up.len(), 1);
        assert!(matches!(up[0], Action::ScrollUp(1)));
        let down = app.map_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(down.len(), 1);
        assert!(matches!(down[0], Action::ScrollDown(1)));
    }

    #[test]
    fn map_key_page_up_down() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        let pup = app.map_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(pup.len(), 1);
        assert!(matches!(pup[0], Action::ScrollUp(10)));
        let pdown = app.map_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(pdown.len(), 1);
        assert!(matches!(pdown[0], Action::ScrollDown(10)));
    }

    #[test]
    fn map_key_enter_idle_returns_submit() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let app = make_app();
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SubmitInput));
    }

    #[test]
    fn map_key_thinking_non_enter_ignored() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        // Char keys are not input-active in Thinking state
        let actions = app.map_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(actions.is_empty());
    }

    // ── map_mouse_event tests ─────────────────────────────────

    fn mouse_event(
        kind: crossterm::event::MouseEventKind,
        col: u16,
        row: u16,
    ) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind,
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    fn setup_sidebar_app() -> TuiApp {
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = true;
        app.session.session_list = vec![
            make_session_entry("s1", "hello"),
            make_session_entry("s2", "world"),
        ];
        app
    }

    #[test]
    fn mouse_left_click_sidebar_selects_session() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = setup_sidebar_app();
        let frame_area = Rect::new(0, 0, 120, 40);
        let sb = app.compute_sidebar_area(frame_area).unwrap();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), sb.x + 2, sb.y + 2),
            frame_area,
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::SidebarHover(Some(_))))
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::SelectSidebarSession))
        );
    }

    #[test]
    fn mouse_move_sidebar_sets_hover() {
        use crossterm::event::MouseEventKind;
        let app = setup_sidebar_app();
        let frame_area = Rect::new(0, 0, 120, 40);
        let sb = app.compute_sidebar_area(frame_area).unwrap();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Moved, sb.x + 2, sb.y + 2),
            frame_area,
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::SidebarHover(Some(_))))
        );
    }

    #[test]
    fn mouse_move_outside_sidebar_clears_hover() {
        use crossterm::event::MouseEventKind;
        let app = setup_sidebar_app();
        let frame_area = Rect::new(0, 0, 120, 40);
        let actions = app.map_mouse_event(mouse_event(MouseEventKind::Moved, 5, 5), frame_area);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SidebarHover(None)));
    }

    #[test]
    fn mouse_scroll_sidebar() {
        use crossterm::event::MouseEventKind;
        let app = setup_sidebar_app();
        let frame_area = Rect::new(0, 0, 120, 40);
        let sb = app.compute_sidebar_area(frame_area).unwrap();
        let down = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollDown, sb.x + 2, sb.y + 2),
            frame_area,
        );
        assert_eq!(down.len(), 1);
        assert!(matches!(down[0], Action::SidebarScroll(1)));
        let up = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollUp, sb.x + 2, sb.y + 2),
            frame_area,
        );
        assert_eq!(up.len(), 1);
        assert!(matches!(up[0], Action::SidebarScroll(-1)));
    }

    #[test]
    fn mouse_left_click_chat_starts_selection() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.chat.chat_area = Some(Rect::new(0, 0, 80, 30));
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 5),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::TextSelectionStart { .. }));
    }

    #[test]
    fn mouse_left_click_outside_chat_clears_selection() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        app.chat.chat_area = Some(Rect::new(10, 10, 60, 20));
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 5),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::TextSelectionClear));
    }

    #[test]
    fn mouse_drag_chat_when_dragging() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        app.chat.chat_area = Some(Rect::new(0, 0, 80, 30));
        app.chat.text_selection.dragging = true;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::TextSelectionDrag { .. }));
    }

    #[test]
    fn mouse_drag_chat_not_dragging_ignored() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        app.chat.chat_area = Some(Rect::new(0, 0, 80, 30));
        app.chat.text_selection.dragging = false;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 120, 40),
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_release_chat_when_dragging() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        app.chat.chat_area = Some(Rect::new(0, 0, 80, 30));
        app.chat.text_selection.dragging = true;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Up(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::TextSelectionEnd { .. }));
    }

    #[test]
    fn mouse_scroll_chat() {
        use crossterm::event::MouseEventKind;
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        app.chat.chat_area = Some(Rect::new(0, 0, 80, 30));
        let down = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollDown, 10, 10),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(down.len(), 2);
        assert!(matches!(down[0], Action::ScrollDown(3)));
        assert!(matches!(down[1], Action::TextSelectionClear));
        let up = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollUp, 10, 10),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(up.len(), 2);
        assert!(matches!(up[0], Action::ScrollUp(3)));
    }

    #[test]
    fn mouse_no_sidebar_no_chat_returns_empty() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.screen = Screen::Chat;
        app.sidebar.visible = false;
        app.chat.chat_area = None;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 120, 40),
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_hover_sidebar_out_of_range_clears() {
        use crossterm::event::MouseEventKind;
        let mut app = setup_sidebar_app();
        app.session.session_list = vec![make_session_entry("s1", "hello")];
        let frame_area = Rect::new(0, 0, 120, 40);
        let sb = app.compute_sidebar_area(frame_area).unwrap();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Moved, sb.x + 2, sb.y + 38),
            frame_area,
        );
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::SidebarHover(None)))
        );
    }

    // ── update() action variant tests ─────────────────────────

    #[test]
    fn update_toggle_sidebar_focus() {
        let mut app = make_app();
        app.sidebar.visible = true;
        app.screen = Screen::Chat;
        app.session.session_list = vec![make_session_entry("s1", "hi")];
        assert!(!app.sidebar.focused);
        app.update(Action::ToggleSidebarFocus);
        assert!(app.sidebar.focused);
        app.update(Action::ToggleSidebarFocus);
        assert!(!app.sidebar.focused);
    }

    #[test]
    fn update_close_model_browser() {
        let mut app = make_app();
        app.open_model_browser("openai".into(), sample_models(), String::new(), "ok".into());
        assert!(matches!(app.state, AppState::ModelBrowsing { .. }));
        app.update(Action::CloseModelBrowser);
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn update_close_session_browser() {
        let mut app = make_app();
        app.open_session_browser();
        assert!(matches!(app.state, AppState::SessionBrowsing { .. }));
        app.update(Action::CloseSessionBrowser);
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn update_mark_model_refreshing() {
        let mut app = make_app();
        app.open_model_browser("openai".into(), sample_models(), String::new(), "ok".into());
        app.update(Action::MarkModelRefreshing);
        if let AppState::ModelBrowsing {
            last_sync_status, ..
        } = &app.state
        {
            assert!(last_sync_status.contains("efresh"));
        }
    }

    #[test]
    fn update_model_catalog() {
        let mut app = make_app();
        app.open_model_browser("openai".into(), Vec::new(), String::new(), "loading".into());
        let new_entries = sample_models();
        app.update(Action::UpdateModelCatalog {
            provider: "openai".into(),
            entries: new_entries.clone(),
            sync_status: "synced".into(),
        });
        if let AppState::ModelBrowsing {
            last_sync_status, ..
        } = &app.state
        {
            assert_eq!(app.model.model_entries.len(), new_entries.len());
            assert_eq!(last_sync_status, "synced");
        } else {
            panic!("expected ModelBrowsing state");
        }
    }

    #[test]
    fn update_remove_session() {
        let mut app = make_app();
        app.session.session_list =
            vec![make_session_entry("s1", "a"), make_session_entry("s2", "b")];
        app.update(Action::RemoveSession(SessionId::from("s1")));
        assert_eq!(app.session.session_list.len(), 1);
        assert_eq!(app.session.session_list[0].id.as_str(), "s2");
    }

    #[test]
    fn update_refresh_session_list() {
        let mut app = make_app();
        app.session.session_list = vec![make_session_entry("old", "x")];
        let new_list = vec![make_session_entry("n1", "a"), make_session_entry("n2", "b")];
        app.update(Action::RefreshSessionList(new_list));
        assert_eq!(app.session.session_list.len(), 2);
        assert_eq!(app.session.session_list[0].id.as_str(), "n1");
    }

    #[test]
    fn update_load_session() {
        use proto::AgentMessage;
        let mut app = make_app();
        let sid = SessionId::from("test-session");
        let msgs = vec![AgentMessage::new(sid.clone(), proto::Role::User, "hi")];
        app.update(Action::LoadSession {
            session_id: sid.clone(),
            messages: msgs,
        });
        assert_eq!(app.session.session_id, sid);
        assert!(!app.chat.messages.is_empty());
    }

    #[test]
    fn update_slash_command_clear() {
        let mut app = make_app();
        app.push_user("hello".into());
        app.push_assistant("world".into());
        assert!(!app.chat.messages.is_empty());
        app.update(Action::SlashCommand("/clear".into()));
        assert!(app.chat.messages.is_empty());
    }

    #[test]
    fn update_slash_command_quit() {
        let mut app = make_app();
        assert!(!app.should_quit);
        app.update(Action::SlashCommand("/quit".into()));
        assert!(app.should_quit);
    }

    #[test]
    fn update_palette_move_down() {
        let mut app = make_app();
        app.chat.input = "/".to_string();
        app.command_palette_cursor = 0;
        app.update(Action::PaletteMoveDown);
        assert!(app.command_palette_cursor >= 1 || app.palette_filtered_commands().len() <= 1);
    }

    #[test]
    fn update_palette_tab_complete() {
        let mut app = make_app();
        app.chat.input = "/".to_string();
        app.command_palette_cursor = 0;
        app.update(Action::PaletteTabComplete);
        assert!(app.chat.input.starts_with('/'));
        assert!(app.chat.input.len() > 1);
    }

    #[test]
    fn update_palette_select() {
        let mut app = make_app();
        app.chat.input = "/".to_string();
        app.command_palette_cursor = 0;
        app.update(Action::PaletteSelect);
    }

    #[test]
    fn update_open_login_browser() {
        let mut app = make_app();
        app.update(Action::OpenLoginBrowser(None));
        assert!(matches!(app.state, AppState::LoginBrowsing(_)));
    }

    #[test]
    fn update_cancel_auth() {
        let mut app = make_app();
        app.update(Action::OpenLoginBrowser(None));
        assert!(matches!(app.state, AppState::LoginBrowsing(_)));
        app.update(Action::CancelAuth);
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn update_text_selection_end_clears_dragging() {
        let mut app = make_app();
        app.update(Action::TextSelectionStart { row: 0, col: 0 });
        assert!(app.chat.text_selection.dragging);
        app.update(Action::TextSelectionEnd { row: 1, col: 5 });
        assert!(!app.chat.text_selection.dragging);
    }

    #[test]
    fn update_apply_progress_via_update() {
        let mut app = make_app();
        app.update(Action::ApplyProgress(
            proto::ProgressEvent::ToolCallStarted {
                call_id: "c1".into(),
                tool_name: "bash".into(),
                args: serde_json::json!({}),
            },
        ));
        assert!(!app.chat.messages.is_empty());
    }

    // ═══ Reactive TEA tests: map_mouse_event() ════════════════════

    fn make_sidebar_app() -> TuiApp {
        let mut app = make_app();
        app.sidebar.visible = true;
        app.screen = Screen::Chat;
        app.session.session_list = vec![
            SessionEntry {
                id: SessionId::from("s1".to_string()),
                channel_id: "cli:tui".to_string(),
                updated_at: chrono::Utc::now(),
                preview: "Hello world".to_string(),
            },
            SessionEntry {
                id: SessionId::from("s2".to_string()),
                channel_id: "cli:tui".to_string(),
                updated_at: chrono::Utc::now(),
                preview: "Goodbye world".to_string(),
            },
        ];
        app
    }

    fn sidebar_frame() -> Rect {
        // 100 wide × 50 tall; sidebar_width()=30 → sidebar at x=70
        Rect::new(0, 0, 100, 50)
    }

    // ── Sidebar mouse tests ──────────────────────────────────

    #[test]
    fn mouse_sidebar_click_selects_first_session() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_sidebar_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 75, 2),
            sidebar_frame(),
        );
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::SidebarHover(Some(0))));
        assert!(matches!(actions[1], Action::SelectSidebarSession));
    }

    #[test]
    fn mouse_sidebar_click_selects_second_session() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_sidebar_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 75, 5),
            sidebar_frame(),
        );
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::SidebarHover(Some(1))));
        assert!(matches!(actions[1], Action::SelectSidebarSession));
    }

    #[test]
    fn mouse_sidebar_click_beyond_entries_returns_empty() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_sidebar_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 75, 8),
            sidebar_frame(),
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_sidebar_moved_hovers_session() {
        use crossterm::event::MouseEventKind;
        let app = make_sidebar_app();
        let actions =
            app.map_mouse_event(mouse_event(MouseEventKind::Moved, 75, 2), sidebar_frame());
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SidebarHover(Some(0))));
    }

    #[test]
    fn mouse_sidebar_moved_beyond_entries_clears_hover() {
        use crossterm::event::MouseEventKind;
        let app = make_sidebar_app();
        let actions =
            app.map_mouse_event(mouse_event(MouseEventKind::Moved, 75, 8), sidebar_frame());
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SidebarHover(None)));
    }

    #[test]
    fn mouse_sidebar_moved_outside_clears_hover() {
        use crossterm::event::MouseEventKind;
        let app = make_sidebar_app();
        let actions =
            app.map_mouse_event(mouse_event(MouseEventKind::Moved, 30, 5), sidebar_frame());
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SidebarHover(None)));
    }

    #[test]
    fn mouse_sidebar_scroll_down() {
        use crossterm::event::MouseEventKind;
        let app = make_sidebar_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollDown, 75, 5),
            sidebar_frame(),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SidebarScroll(1)));
    }

    #[test]
    fn mouse_sidebar_scroll_up() {
        use crossterm::event::MouseEventKind;
        let app = make_sidebar_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollUp, 75, 5),
            sidebar_frame(),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::SidebarScroll(-1)));
    }

    #[test]
    fn mouse_sidebar_hidden_returns_empty_for_click() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_sidebar_app();
        app.sidebar.visible = false;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 75, 5),
            sidebar_frame(),
        );
        assert!(actions.is_empty());
    }

    // ── Chat area mouse tests ───────────────────────────────

    fn make_chat_app() -> TuiApp {
        let mut app = make_app();
        app.sidebar.visible = false;
        app.screen = Screen::Chat;
        app.chat.chat_area = Some(Rect::new(0, 0, 80, 24));
        app
    }

    #[test]
    fn mouse_chat_left_down_in_inner_starts_selection() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_chat_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 5),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            Action::TextSelectionStart { row: 4, col: 4 }
        ));
    }

    #[test]
    fn mouse_chat_left_down_outside_inner_clears_selection() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_chat_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 0),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::TextSelectionClear));
    }

    #[test]
    fn mouse_chat_drag_while_dragging() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_chat_app();
        app.chat.text_selection.dragging = true;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            Action::TextSelectionDrag { row: 9, col: 9 }
        ));
    }

    #[test]
    fn mouse_chat_drag_not_dragging_is_empty() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_chat_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 80, 24),
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_chat_up_while_dragging() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_chat_app();
        app.chat.text_selection.dragging = true;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Up(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            Action::TextSelectionEnd { row: 9, col: 9 }
        ));
    }

    #[test]
    fn mouse_chat_up_not_dragging_is_empty() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let app = make_chat_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Up(MouseButton::Left), 10, 10),
            Rect::new(0, 0, 80, 24),
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_chat_scroll_down() {
        use crossterm::event::MouseEventKind;
        let app = make_chat_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollDown, 5, 5),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::ScrollDown(3)));
        assert!(matches!(actions[1], Action::TextSelectionClear));
    }

    #[test]
    fn mouse_chat_scroll_up() {
        use crossterm::event::MouseEventKind;
        let app = make_chat_app();
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollUp, 5, 5),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::ScrollUp(3)));
        assert!(matches!(actions[1], Action::TextSelectionClear));
    }

    #[test]
    fn mouse_chat_scroll_outside_chat_area_is_empty() {
        use crossterm::event::MouseEventKind;
        let mut app = make_chat_app();
        app.chat.chat_area = Some(Rect::new(10, 10, 40, 10));
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollDown, 5, 5),
            Rect::new(0, 0, 80, 24),
        );
        assert!(actions.is_empty());
    }

    // ── Edge cases ──────────────────────────────────────────

    #[test]
    fn mouse_no_sidebar_no_chat_click_returns_empty() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_app();
        app.sidebar.visible = false;
        app.chat.chat_area = None;
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Down(MouseButton::Left), 40, 20),
            Rect::new(0, 0, 100, 50),
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_sidebar_scroll_outside_sidebar_falls_through() {
        use crossterm::event::MouseEventKind;
        let mut app = make_sidebar_app();
        app.chat.chat_area = Some(Rect::new(0, 0, 70, 50));
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::ScrollDown, 30, 25),
            sidebar_frame(),
        );
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::ScrollDown(3)));
        assert!(matches!(actions[1], Action::TextSelectionClear));
    }

    #[test]
    fn mouse_chat_drag_clamps_to_inner_bounds() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = make_chat_app();
        app.chat.text_selection.dragging = true;
        // inner = Rect(1, 1, 78, 22); max col = 77, max row = 21
        let actions = app.map_mouse_event(
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 200, 100),
            Rect::new(0, 0, 80, 24),
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            Action::TextSelectionDrag { row: 21, col: 77 }
        ));
    }
}
