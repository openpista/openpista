use async_trait::async_trait;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, StartContainerOptions, WaitContainerOptions,
};
use futures_util::TryStreamExt;
use proto::ToolResult;
use serde::Deserialize;
use skills::{SkillExecutionMode, SkillLoader};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;

use crate::Tool;
use crate::wasm_runtime::{WasmRunRequest, run_wasm_skill};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;
const DEFAULT_MEMORY_MB: i64 = 512;
const DEFAULT_CPU_MILLIS: i64 = 1000;
const MAX_OUTPUT_CHARS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeMode {
    Docker,
    Wasm,
}

/// Container/WASM sandbox execution tool (`container.run`).
pub struct ContainerTool;

#[derive(Debug, Deserialize)]
struct ContainerArgs {
    image: Option<String>,
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
}

impl ContainerTool {
    /// Creates a container sandbox tool instance.
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

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image": {
                    "type": "string",
                    "description": "Container image name (for example: alpine:3.20). Optional for wasm mode"
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
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: ContainerArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let runtime_mode = match resolve_runtime_mode(&parsed).await {
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

        let image = match parsed.image.as_deref().and_then(non_empty) {
            Some(image) => image,
            None => {
                return ToolResult::error(call_id, self.name(), "image must not be empty");
            }
        };

        let command = match parsed.command.as_deref().and_then(non_empty) {
            Some(command) => command,
            None => {
                return ToolResult::error(call_id, self.name(), "command must not be empty");
            }
        };

        if image.is_empty() {
            return ToolResult::error(call_id, self.name(), "image must not be empty");
        }
        if command.is_empty() {
            return ToolResult::error(call_id, self.name(), "command must not be empty");
        }

        let timeout_duration = Duration::from_secs(
            parsed
                .timeout_secs
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .clamp(1, MAX_TIMEOUT_SECS),
        );

        let docker = match Docker::connect_with_local_defaults() {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(
                    call_id,
                    self.name(),
                    format!("Failed to connect to Docker daemon: {e}"),
                );
            }
        };

        let container_name = build_container_name(call_id);

        let run = timeout(
            timeout_duration,
            run_container(&docker, &container_name, &parsed),
        )
        .await;

        match run {
            Ok(Ok(output)) => ToolResult::success(call_id, self.name(), output),
            Ok(Err(e)) => ToolResult::error(call_id, self.name(), e),
            Err(_) => {
                let _ = cleanup_container(&docker, &container_name).await;
                ToolResult::error(
                    call_id,
                    self.name(),
                    format!("Command timed out after {}s", timeout_duration.as_secs()),
                )
            }
        }
    }
}

async fn resolve_runtime_mode(args: &ContainerArgs) -> Result<RuntimeMode, String> {
    let Some(skill_name) = args.skill_name.as_deref().and_then(non_empty) else {
        return Ok(RuntimeMode::Docker);
    };

    let workspace = args
        .workspace_dir
        .as_deref()
        .and_then(non_empty)
        .ok_or_else(|| "workspace_dir is required when skill_name is provided".to_string())?;

    let loader = SkillLoader::new(workspace);
    let metadata = loader
        .load_skill_metadata(skill_name)
        .await
        .ok_or_else(|| format!("SKILL.md not found for skill '{skill_name}'"))?;

    if metadata.mode == SkillExecutionMode::Wasm {
        Ok(RuntimeMode::Wasm)
    } else {
        Ok(RuntimeMode::Docker)
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

async fn run_container(
    docker: &Docker,
    container_name: &str,
    args: &ContainerArgs,
) -> Result<String, String> {
    let image = args
        .image
        .as_deref()
        .and_then(non_empty)
        .ok_or_else(|| "image must not be empty".to_string())?;
    let command = args
        .command
        .as_deref()
        .and_then(non_empty)
        .ok_or_else(|| "command must not be empty".to_string())?;

    if args.pull.unwrap_or(false) {
        docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(image)
                        .build(),
                ),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| format!("Failed to pull image '{image}': {e}"))?;
    }

    let mut binds = Vec::new();
    if let Some(workspace_dir) = args.workspace_dir.as_deref() {
        let workspace_dir = workspace_dir.trim();
        if !workspace_dir.is_empty() {
            binds.push(format!("{workspace_dir}:/workspace:ro"));
        }
    }

    let mut tmpfs = HashMap::new();
    tmpfs.insert("/tmp".to_string(), "rw,nosuid,nodev,size=64m".to_string());

    let host_config = HostConfig {
        auto_remove: Some(false),
        network_mode: if args.allow_network.unwrap_or(false) {
            None
        } else {
            Some("none".to_string())
        },
        readonly_rootfs: Some(true),
        privileged: Some(false),
        cap_drop: Some(vec!["ALL".to_string()]),
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        memory: Some(args.memory_mb.unwrap_or(DEFAULT_MEMORY_MB).max(64) * 1024 * 1024),
        nano_cpus: Some(args.cpu_millis.unwrap_or(DEFAULT_CPU_MILLIS).max(100) * 1_000_000),
        pids_limit: Some(256),
        binds: if binds.is_empty() { None } else { Some(binds) },
        tmpfs: Some(tmpfs),
        ..Default::default()
    };

    let config = ContainerCreateBody {
        image: Some(image.to_string()),
        cmd: Some(vec![
            "sh".to_string(),
            "-lc".to_string(),
            command.to_string(),
        ]),
        env: args.env.clone(),
        working_dir: args.working_dir.clone(),
        host_config: Some(host_config),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        tty: Some(false),
        ..Default::default()
    };

    let create_result = docker
        .create_container(
            Some(
                CreateContainerOptionsBuilder::default()
                    .name(container_name)
                    .build(),
            ),
            config,
        )
        .await
        .map_err(|e| format!("Failed to create container: {e}"));

    if let Err(e) = create_result {
        let _ = cleanup_container(docker, container_name).await;
        return Err(e);
    }

    let run_result = async {
        docker
            .start_container(container_name, None::<StartContainerOptions>)
            .await
            .map_err(|e| format!("Failed to start container: {e}"))?;

        let waits = docker
            .wait_container(container_name, None::<WaitContainerOptions>)
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| format!("Failed while waiting container: {e}"))?;

        let exit_code = waits.last().map(|w| w.status_code).unwrap_or(-1);

        let logs = docker
            .logs(
                container_name,
                Some(
                    LogsOptionsBuilder::default()
                        .follow(false)
                        .stdout(true)
                        .stderr(true)
                        .tail("all")
                        .build(),
                ),
            )
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| format!("Failed to read container logs: {e}"))?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        for log in logs {
            match log {
                LogOutput::StdOut { message }
                | LogOutput::Console { message }
                | LogOutput::StdIn { message } => {
                    stdout.push_str(&String::from_utf8_lossy(&message))
                }
                LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(&message))
                }
            }
        }

        Ok(format_output(&stdout, &stderr, exit_code))
    }
    .await;

    let _ = cleanup_container(docker, container_name).await;
    run_result
}

async fn cleanup_container(docker: &Docker, container_name: &str) -> Result<(), String> {
    docker
        .remove_container(
            container_name,
            Some(RemoveContainerOptionsBuilder::default().force(true).build()),
        )
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to remove container '{container_name}': {e}"))
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

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}\n[... output truncated at {max_chars} chars]")
    }
}

fn format_output(stdout: &str, stderr: &str, exit_code: i64) -> String {
    let stdout = truncate_str(stdout, MAX_OUTPUT_CHARS / 2);
    let stderr = truncate_str(stderr, MAX_OUTPUT_CHARS / 2);

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

    use super::*;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, content).expect("write file");
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
                serde_json::json!({"image":"","command":"echo ok", "timeout_secs": 1}),
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("image must not be empty"));
    }

    #[tokio::test]
    async fn resolve_runtime_mode_detects_wasm_skill_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_file(
            &tmp.path().join("skills/hello/SKILL.md"),
            "---\nmode: wasm\n---\n# hello\n",
        );

        let args = ContainerArgs {
            image: None,
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
}
