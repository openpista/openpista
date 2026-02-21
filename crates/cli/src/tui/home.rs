use super::app::{AppState, TuiApp};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

pub fn render(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
    // We want to center the whole block vertically and horizontally.
    // Let's create a centered layout.
    let vertical_layout = Layout::vertical([
        Constraint::Length(10), // Logo area
        Constraint::Length(5),  // Input area
        Constraint::Length(5),  // Info & hints
    ])
    .flex(Flex::Center)
    .split(area);

    let logo_area = vertical_layout[0];
    let input_area = vertical_layout[1];
    let hints_area = vertical_layout[2];

    // 1. Render Logo
    let logo_style = Style::default().fg(Color::Rgb(115, 138, 172)); // A nice muted blue
    let logo_str = "  ___  ____  _____  _  _  ___  ___  ____  ____ \n \
                   / _ \\(  _ \\(  _  )( \\( )/ __)/ _ \\(    \\(  __)\n\
                  ( (_) )) __/ )(_)(  )  (( (__( (_) )) D ( ) _) \n \
                   \\___/(__)  (_____)(_)\\_)\\___)\\___/(____/(____)";
    let logo_text = Text::styled(logo_str, logo_style);

    // To center it horizontally
    let logo_h_layout = Layout::horizontal([Constraint::Length(64)])
        .flex(Flex::Center)
        .split(logo_area);
    frame.render_widget(
        Paragraph::new(logo_text).alignment(Alignment::Center),
        logo_h_layout[0],
    );

    // 2. Render Input Box
    let input_h_layout = Layout::horizontal([Constraint::Length(80)])
        .flex(Flex::Center)
        .split(input_area);
    let border_color = if app.state == AppState::Idle {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let display_text = if app.input.is_empty() && app.state == AppState::Idle {
        " Ask anything... \"Fix broken tests\" "
    } else {
        &app.input
    };
    let input_style = if app.input.is_empty() && app.state == AppState::Idle {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    // We'll mimic the multi-line block from the original code
    let input_block = Block::default().borders(Borders::LEFT).border_style(
        Style::default()
            .fg(border_color)
            .add_modifier(Modifier::BOLD),
    );

    let input_widget = Paragraph::new(Span::styled(display_text, input_style))
        .block(input_block)
        .style(Style::default().bg(Color::Rgb(40, 44, 52))); // slightly lighter background for input

    // To give it some vertical padding within its area
    let inner_input_v = Layout::vertical([Constraint::Length(3)])
        .flex(Flex::Center)
        .split(input_h_layout[0]);
    frame.render_widget(input_widget, inner_input_v[0]);

    // Show cursor when idle
    if app.state == AppState::Idle {
        let cursor_col = app.input[..app.cursor_pos].chars().count() as u16;
        frame.set_cursor_position((inner_input_v[0].x + 1 + cursor_col, inner_input_v[0].y));
    }

    // 3. Render Hints
    let hints_h_layout = Layout::horizontal([Constraint::Length(80)])
        .flex(Flex::Center)
        .split(hints_area);

    // Model info
    let model_line = Line::from(vec![
        Span::styled(
            format!("{} ", app.model_name),
            Style::default().fg(Color::LightCyan),
        ),
        Span::styled(
            "· high",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    // Shortcuts
    let shortcuts_line = Line::from(vec![
        Span::styled(
            "ctrl+t ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("variants  ", Style::default().fg(Color::Rgb(115, 138, 172))),
        Span::styled(
            "tab ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("agents  ", Style::default().fg(Color::Rgb(115, 138, 172))),
        Span::styled(
            "ctrl+p ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("commands", Style::default().fg(Color::Rgb(115, 138, 172))),
    ]);

    // Tip
    let tip_line = Line::from(vec![
        Span::styled("● ", Style::default().fg(Color::Yellow)),
        Span::styled(
            "Tip ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Drag and drop images into the terminal to add them as context",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let info_text = Text::from(vec![
        model_line,
        Line::from(""),
        shortcuts_line.alignment(Alignment::Right),
        Line::from(""),
        Line::from(""),
        tip_line.alignment(Alignment::Center),
    ]);

    frame.render_widget(Paragraph::new(info_text), hints_h_layout[0]);
}
