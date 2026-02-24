use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tracing::debug;

/// Time-to-live for the on-disk model cache (24 hours).
const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// Stability status of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelStatus {
    /// Generally available and production-ready.
    Stable,
    /// Early-access or beta model.
    Preview,
    /// Status not determined.
    Unknown,
}

impl ModelStatus {
    #[cfg_attr(test, allow(dead_code))]
    /// Returns the lowercase string representation of this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
            Self::Unknown => "unknown",
        }
    }
}

/// Where the model entry originated from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    /// Curated from documentation.
    Docs,
    /// Discovered via remote API.
    Api,
}

impl ModelSource {
    #[cfg_attr(test, allow(dead_code))]
    /// Returns the lowercase string representation of this status.
    /// Returns the lowercase string representation of this source.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Docs => "docs",
            Self::Api => "api",
        }
    }
}

/// A single model entry in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    /// Unique model identifier (e.g. `gpt-4o`, `claude-sonnet-4-6`).
    pub id: String,
    /// Provider that serves this model.
    #[serde(default)]
    pub provider: String,
    /// Whether this model is recommended for coding tasks.
    pub recommended_for_coding: bool,
    /// Stability status of the model.
    pub status: ModelStatus,
    /// Whether the entry came from docs or the remote API.
    pub source: ModelSource,
    /// Whether the model is currently accessible.
    pub available: bool,
}

/// Result of loading a single provider's model catalog.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CatalogLoadResult {
    /// Provider name.
    pub provider: String,
    /// Loaded catalog entries.
    pub entries: Vec<ModelCatalogEntry>,
    /// Human-readable sync status message.
    pub sync_status: String,
}

/// Result of loading model catalogs from multiple providers.
#[derive(Debug, Clone)]
pub struct MultiCatalogLoadResult {
    /// Merged catalog entries across all providers.
    pub entries: Vec<ModelCatalogEntry>,
    /// Per-provider sync status messages.
    pub sync_statuses: Vec<String>,
}

/// Model entries grouped into display sections.
#[derive(Debug, Clone, Default)]
pub struct ModelSections {
    /// Recommended models that are currently available.
    pub recommended_available: Vec<ModelCatalogEntry>,
    /// Recommended models that are not currently available.
    pub recommended_unavailable: Vec<ModelCatalogEntry>,
    /// Non-recommended models that are available.
    pub other_available: Vec<ModelCatalogEntry>,
}

/// Summary counts of a filtered model catalog query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSummary {
    /// Total models before filtering.
    pub total: usize,
    /// Models matching the query.
    pub matched: usize,
    /// Matched models recommended for coding.
    pub recommended: usize,
    /// Matched models that are available.
    pub available: usize,
}

/// Serializable on-disk cache for a provider's model catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedCatalog {
    /// UTC timestamp when this cache snapshot was fetched.
    fetched_at: DateTime<Utc>,
    /// Cached model entries.
    entries: Vec<ModelCatalogEntry>,
}

/// Wire format for the `/v1/models` JSON response.
#[derive(Debug, Deserialize)]
struct ZenModelsResponse {
    /// List of model objects returned by the API.
    data: Vec<ZenModel>,
}

/// A single model object in the API response.
#[derive(Debug, Deserialize)]
struct ZenModel {
    /// Model identifier string.
    id: String,
}

/// Returns hardcoded seed models for a known provider.
pub fn seed_models_for_provider(provider: &str) -> Vec<ModelCatalogEntry> {
    let p = provider.to_string();
    match provider {
        "anthropic" => vec![
            ModelCatalogEntry {
                id: "claude-sonnet-4-6".to_string(),
                provider: p.clone(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "claude-opus-4-6".to_string(),
                provider: p.clone(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "claude-haiku-4-5".to_string(),
                provider: p,
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ],
        "openai" => vec![
            ModelCatalogEntry {
                id: "gpt-5.3-codex".to_string(),
                provider: p.clone(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-5.3-codex-spark".to_string(),
                provider: p.clone(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "codex-mini-latest".to_string(),
                provider: p.clone(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "o3".to_string(),
                provider: p.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "o3-mini".to_string(),
                provider: p.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "o4-mini".to_string(),
                provider: p.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-4.1".to_string(),
                provider: p.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-4.1-mini".to_string(),
                provider: p.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-4.1-nano".to_string(),
                provider: p,
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ],
        "together" => vec![ModelCatalogEntry {
            id: "meta-llama/Llama-3.3-70B-Instruct-Turbo".to_string(),
            provider: p,
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }],
        "openrouter" => vec![ModelCatalogEntry {
            id: "openai/gpt-4o".to_string(),
            provider: p,
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }],
        "github-copilot" => vec![
            ModelCatalogEntry {
                id: "gpt-4o".to_string(),
                provider: p.clone(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-4o-mini".to_string(),
                provider: p.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "claude-3.5-sonnet".to_string(),
                provider: p,
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ],
        _ => vec![],
    }
}

/// Groups filtered catalog entries into display sections.
pub fn model_sections(entries: &[ModelCatalogEntry], query: &str, show_all: bool) -> ModelSections {
    let filtered = filtered_entries(entries, query, show_all);
    let mut sections = ModelSections::default();

    for entry in filtered {
        if entry.recommended_for_coding && entry.available {
            sections.recommended_available.push(entry);
        } else if entry.recommended_for_coding && !entry.available {
            sections.recommended_unavailable.push(entry);
        } else if show_all && entry.available {
            sections.other_available.push(entry);
        }
    }

    sections
}

/// Computes summary counts for a filtered catalog query.
pub fn model_summary(entries: &[ModelCatalogEntry], query: &str, show_all: bool) -> ModelSummary {
    let filtered = filtered_entries(entries, query, show_all);
    ModelSummary {
        total: entries.len(),
        matched: filtered.len(),
        recommended: filtered
            .iter()
            .filter(|entry| entry.recommended_for_coding)
            .count(),
        available: filtered.iter().filter(|entry| entry.available).count(),
    }
}

/// Filters and sorts catalog entries by query and recommendation flag.
pub fn filtered_entries(
    entries: &[ModelCatalogEntry],
    query: &str,
    show_all: bool,
) -> Vec<ModelCatalogEntry> {
    let mut result: Vec<ModelCatalogEntry> = entries
        .iter()
        .filter(|entry| show_all || entry.recommended_for_coding)
        .filter(|entry| matches_query(&entry.id, query))
        .cloned()
        .collect();

    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

/// Merges hardcoded seed entries with remotely-discovered model ids.
pub fn merge_seed_with_remote(
    seed: &[ModelCatalogEntry],
    remote_ids: &[String],
) -> Vec<ModelCatalogEntry> {
    let remote_set: BTreeSet<String> = remote_ids.iter().cloned().collect();
    let default_provider = seed.first().map(|e| e.provider.clone()).unwrap_or_default();
    let mut by_id: BTreeMap<String, ModelCatalogEntry> = seed
        .iter()
        .cloned()
        .map(|entry| (entry.id.clone(), entry))
        .collect();

    for remote_id in &remote_set {
        if by_id.contains_key(remote_id) {
            continue;
        }

        by_id.insert(
            remote_id.to_string(),
            ModelCatalogEntry {
                id: remote_id.to_string(),
                provider: default_provider.clone(),
                recommended_for_coding: false,
                status: ModelStatus::Unknown,
                source: ModelSource::Api,
                available: true,
            },
        );
    }

    for entry in by_id.values_mut() {
        match entry.source {
            // Docs-sourced model are manually curated and always available.
            // The remote API is used to discover additional model, not to gate known ones.
            ModelSource::Docs => {}
            ModelSource::Api => {
                entry.available = true;
            }
        }
    }

    by_id.into_values().collect()
}

/// Resolves the models-list URL for a given provider and optional base URL.
fn models_url(provider: &str, base_url: Option<&str>) -> String {
    match (provider, base_url) {
        ("anthropic", _) => "https://api.anthropic.com/v1/models".to_string(),
        (_, Some(url)) => {
            let trimmed = url.trim_end_matches('/');
            format!("{trimmed}/models")
        }
        (_, None) => "https://api.openai.com/v1/models".to_string(),
    }
}

/// Returns the on-disk cache file path for a specific provider.
fn provider_cache_path(provider_name: &str) -> PathBuf {
    if let Ok(path) = std::env::var("openpista_MODELS_CACHE_PATH") {
        return PathBuf::from(path);
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".openpista")
        .join("cache")
        .join("models")
        .join(format!("{provider_name}.json"))
}

/// Fetches model IDs from a remote OpenAI-compatible `/v1/models` endpoint.
async fn fetch_remote_model_ids_from(url: &str, api_key: &str) -> Result<Vec<String>, String> {
    debug!(url = %url, has_key = %!api_key.is_empty(), "Fetching models");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|err| format!("build client: {err}"))?;

    let mut req = client.get(url);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let response = req
        .send()
        .await
        .map_err(|err| format!("request to {url} failed: {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("read body from {url}: {err}"))?;

    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        debug!(url = %url, status = %status.as_u16(), body = %preview, "Models fetch failed");
        return Err(format!(
            "HTTP {} from {}: {}",
            status.as_u16(),
            url,
            preview
        ));
    }

    let parsed: ZenModelsResponse =
        serde_json::from_str(&body).map_err(|err| format!("json decode from {url}: {err}"))?;

    let mut ids: Vec<String> = parsed
        .data
        .into_iter()
        .map(|item| item.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    debug!(url = %url, count = %ids.len(), "Models fetched");
    Ok(ids)
}

/// Fetches model IDs from the Anthropic models API with version-header auth.
async fn fetch_anthropic_model_ids(api_key: &str) -> Result<Vec<String>, String> {
    let url = "https://api.anthropic.com/v1/models?limit=1000";
    debug!(url = %url, "Fetching Anthropic models");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|err| format!("build client: {err}"))?;

    let mut req_builder = client.get(url).header("anthropic-version", "2023-06-01");

    if proto::is_anthropic_oauth_token(api_key) {
        req_builder = req_builder.bearer_auth(api_key);
    } else {
        req_builder = req_builder.header("x-api-key", api_key);
    }

    let response = req_builder
        .send()
        .await
        .map_err(|err| format!("request to {url} failed: {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("read body from {url}: {err}"))?;

    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        debug!(url = %url, status = %status.as_u16(), body = %preview, "Anthropic models fetch failed");
        return Err(format!(
            "HTTP {} from {}: {}",
            status.as_u16(),
            url,
            preview
        ));
    }

    let parsed: ZenModelsResponse =
        serde_json::from_str(&body).map_err(|err| format!("json decode from {url}: {err}"))?;

    let mut ids: Vec<String> = parsed
        .data
        .into_iter()
        .map(|item| item.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    debug!(url = %url, count = %ids.len(), "Anthropic models fetched");
    Ok(ids)
}
/// Backfill empty `provider` fields on cached entries that were serialized before
/// the provider field was added to `ModelCatalogEntry`.
fn backfill_provider(
    mut entries: Vec<ModelCatalogEntry>,
    provider: &str,
) -> Vec<ModelCatalogEntry> {
    for entry in &mut entries {
        if entry.provider.is_empty() {
            entry.provider = provider.to_string();
        }
    }
    entries
}

/// Loads a model catalog for one provider, with cache and remote fallback.
pub async fn load_catalog(
    provider_name: &str,
    base_url: Option<&str>,
    api_key: &str,
    refresh: bool,
) -> CatalogLoadResult {
    debug!(provider = %provider_name, refresh = %refresh, "Loading model catalog");
    if api_key.is_empty() {
        debug!(provider = %provider_name, "API key is empty; fetch will likely fail");
    }
    let cache_path = provider_cache_path(provider_name);

    if !refresh && let Some(cached) = load_cache_if_fresh(&cache_path) {
        debug!(provider = %provider_name, entries = %cached.entries.len(), "Using cached catalog");
        return CatalogLoadResult {
            provider: provider_name.to_string(),
            entries: backfill_provider(cached.entries, provider_name),
            sync_status: format!(
                "Using cache (fetched_at={})",
                cached.fetched_at.format("%Y-%m-%d %H:%M UTC")
            ),
        };
    }

    let seed = seed_models_for_provider(provider_name);
    let fetch_result = if provider_name == "anthropic" {
        fetch_anthropic_model_ids(api_key).await
    } else {
        let url = models_url(provider_name, base_url);
        fetch_remote_model_ids_from(&url, api_key).await
    };
    match fetch_result {
        Ok(ids) => {
            let entries = merge_seed_with_remote(&seed, &ids);
            debug!(provider = %provider_name, remote = %ids.len(), merged = %entries.len(), "Catalog synced from remote");
            let now = Utc::now();
            let cached = CachedCatalog {
                fetched_at: now,
                entries: entries.clone(),
            };
            let _ = save_cache(&cache_path, &cached);

            CatalogLoadResult {
                provider: provider_name.to_string(),
                entries,
                sync_status: format!(
                    "Synced from {} ({} models, fetched_at={})",
                    provider_name,
                    ids.len(),
                    now.format("%Y-%m-%d %H:%M UTC")
                ),
            }
        }
        Err(err) => {
            debug!(provider = %provider_name, error = %err, "Catalog fetch failed, using fallback");
            if let Some(cached) = load_cache(&cache_path) {
                CatalogLoadResult {
                    provider: provider_name.to_string(),
                    entries: backfill_provider(cached.entries, provider_name),
                    sync_status: format!("Fetch failed: {err} — using cache"),
                }
            } else {
                CatalogLoadResult {
                    provider: provider_name.to_string(),
                    entries: seed,
                    sync_status: format!("Fetch failed: {err} — using defaults"),
                }
            }
        }
    }
}

/// Loads model catalogs from multiple providers and merges them into a single list.
pub async fn load_catalog_multi(
    providers: &[(String, Option<String>, String)],
) -> MultiCatalogLoadResult {
    let mut all_entries = Vec::new();
    let mut sync_statuses = Vec::new();

    for (provider_name, base_url, api_key) in providers {
        let result = load_catalog(provider_name, base_url.as_deref(), api_key, false).await;
        sync_statuses.push(format!("{}: {}", provider_name, result.sync_status));
        all_entries.extend(result.entries);
    }

    MultiCatalogLoadResult {
        entries: all_entries,
        sync_statuses,
    }
}

/// Case-insensitive substring match used for model search filtering.
fn matches_query(haystack: &str, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }
    haystack
        .to_ascii_lowercase()
        .contains(&trimmed.to_ascii_lowercase())
}

/// Loads a cached catalog only if it is younger than `CACHE_TTL_SECS`.
fn load_cache_if_fresh(path: &std::path::Path) -> Option<CachedCatalog> {
    let cached = load_cache(path)?;
    let age = Utc::now()
        .signed_duration_since(cached.fetched_at)
        .num_seconds();
    if age <= CACHE_TTL_SECS {
        Some(cached)
    } else {
        None
    }
}

/// Loads a cached catalog from disk regardless of age.
fn load_cache(path: &std::path::Path) -> Option<CachedCatalog> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persists a catalog snapshot to the on-disk JSON cache.
fn save_cache(path: &std::path::Path, cached: &CachedCatalog) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_string_pretty(cached)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    std::fs::write(path, payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn merge_seed_with_remote_marks_availability_and_adds_remote_only() {
        let seed = seed_models_for_provider("openai");
        let merged = merge_seed_with_remote(
            &seed,
            &[
                "gpt-5-codex".to_string(),
                "claude-sonnet-4-6".to_string(),
                "gpt-5.2-codex".to_string(),
            ],
        );

        let by_id: BTreeMap<_, _> = merged
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        // Remote-only models are added and marked available.
        assert!(by_id["gpt-5-codex"].available);
        assert!(by_id["claude-sonnet-4-6"].available);
        // Docs-sourced seed model are always available regardless of remote response.
        assert!(by_id["gpt-4.1"].available);
        // Api-sourced (remote-only) model are also available.
        assert_eq!(by_id["gpt-5.2-codex"].source, ModelSource::Api);
        assert!(by_id["gpt-5.2-codex"].available);
        assert!(!by_id["gpt-5.2-codex"].recommended_for_coding);
    }

    #[test]
    fn filtered_entries_apply_show_all_and_query() {
        let entries = vec![
            ModelCatalogEntry {
                id: "gpt-5-codex".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-5.2".into(),
                provider: String::new(),
                recommended_for_coding: false,
                status: ModelStatus::Unknown,
                source: ModelSource::Api,
                available: true,
            },
        ];

        let only_recommended = filtered_entries(&entries, "", false);
        assert_eq!(only_recommended.len(), 1);

        let matched = filtered_entries(&entries, "5.2", true);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "gpt-5.2");
    }

    #[test]
    fn model_sections_group_correctly() {
        let entries = vec![
            ModelCatalogEntry {
                id: "a".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "b".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: false,
            },
            ModelCatalogEntry {
                id: "c".into(),
                provider: String::new(),
                recommended_for_coding: false,
                status: ModelStatus::Unknown,
                source: ModelSource::Api,
                available: true,
            },
        ];

        let sections = model_sections(&entries, "", true);
        assert_eq!(sections.recommended_available.len(), 1);
        assert_eq!(sections.recommended_unavailable.len(), 1);
        assert_eq!(sections.other_available.len(), 1);
    }

    #[test]
    fn model_summary_counts_match_filtered_set() {
        let entries = vec![
            ModelCatalogEntry {
                id: "gpt-5-codex".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "devstral".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Unknown,
                source: ModelSource::Docs,
                available: false,
            },
        ];

        let summary = model_summary(&entries, "gpt", false);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.matched, 1);
        assert_eq!(summary.recommended, 1);
        assert_eq!(summary.available, 1);
    }

    #[test]
    fn cache_roundtrip_and_ttl_check() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("catalog.json");

        let cached = CachedCatalog {
            fetched_at: Utc::now(),
            entries: seed_models_for_provider("openai"),
        };

        save_cache(&path, &cached).expect("save cache");
        let loaded = load_cache_if_fresh(&path).expect("fresh cache");
        assert_eq!(loaded.entries.len(), cached.entries.len());

        let stale = CachedCatalog {
            fetched_at: Utc::now() - Duration::seconds(CACHE_TTL_SECS + 1),
            entries: seed_models_for_provider("openai"),
        };
        save_cache(&path, &stale).expect("save stale cache");
        assert!(load_cache_if_fresh(&path).is_none());
        assert!(load_cache(&path).is_some());
    }

    #[test]
    fn backfill_provider_fills_empty_entries() {
        let entries = vec![
            ModelCatalogEntry {
                id: "claude-sonnet-4-6".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-4o".into(),
                provider: "openai".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ];
        let result = backfill_provider(entries, "anthropic");
        assert_eq!(result[0].provider, "anthropic");
        // Already-set provider should be preserved
        assert_eq!(result[1].provider, "openai");
    }

    #[test]
    fn backfill_provider_noop_when_all_set() {
        let entries = vec![ModelCatalogEntry {
            id: "gpt-4o".into(),
            provider: "openai".into(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }];
        let result = backfill_provider(entries, "anthropic");
        assert_eq!(result[0].provider, "openai");
    }

    #[test]
    fn seed_models_for_anthropic_provider() {
        let entries = seed_models_for_provider("anthropic");
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.provider == "anthropic"));
        assert!(entries.iter().any(|e| e.id == "claude-sonnet-4-6"));
        assert!(entries.iter().any(|e| e.id == "claude-opus-4-6"));
    }

    #[test]
    fn seed_models_for_together_provider() {
        let entries = seed_models_for_provider("together");
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.provider == "together"));
    }

    #[test]
    fn seed_models_for_unknown_provider_is_empty() {
        let entries = seed_models_for_provider("nonexistent");
        assert!(entries.is_empty());
    }

    #[test]
    fn model_status_as_str_returns_expected_values() {
        assert_eq!(ModelStatus::Stable.as_str(), "stable");
        assert_eq!(ModelStatus::Preview.as_str(), "preview");
        assert_eq!(ModelStatus::Unknown.as_str(), "unknown");
    }

    #[test]
    fn model_source_as_str_returns_expected_values() {
        assert_eq!(ModelSource::Docs.as_str(), "docs");
        assert_eq!(ModelSource::Api.as_str(), "api");
    }

    #[test]
    fn merge_seed_with_remote_preserves_provider_on_new_entries() {
        let seed = seed_models_for_provider("anthropic");
        let merged = merge_seed_with_remote(&seed, &["new-model".to_string()]);
        let new_entry = merged
            .iter()
            .find(|e| e.id == "new-model")
            .expect("new model");
        assert_eq!(new_entry.provider, "anthropic");
        assert_eq!(new_entry.source, ModelSource::Api);
    }

    #[test]
    fn matches_query_handles_edge_cases() {
        assert!(matches_query("gpt-4o", ""));
        assert!(matches_query("gpt-4o", "  "));
        assert!(matches_query("gpt-4o", "GPT"));
        assert!(!matches_query("gpt-4o", "claude"));
    }

    #[test]
    fn oauth_token_detected_by_shared_helper() {
        assert!(proto::is_anthropic_oauth_token("sk-ant-oat01-abc123"));
        assert!(!proto::is_anthropic_oauth_token("sk-ant-api03-abc123"));
        assert!(!proto::is_anthropic_oauth_token(""));
    }

    #[test]
    fn models_url_anthropic_ignores_base_url() {
        assert_eq!(
            models_url("anthropic", Some("https://custom.example.com")),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            models_url("anthropic", None),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn models_url_with_custom_base_url() {
        assert_eq!(
            models_url("openai", Some("https://custom.example.com/v1/")),
            "https://custom.example.com/v1/models"
        );
        assert_eq!(
            models_url("openai", Some("https://custom.example.com/v1")),
            "https://custom.example.com/v1/models"
        );
    }

    #[test]
    fn models_url_defaults_to_openai() {
        assert_eq!(
            models_url("openai", None),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_url("together", None),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn merge_seed_with_empty_remote() {
        let seed = seed_models_for_provider("openai");
        let merged = merge_seed_with_remote(&seed, &[]);
        assert_eq!(merged.len(), seed.len());
        assert!(merged.iter().all(|e| e.available));
    }

    #[test]
    fn merge_seed_with_empty_seed() {
        let merged = merge_seed_with_remote(&[], &["model-a".to_string(), "model-b".to_string()]);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().all(|e| e.provider.is_empty()));
        assert!(merged.iter().all(|e| e.source == ModelSource::Api));
    }

    #[test]
    fn merge_seed_deduplicates_remote_ids() {
        let seed = seed_models_for_provider("openai");
        let merged =
            merge_seed_with_remote(&seed, &["new-model".to_string(), "new-model".to_string()]);
        let new_count = merged.iter().filter(|e| e.id == "new-model").count();
        assert_eq!(new_count, 1);
    }

    #[test]
    fn model_sections_show_all_false_excludes_other() {
        let entries = vec![
            ModelCatalogEntry {
                id: "recommended".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "other".into(),
                provider: String::new(),
                recommended_for_coding: false,
                status: ModelStatus::Unknown,
                source: ModelSource::Api,
                available: true,
            },
        ];
        let sections = model_sections(&entries, "", false);
        assert_eq!(sections.recommended_available.len(), 1);
        assert!(sections.other_available.is_empty());
    }

    #[test]
    fn model_summary_empty_entries() {
        let summary = model_summary(&[], "", true);
        assert_eq!(summary.total, 0);
        assert_eq!(summary.matched, 0);
        assert_eq!(summary.recommended, 0);
        assert_eq!(summary.available, 0);
    }

    #[test]
    fn model_summary_show_all_true_includes_non_recommended() {
        let entries = vec![ModelCatalogEntry {
            id: "a".into(),
            provider: String::new(),
            recommended_for_coding: false,
            status: ModelStatus::Unknown,
            source: ModelSource::Api,
            available: true,
        }];
        let summary = model_summary(&entries, "", true);
        assert_eq!(summary.matched, 1);
        assert_eq!(summary.recommended, 0);
        assert_eq!(summary.available, 1);
    }

    #[test]
    fn filtered_entries_empty_input() {
        let result = filtered_entries(&[], "anything", true);
        assert!(result.is_empty());
    }

    #[test]
    fn filtered_entries_sorts_by_id() {
        let entries = vec![
            ModelCatalogEntry {
                id: "z-model".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "a-model".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ];
        let result = filtered_entries(&entries, "", false);
        assert_eq!(result[0].id, "a-model");
        assert_eq!(result[1].id, "z-model");
    }

    #[test]
    fn provider_cache_path_uses_env_override() {
        let original = std::env::var("openpista_MODELS_CACHE_PATH").ok();
        unsafe {
            std::env::set_var("openpista_MODELS_CACHE_PATH", "/tmp/test-cache.json");
        }
        let path = provider_cache_path("openai");
        assert_eq!(path, PathBuf::from("/tmp/test-cache.json"));
        unsafe {
            match original {
                Some(val) => std::env::set_var("openpista_MODELS_CACHE_PATH", val),
                None => std::env::remove_var("openpista_MODELS_CACHE_PATH"),
            }
        }
    }

    #[test]
    fn provider_cache_path_default_uses_home() {
        let original_cache = std::env::var("openpista_MODELS_CACHE_PATH").ok();
        unsafe {
            std::env::remove_var("openpista_MODELS_CACHE_PATH");
        }
        let path = provider_cache_path("anthropic");
        assert!(path.to_string_lossy().contains("anthropic.json"));
        assert!(path.to_string_lossy().contains("models"));
        if let Some(val) = original_cache {
            unsafe {
                std::env::set_var("openpista_MODELS_CACHE_PATH", val);
            }
        }
    }

    // ── models_url ───────────────────────────────────────────────────────

    #[test]
    fn models_url_custom_provider_with_base_url() {
        assert_eq!(
            models_url("custom", Some("https://api.example.com/v1")),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn models_url_custom_provider_no_base_url_defaults_openai() {
        assert_eq!(
            models_url("custom", None),
            "https://api.openai.com/v1/models"
        );
    }

    // ── save_cache / load_cache edge cases ───────────────────────────────

    #[test]
    fn save_cache_creates_nested_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("deep").join("nested").join("cache.json");
        let cached = CachedCatalog {
            fetched_at: Utc::now(),
            entries: vec![],
        };
        save_cache(&path, &cached).expect("should create parent dirs and save");
        assert!(path.exists());
    }

    #[test]
    fn load_cache_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nonexistent.json");
        assert!(load_cache(&path).is_none());
    }

    #[test]
    fn load_cache_invalid_json_returns_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("bad.json");
        std::fs::write(&path, "not valid json!").unwrap();
        assert!(load_cache(&path).is_none());
    }

    #[test]
    fn save_and_load_cache_roundtrip_empty_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("empty.json");
        let cached = CachedCatalog {
            fetched_at: Utc::now(),
            entries: vec![],
        };
        save_cache(&path, &cached).expect("save");
        let loaded = load_cache(&path).expect("load");
        assert!(loaded.entries.is_empty());
    }

    // ── model_sections additional ────────────────────────────────────────

    #[test]
    fn model_sections_empty_entries() {
        let sections = model_sections(&[], "", true);
        assert!(sections.recommended_available.is_empty());
        assert!(sections.recommended_unavailable.is_empty());
        assert!(sections.other_available.is_empty());
    }

    #[test]
    fn model_sections_query_matches_nothing() {
        let entries = vec![ModelCatalogEntry {
            id: "gpt-4o".into(),
            provider: String::new(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }];
        let sections = model_sections(&entries, "nonexistent", true);
        assert!(sections.recommended_available.is_empty());
        assert!(sections.recommended_unavailable.is_empty());
        assert!(sections.other_available.is_empty());
    }

    // ── model_summary additional ─────────────────────────────────────────

    #[test]
    fn model_summary_query_matches_none() {
        let entries = vec![ModelCatalogEntry {
            id: "gpt-4o".into(),
            provider: String::new(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }];
        let summary = model_summary(&entries, "zzzzz", true);
        assert_eq!(summary.total, 1);
        assert_eq!(summary.matched, 0);
        assert_eq!(summary.recommended, 0);
        assert_eq!(summary.available, 0);
    }

    #[test]
    fn model_summary_all_recommended_and_available() {
        let entries = vec![
            ModelCatalogEntry {
                id: "a".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "b".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ];
        let summary = model_summary(&entries, "", false);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.matched, 2);
        assert_eq!(summary.recommended, 2);
        assert_eq!(summary.available, 2);
    }

    // ── filtered_entries additional ──────────────────────────────────────

    #[test]
    fn filtered_entries_case_insensitive_query() {
        let entries = vec![ModelCatalogEntry {
            id: "GPT-4o-Mini".into(),
            provider: String::new(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }];
        let result = filtered_entries(&entries, "gpt-4o-mini", false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "GPT-4o-Mini");
    }

    #[test]
    fn filtered_entries_multiple_same_id_all_included() {
        let entries = vec![
            ModelCatalogEntry {
                id: "model-x".into(),
                provider: "p1".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "model-x".into(),
                provider: "p2".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Api,
                available: true,
            },
        ];
        let result = filtered_entries(&entries, "", false);
        assert_eq!(result.len(), 2);
    }

    // ── merge_seed_with_remote additional ────────────────────────────────

    #[test]
    fn merge_empty_seed_with_remote_creates_api_entries() {
        let merged = merge_seed_with_remote(&[], &["remote-a".to_string(), "remote-b".to_string()]);
        assert_eq!(merged.len(), 2);
        for entry in &merged {
            assert_eq!(entry.source, ModelSource::Api);
            assert!(!entry.recommended_for_coding);
            assert_eq!(entry.status, ModelStatus::Unknown);
            assert!(entry.available);
        }
    }

    #[test]
    fn merge_empty_remote_returns_seed_as_is() {
        let seed = seed_models_for_provider("anthropic");
        let seed_len = seed.len();
        let merged = merge_seed_with_remote(&seed, &[]);
        assert_eq!(merged.len(), seed_len);
        assert!(merged.iter().all(|e| e.source == ModelSource::Docs));
    }

    #[test]
    fn merge_duplicate_remote_ids_deduplicated() {
        let merged = merge_seed_with_remote(
            &[],
            &["dup".to_string(), "dup".to_string(), "dup".to_string()],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "dup");
    }

    // ── seed_models_for_provider additional ──────────────────────────────

    #[test]
    fn seed_models_openai_count() {
        let entries = seed_models_for_provider("openai");
        assert_eq!(entries.len(), 9);
        assert!(entries.iter().all(|e| e.provider == "openai"));
    }

    #[test]
    fn seed_models_openrouter_count() {
        let entries = seed_models_for_provider("openrouter");
        assert_eq!(entries.len(), 1);
        assert!(entries.iter().all(|e| e.provider == "openrouter"));
    }

    #[test]
    fn seed_models_github_copilot_count_and_provider() {
        let entries = seed_models_for_provider("github-copilot");
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.provider == "github-copilot"));
    }

    #[test]
    fn seed_models_github_copilot_contains_expected_model_ids() {
        let entries = seed_models_for_provider("github-copilot");
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"gpt-4o"), "should contain gpt-4o");
        assert!(ids.contains(&"gpt-4o-mini"), "should contain gpt-4o-mini");
        assert!(
            ids.contains(&"claude-3.5-sonnet"),
            "should contain claude-3.5-sonnet"
        );
    }

    #[test]
    fn seed_models_github_copilot_recommended_flags() {
        let entries = seed_models_for_provider("github-copilot");
        let gpt4o = entries.iter().find(|e| e.id == "gpt-4o").unwrap();
        assert!(gpt4o.recommended_for_coding, "gpt-4o should be recommended");
        let mini = entries.iter().find(|e| e.id == "gpt-4o-mini").unwrap();
        assert!(
            !mini.recommended_for_coding,
            "gpt-4o-mini should not be recommended"
        );
        let claude = entries
            .iter()
            .find(|e| e.id == "claude-3.5-sonnet")
            .unwrap();
        assert!(
            claude.recommended_for_coding,
            "claude-3.5-sonnet should be recommended"
        );
    }

    #[test]
    fn seed_models_github_copilot_all_stable_and_docs() {
        let entries = seed_models_for_provider("github-copilot");
        for entry in &entries {
            assert_eq!(
                entry.status,
                ModelStatus::Stable,
                "seed model {} should be Stable",
                entry.id
            );
            assert_eq!(
                entry.source,
                ModelSource::Docs,
                "seed model {} should have Docs source",
                entry.id
            );
            assert!(
                entry.available,
                "seed model {} should be available",
                entry.id
            );
        }
    }

    #[test]
    fn seed_models_all_have_docs_source() {
        for provider in &[
            "anthropic",
            "openai",
            "together",
            "openrouter",
            "github-copilot",
        ] {
            let entries = seed_models_for_provider(provider);
            for entry in &entries {
                assert_eq!(
                    entry.source,
                    ModelSource::Docs,
                    "seed model {} for {} should have Docs source",
                    entry.id,
                    provider
                );
            }
        }
    }

    // ── load_cache_if_fresh ──────────────────────────────────────────────

    #[test]
    fn load_cache_if_fresh_returns_none_for_expired() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("expired.json");
        let cached = CachedCatalog {
            fetched_at: Utc::now() - Duration::seconds(CACHE_TTL_SECS + 3600),
            entries: seed_models_for_provider("anthropic"),
        };
        save_cache(&path, &cached).unwrap();
        assert!(load_cache_if_fresh(&path).is_none());
        // But raw load still works
        assert!(load_cache(&path).is_some());
    }

    #[test]
    fn load_cache_if_fresh_returns_some_for_recent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("recent.json");
        let cached = CachedCatalog {
            fetched_at: Utc::now() - Duration::seconds(60),
            entries: vec![ModelCatalogEntry {
                id: "test-model".into(),
                provider: "test".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            }],
        };
        save_cache(&path, &cached).unwrap();
        let loaded = load_cache_if_fresh(&path).expect("should be fresh");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].id, "test-model");
    }

    // ── CachedCatalog serde roundtrip ────────────────────────────────────

    #[test]
    fn cached_catalog_serde_roundtrip() {
        let catalog = CachedCatalog {
            fetched_at: Utc::now(),
            entries: vec![
                ModelCatalogEntry {
                    id: "model-a".into(),
                    provider: "prov".into(),
                    recommended_for_coding: true,
                    status: ModelStatus::Stable,
                    source: ModelSource::Docs,
                    available: true,
                },
                ModelCatalogEntry {
                    id: "model-b".into(),
                    provider: "prov".into(),
                    recommended_for_coding: false,
                    status: ModelStatus::Preview,
                    source: ModelSource::Api,
                    available: false,
                },
            ],
        };
        let json = serde_json::to_string(&catalog).unwrap();
        let deserialized: CachedCatalog = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.entries.len(), 2);
        assert_eq!(deserialized.entries[0].id, "model-a");
        assert_eq!(deserialized.entries[1].status, ModelStatus::Preview);
        assert_eq!(deserialized.entries[1].source, ModelSource::Api);
    }

    // ── backfill_provider additional ───────────────────────────────────

    #[test]
    fn backfill_provider_preserves_api_source_entries() {
        let entries = vec![
            ModelCatalogEntry {
                id: "api-model".into(),
                provider: String::new(),
                recommended_for_coding: false,
                status: ModelStatus::Unknown,
                source: ModelSource::Api,
                available: true,
            },
            ModelCatalogEntry {
                id: "docs-model".into(),
                provider: String::new(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
        ];
        let result = backfill_provider(entries, "together");
        assert_eq!(result[0].provider, "together");
        assert_eq!(result[0].source, ModelSource::Api);
        assert_eq!(result[1].provider, "together");
        assert_eq!(result[1].source, ModelSource::Docs);
    }

    // ── matches_query additional ─────────────────────────────────────────

    #[test]
    fn matches_query_case_insensitive_both_directions() {
        assert!(matches_query("CLAUDE-SONNET", "claude"));
        assert!(matches_query("claude-sonnet", "CLAUDE"));
        assert!(matches_query("Claude-Sonnet", "sonnet"));
    }

    #[test]
    fn matches_query_partial_match() {
        assert!(matches_query("gpt-4o-mini-2025", "4o-mini"));
        assert!(!matches_query("gpt-4o", "gpt-5"));
    }

    // ── seed_models_for_provider additional branches ──────────────────

    #[test]
    fn seed_models_for_openai_provider() {
        let entries = seed_models_for_provider("openai");
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.provider == "openai"));
        assert!(entries.iter().any(|e| e.id == "gpt-5.3-codex"));
        assert!(entries.iter().any(|e| e.id == "gpt-4o"));
        assert!(entries.iter().any(|e| e.id == "o3"));
        assert!(entries.iter().any(|e| e.id == "gpt-4.1"));
        assert!(entries.iter().any(|e| e.recommended_for_coding));
        assert!(entries.iter().any(|e| !e.recommended_for_coding));
    }

    #[test]
    fn seed_models_for_openrouter_provider() {
        let entries = seed_models_for_provider("openrouter");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, "openrouter");
        assert_eq!(entries[0].id, "openai/gpt-4o");
        assert!(entries[0].recommended_for_coding);
        assert!(entries[0].available);
    }

    #[test]
    fn seed_models_for_ollama_provider() {
        let entries = seed_models_for_provider("ollama");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, "ollama");
        assert_eq!(entries[0].id, "llama3.2");
        assert!(entries[0].recommended_for_coding);
        assert_eq!(entries[0].status, ModelStatus::Stable);
        assert_eq!(entries[0].source, ModelSource::Docs);
    }

    #[test]
    fn seed_models_for_github_copilot_provider() {
        let entries = seed_models_for_provider("github-copilot");
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.provider == "github-copilot"));
        assert!(entries.iter().any(|e| e.id == "gpt-4o"));
        assert!(entries.iter().any(|e| e.id == "claude-3.5-sonnet"));
        assert!(entries.iter().any(|e| e.recommended_for_coding));
    }
}
