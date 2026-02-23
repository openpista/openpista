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

    #[test]
    fn selection_cols_row_before_selection_returns_zero() {
        // screen_row < sr => (0, 0)
        let (s, e) = selection_cols_for_row(0, (2, 5), (4, 8), 80);
        assert_eq!((s, e), (0, 0));
    }

    #[test]
    fn selection_cols_row_after_selection_returns_zero() {
        // screen_row > er => (0, 0)
        let (s, e) = selection_cols_for_row(5, (2, 5), (4, 8), 80);
        assert_eq!((s, e), (0, 0));
    }

    #[test]
    fn selection_cols_single_row_selection() {
        // sr == er => (sc, ec)
        let (s, e) = selection_cols_for_row(3, (3, 10), (3, 20), 80);
        assert_eq!((s, e), (10, 20));
    }

    #[test]
    fn selection_cols_first_row_of_multi_row() {
        // screen_row == sr, sr != er => (sc, width)
        let (s, e) = selection_cols_for_row(2, (2, 5), (4, 8), 80);
        assert_eq!((s, e), (5, 80));
    }

    #[test]
    fn selection_cols_last_row_of_multi_row() {
        // screen_row == er, sr != er => (0, ec)
        let (s, e) = selection_cols_for_row(4, (2, 5), (4, 8), 80);
        assert_eq!((s, e), (0, 8));
    }

    #[test]
    fn selection_cols_middle_row_of_multi_row() {
        // sr < screen_row < er => (0, width)
        let (s, e) = selection_cols_for_row(3, (2, 5), (5, 8), 80);
        assert_eq!((s, e), (0, 80));
    }
}
