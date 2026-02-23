use proto::ConfigError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{debug, warn};

/// Resolved credential for a specific provider.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    /// API key or access token.
    pub api_key: String,
    /// Optional base URL for the provider.
    pub base_url: Option<String>,
}

/// OAuth 2.0 PKCE application endpoints for a provider.
pub struct OAuthEndpoints {
    /// Authorization endpoint (browser redirect target).
    pub auth_url: &'static str,
    /// Token exchange endpoint (server-side POST).
    pub token_url: &'static str,
    /// Space-separated OAuth scopes to request.
    pub scope: &'static str,
    /// Built-in public client ID (PKCE — not secret).
    /// Users can override via `oauth_client_id` config or `openpista_OAUTH_CLIENT_ID`.
    pub default_client_id: Option<&'static str>,
    /// Default local callback port registered with the OAuth provider.
    /// `None` means the provider uses a remote redirect (code-display flow).
    pub default_callback_port: Option<u16>,
    /// Path component of the OAuth redirect URI.
    /// For localhost flows: appended to `http://localhost:{port}`.
    /// For code-display flows: appended to the auth URL origin.
    /// Base URL for the redirect URI. When `None`, the auth URL origin is used.
    /// Required when auth URL and redirect URL have different domains.
    pub redirect_base: Option<&'static str>,
    pub redirect_path: &'static str,
}

impl OAuthEndpoints {
    /// Returns the effective client ID: user config takes priority, then built-in default.
    /// Returns `None` if neither is available.
    pub fn effective_client_id<'a>(&'a self, configured: &'a str) -> Option<&'a str> {
        let trimmed = configured.trim();
        if !trimmed.is_empty() {
            Some(trimmed)
        } else {
            self.default_client_id
        }
    }
}

/// Known LLM provider presets.
///
/// Each preset auto-configures `base_url` and supplies a default model ID so
/// that users only have to specify what differs from the preset defaults.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderPreset {
    /// OpenAI API (api.openai.com). Default.
    #[default]
    OpenAi,
    /// Anthropic Messages API (api.anthropic.com).
    Anthropic,
    /// Together.ai – OpenAI-compatible endpoint; base_url auto-set.
    Together,
    /// Local Ollama instance – OpenAI-compatible; base_url auto-set, no API key needed.
    Ollama,
    /// OpenRouter – aggregates many providers; base_url auto-set.
    OpenRouter,
    /// Fully custom: set `base_url` and `model` manually.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Authentication UX mode used by TUI `/login`.
pub enum LoginAuthMode {
    /// Browser-based OAuth/PKCE flow.
    OAuth,
    /// API-key-only provider.
    ApiKey,
    /// Endpoint + key provider.
    EndpointAndKey,
    /// No login required.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Provider classification for picker badges.
pub enum ProviderCategory {
    /// Provider is wired into runtime model execution.
    Runtime,
    /// Provider is a credential slot only (runtime pending).
    Extension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Registry metadata for one provider entry.
pub struct ProviderRegistryEntry {
    /// Canonical provider id used in `/login`.
    pub name: &'static str,
    /// Human-readable provider label used by pickers.
    pub display_name: &'static str,
    /// Provider category badge shown in pickers.
    pub category: ProviderCategory,
    /// Sort order for provider picker (lower first).
    pub sort_order: u16,
    /// Additional keywords matched by picker search.
    pub search_aliases: &'static [&'static str],
    /// Authentication mode used by this provider.
    pub auth_mode: LoginAuthMode,
    /// API key env name for this provider.
    pub api_key_env: &'static str,
    /// Optional endpoint env name for endpoint+key providers.
    pub endpoint_env: Option<&'static str>,
    /// Whether the provider is currently wired into runtime model execution.
    pub supports_runtime: bool,
}

const EXTENSION_PROVIDER_SLOTS: &[ProviderRegistryEntry] = &[
    ProviderRegistryEntry {
        name: "github-copilot",
        display_name: "GitHub Copilot",
        category: ProviderCategory::Extension,
        sort_order: 30,
        search_aliases: &["github", "copilot", "gh"],
        auth_mode: LoginAuthMode::ApiKey,
        api_key_env: "GITHUB_COPILOT_TOKEN",
        endpoint_env: None,
        supports_runtime: false,
    },
    ProviderRegistryEntry {
        name: "google",
        display_name: "Google",
        category: ProviderCategory::Extension,
        sort_order: 50,
        search_aliases: &["google", "gemini"],
        auth_mode: LoginAuthMode::ApiKey,
        api_key_env: "GOOGLE_API_KEY",
        endpoint_env: None,
        supports_runtime: false,
    },
    ProviderRegistryEntry {
        name: "vercel-ai-gateway",
        display_name: "Vercel AI Gateway",
        category: ProviderCategory::Extension,
        sort_order: 70,
        search_aliases: &["vercel", "ai gateway"],
        auth_mode: LoginAuthMode::ApiKey,
        api_key_env: "VERCEL_AI_GATEWAY_API_KEY",
        endpoint_env: None,
        supports_runtime: false,
    },
    ProviderRegistryEntry {
        name: "azure-openai",
        display_name: "Azure OpenAI",
        category: ProviderCategory::Extension,
        sort_order: 80,
        search_aliases: &["azure", "aoai", "openai azure"],
        auth_mode: LoginAuthMode::EndpointAndKey,
        api_key_env: "AZURE_OPENAI_API_KEY",
        endpoint_env: Some("AZURE_OPENAI_ENDPOINT"),
        supports_runtime: false,
    },
    ProviderRegistryEntry {
        name: "bedrock",
        display_name: "AWS Bedrock",
        category: ProviderCategory::Extension,
        sort_order: 90,
        search_aliases: &["aws", "bedrock"],
        auth_mode: LoginAuthMode::EndpointAndKey,
        // AWS Bedrock credentials come from ACCESS_KEY_ID + SECRET_ACCESS_KEY.
        api_key_env: "AWS_SECRET_ACCESS_KEY",
        endpoint_env: Some("AWS_REGION"),
        supports_runtime: false,
    },
];

impl ProviderPreset {
    /// Returns all currently supported runtime provider presets.
    pub const fn all() -> &'static [Self] {
        &[
            Self::OpenAi,
            Self::Anthropic,
            Self::Together,
            Self::Ollama,
            Self::OpenRouter,
            Self::Custom,
        ]
    }

    /// Default model ID for the preset. Used when `AgentConfig::model` is empty.
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-4o",
            Self::Anthropic => "claude-sonnet-4-6",
            Self::Together => "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            Self::Ollama => "llama3.2",
            Self::OpenRouter => "openai/gpt-4o",

            Self::Custom => "",
        }
    }

    /// Auto-configured API base URL (`None` = use the SDK's built-in default).
    pub fn base_url(&self) -> Option<&'static str> {
        match self {
            Self::OpenAi => None,
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Together => Some("https://api.together.xyz/v1"),
            Self::Ollama => Some("http://localhost:11434/v1"),
            Self::OpenRouter => Some("https://openrouter.ai/api/v1"),

            Self::Custom => None,
        }
    }

    /// Name of the provider-specific API key environment variable.
    /// Empty string means no API key is required (e.g. Ollama).
    pub fn api_key_env(&self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::Together => "TOGETHER_API_KEY",
            Self::Ollama => "",
            Self::OpenRouter => "OPENROUTER_API_KEY",

            Self::Custom => "OPENAI_API_KEY",
        }
    }

    /// Canonical lowercase name used as the credential-store key.
    pub fn name(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Together => "together",
            Self::Ollama => "ollama",
            Self::OpenRouter => "openrouter",

            Self::Custom => "custom",
        }
    }

    /// Returns OAuth 2.0 PKCE endpoints for providers that support browser login.
    /// Returns `None` for providers without a supported OAuth flow
    /// (Together.ai and Ollama use API keys only).
    pub fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        match self {
            Self::OpenAi => Some(OAuthEndpoints {
                auth_url: "https://auth.openai.com/oauth/authorize",
                token_url: "https://auth.openai.com/oauth/token",
                scope: "openid profile email offline_access",
                default_client_id: Some("app_EMoamEEZ73f0CkXaXp7hrann"),
                default_callback_port: Some(1455),
                redirect_path: "/auth/callback",
                redirect_base: None,
            }),
            Self::Anthropic => Some(OAuthEndpoints {
                auth_url: "https://claude.ai/oauth/authorize",
                token_url: "https://platform.claude.com/v1/oauth/token",
                scope: "user:profile user:inference",
                default_client_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
                default_callback_port: None,
                redirect_path: "/oauth/code/callback",
                redirect_base: Some("https://platform.claude.com"),
            }),
            Self::OpenRouter => Some(OAuthEndpoints {
                auth_url: "https://openrouter.ai/auth",
                token_url: "https://openrouter.ai/api/v1/auth/keys",
                scope: "",
                default_client_id: None,
                default_callback_port: None,
                redirect_path: "",
                redirect_base: None,
            }),

            _ => None,
        }
    }

    /// Returns high-level authentication requirement for this preset.
    pub fn auth_requirements(&self) -> AuthRequirement {
        if self.oauth_endpoints().is_some() {
            AuthRequirement::OAuth
        } else if self.api_key_env().is_empty() {
            AuthRequirement::None
        } else {
            AuthRequirement::ApiKey
        }
    }

    /// Converts a runtime preset into a `/login` registry entry.
    pub fn registry_entry(&self) -> ProviderRegistryEntry {
        let auth_mode = match self.auth_requirements() {
            AuthRequirement::OAuth => LoginAuthMode::OAuth,
            AuthRequirement::ApiKey => {
                if matches!(self, Self::Custom) {
                    LoginAuthMode::EndpointAndKey
                } else {
                    LoginAuthMode::ApiKey
                }
            }
            AuthRequirement::None => LoginAuthMode::None,
        };

        let endpoint_env = if matches!(self, Self::Custom) {
            Some("openpista_BASE_URL")
        } else {
            None
        };

        ProviderRegistryEntry {
            name: self.name(),
            display_name: match self {
                Self::OpenAi => "OpenAI (ChatGPT Plus/Pro or API key)",
                Self::Anthropic => "Anthropic (Claude)",
                Self::OpenRouter => "OpenRouter",
                Self::Together => "Together",
                Self::Ollama => "Ollama",
                Self::Custom => "Custom OpenAI-Compatible",
            },
            category: ProviderCategory::Runtime,
            sort_order: match self {
                Self::Anthropic => 20,
                Self::OpenAi => 40,
                Self::OpenRouter => 60,
                Self::Together => 110,
                Self::Ollama => 140,
                Self::Custom => 150,
            },
            search_aliases: match self {
                Self::OpenAi => &["openai", "chatgpt", "gpt"],
                Self::Anthropic => &["anthropic", "claude", "claude-3", "claude-4"],
                Self::OpenRouter => &["router", "openrouter"],
                Self::Together => &["together", "llama", "mixtral"],
                Self::Ollama => &["ollama", "local"],
                Self::Custom => &["custom", "openai-compatible", "proxy"],
            },
            auth_mode,
            api_key_env: self.api_key_env(),
            endpoint_env,
            supports_runtime: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Low-level auth requirement derived from runtime preset metadata.
pub enum AuthRequirement {
    /// OAuth/PKCE authentication.
    OAuth,
    /// API key authentication.
    ApiKey,
    /// No authentication required.
    None,
}

fn provider_registry_entries() -> &'static Vec<ProviderRegistryEntry> {
    static REGISTRY: OnceLock<Vec<ProviderRegistryEntry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut entries: Vec<ProviderRegistryEntry> = ProviderPreset::all()
            .iter()
            .map(ProviderPreset::registry_entry)
            .collect();
        entries.extend_from_slice(EXTENSION_PROVIDER_SLOTS);
        entries
    })
}

fn provider_registry_for_picker_entries() -> &'static Vec<ProviderRegistryEntry> {
    static PICKER: OnceLock<Vec<ProviderRegistryEntry>> = OnceLock::new();
    PICKER.get_or_init(|| {
        let mut entries = provider_registry_entries().clone();
        entries.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.display_name.cmp(b.display_name))
        });
        entries
    })
}

/// Returns the merged provider registry (runtime providers + extension slots).
pub fn provider_registry() -> Vec<ProviderRegistryEntry> {
    provider_registry_entries().clone()
}

/// Resolves one provider entry by id (case-insensitive).
pub fn provider_registry_entry(name: &str) -> Option<ProviderRegistryEntry> {
    provider_registry_entry_ci(name)
}

/// Resolves one provider entry by id (case-insensitive).
pub fn provider_registry_entry_ci(name: &str) -> Option<ProviderRegistryEntry> {
    let needle = name.trim().to_ascii_lowercase();
    provider_registry_entries()
        .iter()
        .find(|entry| entry.name == needle)
        .cloned()
}

/// Returns picker-ordered provider entries.
pub fn provider_registry_for_picker() -> Vec<ProviderRegistryEntry> {
    provider_registry_for_picker_entries().clone()
}

/// Returns a comma-separated provider name list for user prompts.
#[allow(dead_code)]
pub fn provider_registry_names() -> String {
    provider_registry()
        .iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Top-level CLI configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Gateway networking configuration.
    #[serde(default)]
    pub gateway: GatewayConfig,

    /// Agent provider/model configuration.
    #[serde(default)]
    pub agent: AgentConfig,

    /// Channel adapter configuration.
    #[serde(default)]
    pub channels: ChannelsConfig,

    /// Database configuration.
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Skills workspace configuration.
    #[serde(default)]
    pub skills: SkillsConfig,
}

/// QUIC gateway config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// QUIC listening port.
    pub port: u16,
    /// Optional host/IP advertised to worker containers for QUIC report callbacks.
    /// Defaults to loopback when omitted.
    pub report_host: Option<String>,
    /// Optional TLS cert path/content setting.
    pub tls_cert: String,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 4433,
            report_host: None,
            tls_cert: String::new(),
        }
    }
}

fn default_max_tool_rounds() -> usize {
    10
}

impl std::str::FromStr for ProviderPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "together" => Ok(Self::Together),
            "ollama" => Ok(Self::Ollama),
            "openrouter" => Ok(Self::OpenRouter),

            "custom" => Ok(Self::Custom),
            other => Err(format!("unknown provider '{other}'")),
        }
    }
}

/// Returns OAuth endpoints for extension providers that support browser login.
pub fn extension_oauth_endpoints(provider_name: &str) -> Option<OAuthEndpoints> {
    let _ = provider_name;
    None
}

/// Returns true if OAuth login is available for the given provider name.
/// Checks user-configured client ID first, then provider's built-in default,
/// then extension provider endpoints.
pub fn oauth_available_for(provider_name: &str, config_client_id: &str) -> bool {
    if !config_client_id.trim().is_empty() {
        return true;
    }
    if provider_name
        .parse::<ProviderPreset>()
        .ok()
        .and_then(|p| p.oauth_endpoints())
        .and_then(|e| e.default_client_id)
        .is_some()
    {
        return true;
    }
    extension_oauth_endpoints(provider_name)
        .and_then(|e| e.default_client_id)
        .is_some()
}

/// Agent model/provider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Provider preset: openai | together | ollama | openrouter | custom.
    #[serde(default)]
    pub provider: ProviderPreset,
    /// Model ID. Leave empty (or omit) to use the preset default.
    #[serde(default)]
    pub model: String,
    /// API key (env overrides applied at load time; see `Config::load`).
    #[serde(default)]
    pub api_key: String,
    /// Maximum tool-call rounds per request before bailing out.
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    /// Explicit API base URL. Overrides the preset URL when non-empty.
    /// Required for `provider = "custom"`; optional for others.
    pub base_url: Option<String>,
    /// OAuth 2.0 client ID for `openpista auth login`.
    /// Must be registered with the provider. Also read from
    /// `openpista_OAUTH_CLIENT_ID` environment variable.
    #[serde(default)]
    pub oauth_client_id: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: ProviderPreset::default(),
            model: String::new(),
            api_key: String::new(),
            max_tool_rounds: 10,
            base_url: None,
            oauth_client_id: String::new(),
        }
    }
}

impl AgentConfig {
    /// Returns the effective model ID.
    /// Falls back to the preset default when `model` is empty.
    pub fn effective_model(&self) -> &str {
        if self.model.is_empty() {
            self.provider.default_model()
        } else {
            &self.model
        }
    }

    /// Returns the effective API base URL.
    /// Priority: explicit `base_url` field > preset auto-URL > `None`.
    pub fn effective_base_url(&self) -> Option<&str> {
        if let Some(url) = &self.base_url
            && !url.is_empty()
        {
            return Some(url.as_str());
        }
        self.provider.base_url()
    }
}

/// Container for all channel adapter configs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    /// Telegram adapter config.
    pub telegram: TelegramConfig,
    /// Local CLI adapter config.
    pub cli: CliConfig,
    /// Mobile QUIC adapter config.
    #[serde(default)]
    pub mobile: MobileConfig,
}

/// Telegram adapter config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    /// Whether Telegram adapter is enabled.
    pub enabled: bool,
    /// Telegram bot token.
    pub token: String,
}

/// Mobile QUIC channel config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileConfig {
    /// Whether the mobile QUIC adapter is enabled.
    pub enabled: bool,
    /// QUIC listen port for mobile clients.
    pub port: u16,
    /// Bearer token that mobile clients must present on authentication.
    pub api_token: String,
}

impl Default for MobileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 4434,
            api_token: String::new(),
        }
    }
}

/// Local CLI channel config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    /// Whether CLI adapter is enabled.
    pub enabled: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Database storage config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// SQLite file path.
    pub url: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            url: format!("{home}/.openpista/memory.db"),
        }
    }
}

/// Skills workspace config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Workspace root where `skills/` directory lives.
    pub workspace: String,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            workspace: format!("{home}/.openpista/workspace"),
        }
    }
}

impl Config {
    /// Loads configuration from explicit path, fallback locations, and env overrides.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let config_path = path.map(|p| p.to_path_buf()).or_else(|| {
            // Look in current dir, then home dir
            let cwd = std::env::current_dir().ok()?.join("config.toml");
            if cwd.exists() {
                return Some(cwd);
            }
            let home = std::env::var("HOME").ok()?;
            let home_config = PathBuf::from(home).join(".openpista").join("config.toml");
            if home_config.exists() {
                return Some(home_config);
            }
            None
        });
        debug!(path = ?config_path, "Config file resolved");

        let mut config = if let Some(path) = config_path {
            let content = std::fs::read_to_string(&path).map_err(ConfigError::Io)?;
            toml::from_str(&content).map_err(|e| ConfigError::Toml(e.to_string()))?
        } else {
            Config::default()
        };

        // Environment variable overrides (highest priority → lowest)
        if let Ok(key) = std::env::var("openpista_API_KEY") {
            config.agent.api_key = key;
        }
        if let Ok(model) = std::env::var("openpista_MODEL") {
            config.agent.model = model;
        }
        if let Ok(token) = std::env::var("TELEGRAM_BOT_TOKEN") {
            config.channels.telegram.token = token;
            config.channels.telegram.enabled = true;
        }
        if let Ok(client_id) = std::env::var("openpista_OAUTH_CLIENT_ID") {
            config.agent.oauth_client_id = client_id;
        }
        if let Ok(token) = std::env::var("openpista_MOBILE_TOKEN") {
            config.channels.mobile.api_token = token;
            config.channels.mobile.enabled = true;
        }
        if let Ok(workspace) = std::env::var("openpista_WORKSPACE") {
            config.skills.workspace = workspace;
        }

        debug!(
            provider = %config.agent.provider.name(),
            model = %config.agent.effective_model(),
            base_url = ?config.agent.effective_base_url(),
            "Config loaded"
        );
        Ok(config)
    }

    /// Resolves the API key to use for the configured provider.
    ///
    /// Priority:
    /// 1. `agent.api_key` in config file (or `openpista_API_KEY` applied at load time)
    /// 2. Valid (non-expired) token stored by `openpista auth login`
    /// 3. Provider-specific environment variable (e.g. `TOGETHER_API_KEY`)
    /// 4. `OPENAI_API_KEY` (legacy fallback)
    pub fn resolve_api_key(&self) -> String {
        if !self.agent.api_key.is_empty() {
            debug!(source = "config", provider = %self.agent.provider.name(), "API key resolved");
            return self.agent.api_key.clone();
        }

        // Credential store written by `auth login`
        let creds = crate::auth::Credentials::load();
        if let Some(cred) = creds.get(self.agent.provider.name())
            && !cred.is_expired()
        {
            debug!(source = "credential_store", provider = %self.agent.provider.name(), "API key resolved");
            // Warn about potentially stale credential formats
            let token = &cred.access_token;
            if self.agent.provider == ProviderPreset::OpenAi
                && token.starts_with("eyJ")
                && cred.id_token.is_none()
            {
                warn!(
                    provider = "openai",
                    "Stored credential looks like a raw OAuth JWT — consider re-login with `openpista auth login`"
                );
            }
            if self.agent.provider == ProviderPreset::Anthropic
                && token.starts_with("sk-ant-api03-")
            {
                warn!(
                    provider = "anthropic",
                    "Stored credential looks like a workspace API key — consider re-login with `openpista auth login`"
                );
            }
            return cred.access_token.clone();
        }

        // Provider-specific env var
        let env_var = self.agent.provider.api_key_env();
        if !env_var.is_empty()
            && let Ok(key) = std::env::var(env_var)
        {
            debug!(source = "env", env_var = %env_var, "API key resolved");
            return key;
        }

        // Legacy fallback
        let fallback = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        if fallback.is_empty() {
            debug!(provider = %self.agent.provider.name(), "No API key found from any source");
        } else {
            debug!(
                source = "legacy_fallback",
                "API key resolved from OPENAI_API_KEY"
            );
        }
        fallback
    }

    /// Test stub: delegates directly to [`resolve_api_key`] (no network calls in tests).
    #[cfg(test)]
    pub async fn resolve_api_key_refreshed(&self) -> String {
        self.resolve_api_key()
    }

    /// Like [`resolve_api_key`] but also attempts to auto-refresh an expired (or nearly
    /// expired) OAuth token before returning.
    ///
    /// If the stored credential expires within 5 minutes, this method tries to refresh
    /// it via the provider's token endpoint and persists the updated credential.
    /// On any refresh failure it falls back gracefully to the existing token.
    #[cfg(not(test))]
    pub async fn resolve_api_key_refreshed(&self) -> String {
        use crate::auth::{Credentials, refresh_access_token, refresh_and_exchange};
        use chrono::Utc;

        if !self.agent.api_key.is_empty() {
            return self.agent.api_key.clone();
        }

        let mut creds = Credentials::load();
        let provider_name = self.agent.provider.name();

        if let Some(cred) = creds.get(provider_name) {
            let near_expiry = cred
                .expires_at
                .is_some_and(|t| t < Utc::now() + chrono::Duration::minutes(5));

            // Force refresh when Anthropic workspace key (sk-ant-api03-) needs OAuth upgrade.
            // OpenAI JWTs (eyJ...) are the correct format for ChatGPT Pro subscriptions
            // and do NOT need id_token exchange — only refresh when near_expiry.
            let is_stale_format = self.agent.provider == ProviderPreset::Anthropic
                && cred.access_token.starts_with("sk-ant-api03-");

            if (near_expiry || is_stale_format)
                && let Some(rt) = cred.refresh_token.clone()
                && let Some(endpoints) = self.agent.provider.oauth_endpoints()
            {
                let client_id = endpoints
                    .effective_client_id(&self.agent.oauth_client_id)
                    .unwrap_or_default()
                    .to_string();
                let is_openai = self.agent.provider == ProviderPreset::OpenAi;
                let refresh_result = if is_openai {
                    refresh_and_exchange(endpoints.token_url, &rt, &client_id).await
                } else {
                    refresh_access_token(endpoints.token_url, &rt, &client_id).await
                };
                if let Ok(new_cred) = refresh_result {
                    let api_key = new_cred.access_token.clone();
                    creds.set(provider_name.to_string(), new_cred);
                    let _ = creds.save();
                    debug!(source = "refreshed_credential", provider = %provider_name, "API key resolved after refresh");
                    return api_key;
                }
                debug!(provider = %provider_name, "Token refresh failed, using existing credential");
            }

            if !cred.is_expired() {
                debug!(source = "credential_store", provider = %provider_name, "API key resolved");
                return cred.access_token.clone();
            }
        }

        // Fall back to env vars / legacy key
        self.resolve_api_key()
    }

    /// Resolves the API key for an arbitrary provider name (not just the configured one).
    /// Used by multi-provider model catalog loading.
    ///
    /// Priority:
    /// 1. If `provider_name` matches the configured provider, use `resolve_api_key()`
    /// 2. Valid (non-expired) token stored by `openpista auth login`
    /// 3. Provider-specific environment variable
    pub fn resolve_credential_for(&self, provider_name: &str) -> Option<ResolvedCredential> {
        // If it's the configured provider, delegate to the existing method
        if provider_name == self.agent.provider.name() {
            let key = self.resolve_api_key();
            if key.is_empty() {
                return None;
            }
            return Some(ResolvedCredential {
                api_key: key,
                base_url: self.agent.effective_base_url().map(String::from),
            });
        }

        // Try credential store
        let creds = crate::auth::Credentials::load();
        if let Some(cred) = creds.get(provider_name)
            && !cred.is_expired()
        {
            let base_url = provider_name
                .parse::<ProviderPreset>()
                .ok()
                .and_then(|p| p.base_url().map(String::from));
            return Some(ResolvedCredential {
                api_key: cred.access_token.clone(),
                base_url,
            });
        }

        // Try provider-specific env var
        if let Ok(preset) = provider_name.parse::<ProviderPreset>() {
            let env_var = preset.api_key_env();
            if !env_var.is_empty()
                && let Ok(key) = std::env::var(env_var)
            {
                return Some(ResolvedCredential {
                    api_key: key,
                    base_url: preset.base_url().map(String::from),
                });
            }
        }

        None
    }

    /// Async version of [`resolve_credential_for`] that auto-refreshes stale OAuth tokens.
    #[cfg(not(test))]
    pub async fn resolve_credential_for_refreshed(
        &self,
        provider_name: &str,
    ) -> Option<ResolvedCredential> {
        use crate::auth::{Credentials, refresh_access_token, refresh_and_exchange};
        use chrono::Utc;

        // If it's the configured provider, delegate to the async method
        if provider_name == self.agent.provider.name() {
            let key = self.resolve_api_key_refreshed().await;
            if key.is_empty() {
                return None;
            }
            return Some(ResolvedCredential {
                api_key: key,
                base_url: self.agent.effective_base_url().map(String::from),
            });
        }

        // Try credential store with refresh for other providers
        let mut creds = Credentials::load();
        if let Some(cred) = creds.get(provider_name) {
            let preset = provider_name.parse::<ProviderPreset>().ok();
            let near_expiry = cred
                .expires_at
                .is_some_and(|t| t < Utc::now() + chrono::Duration::minutes(5));
            // Only Anthropic workspace keys (sk-ant-api03-) are stale and need upgrade.
            // OpenAI JWTs are the correct format for ChatGPT Pro subscriptions.
            let is_stale_format = match provider_name {
                "anthropic" => cred.access_token.starts_with("sk-ant-api03-"),
                _ => false,
            };

            if (near_expiry || is_stale_format)
                && let Some(rt) = cred.refresh_token.clone()
                && let Some(ref p) = preset
                && let Some(endpoints) = p.oauth_endpoints()
            {
                let client_id = endpoints
                    .effective_client_id(&self.agent.oauth_client_id)
                    .unwrap_or_default()
                    .to_string();
                let is_openai = provider_name == "openai";
                let refresh_result = if is_openai {
                    refresh_and_exchange(endpoints.token_url, &rt, &client_id).await
                } else {
                    refresh_access_token(endpoints.token_url, &rt, &client_id).await
                };
                if let Ok(new_cred) = refresh_result {
                    let api_key = new_cred.access_token.clone();
                    creds.set(provider_name.to_string(), new_cred);
                    let _ = creds.save();
                    let base_url = preset.and_then(|p| p.base_url().map(String::from));
                    return Some(ResolvedCredential { api_key, base_url });
                }
            }

            if !cred.is_expired() {
                let base_url = preset.and_then(|p| p.base_url().map(String::from));
                return Some(ResolvedCredential {
                    api_key: cred.access_token.clone(),
                    base_url,
                });
            }
        }

        // Try provider-specific env var
        if let Ok(preset) = provider_name.parse::<ProviderPreset>() {
            let env_var = preset.api_key_env();
            if !env_var.is_empty()
                && let Ok(key) = std::env::var(env_var)
            {
                return Some(ResolvedCredential {
                    api_key: key,
                    base_url: preset.base_url().map(String::from),
                });
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{remove_env_var, set_env_var, with_locked_env};

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, content).expect("write config");
    }

    #[test]
    fn default_config_has_expected_values() {
        let cfg = Config::default();
        assert_eq!(cfg.gateway.port, 4433);
        assert_eq!(cfg.agent.provider, ProviderPreset::OpenAi);
        assert_eq!(cfg.agent.effective_model(), "gpt-4o");
        assert_eq!(cfg.agent.max_tool_rounds, 10);
        assert!(!cfg.database.url.is_empty());
        assert!(cfg.channels.cli.enabled);
    }

    #[test]
    fn provider_preset_auto_config() {
        assert_eq!(ProviderPreset::OpenAi.base_url(), None);
        assert_eq!(
            ProviderPreset::Together.base_url(),
            Some("https://api.together.xyz/v1")
        );
        assert_eq!(
            ProviderPreset::Ollama.base_url(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(
            ProviderPreset::OpenRouter.base_url(),
            Some("https://openrouter.ai/api/v1")
        );

        assert_eq!(
            ProviderPreset::Anthropic.base_url(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(
            ProviderPreset::Anthropic.default_model(),
            "claude-sonnet-4-6"
        );

        assert_eq!(
            ProviderPreset::Together.default_model(),
            "meta-llama/Llama-3.3-70B-Instruct-Turbo"
        );
        assert_eq!(ProviderPreset::Ollama.default_model(), "llama3.2");
    }

    #[test]
    fn provider_registry_contains_runtime_and_extension_slots() {
        let entries = provider_registry();
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "openai" && entry.supports_runtime)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "openrouter" && entry.supports_runtime)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "anthropic" && entry.supports_runtime)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "github-copilot" && !entry.supports_runtime)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "vercel-ai-gateway" && !entry.supports_runtime)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "azure-openai" && !entry.supports_runtime)
        );
    }

    #[test]
    fn provider_registry_entry_resolves_known_names() {
        let openai = provider_registry_entry("openai").expect("openai registry entry");
        assert_eq!(openai.auth_mode, LoginAuthMode::OAuth);
        assert_eq!(openai.display_name, "OpenAI (ChatGPT Plus/Pro or API key)");
        assert_eq!(openai.category, ProviderCategory::Runtime);

        let custom = provider_registry_entry("custom").expect("custom registry entry");
        assert_eq!(custom.auth_mode, LoginAuthMode::EndpointAndKey);

        let azure = provider_registry_entry("azure-openai").expect("azure slot entry");
        assert_eq!(azure.auth_mode, LoginAuthMode::EndpointAndKey);
        assert_eq!(azure.endpoint_env, Some("AZURE_OPENAI_ENDPOINT"));

        let copilot = provider_registry_entry_ci("GitHub-COPILOT").expect("copilot slot");
        assert_eq!(copilot.display_name, "GitHub Copilot");
        assert_eq!(copilot.category, ProviderCategory::Extension);
    }

    #[test]
    fn provider_registry_for_picker_has_priority_ordering() {
        let entries = provider_registry_for_picker();
        let top: Vec<&str> = entries.iter().take(7).map(|entry| entry.name).collect();
        assert_eq!(
            top,
            vec![
                "anthropic",
                "github-copilot",
                "openai",
                "google",
                "openrouter",
                "vercel-ai-gateway",
                "azure-openai"
            ]
        );
    }

    #[test]
    fn effective_model_falls_back_to_preset_default() {
        let mut cfg = AgentConfig::default();
        assert_eq!(cfg.effective_model(), "gpt-4o"); // openai preset default

        cfg.provider = ProviderPreset::Together;
        assert_eq!(
            cfg.effective_model(),
            "meta-llama/Llama-3.3-70B-Instruct-Turbo"
        );

        cfg.model = "mistral-7b".to_string();
        assert_eq!(cfg.effective_model(), "mistral-7b"); // explicit override
    }

    #[test]
    fn effective_base_url_preset_vs_explicit() {
        let mut cfg = AgentConfig::default();
        assert_eq!(cfg.effective_base_url(), None); // openai uses SDK default

        cfg.provider = ProviderPreset::Ollama;
        assert_eq!(cfg.effective_base_url(), Some("http://localhost:11434/v1"));

        cfg.base_url = Some("http://custom:11434/v1".to_string());
        assert_eq!(cfg.effective_base_url(), Some("http://custom:11434/v1")); // explicit wins
    }

    #[test]
    fn load_reads_explicit_file_path() {
        with_locked_env(|| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let config_path = tmp.path().join("config.toml");
            write_file(
                &config_path,
                r#"
[gateway]
port = 5555
report_host = "host.docker.internal"
tls_cert = "inline"

[agent]
provider = "openai"
model = "gpt-4.1-mini"
api_key = "from_file"
max_tool_rounds = 7
base_url = "https://example.com/v1"

[channels.telegram]
enabled = true
token = "tg-token"

[channels.cli]
enabled = false

[database]
url = "/tmp/openpista-test.db"

[skills]
workspace = "/tmp/workspace"
"#,
            );
            let cfg = Config::load(Some(&config_path)).expect("config should parse");
            assert_eq!(cfg.gateway.port, 5555);
            assert_eq!(
                cfg.gateway.report_host.as_deref(),
                Some("host.docker.internal")
            );
            assert_eq!(cfg.agent.provider, ProviderPreset::OpenAi);
            assert_eq!(cfg.agent.model, "gpt-4.1-mini");
            assert_eq!(cfg.agent.effective_model(), "gpt-4.1-mini");
            assert_eq!(cfg.agent.api_key, "from_file");
            assert_eq!(cfg.agent.max_tool_rounds, 7);
            assert_eq!(
                cfg.agent.effective_base_url(),
                Some("https://example.com/v1")
            );
            assert!(cfg.channels.telegram.enabled);
            assert_eq!(cfg.channels.telegram.token, "tg-token");
            assert!(!cfg.channels.cli.enabled);
            assert_eq!(cfg.database.url, "/tmp/openpista-test.db");
            assert_eq!(cfg.skills.workspace, "/tmp/workspace");
        });
    }

    #[test]
    fn load_together_preset_auto_configures_url() {
        with_locked_env(|| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let config_path = tmp.path().join("config.toml");
            write_file(
                &config_path,
                r#"
[agent]
provider = "together"
api_key = "tg-key"
"#,
            );
            let cfg = Config::load(Some(&config_path)).expect("config should parse");
            assert_eq!(cfg.agent.provider, ProviderPreset::Together);
            assert_eq!(
                cfg.agent.effective_model(),
                "meta-llama/Llama-3.3-70B-Instruct-Turbo"
            );
            assert_eq!(
                cfg.agent.effective_base_url(),
                Some("https://api.together.xyz/v1")
            );
        });
    }

    #[test]
    fn load_returns_toml_error_for_invalid_content() {
        with_locked_env(|| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let config_path = tmp.path().join("config.toml");
            write_file(&config_path, "[agent\nmodel = \"broken\"");
            let err = Config::load(Some(&config_path)).expect_err("invalid toml must fail");
            assert!(err.to_string().contains("TOML parse error"));
        });
    }

    #[test]
    fn resolve_api_key_prefers_config_key() {
        with_locked_env(|| {
            let mut cfg = Config::default();
            cfg.agent.api_key = "abc123".to_string();
            assert_eq!(cfg.resolve_api_key(), "abc123");
        });
    }

    #[test]
    fn provider_preset_from_str_and_metadata_are_stable() {
        assert_eq!(
            "openai".parse::<ProviderPreset>().ok(),
            Some(ProviderPreset::OpenAi)
        );
        assert_eq!(
            "openrouter".parse::<ProviderPreset>().ok(),
            Some(ProviderPreset::OpenRouter)
        );
        assert!("opencode".parse::<ProviderPreset>().is_err());
        assert!("unknown".parse::<ProviderPreset>().is_err());

        assert_eq!(ProviderPreset::OpenAi.api_key_env(), "OPENAI_API_KEY");
        assert_eq!(ProviderPreset::OpenAi.name(), "openai");
        assert_eq!(ProviderPreset::Ollama.api_key_env(), "");
        assert_eq!(ProviderPreset::Ollama.name(), "ollama");
    }

    #[test]
    fn load_applies_env_overrides_for_agent_and_channels() {
        with_locked_env(|| {
            set_env_var("openpista_API_KEY", "env-api");
            set_env_var("openpista_MODEL", "env-model");
            set_env_var("TELEGRAM_BOT_TOKEN", "env-tg-token");
            set_env_var("openpista_OAUTH_CLIENT_ID", "env-client-id");
            set_env_var("openpista_MOBILE_TOKEN", "env-mobile-token");
            set_env_var("openpista_WORKSPACE", "/tmp/env-workspace");

            let cfg = Config::load(None).expect("config load");
            assert_eq!(cfg.agent.api_key, "env-api");
            assert_eq!(cfg.agent.model, "env-model");
            assert_eq!(cfg.agent.oauth_client_id, "env-client-id");
            assert!(cfg.channels.telegram.enabled);
            assert_eq!(cfg.channels.telegram.token, "env-tg-token");
            assert!(cfg.channels.mobile.enabled);
            assert_eq!(cfg.channels.mobile.api_token, "env-mobile-token");
            assert_eq!(cfg.skills.workspace, "/tmp/env-workspace");

            remove_env_var("openpista_API_KEY");
            remove_env_var("openpista_MODEL");
            remove_env_var("TELEGRAM_BOT_TOKEN");
            remove_env_var("openpista_OAUTH_CLIENT_ID");
            remove_env_var("openpista_MOBILE_TOKEN");
            remove_env_var("openpista_WORKSPACE");
        });
    }

    #[test]
    fn resolve_api_key_uses_provider_specific_env_then_legacy_fallback() {
        with_locked_env(|| {
            remove_env_var("openpista_API_KEY");
            remove_env_var("TOGETHER_API_KEY");
            remove_env_var("OPENAI_API_KEY");

            let mut cfg = Config::default();
            cfg.agent.api_key.clear();
            cfg.agent.provider = ProviderPreset::Together;

            set_env_var("TOGETHER_API_KEY", "provider-key");
            assert_eq!(cfg.resolve_api_key(), "provider-key");

            remove_env_var("TOGETHER_API_KEY");
            set_env_var("OPENAI_API_KEY", "legacy-key");
            assert_eq!(cfg.resolve_api_key(), "legacy-key");

            remove_env_var("OPENAI_API_KEY");
        });
    }

    #[test]
    fn anthropic_preset_oauth_endpoints_are_configured() {
        let ep = ProviderPreset::Anthropic
            .oauth_endpoints()
            .expect("anthropic oauth endpoints");
        assert!(ep.auth_url.contains("claude.ai"));
        assert!(
            ep.token_url.contains("platform.claude.com") || ep.token_url.contains("anthropic.com")
        );
        assert!(ep.default_client_id.is_some());
        assert!(ep.default_callback_port.is_none());
        assert!(!ep.redirect_path.is_empty());
    }

    #[test]
    fn extension_oauth_endpoints_returns_none_for_unknown() {
        assert!(extension_oauth_endpoints("unknown-provider").is_none());
    }

    #[test]
    fn oauth_available_for_anthropic_with_default_client_id() {
        assert!(oauth_available_for("anthropic", ""));
    }

    #[test]
    fn anthropic_registry_entry_has_oauth_auth_mode_and_runtime_category() {
        let entry = provider_registry_entry("anthropic").expect("anthropic entry");
        assert_eq!(entry.auth_mode, LoginAuthMode::OAuth);
        assert_eq!(entry.category, ProviderCategory::Runtime);
        assert!(entry.supports_runtime);
    }
}
