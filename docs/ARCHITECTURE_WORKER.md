# Orchestrator/Worker Architecture

## Overview
`openpista` currently uses a Docker worker execution model with optional QUIC report upload.

1. **Orchestrator**: The host daemon running `openpista start`.
2. **Worker runtime**: `container.run` creates an isolated, short-lived Docker container via `bollard`, executes the requested shell command, collects stdout/stderr and exit code, and optionally sends a report event back over QUIC.

## Current implementation

### 1. Container execution (`crates/tools/src/container.rs`)
- The user command is executed as-is via `sh -lc <command>`.
- Container lifecycle is: create -> optional token injection -> start -> `docker.wait_container` -> `docker.logs` -> cleanup.
- No `openpista worker` subcommand override is used in the current path.

### 2. Task token injection
- When enabled, `container.run` writes a short-lived credential script into `/run/secrets/.openpista_task_env`.
- The command is prefixed to source that token file before executing the user command.
- This path currently uses the `.openpista_task_env` file approach, not `OPENPISTA_WORKER_*` environment variable injection.

### 3. QUIC worker reporting
- When `report_via_quic=true`, `container.run` builds a `WorkerReport`, wraps it in `ChannelEvent.metadata`, and sends it to the orchestrator over QUIC.
- The orchestrator receives the event and persists the worker report via `AgentRuntime::record_worker_report`.

### 4. Scope note
- The previously documented streaming `WorkerEnvelope`, `openpista worker` command override, and `oneshot::Receiver` task map are not part of the currently shipped implementation.
