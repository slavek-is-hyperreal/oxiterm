//! Pure session state: everything the event loop reads and writes about one session, with no
//! transport in sight.
//!
//! Phase 4 step 5 splits the old `ClientSession` in two. This module is the pure half and is
//! written to move into `oxiterm-core`: no tokio, prometheus, russh, byte channels, tokens or
//! connection bookkeeping, and no wall-clock reads (time comes from [`Clock`](crate::clock::Clock)).
//! The transport half (`ClientSession` in `session.rs`) owns an `Arc<SessionState>` and the
//! connection plumbing, and reaches the loop only through the narrow traits in `loop_io.rs`.
//!
//! Locking: each field keeps its own lock (as before the split); there is deliberately no outer
//! lock around the whole struct.

use std::time::Duration;
use parking_lot::RwLock;
use tracing::warn;
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::TerminalProfile;

/// Unique identifier type for a client session.
pub type SessionId = usize;

/// Pty dimensions configuration.
#[derive(Debug, Clone, Copy)]
pub struct PtyDimensions {
    /// Number of terminal columns.
    pub cols: u16,
    /// Number of terminal rows.
    pub rows: u16,
}

/// Predictive echo buffer for client side latency hiding.
///
/// Anchored by spec [S5-21].
#[derive(Debug, Default)]
pub struct PredictiveEcho {
    /// Characters typed by the user but not yet confirmed by the server.
    pub buffer: String,
    /// The input node currently targeted by the echo.
    pub active_node: Option<NodeId>,
}

impl PredictiveEcho {
    /// Pushes a new character to the predictive buffer.
    pub fn push(&mut self, ch: char) {
        self.buffer.push(ch);
    }

    /// Confirms a character from the predictive buffer.
    ///
    /// Removes character from the buffer in FIFO order if it matches the confirmed character.
    pub fn confirm(&mut self, ch: char) {
        let mut chars: Vec<char> = self.buffer.chars().collect();
        if let Some(&first) = chars.first() {
            if first == ch {
                chars.remove(0);
                self.buffer = chars.into_iter().collect();
            } else {
                warn!("PredictiveEcho: expected '{}', got '{}'", first, ch);
            }
        }
    }

    /// Drains and returns all pending buffered text.
    pub fn flush_to_server(&mut self) -> String {
        self.buffer.drain(..).collect()
    }
}

/// Debouncer to prevent rapid, redundant terminal layout recomputations on resizing.
#[derive(Debug)]
pub struct ResizeDebouncer {
    pub pending: Option<PtyDimensions>,
    pub pushed_at: crate::clock::MonoTime,
    pub last_update: crate::clock::MonoTime,
}

impl ResizeDebouncer {
    /// How long after the last push the new size is applied.
    pub const WINDOW: Duration = Duration::from_millis(100);

    /// Creates a new debouncer.
    pub fn new() -> Self {
        Self {
            pending: None,
            pushed_at: crate::clock::MonoTime::ZERO,
            last_update: crate::clock::MonoTime::ZERO,
        }
    }

    /// Queues a new dimensions update.
    ///
    /// `now` comes from the session's host clock ([`SessionState::clock`]); `push` is called
    /// by the transports (SSH/WebSocket), which stamp the same clock the loop polls with.
    pub fn push(&mut self, dims: PtyDimensions, now: crate::clock::MonoTime) {
        self.pending = Some(dims);
        self.pushed_at = now;
    }

    /// Polls for a debounced layout change.
    ///
    /// Returns the dimensions if the debouncing window has elapsed since the last push.
    pub fn poll(&mut self, now: crate::clock::MonoTime) -> Option<PtyDimensions> {
        if let Some(dims) = self.pending {
            if now.saturating_duration_since(self.pushed_at) > Self::WINDOW {
                self.pending = None;
                self.last_update = now;
                return Some(dims);
            }
        }
        None
    }

    /// The earliest time at which [`poll`](Self::poll) returns the pending dimensions, or `None`
    /// when nothing is pending. `poll` compares with a strict `>`, so this is the window plus one
    /// nanosecond after the last push: a `poll(due_at())` always succeeds, which keeps a driver
    /// that waits until this time from spinning.
    pub fn due_at(&self) -> Option<crate::clock::MonoTime> {
        self.pending.map(|_| self.pushed_at + Self::WINDOW + Duration::from_nanos(1))
    }
}

/// Info details for active media elements.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaRenderInfo {
    /// File path of the media resource.
    pub path: String,
    /// X coordinate of layout position in cells; negative when scrolled/positioned past the
    /// left edge (sent as i16 in the 0x21 message).
    pub x: i16,
    /// Y coordinate of layout position in cells; negative when scrolled past the top edge.
    pub y: i16,
    /// Width bounding box.
    pub width: u16,
    /// Height bounding box.
    pub height: u16,
}


/// The pure half of a session: dimensions, local DOM state, echo, resize debouncing, mode flags,
/// the current page and the identity, all as plain data.
pub struct SessionState {
    /// Unique identifier (data only; the registry in the server assigns it).
    pub id: SessionId,
    /// Shared terminal grid dimensions.
    pub dims: RwLock<PtyDimensions>,
    /// Predictive input echo engine.
    pub predictive_echo: RwLock<PredictiveEcho>,
    /// Debouncer for terminal window changes.
    pub resize_debouncer: RwLock<ResizeDebouncer>,
    /// Active terminal emulator profile capability features.
    pub terminal_profile: RwLock<TerminalProfile>,
    /// Host clock of this session. The event loop takes its time from it by default and the
    /// transports use it to stamp resize pushes, so both share one origin.
    pub clock: std::sync::Arc<dyn crate::clock::Clock>,
    /// Session-wide local DOM state database.
    pub state: RwLock<crate::state::StateManager>,
    /// Active media placement layout tracking list.
    pub active_media: RwLock<Vec<MediaRenderInfo>>,
    /// Indicates if the session is a mobile web session.
    pub is_mobile: std::sync::atomic::AtomicBool,
    /// Web client negotiated display-list frames (`0x12` caps bit 0): documents are parsed in
    /// `RenderMode::Gui` and frames go out as display lists instead of cell diffs.
    pub display_list_client: std::sync::atomic::AtomicBool,
    /// Indicates if the session is a web (WebSocket) session with DOM media overlay.
    pub is_web_client: std::sync::atomic::AtomicBool,
    /// The current relative page path for WebSocket announce.
    pub current_page: RwLock<Option<String>>,
    /// The application base directory.
    pub app_base_dir: RwLock<Option<std::path::PathBuf>>,
    /// Authenticated user identity for this session.
    /// Set exactly once at session start by identity.rs; never overwritten on reattach.
    pub identity: RwLock<Option<crate::identity::UserIdentity>>,
    /// Reason byte emitted in the next [0xFF, reason] termination frame.
    /// 0 = takeover (hardcoded at takeover site); 1 = idle; 2 = restart/unknown (default).
    /// Written by the event loop's idle path; read by the transport's frame sink on drop.
    pub death_reason: std::sync::atomic::AtomicU8,
}

impl SessionState {
    /// Creates the pure half with default dimensions (80x24) and the given host clock.
    pub fn new(id: SessionId, clock: std::sync::Arc<dyn crate::clock::Clock>) -> Self {
        Self {
            id,
            dims: RwLock::new(PtyDimensions { cols: 80, rows: 24 }),
            predictive_echo: RwLock::new(PredictiveEcho::default()),
            resize_debouncer: RwLock::new(ResizeDebouncer::new()),
            terminal_profile: RwLock::new(TerminalProfile::default()),
            clock,
            state: RwLock::new(crate::state::StateManager::new()),
            active_media: RwLock::new(Vec::new()),
            is_mobile: std::sync::atomic::AtomicBool::new(false),
            display_list_client: std::sync::atomic::AtomicBool::new(false),
            is_web_client: std::sync::atomic::AtomicBool::new(false),
            current_page: RwLock::new(None),
            app_base_dir: RwLock::new(None),
            identity: RwLock::new(None),
            death_reason: std::sync::atomic::AtomicU8::new(2),
        }
    }

    /// Attempts to attach an identity to the session.
    ///
    /// Injects reserved state keys (including `_is_web`) and sets the identity field if identity is None (C2/M2).
    /// Returns true if the identity was successfully attached.
    pub fn attach_identity(&self, id: crate::identity::UserIdentity) -> bool {
        let mut stored = self.identity.write();
        if stored.is_none() {
            let is_web = self.is_web_client.load(std::sync::atomic::Ordering::SeqCst);
            let mut state = self.state.write();
            id.inject_reserved_keys(&mut *state, is_web);
            *stored = Some(id);
            true
        } else {
            false
        }
    }


    /// Accepts only a relative `.thtml` path for a `navigate` patch: no scheme, no absolute
    /// path, no `open:` prefix, bounded length. Containment within the app base dir is still
    /// enforced by the event loop when the page is loaded.
    pub fn sanitize_navigate_target(path: &str) -> Option<String> {
        let ok = !path.is_empty()
            && path.len() <= 512
            && path.ends_with(".thtml")
            && !path.starts_with('/')
            && !path.starts_with('\\')
            && !path.contains("://")
            && !path.starts_with("open:")
            && !path.contains('\0');
        if ok {
            Some(path.to_string())
        } else {
            warn!("app patch: rejected navigate target");
            None
        }
    }

    /// Applies the plain state keys of a patch to the session state.
    ///
    /// The single key-application path for every patch (App Server response, push endpoint):
    /// the event loop has already taken `navigate`, `open_url` and `set_app_token` out. Enforces
    /// limits on key lengths, array items, and payload sizes to mitigate DoS, and refuses
    /// reserved (`_`-prefixed) keys.
    pub fn apply_patch_keys(&self, obj: serde_json::Map<String, serde_json::Value>) {
        {
            // Anchored by spec [SEC-11a]: limit 100 keys per patch
            if obj.len() > 100 {
                warn!("Ignoring state patch with too many keys: {}", obj.len());
                return;
            }
            let mut state = self.state.write();
            for (key, val) in obj {
                // Reserved keys (prefixed `_`) are written only by identity.rs at session start.
                if key.starts_with('_') {
                    warn!("app patch: skipping reserved key '{}'", key);
                    continue;
                }
                // Anchored by spec [SEC-11b]: limit 256 characters per key
                if key.len() > 256 {
                    warn!("Skipping key in state patch: key length exceeds 256 characters");
                    continue;
                }
                let state_val = match val {
                    serde_json::Value::Number(num) => {
                        if let Some(i) = num.as_i64() {
                            crate::state::StateValue::Int(i)
                        } else {
                            let s = num.to_string();
                            if s.len() > 65536 {
                                warn!("Skipping key {}: number string length exceeds 64KiB", key);
                                continue;
                            }
                            crate::state::StateValue::Str(s)
                        }
                    }
                    serde_json::Value::String(s) => {
                        // Anchored by spec [SEC-11c]: limit 64KiB per string
                        if s.len() > 65536 {
                            warn!("Skipping key {}: string length exceeds 64KiB", key);
                            continue;
                        }
                        crate::state::StateValue::Str(s)
                    }
                    serde_json::Value::Bool(b) => crate::state::StateValue::Bool(b),
                    serde_json::Value::Array(arr) => {
                        // Anchored by spec [SEC-11c]: limit 1000 items in list
                        if arr.len() > 1000 {
                            warn!("Skipping key {}: array size exceeds 1000 items", key);
                            continue;
                        }
                        let mut list = Vec::new();
                        let mut string_too_long = false;
                        for v in arr {
                            let s = match v {
                                serde_json::Value::String(st) => st,
                                other => other.to_string(),
                            };
                            if s.len() > 65536 {
                                string_too_long = true;
                                break;
                            }
                            list.push(s);
                        }
                        if string_too_long {
                            warn!("Skipping key {}: list item string length exceeds 64KiB", key);
                            continue;
                        }
                        crate::state::StateValue::List(list)
                    }
                    serde_json::Value::Null => {
                        crate::state::StateValue::Str(String::new())
                    }
                    other => {
                        let s = other.to_string();
                        if s.len() > 65536 {
                            warn!("Skipping key {}: serialized value length exceeds 64KiB", key);
                            continue;
                        }
                        crate::state::StateValue::Str(s)
                    }
                };
                state.set(key, state_val);
            }
        }
    }
}

#[cfg(test)]
mod resize_debouncer_tests {
    use super::*;
    use crate::clock::MonoTime;

    #[test]
    fn due_at_is_the_first_instant_poll_applies() {
        let dims = PtyDimensions { cols: 10, rows: 5 };
        let t = MonoTime::from_millis(500);
        let mut d = ResizeDebouncer::new();
        assert_eq!(d.due_at(), None);
        d.push(dims, t);
        let due = d.due_at().unwrap();
        assert_eq!(due, t + ResizeDebouncer::WINDOW + Duration::from_nanos(1));
        assert!(d.poll(due - Duration::from_nanos(1)).is_none(), "not due one nanosecond earlier");
        assert_eq!(d.due_at(), Some(due), "still pending after an early poll");
        assert_eq!(d.poll(due).map(|x| (x.cols, x.rows)), Some((10, 5)));
        assert_eq!(d.due_at(), None, "drained");
    }
}
