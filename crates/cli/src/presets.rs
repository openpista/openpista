use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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

impl LoginAuthMode {
    /// Returns the canonical string representation used by web clients.
    #[cfg_attr(test, allow(dead_code))]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::ApiKey => "api_key",
            Self::EndpointAndKey => "endpoint_and_key",
            Self::None => "none",
        }
    }
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
        auth_mode: LoginAuthMode::OAuth,
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
    match provider_name {
        "github-copilot" => Some(OAuthEndpoints {
            auth_url: "https://github.com/login/oauth/authorize",
            token_url: "https://github.com/login/oauth/access_token",
            scope: "read:user",
            default_client_id: Some("Iv1.b507a08c87ecfe98"),
            default_callback_port: Some(1456),
            redirect_path: "/callback",
            redirect_base: None,
        }),
        _ => None,
    }
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
