//! macOS UI automation tools via AppleScript/JXA.
//!
//! All tools in this module are gated behind `#[cfg(target_os = "macos")]`.
//! On non-macOS platforms the module compiles but the tools return an
//! "unsupported platform" error.

pub mod app;
pub mod click;
pub mod clipboard;
pub mod key;
pub mod snapshot;
pub mod type_tool;

pub use app::UiAppTool;
pub use click::UiClickTool;
pub use clipboard::UiClipboardTool;
pub use key::UiKeyTool;
pub use snapshot::UiSnapshotTool;
pub use type_tool::UiTypeTool;

use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::debug;

const DEFAULT_TIMEOUT_SECS: u64 = 15;

/// Run an AppleScript snippet via `osascript` and return stdout.
pub(crate) async fn run_osascript(script: &str, timeout_secs: u64) -> Result<String, String> {
    debug!("osascript: {}", &script[..script.len().min(120)]);
    let dur = Duration::from_secs(timeout_secs);
    let result = timeout(dur, async {
        Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(format!("osascript error: {stderr}"))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to run osascript: {e}")),
        Err(_) => Err(format!("osascript timed out after {timeout_secs}s")),
    }
}

/// Run a JXA (JavaScript for Automation) snippet via `osascript -l JavaScript`.
pub(crate) async fn run_jxa(script: &str, timeout_secs: u64) -> Result<String, String> {
    debug!("jxa: {}", &script[..script.len().min(120)]);
    let dur = Duration::from_secs(timeout_secs);
    let result = timeout(dur, async {
        Command::new("osascript")
            .arg("-l")
            .arg("JavaScript")
            .arg("-e")
            .arg(script)
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(format!("jxa error: {stderr}"))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to run osascript: {e}")),
        Err(_) => Err(format!("jxa timed out after {timeout_secs}s")),
    }
}

/// Parse a key combo string like "cmd+shift+n" into AppleScript modifier and key parts.
///
/// Returns `(key_char_or_code, modifiers_applescript)` e.g. `("n", "{command down, shift down}")`.
pub(crate) fn parse_key_combo(combo: &str) -> Result<(String, String), String> {
    let parts: Vec<String> = combo.split('+').map(|s| s.trim().to_lowercase()).collect();

    if parts.is_empty() {
        return Err("Empty key combo".to_string());
    }

    let key = parts.last().unwrap().clone();
    let modifiers: Vec<String> = parts[..parts.len() - 1]
        .iter()
        .map(|m| match m.as_str() {
            "cmd" | "command" => Ok("command down".to_string()),
            "ctrl" | "control" => Ok("control down".to_string()),
            "alt" | "option" | "opt" => Ok("option down".to_string()),
            "shift" => Ok("shift down".to_string()),
            other => Err(format!("Unknown modifier: {other}")),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let modifiers_str = if modifiers.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", modifiers.join(", "))
    };

    Ok((key, modifiers_str))
}

/// Map named keys to AppleScript key codes.
pub(crate) fn key_name_to_code(name: &str) -> Option<u8> {
    match name.to_lowercase().as_str() {
        "return" | "enter" => Some(36),
        "tab" => Some(48),
        "space" => Some(49),
        "delete" | "backspace" => Some(51),
        "escape" | "esc" => Some(53),
        "left" => Some(123),
        "right" => Some(124),
        "down" => Some(125),
        "up" => Some(126),
        "f1" => Some(122),
        "f2" => Some(120),
        "f3" => Some(99),
        "f4" => Some(118),
        "f5" => Some(96),
        "f6" => Some(97),
        "f7" => Some(98),
        "f8" => Some(100),
        "f9" => Some(101),
        "f10" => Some(109),
        "f11" => Some(103),
        "f12" => Some(111),
        "home" => Some(115),
        "end" => Some(119),
        "pageup" => Some(116),
        "pagedown" => Some(121),
        "forwarddelete" => Some(117),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_combo_simple_key() {
        let (key, mods) = parse_key_combo("n").unwrap();
        assert_eq!(key, "n");
        assert_eq!(mods, "");
    }

    #[test]
    fn parse_key_combo_with_modifiers() {
        let (key, mods) = parse_key_combo("cmd+shift+n").unwrap();
        assert_eq!(key, "n");
        assert!(mods.contains("command down"));
        assert!(mods.contains("shift down"));
    }

    #[test]
    fn parse_key_combo_unknown_modifier() {
        assert!(parse_key_combo("foo+n").is_err());
    }

    #[test]
    fn key_name_to_code_known_keys() {
        assert_eq!(key_name_to_code("return"), Some(36));
        assert_eq!(key_name_to_code("escape"), Some(53));
        assert_eq!(key_name_to_code("tab"), Some(48));
    }

    #[test]
    fn key_name_to_code_unknown() {
        assert_eq!(key_name_to_code("xyz"), None);
    }
}
