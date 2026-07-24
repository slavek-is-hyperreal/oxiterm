//! Shared test environment guard for serializing env-var mutations across all tests.
//!
//! Both `web.rs` and `dispatcher.rs` import this so there is exactly one `ENV_LOCK`
//! in the process, preventing races on env-var reads during concurrent `cargo test`.

use std::collections::HashMap;

pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that holds `ENV_LOCK` for the duration of a test and restores
/// every env-var it touched on drop.
pub struct EnvGuard {
    pub _lock: std::sync::MutexGuard<'static, ()>,
    original_values: HashMap<String, Option<String>>,
}

impl EnvGuard {
    pub fn lock_and_set(vars: &[(&str, Option<&str>)]) -> Self {
        let lock = ENV_LOCK.lock().unwrap();
        let mut original_values = HashMap::new();
        for &(key, val) in vars {
            let orig = std::env::var(key).ok();
            original_values.insert(key.to_string(), orig);
            if let Some(v) = val {
                std::env::set_var(key, v);
            } else {
                std::env::remove_var(key);
            }
        }
        Self { _lock: lock, original_values }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, val) in &self.original_values {
            if let Some(v) = val {
                std::env::set_var(key, v);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}
