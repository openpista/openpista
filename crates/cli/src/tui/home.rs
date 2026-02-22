//! Home/welcome screen widget — centered logo, input box, and keyboard shortcuts.
use unicode_width::UnicodeWidthStr;

use super::app::{AppState, TuiApp};
use super::theme::THEME;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

/// Renders the home screen with centered logo, input box, model info, and keyboard shortcuts.
pub fn render(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
    // We want to center the whole block vertically and horizontally.
    // Let's create a centered layout.
    let vertical_layout = Layout::vertical([
        Constraint::Length(10), // Logo area (Height is 17 lines + padding)
        Constraint::Length(5),  // Input area
        Constraint::Length(5),  // Info & hints
    ])
    .flex(Flex::Center)
    .split(area);

    let logo_area = vertical_layout[0];
    let input_area = vertical_layout[1];
    let hints_area = vertical_layout[2];

    // 1. Render Logo
    let logo_style = Style::default().fg(THEME.logo).add_modifier(Modifier::BOLD);
    let logo_str = concat!(
        "                            _     _      \n",
        "  ___  _ __  ___ _ __  _ __(_)___| |_ __ _\n",
        " / _ \\| '_ \\/ _ \\ '_ \\| '_ \\| / __| __/ _` |\n",
        "| (_) | |_) |  __/ | | | |_) | \\__ \\ || (_| |\n",
        " \\___/| .__/ \\___|_| |_| .__/|_|___/\\__\\__,_|\n",
        "      |_|              |_|                   "
    );
    let logo_text = Text::styled(logo_str, logo_style);

    // To center it horizontally
    let logo_h_layout = Layout::horizontal([Constraint::Length(52)])
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
        THEME.border_active
    } else {
        THEME.fg_muted
    };
    let display_text = if app.input.is_empty() && app.state == AppState::Idle {
        " Ask anything... \"Fix broken tests\" "
    } else {
        &app.input
    };
    let input_style = if app.input.is_empty() && app.state == AppState::Idle {
        Style::default().fg(THEME.fg_muted)
    } else {
        Style::default()
    };
    let input_block = Block::default().borders(Borders::LEFT).border_style(
        Style::default()
            .fg(border_color)
            .add_modifier(Modifier::BOLD),
    );

    let input_widget = Paragraph::new(Span::styled(display_text, input_style))
        .block(input_block)
        .style(Style::default().bg(THEME.home_input_bg));

    // To give it some vertical padding within its area
    let inner_input_v = Layout::vertical([Constraint::Length(3)])
        .flex(Flex::Center)
        .split(input_h_layout[0]);
    frame.render_widget(input_widget, inner_input_v[0]);

    if app.is_palette_active() {
        app.render_command_palette(frame, inner_input_v[0]);
    }

    // Show cursor when idle
    if app.state == AppState::Idle {
        let cursor_col = UnicodeWidthStr::width(&app.input[..app.cursor_pos]) as u16;
        frame.set_cursor_position((inner_input_v[0].x + 1 + cursor_col, inner_input_v[0].y));
    }

    // 3. Render Hints
    let hints_h_layout = Layout::horizontal([Constraint::Length(80)])
        .flex(Flex::Center)
        .split(hints_area);

    let model_line = Line::from(vec![
        Span::styled(
            format!("{} ", app.model_name),
            Style::default().fg(THEME.accent_bright),
        ),
        Span::styled(
            "· high",
            Style::default()
                .fg(THEME.warning)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let shortcuts_line = Line::from(vec![
        Span::styled(
            "ctrl+t ",
            Style::default()
                .fg(THEME.home_shortcut_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("variants  ", Style::default().fg(THEME.home_shortcut_desc)),
        Span::styled(
            "tab ",
            Style::default()
                .fg(THEME.home_shortcut_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("agents  ", Style::default().fg(THEME.home_shortcut_desc)),
        Span::styled(
            "ctrl+p ",
            Style::default()
                .fg(THEME.home_shortcut_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("commands", Style::default().fg(THEME.home_shortcut_desc)),
    ]);

    let tip_line = Line::from(vec![
        Span::styled("● ", Style::default().fg(THEME.home_tip_icon)),
        Span::styled(
            "Tip ",
            Style::default()
                .fg(THEME.home_tip_icon)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Drag and drop images into the terminal to add them as context",
            Style::default().fg(THEME.fg_muted),
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
