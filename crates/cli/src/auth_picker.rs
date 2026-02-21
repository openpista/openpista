use crate::config::{
    LoginAuthMode, ProviderRegistryEntry, provider_registry_entry_ci, provider_registry_for_picker,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethodChoice {
    OAuth,
    ApiKey,
}

impl AuthMethodChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::OAuth => "ChatGPT Plus/Pro (OAuth)",
            Self::ApiKey => "Manually enter API key",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginBrowseStep {
    SelectProvider,
    SelectMethod,
    InputEndpoint,
    InputApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthLoginIntent {
    pub provider: String,
    pub auth_method: AuthMethodChoice,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
}

pub fn provider_matches_query(entry: &ProviderRegistryEntry, query: &str) -> bool {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return true;
    }

    entry.name.to_ascii_lowercase().contains(&needle)
        || entry.display_name.to_ascii_lowercase().contains(&needle)
        || entry
            .search_aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().contains(&needle))
}

pub fn filtered_provider_entries(query: &str) -> Vec<ProviderRegistryEntry> {
    provider_registry_for_picker()
        .into_iter()
        .filter(|entry| provider_matches_query(entry, query))
        .collect()
}

pub fn provider_step_for_entry(entry: &ProviderRegistryEntry) -> LoginBrowseStep {
    if entry.name == "openai" {
        LoginBrowseStep::SelectMethod
    } else {
        match entry.auth_mode {
            LoginAuthMode::OAuth => LoginBrowseStep::SelectProvider,
            LoginAuthMode::ApiKey => LoginBrowseStep::InputApiKey,
            LoginAuthMode::EndpointAndKey => LoginBrowseStep::InputEndpoint,
            LoginAuthMode::None => LoginBrowseStep::SelectProvider,
        }
    }
}

pub fn parse_provider_seed(seed: Option<&str>) -> String {
    seed.unwrap_or_default().trim().to_string()
}

pub fn api_key_method_for_provider(
    provider: &str,
    preferred: Option<AuthMethodChoice>,
) -> AuthMethodChoice {
    if provider == "openai" {
        preferred.unwrap_or(AuthMethodChoice::OAuth)
    } else {
        preferred.unwrap_or(AuthMethodChoice::ApiKey)
    }
}

#[cfg(not(test))]
fn category_label(entry: &ProviderRegistryEntry) -> &'static str {
    match entry.category {
        crate::config::ProviderCategory::Runtime => "runtime",
        crate::config::ProviderCategory::Extension => "extension",
    }
}

#[cfg(not(test))]
fn masked_push(masked: &mut String, input: &mut String, c: char) {
    input.push(c);
    masked.push('*');
}

#[cfg(not(test))]
fn masked_pop(masked: &mut String, input: &mut String) {
    if input.pop().is_some() {
        masked.pop();
    }
}

#[cfg(not(test))]
struct TerminalUiGuard;

#[cfg(not(test))]
impl Drop for TerminalUiGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

#[cfg(not(test))]
#[derive(Debug, Clone)]
struct CliPickerState {
    query: String,
    cursor: usize,
    step: LoginBrowseStep,
    selected_provider: Option<ProviderRegistryEntry>,
    selected_method: Option<AuthMethodChoice>,
    endpoint: Option<String>,
    input_buffer: String,
    masked_buffer: String,
    last_error: Option<String>,
}

#[cfg(not(test))]
impl CliPickerState {
    fn new(seed: Option<&str>) -> Self {
        Self {
            query: parse_provider_seed(seed),
            cursor: 0,
            step: LoginBrowseStep::SelectProvider,
            selected_provider: None,
            selected_method: None,
            endpoint: None,
            input_buffer: String::new(),
            masked_buffer: String::new(),
            last_error: None,
        }
    }

    fn providers(&self) -> Vec<ProviderRegistryEntry> {
        filtered_provider_entries(&self.query)
    }

    fn clamp_cursor(&mut self, len: usize) {
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }

    fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_down(&mut self, len: usize) {
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor + 1).min(len.saturating_sub(1));
    }
}

#[cfg(not(test))]
fn render_cli_picker(state: &CliPickerState) -> anyhow::Result<()> {
    use crossterm::{
        cursor::MoveTo,
        execute,
        terminal::{Clear, ClearType},
    };
    use std::io::{Write, stdout};

    let mut out = stdout();
    execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;

    println!("┌  Add credential");
    println!("│");

    match state.step {
        LoginBrowseStep::SelectProvider => {
            println!("◆  Select provider");
            println!("│");
            println!("│  Search: {}", state.query);

            let providers = state.providers();
            if providers.is_empty() {
                println!("│  No matches for '{}'.", state.query);
            } else {
                let max_rows = 10usize;
                let end = providers.len().min(max_rows);
                for (idx, entry) in providers.iter().take(max_rows).enumerate() {
                    let bullet = if idx == state.cursor { "●" } else { "○" };
                    println!(
                        "│  {} {} [{}]",
                        bullet,
                        entry.display_name,
                        category_label(entry)
                    );
                }
                if providers.len() > end {
                    println!("│  ... ({} more)", providers.len() - end);
                }
            }
            println!("│  ↑/↓ to select • Enter: confirm • Type: to search • Esc: cancel");
        }
        LoginBrowseStep::SelectMethod => {
            let provider = state
                .selected_provider
                .as_ref()
                .map(|p| p.display_name)
                .unwrap_or("OpenAI");
            println!("◆  Select auth method");
            println!("│");
            println!("│  Provider: {}", provider);
            let options = [AuthMethodChoice::OAuth, AuthMethodChoice::ApiKey];
            for (idx, choice) in options.iter().enumerate() {
                let bullet = if idx == state.cursor { "●" } else { "○" };
                println!("│  {} {}", bullet, choice.label());
            }
            println!("│  ↑/↓ to select • Enter: confirm • Esc: back");
        }
        LoginBrowseStep::InputEndpoint => {
            let provider = state
                .selected_provider
                .as_ref()
                .map(|p| p.display_name)
                .unwrap_or("provider");
            println!("◆  Enter endpoint");
            println!("│");
            println!("│  Provider: {}", provider);
            println!("│  Endpoint: {}", state.input_buffer);
            println!("│  Enter: confirm • Type: input • Backspace: delete • Esc: back");
        }
        LoginBrowseStep::InputApiKey => {
            let provider = state
                .selected_provider
                .as_ref()
                .map(|p| p.display_name)
                .unwrap_or("provider");
            let env_name = state
                .selected_provider
                .as_ref()
                .map(|p| p.api_key_env)
                .unwrap_or("API_KEY");
            println!("◆  Enter API key");
            println!("│");
            println!("│  Provider: {}", provider);
            if let Some(endpoint) = &state.endpoint {
                println!("│  Endpoint: {}", endpoint);
            }
            println!("│  {}: {}", env_name, state.masked_buffer);
            println!("│  Enter: confirm • Type: input • Backspace: delete • Esc: back");
        }
    }

    if let Some(err) = &state.last_error {
        println!("│");
        println!("│  Error: {}", err);
    }
    println!("└");

    out.flush()?;
    Ok(())
}

#[cfg(not(test))]
pub fn run_cli_auth_picker(
    initial_provider_seed: Option<&str>,
) -> anyhow::Result<Option<AuthLoginIntent>> {
    use crossterm::{
        cursor::Hide,
        event::{Event, KeyCode, KeyEventKind, KeyModifiers, read},
        execute,
        terminal::{EnterAlternateScreen, enable_raw_mode},
    };

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen, Hide)?;
    let _guard = TerminalUiGuard;

    let mut state = CliPickerState::new(initial_provider_seed);

    loop {
        if matches!(state.step, LoginBrowseStep::SelectProvider) {
            let len = state.providers().len();
            state.clamp_cursor(len);
        }
        render_cli_picker(&state)?;

        let event = read()?;
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match state.step {
            LoginBrowseStep::SelectProvider => {
                let providers = state.providers();
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                        return Ok(None);
                    }
                    (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                        state.move_up();
                    }
                    (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                        state.move_down(providers.len());
                    }
                    (_, KeyCode::Backspace) => {
                        state.query.pop();
                        state.cursor = 0;
                    }
                    (_, KeyCode::Enter) => {
                        if providers.is_empty() {
                            state.last_error = Some(format!("No matches for '{}'.", state.query));
                            continue;
                        }
                        let selected = providers[state.cursor];
                        state.selected_provider = Some(selected);
                        state.selected_method = None;
                        state.endpoint = None;
                        state.input_buffer.clear();
                        state.masked_buffer.clear();
                        state.last_error = None;
                        state.cursor = 0;

                        state.step = match provider_step_for_entry(&selected) {
                            LoginBrowseStep::SelectMethod => LoginBrowseStep::SelectMethod,
                            LoginBrowseStep::InputEndpoint => LoginBrowseStep::InputEndpoint,
                            LoginBrowseStep::InputApiKey => LoginBrowseStep::InputApiKey,
                            LoginBrowseStep::SelectProvider => match selected.auth_mode {
                                LoginAuthMode::OAuth => {
                                    return Ok(Some(AuthLoginIntent {
                                        provider: selected.name.to_string(),
                                        auth_method: AuthMethodChoice::OAuth,
                                        endpoint: None,
                                        api_key: None,
                                    }));
                                }
                                LoginAuthMode::None => {
                                    state.last_error = Some(format!(
                                        "Provider '{}' does not require login.",
                                        selected.display_name
                                    ));
                                    LoginBrowseStep::SelectProvider
                                }
                                _ => LoginBrowseStep::SelectProvider,
                            },
                        };
                    }
                    (_, KeyCode::Char(c)) => {
                        state.query.push(c);
                        state.cursor = 0;
                    }
                    _ => {}
                }
            }
            LoginBrowseStep::SelectMethod => match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(None),
                (_, KeyCode::Esc) => {
                    state.step = LoginBrowseStep::SelectProvider;
                    state.cursor = 0;
                }
                (_, KeyCode::Up) | (_, KeyCode::Char('k')) => state.move_up(),
                (_, KeyCode::Down) | (_, KeyCode::Char('j')) => state.move_down(2),
                (_, KeyCode::Enter) => {
                    let provider = match state.selected_provider {
                        Some(p) => p,
                        None => {
                            state.step = LoginBrowseStep::SelectProvider;
                            continue;
                        }
                    };
                    let method = if state.cursor == 0 {
                        AuthMethodChoice::OAuth
                    } else {
                        AuthMethodChoice::ApiKey
                    };
                    state.selected_method = Some(method);
                    if method == AuthMethodChoice::OAuth {
                        return Ok(Some(AuthLoginIntent {
                            provider: provider.name.to_string(),
                            auth_method: AuthMethodChoice::OAuth,
                            endpoint: None,
                            api_key: None,
                        }));
                    }
                    state.step = LoginBrowseStep::InputApiKey;
                    state.input_buffer.clear();
                    state.masked_buffer.clear();
                }
                _ => {}
            },
            LoginBrowseStep::InputEndpoint => match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(None),
                (_, KeyCode::Esc) => {
                    state.step = LoginBrowseStep::SelectProvider;
                    state.cursor = 0;
                    state.input_buffer.clear();
                }
                (_, KeyCode::Backspace) => {
                    state.input_buffer.pop();
                }
                (_, KeyCode::Enter) => {
                    let endpoint = state.input_buffer.trim().to_string();
                    if endpoint.is_empty() {
                        state.last_error = Some("Endpoint is required".to_string());
                        continue;
                    }
                    state.endpoint = Some(endpoint);
                    state.input_buffer.clear();
                    state.masked_buffer.clear();
                    state.last_error = None;
                    state.step = LoginBrowseStep::InputApiKey;
                }
                (_, KeyCode::Char(c)) => {
                    state.input_buffer.push(c);
                }
                _ => {}
            },
            LoginBrowseStep::InputApiKey => match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(None),
                (_, KeyCode::Esc) => {
                    if let Some(provider) = state.selected_provider {
                        if provider.name == "openai" {
                            state.step = LoginBrowseStep::SelectMethod;
                            state.cursor = 1;
                        } else if matches!(provider.auth_mode, LoginAuthMode::EndpointAndKey) {
                            state.step = LoginBrowseStep::InputEndpoint;
                            state.input_buffer = state.endpoint.clone().unwrap_or_default();
                        } else {
                            state.step = LoginBrowseStep::SelectProvider;
                            state.cursor = 0;
                        }
                    } else {
                        state.step = LoginBrowseStep::SelectProvider;
                    }
                    state.input_buffer.clear();
                    state.masked_buffer.clear();
                }
                (_, KeyCode::Backspace) => {
                    masked_pop(&mut state.masked_buffer, &mut state.input_buffer);
                }
                (_, KeyCode::Enter) => {
                    let provider = match state.selected_provider {
                        Some(provider) => provider,
                        None => {
                            state.step = LoginBrowseStep::SelectProvider;
                            continue;
                        }
                    };
                    let key = state.input_buffer.trim().to_string();
                    if key.is_empty() {
                        state.last_error = Some("API key is required".to_string());
                        continue;
                    }
                    return Ok(Some(AuthLoginIntent {
                        provider: provider.name.to_string(),
                        auth_method: api_key_method_for_provider(
                            provider.name,
                            state.selected_method,
                        ),
                        endpoint: state.endpoint.clone(),
                        api_key: Some(key),
                    }));
                }
                (_, KeyCode::Char(c)) => {
                    masked_push(&mut state.masked_buffer, &mut state.input_buffer, c);
                }
                _ => {}
            },
        }
    }
}

pub fn provider_by_name(name: &str) -> Option<ProviderRegistryEntry> {
    provider_registry_entry_ci(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_matches_query_checks_name_display_and_alias() {
        let entry = provider_by_name("openai").expect("openai provider");
        assert!(provider_matches_query(&entry, "open"));
        assert!(provider_matches_query(&entry, "chatgpt"));
        assert!(provider_matches_query(&entry, "gpt"));
        assert!(!provider_matches_query(&entry, "bedrock"));
    }

    #[test]
    fn filtered_provider_entries_respects_case_insensitive_substring() {
        let entries = filtered_provider_entries("copilot");
        assert!(entries.iter().any(|entry| entry.name == "github-copilot"));
        let entries = filtered_provider_entries("OPEN");
        assert!(entries.iter().any(|entry| entry.name == "openai"));
    }

    #[test]
    fn provider_step_for_entry_maps_auth_modes() {
        let openai = provider_by_name("openai").expect("openai provider");
        assert_eq!(
            provider_step_for_entry(&openai),
            LoginBrowseStep::SelectMethod
        );

        let together = provider_by_name("together").expect("together provider");
        assert_eq!(
            provider_step_for_entry(&together),
            LoginBrowseStep::InputApiKey
        );

        let azure = provider_by_name("azure-openai").expect("azure provider");
        assert_eq!(
            provider_step_for_entry(&azure),
            LoginBrowseStep::InputEndpoint
        );
    }

    #[test]
    fn api_key_method_defaults() {
        assert_eq!(
            api_key_method_for_provider("openai", None),
            AuthMethodChoice::OAuth
        );
        assert_eq!(
            api_key_method_for_provider("openai", Some(AuthMethodChoice::ApiKey)),
            AuthMethodChoice::ApiKey
        );
        assert_eq!(
            api_key_method_for_provider("together", None),
            AuthMethodChoice::ApiKey
        );
    }
}
