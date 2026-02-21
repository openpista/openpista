//! OAuth 2.0 PKCE browser-based authentication flow and credential storage.
//!
//! # Flow
//! 1. [`login`] generates a PKCE code verifier/challenge and CSRF state.
//! 2. It builds the authorization URL and opens the browser.
//! 3. A one-shot local HTTP server receives the OAuth redirect callback.
//! 4. The authorization code is exchanged for tokens at the token endpoint.
//! 5. Credentials are persisted to `~/.openpista/credentials.toml`.

#[cfg(not(test))]
use crate::config::OAuthEndpoints;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(not(test))]
use std::time::Duration;

// ── Credential types ──────────────────────────────────────────────────────────

/// Stored OAuth credential for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    /// Bearer access token.
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Refresh token (if provided by the server).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// UTC expiry timestamp (if `expires_in` was returned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
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

fn base64url_encode(data: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(data)
}

// ── URL helpers ───────────────────────────────────────────────────────────────

/// Percent-encodes a string using RFC 3986 unreserved characters as the pass-set.
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

    let token: TokenResponse = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .context("token exchange request failed")?
        .error_for_status()
        .context("token endpoint returned an error status")?
        .json()
        .await
        .context("failed to parse token response")?;

    let expires_at = token
        .expires_in
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    Ok(ProviderCredential {
        access_token: token.access_token,
        endpoint: None,
        refresh_token: token.refresh_token,
        expires_at,
    })
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
    let code_verifier = generate_code_verifier();
    let code_challenge = compute_code_challenge(&code_verifier);
    let state = generate_state();
    let redirect_uri = format!("http://localhost:{callback_port}/auth/callback");

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

    exchange_code(
        endpoints.token_url,
        client_id,
        code,
        &redirect_uri,
        &code_verifier,
    )
    .await
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
    fn credentials_set_get_remove_roundtrip() {
        let mut creds = Credentials::default();
        let cred = ProviderCredential {
            access_token: "tok_test".to_string(),
            endpoint: None,
            refresh_token: Some("refresh_test".to_string()),
            expires_at: None,
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
}
