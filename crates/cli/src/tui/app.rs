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

/// Determines which "view" is active
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Screen {
    #[default]
    Home,
    Chat,
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
    /// Which screen is currently displayed.
    pub screen: Screen,
    /// Workspace name for status bar.
    pub workspace_name: String,
    /// Git branch for status bar.
    pub branch_name: String,
    /// Available MCP servers for status bar.
    pub mcp_count: usize,
    /// Version text.
    pub version: String,
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
            screen: Screen::Home,
            workspace_name: "~/openpista".into(),
            branch_name: "main".into(),
            mcp_count: 0,
            version: env!("CARGO_PKG_VERSION").into(),
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
            (_, KeyCode::Enter) if self.state == AppState::Idle => {
                // If Enter is pressed, make sure we are heavily into the Chat screen
                if self.screen == Screen::Home {
                    self.screen = Screen::Chat;
                }
                // (The event loop will then extract `self.take_input()` when handling this)
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

        match self.screen {
            Screen::Home => {
                // Layout for home: content(fill) | status(1)
                let chunks =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

                crate::tui::home::render(self, frame, chunks[0]);
                crate::tui::status::render(self, frame, chunks[1]);
            }
            Screen::Chat => {
                // Layout for chat: title(1) | history(fill) | status(1) | input(3)
                let chunks = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(3),
                ])
                .split(area);

                self.render_title(frame, chunks[0]);
                crate::tui::chat::render(self, frame, chunks[1]);
                crate::tui::status::render(self, frame, chunks[2]);
                self.render_input(frame, chunks[3]);
            }
        }
    }

    fn render_title(&self, frame: &mut Frame<'_>, area: Rect) {
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
    use ratatui::{Terminal, backend::TestBackend};

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
    fn apply_progress_llm_thinking_sets_state_round() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::LlmThinking { round: 3 });
        assert_eq!(app.state, AppState::Thinking { round: 3 });
    }

    #[test]
    fn apply_progress_marks_latest_matching_tool_call_done() {
        let mut app = make_app();
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c1".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command":"echo 1"}),
        });
        app.apply_progress(ProgressEvent::ToolCallStarted {
            call_id: "c2".into(),
            tool_name: "system.run".into(),
            args: serde_json::json!({"command":"echo 2"}),
        });
        app.apply_progress(ProgressEvent::ToolCallFinished {
            call_id: "c2".into(),
            tool_name: "system.run".into(),
            output: "ok".into(),
            is_error: false,
        });

        assert!(matches!(
            &app.messages[0],
            TuiMessage::ToolCall { done: false, .. }
        ));
        assert!(matches!(
            &app.messages[1],
            TuiMessage::ToolCall { done: true, .. }
        ));
    }

    #[test]
    fn handle_key_moves_cursor_left_and_right_with_utf8() {
        let mut app = make_app();
        app.input = "a한b".into();
        app.cursor_pos = app.input.len();

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a한".len());

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a".len());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a한".len());

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.cursor_pos, "a한b".len());
    }

    #[test]
    fn handle_key_scroll_shortcuts_update_history_scroll() {
        let mut app = make_app();
        app.history_scroll = 5;

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 4);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 5);

        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.history_scroll, 10);
    }

    #[test]
    fn handle_key_quit_shortcuts_only_when_idle() {
        let mut app = make_app();
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.should_quit);

        app.should_quit = false;
        app.state = AppState::Thinking { round: 1 };
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.should_quit);

        app.state = AppState::Idle;
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn render_draws_all_message_variants_without_mutating_state() {
        let mut app = make_app();
        app.push_user("user line".into());
        app.push_assistant("first\nsecond".into());
        app.messages.push(TuiMessage::ToolCall {
            tool_name: "system.run".into(),
            args_preview: "{\"command\":\"echo ok\"}".into(),
            done: false,
        });
        app.messages.push(TuiMessage::ToolResult {
            tool_name: "system.run".into(),
            output_preview: "ok".into(),
            is_error: false,
        });
        app.push_error("boom".into());
        app.input = "typed".into();
        app.cursor_pos = 2;
        app.state = AppState::ExecutingTool {
            tool_name: "system.run".into(),
        };
        app.spinner_tick = 3;
        app.history_scroll = 7;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.input, "typed");
        assert_eq!(app.cursor_pos, 2);
        assert_eq!(app.history_scroll, 7);
        assert_eq!(app.messages.len(), 5);
    }

    #[test]
    fn render_idle_placeholder_path_executes() {
        let mut app = make_app();
        app.state = AppState::Idle;
        app.input.clear();
        app.cursor_pos = 0;

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.state, AppState::Idle);
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
