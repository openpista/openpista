//! Elm Architecture (TEA) action and command types for the TUI.
//!
//! All state mutations flow through [`Action`], and side effects are
//! expressed as [`Command`] values returned from `TuiApp::update()`.

use proto::{ProgressEvent, SessionId};

// ─── Action ──────────────────────────────────────────────────────────────────

/// Every possible state mutation in the TUI. The `update()` method on
/// `TuiApp` is the *only* place where `Action` variants are matched
/// and applied.
#[derive(Debug, Clone)]
pub enum Action {
    // ── Input ────────────────────────────────────────────────
    /// Insert a character at the current cursor position.
    InsertChar(char),
    /// Delete the character before the cursor.
    DeleteChar,
    /// Move the input cursor one character to the left.
    MoveCursorLeft,
    /// Move the input cursor one character to the right.
    MoveCursorRight,
    /// Submit the current input (Enter key in idle state on Chat screen).
    SubmitInput,

    // ── Navigation ───────────────────────────────────────────
    /// Scroll history panel up by `n` lines.
    ScrollUp(u16),
    /// Scroll history panel down by `n` lines.
    ScrollDown(u16),
    /// Jump to the bottom of the history panel.
    ScrollToBottom,
    /// Switch to a specific screen.
    SwitchScreen(super::app::Screen),
    /// Toggle keyboard focus between sidebar and main input.
    ToggleSidebarFocus,

    // ── Chat / agent lifecycle ───────────────────────────────
    /// Append a user message to conversation history.
    PushUserMessage(String),
    /// Append an assistant message to conversation history.
    PushAssistantMessage(String),
    /// Append an error message to conversation history.
    PushError(String),
    /// Apply an agent progress event (tool call started/finished, thinking).
    ApplyProgress(ProgressEvent),
    /// Apply the final agent result.
    ApplyCompletion(Result<String, String>),

    // ── Sidebar ──────────────────────────────────────────────
    /// Update sidebar hover index.
    SidebarHover(Option<usize>),
    /// Scroll sidebar by delta (positive = down).
    SidebarScroll(i16),
    /// Select the currently hovered sidebar session.
    SelectSidebarSession,
    /// Request deletion of the currently hovered sidebar session.
    RequestDeleteSession,
    /// Confirm the pending session deletion.
    ConfirmDelete,
    /// Cancel the pending session deletion.
    CancelDelete,

    // ── Auth / login browser ─────────────────────────────────
    /// Open the login browser, optionally pre-filtering by seed.
    OpenLoginBrowser(Option<String>),
    /// Cancel the active auth prompt or login browser.
    CancelAuth,
    /// Forward a key event to the login browser state machine.
    LoginBrowserKey(crossterm::event::KeyEvent),
    /// Transition to OAuth code-display input state for a provider.
    SetOAuthCodeDisplayState { provider: String },
    /// Transition to auth-validating state for a provider.
    SetAuthValidating(String),

    // ── Model browser ────────────────────────────────────────
    /// Open the model browser (sets state, doesn't load data).
    OpenModelBrowser {
        provider: String,
        entries: Vec<crate::model_catalog::ModelCatalogEntry>,
        query: String,
        sync_status: String,
    },
    /// Close the model browser, returning to idle.
    CloseModelBrowser,
    /// Forward a key event to the model browser state machine.
    ModelBrowserKey(crossterm::event::KeyEvent),
    /// Mark model catalog refresh as in-progress.
    MarkModelRefreshing,
    /// Update model catalog entries.
    UpdateModelCatalog {
        provider: String,
        entries: Vec<crate::model_catalog::ModelCatalogEntry>,
        sync_status: String,
    },

    // ── Session browser ──────────────────────────────────────
    /// Open the session browser.
    OpenSessionBrowser,
    /// Close the session browser.
    CloseSessionBrowser,
    /// Forward a key event to the session browser state machine.
    SessionBrowserKey(crossterm::event::KeyEvent),

    // ── Command palette ──────────────────────────────────────
    /// Move command palette cursor up.
    PaletteMoveUp,
    /// Move command palette cursor down.
    PaletteMoveDown,
    /// Select the currently highlighted palette command.
    PaletteSelect,
    /// Dismiss the command palette.
    PaletteClose,
    /// Tab-complete the palette selection.
    PaletteTabComplete,

    // ── Text selection (mouse) ───────────────────────────────
    /// Start a text selection at the given chat-relative position.
    TextSelectionStart { row: u16, col: u16 },
    /// Drag the text selection to a new position.
    TextSelectionDrag { row: u16, col: u16 },
    /// End the text selection (mouse release).
    TextSelectionEnd { row: u16, col: u16 },
    /// Copy the current selection to clipboard and clear it.
    TextSelectionCopy,
    /// Clear the current text selection.
    TextSelectionClear,

    // ── System ───────────────────────────────────────────────
    /// Periodic spinner tick.
    Tick,
    /// Request application quit.
    Quit,
    /// Terminal was resized (no-op, triggers redraw).
    Resize,

    /// Transition to agent thinking state.
    SetThinking,
    /// Transition to idle state (e.g. after auth completion).
    SetIdle,

    // ── Session management ───────────────────────────────────
    /// Load messages from a session into the conversation history.
    LoadSession {
        session_id: SessionId,
        messages: Vec<proto::AgentMessage>,
    },
    /// Replace the sidebar session list.
    RefreshSessionList(Vec<super::app::SessionEntry>),
    /// Create a new session with the given id.
    NewSession(SessionId),
    /// Remove a session from the sidebar list.
    RemoveSession(SessionId),

    // ── Model / provider ─────────────────────────────────────
    /// Update the displayed model name.
    SetModel(String),
    /// Update the provider name shown in the status bar.
    SetProviderName(String),

    // ── Slash commands (non-async) ───────────────────────────
    /// Execute a local slash command (e.g. /help, /clear, /quit).
    SlashCommand(String),

    // ── WhatsApp setup ────────────────────────────────────────
    /// Open the WhatsApp pairing screen (checks prereqs, starts bridge, waits for QR).
    OpenWhatsAppSetup,
    /// Cancel the WhatsApp pairing and return to idle.
    WhatsAppSetupCancel,
    /// Prerequisites check completed.
    WhatsAppPrereqsChecked {
        node_ok: bool,
        bridge_installed: bool,
    },
    /// Bridge npm install completed.
    WhatsAppBridgeInstalled(Result<(), String>),
    /// A QR code was received from the WhatsApp bridge.
    WhatsAppQrReceived(String),
    /// The WhatsApp bridge connected successfully.
    WhatsAppConnected { phone: String, name: String },
    // ── QR code display ─────────────────────────────────────
    /// Open the QR code overlay showing the Web UI URL.
    OpenQrCode { url: String, qr_lines: Vec<String> },
    /// Close the QR code overlay and return to idle.
    CloseQrCode,
}

// ─── Command ─────────────────────────────────────────────────────────────────

/// Side effects returned by `TuiApp::update()`. The event loop is
/// responsible for executing these asynchronously.
#[derive(Debug)]
pub enum Command {
    /// No side effect.
    None,
    /// Spawn an agent task with the given user message.
    SpawnAgentTask(String),
    /// Begin the auth flow with the current pending intent.
    StartAuthFlow,
    /// Load the model catalog (browser or list).
    LoadModelCatalog,
    /// Load a session by id from the database.
    LoadSessionFromDb(SessionId),
    /// Delete a session from the database.
    DeleteSession(SessionId),
    /// Create a new session.
    CreateNewSession,
    /// Refresh the sidebar session list.
    RefreshSidebar,
    /// Copy the given text to the system clipboard.
    CopyToClipboard(String),
    /// Execute multiple commands sequentially.
    Batch(Vec<Command>),
    /// Persist the WhatsApp configuration to the config file.
    /// Spawn the WhatsApp bridge subprocess.
    SpawnWhatsAppBridge,
    /// Check WhatsApp bridge prerequisites.
    CheckWhatsAppPrereqs,
    /// Install WhatsApp bridge npm dependencies.
    InstallWhatsAppBridge,
    SaveWhatsAppConfig(crate::config::WhatsAppConfig),
}
