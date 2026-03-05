//! REST HTTP handlers for the web channel adapter.

use super::types::{
    AuthRequest, AuthResponse, AuthSession, SESSION_TOKEN_TTL_HOURS, WebState,
    generate_session_token,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use chrono::{Duration as ChronoDuration, Utc};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Token comparison for authentication.
pub(super) fn validate_token(given: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return true; // no auth required
    }
    given == expected
}

/// Health check endpoint.
pub(super) async fn health_handler() -> &'static str {
    "ok"
}

/// `GET /auth` — redirect to index page (auth modal).
pub(super) async fn auth_page_handler() -> impl IntoResponse {
    Redirect::temporary("/")
}

/// `GET /s/{session_id}` — serve `index.html` for direct session access.
///
/// The client-side JS reads the path to determine which session to connect to.
/// Uses the content cached at startup to avoid per-request disk reads.
pub(super) async fn session_page_handler(
    Path(_session_id): Path<String>,
    State(state): State<Arc<WebState>>,
) -> impl IntoResponse {
    match &state.index_html {
        Some(html) => axum::response::Html(html.clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "index.html not configured or not found",
        )
            .into_response(),
    }
}

/// `POST /auth` — validate token and issue a server-side session token.
pub(super) async fn auth_handler(
    State(state): State<Arc<WebState>>,
    Json(body): Json<AuthRequest>,
) -> impl IntoResponse {
    let token_valid = validate_token(&body.token, &state.auth_token);

    if !token_valid && !state.auth_token.is_empty() {
        return Json(AuthResponse {
            success: false,
            session_token: None,
            client_id: None,
            expires_at: None,
            error: Some("Invalid token".to_string()),
            provider: None,
            model: None,
        });
    }

    let client_id = body
        .client_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let session_token = generate_session_token();
    let expires_at = Utc::now() + ChronoDuration::hours(SESSION_TOKEN_TTL_HOURS);

    state.auth_sessions.insert(
        session_token.clone(),
        AuthSession {
            expires_at,
            client_id: client_id.clone(),
        },
    );

    info!(client_id = %client_id, "Auth session created via POST /auth");

    Json(AuthResponse {
        success: true,
        session_token: Some(session_token),
        client_id: Some(client_id),
        expires_at: Some(expires_at.to_rfc3339()),
        error: None,
        provider: state.selected_provider.read().unwrap().clone(),
        model: state.selected_model.read().unwrap().clone(),
    })
}
