//! Async event loop for the TUI — interleaves crossterm, agent progress, and timer events.
#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use crossterm::{
    event::{Event, EventStream, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use proto::{ChannelId, ProgressEvent, SessionId};
use ratatui::{Terminal, backend::CrosstermBackend};
use skills::SkillLoader;
use tokio::sync::mpsc;

use super::app::{AppState, TuiApp};

/// RAII guard that restores the terminal on drop (even on panic).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the full-screen TUI until the user quits.
pub async fn run_tui(
    runtime: Arc<agent::AgentRuntime>,
    skill_loader: Arc<SkillLoader>,
    channel_id: ChannelId,
    session_id: SessionId,
    model_name: String,
) -> anyhow::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _guard = TerminalGuard; // Drop restores terminal

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // App state
    let mut app = TuiApp::new(&model_name, session_id.clone(), channel_id.clone());

    // Crossterm event stream (async)
    let mut crossterm_stream = EventStream::new();

    // Agent task state
    let mut agent_task: Option<tokio::task::JoinHandle<Result<String, proto::Error>>> = None;
    let mut progress_rx: Option<mpsc::Receiver<ProgressEvent>> = None;

    // Spinner tick interval (100ms)
    let mut spinner_interval = tokio::time::interval(std::time::Duration::from_millis(100));
    spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Render
        terminal.draw(|frame| app.render(frame))?;

        // Event select
        tokio::select! {
            // Branch 1: crossterm terminal events
            maybe_event = crossterm_stream.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        use crossterm::event::KeyCode;
                        // Enter key: submit input (only when Idle)
                        if key.code == KeyCode::Enter && app.state == AppState::Idle && !app.input.is_empty() {
                            let message = app.take_input();
                            app.push_user(message.clone());
                            app.state = AppState::Thinking { round: 0 };
                            app.scroll_to_bottom();

                            // Spawn agent task
                            let (prog_tx, prog_rx_new) = mpsc::channel::<ProgressEvent>(64);
                            let rt = Arc::clone(&runtime);
                            let sl = Arc::clone(&skill_loader);
                            let ch = channel_id.clone();
                            let sess = session_id.clone();

                            let handle = tokio::spawn(async move {
                                let skills_ctx = sl.load_context().await;
                                rt.process_with_progress(
                                    &ch,
                                    &sess,
                                    &message,
                                    Some(&skills_ctx),
                                    prog_tx,
                                )
                                .await
                            });

                            agent_task = Some(handle);
                            progress_rx = Some(prog_rx_new);
                        } else {
                            app.handle_key(key);
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {
                        // Terminal will redraw on next loop iteration
                    }
                    Some(Err(_)) | None => {
                        break; // stream ended or error
                    }
                    _ => {}
                }
            }

            // Branch 2: progress events from agent task
            Some(evt) = async {
                match progress_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                app.apply_progress(evt);
                app.scroll_to_bottom();
            }

            // Branch 3: agent task completed
            result = async {
                match agent_task.as_mut() {
                    Some(handle) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                match result {
                    Ok(inner) => app.apply_completion(inner),
                    Err(join_err) => app.apply_completion(Err(proto::Error::Llm(
                        proto::LlmError::InvalidResponse(format!("Task panicked: {join_err}"))
                    ))),
                }
                app.scroll_to_bottom();
                agent_task = None;
                progress_rx = None;
            }

            // Branch 4: spinner tick
            _ = spinner_interval.tick(), if app.state != AppState::Idle => {
                app.spinner_tick = app.spinner_tick.wrapping_add(1);
            }
        }

        if app.should_quit {
            break;
        }
    }

    // TerminalGuard::drop handles cleanup
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_guard_drop_path_is_safe() {
        let guard = TerminalGuard;
        drop(guard);
    }
}
