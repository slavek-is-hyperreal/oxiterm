//! State dispatcher to external app servers.
//!
//! Sends state snapshots and action payloads to configured application backends
//! in a fire-and-forget thread, receiving and applying JSON state patches to client sessions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

/// The JSON payload dispatched to the application backend on events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchPayload {
    /// The action event string trigger (e.g. `"set:tab=info"`).
    pub action: String,
    /// Currently resolved session state keys and values.
    pub state: HashMap<String, String>,
    /// Unique identifier of the connection session.
    pub session_id: usize,
    /// Username of the authenticated user.
    #[serde(default)]
    pub username: Option<String>,
    /// Authentication method used.
    #[serde(default)]
    pub auth_method: Option<String>,
}

/// Dispatcher responsible for sending session state updates to the app server.
pub struct AppDispatcher {
    /// Destination endpoint URL of the application server events channel.
    app_server_url: String,
}

impl AppDispatcher {
    /// Creates a new dispatcher targeting the given URL.
    pub fn new(app_server_url: String) -> Self {
        Self { app_server_url }
    }

    /// Dispatches the payload in a spawned thread to avoid blocking the event loop.
    ///
    /// Parses responses returning a state patch JSON and applies them to the session.
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
            username: None,
            auth_method: None,
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

    #[test]
    fn test_16_dispatch_payload_has_identity() {
        let payload = DispatchPayload {
            action: "click".to_string(),
            state: HashMap::new(),
            session_id: 123,
            username: Some("test_user".to_string()),
            auth_method: Some("TrustedHeader".to_string()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"username\":\"test_user\""));
        assert!(json.contains("\"auth_method\":\"TrustedHeader\""));

        let deserialized: DispatchPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.username, Some("test_user".to_string()));
        assert_eq!(deserialized.auth_method, Some("TrustedHeader".to_string()));
    }

    #[test]
    fn test_dispatch_unreachable_does_not_panic() {
        let d = AppDispatcher::new("http://127.0.0.1:1/unreachable".to_string());
        let payload = make_payload("toggle:flag", 0);
        let reg = crate::session::SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        d.dispatch(payload, session);
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
        
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).unwrap();
        
        let body = "{\"new_key\":\"patched_val\"}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(2) {
            if session.state.read().get("new_key").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        
        let state = session.state.read();
        assert_eq!(state.get("new_key"), Some(&crate::state::StateValue::Str("patched_val".to_string())));
    }
}
