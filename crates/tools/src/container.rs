//! Container execution tool powered by Docker Engine API.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use bollard::Docker;
use bollard::body_full;
use bollard::container::LogOutput;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, StartContainerOptions, UploadToContainerOptionsBuilder,
    WaitContainerOptions,
};
use futures_util::TryStreamExt;
use proto::ToolResult;
use rand::RngCore;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::timeout;

use crate::Tool;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;
const DEFAULT_TOKEN_TTL_SECS: u64 = 300;
const MAX_TOKEN_TTL_SECS: u64 = 900;
const DEFAULT_MEMORY_MB: i64 = 512;
const DEFAULT_CPU_MILLIS: i64 = 1000;
const MAX_OUTPUT_CHARS: usize = 10_000;
const TOKEN_ENV_FILE_NAME: &str = ".openpista_task_env";
const TOKEN_MOUNT_DIR: &str = "/run/secrets";
const DEFAULT_TOKEN_ENV_NAME: &str = "OPENPISTACRAB_TASK_TOKEN";

/// Tool that runs one-off commands in isolated Docker containers.
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
    inject_task_token: Option<bool>,
    token_ttl_secs: Option<u64>,
    token_env_name: Option<String>,
}

#[derive(Debug, Clone)]
struct TaskCredential {
    token: String,
    expires_at_unix: u64,
    env_name: String,
}

impl ContainerTool {
    /// Creates a new container execution tool.
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
                    "description": "Environment variable name exposed by injected credential file (default: OPENPISTACRAB_TASK_TOKEN)"
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
        let mut credential = match maybe_mint_task_credential(&parsed) {
            Ok(v) => v,
            Err(e) => return ToolResult::error(call_id, self.name(), e),
        };

        let run = timeout(
            timeout_duration,
            run_container(&docker, &container_name, &parsed, credential.as_ref()),
        )
        .await;

        cleanup_task_credential(&mut credential);

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
    credential: Option<&TaskCredential>,
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
    if credential.is_some() {
        tmpfs.insert(
            TOKEN_MOUNT_DIR.to_string(),
            "rw,nosuid,nodev,noexec,size=1m".to_string(),
        );
    }

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
            build_shell_command(args, credential),
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

    if let Some(credential) = credential
        && let Err(e) = upload_task_credential(docker, container_name, credential).await
    {
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

fn maybe_mint_task_credential(args: &ContainerArgs) -> Result<Option<TaskCredential>, String> {
    if !args.inject_task_token.unwrap_or(false) {
        return Ok(None);
    }

    let ttl_secs = args
        .token_ttl_secs
        .unwrap_or(DEFAULT_TOKEN_TTL_SECS)
        .clamp(1, MAX_TOKEN_TTL_SECS);
    let env_name = sanitize_env_name(args.token_env_name.as_deref())?;
    let now = unix_now_secs()?;

    let mut random = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut random);
    let token = general_purpose::URL_SAFE_NO_PAD.encode(random);

    Ok(Some(TaskCredential {
        token,
        expires_at_unix: now + ttl_secs,
        env_name,
    }))
}

fn sanitize_env_name(env_name: Option<&str>) -> Result<String, String> {
    let value = env_name.unwrap_or(DEFAULT_TOKEN_ENV_NAME).trim();
    if !is_valid_env_name(value) {
        return Err("token_env_name must match [A-Za-z_][A-Za-z0-9_]*".to_string());
    }
    Ok(value.to_string())
}

fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn unix_now_secs() -> Result<u64, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {e}"))?;
    Ok(now.as_secs())
}

fn build_shell_command(args: &ContainerArgs, credential: Option<&TaskCredential>) -> String {
    if credential.is_some() {
        format!(
            ". {TOKEN_MOUNT_DIR}/{TOKEN_ENV_FILE_NAME} >/dev/null 2>&1; {}",
            args.command
        )
    } else {
        args.command.clone()
    }
}

async fn upload_task_credential(
    docker: &Docker,
    container_name: &str,
    credential: &TaskCredential,
) -> Result<(), String> {
    let archive = build_task_credential_archive(credential)?;
    docker
        .upload_to_container(
            container_name,
            Some(
                UploadToContainerOptionsBuilder::default()
                    .path(TOKEN_MOUNT_DIR)
                    .build(),
            ),
            body_full(archive.into()),
        )
        .await
        .map_err(|e| format!("Failed to inject task credential: {e}"))
}

fn build_task_credential_archive(credential: &TaskCredential) -> Result<Vec<u8>, String> {
    let payload = build_task_credential_script(credential);
    let payload_bytes = payload.as_bytes();

    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(payload_bytes.len() as u64);
    header.set_mode(0o400);
    header.set_cksum();

    builder
        .append_data(&mut header, TOKEN_ENV_FILE_NAME, payload_bytes)
        .map_err(|e| format!("Failed to build credential archive: {e}"))?;

    builder
        .into_inner()
        .map_err(|e| format!("Failed to finalize credential archive: {e}"))
}

fn build_task_credential_script(credential: &TaskCredential) -> String {
    format!(
        "export {}={}\nexport OPENPISTACRAB_TASK_TOKEN_EXPIRES_AT={}\n",
        credential.env_name,
        shell_single_quote(&credential.token),
        credential.expires_at_unix
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn cleanup_task_credential(credential: &mut Option<TaskCredential>) {
    if let Some(inner) = credential {
        inner.token.clear();
        inner.env_name.clear();
        inner.expires_at_unix = 0;
    }
    *credential = None;
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

    #[test]
    fn maybe_mint_task_credential_returns_none_when_disabled() {
        let args = ContainerArgs {
            image: "alpine:3".to_string(),
            command: "echo hi".to_string(),
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
        };

        let credential = maybe_mint_task_credential(&args).expect("mint result");
        assert!(credential.is_none());
    }

    #[test]
    fn maybe_mint_task_credential_validates_env_name() {
        let args = ContainerArgs {
            image: "alpine:3".to_string(),
            command: "echo hi".to_string(),
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
        };

        let err = maybe_mint_task_credential(&args).expect_err("invalid env name should fail");
        assert!(err.contains("token_env_name"));
    }

    #[test]
    fn build_shell_command_sources_credential_file() {
        let args = ContainerArgs {
            image: "alpine:3".to_string(),
            command: "echo hi".to_string(),
            timeout_secs: None,
            working_dir: None,
            env: None,
            allow_network: None,
            workspace_dir: None,
            memory_mb: None,
            cpu_millis: None,
            pull: None,
            inject_task_token: Some(true),
            token_ttl_secs: None,
            token_env_name: None,
        };
        let credential = TaskCredential {
            token: "tok".to_string(),
            expires_at_unix: 1,
            env_name: DEFAULT_TOKEN_ENV_NAME.to_string(),
        };

        let cmd = build_shell_command(&args, Some(&credential));
        assert!(cmd.contains(TOKEN_ENV_FILE_NAME));
        assert!(cmd.contains("echo hi"));
    }

    #[test]
    fn build_task_credential_script_contains_exports() {
        let credential = TaskCredential {
            token: "abc123".to_string(),
            expires_at_unix: 42,
            env_name: "OPENPISTACRAB_TASK_TOKEN".to_string(),
        };

        let script = build_task_credential_script(&credential);
        assert!(script.contains("export OPENPISTACRAB_TASK_TOKEN='abc123'"));
        assert!(script.contains("OPENPISTACRAB_TASK_TOKEN_EXPIRES_AT=42"));
    }
}
