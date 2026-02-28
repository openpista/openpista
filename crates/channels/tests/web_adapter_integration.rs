use channels::{ChannelAdapter, WebAdapter, WebSessionEntry, web::WsMessage};
use futures_util::{SinkExt, StreamExt};
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

fn make_response(client_id: &str, content: &str) -> AgentResponse {
    AgentResponse::new(
        ChannelId::new("web", client_id),
        SessionId::from(format!("web:{client_id}")),
        content.to_string(),
    )
}

fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

async fn wait_for_health(port: u16) {
    let url = format!("http://127.0.0.1:{port}/health");
    for _ in 0..80 {
        if let Ok(resp) = reqwest::get(&url).await
            && resp.status().is_success()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("web adapter on port {port} did not become healthy in time");
}

async fn recv_ws_message(
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> WsMessage {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str::<WsMessage>(text.as_ref())
                    .expect("valid websocket json message");
            }
            Some(Ok(_)) => continue,
            Some(Err(err)) => panic!("websocket receive failed: {err}"),
            None => panic!("websocket closed unexpectedly"),
        }
    }
}

#[tokio::test]
async fn web_adapter_routes_response_to_registered_client() {
    let adapter = WebAdapter::new(
        3000,
        "secret".to_string(),
        "*".to_string(),
        "".to_string(),
        "shared-main".to_string(),
    );
    let (client_tx, mut client_rx) = mpsc::channel::<AgentResponse>(4);
    adapter.clients.insert("client-1".to_string(), client_tx);

    adapter
        .send_response(make_response("client-1", "hello client"))
        .await
        .expect("send_response should succeed");

    let delivered = timeout(Duration::from_millis(250), client_rx.recv())
        .await
        .expect("timed out waiting for routed response")
        .expect("client channel unexpectedly closed");
    assert_eq!(delivered.channel_id.as_str(), "web:client-1");
    assert_eq!(delivered.content, "hello client");
}

#[tokio::test]
async fn web_adapter_returns_send_failed_when_client_channel_is_closed() {
    let adapter = WebAdapter::new(
        3001,
        "secret".to_string(),
        "*".to_string(),
        "".to_string(),
        "shared-main".to_string(),
    );
    let (client_tx, client_rx) = mpsc::channel::<AgentResponse>(1);
    drop(client_rx);
    adapter
        .clients
        .insert("closed-client".to_string(), client_tx);

    let err = adapter
        .send_response(make_response("closed-client", "should fail"))
        .await
        .expect_err("closed queue must return an error");
    assert!(matches!(err, ChannelError::SendFailed(_)));
}

#[tokio::test]
async fn web_adapter_unknown_client_path_completes_without_blocking_known_client() {
    let adapter = WebAdapter::new(
        3002,
        "secret".to_string(),
        "*".to_string(),
        "".to_string(),
        "shared-main".to_string(),
    );
    let (known_tx, mut known_rx) = mpsc::channel::<AgentResponse>(1);
    adapter.clients.insert("known-client".to_string(), known_tx);

    adapter
        .send_response(make_response("missing-client", "broadcast fallback"))
        .await
        .expect("unknown client path should not fail");

    let known_result = timeout(Duration::from_millis(120), known_rx.recv()).await;
    assert!(
        known_result.is_err(),
        "response for unknown client should not be delivered to other client queues"
    );
}

#[tokio::test]
async fn web_adapter_sessions_request_returns_cached_sessions() {
    let port = pick_free_port();
    let adapter = WebAdapter::new(
        port,
        "secret".to_string(),
        "*".to_string(),
        "".to_string(),
        "shared-main".to_string(),
    );
    adapter.set_sessions(vec![
        WebSessionEntry {
            id: "shared-main".to_string(),
            channel_id: "cli:tui".to_string(),
            updated_at: "2026-02-25T10:00:00Z".to_string(),
            preview: "First prompt".to_string(),
        },
        WebSessionEntry {
            id: "web:client-1".to_string(),
            channel_id: "web:client-1".to_string(),
            updated_at: "2026-02-25T11:00:00Z".to_string(),
            preview: "Second prompt".to_string(),
        },
    ]);

    let (event_tx, _event_rx) = mpsc::channel::<ChannelEvent>(8);
    let server_task = tokio::spawn(async move {
        let _ = adapter.run(event_tx).await;
    });

    wait_for_health(port).await;

    let url = format!(
        "ws://127.0.0.1:{port}/ws?token=secret&client_id=test-client&session_id=shared-main"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect websocket");

    let auth = timeout(Duration::from_secs(2), recv_ws_message(&mut ws))
        .await
        .expect("timeout waiting for auth_result");
    match auth {
        WsMessage::AuthResult { success, .. } => assert!(success),
        other => panic!("expected auth_result, got {other:?}"),
    }

    let initial_list = timeout(Duration::from_secs(2), recv_ws_message(&mut ws))
        .await
        .expect("timeout waiting for initial sessions_list");
    match initial_list {
        WsMessage::SessionsList { sessions } => {
            assert_eq!(sessions.len(), 2);
            assert_eq!(sessions[0].id, "shared-main");
            assert_eq!(sessions[1].id, "web:client-1");
        }
        other => panic!("expected sessions_list, got {other:?}"),
    }

    let request = serde_json::to_string(&WsMessage::SessionsRequest).expect("serialize request");
    ws.send(Message::Text(request.into()))
        .await
        .expect("send sessions_request");

    let refreshed_list = timeout(Duration::from_secs(2), recv_ws_message(&mut ws))
        .await
        .expect("timeout waiting for requested sessions_list");
    match refreshed_list {
        WsMessage::SessionsList { sessions } => {
            assert_eq!(sessions.len(), 2);
            assert_eq!(sessions[0].channel_id, "cli:tui");
            assert_eq!(sessions[1].preview, "Second prompt");
        }
        other => panic!("expected sessions_list, got {other:?}"),
    }

    let _ = ws.close(None).await;
    server_task.abort();
    let _ = server_task.await;
}

// ─── POST /auth integration tests ──────────────────────────

async fn spawn_adapter(token: &str) -> (u16, tokio::task::JoinHandle<()>) {
    let port = pick_free_port();
    let adapter = WebAdapter::new(
        port,
        token.to_string(),
        "*".to_string(),
        "".to_string(),
        "shared-main".to_string(),
    );
    let (event_tx, _event_rx) = mpsc::channel::<ChannelEvent>(8);
    let handle = tokio::spawn(async move {
        let _ = adapter.run(event_tx).await;
    });
    wait_for_health(port).await;
    (port, handle)
}

#[tokio::test]
async fn auth_endpoint_returns_session_token() {
    let (port, server_task) = spawn_adapter("mysecret").await;

    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/auth"))
        .json(&serde_json::json!({"token": "mysecret", "client_id": "test-client"}))
        .send()
        .await
        .expect("POST /auth");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("parse auth response");
    assert_eq!(body["success"], true);
    let session_token = body["session_token"].as_str().expect("session_token field");
    assert_eq!(
        session_token.len(),
        64,
        "session token should be 64 hex chars"
    );
    assert_eq!(body["client_id"], "test-client");
    assert!(body["expires_at"].is_string(), "expires_at should be set");

    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn auth_endpoint_rejects_invalid_token() {
    let (port, server_task) = spawn_adapter("mysecret").await;

    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/auth"))
        .json(&serde_json::json!({"token": "wrongtoken"}))
        .send()
        .await
        .expect("POST /auth");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("parse auth response");
    assert_eq!(body["success"], false);
    assert!(
        body["session_token"].is_null(),
        "no session_token on failure"
    );
    assert!(body["error"].is_string(), "error message should be set");

    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn ws_with_session_token_succeeds() {
    let (port, server_task) = spawn_adapter("mysecret").await;

    // Step 1: POST /auth
    let auth_resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/auth"))
        .json(&serde_json::json!({"token": "mysecret", "client_id": "ws-client"}))
        .send()
        .await
        .expect("POST /auth")
        .json::<serde_json::Value>()
        .await
        .expect("parse auth response");

    let session_token = auth_resp["session_token"]
        .as_str()
        .expect("session_token in auth response");

    // Step 2: WS connect with session_token
    let url = format!(
        "ws://127.0.0.1:{port}/ws?session_token={session_token}&client_id=ws-client&session_id=shared-main"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect websocket");

    let msg = timeout(Duration::from_secs(2), recv_ws_message(&mut ws))
        .await
        .expect("timeout waiting for auth_result");

    match msg {
        WsMessage::AuthResult {
            success, client_id, ..
        } => {
            assert!(success, "auth_result should be success");
            assert_eq!(client_id.as_deref(), Some("ws-client"));
        }
        other => panic!("expected auth_result, got {other:?}"),
    }

    let _ = ws.close(None).await;
    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn ws_with_legacy_token_still_works() {
    let (port, server_task) = spawn_adapter("mysecret").await;

    // Connect directly with legacy ?token= (no POST /auth)
    let url = format!(
        "ws://127.0.0.1:{port}/ws?token=mysecret&client_id=legacy-client&session_id=shared-main"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect websocket");

    let msg = timeout(Duration::from_secs(2), recv_ws_message(&mut ws))
        .await
        .expect("timeout waiting for auth_result");

    match msg {
        WsMessage::AuthResult { success, .. } => {
            assert!(success, "legacy token auth should succeed");
        }
        other => panic!("expected auth_result, got {other:?}"),
    }

    let _ = ws.close(None).await;
    server_task.abort();
    let _ = server_task.await;
}
