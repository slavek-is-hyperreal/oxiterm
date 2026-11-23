//! The action seam of the event loop (OxiUI Phase 4, step 3).
//!
//! The loop hands an action (the event-htmx string plus a snapshot of the bound state) to an
//! [`ActionHandler`] and moves on: dispatch is fire-and-forget. The answer comes back through
//! the [`ActionReply`] handle as an `InputEvent::AppResponse` in the loop's own event queue,
//! so no foreign thread ever mutates the session. This module has no HTTP, thread, tokio or
//! metrics dependency: it is the part that moves into `oxiterm-core`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Permanent app token associated with the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_token: Option<String>,
}

/// One-shot handle for delivering the response to a dispatched action.
///
/// The host builds it around "put an event into this session's queue", so an implementation
/// can answer from any thread, synchronously (in-process handler) or later (a browser `fetch`
/// callback). Dropping it without calling [`ActionReply::send`] means "no response".
pub struct ActionReply(Box<dyn FnOnce(serde_json::Value) + Send>);

impl ActionReply {
    /// Wraps the host's delivery function.
    pub fn new(deliver: impl FnOnce(serde_json::Value) + Send + 'static) -> Self {
        Self(Box::new(deliver))
    }

    /// Delivers the response patch (JSON object: state keys plus `open_url`, `set_app_token`,
    /// `navigate`). The patch is applied later, by the loop. Never blocks.
    pub fn send(self, patch: serde_json::Value) {
        (self.0)(patch)
    }
}

/// Receives actions from the event loop. Fire-and-forget: no return value, the response (if
/// any) goes through `reply`.
pub trait ActionHandler: Send + Sync {
    /// Handles one action. Must not block the caller for long: the loop calls this inline.
    fn dispatch(&self, payload: DispatchPayload, reply: ActionReply);
}

/// In-process handler wrapping a closure: the closure maps a payload to an optional response
/// patch, which is delivered immediately (on the calling, i.e. loop, thread). For tests and for
/// hosts that run the application inside the process.
pub struct FnActionHandler<F>(pub F);

impl<F> ActionHandler for FnActionHandler<F>
where
    F: Fn(&DispatchPayload) -> Option<serde_json::Value> + Send + Sync,
{
    fn dispatch(&self, payload: DispatchPayload, reply: ActionReply) {
        if let Some(patch) = (self.0)(&payload) {
            reply.send(patch);
        }
    }
}
