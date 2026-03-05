//! Model catalog subcommand handlers and interactive picker.

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use agent::AutoApproveHandler;
#[cfg(not(test))]
use proto::{ChannelId, SessionId};
#[cfg(not(test))]
use tracing::{error, info};

#[cfg(not(test))]
use crate::config::{Config, ProviderPreset};
#[cfg(not(test))]
use crate::model_catalog;
#[cfg(not(test))]
use crate::startup::build_runtime;

#[cfg(not(test))]
/// Runs the interactive terminal model picker and returns the selection.
/// Uses an alternate screen with RAII cleanup and returns the result so
/// logging/printing happens after terminal restoration.
pub(crate) fn run_model_picker(
    entries: &[model_catalog::ModelCatalogEntry],
    current_model: &str,
    current_provider: &str,
) -> anyhow::Result<Option<model_catalog::ModelCatalogEntry>> {
    use crossterm::{
        cursor::{Hide, MoveTo, Show},
        event::{Event, KeyCode, KeyEventKind, KeyModifiers, read},
        execute,
        terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode},
    };
    use std::io::{Write, stdout};

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, Show);
        }
    }

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, Hide)?;
    let _guard = Guard;

    let mut query = String::new();
    let mut cursor: usize = 0;

    loop {
        let query_lc = query.to_ascii_lowercase();
        let visible: Vec<&model_catalog::ModelCatalogEntry> = entries
            .iter()
            .filter(|entry| {
                query_lc.is_empty()
                    || entry.id.to_ascii_lowercase().contains(&query_lc)
                    || entry.provider.to_ascii_lowercase().contains(&query_lc)
            })
            .collect();

        cursor = cursor.min(visible.len().saturating_sub(1));

        let mut out = stdout();
        execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;

        let mut lines: Vec<String> = Vec::new();
        lines.push("Select Model".to_string());
        lines.push(String::new());
        lines.push(format!("Search: {query}"));
        lines.push(format!(
            "Current: {} [{}]",
            if current_model.is_empty() {
                "(none)"
            } else {
                current_model
            },
            current_provider
        ));
        lines.push(String::new());

        if visible.is_empty() {
            lines.push(format!("No matches for '{query}'."));
        } else {
            let term_height = crossterm::terminal::size()
                .map(|(_, h)| h as usize)
                .unwrap_or(24);
            let max_visible = term_height.saturating_sub(9);
            let scroll_start = if cursor >= max_visible {
                cursor - max_visible + 1
            } else {
                0
            };
            let scroll_end = (scroll_start + max_visible).min(visible.len());

            for (idx, entry) in visible
                .iter()
                .enumerate()
                .skip(scroll_start)
                .take(scroll_end - scroll_start)
            {
                let marker = if idx == cursor { ">" } else { " " };
                let rec = if entry.recommended_for_coding {
                    "*"
                } else {
                    " "
                };
                let current_tag = if entry.id == current_model && entry.provider == current_provider
                {
                    " (current)"
                } else {
                    ""
                };

                lines.push(format!(
                    "{marker} {rec} {:<30} [{}]{current_tag}",
                    entry.id, entry.provider
                ));
            }
            if scroll_end < visible.len() {
                lines.push(format!("  ... and {} more", visible.len() - scroll_end));
            }
        }

        lines.push(String::new());
        lines.push(format!(
            "{} model(s) | Up/Down move | Enter select | Type search | Esc cancel",
            visible.len()
        ));

        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);
        for line in &lines {
            let display: String = line.chars().take(width).collect();
            out.write_all(display.as_bytes())?;
            out.write_all(b"\r\n")?;
        }
        out.flush()?;

        let event = read()?;
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                return Ok(None);
            }
            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                cursor = cursor.saturating_sub(1);
            }
            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                if !visible.is_empty() {
                    cursor = (cursor + 1).min(visible.len().saturating_sub(1));
                }
            }
            (_, KeyCode::Backspace) => {
                query.pop();
                cursor = 0;
            }
            (_, KeyCode::Enter) => {
                if visible.is_empty() {
                    continue;
                }
                return Ok(Some(visible[cursor].clone()));
            }
            (_, KeyCode::Char(ch)) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                query.push(ch);
                cursor = 0;
            }
            _ => {}
        }
    }
}

#[cfg(not(test))]
/// Interactive model selector: loads available models, lets user choose, and persists it.
pub(crate) async fn cmd_model_select(mut config: Config) -> anyhow::Result<()> {
    println!("Loading model catalog...");
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;

    let mut entries: Vec<model_catalog::ModelCatalogEntry> = catalog
        .entries
        .into_iter()
        .filter(|entry| entry.available)
        .collect();
    entries.sort_by(|a, b| {
        b.recommended_for_coding
            .cmp(&a.recommended_for_coding)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.id.cmp(&b.id))
    });

    if entries.is_empty() {
        anyhow::bail!(
            "No models available. Check your provider credentials with `openpista auth status`."
        );
    }

    let current_model = config.agent.effective_model().to_string();
    let current_provider = config.agent.provider.name().to_string();
    let selected = run_model_picker(&entries, &current_model, &current_provider)?;

    let Some(selected) = selected else {
        println!("Model selection cancelled.");
        return Ok(());
    };

    if let Ok(preset) = selected.provider.parse::<ProviderPreset>() {
        config.agent.provider = preset;
    }
    config.agent.model = selected.id.clone();

    if let Err(e) = config.save() {
        eprintln!("Warning: failed to save config: {e}");
    } else {
        println!("Model set: {} [{}]", selected.id, selected.provider);
        println!("Saved to ~/.openpista/config.toml");
    }

    let _ = crate::config::TuiState::save_selection(selected.id, selected.provider);

    Ok(())
}

#[cfg(not(test))]
pub(crate) async fn cmd_models(config: Config) -> anyhow::Result<()> {
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;
    let summary = model_catalog::model_summary(&catalog.entries, "", false);
    let sections = model_catalog::model_sections(&catalog.entries, "", false);
    let provider_names: Vec<&str> = providers.iter().map(|(n, _, _)| n.as_str()).collect();
    println!(
        "model | providers:{} | total:{} | matched:{} | recommended:{} | available:{}",
        provider_names.join(","),
        summary.total,
        summary.matched,
        summary.recommended,
        summary.available
    );
    for status in &catalog.sync_statuses {
        println!("{status}");
    }
    println!();
    print_model_section("Recommended + Available", &sections.recommended_available);
    print_model_section(
        "Recommended + Unavailable",
        &sections.recommended_unavailable,
    );
    Ok(())
}

#[cfg(not(test))]
fn print_model_section(title: &str, entries: &[model_catalog::ModelCatalogEntry]) {
    println!("{title} ({})", entries.len());
    for entry in entries {
        println!(
            "- {}  [provider:{}]  [status:{}]  [available:{}]  [source:{}]",
            entry.id,
            entry.provider,
            entry.status.as_str(),
            if entry.available { "yes" } else { "no" },
            entry.source.as_str()
        );
    }
    println!();
}

#[cfg(not(test))]
pub(crate) async fn collect_providers_for_test(
    config: &Config,
) -> Vec<(String, Option<String>, String)> {
    let mut providers = Vec::new();
    for preset in ProviderPreset::all() {
        let name = preset.name();
        if let Some(cred) = config.resolve_credential_for_refreshed(name).await {
            providers.push((name.to_string(), cred.base_url, cred.api_key));
        }
    }
    // Ensure the currently configured provider is always included
    let active = config.agent.provider.name().to_string();
    if !providers.iter().any(|(n, _, _)| n == &active) {
        let key = config.resolve_api_key_refreshed().await;
        if !key.is_empty() {
            providers.push((
                active,
                config.agent.effective_base_url().map(String::from),
                key,
            ));
        }
    }
    providers
}

#[cfg(not(test))]
pub(crate) async fn cmd_model_test(
    mut config: Config,
    model_name: String,
    message: String,
) -> anyhow::Result<()> {
    // Look up model in catalog to determine provider
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;
    let entry = catalog.entries.iter().find(|e| e.id == model_name);

    // Override provider if found in catalog
    if let Some(entry) = entry
        && let Ok(preset) = entry.provider.parse::<ProviderPreset>()
    {
        config.agent.provider = preset;
    }
    config.agent.model = model_name.clone();

    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let channel_id = ChannelId::new("cli", "model-test");
    let session_id = SessionId::new();
    println!(
        "Testing model: {} (provider: {})",
        model_name,
        config.agent.provider.name()
    );
    println!("Message: {message}");
    println!("---");

    let start = std::time::Instant::now();
    let result = runtime
        .process(&channel_id, &session_id, &message, None)
        .await;
    let elapsed = start.elapsed();

    match result {
        Ok((text, _usage)) => {
            println!("OK ({:.1}s)\n{text}", elapsed.as_secs_f64());
            info!(model = %model_name, elapsed_ms = %elapsed.as_millis(), "Model test passed");
        }
        Err(e) => {
            eprintln!("FAIL ({:.1}s): {e}", elapsed.as_secs_f64());
            error!(model = %model_name, error = %e, "Model test failed");
            std::process::exit(1);
        }
    }
    Ok(())
}

#[cfg(not(test))]
pub(crate) async fn cmd_model_test_all(config: Config, message: String) -> anyhow::Result<()> {
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;

    // Filter to recommended + available models
    let test_models: Vec<_> = catalog
        .entries
        .iter()
        .filter(|e| e.recommended_for_coding && e.available)
        .collect();

    if test_models.is_empty() {
        println!("No recommended & available models found. Run `openpista auth login` first.");
        return Ok(());
    }

    println!("Testing all available models with: \"{message}\"\n");

    let mut passed = 0u32;
    let mut failed = 0u32;
    let total = test_models.len();

    for entry in &test_models {
        let mut test_config = config.clone();
        if let Ok(preset) = entry.provider.parse::<ProviderPreset>() {
            test_config.agent.provider = preset;
        }
        test_config.agent.model = entry.id.clone();
        let runtime = match build_runtime(&test_config, Arc::new(AutoApproveHandler)).await {
            Ok(rt) => rt,
            Err(e) => {
                println!("  [{}] {:<24} FAIL (setup): {e}", entry.provider, entry.id);
                failed += 1;
                continue;
            }
        };

        let channel_id = ChannelId::new("cli", "model-test");
        let session_id = SessionId::new();

        let start = std::time::Instant::now();
        let result = runtime
            .process(&channel_id, &session_id, &message, None)
            .await;
        let elapsed = start.elapsed();

        match result {
            Ok((text, _usage)) => {
                let preview: String = text.chars().take(50).collect();
                let preview = preview.replace('\n', " ");
                println!(
                    "  [{}] {:<24} OK ({:.1}s) \u{2014} \"{}{}\"",
                    entry.provider,
                    entry.id,
                    elapsed.as_secs_f64(),
                    preview,
                    if text.len() > 50 { "..." } else { "" }
                );
                info!(model = %entry.id, provider = %entry.provider, elapsed_ms = %elapsed.as_millis(), "Model test passed");
                passed += 1;
            }
            Err(e) => {
                println!(
                    "  [{}] {:<24} FAIL ({:.1}s) \u{2014} {e}",
                    entry.provider,
                    entry.id,
                    elapsed.as_secs_f64()
                );
                error!(model = %entry.id, provider = %entry.provider, error = %e, "Model test failed");
                failed += 1;
            }
        }
    }

    println!("\nResults: {passed} passed, {failed} failed out of {total} models");

    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
