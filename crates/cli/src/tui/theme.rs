//! Centralized TUI theme built on ratatui's Tailwind CSS palette.

use ratatui::style::Color;
use ratatui::style::palette::tailwind;

/// The application theme — all visual tokens in one place.
pub struct Theme {
    // ── Base ──
    pub bg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_muted: Color,
    pub border: Color,
    pub border_active: Color,
    pub border_focused: Color,

    // ── Accent / Brand ──
    pub accent: Color,
    pub accent_dim: Color,
    pub accent_bright: Color,

    // ── Semantic ──
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,

    // ── Chat roles ──
    pub user_label: Color,
    pub assistant_label: Color,
    pub tool_call: Color,
    pub tool_result: Color,

    // ── Status bar ──
    pub status_workspace: Color,
    pub status_branch: Color,
    pub status_spinner: Color,
    pub status_hint: Color,

    // ── Sidebar ──
    pub sidebar_border: Color,
    pub sidebar_active_indicator: Color,
    pub sidebar_hover: Color,
    pub sidebar_text: Color,
    pub sidebar_time: Color,
    pub sidebar_divider: Color,

    // ── Home screen ──
    pub logo: Color,
    pub home_input_bg: Color,
    pub home_shortcut_key: Color,
    pub home_shortcut_desc: Color,
    pub home_tip_icon: Color,

    // ── Command palette ──
    pub palette_border: Color,
    pub palette_cmd: Color,
    pub palette_desc: Color,
    pub palette_selected_fg: Color,
    pub palette_auth_dot: Color,

    // ── Login/Model browser ──
    pub browser_title: Color,
    pub browser_selected_marker: Color,
    pub browser_search: Color,
    pub browser_footer: Color,

    // ── Text selection ──
    pub selection_bg: Color,
    pub selection_fg: Color,
}

impl Theme {
    /// The default dark theme using Tailwind palette.
    pub const fn default_dark() -> Self {
        Self {
            // Base
            bg: tailwind::SLATE.c950,
            fg: tailwind::SLATE.c100,
            fg_dim: tailwind::SLATE.c400,
            fg_muted: tailwind::SLATE.c500,
            border: tailwind::SLATE.c700,
            border_active: tailwind::EMERALD.c500,
            border_focused: tailwind::EMERALD.c400,

            // Accent
            accent: tailwind::EMERALD.c500,
            accent_dim: tailwind::EMERALD.c700,
            accent_bright: tailwind::EMERALD.c400,

            // Semantic
            success: tailwind::EMERALD.c500,
            warning: tailwind::AMBER.c500,
            error: tailwind::RED.c500,
            info: tailwind::SKY.c500,

            // Chat
            user_label: tailwind::CYAN.c400,
            assistant_label: tailwind::EMERALD.c400,
            tool_call: tailwind::AMBER.c400,
            tool_result: tailwind::SLATE.c500,

            // Status bar
            status_workspace: tailwind::SKY.c400,
            status_branch: tailwind::SKY.c400,
            status_spinner: tailwind::AMBER.c400,
            status_hint: tailwind::SLATE.c500,

            // Sidebar
            sidebar_border: tailwind::SLATE.c700,
            sidebar_active_indicator: tailwind::CYAN.c400,
            sidebar_hover: tailwind::SLATE.c600,
            sidebar_text: tailwind::SLATE.c300,
            sidebar_time: tailwind::SLATE.c500,
            sidebar_divider: tailwind::SLATE.c800,

            // Home
            logo: tailwind::EMERALD.c400,
            home_input_bg: tailwind::SLATE.c800,
            home_shortcut_key: tailwind::SLATE.c100,
            home_shortcut_desc: tailwind::EMERALD.c600,
            home_tip_icon: tailwind::AMBER.c400,

            // Command palette
            palette_border: tailwind::EMERALD.c500,
            palette_cmd: tailwind::EMERALD.c400,
            palette_desc: tailwind::SLATE.c400,
            palette_selected_fg: tailwind::SLATE.c100,
            palette_auth_dot: tailwind::EMERALD.c400,

            // Login/Model browser
            browser_title: tailwind::EMERALD.c400,
            browser_selected_marker: tailwind::EMERALD.c400,
            browser_search: tailwind::SLATE.c500,
            browser_footer: tailwind::SLATE.c500,

            // Text selection
            selection_bg: tailwind::SKY.c700,
            selection_fg: tailwind::SLATE.c100,
        }
    }
}

/// Global theme instance.
pub const THEME: Theme = Theme::default_dark();
