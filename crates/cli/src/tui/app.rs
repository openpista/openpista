//! TUI application state, rendering, and input handling.
#![allow(dead_code)]

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

/// Spinner animation frames (Braille pattern).
const SPINNER: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

// ─── Command palette ──────────────────────────────────────────

struct SlashCommand {
    name: &'static str,
    description: &'static str,
}

const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/model",
        description: "Browse & select model",
    },
    SlashCommand {
        name: "/model list",
        description: "Print available models to chat",
    },
    SlashCommand {
        name: "/login",
        description: "Change API credentials",
    },
    SlashCommand {
        name: "/connection",
        description: "Change credentials (alias)",
    },
    SlashCommand {
        name: "/clear",
        description: "Clear conversation history",
    },
    SlashCommand {
        name: "/help",
        description: "Show available commands",
    },
    SlashCommand {
        name: "/quit",
        description: "Exit TUI",
    },
    SlashCommand {
        name: "/exit",
        description: "Exit TUI",
    },
];

// ─── Data types ──────────────────────────────────────────────

/// A single rendered item in the conversation history panel.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TuiMessage {
    /// User typed this message.
    User(String),
    /// Assistant final response text.
    Assistant(String),
    /// An in-progress or completed tool call.
    ToolCall {
        tool_name: String,
        args_preview: String,
        done: bool,
    },
    /// A tool call that has completed with output.
    ToolResult {
        tool_name: String,
        output_preview: String,
        is_error: bool,
    },
    /// An error from the agent runtime.
    Error(String),
}

/// Determines which "view" is active
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Screen {
    #[default]
    /// The home/welcome screen shown before entering chat.
    Home,
    /// The active chat conversation screen.
    Chat,
}

/// High-level processing state.
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    /// No agent task running; input box is active.
    Idle,
    /// Waiting for LLM response (spinner shown).
    Thinking { round: usize },
    /// A tool call is executing.
    ExecutingTool { tool_name: String },
    /// Waiting for API key input for selected provider.
    AuthPrompting {
        /// Provider id selected from registry.
        provider: String,
        /// API key env variable name shown in prompt.
        env_name: String,
        /// Optional endpoint captured for endpoint+key providers.
        endpoint: Option<String>,
        /// Optional endpoint env hint shown to users.
        endpoint_env: Option<String>,
    },
    /// Running auth validation or OAuth callback flow.
    AuthValidating {
        /// Provider currently being validated.
        provider: String,
    },
    /// Searchable login browser for provider/method/key flow.
    LoginBrowsing {
        /// Provider list search query.
        query: String,
        /// Current cursor position.
        cursor: usize,
        /// Scroll offset.
        scroll: u16,
        /// Active browser step.
        step: LoginBrowseStep,
        /// Selected provider id.
        selected_provider: Option<String>,
        /// Selected auth method.
        selected_method: Option<AuthMethodChoice>,
        /// Raw input for endpoint/API key steps.
        input_buffer: String,
        /// Masked API-key display buffer.
        masked_buffer: String,
        /// Last error shown in browser.
        last_error: Option<String>,
        /// Endpoint captured from endpoint step.
        endpoint: Option<String>,
    },
    /// Browse model catalog in a dedicated TUI screen.
    ModelBrowsing {
        /// Case-insensitive substring query.
        query: String,
        /// Selected row index among visible model entries.
        cursor: usize,
        /// Scroll offset for model list.
        scroll: u16,
        /// Last sync/fallback status text.
        last_sync_status: String,
        /// Whether in-browser search mode is active.
        search_active: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Auth submission payload extracted from the TUI prompt.
pub struct AuthSubmission {
    /// Provider id.
    pub provider: String,
    /// API key env hint.
    pub env_name: String,
    /// Optional endpoint value.
    pub endpoint: Option<String>,
    /// Raw API key value entered by user.
    pub api_key: String,
}

/// A session entry displayed in the sidebar.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    /// Unique session identifier.
    pub id: SessionId,
    /// Channel identifier (e.g. `cli:tui`, `telegram:123`).
    pub channel_id: String,
    /// Timestamp of the most recent message in this session.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Short preview text of the last message for sidebar display.
    pub preview: String,
}

// ─── TuiApp ──────────────────────────────────────────────────

/// Full state for the TUI session.
pub struct TuiApp {
    /// Ordered conversation history for display.
    pub messages: Vec<TuiMessage>,
    /// Current text typed in the input box (not yet submitted).
    pub input: String,
    /// Cursor position within `input` (byte offset).
    pub cursor_pos: usize,
    /// Current high-level processing state.
    pub state: AppState,
    /// Which screen is currently displayed.
    pub screen: Screen,
    /// Workspace name for status bar.
    pub workspace_name: String,
    /// Git branch for status bar.
    pub branch_name: String,
    /// Available MCP servers for status bar.
    pub mcp_count: usize,
    /// Version text.
    pub version: String,
    /// Vertical scroll offset for the history panel.
    pub history_scroll: u16,
    /// Model name shown in the status bar.
    pub model_name: String,
    /// Session identifier.
    pub session_id: SessionId,
    /// Channel id for this TUI session.
    #[allow(dead_code)]
    pub channel_id: ChannelId,
    /// Spinner animation tick counter.
    pub spinner_tick: u8,
    /// Whether the user requested exit.
    pub should_quit: bool,
    /// Last loaded model catalog entries.
    pub model_entries: Vec<model_catalog::ModelCatalogEntry>,
    /// Provider backing the current model catalog.
    pub model_provider: String,
    /// Set when user pressed `r` inside model browser.
    model_refresh_requested: bool,
    /// Pending auth submission from login browser.
    pending_auth_intent: Option<AuthLoginIntent>,
    /// Selected row in the command palette popup.
    command_palette_cursor: usize,
    /// Provider name used for auth status check (e.g. "openai", "anthropic").
    pub provider_name: String,
    /// Set when the user selects a model in the model browser; consumed by the event loop. (model_id, provider_name)
    pending_model_change: Option<(String, String)>,
    /// Session list for sidebar display.
    pub session_list: Vec<SessionEntry>,
    /// Index of sidebar item under mouse hover.
    pub sidebar_hover: Option<usize>,
    /// Scroll offset for sidebar.
    pub sidebar_scroll: u16,
    /// Whether the sidebar is visible.
    pub sidebar_visible: bool,
    /// Current mouse text selection state.
    pub text_selection: super::selection::TextSelection,
    /// Bounding rect of the chat widget (set each render; used for mouse hit-testing).
    pub chat_area: Option<Rect>,
    /// Character grid mirroring the chat render layout (set each render; used for text extraction).
    pub chat_text_grid: Vec<Vec<char>>,
    /// Scroll value after clamping to max_scroll (set each render; used for text extraction).
    pub chat_scroll_clamped: u16,
}

impl TuiApp {
    /// Create a new TUI application state.
    pub fn new(
        model_name: impl Into<String>,
        session_id: SessionId,
        channel_id: ChannelId,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            state: AppState::Idle,
            screen: Screen::Home,
            workspace_name: "~/openpista".into(),
            branch_name: "main".into(),
            mcp_count: 0,
            version: env!("CARGO_PKG_VERSION").into(),
            history_scroll: 0,
            model_name: model_name.into(),
            session_id,
            channel_id,
            spinner_tick: 0,
            should_quit: false,
            model_entries: Vec::new(),
            model_provider: "openai".to_string(),
            model_refresh_requested: false,
            pending_auth_intent: None,
            command_palette_cursor: 0,
            provider_name: provider_name.into(),
            pending_model_change: None,
            session_list: Vec::new(),
            sidebar_hover: None,
            sidebar_scroll: 0,
            sidebar_visible: true,
            text_selection: super::selection::TextSelection::new(),
            chat_area: None,
            chat_text_grid: Vec::new(),
            chat_scroll_clamped: 0,
        }
    }

    /// Returns `true` if the configured provider has a valid (non-expired) stored credential.
    pub fn is_authenticated(&self) -> bool {
        let creds = crate::auth::Credentials::load();
        creds
            .get(&self.provider_name)
            .is_some_and(|c| !c.is_expired())
    }

    /// Takes the pending model change set by the model browser on selection.
    pub fn take_pending_model_change(&mut self) -> Option<(String, String)> {
        self.pending_model_change.take()
    }

    /// Returns the sidebar `Rect` for the given full-frame area, or `None` if the sidebar is hidden.
    pub fn compute_sidebar_area(&self, full_area: Rect) -> Option<Rect> {
        if !self.sidebar_visible || self.screen != Screen::Chat {
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
        self.state == AppState::Idle && self.input.starts_with('/')
    }

    fn palette_filtered_commands(&self) -> Vec<&'static SlashCommand> {
        let q = self.input.to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(q.as_str()))
            .collect()
    }

    /// Resolves the palette selection into `self.input` and closes the palette.
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
        self.input = name.clone();
        self.cursor_pos = self.input.len();
        self.command_palette_cursor = 0;
        self.screen = Screen::Chat;
        Some(name)
    }

    // ── State mutations ──────────────────────────────────────

    /// Push a user message to the history.
    pub fn push_user(&mut self, text: String) {
        self.messages.push(TuiMessage::User(text));
    }

    /// Push an assistant response to the history.
    pub fn push_assistant(&mut self, text: String) {
        self.messages.push(TuiMessage::Assistant(text));
    }

    /// Push an error message to the history.
    pub fn push_error(&mut self, err: String) {
        self.messages.push(TuiMessage::Error(err));
    }

    /// Take the current input and reset it.
    pub fn take_input(&mut self) -> String {
        self.cursor_pos = 0;
        std::mem::take(&mut self.input)
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
                self.messages.clear();
                self.history_scroll = 0;
            }
            "/help" => {
                self.push_assistant(
                    "TUI commands:\n/help - show this help\n/login - open credential picker\n/connection - open credential picker\n/model - browse model catalog (search with s, refresh with r)\n/model list - print available models to chat\n/clear - clear history\n/quit or /exit - leave TUI"
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

        if self.input.trim().is_empty() {
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
        self.input.clear();
        self.cursor_pos = 0;
        self.state = AppState::Idle;
        self.push_assistant("Login cancelled.".to_string());
    }

    /// Transitions to the `LoginBrowsing` state, optionally pre-filtering by `seed` provider name.
    pub fn open_login_browser(&mut self, seed: Option<String>) {
        self.input.clear();
        self.cursor_pos = 0;
        self.state = AppState::LoginBrowsing {
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
        };
    }

    /// Takes the pending `AuthLoginIntent` that was set during the login browser flow.
    pub fn take_pending_auth_intent(&mut self) -> Option<AuthLoginIntent> {
        self.pending_auth_intent.take()
    }

    /// Re-opens the openai method selector and displays `message` as an error.
    pub fn reopen_openai_method_with_error(&mut self, message: String) {
        self.reopen_method_selector_with_error("openai", message);
    }

    /// Re-opens the method-selector step for `provider`, showing `message` as the last error.
    pub fn reopen_method_selector_with_error(&mut self, provider: &str, message: String) {
        self.state = AppState::LoginBrowsing {
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
        };
    }

    /// Re-opens the provider-selection step, showing `message` as an error banner.
    pub fn reopen_provider_selection_with_error(&mut self, message: String) {
        self.state = AppState::LoginBrowsing {
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
        };
    }

    /// Opens/updates model browser with new catalog data.
    pub fn open_model_browser(
        &mut self,
        provider: String,
        entries: Vec<model_catalog::ModelCatalogEntry>,
        query: String,
        sync_status: String,
    ) {
        self.model_provider = provider;
        self.model_entries = entries;
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
        self.model_provider = provider;
        self.model_entries = entries;
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
        let requested = self.model_refresh_requested;
        self.model_refresh_requested = false;
        requested
    }

    fn visible_model_entries(&self, query: &str) -> Vec<model_catalog::ModelCatalogEntry> {
        let recommended = model_catalog::filtered_entries(&self.model_entries, query, false);
        let all_models = model_catalog::filtered_entries(&self.model_entries, query, true);
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

    fn visible_login_provider_entries(
        &self,
        query: &str,
    ) -> Vec<crate::config::ProviderRegistryEntry> {
        auth_picker::filtered_provider_entries(query)
    }

    fn clamp_login_cursor(&mut self) {
        if let AppState::LoginBrowsing {
            query,
            cursor,
            scroll,
            step,
            ..
        } = &mut self.state
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
                self.messages.push(TuiMessage::ToolCall {
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
                for msg in self.messages.iter_mut().rev() {
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
                self.messages.push(TuiMessage::ToolResult {
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

    // ── Input handling ───────────────────────────────────────

    /// Handle a keyboard event.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        let login_browsing = matches!(self.state, AppState::LoginBrowsing { .. });
        if login_browsing {
            let mut should_clamp = false;
            let mut pending_intent: Option<AuthLoginIntent> = None;
            let mut close_browser = false;

            if let AppState::LoginBrowsing {
                query,
                cursor,
                step,
                selected_provider,
                selected_method,
                input_buffer,
                masked_buffer,
                last_error,
                endpoint,
                ..
            } = &mut self.state
            {
                match step {
                    LoginBrowseStep::SelectProvider => {
                        let providers = auth_picker::filtered_provider_entries(query);
                        match (key.modifiers, key.code) {
                            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                                close_browser = true;
                            }
                            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                                *cursor = cursor.saturating_sub(1);
                                should_clamp = true;
                            }
                            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                                *cursor = cursor.saturating_add(1);
                                should_clamp = true;
                            }
                            (_, KeyCode::PageUp) => {
                                *cursor = cursor.saturating_sub(10);
                                should_clamp = true;
                            }
                            (_, KeyCode::PageDown) => {
                                *cursor = cursor.saturating_add(10);
                                should_clamp = true;
                            }
                            (_, KeyCode::Backspace) => {
                                query.pop();
                                *cursor = 0;
                                should_clamp = true;
                            }
                            (_, KeyCode::Char(c)) => {
                                query.push(c);
                                *cursor = 0;
                                should_clamp = true;
                            }
                            (_, KeyCode::Enter) => {
                                if providers.is_empty() {
                                    *last_error = Some(format!("No matches for '{}'.", query));
                                } else if let Some(selected) = providers.get(*cursor).copied() {
                                    *selected_provider = Some(selected.name.to_string());
                                    *selected_method = None;
                                    input_buffer.clear();
                                    masked_buffer.clear();
                                    *endpoint = None;
                                    *last_error = None;
                                    *cursor = 0;
                                    match selected.auth_mode {
                                        LoginAuthMode::None => {
                                            *last_error = Some(format!(
                                                "Provider '{}' does not require login.",
                                                selected.display_name
                                            ));
                                        }
                                        LoginAuthMode::OAuth => {
                                            if selected.name == "openai"
                                                || selected.name == "anthropic"
                                            {
                                                *step = LoginBrowseStep::SelectMethod;
                                            } else {
                                                pending_intent = Some(AuthLoginIntent {
                                                    provider: selected.name.to_string(),
                                                    auth_method: AuthMethodChoice::OAuth,
                                                    endpoint: None,
                                                    api_key: None,
                                                });
                                            }
                                        }
                                        LoginAuthMode::ApiKey => {
                                            *step = LoginBrowseStep::InputApiKey;
                                        }
                                        LoginAuthMode::EndpointAndKey => {
                                            *step = LoginBrowseStep::InputEndpoint;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    LoginBrowseStep::SelectMethod => match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                        (_, KeyCode::Esc) => {
                            *step = LoginBrowseStep::SelectProvider;
                            *cursor = 0;
                            should_clamp = true;
                        }
                        (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                            *cursor = cursor.saturating_sub(1);
                            should_clamp = true;
                        }
                        (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                            *cursor = cursor.saturating_add(1);
                            should_clamp = true;
                        }
                        (_, KeyCode::Enter) => {
                            let provider = selected_provider.clone().unwrap_or_default();
                            if provider.is_empty() {
                                *step = LoginBrowseStep::SelectProvider;
                                *last_error = Some(
                                    "Provider selection was cleared. Select provider again."
                                        .to_string(),
                                );
                            } else if *cursor == 0 {
                                *selected_method = Some(AuthMethodChoice::OAuth);
                                pending_intent = Some(AuthLoginIntent {
                                    provider,
                                    auth_method: AuthMethodChoice::OAuth,
                                    endpoint: None,
                                    api_key: None,
                                });
                            } else {
                                *selected_method = Some(AuthMethodChoice::ApiKey);
                                input_buffer.clear();
                                masked_buffer.clear();
                                *step = LoginBrowseStep::InputApiKey;
                            }
                        }
                        _ => {}
                    },
                    LoginBrowseStep::InputEndpoint => match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                        (_, KeyCode::Esc) => {
                            *step = LoginBrowseStep::SelectProvider;
                            *cursor = 0;
                            input_buffer.clear();
                        }
                        (_, KeyCode::Backspace) => {
                            input_buffer.pop();
                        }
                        (_, KeyCode::Enter) => {
                            let value = input_buffer.trim().to_string();
                            if value.is_empty() {
                                *last_error = Some("Endpoint is required.".to_string());
                            } else {
                                *endpoint = Some(value);
                                input_buffer.clear();
                                *step = LoginBrowseStep::InputApiKey;
                                *last_error = None;
                            }
                        }
                        (_, KeyCode::Char(c)) => {
                            input_buffer.push(c);
                        }
                        _ => {}
                    },
                    LoginBrowseStep::InputApiKey => match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                        (_, KeyCode::Esc) => {
                            let provider = selected_provider.clone().unwrap_or_default();
                            if let Some(entry) = auth_picker::provider_by_name(&provider) {
                                if matches!(
                                    auth_picker::provider_step_for_entry(&entry),
                                    LoginBrowseStep::SelectMethod
                                ) {
                                    *step = LoginBrowseStep::SelectMethod;
                                    *cursor =
                                        if matches!(selected_method, Some(AuthMethodChoice::OAuth))
                                        {
                                            0
                                        } else {
                                            1
                                        };
                                } else if matches!(entry.auth_mode, LoginAuthMode::EndpointAndKey) {
                                    *step = LoginBrowseStep::InputEndpoint;
                                    input_buffer.clear();
                                    if let Some(saved_endpoint) = endpoint.as_ref() {
                                        input_buffer.push_str(saved_endpoint);
                                    }
                                } else {
                                    *step = LoginBrowseStep::SelectProvider;
                                    *cursor = 0;
                                }
                            } else {
                                *step = LoginBrowseStep::SelectProvider;
                                *cursor = 0;
                            }
                            masked_buffer.clear();
                        }
                        (_, KeyCode::Backspace) => {
                            if input_buffer.pop().is_some() {
                                masked_buffer.pop();
                            }
                        }
                        (_, KeyCode::Enter) => {
                            let provider = selected_provider.clone().unwrap_or_default();
                            let api_key = input_buffer.trim().to_string();
                            if provider.is_empty() {
                                *last_error = Some(
                                    "Provider selection was cleared. Select provider again."
                                        .to_string(),
                                );
                                *step = LoginBrowseStep::SelectProvider;
                            } else if api_key.is_empty() {
                                *last_error = Some("API key is required.".to_string());
                            } else {
                                pending_intent = Some(AuthLoginIntent {
                                    provider: provider.clone(),
                                    auth_method: auth_picker::api_key_method_for_provider(
                                        &provider,
                                        *selected_method,
                                    ),
                                    endpoint: endpoint.clone(),
                                    api_key: Some(api_key),
                                });
                            }
                        }
                        (_, KeyCode::Char(c)) => {
                            input_buffer.push(c);
                            masked_buffer.push('*');
                        }
                        _ => {}
                    },
                }
            }

            if close_browser {
                self.state = AppState::Idle;
                self.push_assistant("Login cancelled.".to_string());
                return;
            }
            if should_clamp {
                self.clamp_login_cursor();
            }
            if let Some(intent) = pending_intent {
                self.pending_auth_intent = Some(intent.clone());
                self.state = AppState::AuthValidating {
                    provider: intent.provider,
                };
            }
            return;
        }

        let browsing = matches!(self.state, AppState::ModelBrowsing { .. });
        if browsing {
            let mut close_browser = false;
            let mut apply_selected = false;
            let mut should_clamp = false;

            if let AppState::ModelBrowsing {
                query,
                cursor,
                scroll,
                search_active,
                ..
            } = &mut self.state
            {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                    (_, KeyCode::Esc) => {
                        if *search_active {
                            *search_active = false;
                        } else {
                            close_browser = true;
                        }
                    }
                    (_, KeyCode::Char('s')) | (_, KeyCode::Char('/')) if !*search_active => {
                        *search_active = true
                    }
                    (_, KeyCode::Enter) if !*search_active => apply_selected = true,
                    (_, KeyCode::Char('r')) if !*search_active => {
                        self.model_refresh_requested = true;
                    }
                    (_, KeyCode::Char('j')) if !*search_active => {
                        *cursor = cursor.saturating_add(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Char('k')) if !*search_active => {
                        *cursor = cursor.saturating_sub(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Down) if !*search_active => {
                        *cursor = cursor.saturating_add(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Up) if !*search_active => {
                        *cursor = cursor.saturating_sub(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::PageDown) if !*search_active => {
                        *cursor = cursor.saturating_add(10);
                        should_clamp = true;
                    }
                    (_, KeyCode::PageUp) if !*search_active => {
                        *cursor = cursor.saturating_sub(10);
                        should_clamp = true;
                    }
                    (_, KeyCode::Backspace) if *search_active => {
                        query.pop();
                        *cursor = 0;
                        *scroll = 0;
                        should_clamp = true;
                    }
                    (_, KeyCode::Char(c)) if *search_active => {
                        query.push(c);
                        *cursor = 0;
                        *scroll = 0;
                        should_clamp = true;
                    }
                    _ => {}
                }
            }

            if close_browser {
                self.state = AppState::Idle;
                return;
            }

            if apply_selected {
                if let Some((query, cursor)) = match &self.state {
                    AppState::ModelBrowsing { query, cursor, .. } => Some((query.clone(), *cursor)),
                    _ => None,
                } {
                    let visible = self.visible_model_entries(&query);
                    if let Some(selected) = visible.get(cursor) {
                        self.model_name = selected.id.clone();
                        self.pending_model_change =
                            Some((selected.id.clone(), selected.provider.clone()));
                        self.push_assistant(format!(
                            "Selected model '{}' (provider: {}) for this session.",
                            selected.id, selected.provider
                        ));
                    }
                }
                self.state = AppState::Idle;
                return;
            }

            if should_clamp {
                self.clamp_model_cursor();
            }
            return;
        }

        let is_input_active = matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                if self.text_selection.is_active() {
                    // Copy selected text then dismiss selection; do NOT quit.
                    if let Some((start, end)) = self.text_selection.ordered_range() {
                        let grid = self.chat_text_grid.clone();
                        let scroll = self.chat_scroll_clamped;
                        if let Some(text) =
                            super::selection::extract_selected_text(&grid, start, end, scroll)
                        {
                            super::selection::copy_to_clipboard(&text);
                        }
                    }
                    self.text_selection.clear();
                } else if self.is_palette_active() {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.command_palette_cursor = 0;
                } else if self.state == AppState::Idle {
                    self.should_quit = true;
                } else if matches!(self.state, AppState::AuthPrompting { .. }) {
                    self.cancel_auth_prompt();
                }
            }
            (_, KeyCode::Tab) if self.is_palette_active() => {
                let cmd_name = self
                    .palette_filtered_commands()
                    .get(self.command_palette_cursor)
                    .map(|c| c.name.to_string());
                if let Some(name) = cmd_name {
                    self.input = name.clone();
                    self.cursor_pos = name.len();
                    self.command_palette_cursor = 0;
                }
            }
            (_, KeyCode::Enter) if self.state == AppState::Idle => {
                // If Enter is pressed, make sure we are heavily into the Chat screen
                if self.screen == Screen::Home {
                    self.screen = Screen::Chat;
                }
                // (The event loop will then extract `self.take_input()` when handling this)
            }
            (_, KeyCode::Char(c)) if is_input_active => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
                self.command_palette_cursor = 0;
            }
            (_, KeyCode::Backspace) if is_input_active => {
                if self.cursor_pos > 0 {
                    let prev = self.input[..self.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input.drain(prev..self.cursor_pos);
                    self.cursor_pos = prev;
                    self.command_palette_cursor = 0;
                }
            }
            (_, KeyCode::Left) if is_input_active => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = self.input[..self.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            (_, KeyCode::Right) if is_input_active => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = self.input[self.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor_pos + i)
                        .unwrap_or(self.input.len());
                }
            }
            (_, KeyCode::Up) if self.is_palette_active() => {
                self.command_palette_cursor = self.command_palette_cursor.saturating_sub(1);
            }
            (_, KeyCode::Down) if self.is_palette_active() => {
                let max = self.palette_filtered_commands().len().saturating_sub(1);
                self.command_palette_cursor = (self.command_palette_cursor + 1).min(max);
            }
            (_, KeyCode::Up) => {
                self.history_scroll = self.history_scroll.saturating_sub(1);
            }
            (_, KeyCode::Down) => {
                self.history_scroll = self.history_scroll.saturating_add(1);
            }
            (_, KeyCode::PageUp) => {
                self.history_scroll = self.history_scroll.saturating_sub(10);
            }
            (_, KeyCode::PageDown) => {
                self.history_scroll = self.history_scroll.saturating_add(10);
            }
            _ => {}
        }
    }

    // ── Rendering ────────────────────────────────────────────

    /// Render the entire TUI into the given frame.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();

        if matches!(self.state, AppState::LoginBrowsing { .. }) {
            self.render_login_browser(frame, area);
            return;
        }

        if matches!(self.state, AppState::ModelBrowsing { .. }) {
            self.render_model_browser(frame, area);
            return;
        }

        match self.screen {
            Screen::Home => {
                // Layout for home: content(fill) | status(1)
                let chunks =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

                crate::tui::home::render(self, frame, chunks[0]);
                crate::tui::status::render(self, frame, chunks[1]);
            }
            Screen::Chat => {
                let sidebar_w = if self.sidebar_visible {
                    crate::tui::sidebar::sidebar_width()
                } else {
                    0
                };
                let h_chunks =
                    Layout::horizontal([Constraint::Min(0), Constraint::Length(sidebar_w)])
                        .split(area);

                let main_area = h_chunks[0];
                let sidebar_area = h_chunks[1];

                let chunks = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(3),
                ])
                .split(main_area);

                self.render_title(frame, chunks[0]);
                crate::tui::chat::render(self, frame, chunks[1]);
                crate::tui::status::render(self, frame, chunks[2]);
                self.render_input(frame, chunks[3]);

                if self.sidebar_visible {
                    crate::tui::sidebar::render(self, frame, sidebar_area);
                }
            }
        }
    }

    fn render_login_browser(&self, frame: &mut Frame<'_>, area: Rect) {
        let AppState::LoginBrowsing {
            query,
            cursor,
            scroll,
            step,
            selected_provider,
            selected_method,
            input_buffer,
            masked_buffer,
            last_error,
            endpoint,
        } = &self.state
        else {
            return;
        };

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Add credential ",
                    Style::default()
                        .fg(THEME.browser_title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " /login or /connection ",
                    Style::default().fg(THEME.fg_muted),
                ),
            ])),
            chunks[0],
        );

        let mut lines: Vec<Line<'_>> = Vec::new();
        match step {
            LoginBrowseStep::SelectProvider => {
                lines.push(Line::from(Span::styled(
                    " Select provider ",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" Search: {}", query),
                    Style::default().fg(THEME.browser_search),
                )));

                let providers = self.visible_login_provider_entries(query);
                let creds = crate::auth::Credentials::load();
                if providers.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(" No matches for '{}'.", query),
                        Style::default().fg(THEME.warning),
                    )));
                } else {
                    for (idx, entry) in providers.iter().enumerate() {
                        let selected = idx == *cursor;
                        let marker = if selected { "●" } else { "○" };
                        let is_authed = creds.get(entry.name).is_some_and(|c| !c.is_expired());
                        let mut spans = vec![
                            Span::styled(
                                format!(" {} ", marker),
                                if selected {
                                    Style::default()
                                        .fg(THEME.browser_selected_marker)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(THEME.fg_muted)
                                },
                            ),
                            Span::styled(
                                entry.display_name,
                                if selected {
                                    Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(THEME.fg)
                                },
                            ),
                        ];
                        if is_authed {
                            spans.push(Span::styled(
                                " ●",
                                Style::default().fg(THEME.palette_auth_dot),
                            ));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
            LoginBrowseStep::SelectMethod => {
                lines.push(Line::from(Span::styled(
                    " Select auth method ",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " Provider: {}",
                        selected_provider.as_deref().unwrap_or("openai")
                    ),
                    Style::default().fg(THEME.fg_muted),
                )));
                let methods = [AuthMethodChoice::OAuth, AuthMethodChoice::ApiKey];
                for (idx, method) in methods.iter().enumerate() {
                    let selected = idx == *cursor;
                    lines.push(Line::from(vec![
                        Span::styled(
                            if selected { " ● " } else { " ○ " },
                            if selected {
                                Style::default()
                                    .fg(THEME.browser_selected_marker)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(THEME.fg_muted)
                            },
                        ),
                        Span::styled(
                            method.label(),
                            if selected {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                    ]));
                }
            }
            LoginBrowseStep::InputEndpoint => {
                lines.push(Line::from(Span::styled(
                    " Enter endpoint ",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " Provider: {}",
                        selected_provider.as_deref().unwrap_or("provider")
                    ),
                    Style::default().fg(THEME.fg_muted),
                )));
                lines.push(Line::from(Span::raw(format!(
                    " Endpoint: {}",
                    input_buffer
                ))));
            }
            LoginBrowseStep::InputApiKey => {
                let is_code_display = matches!(selected_method, Some(AuthMethodChoice::OAuth));
                let title = if is_code_display {
                    " Enter authorization code "
                } else {
                    " Enter API key "
                };
                let label = if is_code_display { "Code" } else { "API key" };
                let display = if is_code_display {
                    input_buffer.as_str()
                } else {
                    masked_buffer.as_str()
                };
                lines.push(Line::from(Span::styled(
                    title,
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " Provider: {}",
                        selected_provider.as_deref().unwrap_or("provider")
                    ),
                    Style::default().fg(THEME.fg_muted),
                )));
                if is_code_display {
                    lines.push(Line::from(Span::styled(
                        " Paste the code shown in your browser after authorizing.",
                        Style::default().fg(THEME.warning),
                    )));
                }
                if let Some(endpoint) = endpoint {
                    lines.push(Line::from(Span::styled(
                        format!(" Endpoint: {}", endpoint),
                        Style::default().fg(THEME.fg_muted),
                    )));
                }
                lines.push(Line::from(Span::raw(format!(" {}: {}", label, display))));
            }
        }

        if let Some(error) = last_error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Error: {}", error),
                Style::default()
                    .fg(THEME.error)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        let content_height = lines.len() as u16;
        let visible_height = chunks[1].height.saturating_sub(2);
        let max_scroll = content_height.saturating_sub(visible_height);
        let effective_scroll = (*scroll).min(max_scroll);
        let body = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.fg_muted)),
            )
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0));
        frame.render_widget(body, chunks[1]);

        let footer =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[2]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                " ↑/↓ to select • Enter: confirm • Type: to search/input ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Esc: back/close • j/k: move • PgUp/PgDn: page ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[1],
        );

        match step {
            LoginBrowseStep::SelectProvider => {
                let cursor_col = query.chars().count() as u16;
                frame.set_cursor_position((chunks[1].x + 10 + cursor_col, chunks[1].y + 3));
            }
            LoginBrowseStep::InputEndpoint => {
                let cursor_col = input_buffer.chars().count() as u16;
                frame.set_cursor_position((chunks[1].x + 12 + cursor_col, chunks[1].y + 4));
            }
            LoginBrowseStep::InputApiKey => {
                let is_code_display = matches!(selected_method, Some(AuthMethodChoice::OAuth));
                let display_len = if is_code_display {
                    input_buffer.chars().count()
                } else {
                    masked_buffer.chars().count()
                };
                // " Code: " = 7, " API key: " = 10
                let label_offset: u16 = if is_code_display { 8 } else { 11 };
                let hint_offset: u16 = if is_code_display { 1 } else { 0 };
                let endpoint_offset: u16 = if endpoint.is_some() { 1 } else { 0 };
                frame.set_cursor_position((
                    chunks[1].x + label_offset + display_len as u16,
                    chunks[1].y + 4 + hint_offset + endpoint_offset,
                ));
            }
            LoginBrowseStep::SelectMethod => {}
        }
    }

    fn render_model_browser(&self, frame: &mut Frame<'_>, area: Rect) {
        let AppState::ModelBrowsing {
            query,
            cursor,
            scroll,
            last_sync_status,
            search_active,
        } = &self.state
        else {
            return;
        };

        let entries = self.visible_model_entries(query);

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

        let header = Line::from(vec![
            Span::styled(
                " Models ",
                Style::default()
                    .fg(THEME.browser_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({}) ", self.model_provider),
                Style::default().fg(THEME.fg_muted),
            ),
            Span::styled(
                format!(" {} ", last_sync_status),
                Style::default().fg(if last_sync_status.starts_with("Offline") {
                    THEME.warning
                } else {
                    THEME.fg_muted
                }),
            ),
        ]);
        frame.render_widget(Paragraph::new(header), chunks[0]);

        let mut lines: Vec<Line<'_>> = Vec::new();
        let mut visible_index = 0usize;
        if entries.is_empty() {
            lines.push(Line::from(Span::styled(
                if query.trim().is_empty() {
                    "  No models available.".to_string()
                } else {
                    format!("  No matches for '{}'.", query)
                },
                Style::default().fg(THEME.warning),
            )));
        } else {
            for entry in entries {
                let selected = visible_index == *cursor;
                lines.push(Line::from(vec![
                    Span::styled(
                        if selected { "› " } else { "  " },
                        if selected {
                            Style::default()
                                .fg(THEME.browser_selected_marker)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(THEME.fg_muted)
                        },
                    ),
                    Span::styled(
                        entry.id,
                        Style::default()
                            .fg(if query.trim().is_empty() {
                                THEME.fg
                            } else {
                                THEME.warning
                            })
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        format!("  [{}]", entry.provider),
                        Style::default().fg(THEME.fg_muted),
                    ),
                ]));
                visible_index += 1;
            }
        }

        let content_height = lines.len() as u16;
        let visible_height = chunks[1].height.saturating_sub(2);
        let max_scroll = content_height.saturating_sub(visible_height);
        let effective_scroll = (*scroll).min(max_scroll);

        let list = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.fg_muted)),
            )
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0));
        frame.render_widget(list, chunks[1]);

        let footer =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[2]);
        let query_label = if *search_active {
            format!(" Search (typing): {}", query)
        } else {
            format!(" Search: {}", query)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                query_label,
                Style::default().fg(THEME.browser_footer),
            )),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                " s or /:search  j/k,↑/↓:move  PgUp/PgDn:page  Enter:use model  r:refresh  Esc:back/close ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[1],
        );

        if *search_active {
            let cursor_col = query.chars().count() as u16;
            frame.set_cursor_position((footer[0].x + 18 + cursor_col, footer[0].y));
        }
    }

    fn render_title(&self, frame: &mut Frame<'_>, area: Rect) {
        let title = Line::from(vec![
            Span::styled(
                " Chat ",
                Style::default()
                    .fg(THEME.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" model:{} ", self.model_name),
                Style::default().fg(THEME.fg),
            ),
            Span::styled(
                format!(" session:{} ", self.session_id.as_str()),
                Style::default().fg(THEME.fg_muted),
            ),
        ]);
        frame.render_widget(Paragraph::new(title), area);
    }

    fn render_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let is_idle_or_prompting =
            matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });
        let border_color = if is_idle_or_prompting {
            THEME.accent
        } else {
            THEME.fg_muted
        };

        let mut display_text = if self.input.is_empty() && is_idle_or_prompting {
            if matches!(self.state, AppState::AuthPrompting { .. }) {
                "Paste your API key here..."
            } else {
                "Type a message..."
            }
            .to_string()
        } else {
            self.input.clone()
        };

        if matches!(self.state, AppState::AuthPrompting { .. }) && !self.input.is_empty() {
            display_text = "*".repeat(self.input.chars().count());
        }

        let input_style = if self.input.is_empty() && is_idle_or_prompting {
            Style::default().fg(THEME.fg_muted)
        } else {
            Style::default()
        };

        let input = Paragraph::new(Span::styled(display_text, input_style)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(if matches!(self.state, AppState::AuthPrompting { .. }) {
                    " Auth "
                } else {
                    " Input "
                }),
        );

        frame.render_widget(input, area);

        if is_idle_or_prompting {
            let cursor_col = self.input[..self.cursor_pos].chars().count() as u16;
            frame.set_cursor_position((area.x + 1 + cursor_col, area.y + 1));
        }

        if self.is_palette_active() {
            self.render_command_palette(frame, area);
        }
    }

    pub(crate) fn render_command_palette(&self, frame: &mut Frame<'_>, input_area: Rect) {
        let cmds = self.palette_filtered_commands();
        if cmds.is_empty() {
            return;
        }

        let popup_h = cmds.len() as u16 + 2; // content + top/bottom border
        let popup_y = input_area.y.saturating_sub(popup_h);
        let popup_rect = Rect {
            x: input_area.x,
            y: popup_y,
            width: input_area.width,
            height: popup_h,
        };

        // Name column width = longest command name.
        let name_w = cmds.iter().map(|c| c.name.len()).max().unwrap_or(0);

        let authenticated = self.is_authenticated();
        let lines: Vec<Line<'_>> = cmds
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let sel = i == self.command_palette_cursor;
                let arrow = if sel { "› " } else { "  " };
                let pad = " ".repeat(name_w.saturating_sub(cmd.name.len()) + 2);
                let is_login = cmd.name == "/login" || cmd.name == "/connection";
                let auth_dot = if is_login && authenticated {
                    Some(Span::styled(
                        "● ",
                        Style::default().fg(THEME.palette_auth_dot),
                    ))
                } else {
                    None
                };
                let mut spans = vec![
                    Span::styled(
                        arrow,
                        Style::default().fg(if sel {
                            THEME.palette_cmd
                        } else {
                            THEME.fg_muted
                        }),
                    ),
                    Span::styled(
                        cmd.name,
                        Style::default().fg(THEME.palette_cmd).add_modifier(if sel {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                    Span::raw(pad),
                ];
                if let Some(dot) = auth_dot {
                    spans.push(dot);
                }
                spans.push(Span::styled(
                    cmd.description,
                    Style::default()
                        .fg(if sel {
                            THEME.palette_selected_fg
                        } else {
                            THEME.palette_desc
                        })
                        .add_modifier(if sel {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ));
                Line::from(spans)
            })
            .collect();

        frame.render_widget(Clear, popup_rect);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.palette_border))
                    .title(Span::styled(
                        " Commands ",
                        Style::default()
                            .fg(THEME.palette_border)
                            .add_modifier(Modifier::BOLD),
                    )),
            ),
            popup_rect,
        );
    }

    /// Returns the number of user/assistant message pairs in the conversation history.
    pub fn conversation_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| matches!(m, TuiMessage::User(_) | TuiMessage::Assistant(_)))
            .count()
    }

    /// Sets `history_scroll` to its maximum value so the next render shows the latest messages.
    pub fn scroll_to_bottom(&mut self) {
        // Set to a large value; render_history clamps it to max_scroll.
        self.history_scroll = u16::MAX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                id: "o3".to_string(),
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
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], TuiMessage::ToolCall { .. }));
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
        assert_eq!(app.messages.len(), 2);
        assert!(matches!(
            &app.messages[1],
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
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], TuiMessage::Assistant(t) if t == "hello world"));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn apply_completion_err_pushes_error() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.apply_completion(Err(proto::Error::Llm(proto::LlmError::RateLimit)));
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], TuiMessage::Error(_)));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn handle_key_inserts_chars() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.input, "ab");
        assert_eq!(app.cursor_pos, 2);
    }

    #[test]
    fn handle_key_backspace_deletes() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn handle_key_ignores_input_when_thinking() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.input, "");
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
        assert_eq!(app.input, "sk");
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
        app.input = "secret".to_string();
        app.cursor_pos = app.input.len();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(app.state, AppState::Idle);
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
        assert!(
            matches!(app.messages.last(), Some(TuiMessage::Assistant(text)) if text.contains("cancelled"))
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
        app.input = "  sk-test  ".to_string();
        app.cursor_pos = app.input.len();

        let submission = app.take_auth_submission().expect("submission expected");

        assert_eq!(submission.provider, "together");
        assert_eq!(submission.env_name, "TOGETHER_API_KEY");
        assert_eq!(submission.endpoint, None);
        assert_eq!(submission.api_key, "sk-test");
        assert_eq!(app.input, "");
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
            &app.messages[0],
            TuiMessage::ToolCall { done: false, .. }
        ));
        assert!(matches!(
            &app.messages[1],
            TuiMessage::ToolCall { done: true, .. }
        ));
    }

    #[test]
    fn handle_key_moves_cursor_left_and_right_with_utf8() {
        let mut app = make_app();
        app.input = "a한b".into();
        app.cursor_pos = app.input.len();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a한".len());

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a".len());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a한".len());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a한b".len());
    }

    #[test]
    fn handle_key_scroll_shortcuts_update_history_scroll() {
        let mut app = make_app();
        app.history_scroll = 5;

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 4);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 5);

        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 10);
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

        assert_eq!(app.model_name, "o3");
        assert_eq!(app.state, AppState::Idle);
        assert!(matches!(
            app.messages.last(),
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
        app.messages.push(TuiMessage::ToolCall {
            tool_name: "system.run".into(),
            args_preview: "{\"command\":\"echo ok\"}".into(),
            done: false,
        });
        app.messages.push(TuiMessage::ToolResult {
            tool_name: "system.run".into(),
            output_preview: "ok".into(),
            is_error: false,
        });
        app.push_error("boom".into());
        app.input = "typed".into();
        app.cursor_pos = 2;
        app.state = AppState::ExecutingTool {
            tool_name: "system.run".into(),
        };
        app.spinner_tick = 3;
        app.history_scroll = 7;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.input, "typed");
        assert_eq!(app.cursor_pos, 2);
        assert_eq!(app.history_scroll, 7);
        assert_eq!(app.messages.len(), 5);
    }

    #[test]
    fn render_idle_placeholder_path_executes() {
        let mut app = make_app();
        app.state = AppState::Idle;
        app.input.clear();
        app.cursor_pos = 0;

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.state, AppState::Idle);
        assert_eq!(app.input, "");
    }

    #[test]
    fn take_input_resets() {
        let mut app = make_app();
        app.input = "hello".into();
        app.cursor_pos = 5;
        let taken = app.take_input();
        assert_eq!(taken, "hello");
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn handle_slash_command_help_pushes_local_message() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/help");
        assert!(handled);
        assert!(matches!(&app.messages[0], TuiMessage::Assistant(_)));
    }

    #[test]
    fn handle_slash_command_login_opens_login_browser_with_seed() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/login openai");
        assert!(handled);
        assert!(matches!(
            &app.state,
            AppState::LoginBrowsing { query, step, .. } if query == "openai" && *step == LoginBrowseStep::SelectProvider
        ));
        assert!(app.messages.is_empty());
    }

    #[test]
    fn handle_slash_command_login_without_provider_opens_browser() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/login");
        assert!(handled);
        assert!(matches!(&app.state, AppState::LoginBrowsing { query, .. } if query.is_empty()));
    }

    #[test]
    fn handle_slash_command_connection_alias_opens_login_browser() {
        let mut app = make_app();
        let handled = app.handle_slash_command("/connection copilot");
        assert!(handled);
        assert!(matches!(
            &app.state,
            AppState::LoginBrowsing { query, .. } if query == "copilot"
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
            AppState::LoginBrowsing {
                step,
                selected_provider,
                ..
            } if *step == LoginBrowseStep::SelectMethod
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
        assert!(app.messages.is_empty());
        assert_eq!(app.history_scroll, 0);
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
        assert!(matches!(&app.messages[0], TuiMessage::Error(_)));
    }

    #[test]
    fn handle_slash_command_returns_false_for_plain_message() {
        let mut app = make_app();
        let handled = app.handle_slash_command("hello");
        assert!(!handled);
        assert!(app.messages.is_empty());
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
        assert_eq!(app.history_scroll, u16::MAX);
    }

    // ── Command picker tests ────────────────────────────────

    #[test]
    fn command_picker_activates_on_slash() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        assert!(!app.is_palette_active());
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(app.is_palette_active());
        assert!(app.input.starts_with('/'));
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
        assert_eq!(app.input, "/help");
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
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
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
        assert_eq!(app.input, "");
    }
}
