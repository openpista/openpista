//! Status bar widget — workspace, branch, MCP count, state indicator, and version.

use super::app::{AppState, TuiApp};
use super::theme::THEME;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Braille-pattern spinner frames for the status bar animation.
const SPINNER: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

/// Renders the status bar showing workspace, branch, MCP count, app state, and version.
pub fn render(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
    let status_text = match &app.state {
        AppState::Idle if app.sidebar_focused => Line::from(vec![
            Span::styled(
                format!(" {} ", app.workspace_name),
                Style::default().fg(THEME.status_workspace),
            ),
            Span::styled(" ⭘ ", Style::default().fg(THEME.success)),
            Span::styled(
                format!("{} ", app.branch_name),
                Style::default().fg(THEME.status_branch),
            ),
            Span::styled(
                format!(" 💚 {} MCP /status ", app.mcp_count),
                Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  ↑↓:navigate  Enter:load  d:delete  n:new  Tab:chat  Ctrl+C:quit",
                Style::default().fg(THEME.sidebar_active_indicator),
            ),
        ]),
        AppState::Idle => Line::from(vec![
            Span::styled(
                format!(" {} ", app.workspace_name),
                Style::default().fg(THEME.status_workspace),
            ),
            Span::styled(" ⭘ ", Style::default().fg(THEME.success)),
            Span::styled(
                format!("{} ", app.branch_name),
                Style::default().fg(THEME.status_branch),
            ),
            Span::styled(
                format!(" 💚 {} MCP /status ", app.mcp_count),
                Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Enter:send  ↑↓:scroll  Tab:sidebar  Ctrl+C:quit",
                Style::default().fg(THEME.status_hint),
            ),
        ]),
        AppState::Thinking { round } => {
            let spinner = SPINNER[(app.spinner_tick as usize) % SPINNER.len()];
            Line::from(vec![
                Span::styled(
                    format!(" {} ", app.workspace_name),
                    Style::default().fg(THEME.status_workspace),
                ),
                Span::styled(
                    format!(" {spinner} Thinking... "),
                    Style::default().fg(THEME.status_spinner),
                ),
                Span::styled(
                    format!("[round {round}]"),
                    Style::default().fg(THEME.status_hint),
                ),
            ])
        }
        AppState::ExecutingTool { tool_name } => {
            let spinner = SPINNER[(app.spinner_tick as usize) % SPINNER.len()];
            Line::from(vec![
                Span::styled(
                    format!(" {} ", app.workspace_name),
                    Style::default().fg(THEME.status_workspace),
                ),
                Span::styled(
                    format!(" {spinner} Running "),
                    Style::default().fg(THEME.status_spinner),
                ),
                Span::styled(tool_name.clone(), Style::default().fg(THEME.info)),
            ])
        }
        AppState::AuthPrompting {
            provider,
            env_name,
            endpoint,
            endpoint_env,
        } => {
            let endpoint_hint = if let (Some(value), Some(key)) = (endpoint, endpoint_env) {
                format!(" endpoint[{key}]={value}")
            } else {
                String::new()
            };
            Line::from(Span::styled(
                format!(" Enter API key for {provider} ({env_name}){endpoint_hint}  Ctrl+C:cancel"),
                Style::default().fg(THEME.status_hint),
            ))
        }
        AppState::AuthValidating { provider } => {
            let spinner = SPINNER[(app.spinner_tick as usize) % SPINNER.len()];
            Line::from(vec![Span::styled(
                format!(" {spinner} Saving credential for {provider}... "),
                Style::default().fg(THEME.status_spinner),
            )])
        }
        AppState::LoginBrowsing { .. } => Line::from(Span::styled(
            " Login browser active ",
            Style::default().fg(THEME.status_hint),
        )),
        AppState::ModelBrowsing { .. } => Line::from(Span::styled(
            " Model browser active ",
            Style::default().fg(THEME.status_hint),
        )),
        AppState::ConfirmDelete { .. } => Line::from(Span::styled(
            " Confirm delete — y/Enter: delete, n/Esc: cancel ",
            Style::default().fg(THEME.error),
        )),
    };

    // Create a split to right-align the version
    let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(10)]).split(area);

    frame.render_widget(Paragraph::new(status_text), chunks[0]);

    let version_text = Line::from(Span::styled(
        format!("{}  ", app.version),
        Style::default().fg(THEME.status_workspace),
    ));
    frame.render_widget(Paragraph::new(version_text).right_aligned(), chunks[1]);
}
