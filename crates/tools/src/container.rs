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
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;

use crate::Tool;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;
const DEFAULT_MEMORY_MB: i64 = 512;
const DEFAULT_CPU_MILLIS: i64 = 1000;
const MAX_OUTPUT_CHARS: usize = 10_000;

pub struct ContainerTool;

#[derive(Debug, Deserialize)]
struct ContainerArgs {
    image: String,
    command: String,
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
                    "description": "Container image name (for example: alpine:3.20)"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute in the container"
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
            "required": ["image", "command"],
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

        if parsed.image.trim().is_empty() {
            return ToolResult::error(call_id, self.name(), "image must not be empty");
        }
        if parsed.command.trim().is_empty() {
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

async fn run_container(
    docker: &Docker,
    container_name: &str,
    args: &ContainerArgs,
) -> Result<String, String> {
    if args.pull.unwrap_or(false) {
        docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(args.image.as_str())
                        .build(),
                ),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| format!("Failed to pull image '{}': {e}", args.image))?;
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
        image: Some(args.image.clone()),
        cmd: Some(vec![
            "sh".to_string(),
            "-lc".to_string(),
            args.command.clone(),
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
    use super::*;

    #[test]
    fn container_tool_metadata_is_stable() {
        let tool = ContainerTool::new();
        assert_eq!(tool.name(), "container.run");
        assert!(tool.description().contains("Docker"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "image");
        assert_eq!(schema["required"][1], "command");
    }

    #[tokio::test]
    async fn execute_rejects_invalid_arguments() {
        let tool = ContainerTool::new();
        let result = tool
            .execute("call-1", serde_json::json!({"image":"alpine:3"}))
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn execute_rejects_empty_required_fields() {
        let tool = ContainerTool::new();
        let result = tool
            .execute(
                "call-2",
                serde_json::json!({"image":"","command":"echo ok"}),
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("image must not be empty"));
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
