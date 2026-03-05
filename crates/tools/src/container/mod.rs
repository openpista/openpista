//! Container execution tool powered by Docker Engine API.

pub mod docker;
pub mod lifecycle;
pub mod skill;

use async_trait::async_trait;
use bollard::Docker;
use proto::ToolResult;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

use crate::Tool;
use crate::wasm_runtime::{WasmRunRequest, run_wasm_skill};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;
const DEFAULT_TOKEN_TTL_SECS: u64 = 300;
const MAX_TOKEN_TTL_SECS: u64 = 900;
const DEFAULT_MEMORY_MB: i64 = 512;
const DEFAULT_CPU_MILLIS: i64 = 1000;
const MAX_OUTPUT_CHARS: usize = 10_000;
const TOKEN_ENV_FILE_NAME: &str = ".openpista_task_env";
const TOKEN_MOUNT_DIR: &str = "/run/secrets";
const DEFAULT_TOKEN_ENV_NAME: &str = "openpista_TASK_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeMode {
    Docker,
    Wasm,
}

/// Container/WASM sandbox execution tool (`container.run`).
pub struct ContainerTool;

#[derive(Debug, Deserialize)]
struct ContainerArgs {
    #[serde(default)]
    image: Option<String>,
    skill_image: Option<String>,
    command: Option<String>,
    skill_name: Option<String>,
    skill_args: Option<serde_json::Value>,
    timeout_secs: Option<u64>,
    working_dir: Option<String>,
    env: Option<Vec<String>>,
    allow_network: Option<bool>,
    workspace_dir: Option<String>,
    memory_mb: Option<i64>,
    cpu_millis: Option<i64>,
    pull: Option<bool>,
    inject_task_token: Option<bool>,
    token_ttl_secs: Option<u64>,
    token_env_name: Option<String>,
    allow_subprocess_fallback: Option<bool>,
}

#[derive(Debug, Clone)]
struct TaskCredential {
    token: String,
    expires_at_unix: u64,
    env_name: String,
}

#[derive(Debug, Clone)]
struct ContainerExecution {
    stdout: String,
    stderr: String,
    exit_code: i64,
}

impl ContainerTool {
    /// Constructs a new ContainerTool.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let tool = ContainerTool::new();
    /// ```
    pub fn new() -> Self {
        Self
    }
}

impl Default for ContainerTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ContainerTool {
    fn name(&self) -> &str {
        "container.run"
    }

    fn description(&self) -> &str {
        "Run a command in an isolated Docker container and return stdout, stderr, and exit code"
    }

    /// Provide the JSON Schema describing valid parameters for the container tool.
    ///
    /// The schema enumerates all accepted top-level properties (image, command, skill-related
    /// fields, resource limits, credential options, etc.), sets no required fields,
    /// and disallows additional properties.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let tool = ContainerTool::new();
    /// let schema = tool.parameters_schema();
    /// // basic sanity check: schema contains a "properties" object
    /// assert!(schema.get("properties").is_some());
    /// ```
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image": {
                    "type": "string",
                    "description": "Container image name (for example: alpine:3.20); when omitted, skill_image can be used. Optional for wasm mode"
                },
                "skill_image": {
                    "type": "string",
                    "description": "Skill-provided default image from SKILL.md front matter (used when image is omitted)"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute in the container. Optional for wasm mode"
                },
                "skill_name": {
                    "type": "string",
                    "description": "Skill directory name under <workspace>/skills to resolve SKILL.md mode"
                },
                "skill_args": {
                    "description": "JSON arguments forwarded to wasm skill ABI as ToolCall.arguments"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300)"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory inside the container"
                },
                "env": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Environment variables in KEY=VALUE format"
                },
                "allow_network": {
                    "type": "boolean",
                    "description": "Allow network access from the container (default: false)"
                },
                "workspace_dir": {
                    "type": "string",
                    "description": "Host path to mount read-only at /workspace"
                },
                "memory_mb": {
                    "type": "integer",
                    "description": "Memory limit in MB (default: 512)"
                },
                "cpu_millis": {
                    "type": "integer",
                    "description": "CPU limit in millicores (default: 1000)"
                },
                "pull": {
                    "type": "boolean",
                    "description": "Pull image before run (default: false)"
                },
                "inject_task_token": {
                    "type": "boolean",
                    "description": "Inject a short-lived per-task credential via tmpfs file (default: false)"
                },
                "token_ttl_secs": {
                    "type": "integer",
                    "description": "TTL for injected task credential in seconds (default: 300, max: 900)"
                },
                "token_env_name": {
                    "type": "string",
                    "description": "Environment variable name exposed by injected credential file (default: openpista_TASK_TOKEN)"
                },
                "allow_subprocess_fallback": {
                    "type": "boolean",
                    "description": "Fallback to local subprocess when Docker is unavailable (default: false)"
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    /// Execute a container command or run a WASM skill based on the provided JSON arguments, and return a ToolResult containing either the formatted execution output or an error message.
    ///
    /// The function:
    /// - Deserializes `args` into `ContainerArgs`.
    /// - Resolves the runtime mode; if the skill metadata indicates WASM mode, it requires `workspace_dir` and `skill_name` and runs the skill via the WASM runtime, returning the skill's output.
    /// - Otherwise, validates and resolves the container image and command, runs the workload in Docker (or falls back to a local subprocess if allowed), and returns the combined stdout/stderr and exit code.
    ///
    /// Parameters:
    /// - `call_id`: identifier used for container naming, reporting and correlating the execution.
    /// - `args`: a JSON value conforming to `ContainerArgs` specifying image, command, WASM skill fields, resource limits, and reporting/fallback options.
    ///
    /// Returns:
    /// - On success: a `ToolResult::success` containing the formatted stdout, stderr and exit code.
    /// - On failure: a `ToolResult::error` containing a diagnostic message describing the failure.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// # use serde_json::json;
    /// # use crate::container::ContainerTool;
    /// # tokio_test::block_on(async {
    /// let tool = ContainerTool::new();
    /// let args = json!({
    ///     "image": "alpine:latest",
    ///     "command": "echo hello",
    ///     "timeout_secs": 10
    /// });
    /// let res = tool.execute("call-123", args).await;
    /// // inspect `res` for success or error
    /// # });
    /// ```
    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let mut parsed: ContainerArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let runtime_mode = match skill::resolve_runtime_mode(&parsed).await {
            Ok(mode) => mode,
            Err(e) => return ToolResult::error(call_id, self.name(), e),
        };

        if runtime_mode == RuntimeMode::Wasm {
            let workspace_dir = parsed
                .workspace_dir
                .as_deref()
                .and_then(non_empty)
                .map(PathBuf::from);
            let Some(workspace_dir) = workspace_dir else {
                return ToolResult::error(
                    call_id,
                    self.name(),
                    "workspace_dir is required when running mode: wasm",
                );
            };
            let Some(skill_name) = parsed
                .skill_name
                .as_deref()
                .and_then(non_empty)
                .map(|s| s.to_string())
            else {
                return ToolResult::error(
                    call_id,
                    self.name(),
                    "skill_name is required when running mode: wasm",
                );
            };

            let request = WasmRunRequest {
                call_id: call_id.to_string(),
                skill_name,
                workspace_dir,
                arguments: parsed
                    .skill_args
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
                timeout_secs: parsed.timeout_secs,
            };

            return match run_wasm_skill(request).await {
                Ok(result) if result.is_error => {
                    ToolResult::error(call_id, self.name(), result.output)
                }
                Ok(result) => ToolResult::success(call_id, self.name(), result.output),
                Err(e) => ToolResult::error(call_id, self.name(), e),
            };
        }

        if parsed.command.as_deref().and_then(non_empty).is_none() {
            return ToolResult::error(call_id, self.name(), "command must not be empty");
        }

        let image_opt = resolve_image(&parsed);
        if image_opt.is_none() {
            return ToolResult::error(call_id, self.name(), "image must not be empty");
        }

        if let Some(image) = image_opt {
            parsed.image = Some(image);
        }

        let timeout_duration = Duration::from_secs(
            parsed
                .timeout_secs
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .clamp(1, MAX_TIMEOUT_SECS),
        );

        let container_name = build_container_name(call_id);

        let docker_result = Docker::connect_with_local_defaults()
            .map_err(|e| format!("Failed to connect to Docker daemon: {e}"));
        let execution_result = lifecycle::run_with_docker_or_subprocess(
            call_id,
            &parsed,
            timeout_duration,
            &container_name,
            docker_result,
        )
        .await;

        match execution_result {
            Ok(execution) => ToolResult::success(
                call_id,
                self.name(),
                format_output(&execution.stdout, &execution.stderr, execution.exit_code),
            ),
            Err(e) => ToolResult::error(call_id, self.name(), e),
        }
    }
}

/// Trim whitespace and yield the input slice when it contains non-empty content.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(non_empty("  hello  "), Some("hello"));
/// assert_eq!(non_empty("   "), None);
/// assert_eq!(non_empty("world"), Some("world"));
/// ```
fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Selects the container image to use from the provided arguments.
///
/// Prefers an explicit `args.image` when present and non-empty (after trimming);
/// otherwise falls back to `args.skill_image` if that is present and non-empty.
///
/// # Returns
///
/// `Some(String)` containing the chosen image (trimmed), or `None` if neither
/// field contains a non-empty value.
///
/// # Examples
///
/// ```ignore
/// let args = ContainerArgs {
///     image: Some("  alpine:3.18  ".into()),
///     skill_image: Some("fallback:latest".into()),
///     ..Default::default()
/// };
/// assert_eq!(resolve_image(&args), Some("alpine:3.18".to_string()));
/// ```
fn resolve_image(args: &ContainerArgs) -> Option<String> {
    if let Some(explicit) = args.image.as_deref().map(str::trim)
        && !explicit.is_empty()
    {
        return Some(explicit.to_string());
    }

    args.skill_image
        .as_deref()
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .map(ToOwned::to_owned)
}

fn build_container_name(call_id: &str) -> String {
    let suffix: String = call_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    format!("openpista-{}", suffix)
}

fn format_output(stdout: &str, stderr: &str, exit_code: i64) -> String {
    let stdout = crate::util::truncate_str(stdout, MAX_OUTPUT_CHARS / 2);
    let stderr = crate::util::truncate_str(stderr, MAX_OUTPUT_CHARS / 2);

    let mut out = String::new();

    if !stdout.is_empty() {
        out.push_str("stdout:\n");
        out.push_str(&stdout);
        if !stdout.ends_with('\n') {
            out.push('\n');
        }
    }

    if !stderr.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("stderr:\n");
        out.push_str(&stderr);
        if !stderr.ends_with('\n') {
            out.push('\n');
        }
    }

    out.push_str(&format!("\nexit_code: {exit_code}"));
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tokio::process::Command;

    use super::docker::build_task_credential_archive;
    use super::lifecycle::{
        build_shell_command, build_task_credential_script, cleanup_task_credential,
        is_valid_env_name, maybe_mint_task_credential, run_as_subprocess,
        run_with_docker_or_subprocess, sanitize_env_name, shell_single_quote, unix_now_secs,
    };
    use super::skill::resolve_runtime_mode;
    use super::*;

    /// Writes `content` to `path`, creating parent directories if they do not exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// let tmp = tempfile::tempdir().unwrap();
    /// let file_path = tmp.path().join("sub/dir/file.txt");
    /// write_file(&file_path, "hello");
    /// assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello");
    /// ```
    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, content).expect("write file");
    }

    /// Create a minimal ContainerArgs preset for tests with the given shell command and a default image of `alpine:3`.
    ///
    /// The returned struct has `command` set to the provided value, `image` set to `"alpine:3"`, and all other optional fields left as `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// let args = base_args("echo hello");
    /// assert_eq!(args.image.as_deref(), Some("alpine:3"));
    /// assert_eq!(args.command.as_deref(), Some("echo hello"));
    /// ```
    fn base_args(command: &str) -> ContainerArgs {
        ContainerArgs {
            image: Some("alpine:3".to_string()),
            skill_image: None,
            command: Some(command.to_string()),
            skill_name: None,
            skill_args: None,
            timeout_secs: None,
            working_dir: None,
            env: None,
            allow_network: None,
            workspace_dir: None,
            memory_mb: None,
            cpu_millis: None,
            pull: None,
            inject_task_token: None,
            token_ttl_secs: None,
            token_env_name: None,
            allow_subprocess_fallback: None,
        }
    }

    #[test]
    fn resolve_image_prefers_explicit_and_falls_back_to_skill_image() {
        let explicit = ContainerArgs {
            image: Some("alpine:3.20".to_string()),
            skill_image: Some("python:3.12-slim".to_string()),
            ..base_args("echo ok")
        };
        assert_eq!(resolve_image(&explicit).as_deref(), Some("alpine:3.20"));

        let fallback = ContainerArgs {
            image: Some(String::new()),
            skill_image: Some("python:3.12-slim".to_string()),
            ..base_args("echo ok")
        };
        assert_eq!(
            resolve_image(&fallback).as_deref(),
            Some("python:3.12-slim")
        );

        let missing = ContainerArgs {
            image: Some(String::new()),
            skill_image: None,
            ..base_args("echo ok")
        };
        assert!(resolve_image(&missing).is_none());
    }

    #[tokio::test]
    async fn run_as_subprocess_returns_output_for_successful_command() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut args = base_args("printf '%s:%s' \"$PWD\" \"$MSG\"");
        args.working_dir = Some(tmp.path().to_string_lossy().to_string());
        args.env = Some(vec!["MSG=hello".to_string()]);

        let execution = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect("subprocess should succeed");

        assert_eq!(execution.exit_code, 0);
        assert!(
            execution
                .stdout
                .contains(tmp.path().to_string_lossy().as_ref())
        );
        assert!(execution.stdout.contains("hello"));
        assert!(execution.stderr.is_empty());
    }

    #[tokio::test]
    async fn run_as_subprocess_captures_non_zero_exit() {
        let args = base_args("echo failed 1>&2; exit 7");
        let execution = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect("subprocess should run");

        assert_eq!(execution.exit_code, 7);
        assert!(execution.stderr.contains("failed"));
    }

    #[tokio::test]
    async fn run_as_subprocess_returns_timeout_error() {
        let args = base_args("sleep 2");
        let err = run_as_subprocess(&args, Duration::from_millis(100))
            .await
            .expect_err("subprocess should time out");

        assert!(err.contains("timed out"));
    }

    #[tokio::test]
    async fn docker_unavailable_uses_subprocess_fallback_when_enabled() {
        let mut args = base_args("printf fallback-ok");
        args.image = Some(String::new());
        args.allow_subprocess_fallback = Some(true);

        let execution = run_with_docker_or_subprocess(
            "call-fallback",
            &args,
            Duration::from_secs(2),
            "ctr",
            Err("docker down".to_string()),
        )
        .await
        .expect("fallback execution should succeed");

        assert_eq!(execution.exit_code, 0);
        assert!(execution.stdout.contains("fallback-ok"));
    }

    #[tokio::test]
    async fn docker_unavailable_returns_error_when_fallback_disabled() {
        let mut args = base_args("echo ignored");
        args.allow_subprocess_fallback = Some(false);

        let err = run_with_docker_or_subprocess(
            "call-no-fallback",
            &args,
            Duration::from_secs(2),
            "ctr",
            Err("docker down".to_string()),
        )
        .await
        .expect_err("docker connection error should be returned");

        assert!(err.contains("docker down"));
    }

    #[tokio::test]
    async fn docker_unavailable_returns_error_when_fallback_is_not_set() {
        let mut args = base_args("echo ignored");
        args.allow_subprocess_fallback = None;

        let err = run_with_docker_or_subprocess(
            "call-default-no-fallback",
            &args,
            Duration::from_secs(2),
            "ctr",
            Err("docker down".to_string()),
        )
        .await
        .expect_err("docker connection error should be returned by default");

        assert!(err.contains("docker down"));
    }

    #[test]
    fn container_tool_metadata_is_stable() {
        let tool = ContainerTool::new();
        assert_eq!(tool.name(), "container.run");
        assert!(tool.description().contains("Docker"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(
            schema["required"]
                .as_array()
                .expect("required array")
                .is_empty()
        );
        assert!(schema["properties"]["skill_image"].is_object());
        assert!(schema["properties"]["allow_subprocess_fallback"].is_object());
    }

    #[tokio::test]
    async fn execute_rejects_non_object_arguments() {
        let tool = ContainerTool::new();
        let result = tool.execute("call-1", serde_json::json!("bad")).await;

        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn execute_rejects_empty_required_fields() {
        let tool = ContainerTool::new();
        let result = tool
            .execute(
                "call-2",
                serde_json::json!({"image":"","command":"", "timeout_secs": 1}),
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("command must not be empty"));
    }

    #[tokio::test]
    async fn execute_with_valid_arguments_returns_result_shape() {
        let tool = ContainerTool::new();
        let result = tool
            .execute(
                "call-2b",
                serde_json::json!({
                    "image":"alpine:3.20",
                    "command":"echo hi",
                    "timeout_secs":1,
                    "pull":false
                }),
            )
            .await;

        assert_eq!(result.call_id, "call-2b");
        assert_eq!(result.tool_name, "container.run");
        assert!(!result.output.is_empty());
    }

    /// Verifies that a skill with SKILL.md declaring `mode: wasm` is resolved to `RuntimeMode::Wasm`.
    ///
    /// # Examples
    ///
    /// ```
    /// // Given a workspace containing "skills/hello/SKILL.md" with front matter `mode: wasm`,
    /// // calling `resolve_runtime_mode(&args).await` returns `RuntimeMode::Wasm`.
    /// ```
    #[tokio::test]
    async fn resolve_runtime_mode_detects_wasm_skill_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_file(
            &tmp.path().join("skills/hello/SKILL.md"),
            "---\nmode: wasm\n---\n# hello\n",
        );

        let args = ContainerArgs {
            image: None,
            skill_image: None,
            command: None,
            skill_name: Some("hello".to_string()),
            skill_args: Some(serde_json::json!({"name":"openpista"})),
            timeout_secs: Some(1),
            working_dir: None,
            env: None,
            allow_network: None,
            workspace_dir: Some(tmp.path().to_string_lossy().to_string()),
            memory_mb: None,
            cpu_millis: None,
            pull: None,
            inject_task_token: None,
            token_ttl_secs: None,
            token_env_name: None,
            allow_subprocess_fallback: None,
        };

        let mode = resolve_runtime_mode(&args).await.expect("mode");
        assert_eq!(mode, RuntimeMode::Wasm);
    }

    #[tokio::test]
    async fn resolve_runtime_mode_defaults_to_docker_for_subprocess_skill() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_file(
            &tmp.path().join("skills/shell/SKILL.md"),
            "---\nmode: subprocess\n---\n# shell\n",
        );

        let args = ContainerArgs {
            image: Some("alpine:3.20".to_string()),
            skill_image: None,
            command: Some("echo ok".to_string()),
            skill_name: Some("shell".to_string()),
            skill_args: None,
            timeout_secs: None,
            working_dir: None,
            env: None,
            allow_network: None,
            workspace_dir: Some(tmp.path().to_string_lossy().to_string()),
            memory_mb: None,
            cpu_millis: None,
            pull: None,
            inject_task_token: None,
            token_ttl_secs: None,
            token_env_name: None,
            allow_subprocess_fallback: None,
        };

        let mode = resolve_runtime_mode(&args).await.expect("mode");
        assert_eq!(mode, RuntimeMode::Docker);
    }

    #[tokio::test]
    async fn execute_requires_workspace_for_wasm_skill_mode() {
        let tool = ContainerTool::new();
        let result = tool
            .execute(
                "call-3",
                serde_json::json!({"skill_name":"hello", "skill_args": {"x": 1}}),
            )
            .await;

        assert!(result.is_error);
        assert!(
            result
                .output
                .contains("workspace_dir is required when skill_name is provided")
        );
    }

    #[tokio::test]
    async fn execute_wasm_skill_path_end_to_end() {
        if std::env::var_os("LLVM_PROFILE_FILE").is_some() {
            eprintln!("skipping wasm e2e under coverage instrumentation");
            return;
        }

        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let skill_dir = repo_root.join("skills/hello-wasm");
        let manifest_path = skill_dir.join("Cargo.toml");
        assert!(manifest_path.exists(), "hello-wasm manifest must exist");
        let build_target = tempfile::tempdir().expect("build target tempdir");

        let build = Command::new("cargo")
            .current_dir(&skill_dir)
            .env("CARGO_TARGET_DIR", build_target.path())
            // Avoid inheriting coverage instrumentation flags when building the wasm fixture.
            .env_remove("RUSTFLAGS")
            .env_remove("RUSTDOCFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("LLVM_PROFILE_FILE")
            .args(["build", "--target", "wasm32-wasip1", "--release"])
            .output()
            .await
            .expect("build hello-wasm");

        if !build.status.success() {
            let stderr = String::from_utf8_lossy(&build.stderr);
            if stderr.contains("wasm32-wasip1")
                && (stderr.contains("target may not be installed")
                    || stderr.contains("can't find crate for `std`")
                    || stderr.contains("rustup target add wasm32-wasip1"))
            {
                eprintln!("skipping wasm e2e: wasm32-wasip1 target unavailable");
                return;
            }
            panic!(
                "failed to build hello-wasm fixture:\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&build.stdout),
                stderr
            );
        }

        let built_wasm = build_target
            .path()
            .join("wasm32-wasip1/release/hello_wasm.wasm");
        assert!(built_wasm.exists(), "compiled wasm fixture should exist");

        let tmp = tempfile::tempdir().expect("tempdir");
        let tmp_skill_dir = tmp.path().join("skills/hello-wasm");
        std::fs::create_dir_all(&tmp_skill_dir).expect("create temp skill dir");
        std::fs::copy(skill_dir.join("SKILL.md"), tmp_skill_dir.join("SKILL.md"))
            .expect("copy skill metadata");
        std::fs::copy(&built_wasm, tmp_skill_dir.join("main.wasm")).expect("copy wasm module");

        let tool = ContainerTool::new();
        let result = tool
            .execute(
                "call-wasm-e2e",
                serde_json::json!({
                    "skill_name": "hello-wasm",
                    "workspace_dir": tmp.path().to_string_lossy().to_string(),
                    "skill_args": {"name": "openpista"},
                    "timeout_secs": 5
                }),
            )
            .await;

        assert!(!result.is_error, "result output: {}", result.output);
        assert_eq!(result.call_id, "call-wasm-e2e");
        assert_eq!(result.tool_name, "container.run");
        assert_eq!(result.output, "hello from wasm, openpista");
    }

    #[test]
    fn build_container_name_sanitizes_input() {
        let name = build_container_name("call:1/abc");
        assert_eq!(name, "openpista-call-1-abc");
    }

    #[test]
    fn format_output_renders_all_sections() {
        let out = format_output("ok\n", "warn\n", 7);
        assert!(out.contains("stdout:\nok"));
        assert!(out.contains("stderr:\nwarn"));
        assert!(out.contains("exit_code: 7"));
    }

    #[test]
    fn maybe_mint_task_credential_returns_none_when_disabled() {
        let args = ContainerArgs {
            image: Some("alpine:3".to_string()),
            skill_image: None,
            command: Some("echo hi".to_string()),
            skill_name: None,
            skill_args: None,
            timeout_secs: None,
            working_dir: None,
            env: None,
            allow_network: None,
            workspace_dir: None,
            memory_mb: None,
            cpu_millis: None,
            pull: None,
            inject_task_token: Some(false),
            token_ttl_secs: None,
            token_env_name: None,
            allow_subprocess_fallback: None,
        };

        let credential = maybe_mint_task_credential(&args).expect("mint result");
        assert!(credential.is_none());
    }

    #[test]
    fn maybe_mint_task_credential_validates_env_name() {
        let args = ContainerArgs {
            image: Some("alpine:3".to_string()),
            skill_image: None,
            command: Some("echo hi".to_string()),
            skill_name: None,
            skill_args: None,
            timeout_secs: None,
            working_dir: None,
            env: None,
            allow_network: None,
            workspace_dir: None,
            memory_mb: None,
            cpu_millis: None,
            pull: None,
            inject_task_token: Some(true),
            token_ttl_secs: Some(120),
            token_env_name: Some("9BAD".to_string()),
            allow_subprocess_fallback: None,
        };

        let err = maybe_mint_task_credential(&args).expect_err("invalid env name should fail");
        assert!(err.contains("token_env_name"));
    }

    #[test]
    fn build_shell_command_sources_credential_file() {
        let credential = TaskCredential {
            token: "tok".to_string(),
            expires_at_unix: 1,
            env_name: DEFAULT_TOKEN_ENV_NAME.to_string(),
        };

        let cmd = build_shell_command("echo hi", Some(&credential));
        assert!(cmd.contains(TOKEN_ENV_FILE_NAME));
        assert!(cmd.contains("echo hi"));
    }

    #[test]
    fn build_task_credential_script_contains_exports() {
        let credential = TaskCredential {
            token: "abc123".to_string(),
            expires_at_unix: 42,
            env_name: "openpista_TASK_TOKEN".to_string(),
        };

        let script = build_task_credential_script(&credential);
        assert!(script.contains("export openpista_TASK_TOKEN='abc123'"));
        assert!(script.contains("openpista_TASK_TOKEN_EXPIRES_AT=42"));
    }

    #[test]
    fn sanitize_env_name_uses_default_and_rejects_invalid_values() {
        assert_eq!(
            sanitize_env_name(None).expect("default env name"),
            DEFAULT_TOKEN_ENV_NAME
        );
        assert_eq!(
            sanitize_env_name(Some("CUSTOM_TOKEN")).expect("custom env name"),
            "CUSTOM_TOKEN"
        );
        assert!(sanitize_env_name(Some("BAD-NAME")).is_err());
        assert!(sanitize_env_name(Some("9BAD")).is_err());
    }

    #[test]
    fn is_valid_env_name_checks_boundaries() {
        assert!(is_valid_env_name("_TOKEN"));
        assert!(is_valid_env_name("TOKEN_1"));
        assert!(!is_valid_env_name(""));
        assert!(!is_valid_env_name("1TOKEN"));
        assert!(!is_valid_env_name("TOKEN-NAME"));
    }

    #[test]
    fn shell_single_quote_escapes_embedded_quotes() {
        let value = "abc'def";
        let quoted = shell_single_quote(value);
        assert_eq!(quoted, "'abc'\"'\"'def'");
    }

    #[test]
    fn build_task_credential_archive_contains_env_file() {
        let credential = TaskCredential {
            token: "tok".to_string(),
            expires_at_unix: 7,
            env_name: DEFAULT_TOKEN_ENV_NAME.to_string(),
        };

        let archive = build_task_credential_archive(&credential).expect("archive");
        let mut ar = tar::Archive::new(std::io::Cursor::new(archive));
        let mut names = Vec::new();
        for entry in ar.entries().expect("entries") {
            let entry = entry.expect("entry");
            names.push(entry.path().expect("path").to_string_lossy().to_string());
        }
        assert_eq!(names, vec![TOKEN_ENV_FILE_NAME.to_string()]);
    }

    #[test]
    fn cleanup_task_credential_clears_sensitive_fields() {
        let mut credential = Some(TaskCredential {
            token: "secret".to_string(),
            expires_at_unix: 99,
            env_name: "ENV".to_string(),
        });
        cleanup_task_credential(&mut credential);
        assert!(credential.is_none());
    }

    #[test]
    fn unix_now_secs_returns_reasonable_value() {
        let ts = unix_now_secs().expect("unix_now");
        assert!(ts > 1_700_000_000);
    }

    #[test]
    fn format_output_empty_stdout() {
        let out = format_output("", "err msg\n", 1);
        assert!(!out.contains("stdout:"));
        assert!(out.contains("stderr:\nerr msg"));
        assert!(out.contains("exit_code: 1"));
    }

    #[test]
    fn format_output_empty_stderr() {
        let out = format_output("hello\n", "", 0);
        assert!(out.contains("stdout:\nhello"));
        assert!(!out.contains("stderr:"));
        assert!(out.contains("exit_code: 0"));
    }

    #[test]
    fn format_output_both_empty() {
        let out = format_output("", "", 0);
        assert!(!out.contains("stdout:"));
        assert!(!out.contains("stderr:"));
        assert!(out.contains("exit_code: 0"));
    }

    #[test]
    fn format_output_appends_newline_when_missing() {
        let out = format_output("no-newline", "also-no-newline", 2);
        assert!(out.contains("stdout:\nno-newline\n"));
        assert!(out.contains("stderr:\nalso-no-newline\n"));
    }

    #[test]
    fn non_empty_with_content() {
        assert_eq!(non_empty("hello"), Some("hello"));
        assert_eq!(non_empty("  hello  "), Some("hello"));
    }

    #[test]
    fn non_empty_with_empty() {
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty("\t\n"), None);
    }

    #[tokio::test]
    async fn run_as_subprocess_rejects_empty_command() {
        let mut args = base_args("");
        args.command = Some(String::new());
        let err = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect_err("empty command should fail");
        assert!(err.contains("command must not be empty"));
    }

    #[tokio::test]
    async fn run_as_subprocess_rejects_whitespace_only_command() {
        let mut args = base_args("  ");
        args.command = Some("   ".to_string());
        let err = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect_err("whitespace command should fail");
        assert!(err.contains("command must not be empty"));
    }

    #[tokio::test]
    async fn run_as_subprocess_rejects_invalid_env_format() {
        let mut args = base_args("echo ok");
        args.env = Some(vec!["NO_EQUALS_SIGN".to_string()]);
        let err = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect_err("malformed env should fail");
        assert!(err.contains("Invalid env entry"));
        assert!(err.contains("KEY=VALUE"));
    }

    #[tokio::test]
    async fn run_as_subprocess_rejects_empty_env_key() {
        let mut args = base_args("echo ok");
        args.env = Some(vec!["=somevalue".to_string()]);
        let err = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect_err("empty key env should fail");
        assert!(err.contains("key must not be empty"));
    }

    #[tokio::test]
    async fn run_as_subprocess_handles_multiple_env_vars() {
        let mut args = base_args("printf '%s:%s' \"$A\" \"$B\"");
        args.env = Some(vec!["A=hello".to_string(), "B=world".to_string()]);
        let exec = run_as_subprocess(&args, Duration::from_secs(2))
            .await
            .expect("multiple env should work");
        assert_eq!(exec.exit_code, 0);
        assert!(exec.stdout.contains("hello:world"));
    }

    #[test]
    fn maybe_mint_task_credential_generates_valid_token_when_enabled() {
        let args = ContainerArgs {
            inject_task_token: Some(true),
            token_ttl_secs: Some(60),
            token_env_name: Some("MY_TOKEN".to_string()),
            ..base_args("echo hi")
        };
        let credential = maybe_mint_task_credential(&args)
            .expect("mint should succeed")
            .expect("credential should be Some");
        assert_eq!(credential.env_name, "MY_TOKEN");
        assert!(!credential.token.is_empty());
        assert!(credential.expires_at_unix > 0);
    }

    #[test]
    fn maybe_mint_task_credential_clamps_ttl() {
        let args = ContainerArgs {
            inject_task_token: Some(true),
            token_ttl_secs: Some(9999),
            token_env_name: None,
            ..base_args("echo hi")
        };
        let credential = maybe_mint_task_credential(&args)
            .expect("mint should succeed")
            .expect("credential should be Some");
        assert_eq!(credential.env_name, DEFAULT_TOKEN_ENV_NAME);
        let now = unix_now_secs().expect("now");
        assert!(credential.expires_at_unix <= now + MAX_TOKEN_TTL_SECS);
    }

    #[test]
    fn build_shell_command_without_credential() {
        let cmd = build_shell_command("echo hello", None);
        assert_eq!(cmd, "echo hello");
    }

    #[test]
    fn resolve_image_whitespace_only_image() {
        let args = ContainerArgs {
            image: Some("   ".to_string()),
            skill_image: None,
            ..base_args("echo ok")
        };
        assert!(resolve_image(&args).is_none());
    }

    #[test]
    fn resolve_image_whitespace_image_with_skill_fallback() {
        let args = ContainerArgs {
            image: Some("  ".to_string()),
            skill_image: Some("  python:3  ".to_string()),
            ..base_args("echo ok")
        };
        assert_eq!(resolve_image(&args).as_deref(), Some("python:3"));
    }

    #[test]
    fn resolve_image_both_whitespace_returns_none() {
        let args = ContainerArgs {
            image: Some("  ".to_string()),
            skill_image: Some("  ".to_string()),
            ..base_args("echo ok")
        };
        assert!(resolve_image(&args).is_none());
    }

    #[tokio::test]
    async fn resolve_runtime_mode_returns_docker_when_no_skill_name() {
        let args = base_args("echo ok");
        let mode = resolve_runtime_mode(&args).await.expect("mode");
        assert_eq!(mode, RuntimeMode::Docker);
    }

    #[tokio::test]
    async fn resolve_runtime_mode_returns_error_when_workspace_missing() {
        let args = ContainerArgs {
            skill_name: Some("myskill".to_string()),
            workspace_dir: None,
            ..base_args("echo")
        };
        let err = resolve_runtime_mode(&args).await.expect_err("should fail");
        assert!(err.contains("workspace_dir is required"));
    }

    #[tokio::test]
    async fn resolve_runtime_mode_returns_error_for_missing_skill_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let args = ContainerArgs {
            skill_name: Some("no_such_skill".to_string()),
            workspace_dir: Some(tmp.path().to_string_lossy().to_string()),
            ..base_args("echo")
        };
        let err = resolve_runtime_mode(&args).await.expect_err("should fail");
        assert!(err.contains("SKILL.md not found"));
    }

    #[tokio::test]
    async fn execute_rejects_missing_image_after_command_validation() {
        let tool = ContainerTool::new();
        let result = tool
            .execute(
                "call-img",
                serde_json::json!({"image":"","command":"echo hi", "timeout_secs": 1}),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("image must not be empty"));
    }

    #[test]
    fn build_container_name_handles_empty_call_id() {
        let name = build_container_name("");
        assert_eq!(name, "openpista-");
    }

    #[test]
    fn build_container_name_handles_unicode() {
        let name = build_container_name("call-été");
        assert!(name.starts_with("openpista-call-"));
    }

    #[test]
    fn format_output_truncates_long_stdout() {
        let long_stdout = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let out = format_output(&long_stdout, "", 0);
        assert!(out.contains("truncated"));
        assert!(out.contains("exit_code: 0"));
    }

    #[test]
    fn is_valid_env_name_accepts_underscore_prefix() {
        assert!(is_valid_env_name("_"));
        assert!(is_valid_env_name("__DOUBLE"));
        assert!(is_valid_env_name("a"));
        assert!(is_valid_env_name("Z9_x"));
    }

    #[test]
    fn shell_single_quote_no_quotes() {
        assert_eq!(shell_single_quote("simple"), "'simple'");
    }

    #[test]
    fn shell_single_quote_empty() {
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn format_output_stdout_only() {
        let out = format_output("hello\n", "", 0);
        assert!(out.contains("stdout:\nhello\n"));
        assert!(out.contains("exit_code: 0"));
        assert!(!out.contains("stderr:"));
    }

    #[test]
    fn format_output_stderr_only() {
        let out = format_output("", "error msg\n", 1);
        assert!(out.contains("stderr:\nerror msg\n"));
        assert!(out.contains("exit_code: 1"));
        assert!(!out.contains("stdout:"));
    }

    #[test]
    fn format_output_both_streams() {
        let out = format_output("ok\n", "warn\n", 0);
        assert!(out.contains("stdout:\nok\n"));
        assert!(out.contains("stderr:\nwarn\n"));
        assert!(out.contains("exit_code: 0"));
    }

    #[test]
    fn format_output_empty_streams() {
        let out = format_output("", "", 42);
        assert_eq!(out, "\nexit_code: 42");
    }

    #[test]
    fn build_task_credential_script_contains_env_vars() {
        let cred = TaskCredential {
            token: "my-secret-token".to_string(),
            expires_at_unix: 1700000000,
            env_name: "MY_TOKEN".to_string(),
        };
        let script = build_task_credential_script(&cred);
        assert!(script.contains("export MY_TOKEN="));
        assert!(script.contains("my-secret-token"));
        assert!(script.contains("export openpista_TASK_TOKEN_EXPIRES_AT=1700000000"));
    }

    #[test]
    fn build_task_credential_archive_produces_valid_tar() {
        let cred = TaskCredential {
            token: "tok".to_string(),
            expires_at_unix: 123,
            env_name: "ENV".to_string(),
        };
        let archive = build_task_credential_archive(&cred).expect("archive should build");
        // A tar archive should be non-empty and start with the filename
        assert!(!archive.is_empty());
        // The archive should contain the env file name
        let archive_str = String::from_utf8_lossy(&archive);
        assert!(archive_str.contains(TOKEN_ENV_FILE_NAME));
    }

    #[test]
    fn cleanup_task_credential_clears_fields() {
        let mut cred = Some(TaskCredential {
            token: "secret".to_string(),
            expires_at_unix: 999,
            env_name: "TOK".to_string(),
        });
        cleanup_task_credential(&mut cred);
        assert!(cred.is_none());
    }

    #[test]
    fn cleanup_task_credential_handles_none() {
        let mut cred: Option<TaskCredential> = None;
        cleanup_task_credential(&mut cred);
        assert!(cred.is_none());
    }
    #[test]
    fn build_shell_command_with_credential_prepends_source() {
        let cred = TaskCredential {
            token: "tok".to_string(),
            expires_at_unix: 0,
            env_name: "E".to_string(),
        };
        let cmd = build_shell_command("echo hello", Some(&cred));
        assert!(cmd.starts_with(". "));
        assert!(cmd.ends_with("echo hello"));
        assert!(cmd.contains(TOKEN_MOUNT_DIR));
        assert!(cmd.contains(TOKEN_ENV_FILE_NAME));
    }

    #[test]
    fn sanitize_env_name_uses_default_when_none() {
        let name = sanitize_env_name(None).unwrap();
        assert_eq!(name, DEFAULT_TOKEN_ENV_NAME);
    }

    #[test]
    fn sanitize_env_name_rejects_invalid() {
        assert!(sanitize_env_name(Some("123invalid")).is_err());
        assert!(sanitize_env_name(Some("")).is_err());
        assert!(sanitize_env_name(Some("has space")).is_err());
    }

    #[test]
    fn sanitize_env_name_accepts_valid() {
        assert_eq!(sanitize_env_name(Some("MY_VAR")).unwrap(), "MY_VAR");
        assert_eq!(sanitize_env_name(Some("_x")).unwrap(), "_x");
    }

    #[test]
    fn non_empty_trims_and_filters() {
        assert_eq!(non_empty("  hello  "), Some("hello"));
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty("world"), Some("world"));
    }

    #[test]
    fn container_tool_metadata() {
        let tool = ContainerTool::new();
        assert_eq!(tool.name(), "container.run");
        assert!(tool.description().contains("Docker"));
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["image"].is_object());
        assert!(schema["properties"]["command"].is_object());
    }

    #[test]
    fn container_tool_default() {
        let _tool = ContainerTool;
    }

    #[tokio::test]
    async fn execute_rejects_missing_command() {
        let tool = ContainerTool::new();
        let result = tool
            .execute("call-1", serde_json::json!({"image": "alpine:3"}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("command must not be empty"));
    }

    #[tokio::test]
    async fn execute_rejects_missing_image() {
        let tool = ContainerTool::new();
        let result = tool
            .execute("call-2", serde_json::json!({"command": "echo hi"}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("image must not be empty"));
    }

    #[tokio::test]
    async fn execute_rejects_invalid_json() {
        let tool = ContainerTool::new();
        let result = tool
            .execute("call-3", serde_json::json!("not an object"))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }
}
