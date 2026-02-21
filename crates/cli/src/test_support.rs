use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Locks process environment mutation for the entire test body.
pub(crate) fn with_locked_env<R>(run: impl FnOnce() -> R) -> R {
    let _guard = env_lock().lock().unwrap();
    run()
}

/// Set an environment variable in test contexts.
///
/// # Safety
/// These calls remain unsafe in this toolchain. Call sites should use
/// `with_locked_env` to avoid data races between parallel tests.
pub(crate) fn set_env_var(key: &str, value: &str) {
    // SAFETY: required for this toolchain's `std::env` API.
    unsafe {
        std::env::set_var(key, value);
    }
}

/// Remove an environment variable in test contexts.
///
/// # Safety
/// These calls remain unsafe in this toolchain. Call sites should use
/// `with_locked_env` to avoid data races between parallel tests.
pub(crate) fn remove_env_var(key: &str) {
    // SAFETY: required for this toolchain's `std::env` API.
    unsafe {
        std::env::remove_var(key);
    }
}
