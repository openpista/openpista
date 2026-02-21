use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const OPENCODE_PROVIDER: &str = "opencode";
pub const OPENCODE_MODELS_URL: &str = "https://opencode.ai/zen/v1/models";
const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelStatus {
    Stable,
    Preview,
    Unknown,
}

impl ModelStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    Docs,
    Api,
}

impl ModelSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Docs => "docs",
            Self::Api => "api",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    pub id: String,
    pub recommended_for_coding: bool,
    pub status: ModelStatus,
    pub source: ModelSource,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct CatalogLoadResult {
    pub provider: String,
    pub entries: Vec<ModelCatalogEntry>,
    pub sync_status: String,
}

#[derive(Debug, Clone, Default)]
pub struct ModelSections {
    pub recommended_available: Vec<ModelCatalogEntry>,
    pub recommended_unavailable: Vec<ModelCatalogEntry>,
    pub other_available: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSummary {
    pub total: usize,
    pub matched: usize,
    pub recommended: usize,
    pub available: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedCatalog {
    fetched_at: DateTime<Utc>,
    entries: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct ZenModelsResponse {
    data: Vec<ZenModel>,
}

#[derive(Debug, Deserialize)]
struct ZenModel {
    id: String,
}

pub fn seed_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "gpt-5-codex".to_string(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: false,
        },
        ModelCatalogEntry {
            id: "claude-sonnet-4.5".to_string(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: false,
        },
        ModelCatalogEntry {
            id: "claude-sonnet-4.6".to_string(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: false,
        },
        ModelCatalogEntry {
            id: "gemini-3.1-pro".to_string(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: false,
        },
        ModelCatalogEntry {
            id: "qwen3-coder".to_string(),
            recommended_for_coding: true,
            status: ModelStatus::Unknown,
            source: ModelSource::Docs,
            available: false,
        },
        ModelCatalogEntry {
            id: "devstral".to_string(),
            recommended_for_coding: true,
            status: ModelStatus::Unknown,
            source: ModelSource::Docs,
            available: false,
        },
    ]
}

pub fn default_cache_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENPISTACRAB_MODELS_CACHE_PATH") {
        return PathBuf::from(path);
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".openpistacrab")
        .join("cache")
        .join("models")
        .join("opencode.json")
}

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

pub async fn load_opencode_catalog(refresh: bool) -> CatalogLoadResult {
    let cache_path = default_cache_path();

    if !refresh && let Some(cached) = load_cache_if_fresh(&cache_path) {
        return CatalogLoadResult {
            provider: OPENCODE_PROVIDER.to_string(),
            entries: cached.entries,
            sync_status: format!(
                "Using cache (fetched_at={})",
                cached.fetched_at.format("%Y-%m-%d %H:%M UTC")
            ),
        };
    }

    match fetch_remote_model_ids().await {
        Ok(ids) => {
            let entries = merge_seed_with_remote(&seed_models(), &ids);
            let now = Utc::now();
            let cached = CachedCatalog {
                fetched_at: now,
                entries: entries.clone(),
            };
            let _ = save_cache(&cache_path, &cached);

            CatalogLoadResult {
                provider: OPENCODE_PROVIDER.to_string(),
                entries,
                sync_status: format!(
                    "Synced from remote ({} models, fetched_at={})",
                    ids.len(),
                    now.format("%Y-%m-%d %H:%M UTC")
                ),
            }
        }
        Err(err) => {
            if let Some(cached) = load_cache(&cache_path) {
                CatalogLoadResult {
                    provider: OPENCODE_PROVIDER.to_string(),
                    entries: cached.entries,
                    sync_status: format!("Remote failed, using cache ({})", sanitize_error(&err)),
                }
            } else {
                CatalogLoadResult {
                    provider: OPENCODE_PROVIDER.to_string(),
                    entries: seed_models(),
                    sync_status: format!(
                        "Remote/cache unavailable, using embedded seed list ({})",
                        sanitize_error(&err)
                    ),
                }
            }
        }
    }
}

pub fn merge_seed_with_remote(
    seed: &[ModelCatalogEntry],
    remote_ids: &[String],
) -> Vec<ModelCatalogEntry> {
    let remote_set: BTreeSet<String> = remote_ids.iter().cloned().collect();
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
                recommended_for_coding: false,
                status: ModelStatus::Unknown,
                source: ModelSource::Api,
                available: true,
            },
        );
    }

    for entry in by_id.values_mut() {
        match entry.source {
            ModelSource::Docs => {
                entry.available = remote_set.contains(&entry.id);
            }
            ModelSource::Api => {
                entry.available = true;
            }
        }
    }

    by_id.into_values().collect()
}

fn matches_query(haystack: &str, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }
    haystack
        .to_ascii_lowercase()
        .contains(&trimmed.to_ascii_lowercase())
}

fn sanitize_error(err: &str) -> String {
    err.replace('\n', " ")
}

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

fn load_cache(path: &std::path::Path) -> Option<CachedCatalog> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_cache(path: &std::path::Path, cached: &CachedCatalog) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_string_pretty(cached)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    std::fs::write(path, payload)
}

async fn fetch_remote_model_ids() -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|err| format!("build client: {err}"))?;

    let body: ZenModelsResponse = client
        .get(OPENCODE_MODELS_URL)
        .send()
        .await
        .map_err(|err| format!("request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("http status: {err}"))?
        .json()
        .await
        .map_err(|err| format!("json decode: {err}"))?;

    let mut ids: Vec<String> = body
        .data
        .into_iter()
        .map(|item| item.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn merge_seed_with_remote_marks_availability_and_adds_remote_only() {
        let seed = seed_models();
        let merged = merge_seed_with_remote(
            &seed,
            &[
                "gpt-5-codex".to_string(),
                "claude-sonnet-4.6".to_string(),
                "gpt-5.2-codex".to_string(),
            ],
        );

        let by_id: BTreeMap<_, _> = merged
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        assert!(by_id["gpt-5-codex"].available);
        assert!(by_id["claude-sonnet-4.6"].available);
        assert!(!by_id["qwen3-coder"].available);
        assert_eq!(by_id["gpt-5.2-codex"].source, ModelSource::Api);
        assert!(!by_id["gpt-5.2-codex"].recommended_for_coding);
    }

    #[test]
    fn filtered_entries_apply_show_all_and_query() {
        let entries = vec![
            ModelCatalogEntry {
                id: "gpt-5-codex".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "gpt-5.2".into(),
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
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "b".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: false,
            },
            ModelCatalogEntry {
                id: "c".into(),
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
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "devstral".into(),
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
            entries: seed_models(),
        };

        save_cache(&path, &cached).expect("save cache");
        let loaded = load_cache_if_fresh(&path).expect("fresh cache");
        assert_eq!(loaded.entries.len(), cached.entries.len());

        let stale = CachedCatalog {
            fetched_at: Utc::now() - Duration::seconds(CACHE_TTL_SECS + 1),
            entries: seed_models(),
        };
        save_cache(&path, &stale).expect("save stale cache");
        assert!(load_cache_if_fresh(&path).is_none());
        assert!(load_cache(&path).is_some());
    }
}
