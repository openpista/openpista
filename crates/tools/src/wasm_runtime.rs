use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use proto::{ToolCall, ToolResult};
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const WASM_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const WASM_FUEL_LIMIT: u64 = 50_000_000;
const PIPE_CAPACITY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct WasmRunRequest {
    pub call_id: String,
    pub skill_name: String,
    pub workspace_dir: PathBuf,
    pub arguments: serde_json::Value,
    pub timeout_secs: Option<u64>,
}

struct WasmStoreData {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

/// Runs a WASM-backed skill request by executing the synchronous runner on a blocking thread.
///
/// On success returns the tool's `ToolResult`; on failure returns a descriptive error `String`.
///
/// # Examples
///
/// ```ignore
/// use std::path::PathBuf;
/// use serde_json::json;
/// // Construct a request (fields shown for illustration)
/// let req = WasmRunRequest {
///     call_id: "1".to_string(),
///     skill_name: "echo".to_string(),
///     workspace_dir: PathBuf::from("/tmp/workspace"),
///     arguments: json!({ "message": "hi" }),
///     timeout_secs: Some(5),
/// };
/// // Call asynchronously (in an async context)
/// // let result = run_wasm_skill(req).await;
/// ```
pub async fn run_wasm_skill(req: WasmRunRequest) -> Result<ToolResult, String> {
    tokio::task::spawn_blocking(move || run_wasm_skill_sync(req))
        .await
        .map_err(|e| format!("WASM task join error: {e}"))?
}

/// Executes a WASM skill described by the given `WasmRunRequest` and returns its `ToolResult`.
///
/// The function locates the skill's WASM module under the workspace, configures a WASI-enabled
/// Wasmtime environment with resource limits and a timeout watchdog, serializes the `ToolCall`
/// into WASM memory, invokes the module's `run` export, and deserializes the returned `ToolResult`.
/// Captured stdout and stderr from the WASM instance are appended to `ToolResult.output` when present.
/// Errors are returned as human-readable strings.
///
/// # Returns
///
/// `Ok(ToolResult)` on success; `Err(String)` containing a descriptive error message on failure.
///
/// # Examples
///
/// ```ignore
/// use std::path::PathBuf;
/// let req = WasmRunRequest {
///     call_id: "call-1".to_string(),
///     skill_name: "example_skill".to_string(),
///     workspace_dir: PathBuf::from("/tmp/workspace"),
///     arguments: serde_json::json!({}),
///     timeout_secs: Some(5),
/// };
/// let _ = run_wasm_skill_sync(req);
/// ```
fn run_wasm_skill_sync(req: WasmRunRequest) -> Result<ToolResult, String> {
    let module_path = resolve_wasm_module_path(&req.workspace_dir, &req.skill_name);
    if !module_path.exists() {
        return Err(format!(
            "WASM module not found for skill '{}': {}",
            req.skill_name,
            module_path.display()
        ));
    }

    let timeout_secs = req
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, 300);

    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);

    let engine = Engine::new(&config).map_err(|e| format!("Failed to create WASM engine: {e}"))?;
    let module = Module::from_file(&engine, &module_path)
        .map_err(|e| format!("Failed to load WASM module: {e}"))?;

    let stdout_pipe = MemoryOutputPipe::new(PIPE_CAPACITY_BYTES);
    let stderr_pipe = MemoryOutputPipe::new(PIPE_CAPACITY_BYTES);

    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.stdout(stdout_pipe.clone());
    wasi_builder.stderr(stderr_pipe.clone());
    wasi_builder
        .preopened_dir(
            &req.workspace_dir,
            "/workspace",
            DirPerms::READ,
            FilePerms::READ,
        )
        .map_err(|e| format!("Failed to configure WASI preopened dir: {e}"))?;

    let store_data = WasmStoreData {
        wasi: wasi_builder.build_p1(),
        limits: StoreLimitsBuilder::new()
            .memory_size(WASM_MEMORY_LIMIT_BYTES)
            .build(),
    };

    let mut store = Store::new(&engine, store_data);
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(WASM_FUEL_LIMIT)
        .map_err(|e| format!("Failed to set WASM fuel: {e}"))?;
    store.set_epoch_deadline(1);

    let mut linker: Linker<WasmStoreData> = Linker::new(&engine);
    p1::add_to_linker_sync(&mut linker, |state| &mut state.wasi)
        .map_err(|e| format!("Failed to link WASI: {e}"))?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("Failed to instantiate WASM module: {e}"))?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| "WASM export 'memory' not found".to_string())?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "alloc")
        .map_err(|e| format!("WASM export 'alloc' not found or invalid: {e}"))?;
    let run = instance
        .get_typed_func::<(i32, i32), i64>(&mut store, "run")
        .map_err(|e| format!("WASM export 'run' not found or invalid: {e}"))?;
    let dealloc = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")
        .ok();

    let tool_call = ToolCall {
        id: req.call_id,
        name: req.skill_name,
        arguments: req.arguments,
    };
    let payload = serde_json::to_vec(&tool_call)
        .map_err(|e| format!("Failed to serialize ToolCall for WASM ABI: {e}"))?;
    let payload_len_i32 =
        i32::try_from(payload.len()).map_err(|_| "WASM ABI input too large".to_string())?;

    let payload_ptr = alloc
        .call(&mut store, payload_len_i32)
        .map_err(|e| format!("WASM alloc failed: {e}"))?;
    if payload_ptr == 0 {
        return Err("WASM alloc returned 0".to_string());
    }
    memory
        .write(&mut store, payload_ptr as usize, &payload)
        .map_err(|e| format!("Failed writing ToolCall into WASM memory: {e}"))?;

    let timeout_engine = engine.clone();
    let (timeout_tx, timeout_rx) = mpsc::channel::<()>();
    let timeout_thread = std::thread::spawn(move || {
        match timeout_rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Err(RecvTimeoutError::Timeout) => {
                timeout_engine.increment_epoch();
            }
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
        }
    });

    let run_result = run.call(&mut store, (payload_ptr, payload_len_i32));
    let _ = timeout_tx.send(());
    timeout_thread
        .join()
        .map_err(|_| "WASM timeout thread panicked".to_string())?;

    let packed = run_result.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("all fuel consumed") {
            "WASM execution aborted: fuel exhausted".to_string()
        } else if msg.to_ascii_lowercase().contains("interrupt") {
            format!("WASM execution timed out after {timeout_secs}s")
        } else {
            format!("WASM execution failed: {msg}")
        }
    })?;

    let (result_ptr, result_len) = unpack_abi_return(packed)?;
    let mut result_buf = vec![0_u8; result_len];
    memory
        .read(&store, result_ptr, &mut result_buf)
        .map_err(|e| format!("Failed reading ToolResult from WASM memory: {e}"))?;

    if let Some(dealloc_fn) = dealloc {
        let _ = dealloc_fn.call(&mut store, (payload_ptr, payload_len_i32));
    }

    let mut tool_result: ToolResult = serde_json::from_slice(&result_buf)
        .map_err(|e| format!("Invalid ToolResult JSON returned from WASM skill: {e}"))?;

    let stdout = String::from_utf8_lossy(stdout_pipe.contents().as_ref())
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(stderr_pipe.contents().as_ref())
        .trim()
        .to_string();
    if !stdout.is_empty() || !stderr.is_empty() {
        let mut out = tool_result.output;
        if !stdout.is_empty() {
            out.push_str("\n\nstdout:\n");
            out.push_str(&stdout);
        }
        if !stderr.is_empty() {
            out.push_str("\n\nstderr:\n");
            out.push_str(&stderr);
        }
        tool_result.output = out;
    }

    Ok(tool_result)
}

/// Build the filesystem path to a skill's WASM module inside a workspace.
///
/// The path is workspace_dir/skills/{skill_name}/main.wasm.
///
/// # Examples
///
/// ```ignore
/// use std::path::Path;
/// let p = super::resolve_wasm_module_path(Path::new("/tmp/workspace"), "echo");
/// assert_eq!(p, Path::new("/tmp/workspace").join("skills").join("echo").join("main.wasm"));
/// ```
fn resolve_wasm_module_path(workspace_dir: &Path, skill_name: &str) -> PathBuf {
    workspace_dir
        .join("skills")
        .join(skill_name)
        .join("main.wasm")
}

/// Decode a packed 64-bit ABI return value into a (pointer, length) pair.
///
/// The input `packed` encodes the pointer in the upper 32 bits and the length in the lower 32 bits.
/// Returns the pointer and length as `usize` when `length > 0`. Returns an `Err` with a descriptive
/// message if the decoded length is zero.
///
/// # Examples
///
/// ```ignore
/// let packed: i64 = ((4096u64 << 32) | 128u64) as i64;
/// let (ptr, len) = unpack_abi_return(packed).expect("should decode");
/// assert_eq!(ptr, 4096usize);
/// assert_eq!(len, 128usize);
/// ```
fn unpack_abi_return(packed: i64) -> Result<(usize, usize), String> {
    let ptr = (packed >> 32) as u32;
    let len = packed as u32;
    if len == 0 {
        return Err("WASM ABI returned empty ToolResult buffer".to_string());
    }
    Ok((ptr as usize, len as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_skill_module(workspace: &Path, skill_name: &str, bytes: &[u8]) {
        let module_dir = workspace.join("skills").join(skill_name);
        fs::create_dir_all(&module_dir).expect("create module directory");
        fs::write(module_dir.join("main.wasm"), bytes).expect("write wasm module");
    }

    fn write_skill_module_from_wat(workspace: &Path, skill_name: &str, wat_src: &str) {
        let wasm = wat::parse_str(wat_src).expect("parse wat");
        write_skill_module(workspace, skill_name, &wasm);
    }

    #[test]
    fn resolve_wasm_module_path_uses_skills_layout() {
        let workspace = PathBuf::from("/tmp/workspace");
        let path = resolve_wasm_module_path(&workspace, "hello-wasm");
        assert_eq!(
            path,
            PathBuf::from("/tmp/workspace/skills/hello-wasm/main.wasm")
        );
    }

    #[test]
    fn resolve_wasm_module_path_with_nested_workspace() {
        let workspace = PathBuf::from("/home/user/.openpista/workspace");
        let path = resolve_wasm_module_path(&workspace, "deploy");
        assert_eq!(
            path,
            PathBuf::from("/home/user/.openpista/workspace/skills/deploy/main.wasm")
        );
    }

    #[test]
    fn resolve_wasm_module_path_with_special_chars_in_skill_name() {
        let workspace = PathBuf::from("/tmp/ws");
        let path = resolve_wasm_module_path(&workspace, "my-skill_v2");
        assert_eq!(path, PathBuf::from("/tmp/ws/skills/my-skill_v2/main.wasm"));
    }

    #[test]
    fn unpack_abi_return_decodes_pointer_and_length() {
        let ptr = 4096_u32;
        let len = 128_u32;
        let packed = ((ptr as i64) << 32) | (len as i64);
        let decoded = unpack_abi_return(packed).expect("decode");
        assert_eq!(decoded, (4096, 128));
    }

    #[test]
    fn unpack_abi_return_rejects_empty_buffer() {
        let packed = (1_i64) << 32;
        let err = unpack_abi_return(packed).expect_err("empty len should fail");
        assert!(err.contains("empty ToolResult"));
    }

    #[test]
    fn unpack_abi_return_handles_large_pointer() {
        let ptr = u32::MAX;
        let len = 1_u32;
        let packed = ((ptr as i64) << 32) | (len as i64);
        let (p, l) = unpack_abi_return(packed).expect("large ptr");
        assert_eq!(p, u32::MAX as usize);
        assert_eq!(l, 1);
    }

    #[test]
    fn unpack_abi_return_handles_large_length() {
        let ptr = 1_u32;
        let len = u32::MAX;
        let packed = ((ptr as i64) << 32) | (len as i64);
        let (p, l) = unpack_abi_return(packed).expect("large len");
        assert_eq!(p, 1);
        assert_eq!(l, u32::MAX as usize);
    }

    #[test]
    fn unpack_abi_return_zero_pointer_with_nonzero_len() {
        let packed = 42_i64; // ptr=0, len=42
        let (p, l) = unpack_abi_return(packed).expect("zero ptr");
        assert_eq!(p, 0);
        assert_eq!(l, 42);
    }

    #[tokio::test]
    async fn run_wasm_skill_returns_error_for_missing_module() {
        let req = WasmRunRequest {
            call_id: "test-missing".to_string(),
            skill_name: "nonexistent-skill".to_string(),
            workspace_dir: PathBuf::from("/tmp/no-such-workspace-xyz"),
            arguments: serde_json::json!({}),
            timeout_secs: Some(5),
        };
        let result = run_wasm_skill(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("WASM module not found"));
        assert!(err.contains("nonexistent-skill"));
    }

    #[tokio::test]
    async fn run_wasm_skill_error_for_missing_module_with_default_timeout() {
        let req = WasmRunRequest {
            call_id: "test-no-timeout".to_string(),
            skill_name: "missing".to_string(),
            workspace_dir: PathBuf::from("/tmp/absent"),
            arguments: serde_json::json!({"key": "value"}),
            timeout_secs: None,
        };
        let result = run_wasm_skill(req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("WASM module not found"));
    }

    #[tokio::test]
    async fn run_wasm_skill_error_includes_module_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let req = WasmRunRequest {
            call_id: "test-path".to_string(),
            skill_name: "echo".to_string(),
            workspace_dir: tmp.path().to_path_buf(),
            arguments: serde_json::json!({}),
            timeout_secs: Some(1),
        };
        let result = run_wasm_skill(req).await;
        let err = result.unwrap_err();
        assert!(
            err.contains("main.wasm"),
            "error should mention the module path: {err}"
        );
    }

    #[tokio::test]
    async fn run_wasm_skill_invalid_wasm_binary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skill_dir = tmp.path().join("skills/bad-wasm");
        std::fs::create_dir_all(&skill_dir).expect("create dir");
        std::fs::write(skill_dir.join("main.wasm"), b"not a valid wasm module").expect("write");

        let req = WasmRunRequest {
            call_id: "test-bad-wasm".to_string(),
            skill_name: "bad-wasm".to_string(),
            workspace_dir: tmp.path().to_path_buf(),
            arguments: serde_json::json!({}),
            timeout_secs: Some(5),
        };
        let result = run_wasm_skill(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Failed to load WASM module"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn wasm_run_request_fields_are_accessible() {
        let req = WasmRunRequest {
            call_id: "c1".to_string(),
            skill_name: "skill".to_string(),
            workspace_dir: PathBuf::from("/ws"),
            arguments: serde_json::json!({"a": 1}),
            timeout_secs: Some(10),
        };
        assert_eq!(req.call_id, "c1");
        assert_eq!(req.skill_name, "skill");
        assert_eq!(req.workspace_dir, PathBuf::from("/ws"));
        assert_eq!(req.arguments["a"], 1);
        assert_eq!(req.timeout_secs, Some(10));
    }

    #[test]
    fn wasm_run_request_clone() {
        let req = WasmRunRequest {
            call_id: "c2".to_string(),
            skill_name: "s".to_string(),
            workspace_dir: PathBuf::from("/w"),
            arguments: serde_json::json!(null),
            timeout_secs: None,
        };
        let cloned = req.clone();
        assert_eq!(cloned.call_id, req.call_id);
        assert_eq!(cloned.timeout_secs, None);
    }

    #[test]
    fn run_wasm_skill_sync_reports_missing_module() {
        let workspace = temp_workspace_dir("wasm-missing");
        fs::create_dir_all(&workspace).expect("create workspace");

        let req = WasmRunRequest {
            call_id: "call-1".to_string(),
            skill_name: "missing-skill".to_string(),
            workspace_dir: workspace.clone(),
            arguments: json!({}),
            timeout_secs: Some(1),
        };
        let err = run_wasm_skill_sync(req).expect_err("missing module should fail");
        assert!(err.contains("WASM module not found"));

        fs::remove_dir_all(&workspace).expect("cleanup workspace");
    }

    #[test]
    fn run_wasm_skill_sync_reports_missing_memory_export_for_minimal_module() {
        let workspace = temp_workspace_dir("wasm-minimal");
        // Minimal valid wasm binary: magic + version, with no exports.
        let minimal_wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        write_skill_module(&workspace, "minimal-skill", &minimal_wasm);

        let req = WasmRunRequest {
            call_id: "call-2".to_string(),
            skill_name: "minimal-skill".to_string(),
            workspace_dir: workspace.clone(),
            arguments: json!({ "input": "hello" }),
            timeout_secs: Some(2),
        };
        let err = run_wasm_skill_sync(req).expect_err("module without memory must fail");
        assert!(err.contains("WASM export 'memory' not found"));

        fs::remove_dir_all(&workspace).expect("cleanup workspace");
    }

    #[tokio::test]
    async fn run_wasm_skill_async_propagates_sync_failure() {
        let workspace = temp_workspace_dir("wasm-async");
        fs::create_dir_all(&workspace).expect("create workspace");

        let req = WasmRunRequest {
            call_id: "call-async".to_string(),
            skill_name: "missing-async".to_string(),
            workspace_dir: workspace.clone(),
            arguments: json!({ "mode": "async" }),
            timeout_secs: Some(3),
        };
        let err = run_wasm_skill(req)
            .await
            .expect_err("async wrapper should surface missing module failure");
        assert!(err.contains("WASM module not found"));

        fs::remove_dir_all(&workspace).expect("cleanup workspace");
    }

    #[test]
    fn run_wasm_skill_sync_reports_empty_abi_when_run_returns_zero() {
        let workspace = temp_workspace_dir("wasm-zero-result");
        let wat = r#"
            (module
              (memory (export "memory") 1)
              (func (export "alloc") (param i32) (result i32)
                i32.const 16)
              (func (export "run") (param i32 i32) (result i64)
                i64.const 0)
            )
        "#;
        write_skill_module_from_wat(&workspace, "zero-result-skill", wat);

        let req = WasmRunRequest {
            call_id: "call-zero".to_string(),
            skill_name: "zero-result-skill".to_string(),
            workspace_dir: workspace.clone(),
            arguments: json!({ "case": "abi-empty" }),
            timeout_secs: Some(2),
        };
        let err = run_wasm_skill_sync(req).expect_err("zero packed result must fail ABI decode");
        assert!(err.contains("WASM ABI returned empty ToolResult buffer"));

        fs::remove_dir_all(&workspace).expect("cleanup workspace");
    }

    #[test]
    fn wasm_run_request_debug() {
        let req = WasmRunRequest {
            call_id: "c3".to_string(),
            skill_name: "dbg".to_string(),
            workspace_dir: PathBuf::from("/d"),
            arguments: serde_json::json!({}),
            timeout_secs: Some(30),
        };
        let debug_str = format!("{:?}", req);
        assert!(debug_str.contains("WasmRunRequest"));
        assert!(debug_str.contains("dbg"));
    }

    #[test]
    fn default_constants_are_reasonable() {
        assert_eq!(DEFAULT_TIMEOUT_SECS, 30);
        let wasm_memory_limit_bytes = std::hint::black_box(WASM_MEMORY_LIMIT_BYTES);
        let wasm_fuel_limit = std::hint::black_box(WASM_FUEL_LIMIT);
        let pipe_capacity_bytes = std::hint::black_box(PIPE_CAPACITY_BYTES);
        assert!(wasm_memory_limit_bytes > 0);
        assert!(wasm_fuel_limit > 0);
        assert!(pipe_capacity_bytes > 0);
    }
}
