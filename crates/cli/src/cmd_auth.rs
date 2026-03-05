//! Auth subcommand handlers (login, logout, status).

#[cfg(not(test))]
use crate::auth_picker::AuthLoginIntent;
#[cfg(not(test))]
use crate::config::Config;

#[cfg(not(test))]
pub(crate) async fn cmd_auth_login(
    config: Config,
    provider: Option<String>,
    api_key: Option<String>,
    endpoint: Option<String>,
    port: u16,
    timeout: u64,
    non_interactive: bool,
) -> anyhow::Result<()> {
    use crate::auth_picker::AuthMethodChoice;
    use crate::config::{LoginAuthMode, provider_registry_entry_ci, provider_registry_names};

    let lookup_env = |env_name: &str| -> Option<String> {
        if env_name.is_empty() {
            None
        } else {
            std::env::var(env_name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        }
    };

    let intent = if non_interactive {
        let provider = provider
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if provider.is_empty() {
            anyhow::bail!(
                "--non-interactive requires --provider <name>. Available providers: {}",
                provider_registry_names()
            );
        }
        let entry = provider_registry_entry_ci(&provider).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown provider '{provider}'. Available providers: {}",
                provider_registry_names()
            )
        })?;

        let resolved_api_key = api_key
            .filter(|value| !value.trim().is_empty())
            .or_else(|| lookup_env(entry.api_key_env))
            .or_else(|| lookup_env("openpista_API_KEY"));

        let resolved_endpoint = endpoint
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                entry
                    .endpoint_env
                    .and_then(lookup_env)
                    .filter(|value| !value.trim().is_empty())
            });

        match entry.auth_mode {
            LoginAuthMode::None => {
                println!("Provider '{}' does not require login.", entry.name);
                return Ok(());
            }
            LoginAuthMode::EndpointAndKey => {
                let endpoint = resolved_endpoint.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Provider '{}' requires endpoint. Set --endpoint or {}.",
                        entry.name,
                        entry.endpoint_env.unwrap_or("PROVIDER_ENDPOINT")
                    )
                })?;
                let api_key = resolved_api_key.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Provider '{}' requires API key. Set --api-key or {}.",
                        entry.name,
                        entry.api_key_env
                    )
                })?;
                AuthLoginIntent {
                    provider: entry.name.to_string(),
                    auth_method: AuthMethodChoice::ApiKey,
                    endpoint: Some(endpoint),
                    api_key: Some(api_key),
                }
            }
            LoginAuthMode::ApiKey => {
                let api_key = resolved_api_key.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Provider '{}' requires API key. Set --api-key, {}, or openpista_API_KEY.",
                        entry.name,
                        entry.api_key_env
                    )
                })?;
                AuthLoginIntent {
                    provider: entry.name.to_string(),
                    auth_method: AuthMethodChoice::ApiKey,
                    endpoint: resolved_endpoint,
                    api_key: Some(api_key),
                }
            }
            LoginAuthMode::OAuth => {
                let method = if resolved_api_key.is_some() {
                    AuthMethodChoice::ApiKey
                } else {
                    AuthMethodChoice::OAuth
                };
                if method == AuthMethodChoice::OAuth
                    && !crate::config::oauth_available_for(
                        entry.name,
                        &config.agent.oauth_client_id,
                    )
                {
                    anyhow::bail!(
                        "No OAuth client ID configured for '{}'. Set openpista_OAUTH_CLIENT_ID or provide --api-key for API-key fallback.",
                        entry.name
                    );
                }
                AuthLoginIntent {
                    provider: entry.name.to_string(),
                    auth_method: method,
                    endpoint: resolved_endpoint,
                    api_key: resolved_api_key,
                }
            }
        }
    } else {
        let intent = crate::auth_picker::run_cli_auth_picker(
            provider.as_deref(),
            config.agent.oauth_client_id.clone(),
        )?;
        let Some(intent) = intent else {
            println!("Login cancelled.");
            return Ok(());
        };
        intent
    };

    let message = persist_cli_auth_intent(&config, intent, port, timeout).await?;
    println!("\n{message}");
    Ok(())
}

#[cfg(not(test))]
async fn persist_cli_auth_intent(
    config: &Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
) -> anyhow::Result<String> {
    crate::tui::event::build_and_store_credential(config, intent, port, timeout)
        .await
        .map_err(anyhow::Error::msg)
}

#[cfg(not(test))]
/// Removes stored credentials for `provider`.
pub(crate) fn cmd_auth_logout(provider: String) -> anyhow::Result<()> {
    let mut creds = crate::auth::Credentials::load();
    if creds.remove(&provider) {
        creds.save()?;
        println!("Logged out from '{provider}'. Credentials removed.");
    } else {
        println!("No stored credentials found for '{provider}'.");
    }
    Ok(())
}

#[cfg(not(test))]
/// Prints the current authentication status for all stored providers.
pub(crate) fn cmd_auth_status() -> anyhow::Result<()> {
    let creds = crate::auth::Credentials::load();
    if creds.providers.is_empty() {
        println!("No stored credentials. Run `openpista auth login` to authenticate.");
        return Ok(());
    }
    println!(
        "Stored credentials ({}):\n",
        crate::auth::Credentials::path().display()
    );
    for (provider, cred) in &creds.providers {
        let status = match cred.expires_at {
            None => "valid (no expiry)".to_string(),
            Some(t) if t > chrono::Utc::now() => {
                format!("valid until {}", t.format("%Y-%m-%d %H:%M UTC"))
            }
            Some(t) => format!("EXPIRED at {}", t.format("%Y-%m-%d %H:%M UTC")),
        };
        println!("  {provider}: {status}");
    }
    Ok(())
}
