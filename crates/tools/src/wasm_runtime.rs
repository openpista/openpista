use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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
/// ```
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
pub async fn run_wasm_skill(req: WasmRunRequest) -> Result<ToolResult, String>;
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
/// ```
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
    memory
        .write(&mut store, payload_ptr as usize, &payload)
        .map_err(|e| format!("Failed writing ToolCall into WASM memory: {e}"))?;

    let done = Arc::new(AtomicBool::new(false));
    let timeout_done = done.clone();
    let timeout_engine = engine.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(timeout_secs));
        if !timeout_done.load(Ordering::SeqCst) {
            timeout_engine.increment_epoch();
        }
    });

    let run_result = run.call(&mut store, (payload_ptr, payload_len_i32));
    done.store(true, Ordering::SeqCst);

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
        let _ = dealloc_fn.call(
            &mut store,
            (
                i32::try_from(result_ptr).unwrap_or_default(),
                i32::try_from(result_len).unwrap_or_default(),
            ),
        );
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
/// ```
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
/// ```
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
}