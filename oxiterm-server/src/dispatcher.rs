//! `AppDispatcher` — sends OxiTerm state/input events to an external app server.
//!
//! When `OXITERM_APP_SERVER` is configured, OxiTerm POSTs a JSON payload to the
//! external URL. The payload carries the current state snapshot and the triggering
//! action so that a backend can react (e.g. store form data, run business logic).
//!
//! # Contract
//! - One public struct: `AppDispatcher`.
//! - One public method: `dispatch`.
//! - No long-lived threads; each `dispatch` call is fire-and-forget on a std thread.
//! - No new Cargo.toml deps (uses `ureq` already present).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

/// The JSON body sent to the external app server on each dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchPayload {
    /// The HTMX action string that triggered this dispatch (e.g. `"set:tab=info"`).
    pub action: String,
    /// Current values for all known state keys (stringified).
    pub state: HashMap<String, String>,
    /// Session identifier for correlating events on the server side.
    pub session_id: usize,
}

/// Sends state events to an external app server (fire-and-forget).
pub struct AppDispatcher {
    /// URL of the external app server (e.g. `http://localhost:3000/events`).
    app_server_url: String,
}

impl AppDispatcher {
    /// Create from the configured URL.
    pub fn new(app_server_url: String) -> Self {
        Self { app_server_url }
    }

    /// Fire-and-forget POST of `payload` to the configured URL.
    ///
    /// Spawns a std thread so the event loop is never blocked.
    pub fn dispatch(&self, payload: DispatchPayload, session: std::sync::Arc<crate::session::ClientSession>) {
        let url = self.app_server_url.clone();
        std::thread::spawn(move || {
            info!("AppDispatcher: POST {} action={}", url, payload.action);
            match ureq::post(&url).send_json(&payload) {
                Ok(resp) => {
                    info!("AppDispatcher: response {}", resp.status());
                    if resp.status() == 200 {
                        match resp.into_json::<serde_json::Value>() {
                            Ok(json) => {
                                info!("AppDispatcher: successfully parsed state patch JSON");
                                session.apply_state_patch(json);
                                let _ = session.event_tx.try_send(oxiterm_proto::input::InputEvent::StatePatched);
                            }
                            Err(e) => {
                                warn!("AppDispatcher: failed to parse response JSON: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("AppDispatcher: POST failed: {}", e);
                }
            }
        });
    }
}

// ─── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_payload(action: &str, session_id: usize) -> DispatchPayload {
        let mut state = HashMap::new();
        state.insert("tab".to_string(), "info".to_string());
        state.insert("count".to_string(), "3".to_string());
        DispatchPayload {
            action: action.to_string(),
            state,
            session_id,
        }
    }

    #[test]
    fn test_payload_serializes() {
        let payload = make_payload("set:tab=info", 42);
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"action\":\"set:tab=info\""));
        assert!(json.contains("\"session_id\":42"));
        assert!(json.contains("\"tab\":\"info\""));
    }

    #[test]
    fn test_payload_deserializes() {
        let raw = r#"{"action":"inc:counter","state":{"counter":"1"},"session_id":7}"#;
        let payload: DispatchPayload = serde_json::from_str(raw).unwrap();
        assert_eq!(payload.action, "inc:counter");
        assert_eq!(payload.session_id, 7);
        assert_eq!(payload.state.get("counter").unwrap(), "1");
    }

    #[test]
    fn test_dispatcher_new() {
        let d = AppDispatcher::new("http://localhost:3000/events".to_string());
        assert_eq!(d.app_server_url, "http://localhost:3000/events");
    }

    /// Verifies that `dispatch` does not panic when URL is unreachable.
    /// (The spawned thread will fail gracefully and log a warning.)
    #[test]
    fn test_dispatch_unreachable_does_not_panic() {
        let d = AppDispatcher::new("http://127.0.0.1:1/unreachable".to_string());
        let payload = make_payload("toggle:flag", 0);
        let reg = crate::session::SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        // Should not panic — failure is logged inside the spawned thread.
        d.dispatch(payload, session);
        // Give thread a moment to attempt and fail.
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    #[test]
    fn test_dispatch_app_server_response_patch() {
        use std::net::TcpListener;
        use std::io::{Write, Read};
        
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        
        let d = AppDispatcher::new(format!("http://127.0.0.1:{}/events", port));
        let payload = make_payload("click", 123);
        
        let reg = crate::session::SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        
        d.dispatch(payload, session.clone());
        
        // Accept the mock server connection
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).unwrap();
        
        // Write mock response with state patch JSON
        let body = "{\"new_key\":\"patched_val\"}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        
        // Wait up to 2 seconds for state patch to be applied
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(2) {
            if session.state.read().get("new_key").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        
        // Verify state patch was applied to session
        let state = session.state.read();
        assert_eq!(state.get("new_key"), Some(&crate::state::StateValue::Str("patched_val".to_string())));
    }
}

