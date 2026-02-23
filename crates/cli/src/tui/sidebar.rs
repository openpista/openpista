//! Session sidebar widget — lists conversation sessions with relative timestamps.

use super::app::TuiApp;
use super::theme::THEME;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Fixed sidebar width in terminal columns.
const SIDEBAR_WIDTH: u16 = 30;

/// Returns the fixed sidebar width in columns.
pub fn sidebar_width() -> u16 {
    SIDEBAR_WIDTH
}

/// Renders the session sidebar with active/hover highlighting and relative timestamps.
pub fn render(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
    let focus_hint = if app.sidebar_focused {
        Span::styled(" ◉", Style::default().fg(THEME.sidebar_active_indicator))
    } else {
        Span::styled(" [Tab]", Style::default().fg(THEME.fg_muted))
    };
    let header = Line::from(vec![
        Span::styled(
            " Sessions ",
            Style::default()
                .fg(THEME.sidebar_active_indicator)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({})", app.session_list.len()),
            Style::default().fg(THEME.fg_muted),
        ),
        focus_hint,
    ]);

    let border_style = if app.sidebar_focused {
        Style::default().fg(THEME.sidebar_active_indicator)
    } else {
        Style::default().fg(THEME.sidebar_border)
    };
    let block = Block::default()
        .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
        .border_style(border_style)
        .title(header);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.session_list.is_empty() {
        let empty_msg = Paragraph::new(Line::from(Span::styled(
            " No sessions yet",
            Style::default().fg(THEME.fg_muted),
        )));
        frame.render_widget(empty_msg, inner);
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();
    let max_name_width = inner.width.saturating_sub(2) as usize;

    for (idx, entry) in app.session_list.iter().enumerate() {
        let is_active = entry.id.as_str() == app.session_id.as_str();
        let is_hovered = Some(idx) == app.sidebar_hover;

        let indicator = if is_active {
            Span::styled("▌", Style::default().fg(THEME.sidebar_active_indicator))
        } else if is_hovered {
            Span::styled(
                "▌",
                Style::default().fg(if app.sidebar_focused {
                    THEME.sidebar_active_indicator
                } else {
                    THEME.sidebar_hover
                }),
            )
        } else {
            Span::raw(" ")
        };

        let name = truncate_str(&entry.preview, max_name_width.saturating_sub(2));
        let name_style = if is_active {
            Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD)
        } else if is_hovered {
            Style::default().fg(THEME.fg)
        } else {
            Style::default().fg(THEME.sidebar_text)
        };

        lines.push(Line::from(vec![
            indicator,
            Span::styled(format!(" {}", name), name_style),
        ]));

        let time_str = format_relative_time(&entry.updated_at);
        let time_style = if is_active {
            Style::default().fg(THEME.sidebar_time)
        } else {
            Style::default().fg(THEME.fg_muted)
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(time_str, time_style),
        ]));

        if idx < app.session_list.len() - 1 {
            lines.push(Line::from(Span::styled(
                "─".repeat(max_name_width),
                Style::default().fg(THEME.sidebar_divider),
            )));
        }
    }

    let content_height = lines.len() as u16;
    let visible_height = inner.height;
    let max_scroll = content_height.saturating_sub(visible_height);
    let scroll = app.sidebar_scroll.min(max_scroll);

    let list = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(list, inner);
}

/// Truncates a string to `max_len` characters, appending `…` if shortened.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    if s.is_empty() {
        return "(new session)".to_string();
    }
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.chars().count() <= max_len {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Formats a UTC timestamp as a human-readable relative time (e.g. "5m ago").
pub fn format_relative_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(*dt);

    if diff.num_seconds() < 60 {
        "just now".to_string()
    } else if diff.num_minutes() < 60 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else if diff.num_days() < 7 {
        format!("{}d ago", diff.num_days())
    } else {
        dt.format("%b %d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_str_handles_empty_and_short() {
        assert_eq!(truncate_str("", 20), "(new session)");
        assert_eq!(truncate_str("hello", 20), "hello");
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_truncates_long_text() {
        let long = "This is a very long session name that should be truncated";
        let result = truncate_str(long, 15);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 15);
    }

    #[test]
    fn truncate_str_uses_first_line_only() {
        assert_eq!(truncate_str("line1\nline2\nline3", 20), "line1");
    }

    #[test]
    fn format_relative_time_just_now() {
        let now = chrono::Utc::now();
        assert_eq!(format_relative_time(&now), "just now");
    }

    #[test]
    fn format_relative_time_minutes_ago() {
        let past = chrono::Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(format_relative_time(&past), "5m ago");
    }

    #[test]
    fn format_relative_time_hours_ago() {
        let past = chrono::Utc::now() - chrono::Duration::hours(3);
        assert_eq!(format_relative_time(&past), "3h ago");
    }

    #[test]
    fn format_relative_time_days_ago() {
        let past = chrono::Utc::now() - chrono::Duration::days(2);
        assert_eq!(format_relative_time(&past), "2d ago");
    }

    #[test]
    fn format_relative_time_weeks_ago() {
        let past = chrono::Utc::now() - chrono::Duration::days(10);
        let result = format_relative_time(&past);
        assert!(!result.contains("ago"));
    }

    #[test]
    fn truncate_str_zero_max_len() {
        assert_eq!(truncate_str("hello", 0), "");
    }

    use crate::tui::app::{Screen, SessionEntry, TuiApp};
    use proto::{ChannelId, SessionId};
    use ratatui::{Terminal, backend::TestBackend};

    fn make_sidebar_app() -> TuiApp {
        let mut app = TuiApp::new(
            "gpt-4o",
            SessionId::new(),
            ChannelId::from("cli:tui"),
            "openai",
        );
        app.screen = Screen::Chat;
        app.sidebar_visible = true;
        app
    }

    fn make_test_session(id: &str, preview: &str) -> SessionEntry {
        SessionEntry {
            id: SessionId::from(id),
            channel_id: "cli:tui".to_string(),
            updated_at: chrono::Utc::now(),
            preview: preview.to_string(),
        }
    }

    #[test]
    fn render_sidebar_with_sessions() {
        let mut app = make_sidebar_app();
        app.session_list = vec![
            make_test_session("s1", "first session"),
            make_test_session("s2", "second session"),
            make_test_session("s3", "third session"),
        ];
        let backend = TestBackend::new(30, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_sidebar_empty() {
        let app = make_sidebar_app();
        let backend = TestBackend::new(30, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_sidebar_with_active_session() {
        let mut app = make_sidebar_app();
        let sid = app.session_id.clone();
        app.session_list = vec![
            make_test_session(sid.as_str(), "active session"),
            make_test_session("other", "other session"),
        ];
        let backend = TestBackend::new(30, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_sidebar_with_hover() {
        let mut app = make_sidebar_app();
        app.session_list = vec![
            make_test_session("s1", "first"),
            make_test_session("s2", "second"),
        ];
        app.sidebar_hover = Some(1);
        let backend = TestBackend::new(30, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_sidebar_focused() {
        let mut app = make_sidebar_app();
        app.sidebar_focused = true;
        app.session_list = vec![
            make_test_session("s1", "first"),
            make_test_session("s2", "second"),
        ];
        app.sidebar_hover = Some(0);
        let backend = TestBackend::new(30, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_sidebar_focused_with_hover() {
        let mut app = make_sidebar_app();
        app.sidebar_focused = true;
        app.session_list = vec![
            make_test_session("s1", "first"),
            make_test_session("s2", "second"),
        ];
        app.sidebar_hover = Some(1);
        let backend = TestBackend::new(30, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_sidebar_with_scroll() {
        let mut app = make_sidebar_app();
        for i in 0..20 {
            app.session_list
                .push(make_test_session(&format!("s{i}"), &format!("session {i}")));
        }
        app.sidebar_scroll = 5;
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&app, frame, frame.area());
            })
            .unwrap();
    }
}
