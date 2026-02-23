//! OAuth 2.0 PKCE browser-based authentication flow and credential storage.
//!
//! # Flow
//! 1. [`login`] generates a PKCE code verifier/challenge and CSRF state.
//! 2. It builds the authorization URL and opens the browser.
//! 3. A one-shot local HTTP server receives the OAuth redirect callback.
//! 4. The authorization code is exchanged for tokens at the token endpoint.
//! 5. Credentials are persisted to `~/.openpista/credentials.toml`.

use crate::config::OAuthEndpoints;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(not(test))]
use std::time::Duration;
#[cfg(not(test))]
use tracing::warn;
use tracing::{debug, trace};

// ── Credential types ──────────────────────────────────────────────────────────

/// Stored OAuth credential for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    /// Bearer access token (or exchanged API key for OpenAI).
    pub access_token: String,
    /// Custom endpoint URL override for non-standard provider URLs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Refresh token (if provided by the server).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// UTC expiry timestamp (if `expires_in` was returned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// OpenAI id_token — needed to re-exchange for an API key after refresh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

impl ProviderCredential {
    /// Returns `true` if the token has a known expiry that has already passed.
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|t| t < Utc::now())
    }
}

/// All provider credentials, backed by `~/.openpista/credentials.toml`.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Credentials {
    /// Map of provider name to its stored credential.
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderCredential>,
}

impl Credentials {
    /// Returns the path to the on-disk credentials file.
    pub fn path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".openpista")
            .join("credentials.toml")
    }

    /// Loads credentials from disk. Returns an empty set on any I/O or parse error.
    pub fn load() -> Self {
        let path = Self::path();
        trace!(path = %path.display(), exists = %path.exists(), "Loading credentials");
        if !path.exists() {
            return Self::default();
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persists credentials to `path`, creating parent directories as needed.
    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string(self).context("failed to serialize credentials")?;
        std::fs::write(path, content)?;
        debug!(path = %path.display(), providers = %self.providers.len(), "Credentials saved");
        Ok(())
    }

    /// Persists credentials to the default path (`~/.openpista/credentials.toml`).
    #[cfg(not(test))]
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    /// Returns the credential for `provider`, if present.
    pub fn get(&self, provider: &str) -> Option<&ProviderCredential> {
        self.providers.get(provider)
    }

    /// Stores or replaces the credential for `provider`.
    pub fn set(&mut self, provider: String, cred: ProviderCredential) {
        self.providers.insert(provider, cred);
    }

    /// Removes the credential for `provider`. Returns `true` if it existed.
    pub fn remove(&mut self, provider: &str) -> bool {
        self.providers.remove(provider).is_some()
    }
}

// ── PKCE helpers ──────────────────────────────────────────────────────────────

/// Generates a 32-byte random PKCE code verifier (base64url, no padding).
pub fn generate_code_verifier() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64url_encode(&bytes)
}

/// Computes the PKCE S256 code challenge: `BASE64URL(SHA256(verifier))`.
pub fn compute_code_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(verifier.as_bytes());
    base64url_encode(&hash)
}

/// Generates a 16-byte random hex CSRF state value.
pub fn generate_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().fold(String::new(), |mut s, b| {
        s.push_str(&format!("{b:02x}"));
        s
    })
}

/// Base64url-encodes raw bytes (no padding), used for PKCE values.
fn base64url_encode(data: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(data)
}

/// Decodes a base64url-encoded string (no padding) into raw bytes.
fn base64url_decode(data: &str) -> Option<Vec<u8>> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.decode(data).ok()
}

/// Extracts the `chatgpt_account_id` from a JWT access token's claims.
///
/// Looks for `{"https://api.openai.com/auth": {"chatgpt_account_id": "..."}}`
/// in the JWT payload (second segment). Returns `None` on any decode or
/// structure mismatch.
pub fn extract_chatgpt_account_id(jwt: &str) -> Option<String> {
    let parts: Vec<&str> = jwt.splitn(3, '.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload_bytes = base64url_decode(parts[1])?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(String::from)
}

// ── URL helpers ───────────────────────────────────────────────────────────────

/// Percent-encodes a string for use in URL query parameters (RFC 3986).
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-decodes a query-string value (`%XX` sequences and `+` → space).
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let h1 = bytes.next().unwrap_or(b'0') as char;
            let h2 = bytes.next().unwrap_or(b'0') as char;
            if let Ok(byte) = u8::from_str_radix(&format!("{h1}{h2}"), 16) {
                out.push(byte as char);
            }
        } else if b == b'+' {
            out.push(' ');
        } else {
            out.push(b as char);
        }
    }
    out
}

// ── Browser opener ────────────────────────────────────────────────────────────

/// Attempts to open `url` in the default system browser (best-effort).
#[cfg(not(test))]
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

// ── Local callback server ─────────────────────────────────────────────────────

/// Starts a one-shot HTTP server on `127.0.0.1:{port}` and returns the
/// query parameters received on the first incoming request.
#[cfg(not(test))]
async fn receive_callback(port: u16) -> anyhow::Result<HashMap<String, String>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .with_context(|| format!("failed to bind OAuth callback port {port}"))?;

    let (mut stream, _) = listener
        .accept()
        .await
        .context("failed to accept callback connection")?;

    let mut buf = vec![0u8; 8192];
    let n = stream
        .read(&mut buf)
        .await
        .context("failed to read callback request")?;

    let request = String::from_utf8_lossy(&buf[..n]);
    let params = parse_callback_params(&request);

    let body = if params.contains_key("code") {
        "<html><body><h2>&#10003; Authentication successful</h2>\
         <p>You may close this tab and return to the terminal.</p></body></html>"
    } else {
        "<html><body><h2>&#10007; Authentication failed</h2>\
         <p>No authorization code received. You may close this tab.</p></body></html>"
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Connection: close\r\n\r\n{body}"
    );
    let _ = stream.write_all(response.as_bytes()).await;

    Ok(params)
}

/// Extracts query parameters from the first line of an HTTP GET request.
fn parse_callback_params(request: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let first_line = request.lines().next().unwrap_or("");

    // "GET /callback?code=X&state=Y HTTP/1.1"
    let path = first_line
        .strip_prefix("GET ")
        .unwrap_or(first_line)
        .split_whitespace()
        .next()
        .unwrap_or("");

    if let Some(query) = path.split_once('?').map(|x| x.1) {
        for kv in query.split('&') {
            if let Some((k, v)) = kv.split_once('=') {
                params.insert(k.to_string(), percent_decode(v));
            }
        }
    }
    params
}

// ── Token exchange ────────────────────────────────────────────────────────────

/// Deserialisation target for the OAuth token endpoint response.
#[cfg(not(test))]
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    id_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Exchanges an authorization code for tokens at the OAuth token endpoint.
#[cfg(not(test))]
async fn exchange_code(
    token_url: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> anyhow::Result<ProviderCredential> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];

    let response = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .context("token exchange request failed")?;
    debug!(status = %response.status().as_u16(), "Token exchange response");

    let token: TokenResponse = response
        .error_for_status()
        .context("token endpoint returned an error status")?
        .json()
        .await
        .context("failed to parse token response")?;

    let expires_at = token
        .expires_in
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    debug!(
        has_refresh = %token.refresh_token.is_some(),
        expires_in = ?token.expires_in,
        "Token exchange completed"
    );

    Ok(ProviderCredential {
        access_token: token.access_token,
        endpoint: None,
        refresh_token: token.refresh_token,
        expires_at,
        id_token: token.id_token,
    })
}

/// Exchanges an OpenAI `id_token` for a usable API key via the token-exchange grant.
///
/// This is an additional step required after the standard PKCE flow for OpenAI:
/// the OAuth `access_token` lacks API permissions; only the exchanged key works.
#[cfg(not(test))]
async fn exchange_id_token_for_api_key(
    token_url: &str,
    id_token: &str,
    client_id: &str,
) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let params = [
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        ("client_id", client_id),
        (
            "subject_token_type",
            "urn:ietf:params:oauth:token-type:id_token",
        ),
        ("subject_token", id_token),
        ("requested_token", "openai-api-key"),
    ];

    let response = client
        .post(token_url)
        .header("originator", "openpista")
        .form(&params)
        .send()
        .await
        .context("id_token exchange request failed")?;
    let status = response.status();
    debug!(status = %status.as_u16(), "id_token → API key exchange response");

    let body = response
        .text()
        .await
        .context("failed to read exchange response body")?;

    if !status.is_success() {
        let safe_msg = sanitize_error_body(&body);
        debug!(
            status = %status.as_u16(),
            "id_token \u{2192} API key exchange skipped (expected for personal ChatGPT Pro accounts)"
        );
        anyhow::bail!("id_token exchange failed: HTTP {status}: {safe_msg}");
    }
    #[derive(Deserialize)]
    struct ApiKeyResponse {
        access_token: String,
    }

    let result: ApiKeyResponse =
        serde_json::from_str(&body).context("failed to parse id_token exchange response")?;
    Ok(result.access_token)
}

/// Refreshes an OAuth access token using the stored refresh token.
///
/// Returns a new [`ProviderCredential`] with updated tokens.
#[cfg(not(test))]
pub async fn refresh_access_token(
    token_url: &str,
    refresh_token: &str,
    client_id: &str,
) -> anyhow::Result<ProviderCredential> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];

    let response = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .context("refresh token request failed")?;
    debug!(status = %response.status().as_u16(), "Refresh token response");

    let token: TokenResponse = response
        .error_for_status()
        .context("refresh token endpoint returned an error status")?
        .json()
        .await
        .context("failed to parse refresh token response")?;

    let expires_at = token
        .expires_in
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    debug!(
        has_id_token = %token.id_token.is_some(),
        has_refresh = %token.refresh_token.is_some(),
        "Refresh token completed"
    );

    Ok(ProviderCredential {
        access_token: token.access_token,
        endpoint: None,
        refresh_token: token.refresh_token,
        expires_at,
        id_token: token.id_token,
    })
}

/// Refreshes an OpenAI OAuth token and immediately exchanges the new `id_token` for an API key.
///
/// Use this for OpenAI where the raw `access_token` cannot be used as an API key.
#[cfg(not(test))]
pub async fn refresh_and_exchange(
    token_url: &str,
    refresh_token: &str,
    client_id: &str,
) -> anyhow::Result<ProviderCredential> {
    let mut cred = refresh_access_token(token_url, refresh_token, client_id).await?;
    if let Some(ref id_tok) = cred.id_token.clone() {
        match exchange_id_token_for_api_key(token_url, id_tok, client_id).await {
            Ok(api_key) => {
                cred.access_token = api_key;
            }
            Err(e) => {
                debug!(error = %e, "id_token exchange after refresh skipped, keeping access_token");
            }
        }
    }
    Ok(cred)
}

// ── Public login flow ─────────────────────────────────────────────────────────

/// Runs the full OAuth 2.0 PKCE browser-based login flow.
///
/// Steps:
/// 1. Generates a PKCE code verifier/challenge and CSRF state.
/// 2. Opens the browser at the authorization URL.
/// 3. Waits up to `timeout_secs` for the redirect on `127.0.0.1:{callback_port}/callback`.
/// 4. Verifies the CSRF state and exchanges the code for tokens.
///
/// Returns the resulting [`ProviderCredential`] on success.
#[cfg(not(test))]
pub async fn login(
    provider_name: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
    callback_port: u16,
    timeout_secs: u64,
) -> anyhow::Result<ProviderCredential> {
    debug!(provider = %provider_name, port = %callback_port, "Starting OAuth PKCE flow");
    let code_verifier = generate_code_verifier();
    let code_challenge = compute_code_challenge(&code_verifier);
    let state = generate_state();
    let redirect_uri = format!(
        "http://localhost:{callback_port}{}",
        endpoints.redirect_path
    );

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}\
         &id_token_add_organizations=true&codex_cli_simplified_flow=true",
        endpoints.auth_url,
        percent_encode(client_id),
        percent_encode(&redirect_uri),
        percent_encode(endpoints.scope),
        percent_encode(&code_challenge),
        percent_encode(&state),
    );

    println!("Opening browser for {provider_name} authentication...\n");
    println!("  {auth_url}\n");
    println!("(If the browser does not open automatically, copy the URL above.)");
    open_browser(&auth_url);

    println!(
        "\nWaiting for authorization callback on port {callback_port} (timeout: {timeout_secs}s)..."
    );

    let params = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        receive_callback(callback_port),
    )
    .await
    .context("authorization timed out — no callback received within the time limit")?
    .context("failed to receive OAuth callback")?;

    debug!(provider = %provider_name, "OAuth callback received, verifying state");
    // CSRF check
    let received_state = params.get("state").map(|s| s.as_str()).unwrap_or("");
    anyhow::ensure!(
        received_state == state,
        "OAuth state mismatch — possible CSRF attack; aborting"
    );

    // Provider error check
    if let Some(err) = params.get("error") {
        let desc = params
            .get("error_description")
            .map(|s| s.as_str())
            .unwrap_or("");
        anyhow::bail!("provider returned OAuth error '{err}': {desc}");
    }

    let code = params
        .get("code")
        .context("no authorization code in callback")?;

    let mut cred = exchange_code(
        endpoints.token_url,
        client_id,
        code,
        &redirect_uri,
        &code_verifier,
    )
    .await?;

    // OpenAI requires an additional token-exchange step: the `id_token` must be
    // exchanged for an actual API key before the credential can be used.
    if let Some(ref id_tok) = cred.id_token.clone() {
        debug!("Exchanging id_token for API key");
        match exchange_id_token_for_api_key(endpoints.token_url, id_tok, client_id).await {
            Ok(api_key) => {
                cred.access_token = api_key;
                debug!("id_token → API key exchange succeeded");
            }
            Err(e) => {
                warn!(error = %e, "id_token exchange failed, using access_token as-is");
            }
        }
    }

    Ok(cred)
}

// ── Code-display OAuth flow (Anthropic-style) ────────────────────────────────

/// Holds state for the two-phase code-display OAuth flow (Anthropic-style).
#[allow(dead_code)]
pub struct PendingOAuthCodeDisplay {
    /// Full authorization URL opened in the browser.
    #[allow(dead_code)]
    pub auth_url: String,
    /// PKCE code verifier for the token exchange.
    pub code_verifier: String,
    /// CSRF state parameter sent in the auth request.
    pub state: String,
    /// Redirect URI registered with the OAuth provider.
    pub redirect_uri: String,
    /// Token endpoint URL for exchanging the auth code.
    pub token_url: String,
    /// OAuth client identifier.
    pub client_id: String,
}

/// Extracts the scheme+host origin from a URL (e.g. `https://example.com`).
fn auth_url_origin(url: &str) -> &str {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    let origin_end = url[after_scheme..]
        .find('/')
        .map(|i| after_scheme + i)
        .unwrap_or(url.len());
    &url[..origin_end]
}

/// Initiates the code-display OAuth flow by opening the browser and returning pending state.
#[cfg(not(test))]
pub fn start_code_display_flow(
    _provider_name: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
) -> PendingOAuthCodeDisplay {
    let code_verifier = generate_code_verifier();
    let code_challenge = compute_code_challenge(&code_verifier);
    let state = generate_state();
    let redirect_base = endpoints
        .redirect_base
        .unwrap_or_else(|| auth_url_origin(endpoints.auth_url));
    let redirect_uri = format!("{}{}", redirect_base, endpoints.redirect_path);

    let auth_url = format!(
        "{}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}",
        endpoints.auth_url,
        percent_encode(client_id),
        percent_encode(&redirect_uri),
        percent_encode(endpoints.scope),
        percent_encode(&code_challenge),
        percent_encode(&state),
    );

    open_browser(&auth_url);

    PendingOAuthCodeDisplay {
        auth_url,
        code_verifier,
        state,
        redirect_uri,
        token_url: endpoints.token_url.to_string(),
        client_id: client_id.to_string(),
    }
}

/// Test stub for code-display OAuth flow; returns deterministic pending state.
#[cfg(test)]
pub fn start_code_display_flow(
    _provider_name: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
) -> PendingOAuthCodeDisplay {
    PendingOAuthCodeDisplay {
        auth_url: endpoints.auth_url.to_string(),
        code_verifier: "test_verifier".to_string(),
        state: "test_state".to_string(),
        redirect_uri: format!(
            "{}{}",
            endpoints
                .redirect_base
                .unwrap_or_else(|| auth_url_origin(endpoints.auth_url)),
            endpoints.redirect_path
        ),
        token_url: endpoints.token_url.to_string(),
        client_id: client_id.to_string(),
    }
}

/// Exchanges an authorization code for tokens using JSON body format.
#[cfg(not(test))]
async fn exchange_code_json(
    token_url: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    state: &str,
) -> anyhow::Result<ProviderCredential> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "state": state,
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "code_verifier": code_verifier,
    });

    let response = client
        .post(token_url)
        .json(&body)
        .send()
        .await
        .context("token exchange request failed")?;
    debug!(status = %response.status().as_u16(), "Token exchange (JSON) response");

    let token: TokenResponse = response
        .error_for_status()
        .context("token endpoint returned an error status")?
        .json()
        .await
        .context("failed to parse token response")?;

    let expires_at = token
        .expires_in
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    debug!(
        has_refresh = %token.refresh_token.is_some(),
        expires_in = ?token.expires_in,
        "Token exchange (JSON) completed"
    );

    Ok(ProviderCredential {
        access_token: token.access_token,
        endpoint: None,
        refresh_token: token.refresh_token,
        expires_at,
        id_token: token.id_token,
    })
}

/// Completes the code-display OAuth flow by exchanging the user-provided code for tokens.
#[cfg(not(test))]
pub async fn complete_code_display_flow(
    pending: &PendingOAuthCodeDisplay,
    code: &str,
) -> anyhow::Result<ProviderCredential> {
    debug!("Completing code-display OAuth flow, exchanging code for token");
    let clean_code = sanitize_auth_code(code);
    exchange_code_json(
        &pending.token_url,
        &pending.client_id,
        &clean_code,
        &pending.redirect_uri,
        &pending.code_verifier,
        &pending.state,
    )
    .await
}

/// Test stub; always returns an error since OAuth exchange is unavailable in tests.
#[cfg(test)]
pub async fn complete_code_display_flow(
    _pending: &PendingOAuthCodeDisplay,
    _code: &str,
) -> anyhow::Result<ProviderCredential> {
    anyhow::bail!("complete_code_display_flow not available in tests")
}

/// Strips URL fragments and whitespace from a pasted authorization code.
fn sanitize_auth_code(raw: &str) -> String {
    let trimmed = raw.trim();
    match trimmed.find('#') {
        Some(pos) => trimmed[..pos].to_string(),
        None => trimmed.to_string(),
    }
}

/// Reads an authorization code from stdin (interactive prompt).
#[cfg(not(test))]
pub async fn read_code_from_stdin() -> anyhow::Result<String> {
    use tokio::io::AsyncBufReadExt;
    print!("Authorization code: ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let line = stdin
        .lines()
        .next_line()
        .await?
        .context("no input received")?;
    Ok(sanitize_auth_code(&line))
}

/// Test stub; always returns an error since stdin is unavailable in tests.
#[cfg(test)]
pub async fn read_code_from_stdin() -> anyhow::Result<String> {
    anyhow::bail!("read_code_from_stdin not available in tests")
}

/// Extracts a short, non-sensitive error description from a response body.
///
/// Tries to parse a JSON `{"error": "..."}` or `{"message": "..."}` field; falls back to a
/// redacted placeholder so raw tokens are never leaked in logs or error messages.
fn sanitize_error_body(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let msg = v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .or_else(|| v["message"].as_str());
        if let Some(m) = msg {
            return m.chars().take(120).collect();
        }
    }
    "<redacted response>".to_string()
}

/// Returns `true` if `api_key` was obtained via an OpenAI OAuth flow.
///
/// A credential that has a `refresh_token` was issued through token exchange (not a
/// manually-entered API key), so the Responses API (subscription billing) should be
/// used instead of Chat Completions.
pub fn is_openai_oauth_credential_for_key(api_key: &str) -> bool {
    let creds = Credentials::load();
    if let Some(cred) = creds.get("openai") {
        cred.access_token == api_key && cred.refresh_token.is_some()
    } else {
        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_code_verifier_is_url_safe_and_non_empty() {
        let v = generate_code_verifier();
        assert!(!v.is_empty());
        assert!(!v.contains('+'));
        assert!(!v.contains('/'));
        assert!(!v.contains('='));
    }

    #[test]
    fn compute_code_challenge_is_url_safe_and_non_empty() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = compute_code_challenge(verifier);
        assert!(!challenge.is_empty());
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
    }

    #[test]
    fn generate_state_is_32_hex_chars() {
        let s = generate_state();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn percent_encode_decode_roundtrip() {
        let cases = [
            "https://example.com/auth",
            "hello world",
            "a+b=c&d",
            "redirect_uri=http://127.0.0.1:9009/callback",
        ];
        for s in cases {
            assert_eq!(
                percent_decode(&percent_encode(s)),
                s,
                "roundtrip failed for {s:?}"
            );
        }
    }

    #[test]
    fn percent_encode_escapes_reserved_chars() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("a+b=c"), "a%2Bb%3Dc");
        assert_eq!(
            percent_encode("https://example.com"),
            "https%3A%2F%2Fexample.com"
        );
    }

    #[test]
    fn parse_callback_params_extracts_code_and_state() {
        let req = "GET /callback?code=abc123&state=deadbeef HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let params = parse_callback_params(req);
        assert_eq!(params["code"], "abc123");
        assert_eq!(params["state"], "deadbeef");
    }

    #[test]
    fn parse_callback_params_handles_percent_encoded_values() {
        let req = "GET /callback?code=abc%20123&state=xyz HTTP/1.1\r\n\r\n";
        let params = parse_callback_params(req);
        assert_eq!(params["code"], "abc 123");
    }

    #[test]
    fn parse_callback_params_returns_empty_for_no_query() {
        let req = "GET /callback HTTP/1.1\r\n\r\n";
        let params = parse_callback_params(req);
        assert!(params.is_empty());
    }

    #[test]
    fn sanitize_auth_code_strips_fragment() {
        assert_eq!(sanitize_auth_code("abc123#frag"), "abc123");
        assert_eq!(sanitize_auth_code("  abc123  "), "abc123");
        assert_eq!(sanitize_auth_code("abc123#"), "abc123");
        assert_eq!(sanitize_auth_code("abc123"), "abc123");
    }

    #[test]
    fn credentials_set_get_remove_roundtrip() {
        let mut creds = Credentials::default();
        let cred = ProviderCredential {
            access_token: "tok_test".to_string(),
            endpoint: None,
            refresh_token: Some("refresh_test".to_string()),
            expires_at: None,
            id_token: None,
        };
        creds.set("openai".to_string(), cred);
        assert_eq!(creds.get("openai").unwrap().access_token, "tok_test");
        assert!(creds.remove("openai"));
        assert!(creds.get("openai").is_none());
        assert!(!creds.remove("openai")); // already gone
    }

    #[test]
    fn provider_credential_is_expired_with_no_expiry_returns_false() {
        let cred = ProviderCredential {
            access_token: "tok".to_string(),
            endpoint: None,
            refresh_token: None,
            expires_at: None,
            id_token: None,
        };
        assert!(!cred.is_expired());
    }

    #[test]
    fn provider_credential_is_expired_with_past_expiry_returns_true() {
        let cred = ProviderCredential {
            access_token: "tok".to_string(),
            endpoint: None,
            refresh_token: None,
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
            id_token: None,
        };
        assert!(cred.is_expired());
    }

    #[test]
    fn credentials_save_to_and_load_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("credentials.toml");

        let mut creds = Credentials::default();
        creds.set(
            "openai".to_string(),
            ProviderCredential {
                access_token: "sk-test".to_string(),
                endpoint: None,
                refresh_token: Some("rt-test".to_string()),
                expires_at: None,
                id_token: None,
            },
        );

        creds.save_to(&path).expect("save_to");

        let content = std::fs::read_to_string(&path).expect("read");
        let loaded: Credentials = toml::from_str(&content).expect("deserialise");
        assert_eq!(loaded.get("openai").unwrap().access_token, "sk-test");
        assert_eq!(
            loaded.get("openai").unwrap().refresh_token.as_deref(),
            Some("rt-test")
        );
    }

    #[test]
    fn auth_url_origin_extracts_scheme_and_host() {
        assert_eq!(
            auth_url_origin("https://console.anthropic.com/oauth/authorize"),
            "https://console.anthropic.com"
        );
        assert_eq!(
            auth_url_origin("https://auth.openai.com/oauth/authorize"),
            "https://auth.openai.com"
        );
        assert_eq!(
            auth_url_origin("http://localhost:8080/callback"),
            "http://localhost:8080"
        );
        assert_eq!(
            auth_url_origin("https://claude.ai/oauth/authorize"),
            "https://claude.ai"
        );
    }

    #[test]
    fn start_code_display_flow_returns_pending_state() {
        let endpoints = OAuthEndpoints {
            auth_url: "https://console.anthropic.com/oauth/authorize",
            token_url: "https://console.anthropic.com/v1/oauth/token",
            scope: "org:create_api_key",
            default_client_id: Some("test-client-id"),
            default_callback_port: None,
            redirect_path: "/oauth/code/callback",
            redirect_base: None,
        };
        let pending = start_code_display_flow("anthropic", &endpoints, "test-client-id");
        assert_eq!(
            pending.redirect_uri,
            "https://console.anthropic.com/oauth/code/callback"
        );
        assert_eq!(pending.client_id, "test-client-id");
        assert!(!pending.code_verifier.is_empty());
        assert!(!pending.state.is_empty());
    }

    /// Helper: build a fake JWT with the given payload JSON.
    fn fake_jwt(payload_json: &str) -> String {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"RS256\"}");
        let payload = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("{header}.{payload}.fake_signature")
    }

    #[test]
    fn extract_chatgpt_account_id_from_valid_jwt() {
        let jwt = fake_jwt(
            r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_123abc"},"sub":"user"}"#,
        );
        assert_eq!(
            extract_chatgpt_account_id(&jwt),
            Some("acct_123abc".to_string())
        );
    }

    #[test]
    fn extract_chatgpt_account_id_returns_none_for_missing_claim() {
        let jwt = fake_jwt(r#"{"sub":"user","iss":"openai"}"#);
        assert_eq!(extract_chatgpt_account_id(&jwt), None);
    }

    #[test]
    fn extract_chatgpt_account_id_returns_none_for_non_jwt() {
        assert_eq!(extract_chatgpt_account_id("sk-not-a-jwt"), None);
        assert_eq!(extract_chatgpt_account_id(""), None);
    }

    #[test]
    fn extract_chatgpt_account_id_returns_none_when_nested_key_missing() {
        let jwt = fake_jwt(r#"{"https://api.openai.com/auth":{"organization_id":"org_xyz"}}"#);
        assert_eq!(extract_chatgpt_account_id(&jwt), None);
    }

    #[test]
    fn sanitize_error_body_extracts_nested_message() {
        let body = r#"{"error":{"message":"quota exceeded","type":"rate_limit"}}"#;
        assert_eq!(sanitize_error_body(body), "quota exceeded");
    }

    #[test]
    fn sanitize_error_body_extracts_flat_error() {
        let body = r#"{"error":"rate limit hit"}"#;
        assert_eq!(sanitize_error_body(body), "rate limit hit");
    }

    #[test]
    fn sanitize_error_body_extracts_message_field() {
        let body = r#"{"message":"bad request"}"#;
        assert_eq!(sanitize_error_body(body), "bad request");
    }

    #[test]
    fn sanitize_error_body_redacts_unknown_json() {
        let body = r#"{"token":"eyJhbGciOiJSUzI1NiJ9.secret","status":403}"#;
        assert_eq!(sanitize_error_body(body), "<redacted response>");
    }

    #[test]
    fn sanitize_error_body_redacts_non_json() {
        assert_eq!(
            sanitize_error_body("raw body with token"),
            "<redacted response>"
        );
    }

    #[test]
    fn is_openai_oauth_credential_for_key_returns_false_when_no_credentials() {
        // Without a persisted OpenAI credential the function should always return false.
        assert!(!is_openai_oauth_credential_for_key("sk-fake-key-1234"));
    }
}
