use proto::ConfigError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// OAuth 2.0 PKCE application endpoints for a provider.
pub struct OAuthEndpoints {
    /// Authorization endpoint (browser redirect target).
    pub auth_url: &'static str,
    /// Token exchange endpoint (server-side POST).
    pub token_url: &'static str,
    /// Space-separated OAuth scopes to request.
    pub scope: &'static str,
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
    GlueGoogle,
    GlueGpt,
    /// Together.ai – OpenAI-compatible endpoint; base_url auto-set.
    Together,
    /// Local Ollama instance – OpenAI-compatible; base_url auto-set, no API key needed.
    Ollama,
    /// OpenRouter – aggregates many providers; base_url auto-set.
    OpenRouter,
    /// OpenCode Zen endpoint – OpenAI-compatible; base_url auto-set.
    OpenCode,
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
        name: "anthropic",
        display_name: "Anthropic",
        category: ProviderCategory::Extension,
        sort_order: 20,
        search_aliases: &["claude", "anthropic"],
        auth_mode: LoginAuthMode::ApiKey,
        api_key_env: "ANTHROPIC_API_KEY",
        endpoint_env: None,
        supports_runtime: false,
    },
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
        api_key_env: "AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY",
        endpoint_env: Some("AWS_REGION"),
        supports_runtime: false,
    },
];

impl ProviderPreset {
    /// Returns all currently supported runtime provider presets.
    pub const fn all() -> &'static [Self] {
        &[
            Self::OpenAi,
            Self::GlueGoogle,
            Self::GlueGpt,
            Self::Together,
            Self::Ollama,
            Self::OpenRouter,
            Self::OpenCode,
            Self::Custom,
        ]
    }

    /// Default model ID for the preset. Used when `AgentConfig::model` is empty.
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-4o",
            Self::GlueGoogle => "gemini-3.1-pro-preview",
            Self::GlueGpt => "gpt-5.3-codex",
            Self::Together => "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            Self::Ollama => "llama3.2",
            Self::OpenRouter => "openai/gpt-4o",
            Self::OpenCode => "gpt-5-codex",
            Self::Custom => "",
        }
    }

    /// Auto-configured API base URL (`None` = use the SDK's built-in default).
    pub fn base_url(&self) -> Option<&'static str> {
        match self {
            Self::OpenAi => None,
            Self::GlueGoogle => None,
            Self::GlueGpt => None,
            Self::Together => Some("https://api.together.xyz/v1"),
            Self::Ollama => Some("http://localhost:11434/v1"),
            Self::OpenRouter => Some("https://openrouter.ai/api/v1"),
            Self::OpenCode => Some("https://opencode.ai/zen/v1"),
            Self::Custom => None,
        }
    }

    /// Name of the provider-specific API key environment variable.
    /// Empty string means no API key is required (e.g. Ollama).
    pub fn api_key_env(&self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI_API_KEY",
            Self::GlueGoogle => "GOOGLE_API_KEY",
            Self::GlueGpt => "OPENAI_API_KEY",
            Self::Together => "TOGETHER_API_KEY",
            Self::Ollama => "",
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::OpenCode => "OPENCODE_API_KEY",
            Self::Custom => "OPENAI_API_KEY",
        }
    }

    /// Canonical lowercase name used as the credential-store key.
    pub fn name(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::GlueGoogle => "glue-google",
            Self::GlueGpt => "glue-gpt",
            Self::Together => "together",
            Self::Ollama => "ollama",
            Self::OpenRouter => "openrouter",
            Self::OpenCode => "opencode",
            Self::Custom => "custom",
        }
    }

    /// Returns OAuth 2.0 PKCE endpoints for providers that support browser login.
    /// Returns `None` for providers without a supported OAuth flow
    /// (Together.ai and Ollama use API keys only).
    pub fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        match self {
            Self::OpenAi => Some(OAuthEndpoints {
                auth_url: "https://auth.openai.com/authorize",
                token_url: "https://auth.openai.com/oauth/token",
                scope: "openid email profile",
            }),
            Self::GlueGoogle => None,
            Self::OpenRouter => Some(OAuthEndpoints {
                auth_url: "https://openrouter.ai/auth",
                token_url: "https://openrouter.ai/api/v1/auth/keys",
                scope: "",
            }),
            Self::OpenCode => None,
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
            Some("OPENPISTACRAB_BASE_URL")
        } else {
            None
        };

        ProviderRegistryEntry {
            name: self.name(),
            display_name: match self {
                Self::OpenCode => "OpenCode Zen",
                Self::OpenAi => "OpenAI (ChatGPT Plus/Pro or API key)",
                Self::OpenRouter => "OpenRouter",
                Self::Together => "Together",
                Self::Ollama => "Ollama",
                Self::GlueGoogle => "Glue Google",
                Self::GlueGpt => "Glue GPT",
                Self::Custom => "Custom OpenAI-Compatible",
            },
            category: ProviderCategory::Runtime,
            sort_order: match self {
                Self::OpenCode => 10,
                Self::OpenAi => 40,
                Self::OpenRouter => 60,
                Self::Together => 110,
                Self::GlueGoogle => 120,
                Self::GlueGpt => 130,
                Self::Ollama => 140,
                Self::Custom => 150,
            },
            search_aliases: match self {
                Self::OpenCode => &["zen", "opencode", "coding"],
                Self::OpenAi => &["openai", "chatgpt", "gpt"],
                Self::OpenRouter => &["router", "openrouter"],
                Self::Together => &["together", "llama", "mixtral"],
                Self::GlueGoogle => &["google", "gemini", "glue"],
                Self::GlueGpt => &["gpt", "glue"],
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

/// Returns the merged provider registry (runtime providers + extension slots).
pub fn provider_registry() -> Vec<ProviderRegistryEntry> {
    let mut entries: Vec<ProviderRegistryEntry> = ProviderPreset::all()
        .iter()
        .map(ProviderPreset::registry_entry)
        .collect();
    entries.extend_from_slice(EXTENSION_PROVIDER_SLOTS);
    entries
}

/// Resolves one provider entry by id (case-insensitive).
pub fn provider_registry_entry(name: &str) -> Option<ProviderRegistryEntry> {
    provider_registry_entry_ci(name)
}

/// Resolves one provider entry by id (case-insensitive).
pub fn provider_registry_entry_ci(name: &str) -> Option<ProviderRegistryEntry> {
    let needle = name.trim().to_ascii_lowercase();
    provider_registry()
        .into_iter()
        .find(|entry| entry.name == needle)
}

/// Returns picker-ordered provider entries.
pub fn provider_registry_for_picker() -> Vec<ProviderRegistryEntry> {
    let mut entries = provider_registry();
    entries.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.display_name.cmp(b.display_name))
    });
    entries
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
            "glue-google" => Ok(Self::GlueGoogle),
            "glue-gpt" => Ok(Self::GlueGpt),
            "together" => Ok(Self::Together),
            "ollama" => Ok(Self::Ollama),
            "openrouter" => Ok(Self::OpenRouter),
            "opencode" => Ok(Self::OpenCode),
            "custom" => Ok(Self::Custom),
            other => Err(format!("unknown provider '{other}'")),
        }
    }
}

/// Agent model/provider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Provider preset: openai | together | ollama | openrouter | opencode | custom.
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
    /// OAuth 2.0 client ID for `openpistacrab auth login`.
    /// Must be registered with the provider. Also read from
    /// `OPENPISTACRAB_OAUTH_CLIENT_ID` environment variable.
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
            url: format!("{home}/.openpistacrab/memory.db"),
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
            workspace: format!("{home}/.openpistacrab/workspace"),
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
            let home_config = PathBuf::from(home)
                .join(".openpistacrab")
                .join("config.toml");
            if home_config.exists() {
                return Some(home_config);
            }
            None
        });

        let mut config = if let Some(path) = config_path {
            let content = std::fs::read_to_string(&path).map_err(ConfigError::Io)?;
            toml::from_str(&content).map_err(|e| ConfigError::Toml(e.to_string()))?
        } else {
            Config::default()
        };

        // Environment variable overrides (highest priority → lowest)
        if let Ok(key) = std::env::var("OPENPISTACRAB_API_KEY") {
            config.agent.api_key = key;
        }
        if let Ok(model) = std::env::var("OPENPISTACRAB_MODEL") {
            config.agent.model = model;
        }
        if let Ok(token) = std::env::var("TELEGRAM_BOT_TOKEN") {
            config.channels.telegram.token = token;
            config.channels.telegram.enabled = true;
        }
        if let Ok(client_id) = std::env::var("OPENPISTACRAB_OAUTH_CLIENT_ID") {
            config.agent.oauth_client_id = client_id;
        }
        if let Ok(token) = std::env::var("OPENPISTACRAB_MOBILE_TOKEN") {
            config.channels.mobile.api_token = token;
            config.channels.mobile.enabled = true;
        }
        if let Ok(workspace) = std::env::var("OPENPISTACRAB_WORKSPACE") {
            config.skills.workspace = workspace;
        }

        Ok(config)
    }

    /// Resolves the API key to use for the configured provider.
    ///
    /// Priority:
    /// 1. `agent.api_key` in config file (or `OPENPISTACRAB_API_KEY` applied at load time)
    /// 2. Valid (non-expired) token stored by `openpistacrab auth login`
    /// 3. Provider-specific environment variable (e.g. `TOGETHER_API_KEY`)
    /// 4. `OPENAI_API_KEY` (legacy fallback)
    pub fn resolve_api_key(&self) -> String {
        if !self.agent.api_key.is_empty() {
            return self.agent.api_key.clone();
        }

        // Credential store written by `auth login`
        let creds = crate::auth::Credentials::load();
        if let Some(cred) = creds.get(self.agent.provider.name())
            && !cred.is_expired()
        {
            return cred.access_token.clone();
        }

        // Provider-specific env var
        let env_var = self.agent.provider.api_key_env();
        if !env_var.is_empty()
            && let Ok(key) = std::env::var(env_var)
        {
            return key;
        }

        // Legacy fallback
        std::env::var("OPENAI_API_KEY").unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn set_env_var(key: &str, value: &str) {
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_env_var(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }

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
            ProviderPreset::OpenCode.base_url(),
            Some("https://opencode.ai/zen/v1")
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
                .any(|entry| entry.name == "opencode" && entry.supports_runtime)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "anthropic" && !entry.supports_runtime)
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
                "opencode",
                "anthropic",
                "github-copilot",
                "openai",
                "google",
                "openrouter",
                "vercel-ai-gateway"
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
        let _guard = env_lock().lock().expect("env lock");

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
url = "/tmp/openpistacrab-test.db"

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
        assert_eq!(cfg.database.url, "/tmp/openpistacrab-test.db");
        assert_eq!(cfg.skills.workspace, "/tmp/workspace");
    }

    #[test]
    fn load_together_preset_auto_configures_url() {
        let _guard = env_lock().lock().expect("env lock");

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
    }

    #[test]
    fn load_returns_toml_error_for_invalid_content() {
        let _guard = env_lock().lock().expect("env lock");

        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join("config.toml");
        write_file(&config_path, "[agent\nmodel = \"broken\"");
        let err = Config::load(Some(&config_path)).expect_err("invalid toml must fail");
        assert!(err.to_string().contains("TOML parse error"));
    }

    #[test]
    fn resolve_api_key_prefers_config_key() {
        let _guard = env_lock().lock().expect("env lock");

        let mut cfg = Config::default();
        cfg.agent.api_key = "abc123".to_string();
        assert_eq!(cfg.resolve_api_key(), "abc123");
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
        assert_eq!(
            "opencode".parse::<ProviderPreset>().ok(),
            Some(ProviderPreset::OpenCode)
        );
        assert!("unknown".parse::<ProviderPreset>().is_err());

        assert_eq!(ProviderPreset::OpenAi.api_key_env(), "OPENAI_API_KEY");
        assert_eq!(ProviderPreset::OpenAi.name(), "openai");
        assert_eq!(ProviderPreset::OpenCode.api_key_env(), "OPENCODE_API_KEY");
        assert_eq!(ProviderPreset::OpenCode.name(), "opencode");
        assert_eq!(ProviderPreset::Ollama.api_key_env(), "");
        assert_eq!(ProviderPreset::Ollama.name(), "ollama");
    }

    #[test]
    fn load_applies_env_overrides_for_agent_and_channels() {
        let _guard = env_lock().lock().expect("env lock");

        set_env_var("OPENPISTACRAB_API_KEY", "env-api");
        set_env_var("OPENPISTACRAB_MODEL", "env-model");
        set_env_var("TELEGRAM_BOT_TOKEN", "env-tg-token");
        set_env_var("OPENPISTACRAB_OAUTH_CLIENT_ID", "env-client-id");
        set_env_var("OPENPISTACRAB_MOBILE_TOKEN", "env-mobile-token");
        set_env_var("OPENPISTACRAB_WORKSPACE", "/tmp/env-workspace");

        let cfg = Config::load(None).expect("config load");
        assert_eq!(cfg.agent.api_key, "env-api");
        assert_eq!(cfg.agent.model, "env-model");
        assert_eq!(cfg.agent.oauth_client_id, "env-client-id");
        assert!(cfg.channels.telegram.enabled);
        assert_eq!(cfg.channels.telegram.token, "env-tg-token");
        assert!(cfg.channels.mobile.enabled);
        assert_eq!(cfg.channels.mobile.api_token, "env-mobile-token");
        assert_eq!(cfg.skills.workspace, "/tmp/env-workspace");

        remove_env_var("OPENPISTACRAB_API_KEY");
        remove_env_var("OPENPISTACRAB_MODEL");
        remove_env_var("TELEGRAM_BOT_TOKEN");
        remove_env_var("OPENPISTACRAB_OAUTH_CLIENT_ID");
        remove_env_var("OPENPISTACRAB_MOBILE_TOKEN");
        remove_env_var("OPENPISTACRAB_WORKSPACE");
    }

    #[test]
    fn resolve_api_key_uses_provider_specific_env_then_legacy_fallback() {
        let _guard = env_lock().lock().expect("env lock");

        remove_env_var("OPENPISTACRAB_API_KEY");
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
    }

    #[test]
    fn resolve_api_key_uses_opencode_env() {
        let _guard = env_lock().lock().expect("env lock");

        remove_env_var("OPENPISTACRAB_API_KEY");
        remove_env_var("OPENCODE_API_KEY");
        remove_env_var("OPENAI_API_KEY");

        let mut cfg = Config::default();
        cfg.agent.api_key.clear();
        cfg.agent.provider = ProviderPreset::OpenCode;

        set_env_var("OPENCODE_API_KEY", "opencode-key");
        assert_eq!(cfg.resolve_api_key(), "opencode-key");

        remove_env_var("OPENCODE_API_KEY");
    }
}
