use super::app::{TuiApp, TuiMessage};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub fn render(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    for msg in &app.messages {
        match msg {
            TuiMessage::User(text) => {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled(
                        "You: ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(text),
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
                                    .fg(Color::Green)
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
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(args_preview, Style::default().fg(Color::DarkGray)),
                ]));
            }
            TuiMessage::ToolResult {
                tool_name,
                output_preview,
                is_error,
            } => {
                let color = if *is_error {
                    Color::Red
                } else {
                    Color::DarkGray
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
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )));
            }
        }
    }

    let content_height = lines.len() as u16;
    let visible_height = area.height;
    let max_scroll = content_height.saturating_sub(visible_height);
    let scroll = app.history_scroll.min(max_scroll);

    let history = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(history, area);
}
