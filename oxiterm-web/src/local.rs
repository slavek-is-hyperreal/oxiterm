//! WASM local mode (OxiUI Stage 7.1): `oxiterm-core` runs inside the browser, no WebSocket.
//!
//! [`LocalSession`] is the pure half (no `web-sys`, testable natively, see
//! `tests/local_vs_thin.rs`): one `EventLoop` driven through the step API with a host-set clock,
//! a `MemPageSource` filled from the server's page index, and a [`FrameSink`] that keeps the
//! `DisplayList` it is given instead of encoding it. It accepts the web client's own binary
//! input messages (`0x01`/`0x02`/`0x03`/`0x10`/`0x11`/`0x31`, decoded by
//! `oxiterm_proto::input::wire`), so the page's input code is shared with the thin client, and it
//! produces the same side-channel messages the WebSocket server sends (`0x30` page announce,
//! `0x33`/`0x34`/`0x35` notices, `0x37` ARIA mirror, `0x21` media coordinates) so the page's
//! message handler is shared too. Display lists skip the wire entirely.
//!
//! App actions: built-in state actions run in the core. The core also hands every action to its
//! `ActionHandler`; here that handler only parks the payload and the core's own [`ActionReply`]
//! (the loop is never blocked). The page's JS POSTs each parked payload to the same-origin
//! proxy (`POST /oxiui/local/events`, which holds the App Server token) and later calls
//! [`LocalSession::action_reply`] (the answer becomes an `AppResponse` event, exactly as on the
//! server) or [`LocalSession::action_failed`] (no answer, as when the App Server is down).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use oxiterm_core::action::{ActionHandler, ActionReply, DispatchPayload};
use oxiterm_core::clock::{FakeClock, MonoTime};
use oxiterm_core::event_loop::{EventLoop, StepOutcome};
use oxiterm_core::loop_io::{EventPush, EventSource, LoopHandles, LoopHost, MemoryQueue};
use oxiterm_core::page_source::{MemPageSource, PageSource};
use oxiterm_core::session_state::{MediaRenderInfo, PtyDimensions, SessionState};
use oxiterm_proto::input::{wire, InputEvent};
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::render::display_list::DisplayList;
use oxiterm_renderer::{CellBuffer, ClientNotice, FrameSink};
use parking_lot::Mutex;

/// Side-channel tags, identical to the WebSocket protocol.
pub const TAG_MEDIA_COORDS: u8 = 0x21;
pub const TAG_PAGE: u8 = 0x30;
pub const TAG_OPEN_URL: u8 = 0x33;
pub const TAG_ERROR_CHIP: u8 = 0x34;
pub const TAG_APP_TOKEN: u8 = 0x35;
pub const TAG_A11Y: u8 = 0x37;
/// Local-mode only: `[0xE0, hash:u32le, rel_path]` asks the page to fetch a media asset.
pub const TAG_ASSET_REQUEST: u8 = 0xE0;

/// Parked, unanswered action replies are bounded: an App Server that never answers cannot grow
/// the table without limit (the oldest reply is dropped = "no response").
const MAX_PENDING_REPLIES: usize = 64;

/// What the page needs to start a local session.
#[derive(Debug, Clone)]
pub struct LocalConfig {
    /// First page (relative to the app base), e.g. `index.thtml`.
    pub start_page: String,
    pub cols: u16,
    pub rows: u16,
    pub mobile: bool,
    /// Install the action handler (an App Server is configured behind the proxy).
    pub actions: bool,
    /// The app token kept in the browser's `localStorage` (empty = none).
    pub app_token: String,
}

/// One action parked for the page to forward.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingAction {
    pub id: u32,
    /// The JSON body of the `DispatchPayload`.
    pub body: String,
}

#[derive(Default)]
struct Parked {
    next_id: u32,
    queue: Vec<PendingAction>,
    replies: Vec<(u32, ActionReply)>,
}

/// The core's `ActionHandler` in the browser: parks payload and reply, never blocks.
struct ParkingHandler(Arc<Mutex<Parked>>);

impl ActionHandler for ParkingHandler {
    fn dispatch(&self, payload: DispatchPayload, reply: ActionReply) {
        let Ok(body) = serde_json::to_string(&payload) else { return };
        let mut p = self.0.lock();
        p.next_id = p.next_id.wrapping_add(1);
        let id = p.next_id;
        p.queue.push(PendingAction { id, body });
        p.replies.push((id, reply));
        if p.replies.len() > MAX_PENDING_REPLIES {
            p.replies.remove(0);
        }
    }
}

/// What the sink collected since the host last looked.
#[derive(Default)]
struct Out {
    frames: Vec<DisplayList>,
    a11y: Vec<Vec<u8>>,
    notices: Vec<ClientNotice>,
    /// The last list the sink accepted (change detection, repaint).
    last_list: Option<DisplayList>,
    last_a11y: Option<Vec<u8>>,
    doc_changed: bool,
}

/// Frame sink of the local mode. Return values mirror the server's `WsFrameSink`: `Ok(true)`
/// when a frame was delivered, `Ok(false)` for an unchanged one (the loop then does not swap
/// buffers or record the frame, exactly as for a thin client).
struct LocalSink(Arc<Mutex<Out>>);

impl FrameSink for LocalSink {
    fn send_frame(&mut self, _front: &CellBuffer, _back: &CellBuffer) -> Result<bool> {
        // Local mode is always a display-list client.
        Ok(false)
    }

    fn send_display_list(&mut self, list: &DisplayList) -> Result<bool> {
        let mut o = self.0.lock();
        if o.last_list.as_ref() == Some(list) {
            return Ok(false);
        }
        o.last_list = Some(list.clone());
        o.frames.push(list.clone());
        Ok(true)
    }

    fn send_a11y(&mut self, json: &[u8]) -> Result<bool> {
        let mut o = self.0.lock();
        if o.last_a11y.as_deref() == Some(json) {
            return Ok(false);
        }
        o.last_a11y = Some(json.to_vec());
        o.a11y.push(json.to_vec());
        Ok(true)
    }

    fn send_notice(&mut self, notice: ClientNotice) -> Result<bool> {
        self.0.lock().notices.push(notice);
        Ok(true)
    }

    fn update_document(&mut self, _doc: &oxiterm_renderer::document::THTMLDocument) -> Result<()> {
        self.0.lock().doc_changed = true;
        Ok(())
    }
}

struct Host;
impl LoopHost for Host {
    fn touch_activity(&self) {}
    fn idle_for(&self) -> Duration {
        Duration::ZERO
    }
    fn loop_exited(&self) {}
}

/// The same notice texts the WebSocket server sends.
pub fn encode_notice(notice: &ClientNotice) -> Vec<u8> {
    let (tag, text): (u8, &str) = match notice {
        ClientNotice::NavError => (TAG_ERROR_CHIP, "Nie znaleziono strony"),
        ClientNotice::NavThrottle => (TAG_ERROR_CHIP, "Zwolnij ;)"),
        ClientNotice::OpenUrl(url) => (TAG_OPEN_URL, url),
        ClientNotice::AppToken(token) => (TAG_APP_TOKEN, token),
    };
    let mut frame = Vec::with_capacity(1 + text.len());
    frame.push(tag);
    frame.extend_from_slice(text.as_bytes());
    frame
}

/// Same hash the server uses to key media (`web.rs::hash_path`).
pub fn hash_path(path: &str) -> u32 {
    let mut hash: u32 = 5381;
    for c in path.chars() {
        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
    }
    hash
}

/// Lexical `dir(page)/src` with `.`/`..` collapsed; `None` when it would leave the app base or
/// is absolute (the same containment the page source applies to navigation).
pub fn media_rel_path(current_page: &str, src: &str) -> Option<String> {
    if src.starts_with('/') || src.starts_with('\\') || src.contains('\0') || src.contains("://") {
        return None;
    }
    // A file name is required: `""`, `.`, `..` and `dir/` name directories.
    if matches!(src.rsplit('/').next(), None | Some("") | Some(".") | Some("..")) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    let dir = current_page.rsplit_once('/').map_or("", |(d, _)| d);
    for seg in dir.split('/').chain(src.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// One in-browser session. All times are host milliseconds (`performance.now()`).
pub struct LocalSession {
    el: EventLoop,
    session: Arc<SessionState>,
    queue: MemoryQueue,
    clock: FakeClock,
    out: Arc<Mutex<Out>>,
    parked: Arc<Mutex<Parked>>,
    messages: Vec<Vec<u8>>,
    sent_page: Option<String>,
    sent_media: Vec<MediaRenderInfo>,
    requested_assets: HashSet<String>,
}

fn ms_to_time(ms: f64) -> MonoTime {
    MonoTime::from_duration(Duration::from_nanos(if ms.is_finite() && ms > 0.0 { (ms * 1e6).round() as u64 } else { 0 }))
}

impl LocalSession {
    /// Builds the session: `pages` is the server's page index (`path -> THTML text`).
    pub fn new(cfg: &LocalConfig, pages: Vec<(String, String)>, now_ms: f64) -> Result<Self, String> {
        Self::new_with_dirs(cfg, pages, Vec::new(), now_ms)
    }

    /// Like [`LocalSession::new`], with the server's list of existing directories (so `..`
    /// resolves exactly as the server's filesystem page source does).
    pub fn new_with_dirs(cfg: &LocalConfig, pages: Vec<(String, String)>, dirs: Vec<String>, now_ms: f64) -> Result<Self, String> {
        let clock = FakeClock::new();
        clock.set(ms_to_time(now_ms));
        let session = Arc::new(SessionState::new(0, Arc::new(clock.clone())));
        let on = std::sync::atomic::Ordering::SeqCst;
        // Same flags a `?gui=1` WebSocket session ends up with (0x12 caps 0x03).
        *session.dims.write() = PtyDimensions::clamped(cfg.cols, cfg.rows);
        session.display_list_client.store(true, on);
        session.display_list_v3.store(true, on);
        session.is_web_client.store(true, on);
        session.is_mobile.store(cfg.mobile, on);
        session.attach_identity(oxiterm_core::identity::UserIdentity::guest());

        let source = Arc::new(MemPageSource::with_dirs(pages, dirs));
        let page = source.resolve(None, &cfg.start_page, cfg.mobile).map_err(|_| "start page unavailable".to_string())?;
        let doc = source.load(&page, RenderMode::Gui).map_err(|e| e.to_string())?;

        let queue = MemoryQueue::new(256);
        let handles = LoopHandles {
            state: session.clone(),
            events_in: Arc::new(Mutex::new(queue.clone())),
            events_out: Arc::new(queue.clone()),
            host: Arc::new(Host),
        };
        let out = Arc::new(Mutex::new(Out::default()));
        let parked = Arc::new(Mutex::new(Parked::default()));
        let mut el = EventLoop::new(
            handles,
            Arc::new(oxiterm_core::events::EventBus::new()),
            Box::new(LocalSink(out.clone())),
            source,
            doc,
        );
        el.clock = Arc::new(clock.clone());
        el.source_path = Some(page.actual.clone());
        el.base_source_path = Some(page.base.clone());
        if cfg.actions {
            el.action_handler = Some(Arc::new(ParkingHandler(parked.clone())));
        }
        el.prepare_session();
        let s = Self {
            el,
            session,
            queue,
            clock,
            out,
            parked,
            messages: Vec::new(),
            sent_page: None,
            sent_media: Vec::new(),
            requested_assets: HashSet::new(),
        };
        if !cfg.app_token.is_empty() {
            s.queue.push(InputEvent::SetAppToken(cfg.app_token.clone()));
        }
        Ok(s)
    }

    /// Feeds one input message of the web protocol. Unknown or malformed messages are dropped.
    /// Does not render: call [`pump`](Self::pump) afterwards.
    pub fn input(&mut self, bytes: &[u8], now_ms: f64) {
        let now = ms_to_time(now_ms);
        self.clock.set(now);
        let Some(&tag) = bytes.first() else { return };
        let event = match tag {
            0x01 => wire::decode_key(bytes).map(InputEvent::KeyPress),
            0x02 => wire::decode_mouse(bytes).map(InputEvent::MouseEvent),
            // A local session is always a display-list client: fractional pointer.
            0x03 => wire::decode_pointer(bytes).map(InputEvent::PointerEvent),
            0x10 => wire::decode_resize(bytes).map(|(cols, rows)| {
                self.session.resize_debouncer.write().push(PtyDimensions { cols, rows }, now);
                InputEvent::Resize { cols, rows }
            }),
            0x11 => wire::decode_viewport(bytes).map(InputEvent::SwitchViewport),
            0x31 => wire::decode_navigate(bytes).map(InputEvent::NavigateTo),
            _ => None,
        };
        if let Some(e) = event {
            if self.el.handle_event(e, now) == StepOutcome::Quit {
                // Keyboard quit is an SSH concept; a browser page just keeps running.
            }
        }
    }

    /// Drains the reply queue into the loop, ticks (renders when due) and collects the
    /// side-channel messages. Call after input and whenever the deadline passes.
    pub fn pump(&mut self, now_ms: f64) {
        let now = ms_to_time(now_ms);
        self.clock.set(now);
        let mut q = self.queue.clone();
        while let Some(e) = q.try_recv() {
            self.el.handle_event(e, now);
        }
        self.el.tick(now);
        self.collect();
    }

    /// Milliseconds until the next `pump` is due (0 = now).
    pub fn next_delay_ms(&self, now_ms: f64) -> f64 {
        let now = ms_to_time(now_ms);
        match self.el.next_deadline(now) {
            Some(d) => (d - now).as_secs_f64() * 1000.0,
            None => 1000.0,
        }
    }

    fn collect(&mut self) {
        // Notices (error chip, open url, app token).
        let notices = std::mem::take(&mut self.out.lock().notices);
        for n in &notices {
            self.messages.push(encode_notice(n));
        }
        // Page announce, like the server's `sync_preamble`.
        let page = self.session.current_page.read().clone();
        if page != self.sent_page {
            if let Some(p) = &page {
                let mut m = vec![TAG_PAGE];
                m.extend_from_slice(p.as_bytes());
                self.messages.push(m);
            }
            self.sent_page = page;
        }
        // Media: coordinates (and asset requests the page must fetch first).
        let media = self.session.active_media.read().clone();
        let doc_changed = std::mem::take(&mut self.out.lock().doc_changed);
        if media != self.sent_media || (doc_changed && !media.is_empty()) {
            self.emit_media(&media);
            self.sent_media = media;
        }
        // ARIA mirror.
        let a11y = std::mem::take(&mut self.out.lock().a11y);
        for json in a11y {
            let mut m = Vec::with_capacity(json.len() + 1);
            m.push(TAG_A11Y);
            m.extend_from_slice(&json);
            self.messages.push(m);
        }
    }

    fn emit_media(&mut self, media: &[MediaRenderInfo]) {
        self.messages.push(vec![TAG_MEDIA_COORDS]);
        let page = self.session.current_page.read().clone().unwrap_or_default();
        for m in media {
            let Some(rel) = media_rel_path(&page, &m.path) else { continue };
            let h = hash_path(&m.path);
            if self.requested_assets.insert(rel.clone()) {
                let mut req = vec![TAG_ASSET_REQUEST];
                req.extend_from_slice(&h.to_le_bytes());
                req.extend_from_slice(rel.as_bytes());
                self.messages.push(req);
            }
            let mut msg = vec![TAG_MEDIA_COORDS];
            msg.extend_from_slice(&h.to_le_bytes());
            msg.extend_from_slice(&m.x.to_le_bytes());
            msg.extend_from_slice(&m.y.to_le_bytes());
            msg.extend_from_slice(&m.width.to_le_bytes());
            msg.extend_from_slice(&m.height.to_le_bytes());
            self.messages.push(msg);
        }
    }

    /// Re-emits the media coordinates (after the page cached a requested asset).
    pub fn media_refresh(&mut self) {
        let media = self.sent_media.clone();
        self.emit_media(&media);
    }

    /// Every list produced since the last call, in order. The painter needs only the last one
    /// (each list repaints from scratch); tests compare all of them.
    pub fn take_frames(&mut self) -> Vec<DisplayList> {
        std::mem::take(&mut self.out.lock().frames)
    }

    /// The last list the sink accepted (repaint after a canvas resize).
    pub fn last_list(&self) -> Option<DisplayList> {
        self.out.lock().last_list.clone()
    }

    /// Side-channel messages produced since the last call, in order.
    pub fn take_messages(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.messages)
    }

    /// Actions parked since the last call; the page forwards each and answers by `id`.
    pub fn take_actions(&mut self) -> Vec<PendingAction> {
        std::mem::take(&mut self.parked.lock().queue)
    }

    /// The App Server answered action `id`: the JSON patch is queued as an `AppResponse`
    /// event (applied by the next `pump`, never from inside this call).
    pub fn action_reply(&mut self, id: u32, patch: serde_json::Value) {
        let reply = {
            let mut p = self.parked.lock();
            p.replies.iter().position(|(i, _)| *i == id).map(|pos| p.replies.remove(pos).1)
        };
        if let Some(r) = reply {
            r.send(patch);
        }
    }

    /// Action `id` got no usable answer (network error, non-200, bad JSON): drop its reply.
    pub fn action_failed(&mut self, id: u32) {
        let mut p = self.parked.lock();
        p.replies.retain(|(i, _)| *i != id);
    }

    /// Unanswered actions currently parked (tests, diagnostics).
    pub fn pending_replies(&self) -> usize {
        self.parked.lock().replies.len()
    }

    /// The core's event loop (tests inspect it; hit tests, focus, layout).
    pub fn event_loop(&self) -> &EventLoop {
        &self.el
    }

    /// Queue handle for an external pusher (tests).
    pub fn pusher(&self) -> Arc<dyn EventPush> {
        Arc::new(self.queue.clone())
    }

    /// Currently announced page id.
    pub fn current_page(&self) -> Option<String> {
        self.session.current_page.read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_paths_are_contained() {
        assert_eq!(media_rel_path("demos/media.thtml", "../assets/a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(media_rel_path("index.thtml", "assets/a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(media_rel_path("demos/x.thtml", "./a.png").as_deref(), Some("demos/a.png"));
        for bad in ["../../a.png", "/etc/passwd", "http://x/a.png", "a\0.png", "", ".", "..", "\\a.png"] {
            assert_eq!(media_rel_path("demos/x.thtml", bad), None, "{bad:?}");
        }
        assert_eq!(media_rel_path("index.thtml", "../a.png"), None);
    }

    #[test]
    fn notices_use_the_server_texts() {
        assert_eq!(encode_notice(&ClientNotice::NavError), [&[0x34u8][..], "Nie znaleziono strony".as_bytes()].concat());
        assert_eq!(encode_notice(&ClientNotice::AppToken("t".into())), vec![0x35, b't']);
        assert_eq!(encode_notice(&ClientNotice::OpenUrl("https://a".into()))[0], 0x33);
    }

    #[test]
    fn hash_matches_the_server_function() {
        // djb2 over chars, as oxiterm-server's `hash_path`.
        assert_eq!(hash_path(""), 5381);
        assert_eq!(hash_path("a"), 5381u32.wrapping_mul(33).wrapping_add('a' as u32));
    }
}
