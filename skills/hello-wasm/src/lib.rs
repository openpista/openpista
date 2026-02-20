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

#[unsafe(no_mangle)]
pub extern "C" fn dealloc(ptr: i32, len: i32) {
    if ptr <= 0 || len <= 0 {
        return;
    }

    unsafe {
        let _ = Vec::from_raw_parts(ptr as *mut u8, 0, len as usize);
    }
}

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
