use super::app::{AppState, TuiApp};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

const SPINNER: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

pub fn render(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
    let status_text = match &app.state {
        AppState::Idle => Line::from(vec![
            Span::styled(
                format!(" {} ", app.workspace_name),
                Style::default().fg(Color::Rgb(115, 138, 172)),
            ),
            Span::styled(" ⭘ ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{} ", app.branch_name),
                Style::default().fg(Color::Rgb(115, 138, 172)),
            ),
            Span::styled(
                format!(" 💚 {} MCP /status ", app.mcp_count),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Enter:send  ↑↓:scroll  Ctrl+C:quit",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        AppState::Thinking { round } => {
            let spinner = SPINNER[(app.spinner_tick as usize) % SPINNER.len()];
            Line::from(vec![
                Span::styled(
                    format!(" {} ", app.workspace_name),
                    Style::default().fg(Color::Rgb(115, 138, 172)),
                ),
                Span::styled(
                    format!(" {spinner} Thinking... "),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("[round {round}]"),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        }
        AppState::ExecutingTool { tool_name } => {
            let spinner = SPINNER[(app.spinner_tick as usize) % SPINNER.len()];
            Line::from(vec![
                Span::styled(
                    format!(" {} ", app.workspace_name),
                    Style::default().fg(Color::Rgb(115, 138, 172)),
                ),
                Span::styled(
                    format!(" {spinner} Running "),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(tool_name.clone(), Style::default().fg(Color::Cyan)),
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
                Style::default().fg(Color::DarkGray),
            ))
        }
        AppState::AuthValidating { provider } => {
            let spinner = SPINNER[(app.spinner_tick as usize) % SPINNER.len()];
            Line::from(vec![Span::styled(
                format!(" {spinner} Saving credential for {provider}... "),
                Style::default().fg(Color::Yellow),
            )])
        }
        AppState::LoginBrowsing { .. } => Line::from(Span::styled(
            " Login browser active ",
            Style::default().fg(Color::DarkGray),
        )),
        AppState::ModelBrowsing { .. } => Line::from(Span::styled(
            " Model browser active ",
            Style::default().fg(Color::DarkGray),
        )),
    };

    // Create a split to right-align the version
    let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(10)]).split(area);

    frame.render_widget(Paragraph::new(status_text), chunks[0]);

    let version_text = Line::from(Span::styled(
        format!("{}  ", app.version),
        Style::default().fg(Color::Rgb(115, 138, 172)),
    ));
    frame.render_widget(Paragraph::new(version_text).right_aligned(), chunks[1]);
}
