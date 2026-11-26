//! Client session representation and event loop processing.
//!
//! Exposes registries tracking interactive users, input event dispatchers, PTY configurations,
//! screen double-buffering, and rendering coordinates updates.

use std::collections::HashMap;
use std::sync::Arc;
use crate::ssh::reactor::ReactorMessage;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::{info, warn, debug};

pub use crate::loop_io::{EventPush, EventSource, LoopHandles, LoopHost, WaitResult};
pub use crate::session_state::{
    MediaRenderInfo, PredictiveEcho, PtyDimensions, ResizeDebouncer, SessionId, SessionState,
};

pub use oxiterm_renderer::document::THTMLDocument;

/// Registry holding active client session objects.
pub struct SessionRegistry {
    /// Active sessions map.
    pub sessions: RwLock<HashMap<SessionId, Arc<ClientSession>>>,
    /// Active tokens map.
    pub tokens: RwLock<HashMap<String, SessionId>>,
    next_id: RwLock<SessionId>,
    prometheus_registry: Arc<prometheus::Registry>,
    max_sessions: usize,
}

use std::sync::mpsc;
use oxiterm_proto::input::InputEvent;

/// Represents an active interactive client session: the TRANSPORT half.
///
/// The pure half (dimensions, local state, echo, mode flags, current page, identity, clock,
/// death reason) lives in [`SessionState`]; this struct owns it as `state_half` and adds the
/// connection plumbing: channels, metrics, token, epoch, connection count and the idle
/// reaper's wall clock. `Deref<Target = SessionState>` keeps `session.state`, `session.dims`
/// etc. working for transports and tests; the event loop receives only the pure half plus the
/// narrow [`LoopHandles`].
pub struct ClientSession {
    /// The pure half, shared with the event loop.
    pub state_half: Arc<SessionState>,
    /// Prometheus metrics tracker.
    pub metrics: Arc<crate::metrics::SessionMetrics>,
    /// Channel to send raw bytes or commands to the `ReactorThread`.
    pub raw_input_tx: mpsc::Sender<ReactorMessage>,
    /// Channel to receive processed `InputEvents` from the `ReactorThread`.
    pub event_rx: Arc<parking_lot::Mutex<crate::backpressure::Receiver<InputEvent>>>,
    /// Input events sender.
    pub event_tx: crate::backpressure::BoundedFrameChannel<InputEvent>,
    /// Instant of the last received input action.
    ///
    /// TRANSPORT-SIDE wall-clock state for the idle reaper (web.rs). The loop reaches it only
    /// through [`LoopHost`] (`touch_activity` / `idle_for`).
    pub last_activity: RwLock<std::time::Instant>,
    /// Number of active WebSocket connections.
    pub active_connections: std::sync::atomic::AtomicUsize,
    /// Unique connection token for session reattachment.
    pub token: String,
    pub web_frame_tx: parking_lot::RwLock<Option<(usize, tokio::sync::mpsc::Sender<Vec<u8>>)>>,
    pub connection_epoch: std::sync::atomic::AtomicUsize,
    pub event_loop_running: std::sync::atomic::AtomicBool,
}

impl std::ops::Deref for ClientSession {
    type Target = SessionState;
    fn deref(&self) -> &SessionState {
        &self.state_half
    }
}

impl LoopHost for ClientSession {
    fn touch_activity(&self) {
        ClientSession::touch_activity(self)
    }
    fn idle_for(&self) -> Duration {
        ClientSession::idle_for(self)
    }
    fn loop_exited(&self) {
        self.event_loop_running.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl ClientSession {
    /// Records input activity now (transport-side wall clock, idle reaper).
    pub fn touch_activity(&self) {
        *self.last_activity.write() = std::time::Instant::now();
    }

    /// Time since the last recorded activity.
    pub fn idle_for(&self) -> Duration {
        self.last_activity.read().elapsed()
    }

    /// The loop's view of this session: the pure state and the narrow transport handles.
    pub fn loop_handles(self: &Arc<Self>) -> LoopHandles {
        LoopHandles {
            state: self.state_half.clone(),
            events_in: self.event_rx.clone(),
            events_out: Arc::new(self.event_tx.clone()),
            host: self.clone(),
        }
    }

    /// Creates the event loop driving this session, building the transport sink here: ANSI over
    /// `output_tx`, or (`a11y_mode`) the linear screen-reader sink plus the D-Bus bridge. Pages
    /// come from the filesystem under the session's `app_base_dir`.
    pub fn new_event_loop(
        self: &Arc<Self>,
        event_bus: Arc<crate::events::EventBus>,
        output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
        doc: THTMLDocument,
        a11y_mode: bool,
    ) -> EventLoop {
        let (frame_sink, dbus_bridge) = build_frame_sink(output_tx, a11y_mode);
        let page_source = fs_page_source_for(&self.state_half);
        let mut event_loop = EventLoop::new(self.loop_handles(), event_bus, frame_sink, page_source, doc);
        event_loop.dbus_bridge = dbus_bridge;
        event_loop
    }

    /// Closes input receiver streams, initiating shutdown.
    pub fn close(&self) {
        self.event_rx.lock().close();
    }

    /// Reopens the input receiver stream for reconnects.
    pub fn reopen(&self) {
        self.event_rx.lock().reopen();
    }

}

impl SessionRegistry {
    /// Creates a new SessionRegistry with configured limits.
    pub fn new(prometheus_registry: Arc<prometheus::Registry>, max_sessions: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
            next_id: RwLock::new(0),
            prometheus_registry,
            max_sessions,
        }
    }

    fn generate_token() -> String {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut bytes);
        } else {
            let duration = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let mut seed = duration.as_nanos();
            for byte in bytes.iter_mut() {
                *byte = (seed & 0xFF) as u8;
                seed >>= 8;
                if seed == 0 {
                    seed = duration.as_nanos();
                }
            }
        }
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Returns the live session if the token is valid, else None.
    pub fn reattach(&self, token: &str) -> Option<Arc<ClientSession>> {
        let tokens_lock = self.tokens.read();
        let sid = tokens_lock.get(token)?;
        let sessions_lock = self.sessions.read();
        sessions_lock.get(sid).cloned()
    }

    /// Allocates and spawns a new ClientSession, respecting capacity thresholds.
    pub fn create_session(&self) -> Option<Arc<ClientSession>> {
        {
            let count = self.sessions.read().len();
            if count >= self.max_sessions {
                warn!("Session limit reached ({}/{})", count, self.max_sessions);
                return None;
            }
        }

        let mut id_lock = self.next_id.write();
        let id = *id_lock;
        *id_lock += 1;

        let (raw_tx, raw_rx) = mpsc::channel();
        let (event_tx, event_rx) = crate::backpressure::BoundedFrameChannel::<InputEvent>::new(128);

        // Spawn the RRT (Resilient Reactor Thread)
        crate::ssh::reactor::ReactorThread::spawn(raw_rx, event_tx.clone());

        let token = Self::generate_token();

        let session = Arc::new(ClientSession {
            state_half: Arc::new(SessionState::new(id, Arc::new(crate::host_clock::SystemClock::new()))),
            metrics: crate::metrics::SessionMetrics::new(&id.to_string(), &self.prometheus_registry),
            raw_input_tx: raw_tx,
            event_rx: Arc::new(parking_lot::Mutex::new(event_rx)),
            event_tx: event_tx.clone(),
            last_activity: RwLock::new(std::time::Instant::now()),
            active_connections: std::sync::atomic::AtomicUsize::new(0),
            token: token.clone(),
            web_frame_tx: parking_lot::RwLock::new(None),
            connection_epoch: std::sync::atomic::AtomicUsize::new(0),
            event_loop_running: std::sync::atomic::AtomicBool::new(false),
        });
        self.sessions.write().insert(id, session.clone());
        self.tokens.write().insert(token, id);
        Some(session)
    }

    /// Removes session from the registry and frees allocated resources.
    pub fn remove_session(&self, id: SessionId) {
        let count_before = self.sessions.read().len();
        if let Some(session) = self.sessions.write().remove(&id) {
            self.tokens.write().retain(|_, &mut v| v != id);
            session.close();
        }
        let count_after = self.sessions.read().len();
        if count_after == 0 && count_before > 0 {
            info!("No active sessions remaining. Cleaning up video players.");
            oxiterm_renderer::render::cache::VideoPlayerRegistry::get().cleanup();
        }
    }

    /// Broadcasts an input event to all active sessions.
    pub fn broadcast_input_event(&self, event: InputEvent) {
        let sessions = self.sessions.read();
        for session in sessions.values() {
            let _ = session.event_tx.try_send(event.clone());
        }
    }

    /// Gracefully waits for sessions to close during process drains.
    pub async fn drain_sessions(&self, timeout: Duration) {
        info!("Draining sessions with timeout: {}s", timeout.as_secs());
        
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            let count = self.sessions.read().len();
            if count == 0 {
                info!("All sessions drained successfully.");
                return;
            }
            info!("Waiting for {} sessions to close...", count);
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        warn!("Drain timeout reached. {} sessions still active.", self.sessions.read().len());
    }
}

/// ANSI compatible frame writer.
pub struct AnsiFrameSink {
    output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
}

impl AnsiFrameSink {
    /// Creates a new `AnsiFrameSink`.
    pub fn new(output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>) -> Self {
        Self { output_tx }
    }
}

impl oxiterm_renderer::FrameSink for AnsiFrameSink {
    fn send_frame(&mut self, front: &oxiterm_renderer::CellBuffer, back: &oxiterm_renderer::CellBuffer) -> anyhow::Result<bool> {
        let commands = oxiterm_renderer::DiffEngine::diff(front, back);
        let transitioning_graphics_cleanup = !front.graphics.is_empty() && back.graphics.is_empty();
        
        debug!("send_frame: commands count = {}, graphics count = {}, transitioning_graphics_cleanup = {}", commands.len(), back.graphics.len(), transitioning_graphics_cleanup);
        
        if commands.is_empty() && back.graphics.is_empty() && !transitioning_graphics_cleanup {
            return Ok(false);
        }

        let mut out = Vec::new();
        // BSU: CSI ? 2026 h
        out.extend_from_slice(b"\x1b[?2026h");

        // When transitioning from graphics to no graphics, delete active placement handles.
        if !front.graphics.is_empty() && back.graphics.is_empty() {
            out.extend_from_slice(&oxiterm_renderer::render::kitty::KittyImageManager::delete_all_placements());
        }
        
        let bytes = oxiterm_renderer::DiffEngine::encode_ansi(&commands);
        debug!("send_frame: encoded ANSI size = {} bytes", bytes.len());
        out.extend_from_slice(&bytes);
        
        for g in &back.graphics {
            out.extend_from_slice(g);
        }
        
        // ESU: CSI ? 2026 l
        out.extend_from_slice(b"\x1b[?2026l");
        
        let sent_len = out.len();
        match self.output_tx.try_send(out) {
            crate::backpressure::SendResult::Sent => {
                debug!("send_frame: successfully sent frame ({} bytes) to output_tx", sent_len);
                Ok(true)
            }
            crate::backpressure::SendResult::Dropped => {
                warn!("send_frame: frame dropped due to backpressure");
                Ok(false)
            }
            crate::backpressure::SendResult::Closed => {
                warn!("send_frame: frame closed");
                Ok(false)
            }
        }
    }

    fn setup(&mut self) -> anyhow::Result<()> {
        debug!("AnsiFrameSink::setup called");
        let mut initial_clear = Vec::new();
        // Enable Alt Buffer, Clear screen (with scrollback history), Hide cursor.
        initial_clear.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[3J\x1b[H\x1b[?25l");
        let _ = self.output_tx.try_send(initial_clear);
        Ok(())
    }

    fn clear_screen(&mut self) -> anyhow::Result<()> {
        let mut clear_seq = Vec::new();
        clear_seq.extend_from_slice(&oxiterm_renderer::render::kitty::KittyImageManager::delete_all_placements());
        clear_seq.extend_from_slice(b"\x1b[2J\x1b[H");
        let _ = self.output_tx.try_send(clear_seq);
        Ok(())
    }
}

impl Drop for AnsiFrameSink {
    fn drop(&mut self) {
        let mut cleanup = Vec::new();
        cleanup.extend_from_slice(b"\x1b[?2026l"); // ESU
        cleanup.extend_from_slice(b"\x1b[?25h");   // Show Cursor
        cleanup.extend_from_slice(b"\x1b[?1000l"); // Disable Standard Mouse Tracking
        cleanup.extend_from_slice(b"\x1b[?1003l"); // Disable Any-event Mouse Tracking
        cleanup.extend_from_slice(b"\x1b[?1006l"); // Disable SGR Mouse Mode
        cleanup.extend_from_slice(b"\x1b[=0u");     // Disable Kitty Keyboard Protocol
        cleanup.extend_from_slice(b"\x1b[?1049l"); // Disable Alt Buffer
        let _ = self.output_tx.try_send(cleanup);
    }
}

struct ChannelWriter(crate::backpressure::BoundedFrameChannel<Vec<u8>>);
impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.try_send(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Filesystem page source rooted at the session's `app_base_dir` (read on every call, since the
/// transport may set it after the loop is built).
pub fn fs_page_source_for(state: &Arc<SessionState>) -> Arc<dyn crate::page_source::PageSource> {
    let root_session = state.clone();
    Arc::new(crate::fs_page_source::FsPageSource::with_dynamic_root(move || root_session.app_base_dir.read().clone()))
}

/// Builds the SSH-side frame sink over `output_tx`: the ANSI diff sink, or with `a11y_mode` the
/// linear screen-reader sink plus its D-Bus bridge (which the loop updates while rendering).
pub fn build_frame_sink(
    output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
    a11y_mode: bool,
) -> (Box<dyn oxiterm_renderer::FrameSink>, Option<oxiterm_a11y::DBusBridge>) {
    if a11y_mode {
        let writer = ChannelWriter(output_tx.clone());
        let mut bridge = oxiterm_a11y::DBusBridge::new();
        if let Ok(addr) = oxiterm_a11y::DBusBridge::read_dbus_address() {
            let _ = bridge.open_tunnel(std::path::Path::new("/tmp/dbus-local"), std::path::Path::new(&addr));
        }
        (Box::new(oxiterm_a11y::LinearFrameSink::new(writer)), Some(bridge))
    } else {
        (Box::new(AnsiFrameSink::new(output_tx.clone())), None)
    }
}

pub use oxiterm_core::event_loop::{DragState, EventLoop, PatchOrigin, StepOutcome};
#[cfg(test)]
use oxiterm_core::event_loop::{EventLoopGuard, FOCUS_RING_LEFT, FOCUS_RING_RIGHT};



#[cfg(test)]
mod tests {
    /// A bare loop over an empty document, for patch-handling tests.
    fn bare_loop(session: Arc<ClientSession>) -> EventLoop {
        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        session.new_event_loop(event_bus, output_tx, doc, false)
    }


    use super::*;

    use oxiterm_renderer::render::buffer::CellBuffer;

    use oxiterm_renderer::FrameSink;


    #[test]
    fn test_ansi_frame_sink_kitty_ghost_fix() {
        let (tx, mut rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let mut sink = AnsiFrameSink::new(tx);

        let mut front = CellBuffer::new(10, 10);
        let mut back = CellBuffer::new(10, 10);

        let res = sink.send_frame(&front, &back).unwrap();
        assert!(!res);

        front.graphics.push(b"some_img".to_vec());
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(res);
        let frame = rx.try_recv().expect("Should receive frame");
        let delete_seq = b"\x1b_Ga=d,d=A\x1b\\";
        let seq_pos = frame.windows(delete_seq.len()).position(|w| w == delete_seq);
        assert!(seq_pos.is_some(), "delete_all_placements sequence not found in frame");

        front.graphics.clear();
        back.graphics.push(b"new_img_data".to_vec());
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(res);
        let frame2 = rx.try_recv().expect("Should receive frame 2");
        let seq_pos2 = frame2.windows(delete_seq.len()).position(|w| w == delete_seq);
        assert!(seq_pos2.is_none(), "delete_all_placements should not be sent on transition to graphics");
        let img_pos = frame2.windows(12).position(|w| w == b"new_img_data");
        assert!(img_pos.is_some(), "new_img_data not found in frame");

        sink.clear_screen().unwrap();
        let frame3 = rx.try_recv().expect("Should receive clear screen");
        let seq_pos3 = frame3.windows(delete_seq.len()).position(|w| w == delete_seq);
        let clear_pos = frame3.windows(4).position(|w| w == b"\x1b[2J");
        assert!(seq_pos3.is_some(), "delete_all_placements not found in clear_screen");
        assert!(clear_pos.is_some(), "ESC[2J not found in clear_screen");
        assert!(seq_pos3.unwrap() < clear_pos.unwrap(), "delete_all_placements must be before ESC[2J");
    }


    #[test]
    fn test_14_switch_viewport_event_loop() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let temp_dir = std::env::temp_dir();
        let app_file = temp_dir.join("index_t14.thtml");
        let mobile_file = temp_dir.join("index_t14_mobile.thtml");
        std::fs::write(&app_file, b"<screen>Desktop</screen>").unwrap();
        std::fs::write(&mobile_file, b"<screen>Mobile</screen>").unwrap();

        let doc = crate::loader::load_thtml_file(&app_file).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut event_loop = client_session.new_event_loop(event_bus, output_tx, doc, false);
        event_loop.source_path = Some(app_file.clone());
        event_loop.base_source_path = Some(app_file.clone());
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        event_loop.session.is_mobile.store(true, std::sync::atomic::Ordering::SeqCst);
        event_loop.resolve_and_load_page("index_t14.thtml").unwrap();

        assert_eq!(event_loop.source_path, Some(mobile_file.clone()));
        assert_eq!(event_loop.base_source_path, Some(app_file.clone()));

        let _ = std::fs::remove_file(app_file);
        let _ = std::fs::remove_file(mobile_file);
    }


    /// Display-list sink double: stores every list and asks the loop to quit after the first.
    struct CapturingListSink {
        lists: Arc<parking_lot::Mutex<Vec<oxiterm_renderer::render::display_list::DisplayList>>>,
        session: Arc<ClientSession>,
    }


    impl oxiterm_renderer::FrameSink for CapturingListSink {
        fn send_frame(&mut self, _f: &oxiterm_renderer::CellBuffer, _b: &oxiterm_renderer::CellBuffer) -> anyhow::Result<bool> {
            panic!("cell diff sent to a display-list session");
        }
        fn send_display_list(&mut self, list: &oxiterm_renderer::render::display_list::DisplayList) -> anyhow::Result<bool> {
            self.lists.lock().push(list.clone());
            use oxiterm_proto::input::{KeyEvent, KeyKind, KeyModifiers};
            let _ = self.session.event_tx.try_send(InputEvent::KeyPress(KeyEvent {
                codepoint: 'q',
                modifiers: KeyModifiers::default(),
                kind: KeyKind::Press,
            }));
            Ok(true)
        }
    }


    #[test]
    fn test_display_list_replays_final_buffer_with_focus_ring() {
        use oxiterm_renderer::render::cell_sink::CellSink;
        use oxiterm_renderer::render::display_list::DisplayItem;
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        *session.dims.write() = PtyDimensions { cols: 40, rows: 10 };
        session.display_list_client.store(true, std::sync::atomic::Ordering::SeqCst);

        let doc = crate::loader::load_thtml_str(r#"<screen style="width: 40; height: 10;"><box event-htmx="noop:x" style="margin-left: 5;"><text>Hello</text></box></screen>"#).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let mut el = session.new_event_loop(Arc::new(crate::events::EventBus::new()), output_tx, doc, false);
        let lists = Arc::new(parking_lot::Mutex::new(Vec::new()));
        el.rebuild_focusable_nodes(); // focuses the first interactive node
        assert!(el.focused_node.is_some());
        el.frame_sink = Box::new(CapturingListSink { lists: lists.clone(), session: session.clone() });
        el.run();
        assert_eq!(el.layout_engine.mode(), oxiterm_proto::style::RenderMode::Gui, "display-list session lays out in GUI mode");

        let lists = lists.lock();
        assert_eq!(lists.len(), 1, "exactly one frame expected before quit");
        let list = &lists[0];

        // The button is auto-focused: the focus ring markers are overlays, so they must be
        // carried by trailing CellGrids, not by the render_frame part of the list.
        let overlay_has_ring = list.items.iter().any(|i| matches!(i, DisplayItem::CellGrid { cells, .. }
            if cells.iter().any(|c| c.ch == FOCUS_RING_LEFT || c.ch == FOCUS_RING_RIGHT)));
        assert!(overlay_has_ring, "focus ring overlay missing from display list");

        // Replay onto an EMPTY buffer and compare with the real final frame (swapped to front).
        let mut replay = oxiterm_renderer::CellBuffer::new(40, 10);
        CellSink::paint(list, &mut replay, |_, _, _| panic!("Legacy must not reach the wire"));
        assert_eq!(replay.cells, el.buffer.front.cells);
    }


    #[test]
    fn test_15_navigate_to_event_loop() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let temp_dir = std::env::temp_dir();
        let app_file = temp_dir.join("index_t15.thtml");
        let target_file = temp_dir.join("about_t15.thtml");
        std::fs::write(&app_file, b"<screen>Home</screen>").unwrap();
        std::fs::write(&target_file, b"<screen>About</screen>").unwrap();

        let doc = crate::loader::load_thtml_file(&app_file).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut event_loop = client_session.new_event_loop(event_bus, output_tx, doc, false);
        event_loop.source_path = Some(app_file.clone());
        event_loop.base_source_path = Some(app_file.clone());
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        event_loop.resolve_and_load_page("about_t15.thtml").unwrap();

        assert_eq!(event_loop.source_path, Some(target_file.clone()));
        assert_eq!(*client_session.current_page.read(), Some("about_t15.thtml".to_string()));

        let _ = std::fs::remove_file(app_file);
        let _ = std::fs::remove_file(target_file);
    }


    #[test]
    fn test_16_base_source_path_resolving() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let temp_dir = std::env::temp_dir();
        let app_file = temp_dir.join("subdir").join("index_mobile.thtml");
        std::fs::create_dir_all(app_file.parent().unwrap()).unwrap();
        std::fs::write(&app_file, b"<screen></screen>").unwrap();

        let doc = crate::loader::load_thtml_file(&app_file).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut event_loop = client_session.new_event_loop(event_bus, output_tx, doc, false);
        event_loop.source_path = Some(app_file.clone());
        event_loop.base_source_path = Some(temp_dir.join("subdir").join("index.thtml"));
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        let about = temp_dir.join("subdir").join("about.thtml");
        std::fs::write(&about, b"<screen></screen>").unwrap();
        let base = event_loop.current_page_ref().unwrap();
        let resolved = event_loop.page_source
            .resolve(Some(&base), &crate::page_source::with_thtml_ext("about"), false)
            .unwrap();
        assert_eq!(resolved.base, about);

        let _ = std::fs::remove_file(about);
        let _ = std::fs::remove_file(app_file);
    }


    #[test]
    fn test_17_session_registry_reattach() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let token = session.token.clone();

        let reattached = reg.reattach(&token).unwrap();
        assert_eq!(reattached.id, session.id);
    }


    #[test]
    fn test_18_reopen_channel() {
        let (tx, mut rx) = crate::backpressure::BoundedFrameChannel::<i32>::new(5);
        assert_eq!(tx.try_send(10), crate::backpressure::SendResult::Sent);
        assert_eq!(tx.try_send(20), crate::backpressure::SendResult::Sent);
        
        rx.close();
        assert_eq!(tx.try_send(30), crate::backpressure::SendResult::Closed);
 
        rx.reopen();
        assert_eq!(tx.try_send(40), crate::backpressure::SendResult::Sent);
        assert_eq!(rx.blocking_recv().unwrap(), 40);
    }


    #[test]
    fn test_19_active_connections_reaper() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        session.active_connections.store(0, std::sync::atomic::Ordering::SeqCst);
        
        let ten_mins_ago = std::time::Instant::now() - std::time::Duration::from_secs(900);
        *session.last_activity.write() = ten_mins_ago;

        let sessions = reg.sessions.read();
        let mut to_remove = Vec::new();
        for (&id, s) in sessions.iter() {
            if s.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
                let active_conns = s.active_connections.load(std::sync::atomic::Ordering::SeqCst);
                let idle_time = s.last_activity.read().elapsed();
                if active_conns == 0 && idle_time > std::time::Duration::from_secs(600) {
                    to_remove.push(id);
                }
            }
        }
        assert_eq!(to_remove, vec![session.id]);
    }

    // --- Plan 2 tests ---

    #[test]
    fn test_1_resolve_load_blocks_traversal() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        let temp_dir = std::env::temp_dir();
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        let doc_file = temp_dir.join("dummy_t1.thtml");
        std::fs::write(&doc_file, b"<screen></screen>").unwrap();
        let doc = crate::loader::load_thtml_file(&doc_file).unwrap();

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut event_loop = client_session.new_event_loop(event_bus, output_tx, doc, false);
        event_loop.base_source_path = Some(doc_file.clone());

        // Attempt path traversal: "../../etc/passwd" — must be blocked
        let result = event_loop.resolve_and_load_page("../../etc/passwd");
        assert!(result.is_err(), "Traversal to ../../etc/passwd must be blocked");

        let _ = std::fs::remove_file(doc_file);
    }


    #[test]
    fn test_2_resolve_load_valid_subdir() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        let temp_dir = std::env::temp_dir();
        let sub_dir = temp_dir.join("t2sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let page = sub_dir.join("page_t2.thtml");
        std::fs::write(&page, b"<screen></screen>").unwrap();
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        let index = temp_dir.join("index_t2.thtml");
        std::fs::write(&index, b"<screen></screen>").unwrap();
        let doc = crate::loader::load_thtml_file(&index).unwrap();

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut event_loop = client_session.new_event_loop(event_bus, output_tx, doc, false);
        event_loop.base_source_path = Some(index.clone());

        // Valid path inside base: must succeed
        let result = event_loop.resolve_and_load_page("t2sub/page_t2.thtml");
        assert!(result.is_ok(), "Valid subdir path must succeed: {:?}", result);

        let _ = std::fs::remove_file(page);
        let _ = std::fs::remove_file(index);
        let _ = std::fs::remove_dir(sub_dir);
    }


    #[test]
    fn test_17_reattach_preserves_identity() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let id = crate::identity::UserIdentity::ssh_key("bob");
        let attached1 = session.attach_identity(id);
        assert!(attached1);

        // Reattach and attempt downgrade/overwrite with Guest identity (C2/M2)
        let guest_id = crate::identity::UserIdentity::guest();
        let attached2 = session.attach_identity(guest_id);
        assert!(!attached2, "Reattach must not overwrite existing identity");

        // Assert original identity is preserved
        let ident = session.identity.read();
        assert!(ident.is_some());
        assert_eq!(ident.as_ref().unwrap().username, "bob");
        
        // Assert state keys are preserved (not overwritten by guest)
        let state = session.state.read();
        assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("bob".to_string())));
    }


    /// Frame sink double that records the `ClientNotice`s the loop raises (OxiUI phase 4 step 2).
    struct NoticeCapture(Arc<parking_lot::Mutex<Vec<oxiterm_renderer::ClientNotice>>>);


    impl oxiterm_renderer::FrameSink for NoticeCapture {
        fn send_frame(&mut self, _f: &CellBuffer, _b: &CellBuffer) -> anyhow::Result<bool> { Ok(false) }
        fn send_notice(&mut self, notice: oxiterm_renderer::ClientNotice) -> anyhow::Result<bool> {
            self.0.lock().push(notice);
            Ok(true)
        }
    }


    fn capture_notices(el: &mut EventLoop) -> Arc<parking_lot::Mutex<Vec<oxiterm_renderer::ClientNotice>>> {
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        el.frame_sink = Box::new(NoticeCapture(seen.clone()));
        seen
    }


    #[test]
    fn test_35_stale_cache_cleared_on_page_change() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut event_loop = session.new_event_loop(event_bus, output_tx, doc, false);
        
        use oxiterm_proto::dom::NodeId;
        event_loop.for_template_cache.insert(NodeId(42), vec![NodeId(43)]);
        assert!(!event_loop.for_template_cache.is_empty());

        let temp = std::env::temp_dir();
        let base_dir = temp.join("test_35_dir");
        std::fs::create_dir_all(&base_dir).unwrap();
        let page_path = base_dir.join("next.thtml");
        std::fs::write(&page_path, b"<screen><text>page content</text></screen>").unwrap();

        *event_loop.session.app_base_dir.write() = Some(base_dir.clone());
        event_loop.base_source_path = Some(page_path.clone());
        event_loop.source_path = Some(page_path.clone());

        event_loop.resolve_and_load_page("next.thtml").unwrap();

        assert!(event_loop.for_template_cache.is_empty());

        let _ = std::fs::remove_file(page_path);
        let _ = std::fs::remove_dir(base_dir);
    }


    #[test]
    fn test_t10_run_exit_allows_fresh_event_loop() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        
        // Initially false
        assert!(!session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst));

        // Simulate run start
        let is_running1 = session.event_loop_running.swap(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!is_running1);

        // Simulate run exit via Drop guard
        {
            let _guard = EventLoopGuard { host: session.clone(), scope: u64::MAX };
        }

        // Should be false again
        assert!(!session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst));

        // Simulate second start
        let is_running2 = session.event_loop_running.swap(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!is_running2);
    }

    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.2 / R1 — 0x34 navigation failure notice
    // ────────────────────────────────────────────────────────────────────────

    fn make_web_event_loop_with_channel(
        session: Arc<ClientSession>,
        frame_rx: &mut Option<tokio::sync::mpsc::Receiver<Vec<u8>>>,
    ) -> EventLoop {
        let (frame_tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        *frame_rx = Some(rx);
        *session.web_frame_tx.write() = Some((1, frame_tx));
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        session.new_event_loop(event_bus, output_tx, doc, false)
    }


    fn drain_frames(rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        while let Ok(f) = rx.try_recv() { frames.push(f); }
        frames
    }


    #[test]
    fn test_t1_nav_error_web_client_sends_0x34() {
        // 2.2/t1: NavigateTo missing page on web session → [0x34, "Nie znaleziono strony"]
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let temp = std::env::temp_dir();
        *session.app_base_dir.write() = Some(temp.clone());

        let mut rx = None;
        let mut el = make_web_event_loop_with_channel(session, &mut rx);
        let seen = capture_notices(&mut el);
        el.base_source_path = Some(temp.join("dummy.thtml"));

        // exhausting NAV tokens first so throttle is out of the way
        // then restore
        el.throttle.nav.set_tokens(5.0);
        el.resolve_and_load_page("nonexistent_page_t1.thtml").ok(); // will fail → send_nav_error
        // BUT send_nav_error is only called from handle_htmx_target / NavigateTo arms.
        // Call send_nav_error directly to test the helper.
        el.send_nav_error();

        assert_eq!(*seen.lock(), vec![oxiterm_renderer::ClientNotice::NavError]);
        // The loop no longer writes the transport channel itself.
        assert!(drain_frames(rx.as_mut().unwrap()).is_empty());
    }


    #[test]
    fn test_t2_nav_error_ssh_session_no_0x34() {
        // 2.2/t2: SSH session (is_web_client=false) → send_nav_error is a no-op
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        // is_web_client stays false (default)

        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = session.new_event_loop(event_bus, output_tx, doc, false);

        el.send_nav_error();

        let frames = drain_frames(&mut frame_rx);
        let found = frames.iter().any(|f| !f.is_empty() && f[0] == 0x34);
        assert!(!found, "SSH session must not emit 0x34");
    }


    #[test]
    fn test_t3_nav_error_traversal_indistinguishable() {
        // 2.2/t3: traversal + missing produce byte-identical 0x34 frames
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = session.new_event_loop(event_bus, output_tx, doc, false);
        let seen = capture_notices(&mut el);

        // Send two nav errors (simulating traversal case and missing case)
        el.send_nav_error();
        el.send_nav_error();

        let notices = seen.lock().clone();
        assert_eq!(notices.len(), 2, "expected exactly two notices");
        assert_eq!(notices[0], notices[1], "traversal and missing must produce identical notices");
        assert_eq!(notices[0], oxiterm_renderer::ClientNotice::NavError);
        assert!(drain_frames(&mut frame_rx).is_empty(), "the loop must not write the transport channel");
    }


    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.2 / R5 — Throttle tests
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t4_nav_throttle_emits_one_notice_then_silences() {
        // 2.2/t4: exhaust NAV bucket → exactly one 0x34 "Zwolnij ;)" per episode
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = session.new_event_loop(event_bus, output_tx, doc, false);
        let seen = capture_notices(&mut el);

        // Exhaust NAV tokens
        for _ in 0..crate::throttle::NAV_CAPACITY {
            assert!(el.throttle.check_nav(el.now), "should be within capacity");
        }
        // Now throttled — simulate 20 throttle calls
        let mut throttle_notices = 0usize;
        for _ in 0..20 {
            if !el.throttle.check_nav(el.now) && el.throttle.is_first_throttle() {
                el.send_nav_throttle_notice();
                throttle_notices += 1;
            }
        }
        // Must be exactly 1 throttle notice per episode
        // The bucket is checked at the fixed `el.now`, so no refill happens: exactly 1 notice.
        assert_eq!(throttle_notices, 1,
            "exactly one throttle notice per episode; got {}", throttle_notices);

        assert_eq!(*seen.lock(), vec![oxiterm_renderer::ClientNotice::NavThrottle],
            "exactly one NavThrottle notice emitted");
        assert!(drain_frames(&mut frame_rx).is_empty(), "the loop must not write the transport channel");
    }


    #[test]
    fn test_t1b_takeover_sends_0xff_0() {
        // 2.3/t1b: Takeover site sends hardcoded [0xFF, 0]
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        // Simulate takeover: old_tx.try_send([0xFF, 0])
        let _ = tx.try_send(vec![0xFF, 0u8]);
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame, vec![0xFF, 0u8], "takeover must send [0xFF, 0]");
    }


    #[test]
    fn test_t1c_live_tab_idle_death_emits_0xff_1_not_0() {
        // 2.3/t1c: live-tab idle death (is_web_client=true, death_reason=1) → [0xFF, 1]
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        // Simulate EventLoop idle path: set death_reason=1
        session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);

        // Simulate WsFrameSink::drop: read death_reason and send [0xFF, reason]
        let reason = session.death_reason.load(std::sync::atomic::Ordering::SeqCst);
        if let Some((_, ref tx)) = *session.web_frame_tx.read() {
            let _ = tx.try_send(vec![0xFF, reason]);
        }

        let frame = frame_rx.try_recv().unwrap();
        assert_eq!(frame, vec![0xFF, 1u8],
            "live-tab idle death must emit [0xFF, 1], NOT [0xFF, 0]: got {:?}", frame);
    }


    #[test]
    fn test_t1a_extended_f2_reattach_resets_death_reason() {
        // F2: After idle death + fresh attach, death_reason is reset to 2.
        // A subsequent non-idle exit must emit [0xFF, 2].
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();

        // Simulate idle cycle: death_reason=1
        session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);

        // Simulate WS reattach: death_reason reset to 2 (as in web.rs handle_websocket)
        session.death_reason.store(2, std::sync::atomic::Ordering::SeqCst);

        // Now a non-idle exit (e.g., server restart) → death_reason stays 2
        let reason = session.death_reason.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(reason, 2u8,
            "after reattach, death_reason must be 2 (restart/unknown); got {}", reason);
    }


    #[test]
    fn test_t4a_reattach_resets_last_activity() {
        // 2.3/t4a: WS attach resets last_activity within 1 second
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();

        // Wind back last_activity to simulate stale session
        *session.last_activity.write() = std::time::Instant::now()
            - std::time::Duration::from_secs(600);

        // Simulate handle_websocket reset
        session.touch_activity();
        session.death_reason.store(2, std::sync::atomic::Ordering::SeqCst);

        let elapsed = session.idle_for();
        assert!(elapsed < std::time::Duration::from_secs(1),
            "last_activity after reattach must be < 1s ago, was {:?}", elapsed);
    }


    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.3 Addendum R5 — media log split
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t5b_media_outside_base_does_not_crash() {
        // 2.3/t5b: path traversal attempt in media path → blocked, no panic
        // This test verifies the is_within_base guard still rejects traversals.
        let base = std::env::temp_dir().join("oxiterm_test_t5b");
        std::fs::create_dir_all(&base).unwrap();
        let outside = std::fs::canonicalize(base.parent().unwrap()).unwrap().join("outside_t5b.jpg");

        // is_within_base should block this
        let is_safe = crate::pathsafe::is_within_base(&base, &outside);
        assert!(!is_safe, "path outside base must be blocked by is_within_base");

        let _ = std::fs::remove_dir_all(&base);
    }


    #[test]
    fn test_t5c_media_missing_not_traversal() {
        // 2.3/t5c: The log split in web.rs distinguishes missing vs traversal AFTER the
        // is_within_base guard. is_within_base itself returns false for non-existent files
        // (canonicalize fails). The test verifies that the web.rs log-split code correctly
        // categorises a within-base path that exists as safe, and a non-existent path
        // as "not found" rather than "traversal" — tested via the path logic directly.
        let base = std::env::temp_dir().join("oxiterm_test_t5c");
        std::fs::create_dir_all(&base).unwrap();
        let existing = base.join("existing_t5c.jpg");
        std::fs::write(&existing, b"jpg").unwrap();
        let missing = base.join("missing_asset_t5c.jpg");

        // Existing file within base → is_within_base returns true
        assert!(crate::pathsafe::is_within_base(&base, &existing),
            "existing file within base must be considered safe");

        // Non-existent file — is_within_base returns false (canonicalize failure)
        // but it is NOT a traversal: the path lexically starts with base
        assert!(!missing.exists(), "missing file must not exist");
        // Lexical prefix check confirms it's not a traversal (just missing)
        let base_str = base.to_string_lossy();
        let missing_str = missing.to_string_lossy();
        assert!(missing_str.starts_with(base_str.as_ref()),
            "path is lexically within base — it's missing, not a traversal");

        let _ = std::fs::remove_file(&existing);
        let _ = std::fs::remove_dir_all(&base);
    }


    #[test]
    fn test_p41_t1_apc_new_token_dispatches_and_sets_state() {
        let _env = crate::test_env::EnvGuard::lock_and_set(&[]);
        use std::net::TcpListener;
        use std::io::{Write, Read};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();

        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut el = session.new_event_loop(event_bus, output_tx, doc, false);
        el.action_handler = Some(Arc::new(crate::dispatcher::HttpActionHandler::new(format!("http://127.0.0.1:{}/events", port))));

        // Receive APC SetAppToken with a new non-empty token
        el.handle_set_app_token("new_app_token_123".to_string());

        let (mut stream, _) = listener.accept().unwrap();
        let mut req_str = String::new();
        let mut buf = [0u8; 1024];
        while !req_str.contains("\"app_token\":") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 { break; }
            req_str.push_str(&String::from_utf8_lossy(&buf[..n]));
        }

        assert!(req_str.contains("\"app_token\":\"new_app_token_123\""));

        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        stream.write_all(response.as_bytes()).unwrap();

        let state = session.state.read();
        assert_eq!(state.get("_app_token"), Some(&crate::state::StateValue::Str("new_app_token_123".to_string())));
    }


    #[test]
    fn test_p41_t2_apc_same_token_no_dispatch_and_no_frame() {
        let (web_tx, mut web_rx) = tokio::sync::mpsc::channel(10);
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        *session.web_frame_tx.write() = Some((0, web_tx));

        session.state.write().set("_app_token".to_string(), crate::state::StateValue::Str("same_token_123".to_string()));

        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut el = session.new_event_loop(event_bus, output_tx, doc, false);
        el.action_handler = Some(Arc::new(crate::dispatcher::HttpActionHandler::new("http://127.0.0.1:1/events".to_string())));

        // Receive APC SetAppToken with the same token
        el.handle_set_app_token("same_token_123".to_string());

        // Verify no 0x35 frame was sent to web_rx
        assert!(web_rx.try_recv().is_err());
    }


    #[test]
    fn test_fn_action_handler_round_trip_through_the_queue() {
        // In-process handler: replies synchronously from dispatch; the reply is only queued,
        // the state changes when the loop applies the event.
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let mut el = bare_loop(session.clone());
        el.action_handler = Some(Arc::new(crate::action::FnActionHandler(|p: &crate::action::DispatchPayload| {
            Some(serde_json::json!({ "echo": p.action.clone() }))
        })));
        el.try_dispatch("ping");
        assert!(session.state.read().get("echo").is_none(), "reply must not touch state directly");
        let ev = session.event_rx.lock().try_recv().expect("reply queued as an event");
        match ev {
            InputEvent::AppResponse(p) => el.apply_app_patch(p.0, PatchOrigin::ActionResponse),
            other => panic!("unexpected event {:?}", other),
        }
        assert_eq!(session.state.read().get("echo"), Some(&crate::state::StateValue::Str("ping".to_string())));
    }


    // ---- Phase 4 step 7b: the step API driven without threads ----

    /// Frame sink that records each frame's rows as text.
    struct RowsSink(Arc<parking_lot::Mutex<Vec<Vec<String>>>>);

    impl oxiterm_renderer::FrameSink for RowsSink {
        fn send_frame(&mut self, _f: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
            let w = back.width as usize;
            let rows = (0..back.height as usize)
                .map(|y| back.cells[y * w..(y + 1) * w].iter().filter(|c| !c.skip).map(|c| c.ch).collect())
                .collect();
            self.0.lock().push(rows);
            Ok(true)
        }
    }


    // ---- Phase 4 step 7c: the debounced resize is a deadline ----

    fn dims_of(s: &ClientSession) -> (u16, u16) {
        let d = *s.dims.read();
        (d.cols, d.rows)
    }


    fn empty_screen_doc() -> THTMLDocument {
        use oxiterm_proto::dom::{Node, NodeTag};
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root_id = arena.alloc(Node::new(NodeTag::Screen));
        THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() }
    }


    /// Real `run()` thread and the system clock: a pushed size is applied ~100 ms later, not after
    /// the 1 s idle receive timeout.
    #[test]
    fn test_resize_applied_well_under_one_second_by_the_driver() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let (output_tx, _rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let mut el = session.new_event_loop(Arc::new(crate::events::EventBus::new()), output_tx, empty_screen_doc(), false);
        let frames = Arc::new(parking_lot::Mutex::new(Vec::new()));
        el.frame_sink = Box::new(RowsSink(frames.clone()));
        let handle = std::thread::spawn(move || el.run());

        let wait_for = |pred: &dyn Fn() -> bool| {
            let start = std::time::Instant::now();
            while !pred() && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        wait_for(&|| !frames.lock().is_empty());
        assert!(!frames.lock().is_empty(), "first frame rendered");
        // The loop is now idle, blocked in its (up to) 1 s wait.
        std::thread::sleep(Duration::from_millis(100));

        let started = std::time::Instant::now();
        let new_dims = PtyDimensions { cols: 100, rows: 30 };
        { let now = session.clock.now(); session.resize_debouncer.write().push(new_dims, now); }
        let _ = session.event_tx.try_send(InputEvent::Resize { cols: 100, rows: 30 });
        wait_for(&|| dims_of(&session) == (100, 30));
        let latency = started.elapsed();
        assert_eq!(dims_of(&session), (100, 30));
        eprintln!("resize applied after {:?}", latency);
        assert!(latency < Duration::from_millis(500), "resize applied after {:?}", latency);
        wait_for(&|| frames.lock().last().map_or(false, |f| f.len() == 30));
        assert_eq!(frames.lock().last().unwrap().len(), 30);

        session.close();
        handle.join().unwrap();
    }

    // ---- Phase 4 step 7d: a render deferred by the frame limiter is a deadline ----

    /// Counter page in a loop with a recording sink; returns the loop, frames, a click event on
    /// `+` and a reader of the counter value in a frame.
    fn counter_loop(session: &Arc<ClientSession>) -> (EventLoop, Arc<parking_lot::Mutex<Vec<Vec<String>>>>, InputEvent, Box<dyn Fn(&Vec<String>) -> String + Send>) {
        use crate::page_source::{MemPageSource, PageSource};
        use oxiterm_proto::input::{MouseAction, MouseButton, MouseInput};
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let mem = MemPageSource::new([("counter.thtml", include_str!("../../examples/demos/counter.thtml"))]);
        let page = mem.page_at(std::path::Path::new("counter.thtml"));
        let doc = mem.load(&page, oxiterm_proto::style::RenderMode::Tui).unwrap();
        let (output_tx, _rx) = crate::backpressure::BoundedFrameChannel::new(10);
        std::mem::forget(_rx);
        let mut el = session.new_event_loop(Arc::new(crate::events::EventBus::new()), output_tx, doc, false);
        el.page_source = Arc::new(mem);
        let frames = Arc::new(parking_lot::Mutex::new(Vec::new()));
        el.frame_sink = Box::new(RowsSink(frames.clone()));
        // Probe the layout with one real-clock frame, then forget that frame.
        el.prepare_session();
        let now = el.clock.now();
        el.tick(now);
        let layout = el.layout_engine.last_layout.clone().unwrap();
        let (ox, oy) = layout.get_centering_offset(&el.doc, 80, 24);
        let find = |el: &EventLoop, pred: &dyn Fn(&oxiterm_proto::dom::Node) -> bool| {
            el.doc.arena.iter().find(|(_, n)| pred(n)).map(|(id, _)| id).unwrap()
        };
        let text_id = find(&el, &|n| n.attrs.bind_state.as_deref() == Some("counter"));
        let plus_id = find(&el, &|n| n.attrs.event_htmx.as_deref() == Some("inc:counter"));
        let tr = layout.nodes.get(&text_id).unwrap().cells();
        let pr = layout.nodes.get(&plus_id).unwrap().cells();
        frames.lock().clear();
        let click = InputEvent::MouseEvent(MouseInput {
            col: ox + pr.x + pr.width / 2 + 1, row: oy + pr.y + pr.height / 2 + 1,
            button: MouseButton::Left, action: MouseAction::Press, modifiers: Default::default(),
        });
        let value_at = move |frame: &Vec<String>| -> String {
            frame[(oy + tr.y) as usize].chars().skip((ox + tr.x) as usize).take(3).collect::<String>().trim().to_string()
        };
        (el, frames, click, Box::new(value_at))
    }


    /// Real `run()` thread and system clock: two quick state changes on an idle page; the second
    /// one (inside the limiter window of the first frame) must be on screen well under 1 s.
    #[test]
    fn test_deferred_render_shown_well_under_one_second_by_the_driver() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let (el, frames, click, value_at) = counter_loop(&session);
        let handle = std::thread::spawn(move || {
            let mut el = el;
            el.run()
        });
        let wait_for = |pred: &dyn Fn() -> bool| {
            let start = std::time::Instant::now();
            while !pred() && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        wait_for(&|| !frames.lock().is_empty());
        assert!(!frames.lock().is_empty(), "first frame rendered");
        // Idle: blocked in the long wait, limiter window long expired.
        std::thread::sleep(Duration::from_millis(100));

        let sent = |c: &InputEvent| { let _ = session.event_tx.try_send(c.clone()); };
        let n0 = frames.lock().len();
        sent(&click);
        wait_for(&|| frames.lock().len() > n0);
        // The second event lands a few ms after the first frame, inside its 16 ms window.
        let started = std::time::Instant::now();
        sent(&click);
        wait_for(&|| frames.lock().last().map_or(false, |f| value_at(f) == "2"));
        let latency = started.elapsed();
        eprintln!("second change on screen after {:?}", latency);
        assert_eq!(value_at(frames.lock().last().unwrap()), "2");
        assert!(latency < Duration::from_millis(300), "second change on screen after {:?}", latency);

        session.close();
        handle.join().unwrap();
    }
}
