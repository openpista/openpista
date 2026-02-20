//! Browser automation tools backed by Chromium CDP.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use chromiumoxide::Page;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::ScreenshotParams;
use futures_util::StreamExt;
use proto::ToolResult;
use reqwest::Url;
use serde::Deserialize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::Tool;

/// Tool that navigates the shared browser page to a URL.
pub struct BrowserTool;
/// Tool that clicks an element on the shared browser page.
pub struct BrowserClickTool;
/// Tool that types text into an element on the shared browser page.
pub struct BrowserTypeTool;
/// Tool that captures a screenshot from the shared browser page.
pub struct BrowserScreenshotTool;

const DEFAULT_TIMEOUT_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 60;

struct BrowserState {
    browser: Option<Browser>,
    page: Option<Page>,
    handler_task: Option<JoinHandle<()>>,
}

impl BrowserState {
    fn new() -> Self {
        Self {
            browser: None,
            page: None,
            handler_task: None,
        }
    }

    async fn ensure_ready(&mut self) -> Result<(), String> {
        if self.browser.is_none() {
            self.launch().await?;
        }

        if self.page.is_none() {
            let browser = self
                .browser
                .as_mut()
                .ok_or_else(|| "Browser is not initialized".to_string())?;
            let page = browser
                .new_page("about:blank")
                .await
                .map_err(|e| format!("Failed to create page: {e}"))?;
            self.page = Some(page);
        }

        Ok(())
    }

    async fn launch(&mut self) -> Result<(), String> {
        let config = BrowserConfig::builder()
            .build()
            .map_err(|e| format!("Failed to build browser config: {e}"))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| format!("Failed to launch browser: {e}"))?;

        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        self.browser = Some(browser);
        self.handler_task = Some(handler_task);
        Ok(())
    }
}

impl Drop for BrowserState {
    fn drop(&mut self) {
        if let Some(handle) = self.handler_task.take() {
            handle.abort();
        }
    }
}

fn shared_state() -> Arc<Mutex<BrowserState>> {
    static STATE: OnceLock<Arc<Mutex<BrowserState>>> = OnceLock::new();
    STATE
        .get_or_init(|| Arc::new(Mutex::new(BrowserState::new())))
        .clone()
}

fn operation_timeout(timeout_secs: Option<u64>) -> Duration {
    Duration::from_secs(
        timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS),
    )
}

#[derive(Debug, Deserialize)]
struct NavigateArgs {
    url: String,
    timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ClickArgs {
    selector: String,
    timeout_secs: Option<u64>,
    wait_for_navigation: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TypeArgs {
    selector: String,
    text: String,
    timeout_secs: Option<u64>,
    press_enter: Option<bool>,
    wait_for_navigation: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScreenshotArgs {
    full_page: Option<bool>,
    timeout_secs: Option<u64>,
}

impl BrowserTool {
    /// Creates a browser navigation tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserClickTool {
    /// Creates a browser click tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserClickTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTypeTool {
    /// Creates a browser typing tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserTypeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserScreenshotTool {
    /// Creates a browser screenshot tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser.navigate"
    }

    fn description(&self) -> &str {
        "Navigate the browser page to a URL and return final URL and title"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to navigate to"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Operation timeout in seconds (default: 15, max: 60)"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: NavigateArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let url = match Url::parse(&parsed.url) {
            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u,
            Ok(_) => {
                return ToolResult::error(
                    call_id,
                    self.name(),
                    "Only http/https URLs are supported".to_string(),
                );
            }
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid URL: {e}"));
            }
        };

        let timeout_duration = operation_timeout(parsed.timeout_secs);
        let state = shared_state();

        let run = timeout(timeout_duration, async move {
            let mut state = state.lock().await;
            state.ensure_ready().await?;

            let page = state
                .page
                .as_ref()
                .ok_or_else(|| "Browser page is not initialized".to_string())?;

            page.goto(url.as_str())
                .await
                .map_err(|e| format!("Navigation failed: {e}"))?;

            let final_url = page
                .url()
                .await
                .map_err(|e| format!("Failed to read page URL: {e}"))?
                .unwrap_or_else(|| url.to_string());

            let title = page
                .get_title()
                .await
                .map_err(|e| format!("Failed to read page title: {e}"))?;

            let output = serde_json::json!({
                "action": "navigate",
                "requested_url": url.as_str(),
                "final_url": final_url,
                "title": title,
            });

            serde_json::to_string(&output).map_err(|e| format!("Failed to encode output: {e}"))
        })
        .await;

        match run {
            Ok(Ok(payload)) => ToolResult::success(call_id, self.name(), payload),
            Ok(Err(err)) => ToolResult::error(call_id, self.name(), err),
            Err(_) => ToolResult::error(
                call_id,
                self.name(),
                format!("Operation timed out after {}s", timeout_duration.as_secs()),
            ),
        }
    }
}

#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &str {
        "browser.click"
    }

    fn description(&self) -> &str {
        "Click an element on the current browser page"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector for the element to click"
                },
                "wait_for_navigation": {
                    "type": "boolean",
                    "description": "Wait for navigation after click (default: false)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Operation timeout in seconds (default: 15, max: 60)"
                }
            },
            "required": ["selector"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: ClickArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let timeout_duration = operation_timeout(parsed.timeout_secs);
        let wait_for_navigation = parsed.wait_for_navigation.unwrap_or(false);
        let selector = parsed.selector;
        let state = shared_state();

        let run = timeout(timeout_duration, async move {
            let mut state = state.lock().await;
            state.ensure_ready().await?;

            let page = state
                .page
                .as_ref()
                .ok_or_else(|| "Browser page is not initialized".to_string())?;

            let element = page
                .find_element(&selector)
                .await
                .map_err(|e| format!("Failed to find element '{selector}': {e}"))?;

            element
                .click()
                .await
                .map_err(|e| format!("Failed to click element '{selector}': {e}"))?;

            if wait_for_navigation {
                page.wait_for_navigation()
                    .await
                    .map_err(|e| format!("Navigation wait failed: {e}"))?;
            }

            let final_url = page
                .url()
                .await
                .map_err(|e| format!("Failed to read page URL: {e}"))?
                .unwrap_or_default();

            let output = serde_json::json!({
                "action": "click",
                "selector": selector,
                "final_url": final_url,
            });

            serde_json::to_string(&output).map_err(|e| format!("Failed to encode output: {e}"))
        })
        .await;

        match run {
            Ok(Ok(payload)) => ToolResult::success(call_id, self.name(), payload),
            Ok(Err(err)) => ToolResult::error(call_id, self.name(), err),
            Err(_) => ToolResult::error(
                call_id,
                self.name(),
                format!("Operation timed out after {}s", timeout_duration.as_secs()),
            ),
        }
    }
}

#[async_trait]
impl Tool for BrowserTypeTool {
    fn name(&self) -> &str {
        "browser.type"
    }

    fn description(&self) -> &str {
        "Type text into an element on the current browser page"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector for the input element"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type into the target element"
                },
                "press_enter": {
                    "type": "boolean",
                    "description": "Press Enter after typing (default: false)"
                },
                "wait_for_navigation": {
                    "type": "boolean",
                    "description": "Wait for navigation after typing (default: false)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Operation timeout in seconds (default: 15, max: 60)"
                }
            },
            "required": ["selector", "text"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: TypeArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let timeout_duration = operation_timeout(parsed.timeout_secs);
        let selector = parsed.selector;
        let text = parsed.text;
        let press_enter = parsed.press_enter.unwrap_or(false);
        let wait_for_navigation = parsed.wait_for_navigation.unwrap_or(false);
        let state = shared_state();

        let run = timeout(timeout_duration, async move {
            let mut state = state.lock().await;
            state.ensure_ready().await?;

            let page = state
                .page
                .as_ref()
                .ok_or_else(|| "Browser page is not initialized".to_string())?;

            let element = page
                .find_element(&selector)
                .await
                .map_err(|e| format!("Failed to find element '{selector}': {e}"))?;

            let element = element
                .click()
                .await
                .map_err(|e| format!("Failed to focus element '{selector}': {e}"))?;

            let element = element
                .type_str(&text)
                .await
                .map_err(|e| format!("Failed to type into element '{selector}': {e}"))?;

            if press_enter {
                element
                    .press_key("Enter")
                    .await
                    .map_err(|e| format!("Failed to press Enter on '{selector}': {e}"))?;
            }

            if wait_for_navigation {
                page.wait_for_navigation()
                    .await
                    .map_err(|e| format!("Navigation wait failed: {e}"))?;
            }

            let final_url = page
                .url()
                .await
                .map_err(|e| format!("Failed to read page URL: {e}"))?
                .unwrap_or_default();

            let output = serde_json::json!({
                "action": "type",
                "selector": selector,
                "typed_chars": text.chars().count(),
                "press_enter": press_enter,
                "final_url": final_url,
            });

            serde_json::to_string(&output).map_err(|e| format!("Failed to encode output: {e}"))
        })
        .await;

        match run {
            Ok(Ok(payload)) => ToolResult::success(call_id, self.name(), payload),
            Ok(Err(err)) => ToolResult::error(call_id, self.name(), err),
            Err(_) => ToolResult::error(
                call_id,
                self.name(),
                format!("Operation timed out after {}s", timeout_duration.as_secs()),
            ),
        }
    }
}

#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "browser.screenshot"
    }

    fn description(&self) -> &str {
        "Capture a screenshot of the current browser page and return PNG bytes as base64"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "full_page": {
                    "type": "boolean",
                    "description": "Capture the full scrollable page (default: false)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Operation timeout in seconds (default: 15, max: 60)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: ScreenshotArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let timeout_duration = operation_timeout(parsed.timeout_secs);
        let full_page = parsed.full_page.unwrap_or(false);
        let state = shared_state();

        let run = timeout(timeout_duration, async move {
            let mut state = state.lock().await;
            state.ensure_ready().await?;

            let page = state
                .page
                .as_ref()
                .ok_or_else(|| "Browser page is not initialized".to_string())?;

            let screenshot = page
                .screenshot(ScreenshotParams::builder().full_page(full_page).build())
                .await
                .map_err(|e| format!("Failed to capture screenshot: {e}"))?;

            let (width, height) = image::load_from_memory(&screenshot)
                .map(|img| (img.width(), img.height()))
                .unwrap_or((0, 0));

            let final_url = page
                .url()
                .await
                .map_err(|e| format!("Failed to read page URL: {e}"))?
                .unwrap_or_default();

            let output = serde_json::json!({
                "mime": "image/png",
                "width": width,
                "height": height,
                "size_bytes": screenshot.len(),
                "data_b64": general_purpose::STANDARD.encode(&screenshot),
                "url": final_url,
                "full_page": full_page,
            });

            serde_json::to_string(&output).map_err(|e| format!("Failed to encode output: {e}"))
        })
        .await;

        match run {
            Ok(Ok(payload)) => ToolResult::success(call_id, self.name(), payload),
            Ok(Err(err)) => ToolResult::error(call_id, self.name(), err),
            Err(_) => ToolResult::error(
                call_id,
                self.name(),
                format!("Operation timed out after {}s", timeout_duration.as_secs()),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_navigate_tool_metadata_is_stable() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser.navigate");
        assert!(tool.description().contains("Navigate"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "url");
    }

    #[test]
    fn browser_click_tool_metadata_is_stable() {
        let tool = BrowserClickTool::new();
        assert_eq!(tool.name(), "browser.click");
        assert!(tool.description().contains("Click"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "selector");
    }

    #[test]
    fn browser_type_tool_metadata_is_stable() {
        let tool = BrowserTypeTool::new();
        assert_eq!(tool.name(), "browser.type");
        assert!(tool.description().contains("Type"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "selector");
        assert_eq!(schema["required"][1], "text");
    }

    #[test]
    fn browser_screenshot_tool_metadata_is_stable() {
        let tool = BrowserScreenshotTool::new();
        assert_eq!(tool.name(), "browser.screenshot");
        assert!(tool.description().contains("screenshot"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[tokio::test]
    async fn navigate_rejects_non_http_url() {
        let tool = BrowserTool::new();
        let result = tool
            .execute("call-1", serde_json::json!({"url":"file:///etc/passwd"}))
            .await;
        assert_eq!(result.call_id, "call-1");
        assert_eq!(result.tool_name, "browser.navigate");
        assert!(result.is_error);
        assert!(result.output.contains("Only http/https URLs"));
    }

    #[tokio::test]
    async fn navigate_rejects_invalid_url() {
        let tool = BrowserTool::new();
        let result = tool
            .execute("call-1b", serde_json::json!({"url":"not a url"}))
            .await;
        assert_eq!(result.call_id, "call-1b");
        assert_eq!(result.tool_name, "browser.navigate");
        assert!(result.is_error);
        assert!(result.output.contains("Invalid URL"));
    }

    #[tokio::test]
    async fn click_rejects_invalid_arguments() {
        let tool = BrowserClickTool::new();
        let result = tool
            .execute("call-2", serde_json::json!({"selector":7}))
            .await;
        assert_eq!(result.call_id, "call-2");
        assert_eq!(result.tool_name, "browser.click");
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn type_rejects_invalid_arguments() {
        let tool = BrowserTypeTool::new();
        let result = tool
            .execute("call-3", serde_json::json!({"selector":"#q"}))
            .await;
        assert_eq!(result.call_id, "call-3");
        assert_eq!(result.tool_name, "browser.type");
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn screenshot_rejects_invalid_arguments() {
        let tool = BrowserScreenshotTool::new();
        let result = tool
            .execute("call-4", serde_json::json!({"full_page":"yes"}))
            .await;
        assert_eq!(result.call_id, "call-4");
        assert_eq!(result.tool_name, "browser.screenshot");
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn navigate_with_valid_url_returns_result_shape() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                "call-5",
                serde_json::json!({"url":"https://example.com","timeout_secs":1}),
            )
            .await;
        assert_eq!(result.call_id, "call-5");
        assert_eq!(result.tool_name, "browser.navigate");
        assert!(!result.output.is_empty());
    }

    #[tokio::test]
    async fn click_with_valid_selector_returns_result_shape() {
        let tool = BrowserClickTool::new();
        let result = tool
            .execute(
                "call-6",
                serde_json::json!({"selector":"body","timeout_secs":1}),
            )
            .await;
        assert_eq!(result.call_id, "call-6");
        assert_eq!(result.tool_name, "browser.click");
        assert!(!result.output.is_empty());
    }

    #[tokio::test]
    async fn type_with_valid_selector_returns_result_shape() {
        let tool = BrowserTypeTool::new();
        let result = tool
            .execute(
                "call-7",
                serde_json::json!({"selector":"body","text":"hello","timeout_secs":1}),
            )
            .await;
        assert_eq!(result.call_id, "call-7");
        assert_eq!(result.tool_name, "browser.type");
        assert!(!result.output.is_empty());
    }

    #[tokio::test]
    async fn screenshot_with_valid_args_returns_result_shape() {
        let tool = BrowserScreenshotTool::new();
        let result = tool
            .execute(
                "call-8",
                serde_json::json!({"full_page":false,"timeout_secs":1}),
            )
            .await;
        assert_eq!(result.call_id, "call-8");
        assert_eq!(result.tool_name, "browser.screenshot");
        assert!(!result.output.is_empty());
    }

    #[test]
    fn operation_timeout_clamps_values() {
        assert_eq!(
            operation_timeout(None),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS)
        );
        assert_eq!(operation_timeout(Some(0)), Duration::from_secs(1));
        assert_eq!(
            operation_timeout(Some(MAX_TIMEOUT_SECS + 100)),
            Duration::from_secs(MAX_TIMEOUT_SECS)
        );
    }

    #[test]
    fn shared_state_returns_same_instance() {
        let a = shared_state();
        let b = shared_state();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
