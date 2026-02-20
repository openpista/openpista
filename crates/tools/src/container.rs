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
use proto::{ChannelEvent, ChannelId, SessionId, ToolResult, WorkerReport};
use rand::RngCore;
use rustls::DigitallySignedStruct;
use rustls::SignatureScheme;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::Tool;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;
const DEFAULT_TOKEN_TTL_SECS: u64 = 300;
const MAX_TOKEN_TTL_SECS: u64 = 900;
const DEFAULT_REPORT_TIMEOUT_SECS: u64 = 10;
const MAX_REPORT_TIMEOUT_SECS: u64 = 30;
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
    #[serde(default)]
    image: String,
    skill_image: Option<String>,
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
    report_via_quic: Option<bool>,
    report_timeout_secs: Option<u64>,
    orchestrator_quic_addr: Option<String>,
    orchestrator_channel_id: Option<String>,
    orchestrator_session_id: Option<String>,
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
                    "description": "Container image name (for example: alpine:3.20); when omitted, skill_image can be used"
                },
                "skill_image": {
                    "type": "string",
                    "description": "Skill-provided default image from SKILL.md front matter (used when image is omitted)"
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
                },
                "report_via_quic": {
                    "type": "boolean",
                    "description": "When true, submit worker execution report back to orchestrator over QUIC"
                },
                "report_timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds when submitting worker report over QUIC (default: 10, max: 30)"
                },
                "orchestrator_quic_addr": {
                    "type": "string",
                    "description": "Orchestrator QUIC listen address in host:port format"
                },
                "orchestrator_channel_id": {
                    "type": "string",
                    "description": "Orchestrator channel_id that owns this worker task"
                },
                "orchestrator_session_id": {
                    "type": "string",
                    "description": "Orchestrator session_id that should receive the worker report"
                },
                "allow_subprocess_fallback": {
                    "type": "boolean",
                    "description": "Fallback to local subprocess when Docker is unavailable (default: true)"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let mut parsed: ContainerArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        if parsed.command.trim().is_empty() {
            return ToolResult::error(call_id, self.name(), "command must not be empty");
        }

        if let Some(image) = resolve_image(&parsed) {
            parsed.image = image;
        }

        let timeout_duration = Duration::from_secs(
            parsed
                .timeout_secs
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .clamp(1, MAX_TIMEOUT_SECS),
        );

        let container_name = build_container_name(call_id);
        let image_for_report = if parsed.image.trim().is_empty() {
            "local-subprocess".to_string()
        } else {
            parsed.image.clone()
        };

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
            Ok(execution) => {
                if let Err(err) = maybe_report_worker_result(
                    call_id,
                    &parsed,
                    &container_name,
                    &image_for_report,
                    &execution,
                    timeout_duration,
                )
                .await
                {
                    warn!(
                        "worker report upload failed for call_id={call_id} container={container_name}: {err}"
                    );
                }

                ToolResult::success(
                    call_id,
                    self.name(),
                    format_output(&execution.stdout, &execution.stderr, execution.exit_code),
                )
            }
            Err(e) => ToolResult::error(call_id, self.name(), e),
        }
    }
}

async fn run_container(
    docker: &Docker,
    container_name: &str,
    args: &ContainerArgs,
    credential: Option<&TaskCredential>,
    image: &str,
) -> Result<ContainerExecution, String> {
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

async fn run_with_docker_or_subprocess(
    call_id: &str,
    args: &ContainerArgs,
    timeout_duration: Duration,
    container_name: &str,
    docker_result: Result<Docker, String>,
) -> Result<ContainerExecution, String> {
    match docker_result {
        Ok(docker) => {
            if args.image.trim().is_empty() {
                return Err("image must not be empty".to_string());
            }

            let mut credential = maybe_mint_task_credential(args)?;
            let run = timeout(
                timeout_duration,
                run_container(
                    &docker,
                    container_name,
                    args,
                    credential.as_ref(),
                    &args.image,
                ),
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
            if !args.allow_subprocess_fallback.unwrap_or(true) {
                return Err(e);
            }

            warn!("Docker unavailable for call_id={call_id}; falling back to subprocess mode: {e}");
            run_as_subprocess(args, timeout_duration).await
        }
    }
}

async fn run_as_subprocess(
    args: &ContainerArgs,
    timeout_duration: Duration,
) -> Result<ContainerExecution, String> {
    warn!("Docker is unavailable; running command as local subprocess fallback");
    warn!(
        "subprocess fallback does not enforce Docker CPU/memory limits (requested memory_mb={:?}, cpu_millis={:?})",
        args.memory_mb, args.cpu_millis
    );

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(&args.command);
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

async fn maybe_report_worker_result(
    call_id: &str,
    args: &ContainerArgs,
    container_name: &str,
    image: &str,
    execution: &ContainerExecution,
    run_timeout: Duration,
) -> Result<(), String> {
    if !args.report_via_quic.unwrap_or(false) {
        return Ok(());
    }

    let quic_addr = required_arg_string(
        args.orchestrator_quic_addr.as_deref(),
        "orchestrator_quic_addr",
    )?;
    let channel_id = required_arg_string(
        args.orchestrator_channel_id.as_deref(),
        "orchestrator_channel_id",
    )?;
    let session_id = required_arg_string(
        args.orchestrator_session_id.as_deref(),
        "orchestrator_session_id",
    )?;

    let addr: SocketAddr = quic_addr
        .parse()
        .map_err(|e| format!("Invalid orchestrator_quic_addr '{quic_addr}': {e}"))?;

    let output = format_output(&execution.stdout, &execution.stderr, execution.exit_code);
    let summary = build_worker_summary(call_id, execution.exit_code, &args.command);
    let report = WorkerReport::new(
        call_id.to_string(),
        container_name.to_string(),
        image.to_string(),
        args.command.clone(),
        proto::WorkerOutput {
            exit_code: execution.exit_code,
            stdout: execution.stdout.clone(),
            stderr: execution.stderr.clone(),
            output,
        },
    );

    let mut event = ChannelEvent::new(
        ChannelId::from(channel_id),
        SessionId::from(session_id),
        summary,
    );
    event.metadata = Some(
        serde_json::to_value(&report)
            .map_err(|e| format!("Failed to serialize worker report metadata: {e}"))?,
    );

    let report_timeout = Duration::from_secs(
        args.report_timeout_secs
            .unwrap_or(DEFAULT_REPORT_TIMEOUT_SECS)
            .clamp(1, MAX_REPORT_TIMEOUT_SECS),
    );

    submit_worker_report_over_quic(addr, event, report_timeout.min(run_timeout)).await
}

fn required_arg_string(value: Option<&str>, field_name: &str) -> Result<String, String> {
    let Some(raw) = value else {
        return Err(format!(
            "{field_name} is required when report_via_quic=true"
        ));
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{field_name} must not be empty when report_via_quic=true"
        ));
    }

    Ok(trimmed.to_string())
}

fn build_worker_summary(call_id: &str, exit_code: i64, command: &str) -> String {
    let status = if exit_code == 0 { "success" } else { "error" };
    format!(
        "worker_report call_id={call_id} status={status} exit_code={exit_code} command={command}"
    )
}

async fn submit_worker_report_over_quic(
    addr: SocketAddr,
    event: ChannelEvent,
    timeout_duration: Duration,
) -> Result<(), String> {
    let payload = serde_json::to_vec(&event)
        .map_err(|e| format!("Failed to serialize worker report event: {e}"))?;

    let endpoint = quinn::Endpoint::client(
        "0.0.0.0:0"
            .parse()
            .map_err(|e| format!("Failed to bind QUIC client endpoint: {e}"))?,
    )
    .map_err(|e| format!("Failed to create QUIC client endpoint: {e}"))?;

    let client_cfg = build_insecure_quic_client_config()?;
    let mut endpoint = endpoint;
    endpoint.set_default_client_config(client_cfg);

    let connect = timeout(timeout_duration, async {
        endpoint
            .connect(addr, "localhost")
            .map_err(|e| format!("Failed to prepare QUIC connect: {e}"))?
            .await
            .map_err(|e| format!("Failed to connect to orchestrator QUIC {addr}: {e}"))
    })
    .await
    .map_err(|_| format!("Timed out connecting to orchestrator QUIC {addr}"))??;

    let (mut send, mut recv) = timeout(timeout_duration, connect.open_bi())
        .await
        .map_err(|_| "Timed out opening QUIC stream for worker report".to_string())?
        .map_err(|e| format!("Failed to open QUIC stream: {e}"))?;

    timeout(timeout_duration, async {
        send.write_all(&(payload.len() as u32).to_be_bytes())
            .await
            .map_err(|e| format!("Failed to write worker report size prefix: {e}"))?;
        send.write_all(&payload)
            .await
            .map_err(|e| format!("Failed to write worker report payload: {e}"))?;
        send.finish()
            .map_err(|e| format!("Failed to finish worker report stream send: {e}"))
    })
    .await
    .map_err(|_| "Timed out sending worker report payload".to_string())??;

    let response = timeout(timeout_duration, async {
        let mut len_buf = [0_u8; 4];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(|e| format!("Failed to read worker report ACK size: {e}"))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_OUTPUT_CHARS {
            return Err("Worker report ACK exceeded expected size".to_string());
        }
        let mut buf = vec![0_u8; len];
        recv.read_exact(&mut buf)
            .await
            .map_err(|e| format!("Failed to read worker report ACK payload: {e}"))?;
        String::from_utf8(buf).map_err(|e| format!("Invalid UTF-8 worker report ACK payload: {e}"))
    })
    .await
    .map_err(|_| "Timed out waiting for worker report ACK".to_string())??;

    connect.close(0_u32.into(), b"worker-report-sent");
    endpoint.close(0_u32.into(), b"worker-report-endpoint-close");

    debug!("worker report delivered over QUIC to {addr}: {response}");
    Ok(())
}

#[derive(Debug)]
struct InsecureServerCertVerifier;

impl ServerCertVerifier for InsecureServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

fn build_insecure_quic_client_config() -> Result<quinn::ClientConfig, String> {
    let mut tls = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"openpista-quic-v1".to_vec()];
    let quic_client = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| format!("Failed to build QUIC client TLS config: {e}"))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_client)))
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

fn resolve_image(args: &ContainerArgs) -> Option<String> {
    let explicit = args.image.trim();
    if !explicit.is_empty() {
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
    #[test]
    fn required_arg_string_returns_value_or_err() {
        assert_eq!(required_arg_string(Some("val"), "field").unwrap(), "val");
        assert!(required_arg_string(Some(""), "field").is_err());
        assert!(required_arg_string(None, "field").is_err());
    }

    #[tokio::test]
    async fn submit_worker_report_over_quic_fails_invalid_addr() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        let addr = "127.0.0.1:0".parse().unwrap();
        let report = proto::WorkerReport::new(
            "call_1",
            "worker_1",
            "image",
            "cmd",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "".into(),
                stderr: "".into(),
                output: "".into(),
            },
        );
        let mut event = proto::ChannelEvent::new(
            proto::ChannelId::new("cli", "test"),
            proto::SessionId::from("ses"),
            "summary",
        );
        event.metadata = Some(serde_json::to_value(&report).unwrap());
        let result =
            submit_worker_report_over_quic(addr, event, std::time::Duration::from_secs(1)).await;
        // Since no server is listening, it should fail.
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn submit_worker_report_over_quic_success() {
        use std::sync::Arc;
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_der = rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
        let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

        let mut server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert_der],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key_der),
            )
            .unwrap();
        server_crypto.alpn_protocols = vec![b"openpista-quic-v1".to_vec()];
        let server_config = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto).unwrap(),
        ));

        let server_endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server_endpoint.local_addr().unwrap();

        tokio::spawn(async move {
            if let Some(incoming) = server_endpoint.accept().await
                && let Ok(conn) = incoming.await
                && let Ok((mut send, mut recv)) = conn.accept_bi().await
            {
                let mut len_buf = [0u8; 4];
                let _ = recv.read_exact(&mut len_buf).await;
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut body = vec![0u8; len];
                let _ = recv.read_exact(&mut body).await;
                let resp = serde_json::json!({
                    "channel_id": "cli:test",
                    "session_id": "ses",
                    "content": "ok",
                    "is_error": false
                })
                .to_string();
                let resp_bytes = resp.as_bytes();
                let resp_len = (resp_bytes.len() as u32).to_be_bytes();
                let _ = send.write_all(&resp_len).await;
                let _ = send.write_all(resp_bytes).await;
                let _ = send.finish();
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });

        let report = proto::WorkerReport::new(
            "call_1",
            "worker_1",
            "image",
            "cmd",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "".into(),
                stderr: "".into(),
                output: "".into(),
            },
        );
        let mut event = proto::ChannelEvent::new(
            proto::ChannelId::new("cli", "test"),
            proto::SessionId::from("ses"),
            "summary",
        );
        event.metadata = Some(serde_json::to_value(&report).unwrap());

        let result =
            submit_worker_report_over_quic(server_addr, event, std::time::Duration::from_secs(5))
                .await;
        assert!(
            result.is_ok(),
            "Expected QUIC report success, got {:?}",
            result
        );
    }

    use super::*;

    fn base_args(command: &str) -> ContainerArgs {
        ContainerArgs {
            image: "alpine:3".to_string(),
            skill_image: None,
            command: command.to_string(),
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
            report_via_quic: None,
            report_timeout_secs: None,
            orchestrator_quic_addr: None,
            orchestrator_channel_id: None,
            orchestrator_session_id: None,
            allow_subprocess_fallback: None,
        }
    }

    #[test]
    fn resolve_image_prefers_explicit_and_falls_back_to_skill_image() {
        let explicit = ContainerArgs {
            image: "alpine:3.20".to_string(),
            skill_image: Some("python:3.12-slim".to_string()),
            ..base_args("echo ok")
        };
        assert_eq!(resolve_image(&explicit).as_deref(), Some("alpine:3.20"));

        let fallback = ContainerArgs {
            image: String::new(),
            skill_image: Some("python:3.12-slim".to_string()),
            ..base_args("echo ok")
        };
        assert_eq!(
            resolve_image(&fallback).as_deref(),
            Some("python:3.12-slim")
        );

        let missing = ContainerArgs {
            image: String::new(),
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
        args.image = String::new();
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

    #[test]
    fn container_tool_metadata_is_stable() {
        let tool = ContainerTool::new();
        assert_eq!(tool.name(), "container.run");
        assert!(tool.description().contains("Docker"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "command");
        assert!(schema["properties"]["skill_image"].is_object());
        assert!(schema["properties"]["allow_subprocess_fallback"].is_object());
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
            .execute("call-2", serde_json::json!({"image":"","command":""}))
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
            skill_image: None,
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
            report_via_quic: None,
            report_timeout_secs: None,
            orchestrator_quic_addr: None,
            orchestrator_channel_id: None,
            orchestrator_session_id: None,
            allow_subprocess_fallback: None,
        };

        let credential = maybe_mint_task_credential(&args).expect("mint result");
        assert!(credential.is_none());
    }

    #[test]
    fn maybe_mint_task_credential_validates_env_name() {
        let args = ContainerArgs {
            image: "alpine:3".to_string(),
            skill_image: None,
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
            report_via_quic: None,
            report_timeout_secs: None,
            orchestrator_quic_addr: None,
            orchestrator_channel_id: None,
            orchestrator_session_id: None,
            allow_subprocess_fallback: None,
        };

        let err = maybe_mint_task_credential(&args).expect_err("invalid env name should fail");
        assert!(err.contains("token_env_name"));
    }

    #[test]
    fn build_shell_command_sources_credential_file() {
        let args = ContainerArgs {
            image: "alpine:3".to_string(),
            skill_image: None,
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
            report_via_quic: None,
            report_timeout_secs: None,
            orchestrator_quic_addr: None,
            orchestrator_channel_id: None,
            orchestrator_session_id: None,
            allow_subprocess_fallback: None,
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
    fn build_worker_summary_success_and_error() {
        let ok = build_worker_summary("c1", 0, "echo hi");
        assert!(ok.contains("status=success"));
        assert!(ok.contains("call_id=c1"));
        assert!(ok.contains("exit_code=0"));

        let fail = build_worker_summary("c2", 1, "false");
        assert!(fail.contains("status=error"));
        assert!(fail.contains("exit_code=1"));
    }

    #[tokio::test]
    async fn maybe_report_worker_result_skips_when_disabled() {
        let args = ContainerArgs {
            image: "alpine".into(),
            skill_image: None,
            command: "echo".into(),
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
            report_via_quic: None,
            report_timeout_secs: None,
            orchestrator_quic_addr: None,
            orchestrator_channel_id: None,
            orchestrator_session_id: None,
            allow_subprocess_fallback: None,
        };
        let execution = ContainerExecution {
            stdout: "ok".into(),
            stderr: "".into(),
            exit_code: 0,
        };
        let result = maybe_report_worker_result(
            "call-1",
            &args,
            "ctr",
            "alpine",
            &execution,
            Duration::from_secs(10),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn maybe_report_worker_result_fails_missing_addr() {
        let args = ContainerArgs {
            image: "alpine".into(),
            skill_image: None,
            command: "echo".into(),
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
            report_via_quic: Some(true),
            report_timeout_secs: None,
            orchestrator_quic_addr: None,
            orchestrator_channel_id: Some("ch".into()),
            orchestrator_session_id: Some("ses".into()),
            allow_subprocess_fallback: None,
        };
        let execution = ContainerExecution {
            stdout: "".into(),
            stderr: "".into(),
            exit_code: 0,
        };
        let result = maybe_report_worker_result(
            "call-1",
            &args,
            "ctr",
            "alpine",
            &execution,
            Duration::from_secs(10),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("orchestrator_quic_addr"));
    }

    #[tokio::test]
    async fn maybe_report_worker_result_fails_bad_addr() {
        let args = ContainerArgs {
            image: "alpine".into(),
            skill_image: None,
            command: "echo".into(),
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
            report_via_quic: Some(true),
            report_timeout_secs: Some(1),
            orchestrator_quic_addr: Some("not-a-socket-addr".into()),
            orchestrator_channel_id: Some("ch".into()),
            orchestrator_session_id: Some("ses".into()),
            allow_subprocess_fallback: None,
        };
        let execution = ContainerExecution {
            stdout: "".into(),
            stderr: "err".into(),
            exit_code: 1,
        };
        let result = maybe_report_worker_result(
            "call-2",
            &args,
            "ctr",
            "alpine",
            &execution,
            Duration::from_secs(10),
        )
        .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Invalid orchestrator_quic_addr")
        );
    }

    #[test]
    fn build_insecure_quic_client_config_returns_ok() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        let cfg = build_insecure_quic_client_config();
        assert!(cfg.is_ok());
    }

    #[test]
    fn unix_now_secs_returns_reasonable_value() {
        let ts = unix_now_secs().expect("unix_now");
        assert!(ts > 1_700_000_000);
    }
}
