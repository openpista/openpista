//! WebSocket connection handling: upgrade, message relay, broadcast.

use super::WebAdapter;
use super::handlers::validate_token;
use super::types::{
    AuthSession, ProviderAuthIntent, ProviderAuthResult, SESSION_TOKEN_TTL_HOURS, WebSessionEntry,
    WebState, WsConnectParams, WsMessage, generate_session_token, to_web_history,
};
use axum::{
    extract::{Query, State, WebSocketUpgrade, ws},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use proto::{AgentResponse, ChannelEvent, ChannelId, SessionId};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// WebSocket upgrade handler.
pub(super) async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsConnectParams>,
    State(state): State<Arc<WebState>>,
) -> impl IntoResponse {
    // Try session_token first (from POST /auth flow)
    let session_auth = params.session_token.as_deref().and_then(|st| {
        let session = state.auth_sessions.get(st)?;
        if session.expires_at <= Utc::now() {
            state.auth_sessions.remove(st);
            return None;
        }
        Some(session.client_id.clone())
    });

    if let Some(session_client_id) = session_auth {
        // Authenticated via session_token
        let client_id = params
            .client_id
            .filter(|id| !id.is_empty())
            .unwrap_or(session_client_id);
        let session_id = WebAdapter::resolve_session_id(
            params.session_id.as_deref(),
            &state.shared_session_id,
            &client_id,
        );
        let shared_with_tui = WebAdapter::is_shared_with_tui(&session_id, &state.shared_session_id);

        return ws
            .on_upgrade(move |socket| {
                handle_ws(socket, client_id, session_id, shared_with_tui, state)
            })
            .into_response();
    }

    // Fallback: raw token query parameter (handles stale session_token + valid raw token,
    // new tabs with saved token, and legacy ?token= connections)
    let token_valid = params
        .token
        .as_deref()
        .is_some_and(|t| validate_token(t, &state.auth_token));

    if !token_valid && !state.auth_token.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let client_id = params
        .client_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Auto-create a server-side session so subsequent reconnects can use session_token
    if token_valid && !state.auth_token.is_empty() {
        let new_session_token = generate_session_token();
        let expires_at = Utc::now() + ChronoDuration::hours(SESSION_TOKEN_TTL_HOURS);
        state.auth_sessions.insert(
            new_session_token,
            AuthSession {
                expires_at,
                client_id: client_id.clone(),
            },
        );
    }

    let session_id = WebAdapter::resolve_session_id(
        params.session_id.as_deref(),
        &state.shared_session_id,
        &client_id,
    );
    let shared_with_tui = WebAdapter::is_shared_with_tui(&session_id, &state.shared_session_id);

    ws.on_upgrade(move |socket| handle_ws(socket, client_id, session_id, shared_with_tui, state))
        .into_response()
}

/// Manages a single WebSocket connection lifecycle.
async fn handle_ws(
    socket: ws::WebSocket,
    client_id: String,
    session_id: SessionId,
    shared_with_tui: bool,
    state: Arc<WebState>,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Per-client response channel
    let (resp_tx, mut resp_rx) = mpsc::channel::<AgentResponse>(64);
    state.clients.insert(client_id.clone(), resp_tx);
    let channel_id = ChannelId::new("web", &client_id);

    info!(
        client_id = %client_id,
        channel_id = %channel_id,
        session_id = %session_id,
        shared_with_tui,
        "WebSocket client connected"
    );

    // Send auth_result to client
    let auth_msg = WsMessage::AuthResult {
        success: true,
        client_id: Some(client_id.clone()),
        error: None,
        provider: state.selected_provider.read().unwrap().clone(),
        model: state.selected_model.read().unwrap().clone(),
        session_id: Some(session_id.as_str().to_string()),
        shared_with_tui: Some(shared_with_tui),
    };
    if let Err(e) = send_ws_message(&mut ws_tx, &auth_msg).await {
        error!(
            client_id = %client_id,
            channel_id = %channel_id,
            session_id = %session_id,
            "Failed to send auth_result: {e}"
        );
        drop(resp_rx);
        state.clients.remove_if(&client_id, |_, tx| tx.is_closed());
        return;
    }

    let initial_sessions = sessions_snapshot(&state.sessions);
    debug!(
        client_id = %client_id,
        channel_id = %channel_id,
        session_id = %session_id,
        session_count = initial_sessions.len(),
        "Sending initial sessions_list"
    );
    if let Err(e) = send_ws_message(
        &mut ws_tx,
        &WsMessage::SessionsList {
            sessions: initial_sessions,
        },
    )
    .await
    {
        error!(
            client_id = %client_id,
            channel_id = %channel_id,
            session_id = %session_id,
            "Failed to send sessions_list: {e}"
        );
        drop(resp_rx);
        state.clients.remove_if(&client_id, |_, tx| tx.is_closed());
        return;
    }

    // Send session history if a loader is configured
    if let Some(loader) = &state.session_loader {
        match loader.load_session_messages(session_id.as_str()).await {
            Ok(msgs) => {
                let history_msg = WsMessage::SessionHistory {
                    session_id: session_id.as_str().to_string(),
                    messages: to_web_history(msgs),
                };
                if let Err(e) = send_ws_message(&mut ws_tx, &history_msg).await {
                    error!(
                        client_id = %client_id,
                        channel_id = %channel_id,
                        session_id = %session_id,
                        "Failed to send session_history: {e}"
                    );
                    drop(resp_rx);
                    state.clients.remove_if(&client_id, |_, tx| tx.is_closed());
                    return;
                }
                debug!(
                    client_id = %client_id,
                    session_id = %session_id,
                    "Sent session_history on connect"
                );
            }
            Err(e) => {
                warn!(
                    client_id = %client_id,
                    session_id = %session_id,
                    "Failed to load session history: {e}"
                );
            }
        }
    }

    let (ws_out_tx, mut ws_out_rx) = mpsc::channel::<WsMessage>(64);

    let client_id_read = client_id.clone();
    let channel_id_read = channel_id.clone();
    let session_id_read = session_id.clone();
    let state_read = state.clone();
    let ws_out_tx_read = ws_out_tx.clone();

    // Read task: client -> server
    let mut read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            let ws::Message::Text(text) = msg else {
                continue;
            };
            let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) else {
                warn!(
                    client_id = %client_id_read,
                    channel_id = %channel_id_read,
                    session_id = %session_id_read,
                    "Invalid WS message"
                );
                continue;
            };
            match ws_msg {
                WsMessage::UserMessage {
                    content,
                    message_id,
                } => {
                    let event = ChannelEvent::new(
                        channel_id_read.clone(),
                        session_id_read.clone(),
                        content,
                    );
                    match state_read.event_tx.send(event).await {
                        Ok(_) => {
                            let ack = WsMessage::MessageAck {
                                message_id,
                                session_id: session_id_read.as_str().to_string(),
                            };
                            if let Err(e) = ws_out_tx_read.send(ack).await {
                                error!(
                                    client_id = %client_id_read,
                                    channel_id = %channel_id_read,
                                    session_id = %session_id_read,
                                    "Failed to queue message_ack: {e}"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            error!(
                                client_id = %client_id_read,
                                channel_id = %channel_id_read,
                                session_id = %session_id_read,
                                "Failed to enqueue web event: {e}"
                            );
                            let _ = ws_out_tx_read
                                .send(WsMessage::MessageError {
                                    message_id,
                                    session_id: Some(session_id_read.as_str().to_string()),
                                    error: "Failed to queue request. Check server connection."
                                        .to_string(),
                                })
                                .await;
                            break;
                        }
                    }
                }
                WsMessage::Ping => {
                    debug!(
                        client_id = %client_id_read,
                        channel_id = %channel_id_read,
                        session_id = %session_id_read,
                        "Ping received"
                    );
                    if let Err(e) = ws_out_tx_read.send(WsMessage::Pong).await {
                        error!(
                            client_id = %client_id_read,
                            channel_id = %channel_id_read,
                            session_id = %session_id_read,
                            "Failed to queue pong: {e}"
                        );
                        break;
                    }
                }
                WsMessage::SessionsRequest => {
                    debug!(
                        client_id = %client_id_read,
                        channel_id = %channel_id_read,
                        session_id = %session_id_read,
                        "Received sessions_request"
                    );
                    let sessions = sessions_snapshot(&state_read.sessions);
                    debug!(
                        client_id = %client_id_read,
                        channel_id = %channel_id_read,
                        session_id = %session_id_read,
                        session_count = sessions.len(),
                        "Queueing sessions_list"
                    );
                    if let Err(e) = ws_out_tx_read
                        .send(WsMessage::SessionsList { sessions })
                        .await
                    {
                        error!(
                            client_id = %client_id_read,
                            channel_id = %channel_id_read,
                            session_id = %session_id_read,
                            "Failed to queue sessions_list: {e}"
                        );
                        break;
                    }
                }
                WsMessage::SessionHistoryRequest {
                    session_id: req_session_id,
                } => {
                    debug!(
                        client_id = %client_id_read,
                        channel_id = %channel_id_read,
                        session_id = %session_id_read,
                        requested_session = %req_session_id,
                        "Received session_history_request"
                    );
                    if let Some(loader) = &state_read.session_loader {
                        let history_msg = match loader.load_session_messages(&req_session_id).await
                        {
                            Ok(msgs) => WsMessage::SessionHistory {
                                session_id: req_session_id.clone(),
                                messages: to_web_history(msgs),
                            },
                            Err(e) => {
                                warn!(
                                    client_id = %client_id_read,
                                    requested_session = %req_session_id,
                                    "Failed to load session history on request: {e}"
                                );
                                WsMessage::SessionHistory {
                                    session_id: req_session_id.clone(),
                                    messages: Vec::new(),
                                }
                            }
                        };
                        if let Err(e) = ws_out_tx_read.send(history_msg).await {
                            error!(
                                client_id = %client_id_read,
                                channel_id = %channel_id_read,
                                session_id = %session_id_read,
                                "Failed to queue session_history: {e}"
                            );
                            break;
                        }
                    }
                }
                WsMessage::ModelListRequest => {
                    let all_models = state_read.model_list.as_ref().clone();
                    let models = if let Some(cb) = &state_read.provider_list_cb {
                        let providers = cb();
                        let authenticated: std::collections::HashSet<String> = providers
                            .iter()
                            .filter(|p| p.authenticated || p.auth_mode == "none")
                            .map(|p| p.name.clone())
                            .collect();
                        if authenticated.is_empty() {
                            all_models
                        } else {
                            all_models
                                .into_iter()
                                .filter(|m| authenticated.contains(&m.provider))
                                .collect()
                        }
                    } else {
                        all_models
                    };
                    if let Err(e) = ws_out_tx_read.send(WsMessage::ModelList { models }).await {
                        error!(
                            client_id = %client_id_read,
                            channel_id = %channel_id_read,
                            session_id = %session_id_read,
                            "Failed to send model_list: {e}"
                        );
                        break;
                    }
                }
                WsMessage::ModelChange { provider, model } => {
                    info!(
                        client_id = %client_id_read,
                        provider = %provider,
                        model = %model,
                        "Model change requested"
                    );
                    if let Some(cb) = &state_read.model_change_cb {
                        cb(provider.clone(), model.clone());
                    }
                    // Update shared state so new connections see the change
                    *state_read.selected_provider.write().unwrap() = Some(provider.clone());
                    *state_read.selected_model.write().unwrap() = Some(model.clone());
                    if let Err(e) = ws_out_tx_read
                        .send(WsMessage::ModelChanged {
                            provider: provider.clone(),
                            model: model.clone(),
                        })
                        .await
                    {
                        error!(
                            client_id = %client_id_read,
                            channel_id = %channel_id_read,
                            session_id = %session_id_read,
                            "Failed to send model_changed: {e}"
                        );
                        break;
                    }
                }
                WsMessage::ProviderAuthRequest => {
                    if let Some(cb) = &state_read.provider_list_cb {
                        let providers = cb();
                        if let Err(e) = ws_out_tx_read
                            .send(WsMessage::ProviderAuthStatus { providers })
                            .await
                        {
                            error!(
                                client_id = %client_id_read,
                                channel_id = %channel_id_read,
                                session_id = %session_id_read,
                                "Failed to send provider_auth_status: {e}"
                            );
                            break;
                        }
                    }
                }
                WsMessage::ProviderLogin {
                    provider,
                    api_key,
                    endpoint,
                    auth_code,
                } => {
                    info!(
                        client_id = %client_id_read,
                        provider = %provider,
                        "Provider login requested"
                    );
                    if let Some(cb) = &state_read.provider_auth_cb {
                        let intent = ProviderAuthIntent {
                            provider: provider.clone(),
                            api_key,
                            endpoint,
                            auth_code,
                        };
                        let cb = cb.clone();
                        let ws_tx = ws_out_tx_read.clone();
                        tokio::spawn(async move {
                            let result = cb(intent).await;
                            let msg = match result {
                                Ok(ProviderAuthResult::OAuthUrl { url, flow_type }) => {
                                    WsMessage::ProviderAuthUrl {
                                        provider,
                                        auth_url: url,
                                        flow_type,
                                    }
                                }
                                Ok(ProviderAuthResult::Completed { message }) => {
                                    WsMessage::ProviderAuthCompleted {
                                        provider,
                                        success: true,
                                        message,
                                    }
                                }
                                Err(e) => WsMessage::ProviderAuthCompleted {
                                    provider,
                                    success: false,
                                    message: e,
                                },
                            };
                            let _ = ws_tx.send(msg).await;
                        });
                    }
                }
                WsMessage::CancelGeneration => {
                    info!(
                        client_id = %client_id_read,
                        channel_id = %channel_id_read,
                        session_id = %session_id_read,
                        "Cancel generation requested"
                    );
                    // Drop the response sender for this client to signal cancellation.
                    // The spawned processing task will get a send error and stop.
                    // Re-insert a fresh channel so the next message works.
                    let (new_tx, new_rx) = mpsc::channel::<AgentResponse>(64);
                    state_read.clients.insert(client_id_read.clone(), new_tx);
                    drop(new_rx);
                    // Send confirmation back to the client.
                    if let Err(e) = ws_out_tx_read.send(WsMessage::GenerationCancelled).await {
                        error!(
                            client_id = %client_id_read,
                            "Failed to send generation_cancelled: {e}"
                        );
                        break;
                    }
                }
                WsMessage::ToolApprovalResponse { call_id, decision } => {
                    debug!(
                        client_id = %client_id_read,
                        call_id = %call_id,
                        decision = %decision,
                        "Tool approval response received"
                    );
                    let parsed = match decision.as_str() {
                        "approve" => proto::ToolApprovalDecision::Approve,
                        "allow_for_session" => proto::ToolApprovalDecision::AllowForSession,
                        _ => proto::ToolApprovalDecision::Reject,
                    };
                    if let Some((_, tx)) = state_read.pending_approvals.remove(&call_id) {
                        let _ = tx.send(parsed);
                    } else {
                        warn!(
                            call_id = %call_id,
                            "No pending approval request found for call_id"
                        );
                    }
                }
                _ => {
                    debug!(
                        client_id = %client_id_read,
                        channel_id = %channel_id_read,
                        session_id = %session_id_read,
                        "Ignoring WS message"
                    );
                }
            }
        }
    });

    drop(ws_out_tx);

    let mut approval_rx = state.approval_broadcast.subscribe();

    let client_id_write = client_id.clone();
    let channel_id_write = channel_id.clone();
    let session_id_write = session_id.clone();

    // Write task: server -> client
    let mut write_task = tokio::spawn(async move {
        let mut resp_open = true;
        let mut ws_out_open = true;

        loop {
            if !resp_open && !ws_out_open {
                break;
            }
            tokio::select! {
                maybe_resp = resp_rx.recv(), if resp_open => {
                    match maybe_resp {
                        Some(resp) => {
                            let ws_msg = WsMessage::AgentReply {
                                content: resp.content,
                                is_error: resp.is_error,
                            };
                            if let Err(e) = send_ws_message(&mut ws_tx, &ws_msg).await {
                                error!(
                                    client_id = %client_id_write,
                                    channel_id = %channel_id_write,
                                    session_id = %session_id_write,
                                    "Failed to send response over websocket: {e}"
                                );
                                break;
                            }
                        }
                        None => resp_open = false,
                    }
                }
                maybe_ws = ws_out_rx.recv(), if ws_out_open => {
                    match maybe_ws {
                        Some(ws_msg) => {
                            if let Err(e) = send_ws_message(&mut ws_tx, &ws_msg).await {
                                error!(
                                    client_id = %client_id_write,
                                    channel_id = %channel_id_write,
                                    session_id = %session_id_write,
                                    "Failed to send websocket diagnostic event: {e}"
                                );
                                break;
                            }
                        }
                        None => ws_out_open = false,
                    }
                }
                result = approval_rx.recv() => {
                    match result {
                        Ok(ws_msg) => {
                            if let Err(e) = send_ws_message(&mut ws_tx, &ws_msg).await {
                                error!(
                                    client_id = %client_id_write,
                                    channel_id = %channel_id_write,
                                    session_id = %session_id_write,
                                    "Failed to send tool approval request: {e}"
                                );
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(
                                client_id = %client_id_write,
                                lagged = n,
                                "Approval broadcast lagged"
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => {}
                    }
                }
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = &mut read_task => {
            write_task.abort();
            let _ = write_task.await;
        },
        _ = &mut write_task => {
            read_task.abort();
            let _ = read_task.await;
        },
    }

    // Only remove if the stored sender is closed (our receiver was dropped).
    // A newer connection from the same client_id may have already replaced
    // this entry; removing unconditionally would break that connection.
    state.clients.remove_if(&client_id, |_, tx| tx.is_closed());
    info!(
        client_id = %client_id,
        channel_id = %channel_id,
        session_id = %session_id,
        "WebSocket client disconnected"
    );
}

// ─── Helpers ───────────────────────────────────────────────

async fn send_ws_message(
    ws_tx: &mut futures_util::stream::SplitSink<ws::WebSocket, ws::Message>,
    ws_msg: &WsMessage,
) -> Result<(), String> {
    let json = serde_json::to_string(ws_msg).map_err(|e| e.to_string())?;
    ws_tx
        .send(ws::Message::Text(json.into()))
        .await
        .map_err(|e| e.to_string())
}

/// Returns a snapshot of the session list.
///
/// The read lock is held only long enough to clone the inner `Arc` (a pointer
/// bump), so the actual `Vec` clone — if any — happens outside the lock.
fn sessions_snapshot(sessions: &Arc<RwLock<Arc<Vec<WebSessionEntry>>>>) -> Vec<WebSessionEntry> {
    let arc = match sessions.read() {
        Ok(guard) => Arc::clone(&*guard),
        Err(poisoned) => Arc::clone(&*poisoned.into_inner()),
    };
    (*arc).clone()
}
