//! Shared utility functions for tool output formatting.

/// Truncates UTF-8 text to `max_chars` code points and appends a suffix when truncated.
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}\n[... output truncated at {max_chars} chars]")
    }
}

/// Formats command stdout/stderr and exit code into a single text payload.
pub fn format_output(stdout: &str, stderr: &str, exit_code: i64) -> String {
    let mut out = String::new();

    if !stdout.is_empty() {
        out.push_str("stdout:\n");
        out.push_str(stdout);
        if !stdout.ends_with('\n') {
            out.push('\n');
        }
    }

    if !stderr.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("stderr:\n");
        out.push_str(stderr);
        if !stderr.ends_with('\n') {
            out.push('\n');
        }
    }

    out.push_str(&format!("\nexit_code: {exit_code}"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_str_keeps_short_text() {
        assert_eq!(truncate_str("abc", 5), "abc");
    }

    #[test]
    fn truncate_str_exact_boundary() {
        assert_eq!(truncate_str("abc", 3), "abc");
    }

    #[test]
    fn truncate_str_empty_input() {
        assert_eq!(truncate_str("", 10), "");
    }

    #[test]
    fn truncate_str_adds_suffix_for_long_text() {
        let out = truncate_str("abcdef", 3);
        assert!(out.starts_with("abc"));
        assert!(out.contains("output truncated"));
    }

    #[test]
    fn format_output_renders_all_sections() {
        let out = format_output("ok\n", "warn\n", 2);
        assert!(out.contains("stdout:\nok"));
        assert!(out.contains("stderr:\nwarn"));
        assert!(out.contains("exit_code: 2"));
    }

    #[test]
    fn format_output_stdout_only() {
        let out = format_output("hello\n", "", 0);
        assert!(out.contains("stdout:\nhello"));
        assert!(!out.contains("stderr:"));
        assert!(out.contains("exit_code: 0"));
    }

    #[test]
    fn format_output_stderr_only() {
        let out = format_output("", "error\n", 1);
        assert!(!out.contains("stdout:"));
        assert!(out.contains("stderr:\nerror"));
        assert!(out.contains("exit_code: 1"));
    }

    #[test]
    fn format_output_empty_both() {
        let out = format_output("", "", 0);
        assert!(!out.contains("stdout:"));
        assert!(!out.contains("stderr:"));
        assert!(out.contains("exit_code: 0"));
    }

    #[test]
    fn format_output_appends_newline_when_missing() {
        let out = format_output("no-newline", "also-no-newline", 0);
        assert!(out.contains("stdout:\nno-newline\n"));
        assert!(out.contains("stderr:\nalso-no-newline\n"));
    }
}
