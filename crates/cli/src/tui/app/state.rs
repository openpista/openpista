//! TUI application state types, enums, structs, and constants.
#![allow(dead_code)]

use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice, LoginBrowseStep};
use crate::model_catalog;
use proto::{ChannelId, SessionId};
use ratatui::layout::Rect;

/// Spinner animation frames (Braille pattern).
pub(crate) const SPINNER: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

// ─── Command palette ──────────────────────────────────────────

pub(crate) struct SlashCommand {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}

pub(crate) const SLASH_COMMANDS: &[SlashCommand] = &[
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
    SlashCommand {
        name: "/session",
        description: "Browse sessions",
    },
    SlashCommand {
        name: "/session new",
        description: "Start a new session",
    },
    SlashCommand {
        name: "/session load <id>",
        description: "Load a session by ID",
    },
    SlashCommand {
        name: "/session delete <id>",
        description: "Delete a session by ID",
    },
    SlashCommand {
        name: "/web",
        description: "Show web adapter status",
    },
    SlashCommand {
        name: "/web setup",
        description: "Configure web adapter (wizard)",
    },
    SlashCommand {
        name: "/whatsapp",
        description: "Configure WhatsApp channel",
    },
    SlashCommand {
        name: "/whatsapp status",
        description: "Show WhatsApp config status",
    },
    SlashCommand {
        name: "/telegram",
        description: "Telegram bot setup guide",
    },
    SlashCommand {
        name: "/telegram status",
        description: "Show Telegram config status",
    },
    SlashCommand {
        name: "/telegram start",
        description: "Start Telegram adapter info",
    },
    SlashCommand {
        name: "/qr",
        description: "Show QR code for Web UI URL",
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

/// Steps in the web adapter configuration wizard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WebConfigStep {
    /// Toggle web adapter enabled/disabled.
    #[default]
    Enable,
    /// Enter auth token.
    Token,
    /// Enter listen port.
    Port,
    /// Enter CORS origins.
    CorsOrigins,
    /// Enter static file directory.
    StaticDir,
    /// Confirm and save settings.
    Confirm,
}

/// State for the login provider browser.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoginBrowsingState {
    /// Provider list search query.
    pub query: String,
    /// Current cursor position.
    pub cursor: usize,
    /// Scroll offset.
    pub scroll: u16,
    /// Active browser step.
    pub step: LoginBrowseStep,
    /// Selected provider id.
    pub selected_provider: Option<String>,
    /// Selected auth method.
    pub selected_method: Option<AuthMethodChoice>,
    /// Raw input for endpoint/API key steps.
    pub input_buffer: String,
    /// Masked API-key display buffer.
    pub masked_buffer: String,
    /// Last error shown in browser.
    pub last_error: Option<String>,
    /// Endpoint captured from endpoint step.
    pub endpoint: Option<String>,
}

/// State for the web adapter configuration wizard.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WebConfiguringState {
    /// Current wizard step.
    pub step: WebConfigStep,
    /// Whether web adapter is enabled.
    pub enabled: bool,
    /// Auth token value being configured.
    pub token: String,
    /// Port string being configured.
    pub port: String,
    /// CORS origins value being configured.
    pub cors_origins: String,
    /// Static dir value being configured.
    pub static_dir: String,
    /// Text input buffer for the current step.
    pub input_buffer: String,
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
    LoginBrowsing(LoginBrowsingState),
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
    /// Browse sessions in a dedicated TUI screen.
    SessionBrowsing {
        /// Case-insensitive substring query for filtering.
        query: String,
        /// Selected row index among visible sessions.
        cursor: usize,
        /// Scroll offset for session list.
        scroll: u16,
        /// Whether in-browser search mode is active.
        search_active: bool,
    },
    /// Confirmation dialog before deleting a session.
    ConfirmDelete {
        /// Session ID being deleted.
        session_id: String,
        /// Short preview text shown in the confirmation dialog.
        session_preview: String,
    },
    /// Step-by-step web adapter configuration wizard.
    WebConfiguring(WebConfiguringState),
    /// WhatsApp pairing flow (QR code from Baileys bridge).
    WhatsAppSetup {
        /// Current pairing step.
        step: WhatsAppSetupStep,
    },
    /// QR code overlay showing the Web UI URL.
    QrCodeDisplay {
        /// The URL encoded in the QR code.
        url: String,
        /// Pre-rendered QR code lines (Unicode half-blocks).
        qr_lines: Vec<String>,
    },
}

/// Steps in the WhatsApp pairing flow.
#[derive(Debug, Clone, PartialEq)]
pub enum WhatsAppSetupStep {
    /// Checking if Node.js and bridge dependencies are available.
    CheckingPrereqs,
    /// Installing bridge dependencies (npm install).
    InstallingBridge,
    /// Waiting for the bridge to produce a QR code.
    WaitingForQr,
    /// Displaying a QR code for the user to scan.
    DisplayQr {
        /// QR code data string from the bridge.
        qr_data: String,
        /// Pre-rendered QR lines (Unicode half-blocks).
        qr_lines: Vec<String>,
    },
    /// Successfully connected.
    Connected {
        /// Phone number of the paired device.
        phone: String,
        /// Display name of the paired device.
        name: String,
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

// ─── Sub-structs ──────────────────────────────────────────────

/// Chat/conversation-related state.
pub(crate) struct ChatState {
    /// Ordered conversation history for display.
    pub(crate) messages: Vec<TuiMessage>,
    /// Current text typed in the input box (not yet submitted).
    pub(crate) input: String,
    /// Cursor position within `input` (byte offset).
    pub(crate) cursor_pos: usize,
    /// Vertical scroll offset for the history panel.
    pub(crate) history_scroll: u16,
    /// Current mouse text selection state.
    pub(crate) text_selection: crate::tui::selection::TextSelection,
    /// Bounding rect of the chat widget (set each render; used for mouse hit-testing).
    pub(crate) chat_area: Option<Rect>,
    /// Character grid mirroring the chat render layout (set each render; used for text extraction).
    pub(crate) chat_text_grid: Vec<Vec<char>>,
    /// Scroll value after clamping to max_scroll (set each render; used for text extraction).
    pub(crate) chat_scroll_clamped: u16,
    /// Pending tool approval request awaiting user decision (Y/N/A).
    pub(crate) pending_approval: Option<crate::tui::approval::PendingApproval>,
}

/// Session-related state.
pub(crate) struct SessionState {
    /// Session identifier.
    pub(crate) session_id: SessionId,
    /// Channel id for this TUI session.
    #[allow(dead_code)]
    pub(crate) channel_id: ChannelId,
    /// Session list for sidebar display.
    pub(crate) session_list: Vec<SessionEntry>,
    /// Pending sidebar session selection (set by Enter key, consumed by event loop).
    pub(crate) pending_sidebar_selection: Option<SessionId>,
    /// Confirmed session deletion (set by ConfirmDelete y/Enter, consumed by event loop).
    pub(crate) confirmed_delete: Option<SessionId>,
    /// Pending session browser action: create new session (consumed by event loop).
    pub(crate) session_browser_new_requested: bool,
}

/// Model/provider/auth-related state.
pub(crate) struct ModelState {
    /// Model name shown in the status bar.
    pub(crate) model_name: String,
    /// Last loaded model catalog entries.
    pub(crate) model_entries: Vec<model_catalog::ModelCatalogEntry>,
    /// Provider backing the current model catalog.
    pub(crate) model_provider: String,
    /// Set when user pressed `r` inside model browser.
    pub(crate) model_refresh_requested: bool,
    /// Set when the user selects a model in the model browser; consumed by the event loop. (model_id, provider_name)
    pub(crate) pending_model_change: Option<(String, String)>,
    /// Provider name used for auth status check (e.g. "openai", "anthropic").
    pub(crate) provider_name: String,
    /// Pending auth submission from login browser.
    pub(crate) pending_auth_intent: Option<AuthLoginIntent>,
}

/// Sidebar-related state.
pub(crate) struct SidebarState {
    /// Index of sidebar item under mouse hover.
    pub(crate) hover: Option<usize>,
    /// Scroll offset for sidebar.
    pub(crate) scroll: u16,
    /// Whether the sidebar is visible.
    pub(crate) visible: bool,
    /// Whether keyboard input is directed to sidebar.
    pub(crate) focused: bool,
}

// ─── TuiApp ──────────────────────────────────────────────────

/// Full state for the TUI session.
pub struct TuiApp {
    // ── Sub-structs ──
    /// Chat/conversation state.
    pub(crate) chat: ChatState,
    /// Session state.
    pub(crate) session: SessionState,
    /// Model/provider/auth state.
    pub(crate) model: ModelState,
    /// Sidebar state.
    pub(crate) sidebar: SidebarState,
    // ── Flat UI state ──
    /// Current high-level processing state.
    pub(crate) state: AppState,
    /// Which screen is currently displayed.
    pub(crate) screen: Screen,
    /// Spinner animation tick counter.
    pub(crate) spinner_tick: u8,
    /// Whether the user requested exit.
    pub(crate) should_quit: bool,
    /// Selected row in the command palette popup.
    pub(crate) command_palette_cursor: usize,
    /// Workspace name for status bar.
    pub(crate) workspace_name: String,
    /// Git branch for status bar.
    pub(crate) branch_name: String,
    /// Available MCP servers for status bar.
    pub(crate) mcp_count: usize,
    /// Version text.
    pub(crate) version: String,
    /// Pending web config from completed wizard (consumed by event loop).
    pub(crate) pending_web_config: Option<crate::config::WebConfig>,
}

/// Generates QR code lines using Unicode half-block characters.
/// Each output line represents two QR module rows.
pub fn generate_qr_lines(url: &str) -> Result<Vec<String>, String> {
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| format!("QR encode error: {e}"))?;
    let modules = code.to_colors();
    let width = code.width();
    let mut lines = Vec::new();
    let rows: Vec<&[qrcode::Color]> = modules.chunks(width).collect();
    let mut y = 0;
    while y < rows.len() {
        let top = rows[y];
        let bottom = rows.get(y + 1);
        let mut line = String::new();
        for x in 0..width {
            let t = top[x] == qrcode::Color::Dark;
            let b = bottom.is_some_and(|r| r[x] == qrcode::Color::Dark);
            line.push(match (t, b) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        lines.push(line);
        y += 2;
    }
    Ok(lines)
}
