//! TUI input handling methods for TuiApp.

use crate::auth_picker::{self, AuthLoginIntent, AuthMethodChoice, LoginBrowseStep};
use crate::config::LoginAuthMode;
use crate::tui::action::Action;
use proto::SessionId;

use super::state::*;

impl TuiApp {
    /// Handle a keyboard event.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        if matches!(self.state, AppState::QrCodeDisplay { .. }) {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.state = AppState::Idle;
                }
                _ => {}
            }
            return;
        }

        // ── Tool Approval prompt ────────────────────────────
        if self.chat.pending_approval.is_some() {
            let decision = match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    Some(proto::ToolApprovalDecision::Approve)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    Some(proto::ToolApprovalDecision::Reject)
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    Some(proto::ToolApprovalDecision::AllowForSession)
                }
                _ => None,
            };
            if let Some(decision) = decision
                && let Some(pending) = self.chat.pending_approval.take()
            {
                let _ = pending.reply_tx.send(decision);
            }
            return;
        }

        // ── ConfirmDelete state ──────────────────────────────
        if let AppState::ConfirmDelete { session_id, .. } = &self.state {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let id = SessionId::from(session_id.clone());
                    self.session.confirmed_delete = Some(id);
                    self.state = AppState::Idle;
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.state = AppState::Idle;
                }
                _ => {}
            }
            return;
        }

        // ── Sidebar focused state ───────────────────────────
        if self.sidebar.focused && self.state == AppState::Idle {
            match (key.modifiers, key.code) {
                (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                    let max = self.session.session_list.len().saturating_sub(1);
                    let current = self.sidebar.hover.unwrap_or(0);
                    self.sidebar.hover = Some((current + 1).min(max));
                }
                (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                    let current = self.sidebar.hover.unwrap_or(0);
                    self.sidebar.hover = Some(current.saturating_sub(1));
                }
                (_, KeyCode::Enter) => {
                    self.select_sidebar_session();
                }
                (_, KeyCode::Char('d')) | (_, KeyCode::Delete) => {
                    self.request_delete_session();
                }
                (_, KeyCode::Esc) | (_, KeyCode::Tab) => {
                    self.sidebar.focused = false;
                }
                _ => {}
            }
            return;
        }

        // ── WhatsApp pairing ────────────────────────────────────────
        if matches!(self.state, AppState::WhatsAppSetup { .. }) {
            if key.code == KeyCode::Esc {
                self.state = AppState::Idle;
                self.push_assistant("WhatsApp pairing cancelled.".to_string());
            }
            return;
        }

        let login_browsing = matches!(self.state, AppState::LoginBrowsing(_));
        if login_browsing {
            let mut should_clamp = false;
            let mut pending_intent: Option<AuthLoginIntent> = None;
            let mut close_browser = false;

            if let AppState::LoginBrowsing(LoginBrowsingState {
                query,
                cursor,
                step,
                selected_provider,
                selected_method,
                input_buffer,
                masked_buffer,
                last_error,
                endpoint,
                ..
            }) = &mut self.state
            {
                match step {
                    LoginBrowseStep::SelectProvider => {
                        let providers = auth_picker::filtered_provider_entries(query);
                        match (key.modifiers, key.code) {
                            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                                close_browser = true;
                            }
                            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                                *cursor = cursor.saturating_sub(1);
                                should_clamp = true;
                            }
                            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                                *cursor = cursor.saturating_add(1);
                                should_clamp = true;
                            }
                            (_, KeyCode::PageUp) => {
                                *cursor = cursor.saturating_sub(10);
                                should_clamp = true;
                            }
                            (_, KeyCode::PageDown) => {
                                *cursor = cursor.saturating_add(10);
                                should_clamp = true;
                            }
                            (_, KeyCode::Backspace) => {
                                query.pop();
                                *cursor = 0;
                                should_clamp = true;
                            }
                            (_, KeyCode::Char(c)) => {
                                query.push(c);
                                *cursor = 0;
                                should_clamp = true;
                            }
                            (_, KeyCode::Enter) => {
                                if providers.is_empty() {
                                    *last_error = Some(format!("No matches for '{}'.", query));
                                } else if let Some(selected) = providers.get(*cursor).copied() {
                                    *selected_provider = Some(selected.name.to_string());
                                    *selected_method = None;
                                    input_buffer.clear();
                                    masked_buffer.clear();
                                    *endpoint = None;
                                    *last_error = None;
                                    *cursor = 0;
                                    match selected.auth_mode {
                                        LoginAuthMode::None => {
                                            *last_error = Some(format!(
                                                "Provider '{}' does not require login.",
                                                selected.display_name
                                            ));
                                        }
                                        LoginAuthMode::OAuth => {
                                            if selected.name == "openai"
                                                || selected.name == "anthropic"
                                            {
                                                *step = LoginBrowseStep::SelectMethod;
                                            } else {
                                                pending_intent = Some(AuthLoginIntent {
                                                    provider: selected.name.to_string(),
                                                    auth_method: AuthMethodChoice::OAuth,
                                                    endpoint: None,
                                                    api_key: None,
                                                });
                                            }
                                        }
                                        LoginAuthMode::ApiKey => {
                                            *step = LoginBrowseStep::InputApiKey;
                                        }
                                        LoginAuthMode::EndpointAndKey => {
                                            *step = LoginBrowseStep::InputEndpoint;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    LoginBrowseStep::SelectMethod => match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                        (_, KeyCode::Esc) => {
                            *step = LoginBrowseStep::SelectProvider;
                            *cursor = 0;
                            should_clamp = true;
                        }
                        (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                            *cursor = cursor.saturating_sub(1);
                            should_clamp = true;
                        }
                        (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                            *cursor = cursor.saturating_add(1);
                            should_clamp = true;
                        }
                        (_, KeyCode::Enter) => {
                            let provider = selected_provider.clone().unwrap_or_default();
                            if provider.is_empty() {
                                *step = LoginBrowseStep::SelectProvider;
                                *last_error = Some(
                                    "Provider selection was cleared. Select provider again."
                                        .to_string(),
                                );
                            } else if *cursor == 0 {
                                *selected_method = Some(AuthMethodChoice::OAuth);
                                pending_intent = Some(AuthLoginIntent {
                                    provider,
                                    auth_method: AuthMethodChoice::OAuth,
                                    endpoint: None,
                                    api_key: None,
                                });
                            } else {
                                *selected_method = Some(AuthMethodChoice::ApiKey);
                                input_buffer.clear();
                                masked_buffer.clear();
                                *step = LoginBrowseStep::InputApiKey;
                            }
                        }
                        _ => {}
                    },
                    LoginBrowseStep::InputEndpoint => match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                        (_, KeyCode::Esc) => {
                            *step = LoginBrowseStep::SelectProvider;
                            *cursor = 0;
                            input_buffer.clear();
                        }
                        (_, KeyCode::Backspace) => {
                            input_buffer.pop();
                        }
                        (_, KeyCode::Enter) => {
                            let value = input_buffer.trim().to_string();
                            if value.is_empty() {
                                *last_error = Some("Endpoint is required.".to_string());
                            } else {
                                *endpoint = Some(value);
                                input_buffer.clear();
                                *step = LoginBrowseStep::InputApiKey;
                                *last_error = None;
                            }
                        }
                        (_, KeyCode::Char(c)) => {
                            input_buffer.push(c);
                        }
                        _ => {}
                    },
                    LoginBrowseStep::InputApiKey => match (key.modifiers, key.code) {
                        (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                        (_, KeyCode::Esc) => {
                            let provider = selected_provider.clone().unwrap_or_default();
                            if let Some(entry) = auth_picker::provider_by_name(&provider) {
                                if matches!(
                                    auth_picker::provider_step_for_entry(&entry),
                                    LoginBrowseStep::SelectMethod
                                ) {
                                    *step = LoginBrowseStep::SelectMethod;
                                    *cursor =
                                        if matches!(selected_method, Some(AuthMethodChoice::OAuth))
                                        {
                                            0
                                        } else {
                                            1
                                        };
                                } else if matches!(entry.auth_mode, LoginAuthMode::EndpointAndKey) {
                                    *step = LoginBrowseStep::InputEndpoint;
                                    input_buffer.clear();
                                    if let Some(saved_endpoint) = endpoint.as_ref() {
                                        input_buffer.push_str(saved_endpoint);
                                    }
                                } else {
                                    *step = LoginBrowseStep::SelectProvider;
                                    *cursor = 0;
                                }
                            } else {
                                *step = LoginBrowseStep::SelectProvider;
                                *cursor = 0;
                            }
                            masked_buffer.clear();
                        }
                        (_, KeyCode::Backspace) => {
                            if input_buffer.pop().is_some() {
                                masked_buffer.pop();
                            }
                        }
                        (_, KeyCode::Enter) => {
                            let provider = selected_provider.clone().unwrap_or_default();
                            let api_key = input_buffer.trim().to_string();
                            if provider.is_empty() {
                                *last_error = Some(
                                    "Provider selection was cleared. Select provider again."
                                        .to_string(),
                                );
                                *step = LoginBrowseStep::SelectProvider;
                            } else if api_key.is_empty() {
                                *last_error = Some("API key is required.".to_string());
                            } else {
                                pending_intent = Some(AuthLoginIntent {
                                    provider: provider.clone(),
                                    auth_method: auth_picker::api_key_method_for_provider(
                                        &provider,
                                        *selected_method,
                                    ),
                                    endpoint: endpoint.clone(),
                                    api_key: Some(api_key),
                                });
                            }
                        }
                        (_, KeyCode::Char(c)) => {
                            input_buffer.push(c);
                            masked_buffer.push('*');
                        }
                        _ => {}
                    },
                }
            }

            if close_browser {
                self.state = AppState::Idle;
                self.push_assistant("Login cancelled.".to_string());
                return;
            }
            if should_clamp {
                self.clamp_login_cursor();
            }
            if let Some(intent) = pending_intent {
                self.model.pending_auth_intent = Some(intent.clone());
                self.state = AppState::AuthValidating {
                    provider: intent.provider,
                };
            }
            return;
        }

        let browsing = matches!(self.state, AppState::ModelBrowsing { .. });
        if browsing {
            let mut close_browser = false;
            let mut apply_selected = false;
            let mut should_clamp = false;

            if let AppState::ModelBrowsing {
                query,
                cursor,
                scroll,
                search_active,
                ..
            } = &mut self.state
            {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                    (_, KeyCode::Esc) => {
                        if *search_active {
                            *search_active = false;
                        } else {
                            close_browser = true;
                        }
                    }
                    (_, KeyCode::Char('s')) | (_, KeyCode::Char('/')) if !*search_active => {
                        *search_active = true
                    }
                    (_, KeyCode::Enter) if !*search_active => apply_selected = true,
                    (_, KeyCode::Char('r')) if !*search_active => {
                        self.model.model_refresh_requested = true;
                    }
                    (_, KeyCode::Char('j')) if !*search_active => {
                        *cursor = cursor.saturating_add(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Char('k')) if !*search_active => {
                        *cursor = cursor.saturating_sub(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Down) if !*search_active => {
                        *cursor = cursor.saturating_add(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Up) if !*search_active => {
                        *cursor = cursor.saturating_sub(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::PageDown) if !*search_active => {
                        *cursor = cursor.saturating_add(10);
                        should_clamp = true;
                    }
                    (_, KeyCode::PageUp) if !*search_active => {
                        *cursor = cursor.saturating_sub(10);
                        should_clamp = true;
                    }
                    (_, KeyCode::Backspace) if *search_active => {
                        query.pop();
                        *cursor = 0;
                        *scroll = 0;
                        should_clamp = true;
                    }
                    (_, KeyCode::Char(c)) if *search_active => {
                        query.push(c);
                        *cursor = 0;
                        *scroll = 0;
                        should_clamp = true;
                    }
                    _ => {}
                }
            }

            if close_browser {
                self.state = AppState::Idle;
                return;
            }

            if apply_selected {
                if let Some((query, cursor)) = match &self.state {
                    AppState::ModelBrowsing { query, cursor, .. } => Some((query.clone(), *cursor)),
                    _ => None,
                } {
                    let visible = self.visible_model_entries(&query);
                    if let Some(selected) = visible.get(cursor) {
                        self.model.model_name = selected.id.clone();
                        self.model.pending_model_change =
                            Some((selected.id.clone(), selected.provider.clone()));
                        self.push_assistant(format!(
                            "Selected model '{}' (provider: {}) for this session.",
                            selected.id, selected.provider
                        ));
                    }
                }
                self.state = AppState::Idle;
                return;
            }

            if should_clamp {
                self.clamp_model_cursor();
            }
            return;
        }

        // ── SessionBrowsing state ─────────────────────────────
        let session_browsing = matches!(self.state, AppState::SessionBrowsing { .. });
        if session_browsing {
            let mut close_browser = false;
            let mut load_selected = false;
            let mut create_new = false;
            let mut delete_selected = false;
            let mut should_clamp = false;

            if let AppState::SessionBrowsing {
                query,
                cursor,
                scroll,
                search_active,
            } = &mut self.state
            {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => close_browser = true,
                    (_, KeyCode::Esc) => {
                        if *search_active {
                            *search_active = false;
                        } else {
                            close_browser = true;
                        }
                    }
                    (_, KeyCode::Char('s')) | (_, KeyCode::Char('/')) if !*search_active => {
                        *search_active = true
                    }
                    (_, KeyCode::Enter) if !*search_active => load_selected = true,
                    (_, KeyCode::Char('n')) if !*search_active => create_new = true,
                    (_, KeyCode::Char('d')) | (_, KeyCode::Delete) if !*search_active => {
                        delete_selected = true
                    }
                    (_, KeyCode::Char('j')) if !*search_active => {
                        *cursor = cursor.saturating_add(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Char('k')) if !*search_active => {
                        *cursor = cursor.saturating_sub(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Down) if !*search_active => {
                        *cursor = cursor.saturating_add(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::Up) if !*search_active => {
                        *cursor = cursor.saturating_sub(1);
                        should_clamp = true;
                    }
                    (_, KeyCode::PageDown) if !*search_active => {
                        *cursor = cursor.saturating_add(10);
                        should_clamp = true;
                    }
                    (_, KeyCode::PageUp) if !*search_active => {
                        *cursor = cursor.saturating_sub(10);
                        should_clamp = true;
                    }
                    (_, KeyCode::Backspace) if *search_active => {
                        query.pop();
                        *cursor = 0;
                        *scroll = 0;
                        should_clamp = true;
                    }
                    (_, KeyCode::Char(c)) if *search_active => {
                        query.push(c);
                        *cursor = 0;
                        *scroll = 0;
                        should_clamp = true;
                    }
                    _ => {}
                }
            }

            if close_browser {
                self.state = AppState::Idle;
                return;
            }

            if create_new {
                self.session.session_browser_new_requested = true;
                self.state = AppState::Idle;
                return;
            }

            if load_selected {
                if let Some((query, cursor)) = match &self.state {
                    AppState::SessionBrowsing { query, cursor, .. } => {
                        Some((query.clone(), *cursor))
                    }
                    _ => None,
                } {
                    let visible = self.visible_sessions(&query);
                    if let Some(selected) = visible.get(cursor) {
                        self.set_pending_sidebar_selection(selected.id.clone());
                    }
                }
                self.state = AppState::Idle;
                return;
            }

            if delete_selected
                && let Some((query, cursor)) = match &self.state {
                    AppState::SessionBrowsing { query, cursor, .. } => {
                        Some((query.clone(), *cursor))
                    }
                    _ => None,
                }
            {
                let visible = self.visible_sessions(&query);
                if let Some(selected) = visible.get(cursor) {
                    // Find the index in session_list for sidebar_hover
                    let target_id = selected.id.clone();
                    if let Some(idx) = self
                        .session
                        .session_list
                        .iter()
                        .position(|e| e.id.as_str() == target_id.as_str())
                    {
                        self.sidebar.hover = Some(idx);
                        self.state = AppState::Idle;
                        self.request_delete_session();
                        return;
                    }
                }
            }

            if should_clamp {
                self.clamp_session_cursor();
            }
            return;
        }

        let is_input_active = matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                if self.chat.text_selection.is_active() {
                    // Copy selected text then dismiss selection; do NOT quit.
                    if let Some((start, end)) = self.chat.text_selection.ordered_range() {
                        let grid = self.chat.chat_text_grid.clone();
                        let scroll = self.chat.chat_scroll_clamped;
                        if let Some(text) =
                            crate::tui::selection::extract_selected_text(&grid, start, end, scroll)
                        {
                            crate::tui::selection::copy_to_clipboard(&text);
                        }
                    }
                    self.chat.text_selection.clear();
                } else if self.is_palette_active() {
                    self.chat.input.clear();
                    self.chat.cursor_pos = 0;
                    self.command_palette_cursor = 0;
                } else if self.state == AppState::Idle {
                    self.should_quit = true;
                } else if matches!(self.state, AppState::AuthPrompting { .. }) {
                    self.cancel_auth_prompt();
                }
            }
            (_, KeyCode::Tab) if self.is_palette_active() => {
                let cmd_name = self
                    .palette_filtered_commands()
                    .get(self.command_palette_cursor)
                    .map(|c| c.name.to_string());
                if let Some(name) = cmd_name {
                    self.chat.input = name.clone();
                    self.chat.cursor_pos = name.len();
                    self.command_palette_cursor = 0;
                }
            }
            (_, KeyCode::Tab) if self.state == AppState::Idle && self.sidebar.visible => {
                self.toggle_sidebar_focus();
            }
            (_, KeyCode::Enter) if self.state == AppState::Idle => {
                // If Enter is pressed, make sure we are heavily into the Chat screen
                if self.screen == Screen::Home {
                    self.screen = Screen::Chat;
                }
                // (The event loop will then extract `self.take_input()` when handling this)
            }
            (_, KeyCode::Char(c)) if is_input_active => {
                self.chat.input.insert(self.chat.cursor_pos, c);
                self.chat.cursor_pos += c.len_utf8();
                self.command_palette_cursor = 0;
            }
            (_, KeyCode::Backspace) if is_input_active => {
                if self.chat.cursor_pos > 0 {
                    let prev = self.chat.input[..self.chat.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.chat.input.drain(prev..self.chat.cursor_pos);
                    self.chat.cursor_pos = prev;
                    self.command_palette_cursor = 0;
                }
            }
            (_, KeyCode::Left) if is_input_active => {
                if self.chat.cursor_pos > 0 {
                    self.chat.cursor_pos = self.chat.input[..self.chat.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            (_, KeyCode::Right) if is_input_active => {
                if self.chat.cursor_pos < self.chat.input.len() {
                    self.chat.cursor_pos = self.chat.input[self.chat.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.chat.cursor_pos + i)
                        .unwrap_or(self.chat.input.len());
                }
            }
            (_, KeyCode::Up) if self.is_palette_active() => {
                self.command_palette_cursor = self.command_palette_cursor.saturating_sub(1);
            }
            (_, KeyCode::Down) if self.is_palette_active() => {
                let max = self.palette_filtered_commands().len().saturating_sub(1);
                self.command_palette_cursor = (self.command_palette_cursor + 1).min(max);
            }
            (_, KeyCode::Up) => {
                self.chat.history_scroll = self.chat.history_scroll.saturating_sub(1);
            }
            (_, KeyCode::Down) => {
                self.chat.history_scroll = self.chat.history_scroll.saturating_add(1);
            }
            (_, KeyCode::PageUp) => {
                self.chat.history_scroll = self.chat.history_scroll.saturating_sub(10);
            }
            (_, KeyCode::PageDown) => {
                self.chat.history_scroll = self.chat.history_scroll.saturating_add(10);
            }
            _ => {}
        }
    }

    pub fn map_key_event(&self, key: crossterm::event::KeyEvent) -> Vec<Action> {
        use crate::tui::action::Action;
        use crossterm::event::{KeyCode, KeyModifiers};

        if matches!(self.state, AppState::QrCodeDisplay { .. }) {
            return match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => vec![Action::CloseQrCode],
                _ => vec![],
            };
        }

        if let AppState::ConfirmDelete { .. } = &self.state {
            return match key.code {
                KeyCode::Char('y') | KeyCode::Enter => vec![Action::ConfirmDelete],
                KeyCode::Char('n') | KeyCode::Esc => vec![Action::CancelDelete],
                _ => vec![],
            };
        }

        if self.sidebar.focused && self.state == AppState::Idle {
            return match (key.modifiers, key.code) {
                (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                    let max = self.session.session_list.len().saturating_sub(1);
                    let current = self.sidebar.hover.unwrap_or(0);
                    vec![Action::SidebarHover(Some((current + 1).min(max)))]
                }
                (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                    let current = self.sidebar.hover.unwrap_or(0);
                    vec![Action::SidebarHover(Some(current.saturating_sub(1)))]
                }
                (_, KeyCode::Enter) => vec![Action::SelectSidebarSession],
                (_, KeyCode::Char('d')) | (_, KeyCode::Delete) => {
                    vec![Action::RequestDeleteSession]
                }
                (_, KeyCode::Esc) | (_, KeyCode::Tab) => {
                    vec![
                        Action::SidebarHover(self.sidebar.hover),
                        Action::ToggleSidebarFocus,
                    ]
                }
                _ => vec![],
            };
        }

        if matches!(self.state, AppState::LoginBrowsing(_)) {
            return vec![Action::LoginBrowserKey(key)];
        }

        if matches!(self.state, AppState::ModelBrowsing { .. }) {
            return vec![Action::ModelBrowserKey(key)];
        }

        if matches!(self.state, AppState::SessionBrowsing { .. }) {
            return vec![Action::SessionBrowserKey(key)];
        }

        if matches!(self.state, AppState::WebConfiguring(_)) {
            return vec![Action::WebConfigKey(key)];
        }

        if matches!(self.state, AppState::WhatsAppSetup { .. }) {
            return match (key.modifiers, key.code) {
                (_, KeyCode::Esc) => vec![Action::WhatsAppSetupCancel],
                _ => vec![], // No text input in QR pairing flow
            };
        }

        let is_input_active = matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                if self.chat.text_selection.is_active() {
                    vec![Action::TextSelectionCopy]
                } else if self.is_palette_active() {
                    vec![Action::PaletteClose]
                } else if self.state == AppState::Idle {
                    vec![Action::Quit]
                } else if matches!(self.state, AppState::AuthPrompting { .. }) {
                    vec![Action::CancelAuth]
                } else {
                    vec![]
                }
            }
            (_, KeyCode::Tab) if self.is_palette_active() => {
                vec![Action::PaletteTabComplete]
            }
            (_, KeyCode::Tab) if self.state == AppState::Idle && self.sidebar.visible => {
                vec![Action::ToggleSidebarFocus]
            }
            (_, KeyCode::Enter) if self.state == AppState::Idle => {
                vec![Action::SubmitInput]
            }
            (_, KeyCode::Char(c)) if is_input_active => {
                vec![Action::InsertChar(c)]
            }
            (_, KeyCode::Backspace) if is_input_active => {
                vec![Action::DeleteChar]
            }
            (_, KeyCode::Left) if is_input_active => {
                vec![Action::MoveCursorLeft]
            }
            (_, KeyCode::Right) if is_input_active => {
                vec![Action::MoveCursorRight]
            }
            (_, KeyCode::Up) if self.is_palette_active() => {
                vec![Action::PaletteMoveUp]
            }
            (_, KeyCode::Down) if self.is_palette_active() => {
                vec![Action::PaletteMoveDown]
            }
            (_, KeyCode::Up) => vec![Action::ScrollUp(1)],
            (_, KeyCode::Down) => vec![Action::ScrollDown(1)],
            (_, KeyCode::PageUp) => vec![Action::ScrollUp(10)],
            (_, KeyCode::PageDown) => vec![Action::ScrollDown(10)],
            _ => vec![],
        }
    }

    pub fn map_mouse_event(
        &self,
        mouse: crossterm::event::MouseEvent,
        frame_area: ratatui::layout::Rect,
    ) -> Vec<Action> {
        use crate::tui::action::Action;
        use crossterm::event::{MouseButton, MouseEventKind};
        use ratatui::layout::Position;

        let mut actions = Vec::new();
        let pos = Position::new(mouse.column, mouse.row);

        if let Some(sb_area) = self.compute_sidebar_area(frame_area) {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if sb_area.contains(pos) {
                        let inner_y = mouse.row.saturating_sub(sb_area.y + 1);
                        let entry_height = 3u16;
                        let scrolled_y = inner_y + self.sidebar.scroll * entry_height;
                        let idx = (scrolled_y / entry_height) as usize;
                        if idx < self.session.session_list.len() {
                            actions.push(Action::SidebarHover(Some(idx)));
                            actions.push(Action::SelectSidebarSession);
                        }
                        return actions;
                    }
                }
                MouseEventKind::Moved => {
                    if sb_area.contains(pos) {
                        let inner_y = mouse.row.saturating_sub(sb_area.y + 1);
                        let entry_height = 3u16;
                        let scrolled_y = inner_y + self.sidebar.scroll * entry_height;
                        let idx = (scrolled_y / entry_height) as usize;
                        if idx < self.session.session_list.len() {
                            actions.push(Action::SidebarHover(Some(idx)));
                        } else {
                            actions.push(Action::SidebarHover(None));
                        }
                        return actions;
                    } else {
                        actions.push(Action::SidebarHover(None));
                        return actions;
                    }
                }
                MouseEventKind::ScrollDown => {
                    if sb_area.contains(pos) {
                        actions.push(Action::SidebarScroll(1));
                        return actions;
                    }
                }
                MouseEventKind::ScrollUp => {
                    if sb_area.contains(pos) {
                        actions.push(Action::SidebarScroll(-1));
                        return actions;
                    }
                }
                _ => {}
            }
        }

        if let Some(chat_area) = self.chat.chat_area {
            let inner = ratatui::layout::Rect {
                x: chat_area.x + 1,
                y: chat_area.y + 1,
                width: chat_area.width.saturating_sub(2),
                height: chat_area.height.saturating_sub(2),
            };

            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if inner.contains(pos) {
                        let rel_col = mouse.column - inner.x;
                        let rel_row = mouse.row - inner.y;
                        actions.push(Action::TextSelectionStart {
                            row: rel_row,
                            col: rel_col,
                        });
                    } else {
                        actions.push(Action::TextSelectionClear);
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if self.chat.text_selection.dragging {
                        let rel_col = mouse
                            .column
                            .saturating_sub(inner.x)
                            .min(inner.width.saturating_sub(1));
                        let rel_row = mouse
                            .row
                            .saturating_sub(inner.y)
                            .min(inner.height.saturating_sub(1));
                        actions.push(Action::TextSelectionDrag {
                            row: rel_row,
                            col: rel_col,
                        });
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if self.chat.text_selection.dragging {
                        let rel_col = mouse
                            .column
                            .saturating_sub(inner.x)
                            .min(inner.width.saturating_sub(1));
                        let rel_row = mouse
                            .row
                            .saturating_sub(inner.y)
                            .min(inner.height.saturating_sub(1));
                        actions.push(Action::TextSelectionEnd {
                            row: rel_row,
                            col: rel_col,
                        });
                    }
                }
                MouseEventKind::ScrollDown => {
                    if chat_area.contains(pos) {
                        actions.push(Action::ScrollDown(3));
                        actions.push(Action::TextSelectionClear);
                    }
                }
                MouseEventKind::ScrollUp => {
                    if chat_area.contains(pos) {
                        actions.push(Action::ScrollUp(3));
                        actions.push(Action::TextSelectionClear);
                    }
                }
                _ => {}
            }
        }

        actions
    }
}
