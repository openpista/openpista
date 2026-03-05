//! Authentication, credential persistence, and OAuth flow helpers.
#![allow(dead_code, unused_imports)]

use std::str::FromStr;

use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
use crate::config::{
    Config, LoginAuthMode, OAuthEndpoints, ProviderPreset, provider_registry_entry,
};

/// Local port used for the OAuth redirect callback server.
pub(super) const OAUTH_CALLBACK_PORT: u16 = 9009;

/// Maximum seconds to wait for the OAuth callback before timing out.
pub(super) const OAUTH_TIMEOUT_SECS: u64 = 120;

/// Basic API key validation for interactive TUI login input.
pub(super) fn validate_api_key(api_key: String) -> Result<String, String> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty".to_string());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("API key must not contain whitespace".to_string());
    }
    Ok(key)
}

/// Persists one provider credential into credentials storage.
pub(super) fn persist_credential(
    provider: String,
    credential: crate::auth::ProviderCredential,
    path: std::path::PathBuf,
) -> Result<(), String> {
    let mut creds = load_credentials(&path);
    creds.set(provider, credential);
    creds.save_to(&path).map_err(|e| e.to_string())
}

/// Loads provider credentials from the given TOML file path.
pub(super) fn load_credentials(path: &std::path::Path) -> crate::auth::Credentials {
    if !path.exists() {
        return crate::auth::Credentials::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

/// Returns the default on-disk credentials file path.
pub(super) fn credentials_path() -> std::path::PathBuf {
    crate::auth::Credentials::path()
}

#[cfg(not(test))]
/// Runs browser OAuth login flow for one provider.
pub(super) async fn run_oauth_login(
    provider: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
    port: u16,
    timeout: u64,
) -> anyhow::Result<crate::auth::ProviderCredential> {
    crate::auth::login(provider, endpoints, client_id, port, timeout).await
}

#[cfg(test)]
/// Test stub for OAuth flow; keeps tests deterministic.
pub(super) async fn run_oauth_login(
    _provider: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
    _port: u16,
    _timeout: u64,
) -> anyhow::Result<crate::auth::ProviderCredential> {
    let _ = (
        endpoints.auth_url,
        endpoints.token_url,
        endpoints.scope,
        client_id,
    );
    anyhow::bail!("OAuth login is not available in tests")
}

/// Applies provider-specific post-OAuth token exchange.
///
/// GitHub Copilot requires exchanging the GitHub OAuth token for a
/// Copilot-specific session token.  For all other providers the credential
/// is returned unchanged.
pub(super) async fn maybe_exchange_copilot_token(
    provider_name: &str,
    credential: crate::auth::ProviderCredential,
) -> Result<crate::auth::ProviderCredential, String> {
    if provider_name == "github-copilot" {
        #[cfg(not(test))]
        {
            return crate::auth::exchange_github_copilot_token(&credential.access_token)
                .await
                .map_err(|e| format!("Copilot token exchange failed: {e}"));
        }
        #[cfg(test)]
        {
            Ok(credential)
        }
    } else {
        Ok(credential)
    }
}

/// Builds and persists a provider credential using the default credentials path.
pub(crate) async fn build_and_store_credential(
    config: &Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
) -> Result<String, String> {
    build_and_store_credential_with_path(config, intent, port, timeout, credentials_path()).await
}

/// Builds and persists a provider credential to a specified path.
async fn build_and_store_credential_with_path(
    config: &Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
    cred_path: std::path::PathBuf,
) -> Result<String, String> {
    let provider = intent.provider.to_ascii_lowercase();
    let entry = provider_registry_entry(&provider).ok_or_else(|| {
        format!(
            "Unknown provider '{provider}'. Available providers: {}",
            crate::config::provider_registry_names()
        )
    })?;
    let provider_name = entry.name.to_string();
    let resolved_method = intent.auth_method;

    let (credential, success_message) = match entry.auth_mode {
        LoginAuthMode::OAuth => {
            if resolved_method == AuthMethodChoice::ApiKey {
                let raw_key = intent
                    .api_key
                    .ok_or_else(|| "API key input is required".to_string())?;
                let key = validate_api_key(raw_key)?;
                (
                    crate::auth::ProviderCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        endpoint: intent.endpoint,
                        id_token: None,
                    },
                    format!(
                        "Saved API key for '{provider_name}'. It will be used on the next launch (equivalent to setting {}).",
                        entry.api_key_env
                    ),
                )
            } else {
                let endpoints = ProviderPreset::from_str(entry.name)
                    .ok()
                    .and_then(|p| p.oauth_endpoints())
                    .or_else(|| crate::config::extension_oauth_endpoints(&provider_name))
                    .ok_or_else(|| {
                        format!("Provider '{provider_name}' does not support OAuth login")
                    })?;

                let client_id = endpoints
                    .effective_client_id(&config.agent.oauth_client_id)
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        "No OAuth client ID configured. Set openpista_OAUTH_CLIENT_ID environment variable or add oauth_client_id to [agent] in config.toml.".to_string()
                    })?;

                let oauth_credential = if endpoints.default_callback_port.is_none()
                    && !endpoints.redirect_path.is_empty()
                {
                    let pending = crate::auth::start_code_display_flow(
                        &provider_name,
                        &endpoints,
                        &client_id,
                    );
                    let code = if let Some(c) = intent.api_key.as_deref().filter(|s| !s.is_empty())
                    {
                        c.to_string()
                    } else {
                        crate::auth::read_code_from_stdin()
                            .await
                            .map_err(|e| e.to_string())?
                    };
                    crate::auth::complete_code_display_flow(&pending, &code)
                        .await
                        .map_err(|e| e.to_string())?
                } else {
                    let effective_port = endpoints.default_callback_port.unwrap_or(port);
                    run_oauth_login(
                        &provider_name,
                        &endpoints,
                        &client_id,
                        effective_port,
                        timeout,
                    )
                    .await
                    .map_err(|e| e.to_string())?
                };

                let credential =
                    maybe_exchange_copilot_token(&provider_name, oauth_credential).await?;

                (
                    credential,
                    format!(
                        "Authenticated as '{provider_name}'. Token stored in {}",
                        cred_path.display()
                    ),
                )
            }
        }
        LoginAuthMode::ApiKey => {
            let raw_key = intent
                .api_key
                .ok_or_else(|| "API key input is required".to_string())?;
            let key = validate_api_key(raw_key)?;
            (
                crate::auth::ProviderCredential {
                    access_token: key,
                    refresh_token: None,
                    expires_at: None,
                    endpoint: intent.endpoint,
                    id_token: None,
                },
                format!(
                    "Saved API key for '{provider_name}'. It will be used on the next launch (equivalent to setting {}).",
                    entry.api_key_env
                ),
            )
        }
        LoginAuthMode::EndpointAndKey => {
            let raw_key = intent
                .api_key
                .ok_or_else(|| "API key input is required".to_string())?;
            let key = validate_api_key(raw_key)?;
            let endpoint = intent
                .endpoint
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Endpoint is required for this provider".to_string())?;

            (
                crate::auth::ProviderCredential {
                    access_token: key,
                    refresh_token: None,
                    expires_at: None,
                    endpoint: Some(endpoint.clone()),
                    id_token: None,
                },
                format!(
                    "Saved endpoint+key for '{provider_name}'. Endpoint stored as {}.",
                    entry.endpoint_env.unwrap_or("PROVIDER_ENDPOINT")
                ),
            )
        }
        LoginAuthMode::None => {
            return Err(format!(
                "Provider '{provider_name}' does not require authentication"
            ));
        }
    };

    tokio::task::spawn_blocking(move || persist_credential(provider_name, credential, cred_path))
        .await
        .map_err(|e| format!("Auth task join failed: {e}"))??;
    if entry.supports_runtime {
        Ok(success_message)
    } else {
        Ok(format!(
            "{} Credential stored; runtime execution not yet wired.",
            success_message
        ))
    }
}

/// Persists authentication data for OAuth/API-key login paths.
pub(super) async fn persist_auth(
    config: Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
) -> Result<String, String> {
    build_and_store_credential(&config, intent, port, timeout).await
}

/// Test helper that delegates to `build_and_store_credential_with_path`.
#[cfg(test)]
pub(super) async fn persist_auth_with_path(
    config: Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
    cred_path: std::path::PathBuf,
) -> Result<String, String> {
    build_and_store_credential_with_path(&config, intent, port, timeout, cred_path).await
}
