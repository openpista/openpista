//! Docker Engine API operations for container management.

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
use std::collections::HashMap;

use super::lifecycle::{build_shell_command, build_task_credential_script};
use super::{
    ContainerArgs, ContainerExecution, DEFAULT_CPU_MILLIS, DEFAULT_MEMORY_MB, TOKEN_ENV_FILE_NAME,
    TOKEN_MOUNT_DIR, TaskCredential, non_empty,
};

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
pub(super) async fn run_container(
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

pub(super) async fn upload_task_credential(
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

pub(super) fn build_task_credential_archive(
    credential: &TaskCredential,
) -> Result<Vec<u8>, String> {
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

pub(super) async fn cleanup_container(docker: &Docker, container_name: &str) -> Result<(), String> {
    docker
        .remove_container(
            container_name,
            Some(RemoveContainerOptionsBuilder::default().force(true).build()),
        )
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to remove container '{container_name}': {e}"))
}
