//! TUI application state, rendering, and input handling.
#![allow(dead_code)]

use proto::{ChannelId, ProgressEvent, SessionId};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Spinner animation frames (Braille pattern).
const SPINNER: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

// ─── Data types ──────────────────────────────────────────────

/// A single rendered item in the conversation history panel.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TuiMessage {
    /// User typed this message.
    User(String),
    /// Assistant final response text.
    Assistant(String),
    /// An in-progress or completed tool call.
    ToolCall {
        tool_name: String,
        args_preview: String,
        done: bool,
    },
    /// A tool call that has completed with output.
    ToolResult {
        tool_name: String,
        output_preview: String,
        is_error: bool,
    },
    /// An error from the agent runtime.
    Error(String),
}

/// High-level processing state.
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    /// No agent task running; input box is active.
    Idle,
    /// Waiting for LLM response (spinner shown).
    Thinking { round: usize },
    /// A tool call is executing.
    ExecutingTool { tool_name: String },
}

// ─── TuiApp ──────────────────────────────────────────────────

/// Full state for the TUI session.
pub struct TuiApp {
    /// Ordered conversation history for display.
    pub messages: Vec<TuiMessage>,
    /// Current text typed in the input box (not yet submitted).
    pub input: String,
    /// Cursor position within `input` (byte offset).
    pub cursor_pos: usize,
    /// Current high-level processing state.
    pub state: AppState,
    /// Vertical scroll offset for the history panel.
    pub history_scroll: u16,
    /// Model name shown in the status bar.
    pub model_name: String,
    /// Session identifier.
    pub session_id: SessionId,
    /// Channel id for this TUI session.
    #[allow(dead_code)]
    pub channel_id: ChannelId,
    /// Spinner animation tick counter.
    pub spinner_tick: u8,
    /// Whether the user requested exit.
    pub should_quit: bool,
}

impl TuiApp {
    /// Create a new TUI application state.
    pub fn new(
        model_name: impl Into<String>,
        session_id: SessionId,
        channel_id: ChannelId,
    ) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            state: AppState::Idle,
            history_scroll: 0,
            model_name: model_name.into(),
            session_id,
            channel_id,
            spinner_tick: 0,
            should_quit: false,
        }
    }

    // ── State mutations ──────────────────────────────────────

    /// Push a user message to the history.
    pub fn push_user(&mut self, text: String) {
        self.messages.push(TuiMessage::User(text));
    }

    /// Push an assistant response to the history.
    pub fn push_assistant(&mut self, text: String) {
        self.messages.push(TuiMessage::Assistant(text));
    }

    /// Push an error message to the history.
    pub fn push_error(&mut self, err: String) {
        self.messages.push(TuiMessage::Error(err));
    }

    /// Take the current input and reset it.
    pub fn take_input(&mut self) -> String {
        self.cursor_pos = 0;
        std::mem::take(&mut self.input)
    }

    /// Apply a progress event from the agent runtime.
    pub fn apply_progress(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::LlmThinking { round } => {
                self.state = AppState::Thinking { round };
            }
            ProgressEvent::ToolCallStarted {
                tool_name, args, ..
            } => {
                let args_str = args.to_string();
                let preview = if args_str.len() > 80 {
                    format!("{}…", &args_str[..80])
                } else {
                    args_str
                };
                self.state = AppState::ExecutingTool {
                    tool_name: tool_name.clone(),
                };
                self.messages.push(TuiMessage::ToolCall {
                    tool_name,
                    args_preview: preview,
                    done: false,
                });
            }
            ProgressEvent::ToolCallFinished {
                tool_name,
                output,
                is_error,
                ..
            } => {
                // Mark the last matching ToolCall as done
                for msg in self.messages.iter_mut().rev() {
                    if let TuiMessage::ToolCall {
                        tool_name: name,
                        done,
                        ..
                    } = msg
                        && *name == tool_name
                        && !*done
                    {
                        *done = true;
                        break;
                    }
                }
                let preview = if output.len() > 120 {
                    format!("{}…", &output[..120])
                } else {
                    output
                };
                self.messages.push(TuiMessage::ToolResult {
                    tool_name,
                    output_preview: preview,
                    is_error,
                });
            }
        }
    }

    /// Apply the final result from the agent runtime.
    pub fn apply_completion(&mut self, result: Result<String, proto::Error>) {
        match result {
            Ok(text) => {
                self.push_assistant(text);
            }
            Err(e) => {
                self.push_error(format!("{e}"));
            }
        }
        self.state = AppState::Idle;
    }

    // ── Input handling ───────────────────────────────────────

    /// Handle a keyboard event.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                if self.state == AppState::Idle {
                    self.should_quit = true;
                }
            }
            (_, KeyCode::Char(c)) if self.state == AppState::Idle => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
            (_, KeyCode::Backspace) if self.state == AppState::Idle => {
                if self.cursor_pos > 0 {
                    // Find the previous character boundary
                    let prev = self.input[..self.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input.drain(prev..self.cursor_pos);
                    self.cursor_pos = prev;
                }
            }
            (_, KeyCode::Left) if self.state == AppState::Idle => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = self.input[..self.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            (_, KeyCode::Right) if self.state == AppState::Idle => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = self.input[self.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor_pos + i)
                        .unwrap_or(self.input.len());
                }
            }
            (_, KeyCode::Up) => {
                self.history_scroll = self.history_scroll.saturating_sub(1);
            }
            (_, KeyCode::Down) => {
                self.history_scroll = self.history_scroll.saturating_add(1);
            }
            (_, KeyCode::PageUp) => {
                self.history_scroll = self.history_scroll.saturating_sub(10);
            }
            (_, KeyCode::PageDown) => {
                self.history_scroll = self.history_scroll.saturating_add(10);
            }
            _ => {}
        }
    }

    // ── Rendering ────────────────────────────────────────────

    /// Render the entire TUI into the given frame.
    pub fn render(&self, frame: &mut Frame<'_>) {
        let area = frame.area();

        // Layout: title(1) | history(fill) | status(1) | input(3)
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(area);

        self.render_title(frame, chunks[0]);
        self.render_history(frame, chunks[1]);
        self.render_status(frame, chunks[2]);
        self.render_input(frame, chunks[3]);
    }

    fn render_title(&self, frame: &mut Frame<'_>, area: Rect) {
        let session_prefix = &self.session_id.as_str()[..8.min(self.session_id.as_str().len())];
        let title = Line::from(vec![
            Span::styled(
                " openpista ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" session:{session_prefix} "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!(" {} ", self.model_name),
                Style::default().fg(Color::Green),
            ),
        ]);
        frame.render_widget(Paragraph::new(title), area);
    }

    fn render_history(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines: Vec<Line<'_>> = Vec::new();

        for msg in &self.messages {
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
                    // Split multi-line responses
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

        // Auto-scroll to bottom
        let content_height = lines.len() as u16;
        let visible_height = area.height.saturating_sub(2); // block borders
        let max_scroll = content_height.saturating_sub(visible_height);
        let scroll = self.history_scroll.min(max_scroll);

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

    fn render_status(&self, frame: &mut Frame<'_>, area: Rect) {
        let status_text = match &self.state {
            AppState::Idle => Line::from(Span::styled(
                " Enter:send  ↑↓:scroll  Ctrl+C:quit",
                Style::default().fg(Color::DarkGray),
            )),
            AppState::Thinking { round } => {
                let spinner = SPINNER[(self.spinner_tick as usize) % SPINNER.len()];
                Line::from(vec![
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
                let spinner = SPINNER[(self.spinner_tick as usize) % SPINNER.len()];
                Line::from(vec![
                    Span::styled(
                        format!(" {spinner} Running "),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(tool_name, Style::default().fg(Color::Cyan)),
                ])
            }
        };
        frame.render_widget(Paragraph::new(status_text), area);
    }

    fn render_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let border_color = if self.state == AppState::Idle {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let display_text = if self.input.is_empty() && self.state == AppState::Idle {
            "Type a message..."
        } else {
            &self.input
        };

        let input_style = if self.input.is_empty() && self.state == AppState::Idle {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

        let input = Paragraph::new(Span::styled(display_text, input_style)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(" Input "),
        );

        frame.render_widget(input, area);

        // Show cursor when idle
        if self.state == AppState::Idle {
            // Calculate cursor column (char width up to cursor_pos)
            let cursor_col = self.input[..self.cursor_pos].chars().count() as u16;
            frame.set_cursor_position((area.x + 1 + cursor_col, area.y + 1));
        }
    }

    /// Ensure scroll is at the bottom (for auto-scroll on new messages).
    pub fn scroll_to_bottom(&mut self) {
        // Set to a large value; render_history clamps it to max_scroll.
        self.history_scroll = u16::MAX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app() -> TuiApp {
        TuiApp::new("gpt-4o", SessionId::new(), ChannelId::from("cli:tui"))
    }

    #[test]
    fn apply_progress_tool_started_updates_state() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command":"ls"}),
        });
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], TuiMessage::ToolCall { .. }));
        assert_eq!(
            app.state,
            AppState::ExecutingTool {
                tool_name: "system.run".into()
            }
        );
    }

    #[test]
    fn apply_progress_tool_finished_adds_result() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({}),
        });
        app.apply_progress(ProgressEvent::ToolCallFinished {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            output: "file1.rs\nfile2.rs".into(),
            is_error: false,
        });
        assert_eq!(app.messages.len(), 2);
        assert!(matches!(
            &app.messages[1],
            TuiMessage::ToolResult {
                is_error: false,
                ..
            }
        ));
    }

    #[test]
    fn apply_completion_ok_pushes_assistant() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.apply_completion(Ok("hello world".into()));
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], TuiMessage::Assistant(t) if t == "hello world"));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn apply_completion_err_pushes_error() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        app.apply_completion(Err(proto::Error::Llm(proto::LlmError::RateLimit)));
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(&app.messages[0], TuiMessage::Error(_)));
        assert_eq!(app.state, AppState::Idle);
    }

    #[test]
    fn handle_key_inserts_chars() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.input, "ab");
        assert_eq!(app.cursor_pos, 2);
    }

    #[test]
    fn handle_key_backspace_deletes() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn handle_key_ignores_input_when_thinking() {
        let mut app = make_app();
        app.state = AppState::Thinking { round: 0 };
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.input, "");
    }

    #[test]
    fn take_input_resets() {
        let mut app = make_app();
        app.input = "hello".into();
        app.cursor_pos = 5;
        let taken = app.take_input();
        assert_eq!(taken, "hello");
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn app_state_equality() {
        assert_eq!(AppState::Idle, AppState::Idle);
        assert_eq!(
            AppState::Thinking { round: 1 },
            AppState::Thinking { round: 1 }
        );
        assert_ne!(AppState::Idle, AppState::Thinking { round: 1 });
    }

    #[test]
    fn scroll_to_bottom_sets_max() {
        let mut app = make_app();
        app.scroll_to_bottom();
        assert_eq!(app.history_scroll, u16::MAX);
    }
}
