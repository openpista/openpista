use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::slice;
use std::sync::Mutex;

#[derive(Debug, Deserialize)]
struct ToolCall {
    id: String,
    name: String,
    arguments: Value,
}

#[derive(Debug, Serialize)]
struct ToolResult {
    call_id: String,
    tool_name: String,
    output: String,
    is_error: bool,
}

static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Allocate a heap buffer with the given capacity and return a raw pointer to it.
///
/// The caller is responsible for deallocating the returned buffer (using the corresponding
/// `dealloc` function) with the same pointer and length to avoid memory leaks.
///
/// # Parameters
///
/// * `len` — Requested capacity in bytes; must be greater than zero.
///
/// # Returns
///
/// `0` if `len` is less than or equal to zero, otherwise a raw pointer to the allocated
/// buffer cast to `i32`.
///
/// # Examples
///
/// ```
/// // Safety: `alloc`/`dealloc` operate on raw pointers; the caller must ensure correct use.
/// unsafe {
///     let ptr = alloc(16);
///     assert!(ptr != 0);
///     // ... use the buffer ...
///     dealloc(ptr, 16);
/// }
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }

    let mut buf = Vec::<u8>::with_capacity(len as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as i32
}

/// Deallocates a previously allocated byte buffer obtained via this module's ABI.
///
/// If `ptr` or `len` are non-positive the function does nothing. Otherwise it reconstitutes
/// the original `Vec<u8>` from the raw pointer and capacity so Rust can drop and free the buffer.
///
/// # Examples
///
/// ```
/// // allocate a buffer through the FFI allocator, then free it
/// let ptr = unsafe { alloc(16) };
/// assert!(ptr != 0);
/// unsafe { dealloc(ptr, 16) };
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn dealloc(ptr: i32, len: i32) {
    if ptr <= 0 || len <= 0 {
        return;
    }

    unsafe {
        let _ = Vec::from_raw_parts(ptr as *mut u8, 0, len as usize);
    }
}

/// Process a JSON-encoded `ToolCall` located at the given pointer and length, produce a JSON-encoded `ToolResult`, store it in the global `OUTPUT` buffer, and return a pointer/length handle to the stored result.
///
/// The function treats non-positive `ptr` or `len` as invalid ABI input and returns an error `ToolResult`; otherwise it reads `len` bytes from `ptr`, attempts to decode a `ToolCall`, constructs an appropriate `ToolResult` (including decode or encode error payloads when necessary), serializes that result to JSON, stores the bytes in `OUTPUT`, and returns a combined handle.
///
/// # Returns
///
/// A 64-bit value where the upper 32 bits are the pointer to the stored JSON-encoded `ToolResult` and the lower 32 bits are its length.
///
/// # Examples
///
/// ```
/// use serde_json::json;
///
/// // Prepare a ToolCall payload.
/// let call = json!({
///     "id": "1",
///     "name": "hello-wasm",
///     "arguments": { "name": "alice" }
/// });
/// let bytes = serde_json::to_vec(&call).unwrap();
///
/// // Allocate, copy, and invoke `run` (unsafe FFI-style).
/// let ptr = crate::alloc(bytes.len() as i32);
/// unsafe {
///     std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
///     let handle = crate::run(ptr as i32, bytes.len() as i32);
///     let out_ptr = (handle >> 32) as *const u8;
///     let out_len = (handle & 0xffff_ffff) as usize;
///     let slice = std::slice::from_raw_parts(out_ptr, out_len);
///     let result_json = std::str::from_utf8(slice).unwrap();
///     assert!(result_json.contains("hello from wasm"));
/// }
/// ```
#[unsafe(no_mangle)]
pub extern "C" fn run(ptr: i32, len: i32) -> i64 {
    let result = if ptr <= 0 || len <= 0 {
        ToolResult {
            call_id: "invalid".to_string(),
            tool_name: "hello-wasm".to_string(),
            output: "invalid ABI input".to_string(),
            is_error: true,
        }
    } else {
        let bytes = unsafe { slice::from_raw_parts(ptr as *const u8, len as usize) };
        match serde_json::from_slice::<ToolCall>(bytes) {
            Ok(call) => {
                let who = call
                    .arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("world");
                ToolResult {
                    call_id: call.id,
                    tool_name: call.name,
                    output: format!("hello from wasm, {who}"),
                    is_error: false,
                }
            }
            Err(e) => ToolResult {
                call_id: "decode-error".to_string(),
                tool_name: "hello-wasm".to_string(),
                output: format!("invalid ToolCall payload: {e}"),
                is_error: true,
            },
        }
    };

    let encoded = serde_json::to_vec(&result).unwrap_or_else(|_| {
        b"{\"call_id\":\"encode-error\",\"tool_name\":\"hello-wasm\",\"output\":\"encode failure\",\"is_error\":true}".to_vec()
    });

    let mut out = OUTPUT.lock().expect("output lock");
    *out = encoded;

    let out_ptr = out.as_ptr() as u32;
    let out_len = out.len() as u32;
    ((out_ptr as i64) << 32) | out_len as i64
}