//! TUI rendering methods for TuiApp.

use unicode_width::UnicodeWidthStr;

use crate::auth_picker::{AuthMethodChoice, LoginBrowseStep};
use crate::tui::theme::THEME;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::state::*;

impl TuiApp {
    /// Render the entire TUI into the given frame.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();

        if matches!(self.state, AppState::LoginBrowsing(_)) {
            self.render_login_browser(frame, area);
            return;
        }

        if matches!(self.state, AppState::ModelBrowsing { .. }) {
            self.render_model_browser(frame, area);
            return;
        }

        if matches!(self.state, AppState::SessionBrowsing { .. }) {
            self.render_session_browser(frame, area);
            return;
        }

        if let AppState::WhatsAppSetup { .. } = &self.state {
            self.render_whatsapp_setup(frame, area);
            return;
        }

        match self.screen {
            Screen::Home => {
                // Layout for home: content(fill) | status(1)
                let chunks =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

                crate::tui::home::render(self, frame, chunks[0]);
                crate::tui::status::render(self, frame, chunks[1]);
            }
            Screen::Chat => {
                let sidebar_w = if self.sidebar.visible {
                    crate::tui::sidebar::sidebar_width()
                } else {
                    0
                };
                let h_chunks =
                    Layout::horizontal([Constraint::Min(0), Constraint::Length(sidebar_w)])
                        .split(area);

                let main_area = h_chunks[0];
                let sidebar_area = h_chunks[1];

                let chunks = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(3),
                ])
                .split(main_area);

                self.render_title(frame, chunks[0]);
                crate::tui::chat::render(self, frame, chunks[1]);
                crate::tui::status::render(self, frame, chunks[2]);
                self.render_input(frame, chunks[3]);

                if self.sidebar.visible {
                    crate::tui::sidebar::render(self, frame, sidebar_area);
                }

                // ── ConfirmDelete overlay ──────────────────
                if let AppState::ConfirmDelete {
                    session_preview, ..
                } = &self.state
                {
                    let popup_width = 50u16.min(area.width.saturating_sub(4));
                    let popup_height = 5u16;
                    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
                    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
                    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

                    frame.render_widget(Clear, popup_area);
                    let dialog = Paragraph::new(vec![
                        Line::from(Span::styled(
                            " Delete session? ",
                            Style::default()
                                .fg(THEME.error)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            format!(" {}", session_preview),
                            Style::default().fg(THEME.fg_dim),
                        )),
                        Line::from(""),
                        Line::from(vec![
                            Span::styled(
                                " y",
                                Style::default()
                                    .fg(THEME.error)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("/Enter: delete  ", Style::default().fg(THEME.fg_muted)),
                            Span::styled(
                                "n",
                                Style::default()
                                    .fg(THEME.success)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("/Esc: cancel", Style::default().fg(THEME.fg_muted)),
                        ]),
                    ])
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(THEME.error)),
                    )
                    .wrap(Wrap { trim: false });
                    frame.render_widget(dialog, popup_area);
                }

                if let AppState::QrCodeDisplay { url, qr_lines } = &self.state {
                    let qr_height = qr_lines.len() as u16 + 4;
                    let qr_width = qr_lines.first().map_or(20, |l| l.len() as u16) + 4;
                    let popup_width = qr_width.min(area.width.saturating_sub(4));
                    let popup_height = qr_height.min(area.height.saturating_sub(2));
                    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
                    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
                    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

                    frame.render_widget(Clear, popup_area);
                    let mut lines_vec: Vec<Line<'_>> = qr_lines
                        .iter()
                        .map(|l| Line::from(Span::raw(format!(" {l} "))))
                        .collect();
                    lines_vec.push(Line::from(""));
                    lines_vec.push(Line::from(Span::styled(
                        format!(" {url} "),
                        Style::default().fg(THEME.info),
                    )));
                    lines_vec.push(Line::from(Span::styled(
                        " Esc/Enter: close ",
                        Style::default().fg(THEME.fg_muted),
                    )));
                    let qr_widget = Paragraph::new(lines_vec)
                        .block(
                            Block::default()
                                .title(" QR Code — Web UI ")
                                .borders(Borders::ALL)
                                .border_style(Style::default().fg(THEME.info)),
                        )
                        .wrap(Wrap { trim: false });
                    frame.render_widget(qr_widget, popup_area);
                }

                // ── Tool Approval overlay ────────────────────
                if let Some(pending) = &self.chat.pending_approval {
                    let tool_name = &pending.request.tool_name;
                    let args_str = serde_json::to_string_pretty(&pending.request.arguments)
                        .unwrap_or_else(|_| pending.request.arguments.to_string());
                    let args_preview: String = args_str.chars().take(200).collect();

                    let popup_width = 60u16.min(area.width.saturating_sub(4));
                    let popup_height = 9u16.min(area.height.saturating_sub(2));
                    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
                    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
                    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

                    frame.render_widget(Clear, popup_area);
                    let dialog = Paragraph::new(vec![
                        Line::from(Span::styled(
                            " Tool Approval Required ",
                            Style::default()
                                .fg(THEME.warning)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            format!(" Tool: {tool_name}"),
                            Style::default().fg(THEME.fg),
                        )),
                        Line::from(Span::styled(
                            format!(" Args: {args_preview}"),
                            Style::default().fg(THEME.fg_dim),
                        )),
                        Line::from(""),
                        Line::from(vec![
                            Span::styled(
                                " y",
                                Style::default()
                                    .fg(THEME.success)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(": approve  ", Style::default().fg(THEME.fg_muted)),
                            Span::styled(
                                "n",
                                Style::default()
                                    .fg(THEME.error)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(": reject  ", Style::default().fg(THEME.fg_muted)),
                            Span::styled(
                                "a",
                                Style::default()
                                    .fg(THEME.accent)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                ": allow for session",
                                Style::default().fg(THEME.fg_muted),
                            ),
                        ]),
                    ])
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(THEME.warning)),
                    )
                    .wrap(Wrap { trim: false });
                    frame.render_widget(dialog, popup_area);
                }
            }
        }
    }

    fn render_login_browser(&self, frame: &mut Frame<'_>, area: Rect) {
        let AppState::LoginBrowsing(LoginBrowsingState {
            query,
            cursor,
            scroll,
            step,
            selected_provider,
            selected_method,
            input_buffer,
            masked_buffer,
            last_error,
            endpoint,
        }) = &self.state
        else {
            return;
        };

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Add credential ",
                    Style::default()
                        .fg(THEME.browser_title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " /login or /connection ",
                    Style::default().fg(THEME.fg_muted),
                ),
            ])),
            chunks[0],
        );

        let mut lines: Vec<Line<'_>> = Vec::new();
        match step {
            LoginBrowseStep::SelectProvider => {
                lines.push(Line::from(Span::styled(
                    " Select provider ",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" Search: {}", query),
                    Style::default().fg(THEME.browser_search),
                )));

                let providers = self.visible_login_provider_entries(query);
                let creds = crate::auth::Credentials::load();
                if providers.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(" No matches for '{}'.", query),
                        Style::default().fg(THEME.warning),
                    )));
                } else {
                    for (idx, entry) in providers.iter().enumerate() {
                        let selected = idx == *cursor;
                        let marker = if selected { "●" } else { "○" };
                        let is_authed = creds.get(entry.name).is_some_and(|c| !c.is_expired());
                        let mut spans = vec![
                            Span::styled(
                                format!(" {} ", marker),
                                if selected {
                                    Style::default()
                                        .fg(THEME.browser_selected_marker)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(THEME.fg_muted)
                                },
                            ),
                            Span::styled(
                                entry.display_name,
                                if selected {
                                    Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(THEME.fg)
                                },
                            ),
                        ];
                        if is_authed {
                            spans.push(Span::styled(
                                " ●",
                                Style::default().fg(THEME.palette_auth_dot),
                            ));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
            LoginBrowseStep::SelectMethod => {
                lines.push(Line::from(Span::styled(
                    " Select auth method ",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " Provider: {}",
                        selected_provider.as_deref().unwrap_or("openai")
                    ),
                    Style::default().fg(THEME.fg_muted),
                )));
                let methods = [AuthMethodChoice::OAuth, AuthMethodChoice::ApiKey];
                for (idx, method) in methods.iter().enumerate() {
                    let selected = idx == *cursor;
                    lines.push(Line::from(vec![
                        Span::styled(
                            if selected { " ● " } else { " ○ " },
                            if selected {
                                Style::default()
                                    .fg(THEME.browser_selected_marker)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(THEME.fg_muted)
                            },
                        ),
                        Span::styled(
                            method.label(),
                            if selected {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                    ]));
                }
            }
            LoginBrowseStep::InputEndpoint => {
                lines.push(Line::from(Span::styled(
                    " Enter endpoint ",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " Provider: {}",
                        selected_provider.as_deref().unwrap_or("provider")
                    ),
                    Style::default().fg(THEME.fg_muted),
                )));
                lines.push(Line::from(Span::raw(format!(
                    " Endpoint: {}",
                    input_buffer
                ))));
            }
            LoginBrowseStep::InputApiKey => {
                let is_code_display = matches!(selected_method, Some(AuthMethodChoice::OAuth));
                let title = if is_code_display {
                    " Enter authorization code "
                } else {
                    " Enter API key "
                };
                let label = if is_code_display { "Code" } else { "API key" };
                let display = if is_code_display {
                    input_buffer.as_str()
                } else {
                    masked_buffer.as_str()
                };
                lines.push(Line::from(Span::styled(
                    title,
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " Provider: {}",
                        selected_provider.as_deref().unwrap_or("provider")
                    ),
                    Style::default().fg(THEME.fg_muted),
                )));
                if is_code_display {
                    lines.push(Line::from(Span::styled(
                        " Paste the code shown in your browser after authorizing.",
                        Style::default().fg(THEME.warning),
                    )));
                }
                if let Some(endpoint) = endpoint {
                    lines.push(Line::from(Span::styled(
                        format!(" Endpoint: {}", endpoint),
                        Style::default().fg(THEME.fg_muted),
                    )));
                }
                lines.push(Line::from(Span::raw(format!(" {}: {}", label, display))));
            }
        }

        if let Some(error) = last_error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Error: {}", error),
                Style::default()
                    .fg(THEME.error)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        let content_height = lines.len() as u16;
        let visible_height = chunks[1].height.saturating_sub(2);
        let max_scroll = content_height.saturating_sub(visible_height);
        let effective_scroll = (*scroll).min(max_scroll);
        let body = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.fg_muted)),
            )
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0));
        frame.render_widget(body, chunks[1]);

        let footer =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[2]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                " ↑/↓ to select • Enter: confirm • Type: to search/input ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Esc: back/close • j/k: move • PgUp/PgDn: page ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[1],
        );

        match step {
            LoginBrowseStep::SelectProvider => {
                let cursor_col = UnicodeWidthStr::width(query.as_str()) as u16;
                frame.set_cursor_position((chunks[1].x + 10 + cursor_col, chunks[1].y + 3));
            }
            LoginBrowseStep::InputEndpoint => {
                let cursor_col = UnicodeWidthStr::width(input_buffer.as_str()) as u16;
                frame.set_cursor_position((chunks[1].x + 12 + cursor_col, chunks[1].y + 4));
            }
            LoginBrowseStep::InputApiKey => {
                let is_code_display = matches!(selected_method, Some(AuthMethodChoice::OAuth));
                let display_len = if is_code_display {
                    UnicodeWidthStr::width(input_buffer.as_str())
                } else {
                    UnicodeWidthStr::width(masked_buffer.as_str())
                };
                // " Code: " = 7, " API key: " = 10
                let label_offset: u16 = if is_code_display { 8 } else { 11 };
                let hint_offset: u16 = if is_code_display { 1 } else { 0 };
                let endpoint_offset: u16 = if endpoint.is_some() { 1 } else { 0 };
                frame.set_cursor_position((
                    chunks[1].x + label_offset + display_len as u16,
                    chunks[1].y + 4 + hint_offset + endpoint_offset,
                ));
            }
            LoginBrowseStep::SelectMethod => {}
        }
    }

    fn render_model_browser(&self, frame: &mut Frame<'_>, area: Rect) {
        let AppState::ModelBrowsing {
            query,
            cursor,
            scroll,
            last_sync_status,
            search_active,
        } = &self.state
        else {
            return;
        };

        let entries = self.visible_model_entries(query);

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

        let header = Line::from(vec![
            Span::styled(
                " Models ",
                Style::default()
                    .fg(THEME.browser_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({}) ", self.model.model_provider),
                Style::default().fg(THEME.fg_muted),
            ),
            Span::styled(
                format!(" {} ", last_sync_status),
                Style::default().fg(if last_sync_status.starts_with("Offline") {
                    THEME.warning
                } else {
                    THEME.fg_muted
                }),
            ),
        ]);
        frame.render_widget(Paragraph::new(header), chunks[0]);

        let mut lines: Vec<Line<'_>> = Vec::new();
        let mut visible_index = 0usize;
        if entries.is_empty() {
            lines.push(Line::from(Span::styled(
                if query.trim().is_empty() {
                    "  No models available.".to_string()
                } else {
                    format!("  No matches for '{}'.", query)
                },
                Style::default().fg(THEME.warning),
            )));
        } else {
            for entry in entries {
                let selected = visible_index == *cursor;
                lines.push(Line::from(vec![
                    Span::styled(
                        if selected { "› " } else { "  " },
                        if selected {
                            Style::default()
                                .fg(THEME.browser_selected_marker)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(THEME.fg_muted)
                        },
                    ),
                    Span::styled(
                        entry.id,
                        Style::default()
                            .fg(if query.trim().is_empty() {
                                THEME.fg
                            } else {
                                THEME.warning
                            })
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        format!("  [{}]", entry.provider),
                        Style::default().fg(THEME.fg_muted),
                    ),
                ]));
                visible_index += 1;
            }
        }

        let content_height = lines.len() as u16;
        let visible_height = chunks[1].height.saturating_sub(2);
        let max_scroll = content_height.saturating_sub(visible_height);
        let effective_scroll = (*scroll).min(max_scroll);

        let list = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.fg_muted)),
            )
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0));
        frame.render_widget(list, chunks[1]);

        let footer =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[2]);
        let query_label = if *search_active {
            format!(" Search (typing): {}", query)
        } else {
            format!(" Search: {}", query)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                query_label,
                Style::default().fg(THEME.browser_footer),
            )),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                " s or /:search  j/k,↑/↓:move  PgUp/PgDn:page  Enter:use model  r:refresh  Esc:back/close ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[1],
        );

        if *search_active {
            let cursor_col = UnicodeWidthStr::width(query.as_str()) as u16;
            frame.set_cursor_position((footer[0].x + 18 + cursor_col, footer[0].y));
        }
    }

    fn render_session_browser(&self, frame: &mut Frame<'_>, area: Rect) {
        let AppState::SessionBrowsing {
            query,
            cursor,
            scroll,
            search_active,
        } = &self.state
        else {
            return;
        };

        let entries = self.visible_sessions(query);

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

        let header = Line::from(vec![
            Span::styled(
                " Sessions ",
                Style::default()
                    .fg(THEME.browser_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({}) ", entries.len()),
                Style::default().fg(THEME.fg_muted),
            ),
        ]);
        frame.render_widget(Paragraph::new(header), chunks[0]);

        let mut lines: Vec<Line<'_>> = Vec::new();
        let mut visible_index = 0usize;
        if entries.is_empty() {
            lines.push(Line::from(Span::styled(
                if query.trim().is_empty() {
                    "  No sessions available.".to_string()
                } else {
                    format!("  No matches for '{}'.", query)
                },
                Style::default().fg(THEME.warning),
            )));
        } else {
            for entry in &entries {
                let selected = visible_index == *cursor;
                let is_active = entry.id.as_str() == self.session.session_id.as_str();
                let id_short = if entry.id.as_str().len() > 8 {
                    &entry.id.as_str()[..8]
                } else {
                    entry.id.as_str()
                };
                let preview = crate::tui::sidebar::truncate_str(&entry.preview, 40);
                let time_str = crate::tui::sidebar::format_relative_time(&entry.updated_at);
                let active_marker = if is_active { " ← active" } else { "" };
                lines.push(Line::from(vec![
                    Span::styled(
                        if selected { "› " } else { "  " },
                        if selected {
                            Style::default()
                                .fg(THEME.browser_selected_marker)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(THEME.fg_muted)
                        },
                    ),
                    Span::styled(
                        id_short.to_string(),
                        Style::default()
                            .fg(if is_active { THEME.accent } else { THEME.fg })
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        format!("  {}", preview),
                        Style::default().fg(THEME.fg_muted),
                    ),
                    Span::styled(format!("  {}", time_str), Style::default().fg(THEME.fg_dim)),
                    Span::styled(active_marker.to_string(), Style::default().fg(THEME.accent)),
                ]));
                visible_index += 1;
            }
        }

        let content_height = lines.len() as u16;
        let visible_height = chunks[1].height.saturating_sub(2);
        let max_scroll = content_height.saturating_sub(visible_height);
        let effective_scroll = (*scroll).min(max_scroll);

        let list = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.fg_muted)),
            )
            .wrap(Wrap { trim: false })
            .scroll((effective_scroll, 0));
        frame.render_widget(list, chunks[1]);

        let footer =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[2]);
        let query_label = if *search_active {
            format!(" Search (typing): {}", query)
        } else {
            format!(" Search: {}", query)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                query_label,
                Style::default().fg(THEME.browser_footer),
            )),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                " s or /:search  j/k,↑/↓:move  PgUp/PgDn:page  Enter:load  n:new  d:delete  Esc:back/close ",
                Style::default().fg(THEME.browser_footer),
            )),
            footer[1],
        );

        if *search_active {
            let cursor_col = query.chars().count() as u16;
            frame.set_cursor_position((footer[0].x + 18 + cursor_col, footer[0].y));
        }
    }

    fn render_title(&self, frame: &mut Frame<'_>, area: Rect) {
        let title = Line::from(vec![
            Span::styled(
                " Chat ",
                Style::default()
                    .fg(THEME.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" model:{} ", self.model.model_name),
                Style::default().fg(THEME.fg),
            ),
            Span::styled(
                format!(" session:{} ", self.session.session_id.as_str()),
                Style::default().fg(THEME.fg_muted),
            ),
        ]);
        frame.render_widget(Paragraph::new(title), area);
    }

    fn render_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let is_idle_or_prompting =
            matches!(self.state, AppState::Idle | AppState::AuthPrompting { .. });
        let border_color = if is_idle_or_prompting {
            THEME.accent
        } else {
            THEME.fg_muted
        };

        let mut display_text = if self.chat.input.is_empty() && is_idle_or_prompting {
            if matches!(self.state, AppState::AuthPrompting { .. }) {
                "Paste your API key here..."
            } else {
                "Type a message..."
            }
            .to_string()
        } else {
            self.chat.input.clone()
        };

        if matches!(self.state, AppState::AuthPrompting { .. }) && !self.chat.input.is_empty() {
            display_text = "*".repeat(self.chat.input.chars().count());
        }

        let input_style = if self.chat.input.is_empty() && is_idle_or_prompting {
            Style::default().fg(THEME.fg_muted)
        } else {
            Style::default()
        };

        let input = Paragraph::new(Span::styled(display_text, input_style)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(if matches!(self.state, AppState::AuthPrompting { .. }) {
                    " Auth "
                } else {
                    " Input "
                }),
        );

        frame.render_widget(input, area);

        if is_idle_or_prompting {
            let cursor_col =
                UnicodeWidthStr::width(&self.chat.input[..self.chat.cursor_pos]) as u16;
            frame.set_cursor_position((area.x + 1 + cursor_col, area.y + 1));
        }

        if self.is_palette_active() {
            self.render_command_palette(frame, area);
        }
    }

    pub(crate) fn render_command_palette(&self, frame: &mut Frame<'_>, input_area: Rect) {
        let cmds = self.palette_filtered_commands();
        if cmds.is_empty() {
            return;
        }

        let popup_h = cmds.len() as u16 + 2; // content + top/bottom border
        let popup_y = input_area.y.saturating_sub(popup_h);
        let popup_rect = Rect {
            x: input_area.x,
            y: popup_y,
            width: input_area.width,
            height: popup_h,
        };

        // Name column width = longest command name.
        let name_w = cmds.iter().map(|c| c.name.len()).max().unwrap_or(0);

        let authenticated = self.is_authenticated();
        let lines: Vec<Line<'_>> = cmds
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let sel = i == self.command_palette_cursor;
                let arrow = if sel { "› " } else { "  " };
                let pad = " ".repeat(name_w.saturating_sub(cmd.name.len()) + 2);
                let is_login = cmd.name == "/login" || cmd.name == "/connection";
                let auth_dot = if is_login && authenticated {
                    Some(Span::styled(
                        "● ",
                        Style::default().fg(THEME.palette_auth_dot),
                    ))
                } else {
                    None
                };
                let mut spans = vec![
                    Span::styled(
                        arrow,
                        Style::default().fg(if sel {
                            THEME.palette_cmd
                        } else {
                            THEME.fg_muted
                        }),
                    ),
                    Span::styled(
                        cmd.name,
                        Style::default().fg(THEME.palette_cmd).add_modifier(if sel {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                    Span::raw(pad),
                ];
                if let Some(dot) = auth_dot {
                    spans.push(dot);
                }
                spans.push(Span::styled(
                    cmd.description,
                    Style::default()
                        .fg(if sel {
                            THEME.palette_selected_fg
                        } else {
                            THEME.palette_desc
                        })
                        .add_modifier(if sel {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ));
                Line::from(spans)
            })
            .collect();

        frame.render_widget(Clear, popup_rect);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.palette_border))
                    .title(Span::styled(
                        " Commands ",
                        Style::default()
                            .fg(THEME.palette_border)
                            .add_modifier(Modifier::BOLD),
                    )),
            ),
            popup_rect,
        );
    }

    fn render_whatsapp_setup(&self, frame: &mut Frame<'_>, area: Rect) {
        let AppState::WhatsAppSetup { step } = &self.state else {
            return;
        };
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
        let (phase, phase_style) = match step {
            WhatsAppSetupStep::CheckingPrereqs => ("Checking prerequisites\u{2026}", THEME.fg_dim),
            WhatsAppSetupStep::InstallingBridge => ("Installing bridge\u{2026}", THEME.fg_dim),
            WhatsAppSetupStep::WaitingForQr => ("Waiting for QR\u{2026}", THEME.fg_dim),
            WhatsAppSetupStep::DisplayQr { .. } => ("Scan QR code", THEME.accent),
            WhatsAppSetupStep::Connected { .. } => ("Connected \u{2713}", THEME.success),
        };
        let title = Line::from(vec![
            Span::styled(
                " WhatsApp Pairing ",
                Style::default()
                    .fg(THEME.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("({phase})"), Style::default().fg(phase_style)),
        ]);
        frame.render_widget(Paragraph::new(title), chunks[0]);
        let content_lines: Vec<Line<'_>> = match step {
            WhatsAppSetupStep::CheckingPrereqs => vec![
                Line::from(""),
                Line::from(Span::styled(
                    " Checking prerequisites\u{2026}",
                    Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    " Verifying Node.js and bridge dependencies.",
                    Style::default().fg(THEME.fg_dim),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " Esc: cancel",
                    Style::default().fg(THEME.error),
                )),
            ],
            WhatsAppSetupStep::InstallingBridge => vec![
                Line::from(""),
                Line::from(Span::styled(
                    " Installing bridge dependencies\u{2026}",
                    Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    " Running npm install in whatsapp-bridge/",
                    Style::default().fg(THEME.fg_dim),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " Esc: cancel",
                    Style::default().fg(THEME.error),
                )),
            ],
            WhatsAppSetupStep::WaitingForQr => vec![
                Line::from(""),
                Line::from(Span::styled(
                    " Starting WhatsApp bridge…",
                    Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    " A QR code will appear here shortly.",
                    Style::default().fg(THEME.fg_dim),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " Esc: cancel",
                    Style::default().fg(THEME.error),
                )),
            ],
            WhatsAppSetupStep::DisplayQr { qr_lines, .. } => {
                let mut lines: Vec<Line<'_>> = vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        " Open WhatsApp on your phone → Linked Devices → Link a Device",
                        Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                ];
                for ql in qr_lines {
                    lines.push(Line::from(Span::raw(format!("  {ql}"))));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " Esc: cancel",
                    Style::default().fg(THEME.error),
                )));
                lines
            }
            WhatsAppSetupStep::Connected { phone, name } => vec![
                Line::from(""),
                Line::from(Span::styled(
                    " ✓ WhatsApp paired successfully!",
                    Style::default()
                        .fg(THEME.success)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("   Phone:  ", Style::default().fg(THEME.fg_dim)),
                    Span::styled(phone.as_str(), Style::default().fg(THEME.fg)),
                ]),
                Line::from(vec![
                    Span::styled("   Name:   ", Style::default().fg(THEME.fg_dim)),
                    Span::styled(name.as_str(), Style::default().fg(THEME.fg)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    " Esc: close",
                    Style::default().fg(THEME.fg_muted),
                )),
            ],
        };
        let content = Paragraph::new(content_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.border))
                    .title(Span::styled(
                        " WhatsApp Pairing ",
                        Style::default().fg(THEME.accent),
                    )),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(content, chunks[1]);
    }
}
