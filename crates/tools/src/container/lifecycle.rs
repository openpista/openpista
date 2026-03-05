//! Container lifecycle management, subprocess fallback, and credential handling.

use base64::{Engine as _, engine::general_purpose};
use bollard::Docker;
use rand::RngCore;
use std::process::Stdio;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::warn;

use super::docker::{cleanup_container, run_container};
use super::{
    ContainerArgs, ContainerExecution, DEFAULT_TOKEN_ENV_NAME, DEFAULT_TOKEN_TTL_SECS,
    MAX_TOKEN_TTL_SECS, TOKEN_ENV_FILE_NAME, TOKEN_MOUNT_DIR, TaskCredential, non_empty,
    resolve_image,
};

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
pub(super) async fn run_with_docker_or_subprocess(
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
pub(super) async fn run_as_subprocess(
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

pub(super) fn maybe_mint_task_credential(
    args: &ContainerArgs,
) -> Result<Option<TaskCredential>, String> {
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

pub(super) fn sanitize_env_name(env_name: Option<&str>) -> Result<String, String> {
    let value = env_name.unwrap_or(DEFAULT_TOKEN_ENV_NAME).trim();
    if !is_valid_env_name(value) {
        return Err("token_env_name must match [A-Za-z_][A-Za-z0-9_]*".to_string());
    }
    Ok(value.to_string())
}

pub(super) fn is_valid_env_name(name: &str) -> bool {
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
pub(super) fn unix_now_secs() -> Result<u64, String> {
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
pub(super) fn build_shell_command(command: &str, credential: Option<&TaskCredential>) -> String {
    if credential.is_some() {
        format!(
            ". {TOKEN_MOUNT_DIR}/{TOKEN_ENV_FILE_NAME} >/dev/null 2>&1; {}",
            command
        )
    } else {
        command.to_string()
    }
}

pub(super) fn build_task_credential_script(credential: &TaskCredential) -> String {
    format!(
        "export {}={}\nexport openpista_TASK_TOKEN_EXPIRES_AT={}\n",
        credential.env_name,
        shell_single_quote(&credential.token),
        credential.expires_at_unix
    )
}

pub(super) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(super) fn cleanup_task_credential(credential: &mut Option<TaskCredential>) {
    if let Some(inner) = credential {
        inner.token.clear();
        inner.env_name.clear();
        inner.expires_at_unix = 0;
    }
    *credential = None;
}
