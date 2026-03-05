//! Path utility helpers shared across crates.

/// Expands a leading `~` in `path` to the current user's home directory.
///
/// If `path` does not start with `~`, it is returned unchanged.
/// If the `HOME` environment variable is not set, `.` is used as a fallback.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}{rest}")
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_replaces_home() {
        let result = expand_tilde("~/.openpista/web");
        assert!(!result.starts_with('~'));
        assert!(result.ends_with("/.openpista/web"));
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_unchanged() {
        assert_eq!(expand_tilde("/var/www"), "/var/www");
    }

    #[test]
    fn expand_tilde_leaves_relative_paths_unchanged() {
        assert_eq!(expand_tilde("relative/path"), "relative/path");
    }

    #[test]
    fn expand_tilde_tilde_only() {
        let result = expand_tilde("~");
        assert!(!result.starts_with('~'));
    }
}
