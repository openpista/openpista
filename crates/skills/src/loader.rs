//! Skill loading and execution utilities.

use std::path::{Path, PathBuf};

use proto::ToolResult;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::{debug, info, warn};

/// Skill execution mode selected from `SKILL.md` front-matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SkillExecutionMode {
    /// Execute skill entrypoints such as `run.sh`/`main.py` as subprocesses.
    #[default]
    Subprocess,
    /// Execute a `main.wasm` module with the embedded WASM runtime.
    Wasm,
}

/// Parsed metadata for one skill directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    /// Skill directory name under `<workspace>/skills`.
    pub name: String,
    /// Optional container image hint from front-matter (`image:`).
    pub image: Option<String>,
    /// Optional user-facing description from front-matter.
    pub description: Option<String>,
    /// Execution mode used by runtime dispatch.
    pub mode: SkillExecutionMode,
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontMatter {
    image: Option<String>,
    description: Option<String>,
    mode: Option<String>,
}

/// Loads skills from SKILL.md files in the workspace
pub struct SkillLoader {
    workspace: PathBuf,
}

impl SkillLoader {
    /// Creates a loader rooted at the given workspace path.
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// Creates a loader using `OPENPISTACRAB_WORKSPACE` or default workspace path.
    pub fn from_env_or_default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_workspace = format!("{home}/.openpistacrab/workspace");
        let workspace = std::env::var("OPENPISTACRAB_WORKSPACE").unwrap_or(default_workspace);
        Self::new(workspace)
    }

    /// Load all SKILL.md files and concatenate their content as system prompt context
    pub async fn load_context(&self) -> String {
        let skills_dir = self.workspace.join("skills");

        if !skills_dir.exists() {
            debug!("Skills directory does not exist: {}", skills_dir.display());
            return String::new();
        }

        let mut context = String::new();
        let mut dirs = vec![skills_dir.clone()];

        while let Some(dir) = dirs.pop() {
            match tokio::fs::read_dir(&dir).await {
                Ok(mut entries) => {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let path = entry.path();
                        if path.is_dir() {
                            dirs.push(path);
                            continue;
                        }

                        let is_recursive_skill_file =
                            path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md");
                        let is_top_level_markdown = dir == skills_dir
                            && path.extension().and_then(|e| e.to_str()) == Some("md");

                        if !(is_recursive_skill_file || is_top_level_markdown) {
                            continue;
                        }

                        if let Some(content) = self.read_skill_file(&path).await {
                            let skill_name = if is_recursive_skill_file {
                                path.parent()
                                    .and_then(|p| p.file_name())
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown")
                            } else {
                                path.file_stem()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown")
                            };

                            info!("Loaded skill: {skill_name}");
                            context.push_str(&format!("### Skill: {skill_name}\n\n"));
                            context.push_str(&content);
                            context.push_str("\n\n");
                        } else {
                            warn!("Failed to read skill file: {}", path.display());
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read skills directory {}: {e}", dir.display());
                }
            }
        }

        context
    }

    /// Reads a markdown skill file as UTF-8 text.
    async fn read_skill_file(&self, path: &Path) -> Option<String> {
        tokio::fs::read_to_string(path).await.ok()
    }

    /// Loads parsed skill metadata from `skills/<name>/SKILL.md` or `skills/<name>.md`.
    pub async fn load_skill_metadata(&self, skill_name: &str) -> Option<SkillMetadata> {
        if !is_valid_skill_name(skill_name) {
            warn!("Invalid skill name attempt: {skill_name}");
            return None;
        }

        let skills_dir = self.workspace.join("skills");
        let candidates = [
            skills_dir.join(skill_name).join("SKILL.md"),
            skills_dir.join(format!("{skill_name}.md")),
        ];

        for path in candidates {
            let Some(content) = self.read_skill_file(&path).await else {
                continue;
            };

            let front = parse_skill_front_matter(&content);
            let mode = front
                .as_ref()
                .and_then(|fm| fm.mode.as_deref())
                .map(parse_mode)
                .unwrap_or_default();

            let image = front
                .as_ref()
                .and_then(|fm| fm.image.clone())
                .and_then(normalize_optional_text);

            let description = front
                .as_ref()
                .and_then(|fm| fm.description.clone())
                .and_then(normalize_optional_text);

            return Some(SkillMetadata {
                name: skill_name.to_string(),
                image,
                description,
                mode,
            });
        }

        None
    }

    /// Execute a skill subprocess
    pub async fn run_skill(&self, skill_name: &str, args: &[&str]) -> ToolResult {
        if !is_valid_skill_name(skill_name) {
            warn!("Invalid skill name attempt: {skill_name}");
            return ToolResult::error(
                "skill",
                skill_name,
                format!("Invalid skill name: {skill_name}"),
            );
        }

        let skill_path = self.workspace.join("skills").join(skill_name);

        // Look for executable script
        let (executor, script) = if skill_path.join("run.sh").exists() {
            ("bash", skill_path.join("run.sh"))
        } else if skill_path.join("main.py").exists() {
            ("python", skill_path.join("main.py"))
        } else if skill_path.join("main.sh").exists() {
            ("bash", skill_path.join("main.sh"))
        } else {
            return ToolResult::error(
                "skill",
                skill_name,
                format!(
                    "Skill '{skill_name}' not found in {}",
                    self.workspace.display()
                ),
            );
        };

        let executors = if executor == "python" {
            vec!["python", "python3"]
        } else {
            vec![executor]
        };

        match run_with_executor_candidates(&executors, &script, args, &skill_path).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\nstderr: {stderr}")
                };

                if exit_code == 0 {
                    ToolResult::success("skill", skill_name, combined)
                } else {
                    ToolResult::error("skill", skill_name, combined)
                }
            }
            Err(e) => ToolResult::error("skill", skill_name, format!("Failed to run skill: {e}")),
        }
    }

    /// Returns the configured workspace root path.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
}

fn parse_skill_front_matter(content: &str) -> Option<SkillFrontMatter> {
    let yaml = extract_front_matter(content)?;
    serde_yaml::from_str::<SkillFrontMatter>(&yaml).ok()
}

fn extract_front_matter(content: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut yaml_lines = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            return Some(yaml_lines.join("\n"));
        }
        yaml_lines.push(line);
    }

    None
}

fn parse_mode(value: &str) -> SkillExecutionMode {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "wasm" => SkillExecutionMode::Wasm,
        _ => {
            warn!(
                value = %value,
                normalized = %normalized,
                "unknown skill execution mode, defaulting to subprocess"
            );
            SkillExecutionMode::Subprocess
        }
    }
}

fn normalize_optional_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Executes a script by trying executors in order until one launches successfully.
async fn run_with_executor_candidates(
    executors: &[&str],
    script: &Path,
    args: &[&str],
    current_dir: &Path,
) -> std::io::Result<std::process::Output> {
    let mut last_err: Option<std::io::Error> = None;

    for executor in executors {
        let mut cmd = Command::new(executor);
        cmd.arg(script);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.current_dir(current_dir);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        match cmd.output().await {
            Ok(output) => return Ok(output),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No executable candidate could be started",
        )
    }))
}

fn is_valid_skill_name(name: &str) -> bool {
    let path = Path::new(name);
    let mut components = path.components();
    let Some(std::path::Component::Normal(valid_name)) = components.next() else {
        return false;
    };
    if components.next().is_some() {
        return false; // must be exactly one component
    }
    valid_name.to_str() == Some(name)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, content).expect("write file");
    }

    #[tokio::test]
    async fn load_context_returns_empty_when_skills_dir_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let loader = SkillLoader::new(tmp.path());
        let ctx = loader.load_context().await;
        assert!(ctx.is_empty());
    }

    #[tokio::test]
    async fn load_context_reads_top_level_and_nested_skill_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skills_dir = tmp.path().join("skills");
        write_file(&skills_dir.join("quick.md"), "# Quick\nUse this");
        write_file(&skills_dir.join("shell/SKILL.md"), "# Shell\nDo commands");
        write_file(
            &skills_dir.join("automation/browser/login/SKILL.md"),
            "# Login\nDeep nested skill",
        );

        let loader = SkillLoader::new(tmp.path());
        let ctx = loader.load_context().await;
        assert!(ctx.contains("### Skill: quick"));
        assert!(ctx.contains("Use this"));
        assert!(ctx.contains("### Skill: shell"));
        assert!(ctx.contains("Do commands"));
        assert!(ctx.contains("### Skill: login"));
        assert!(ctx.contains("Deep nested skill"));
    }

    #[tokio::test]
    async fn load_skill_metadata_returns_none_when_skill_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let loader = SkillLoader::new(tmp.path());
        let metadata = loader.load_skill_metadata("missing").await;
        assert!(metadata.is_none());
    }

    #[tokio::test]
    async fn load_skill_metadata_parses_front_matter_image() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skill_file = tmp.path().join("skills/py/SKILL.md");
        write_file(
            &skill_file,
            "---\nimage: python:3.12-slim\n---\n# Python\nRuns scripts",
        );

        let loader = SkillLoader::new(tmp.path());
        let metadata = loader
            .load_skill_metadata("py")
            .await
            .expect("metadata present");

        assert_eq!(metadata.name, "py");
        assert_eq!(metadata.image.as_deref(), Some("python:3.12-slim"));
        assert!(metadata.description.is_none());
        assert_eq!(metadata.mode, SkillExecutionMode::Subprocess);
    }

    #[tokio::test]
    async fn load_skill_metadata_handles_front_matter_without_image() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skill_file = tmp.path().join("skills/no-image/SKILL.md");
        write_file(
            &skill_file,
            "---\ndescription: test\n---\n# Skill\nNo image declared",
        );

        let loader = SkillLoader::new(tmp.path());
        let metadata = loader
            .load_skill_metadata("no-image")
            .await
            .expect("metadata present");

        assert_eq!(metadata.name, "no-image");
        assert!(metadata.image.is_none());
        assert_eq!(metadata.description.as_deref(), Some("test"));
        assert_eq!(metadata.mode, SkillExecutionMode::Subprocess);
    }

    #[tokio::test]
    async fn load_skill_metadata_handles_markdown_without_front_matter() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skill_file = tmp.path().join("skills/plain/SKILL.md");
        write_file(&skill_file, "# Plain\nNo front matter");

        let loader = SkillLoader::new(tmp.path());
        let metadata = loader
            .load_skill_metadata("plain")
            .await
            .expect("metadata present");

        assert_eq!(metadata.name, "plain");
        assert!(metadata.image.is_none());
        assert!(metadata.description.is_none());
        assert_eq!(metadata.mode, SkillExecutionMode::Subprocess);
    }

    #[tokio::test]
    async fn run_skill_returns_not_found_for_missing_skill() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let loader = SkillLoader::new(tmp.path());
        let result = loader.run_skill("missing", &[]).await;
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn run_skill_executes_run_sh_script() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run_path = tmp.path().join("skills/echo/run.sh");
        write_file(&run_path, "echo script:$1");

        let loader = SkillLoader::new(tmp.path());
        let result = loader.run_skill("echo", &["ok"]).await;
        assert!(!result.is_error);
        assert!(result.output.contains("script:ok"));
    }

    #[tokio::test]
    async fn run_skill_executes_main_py_script_with_python() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let py_path = tmp.path().join("skills/echo/main.py");
        write_file(&py_path, "import sys\nprint(f'py:{sys.argv[1]}')");

        let loader = SkillLoader::new(tmp.path());
        let result = loader.run_skill("echo", &["ok"]).await;
        assert!(!result.is_error);
        assert!(result.output.contains("py:ok"));
    }

    #[tokio::test]
    async fn run_skill_reports_non_zero_exit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run_path = tmp.path().join("skills/fail/run.sh");
        write_file(&run_path, "echo bad 1>&2\nexit 2");

        let loader = SkillLoader::new(tmp.path());
        let result = loader.run_skill("fail", &[]).await;
        assert!(result.is_error);
        assert!(result.output.contains("stderr: bad"));
    }

    #[test]
    fn workspace_accessor_returns_loader_workspace() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let loader = SkillLoader::new(tmp.path());
        assert_eq!(loader.workspace(), tmp.path());
    }

    #[tokio::test]
    async fn load_skill_metadata_parses_mode_and_image_from_front_matter() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skill_md = tmp.path().join("skills/hello-wasm/SKILL.md");
        write_file(
            &skill_md,
            "---\nmode: wasm\nimage: ghcr.io/openpista/wasm:latest\ndescription: hello wasm\n---\n# hello\n",
        );

        let loader = SkillLoader::new(tmp.path());
        let metadata = loader
            .load_skill_metadata("hello-wasm")
            .await
            .expect("metadata");

        assert_eq!(metadata.name, "hello-wasm");
        assert_eq!(metadata.mode, SkillExecutionMode::Wasm);
        assert_eq!(
            metadata.image.as_deref(),
            Some("ghcr.io/openpista/wasm:latest")
        );
        assert_eq!(metadata.description.as_deref(), Some("hello wasm"));
    }

    #[tokio::test]
    async fn load_skill_metadata_defaults_mode_when_missing_or_invalid() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let a = tmp.path().join("skills/a/SKILL.md");
        let b = tmp.path().join("skills/b/SKILL.md");
        write_file(&a, "---\nimage: alpine:3.20\n---\n# a\n");
        write_file(&b, "---\nmode: unknown\n---\n# b\n");

        let loader = SkillLoader::new(tmp.path());
        let a_md = loader.load_skill_metadata("a").await.expect("a metadata");
        let b_md = loader.load_skill_metadata("b").await.expect("b metadata");

        assert_eq!(a_md.mode, SkillExecutionMode::Subprocess);
        assert_eq!(b_md.mode, SkillExecutionMode::Subprocess);
    }

    #[test]
    fn parse_mode_recognizes_wasm_and_defaults_other_values() {
        assert_eq!(parse_mode("wasm"), SkillExecutionMode::Wasm);
        assert_eq!(parse_mode("WASM"), SkillExecutionMode::Wasm);
        assert_eq!(parse_mode("subprocess"), SkillExecutionMode::Subprocess);
        assert_eq!(parse_mode(""), SkillExecutionMode::Subprocess);
    }
}
