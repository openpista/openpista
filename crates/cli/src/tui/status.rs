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
        AppState::SessionBrowsing { .. } => Line::from(Span::styled(
            " Session browser active ",
            Style::default().fg(THEME.status_hint),
        )),
        AppState::ConfirmDelete { .. } => Line::from(Span::styled(
            " Confirm delete — y/Enter: delete, n/Esc: cancel ",
            Style::default().fg(THEME.error),
        )),
        AppState::WhatsAppSetup { .. } => Line::from(Span::styled(
            " WhatsApp setup wizard — Enter: next, Esc: cancel ",
            Style::default().fg(THEME.status_hint),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_picker::LoginBrowseStep;
    use proto::{ChannelId, SessionId};
    use ratatui::{Terminal, backend::TestBackend};

    fn make_app() -> TuiApp {
        TuiApp::new(
            "gpt-4o",
            SessionId::new(),
            ChannelId::from("cli:tui"),
            "openai",
        )
    }

    fn render_status(app: &TuiApp) {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_idle_state() {
        let app = make_app();
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.sidebar_focused);
        render_status(&app);
    }

    #[test]
    fn render_idle_sidebar_focused() {
        let mut app = make_app();
        app.sidebar_focused = true;
        render_status(&app);
    }

    #[test]
    fn render_thinking_state() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 3 };
        render_status(&app);
    }

    #[test]
    fn render_thinking_spinner_tick_wraps() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 1 };
        app.spinner_tick = 255;
        render_status(&app);
    }

    #[test]
    fn render_executing_tool_state() {
        let mut app = make_app();
        app.state = AppState::ExecutingTool {
            tool_name: "system.run".to_string(),
        };
        render_status(&app);
    }

    #[test]
    fn render_auth_prompting_without_endpoint() {
        let mut app = make_app();
        app.state = AppState::AuthPrompting {
            provider: "openai".to_string(),
            env_name: "OPENAI_API_KEY".to_string(),
            endpoint: None,
            endpoint_env: None,
        };
        render_status(&app);
    }

    #[test]
    fn render_auth_prompting_with_endpoint() {
        let mut app = make_app();
        app.state = AppState::AuthPrompting {
            provider: "custom".to_string(),
            env_name: "CUSTOM_API_KEY".to_string(),
            endpoint: Some("https://api.example.com".to_string()),
            endpoint_env: Some("CUSTOM_ENDPOINT".to_string()),
        };
        render_status(&app);
    }

    #[test]
    fn render_auth_prompting_partial_endpoint() {
        let mut app = make_app();
        // Only endpoint set, no endpoint_env — should produce empty hint
        app.state = AppState::AuthPrompting {
            provider: "custom".to_string(),
            env_name: "CUSTOM_API_KEY".to_string(),
            endpoint: Some("https://api.example.com".to_string()),
            endpoint_env: None,
        };
        render_status(&app);
    }

    #[test]
    fn render_auth_validating_state() {
        let mut app = make_app();
        app.state = AppState::AuthValidating {
            provider: "anthropic".to_string(),
        };
        render_status(&app);
    }

    #[test]
    fn render_login_browsing_state() {
        let mut app = make_app();
        app.state = AppState::LoginBrowsing {
            query: String::new(),
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
        render_status(&app);
    }

    #[test]
    fn render_model_browsing_state() {
        let mut app = make_app();
        app.state = AppState::ModelBrowsing {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            last_sync_status: "OK".to_string(),
            search_active: false,
        };
        render_status(&app);
    }

    #[test]
    fn render_session_browsing_state() {
        let mut app = make_app();
        app.state = AppState::SessionBrowsing {
            query: String::new(),
            cursor: 0,
            scroll: 0,
            search_active: false,
        };
        render_status(&app);
    }

    #[test]
    fn render_confirm_delete_state() {
        let mut app = make_app();
        app.state = AppState::ConfirmDelete {
            session_id: "sess-123".to_string(),
            session_preview: "hello world".to_string(),
        };
        render_status(&app);
    }

    #[test]
    fn render_shows_version() {
        let mut app = make_app();
        app.version = "1.2.3".to_string();
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        // Version is rendered in the right-aligned chunk
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("1.2.3"));
    }

    #[test]
    fn render_shows_workspace_and_branch() {
        let mut app = make_app();
        app.workspace_name = "myproject".to_string();
        app.branch_name = "feature-x".to_string();
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("myproject"));
        assert!(content.contains("feature-x"));
    }

    #[test]
    fn render_shows_mcp_count() {
        let mut app = make_app();
        app.mcp_count = 5;
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("5 MCP"));
    }

    #[test]
    fn spinner_constant_has_eight_frames() {
        assert_eq!(SPINNER.len(), 8);
    }

    #[test]
    fn render_with_zero_mcp_count() {
        let mut app = make_app();
        app.mcp_count = 0;
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("0 MCP"));
    }

    #[test]
    fn render_spinner_tick_zero() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 1 };
        app.spinner_tick = 0;
        render_status(&app);
    }

    #[test]
    fn render_spinner_tick_seven() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 1 };
        app.spinner_tick = 7;
        render_status(&app);
    }

    #[test]
    fn render_spinner_tick_eight_wraps() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 1 };
        app.spinner_tick = 8;
        render_status(&app);
    }

    #[test]
    fn render_executing_tool_with_spinner_ticks() {
        let mut app = make_app();
        app.state = AppState::ExecutingTool {
            tool_name: "system.run".to_string(),
        };
        for tick in [0u8, 3, 7, 8, 15, 128, 255] {
            app.spinner_tick = tick;
            render_status(&app);
        }
    }

    #[test]
    fn render_idle_with_custom_workspace_branch() {
        let mut app = make_app();
        app.workspace_name = "myrepo".to_string();
        app.branch_name = "feat/test-branch".to_string();
        app.mcp_count = 3;
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("myrepo"));
        assert!(content.contains("feat/test-branch"));
        assert!(content.contains("3 MCP"));
    }

    #[test]
    fn render_auth_validating_with_spinner() {
        let mut app = make_app();
        app.state = AppState::AuthValidating {
            provider: "openai".to_string(),
        };
        for tick in [0u8, 4, 7, 255] {
            app.spinner_tick = tick;
            render_status(&app);
        }
    }

    #[test]
    fn render_thinking_various_rounds() {
        let mut app = make_app();
        for round in [1, 5, 10, 100] {
            app.state = AppState::Thinking { round };
            render_status(&app);
        }
    }

    #[test]
    fn render_version_right_aligned_position() {
        let mut app = make_app();
        app.version = "9.8.7".to_string();
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // Version should appear in the rightmost 10 columns
        let right_10: String = (70..80)
            .map(|x| {
                buf.cell(ratatui::layout::Position { x, y: 0 })
                    .map(|c| c.symbol().chars().next().unwrap_or(' '))
                    .unwrap_or(' ')
            })
            .collect();
        assert!(right_10.contains("9.8.7"));
    }

    #[test]
    fn render_sidebar_focused_idle_shows_sidebar_hints() {
        let mut app = make_app();
        app.sidebar_focused = true;
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("navigate"));
        assert!(content.contains("Tab:chat"));
    }
}
