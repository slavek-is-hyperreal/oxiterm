//! HTTP implementation of the [`ActionHandler`] seam: POSTs actions to the App Server.
//!
//! Sends state snapshots and action payloads to the configured application backend in a
//! fire-and-forget thread. The JSON response goes back through the [`ActionReply`], i.e. into the
//! session's event queue; it is never applied from this thread.

use tracing::{info, warn};

pub use crate::action::DispatchPayload;
use crate::action::{ActionHandler, ActionReply};

/// Sends session state updates to the app server over HTTP (thread + ureq per call).
pub struct HttpActionHandler {
    /// Destination endpoint URL of the application server events channel.
    app_server_url: String,
}

impl HttpActionHandler {
    /// Creates a new handler targeting the given URL.
    pub fn new(app_server_url: String) -> Self {
        Self { app_server_url }
    }
}

impl ActionHandler for HttpActionHandler {
    /// Dispatches the payload in a spawned thread to avoid blocking the event loop. A 200
    /// response with a JSON body is handed to `reply`.
    fn dispatch(&self, payload: DispatchPayload, reply: ActionReply) {
        let url = self.app_server_url.clone();
        std::thread::spawn(move || {
            info!("HttpActionHandler: POST {} action={}", url, payload.action);
            let mut req = ureq::post(&url);
            if let Ok(token) = std::env::var("OXITERM_APP_TOKEN") {
                if !token.is_empty() {
                    req = req.set("Authorization", &format!("Bearer {}", token));
                }
            }
            match req.send_json(&payload) {
                Ok(resp) => {
                    info!("HttpActionHandler: response {}", resp.status());
                    if resp.status() == 200 {
                        match resp.into_json::<serde_json::Value>() {
                            Ok(json) => {
                                info!("HttpActionHandler: successfully parsed state patch JSON");
                                reply.send(json);
                            }
                            Err(e) => {
                                warn!("HttpActionHandler: failed to parse response JSON: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("HttpActionHandler: POST failed: {}", e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    // Use the single shared ENV_LOCK from crate::test_env to avoid races with web.rs tests.
    use crate::test_env::EnvGuard;

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
            app_token: None,
        }
    }

    /// A reply that hands the response to the test instead of a session queue.
    fn capture_reply() -> (ActionReply, mpsc::Receiver<serde_json::Value>) {
        let (tx, rx) = mpsc::channel();
        (ActionReply::new(move |v| { let _ = tx.send(v); }), rx)
    }

    /// Accepts one request, returns its raw text and the stream to answer on.
    fn accept_request(listener: &TcpListener) -> (std::net::TcpStream, String) {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let n = stream.read(&mut buf).unwrap();
        (stream, String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    fn respond_json(mut stream: std::net::TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
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
    fn test_payload_deserializes_round_trip() {
        // Verify full round-trip including identity fields.
        let raw = r#"{"action":"inc:counter","state":{"counter":"1"},"session_id":7,"username":"alice","auth_method":"TrustedHeader"}"#;
        let payload: DispatchPayload = serde_json::from_str(raw).unwrap();
        assert_eq!(payload.action, "inc:counter");
        assert_eq!(payload.session_id, 7);
        assert_eq!(payload.state.get("counter").unwrap(), "1");
        assert_eq!(payload.username.as_deref(), Some("alice"));
        assert_eq!(payload.auth_method.as_deref(), Some("TrustedHeader"));
    }

    #[test]
    fn test_handler_new() {
        let d = HttpActionHandler::new("http://localhost:3000/events".to_string());
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
            app_token: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"username\":\"test_user\""));
        assert!(json.contains("\"auth_method\":\"TrustedHeader\""));
    }

    #[test]
    fn test_dispatch_unreachable_does_not_panic() {
        // Dispatching to an unreachable server is a no-op: never panics, never replies.
        let d = HttpActionHandler::new("http://127.0.0.1:1/events".to_string());
        let (reply, rx) = capture_reply();
        d.dispatch(make_payload("click", 42), reply);
        // The thread drops the reply without sending: the channel disconnects.
        assert!(rx.recv_timeout(std::time::Duration::from_secs(3)).is_err());
    }

    #[test]
    fn test_dispatch_app_server_response_goes_to_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = HttpActionHandler::new(format!("http://127.0.0.1:{}/events", port));
        let (reply, rx) = capture_reply();
        d.dispatch(make_payload("click", 123), reply);

        let (stream, _req) = accept_request(&listener);
        respond_json(stream, "{\"new_key\":\"patched_val\"}");

        let patch = rx.recv_timeout(std::time::Duration::from_secs(2)).expect("response patch delivered");
        assert_eq!(patch, serde_json::json!({"new_key": "patched_val"}));
    }

    #[test]
    fn test_22_dispatcher_sends_bearer_auth() {
        // Uses the shared ENV_LOCK so this test cannot race with web.rs tests
        // that also mutate OXITERM_APP_TOKEN.
        let _env = EnvGuard::lock_and_set(&[("OXITERM_APP_TOKEN", Some("test_app_token_123"))]);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = HttpActionHandler::new(format!("http://127.0.0.1:{}/events", port));
        let (reply, _rx) = capture_reply();
        d.dispatch(make_payload("click", 123), reply);

        let (mut stream, request_str) = accept_request(&listener);
        assert!(request_str.contains("Authorization: Bearer test_app_token_123"));
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        stream.write_all(response.as_bytes()).unwrap();
    }

    #[test]
    fn test_response_is_delivered_verbatim_including_control_keys() {
        // The handler no longer interprets the response: `open_url` and `set_app_token` reach the
        // reply untouched; the event loop extracts them (see session.rs tests).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = HttpActionHandler::new(format!("http://127.0.0.1:{}/events", port));
        let (reply, rx) = capture_reply();
        d.dispatch(make_payload("login_action", 1), reply);

        let (stream, _req) = accept_request(&listener);
        let body = r#"{"open_url":"https://accounts.example.com/authorize?state=abc","set_app_token":"tok_12345","normal_key":"val_123"}"#;
        respond_json(stream, body);

        let patch = rx.recv_timeout(std::time::Duration::from_secs(2)).expect("response patch delivered");
        assert_eq!(patch, serde_json::from_str::<serde_json::Value>(body).unwrap());
    }

    #[test]
    fn test_non_200_and_bad_json_deliver_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = HttpActionHandler::new(format!("http://127.0.0.1:{}/events", port));
        let (reply, rx) = capture_reply();
        d.dispatch(make_payload("click", 1), reply);
        let (stream, _req) = accept_request(&listener);
        respond_json(stream, "not json");
        assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_err());
    }

    #[test]
    fn test_fn_action_handler_replies_synchronously() {
        let h = crate::action::FnActionHandler(|p: &DispatchPayload| Some(serde_json::json!({ "seen": p.action.clone() })));
        let (reply, rx) = capture_reply();
        h.dispatch(make_payload("x", 1), reply);
        assert_eq!(rx.try_recv().unwrap(), serde_json::json!({"seen": "x"}));
    }
}
