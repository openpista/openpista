//! Chat history widget — renders messages, tool calls, and text selection overlay.

use super::app::{TuiApp, TuiMessage};
use super::selection::compute_text_grid;
use super::theme::THEME;
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Renders the chat history area with user/assistant messages, tool calls, and text selection overlay.
pub fn render(app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    for msg in &app.messages {
        match msg {
            TuiMessage::User(text) => {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled(
                        "You: ",
                        Style::default()
                            .fg(THEME.user_label)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(text.as_str()),
                ]));
            }
            TuiMessage::Assistant(text) => {
                lines.push(Line::from(""));
                let mut first = true;
                for line in text.lines() {
                    if first {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "Agent: ",
                                Style::default()
                                    .fg(THEME.assistant_label)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(line.to_string()),
                        ]));
                        first = false;
                    } else {
                        lines.push(Line::from(Span::raw(format!("       {line}"))));
                    }
                }
            }
            TuiMessage::ToolCall {
                tool_name,
                args_preview,
                done,
            } => {
                let status = if *done { "✓" } else { "⟳" };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  [{status} {tool_name}] "),
                        Style::default().fg(THEME.tool_call),
                    ),
                    Span::styled(args_preview, Style::default().fg(THEME.fg_muted)),
                ]));
            }
            TuiMessage::ToolResult {
                tool_name,
                output_preview,
                is_error,
            } => {
                let color = if *is_error {
                    THEME.error
                } else {
                    THEME.tool_result
                };
                lines.push(Line::from(Span::styled(
                    format!("    [{tool_name}] → {output_preview}"),
                    Style::default().fg(color),
                )));
            }
            TuiMessage::Error(text) => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("Error: {text}"),
                    Style::default()
                        .fg(THEME.error)
                        .add_modifier(Modifier::BOLD),
                )));
            }
        }
    }

    // Inner width (area minus 1-cell border on each side).
    let inner_width = area.width.saturating_sub(2);

    // Build the character grid before `lines` is consumed.
    let grid = compute_text_grid(&lines, inner_width);

    let content_height = grid.len() as u16;
    let visible_height = area.height.saturating_sub(2);
    let max_scroll = content_height.saturating_sub(visible_height);
    let scroll = app.history_scroll.min(max_scroll);

    let history = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.fg_muted)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(history, area);

    // Persist grid, area, and clamped scroll for mouse hit-testing and text extraction.
    app.chat_text_grid = grid;
    app.chat_area = Some(area);
    app.chat_scroll_clamped = scroll;

    // ── Selection highlight overlay ──────────────────────────────────────────
    if let Some((start, end)) = app.text_selection.ordered_range() {
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };

        let buf = frame.buffer_mut();

        for screen_row in 0..inner.height {
            let (c_start, c_end) = selection_cols_for_row(screen_row, start, end, inner.width);
            for col in c_start..c_end {
                let pos = Position {
                    x: inner.x + col,
                    y: inner.y + screen_row,
                };
                if let Some(cell) = buf.cell_mut(pos) {
                    cell.set_bg(THEME.selection_bg);
                    cell.set_fg(THEME.selection_fg);
                }
            }
        }
    }
}

/// Returns the `(start_col, end_col)` range (exclusive end) to highlight on
/// `screen_row` given a selection from `start` to `end` (both screen-relative).
fn selection_cols_for_row(
    screen_row: u16,
    start: (u16, u16),
    end: (u16, u16),
    width: u16,
) -> (u16, u16) {
    let (sr, sc) = start;
    let (er, ec) = end;

    if screen_row < sr || screen_row > er {
        return (0, 0);
    }

    if sr == er {
        (sc, ec)
    } else if screen_row == sr {
        (sc, width)
    } else if screen_row == er {
        (0, ec)
    } else {
        (0, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::TuiApp;
    use proto::{ChannelId, SessionId};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    fn test_app() -> TuiApp {
        TuiApp::new(
            "gpt-4o",
            SessionId::new(),
            ChannelId::from("cli:tui"),
            "openai",
        )
    }

    // ── selection_cols_for_row ─────────────────────────────────────────────

    #[test]
    fn selection_row_before_range_returns_zero() {
        assert_eq!(selection_cols_for_row(0, (2, 3), (5, 8), 80), (0, 0));
    }

    #[test]
    fn selection_row_after_range_returns_zero() {
        assert_eq!(selection_cols_for_row(6, (2, 3), (5, 8), 80), (0, 0));
    }

    #[test]
    fn selection_single_row() {
        // sr == er → returns (sc, ec)
        assert_eq!(selection_cols_for_row(3, (3, 5), (3, 15), 80), (5, 15));
    }

    #[test]
    fn selection_multi_row_first_row() {
        // screen_row == sr → (sc, width)
        assert_eq!(selection_cols_for_row(2, (2, 5), (7, 10), 80), (5, 80));
    }

    #[test]
    fn selection_multi_row_last_row() {
        // screen_row == er → (0, ec)
        assert_eq!(selection_cols_for_row(7, (2, 5), (7, 10), 80), (0, 10));
    }

    #[test]
    fn selection_multi_row_middle_row() {
        // middle row → (0, width)
        assert_eq!(selection_cols_for_row(4, (2, 5), (7, 10), 80), (0, 80));
    }

    // ── render function ──────────────────────────────────────────────────

    fn render_chat(app: &mut TuiApp, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(app, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn render_empty_messages() {
        let mut app = test_app();
        render_chat(&mut app, 80, 24);
        assert!(app.chat_text_grid.is_empty());
        assert_eq!(app.chat_area, Some(Rect::new(0, 0, 80, 24)));
        assert_eq!(app.chat_scroll_clamped, 0);
    }

    #[test]
    fn render_user_messages() {
        let mut app = test_app();
        app.messages
            .push(TuiMessage::User("hello world".to_string()));
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
        assert_eq!(app.chat_area, Some(Rect::new(0, 0, 80, 24)));
        // User message produces 2 lines: blank + "You: hello world"
        assert!(app.chat_text_grid.len() >= 2);
    }

    #[test]
    fn render_assistant_single_line() {
        let mut app = test_app();
        app.messages
            .push(TuiMessage::Assistant("single line".to_string()));
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
        // blank + "Agent: single line"
        assert!(app.chat_text_grid.len() >= 2);
    }

    #[test]
    fn render_assistant_multi_line() {
        let mut app = test_app();
        app.messages.push(TuiMessage::Assistant(
            "line one\nline two\nline three".to_string(),
        ));
        render_chat(&mut app, 80, 24);
        // blank + "Agent: line one" + "       line two" + "       line three"
        assert!(app.chat_text_grid.len() >= 4);
    }

    #[test]
    fn render_tool_call_done() {
        let mut app = test_app();
        app.messages.push(TuiMessage::ToolCall {
            tool_name: "system.run".to_string(),
            args_preview: "ls -la".to_string(),
            done: true,
        });
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
    }

    #[test]
    fn render_tool_call_in_progress() {
        let mut app = test_app();
        app.messages.push(TuiMessage::ToolCall {
            tool_name: "bash".to_string(),
            args_preview: "echo hi".to_string(),
            done: false,
        });
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
    }

    #[test]
    fn render_tool_result_success() {
        let mut app = test_app();
        app.messages.push(TuiMessage::ToolResult {
            tool_name: "system.run".to_string(),
            output_preview: "file1.txt\nfile2.txt".to_string(),
            is_error: false,
        });
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
    }

    #[test]
    fn render_tool_result_error() {
        let mut app = test_app();
        app.messages.push(TuiMessage::ToolResult {
            tool_name: "system.run".to_string(),
            output_preview: "command not found".to_string(),
            is_error: true,
        });
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
    }

    #[test]
    fn render_error_message() {
        let mut app = test_app();
        app.messages
            .push(TuiMessage::Error("something went wrong".to_string()));
        render_chat(&mut app, 80, 24);
        assert!(!app.chat_text_grid.is_empty());
        // blank + "Error: something went wrong"
        assert!(app.chat_text_grid.len() >= 2);
    }

    #[test]
    fn render_mixed_messages_sets_chat_area() {
        let mut app = test_app();
        app.messages.push(TuiMessage::User("q".to_string()));
        app.messages.push(TuiMessage::Assistant("a".to_string()));
        app.messages.push(TuiMessage::ToolCall {
            tool_name: "t".to_string(),
            args_preview: "x".to_string(),
            done: true,
        });
        app.messages.push(TuiMessage::ToolResult {
            tool_name: "t".to_string(),
            output_preview: "ok".to_string(),
            is_error: false,
        });
        app.messages.push(TuiMessage::Error("err".to_string()));

        render_chat(&mut app, 100, 30);
        assert!(app.chat_area.is_some());
        assert!(!app.chat_text_grid.is_empty());
    }

    #[test]
    fn render_with_text_selection_active() {
        let mut app = test_app();
        app.messages
            .push(TuiMessage::User("hello world".to_string()));
        app.messages
            .push(TuiMessage::Assistant("response text".to_string()));

        // Set up a selection spanning rows 0-1
        app.text_selection.anchor = Some((0, 2));
        app.text_selection.endpoint = Some((1, 5));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(&mut app, frame, frame.area());
            })
            .unwrap();

        // Selection overlay should have been applied; verify state is persisted
        assert!(app.chat_area.is_some());
        assert!(!app.chat_text_grid.is_empty());
    }

    #[test]
    fn render_scroll_clamped_to_max() {
        let mut app = test_app();
        // Push enough messages to overflow a small viewport
        for i in 0..50 {
            app.messages.push(TuiMessage::User(format!("message {i}")));
        }
        app.history_scroll = 9999; // Excessively high scroll
        render_chat(&mut app, 60, 10);
        // Scroll should be clamped to max_scroll
        let content_height = app.chat_text_grid.len() as u16;
        let visible_height = 10u16.saturating_sub(2);
        let expected_max = content_height.saturating_sub(visible_height);
        assert_eq!(app.chat_scroll_clamped, expected_max);
    }
}
