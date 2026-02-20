# Orchestrator/Worker Architecture

## Overview
To isolate task execution and allow robust reporting, `openpista` will use an Orchestrator/Worker architecture via Docker and QUIC.

1. **Orchestrator**: The host agent running `openpistacrab start`. It listens for QUIC connections (e.g., port 4435 for internal workers).
2. **Worker**: A short-lived Docker container spawned via `bollard`. It runs `openpistacrab worker --task-spec ...` instead of directly executing user shell scripts.

## Components to Update

### 1. `WorkerEnvelope` Protocol
Add to `crates/proto/src`:
```rust
pub enum WorkerEnvelope {
    Started,
    StdoutChunk(String),
    StderrChunk(String),
    Finished { exit_code: i32, output: String },
    Failed(String),
}
```

### 2. Worker Subcommand (`crates/cli/src/main.rs`)
Add a new `worker` subcommand that reads the task payload, executes the target skill/command, and streams output via a `quinn` client connection to the orchestrator.

### 3. Orchestrator Listener
The main daemon must listen for internal worker connections, validate them using `OPENPISTA_WORKER_REPORT_TOKEN`, and map streams to a `DashMap<task_id, oneshot::Sender<ToolResult>>`.

### 4. `container.run` Upgrade (`crates/tools/src/container.rs`)
- Inject ENVs into the container:
  - `OPENPISTA_WORKER_REPORT_ADDR`
  - `OPENPISTA_WORKER_TASK_ID`
  - `OPENPISTA_WORKER_REPORT_TOKEN`
- Override container command to launch `openpistacrab worker`.
- Change `docker.wait_container` + `docker.logs` polling to an async await on the orchestrator's `oneshot::Receiver` mapped by `task_id`.
