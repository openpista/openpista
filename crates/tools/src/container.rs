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
use skills::{SkillExecutionMode, SkillLoader};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::warn;

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
        let execution_result = run_with_docker_or_subprocess(
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

/// Determine whether the provided container arguments should run as a Docker container or as a WASM skill.
///
/// If `skill_name` is not provided or is empty, this returns `RuntimeMode::Docker`.
/// When `skill_name` is present, `workspace_dir` must also be provided; the function loads the skill's metadata
/// from the workspace and returns `RuntimeMode::Wasm` when the skill's execution mode is WASM, otherwise
/// it returns `RuntimeMode::Docker`.
///
/// # Errors
///
/// Returns `Err` if `skill_name` is present but `workspace_dir` is missing or empty, or if the skill metadata
/// (SKILL.md) cannot be found for the given `skill_name`.
///
/// # Examples
///
/// ```ignore
/// # use crate::ContainerArgs;
/// # use crate::RuntimeMode;
/// # tokio_test::block_on(async {
/// let args = ContainerArgs {
///     skill_name: None,
///     workspace_dir: None,
///     image: None,
///     command: None,
///     skill_image: None,
///     skill_args: None,
///     timeout_secs: None,
///     working_dir: None,
///     env: None,
///     allow_network: None,
///     workspace_dir: None,
///     memory_mb: None,
///     cpu_millis: None,
///     pull: None,
///     inject_task_token: None,
///     token_ttl_secs: None,
///     token_env_name: None,
///     allow_subprocess_fallback: None,
/// };
///
/// let mode = crate::resolve_runtime_mode(&args).await.unwrap();
/// assert_eq!(mode, RuntimeMode::Docker);
/// # });
/// ```
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

/// Creates and runs a Docker container from the resolved image and returns its captured output and exit code.
///
/// The function resolves the image from `args.image` or `args.skill_image`, validates the command,
/// optionally pulls the image if `args.pull` is true, and starts a container configured with the
/// resource limits and mounts derived from `args`. If `credential` is provided it is injected into
/// the container at the configured secure mount path. The container is removed/cleaned up after
/// execution completes (or on error).
///
/// # Returns
///
/// `Ok(ContainerExecution)` containing `stdout`, `stderr`, and `exit_code` on success; `Err(String)` with
/// a human-readable error message on failure.
///
/// # Examples
///
/// ```ignore
/// # tokio_test::block_on(async {
/// use bollard::Docker;
/// // Construct a Docker client (adjust connection as appropriate for the environment).
/// let docker = Docker::connect_with_socket_defaults().unwrap();
///
/// let args = crate::ContainerArgs {
///     image: Some("alpine:latest".into()),
///     command: Some("echo hello".into()),
///     ..Default::default()
/// };
///
/// // Run the container and inspect the captured output.
/// let result = crate::run_container(&docker, "example-container", &args, None).await;
/// let exec = result.expect("container run failed");
/// assert!(exec.stdout.contains("hello"));
/// # });
/// ```
async fn run_container(
    docker: &Docker,
    container_name: &str,
    args: &ContainerArgs,
    credential: Option<&TaskCredential>,
) -> Result<ContainerExecution, String> {
    let image = args
        .image
        .as_deref()
        .and_then(non_empty)
        .or_else(|| args.skill_image.as_deref().and_then(non_empty))
        .ok_or_else(|| "image or skill_image must not be empty".to_string())?;

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
        image: Some(image.to_string()),
        cmd: Some(vec![
            "sh".to_string(),
            "-lc".to_string(),
            build_shell_command(command, credential),
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

        Ok(ContainerExecution {
            stdout,
            stderr,
            exit_code,
        })
    }
    .await;

    let _ = cleanup_container(docker, container_name).await;
    run_result
}

/// Execute the requested workload using the Docker daemon when available; if Docker is unavailable and
/// `args.allow_subprocess_fallback` is true, fall back to running the command as a local subprocess.
///
/// This function:
/// - Requires a resolved image when using Docker and returns an error if none is provided.
/// - May mint a per-task credential (when requested) and uploads it into the container; the credential is
///   always cleaned up after the run attempt.
/// - Applies `timeout_duration` to the container execution; on timeout it attempts to clean up the created
///   container and returns a timeout error.
/// - When Docker is unavailable and fallback is allowed, runs the command via the local shell with the same
///   timeout semantics.
///
/// # Parameters
///
/// - `call_id`: identifier used for container naming and logging.
/// - `args`: container run arguments and runtime options.
/// - `timeout_duration`: maximum allowed duration for the workload execution.
/// - `container_name`: sanitized container name to use when creating the container.
/// - `docker_result`: the result of attempting to connect to Docker; `Ok` branch runs in Docker, `Err` branch
///   triggers optional subprocess fallback.
///
/// # Returns
///
/// `ContainerExecution` on success; an error `String` describing why execution failed otherwise.
///
/// # Examples
///
/// ```ignore
/// # use std::time::Duration;
/// # async fn example() -> Result<(), String> {
/// let call_id = "call-123";
/// let args = /* construct ContainerArgs with desired fields */ todo!();
/// let timeout = Duration::from_secs(30);
/// let container_name = "call-123-container";
/// let docker_conn: Result<_, String> = Err("docker connect failed".to_string());
///
/// let result = run_with_docker_or_subprocess(call_id, &args, timeout, container_name, docker_conn).await;
/// match result {
///     Ok(exec) => println!("exit={} stdout={}", exec.exit_code, exec.stdout),
///     Err(e) => eprintln!("execution failed: {}", e),
/// }
/// # Ok(())
/// # }
/// ```
async fn run_with_docker_or_subprocess(
    call_id: &str,
    args: &ContainerArgs,
    timeout_duration: Duration,
    container_name: &str,
    docker_result: Result<Docker, String>,
) -> Result<ContainerExecution, String> {
    match docker_result {
        Ok(docker) => {
            if resolve_image(args).is_none() {
                return Err("image must not be empty".to_string());
            }

            let mut credential = maybe_mint_task_credential(args)?;
            let run = timeout(
                timeout_duration,
                run_container(&docker, container_name, args, credential.as_ref()),
            )
            .await;

            cleanup_task_credential(&mut credential);

            match run {
                Ok(Ok(execution)) => Ok(execution),
                Ok(Err(e)) => Err(e),
                Err(_) => {
                    let _ = cleanup_container(&docker, container_name).await;
                    Err(format!(
                        "Command timed out after {}s",
                        timeout_duration.as_secs()
                    ))
                }
            }
        }
        Err(e) => {
            if !args.allow_subprocess_fallback.unwrap_or(false) {
                return Err(e);
            }

            warn!("Docker unavailable for call_id={call_id}; falling back to subprocess mode: {e}");
            run_as_subprocess(args, timeout_duration).await
        }
    }
}

/// Execute the configured command locally as a subprocess when Docker is unavailable.
///
/// Attempts to spawn `/bin/sh -c <command>` using the provided `ContainerArgs`, applies
/// optional working directory and environment entries, enforces the provided timeout,
/// and captures stdout, stderr, and the process exit code. This fallback does not
/// enforce Docker resource limits.
///
/// # Errors
///
/// Returns an `Err(String)` when the command is missing or empty, when environment
/// entries are malformed, when the subprocess cannot be spawned or run, or when the
/// execution exceeds `timeout_duration`.
///
/// # Examples
///
/// ```ignore
/// # use std::time::Duration;
/// # use tokio::runtime::Runtime;
/// # use crate::ContainerArgs;
/// # use crate::run_as_subprocess;
/// let rt = Runtime::new().unwrap();
/// rt.block_on(async {
///     let args = ContainerArgs {
///         image: None,
///         skill_image: None,
///         command: Some("echo hello".to_string()),
///         skill_name: None,
///         skill_args: None,
///         timeout_secs: None,
///         working_dir: None,
///         env: None,
///         allow_network: None,
///         workspace_dir: None,
///         memory_mb: None,
///         cpu_millis: None,
///         pull: None,
///         inject_task_token: None,
///         token_ttl_secs: None,
///         token_env_name: None,
///         allow_subprocess_fallback: None,
///     };
///     let res = run_as_subprocess(&args, Duration::from_secs(5)).await.unwrap();
///     assert!(res.stdout.contains("hello"));
///     assert_eq!(res.exit_code, 0);
/// });
/// ```
async fn run_as_subprocess(
    args: &ContainerArgs,
    timeout_duration: Duration,
) -> Result<ContainerExecution, String> {
    warn!("Docker is unavailable; running command as local subprocess fallback");
    warn!(
        "subprocess fallback does not enforce Docker CPU/memory limits (requested memory_mb={:?}, cpu_millis={:?})",
        args.memory_mb, args.cpu_millis
    );

    let command = args
        .command
        .as_deref()
        .and_then(non_empty)
        .ok_or_else(|| "command must not be empty".to_string())?;

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    if let Some(working_dir) = args
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        cmd.current_dir(working_dir);
    }

    if let Some(env_values) = args.env.as_deref() {
        for value in env_values {
            let Some((key, env_value)) = value.split_once('=') else {
                return Err(format!("Invalid env entry '{value}', expected KEY=VALUE"));
            };

            let key = key.trim();
            if key.is_empty() {
                return Err(format!(
                    "Invalid env entry '{value}', key must not be empty"
                ));
            }

            cmd.env(key, env_value);
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn subprocess: {e}"))?;
    let output = timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| format!("Command timed out after {}s", timeout_duration.as_secs()))?
        .map_err(|e| format!("Failed to run subprocess: {e}"))?;

    Ok(ContainerExecution {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: i64::from(output.status.code().unwrap_or(-1)),
    })
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

/// Returns the current Unix timestamp in seconds.
///
/// Produces the number of seconds elapsed since 1970-01-01 00:00:00 UTC.
/// Returns an `Err(String)` if the system clock is earlier than the Unix epoch
/// or another system time error occurs.
///
/// # Examples
///
/// ```ignore
/// let ts = unix_now_secs().expect("failed to read system time");
/// assert!(ts > 0);
/// ```
fn unix_now_secs() -> Result<u64, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {e}"))?;
    Ok(now.as_secs())
}

/// Prepends credential sourcing to a shell command when a task credential is provided.
///
/// When `credential` is `Some`, the returned string first sources the credential file
/// mounted at the tool's token mount directory and suppresses its output, then runs
/// the provided `command`. When `credential` is `None`, the original `command` is
/// returned unchanged.
///
/// # Examples
///
/// ```ignore
/// // No credential: command unchanged
/// let cmd = build_shell_command("echo hello", None);
/// assert_eq!(cmd, "echo hello");
///
/// // With credential: result contains the original command and a sourcing prefix
/// let cred = TaskCredential {
///     token: "token".into(),
///     expires_at_unix: 0,
///     env_name: "openpista_TASK_TOKEN".into(),
/// };
/// let cmd_with_cred = build_shell_command("echo secret", Some(&cred));
/// assert!(cmd_with_cred.ends_with("echo secret"));
/// assert!(cmd_with_cred.starts_with(". "));
/// ```
fn build_shell_command(command: &str, credential: Option<&TaskCredential>) -> String {
    if credential.is_some() {
        format!(
            ". {TOKEN_MOUNT_DIR}/{TOKEN_ENV_FILE_NAME} >/dev/null 2>&1; {}",
            command
        )
    } else {
        command.to_string()
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
    header.set_mode(0o444); // readable by non-root users in container
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
        "export {}={}\nexport openpista_TASK_TOKEN_EXPIRES_AT={}\n",
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
    use std::path::Path;

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
    fn truncate_str_short_string() {
        assert_eq!(truncate_str("hello", 100), "hello");
    }

    #[test]
    fn truncate_str_exact_boundary() {
        assert_eq!(truncate_str("abc", 3), "abc");
    }

    #[test]
    fn truncate_str_over_limit() {
        let result = truncate_str("abcdef", 3);
        assert!(result.starts_with("abc"));
        assert!(result.contains("truncated at 3 chars"));
    }

    #[test]
    fn truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
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
}
