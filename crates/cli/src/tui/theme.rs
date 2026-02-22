//! Centralized TUI theme built on ratatui's Tailwind CSS palette.

use ratatui::style::Color;
use ratatui::style::palette::tailwind;

/// The application theme — all visual tokens in one place.
pub struct Theme {
    // ── Base ──
    /// Background color for the main application surface.
    pub bg: Color,
    /// Primary foreground/text color.
    pub fg: Color,
    /// Dimmed foreground for less prominent text.
    pub fg_dim: Color,
    /// Muted foreground for minimal-emphasis elements.
    pub fg_muted: Color,
    /// Default border color for panels and widgets.
    pub border: Color,
    /// Border color for the active/focused widget.
    pub border_active: Color,
    /// Border color for a secondary-focused widget.
    pub border_focused: Color,

    // ── Accent / Brand ──
    /// Primary accent/brand color.
    pub accent: Color,
    /// Dimmed accent for subtle branding elements.
    pub accent_dim: Color,
    /// Bright accent for high-emphasis elements.
    pub accent_bright: Color,

    // ── Semantic ──
    /// Color for success indicators.
    pub success: Color,
    /// Color for warning indicators.
    pub warning: Color,
    /// Color for error indicators.
    pub error: Color,
    /// Color for informational indicators.
    pub info: Color,

    // ── Chat roles ──
    /// Label color for user messages in chat.
    pub user_label: Color,
    /// Label color for assistant responses in chat.
    pub assistant_label: Color,
    /// Color for tool call notifications.
    pub tool_call: Color,
    /// Color for tool result output.
    pub tool_result: Color,

    // ── Status bar ──
    /// Workspace name color in the status bar.
    pub status_workspace: Color,
    /// Git branch name color in the status bar.
    pub status_branch: Color,
    /// Spinner animation color in the status bar.
    pub status_spinner: Color,
    /// Hint/keybinding text color in the status bar.
    pub status_hint: Color,

    // ── Sidebar ──
    /// Border around the session sidebar panel.
    pub sidebar_border: Color,
    /// Indicator mark for the active session entry.
    pub sidebar_active_indicator: Color,
    /// Hover highlight color for sidebar entries.
    pub sidebar_hover: Color,
    /// Session preview text color in the sidebar.
    pub sidebar_text: Color,
    /// Relative timestamp color in the sidebar.
    pub sidebar_time: Color,
    /// Horizontal divider between sidebar entries.
    pub sidebar_divider: Color,

    // ── Home screen ──
    /// ASCII art logo color on the welcome screen.
    pub logo: Color,
    /// Background for the home-screen input box.
    pub home_input_bg: Color,
    /// Keyboard shortcut key label color.
    pub home_shortcut_key: Color,
    /// Keyboard shortcut description text color.
    pub home_shortcut_desc: Color,
    /// Tip bullet icon color on the welcome screen.
    pub home_tip_icon: Color,

    // ── Command palette ──
    /// Border around the command palette popup.
    pub palette_border: Color,
    /// Command name text in the palette list.
    pub palette_cmd: Color,
    /// Command description text in the palette list.
    pub palette_desc: Color,
    /// Foreground color for the currently selected palette item.
    pub palette_selected_fg: Color,
    /// Authenticated-provider indicator dot in the palette.
    pub palette_auth_dot: Color,

    // ── Login/Model browser ──
    /// Title bar text in the browser overlay.
    pub browser_title: Color,
    /// Selected-row marker in the browser list.
    pub browser_selected_marker: Color,
    /// Search query text in the browser.
    pub browser_search: Color,
    /// Footer hint text in the browser overlay.
    pub browser_footer: Color,

    // ── Text selection ──
    /// Background highlight for mouse-selected text.
    pub selection_bg: Color,
    /// Foreground color for mouse-selected text.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dark_theme_has_distinct_colors() {
        let theme = Theme::default_dark();
        // Verify key color assignments are not the same
        assert_ne!(theme.bg, theme.fg);
        assert_ne!(theme.accent, theme.error);
        assert_ne!(theme.user_label, theme.assistant_label);
    }

    #[test]
    fn global_theme_is_accessible() {
        // Ensures THEME constant is valid and usable
        let _ = THEME.bg;
        let _ = THEME.fg;
        let _ = THEME.accent;
        let _ = THEME.error;
        let _ = THEME.success;
        let _ = THEME.warning;
        let _ = THEME.info;
        let _ = THEME.user_label;
        let _ = THEME.assistant_label;
        let _ = THEME.tool_call;
        let _ = THEME.tool_result;
        let _ = THEME.sidebar_border;
        let _ = THEME.logo;
        let _ = THEME.selection_bg;
    }
}
