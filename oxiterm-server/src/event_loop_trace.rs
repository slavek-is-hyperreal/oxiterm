//! Event-loop trace harness (OxiUI Phase 4, step 0).
//!
//! Drives the REAL [`EventLoop`] (its `run()` on its own thread, exactly as the web/SSH
//! hosts do) with scripted [`InputEvent`]s and records everything observable that leaves it:
//! the final cell picture, the final display list and ARIA JSON (GUI sessions), the notices
//! raised through `FrameSink::send_notice` (0x33 / 0x34; captured by `TraceSink`, encoded with
//! the real WS encoding; 0x35 too since step 3), the actions sent to a mocked App
//! Server and the bound state. The result is compared with a committed baseline in
//! `tests/traces/`; an identical refactor yields an identical trace.
//!
//! Regenerating baselines (lead only): `OXI_TRACE_BLESS=1 cargo test -p oxiterm-server
//! event_loop_trace`.
//!
//! # Determinism
//! Frames are NOT recorded one by one (the frame limiter and the 1 s idle timeout make their
//! number timing dependent). Instead every step ends in a quiescent state:
//! 1. events are only fed once the last sink call is at least `GAP` old, so the 60 fps limiter
//!    never defers the frame the event causes (a deferred frame would only appear after the
//!    loop's 1 s idle timeout);
//! 2. after feeding, the harness waits for a NEW sink call (deadline `RENDER_DEADLINE`);
//! 3. then it waits until nothing happened for `QUIET`: no sink call, no notice, no App Server
//!    request.
//! A snapshot is taken only then. Notices are recorded as semantic lines (not raw bytes) so
//! that routing them through the frame sink later (Phase 4 step 2) does not change baselines.
//!
//! # Fake clock (transitions)
//! The scripts above run on the real host clock. Timing dependent pages (CSS transitions) run
//! on a [`FakeClock`] instead (`Run::start_fake`): the loop's `now` is whatever the script
//! sets, so a frame's picture depends only on the fake time it was rendered at. The frame
//! limiter lives on the same clock, so a frozen clock allows no further frame: every expected
//! frame is preceded by `advance`/`step_frame`.
//!
//! Caveat: only `el.clock` is swapped. `ClientSession::clock` (used by the transports to stamp
//! `ResizeDebouncer::push`) stays the system clock, so a fake-clock script must not resize.
//!
//! Not covered: media animations (renderer cache clock, step 8).

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use crate::clock::{FakeClock, MonoTime};

use oxiterm_proto::input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseAction, MouseButton, MouseInput, PointerInput};
use oxiterm_proto::style::{AnsiColor, RenderMode};
use oxiterm_renderer::render::buffer::Cell;
use oxiterm_renderer::render::cell_sink::CellSink;
use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
use oxiterm_renderer::{CellBuffer, FrameSink, LayoutEngine};

use crate::dispatcher::HttpActionHandler;
use crate::session::{ClientSession, EventLoop, PtyDimensions, SessionRegistry};

/// A fresh sink call must be at least this old before an event is fed (see module docs).
const GAP: Duration = Duration::from_millis(30);
/// Nothing may happen for this long before a snapshot is taken.
const QUIET: Duration = Duration::from_millis(100);
/// A burst of events may leave one limiter-deferred frame behind (up to the loop's 1 s idle
/// timeout), so bursts settle for longer than that.
const BURST_QUIET: Duration = Duration::from_millis(1400);
/// Upper bound for "an event must lead to a frame".
const RENDER_DEADLINE: Duration = Duration::from_secs(10);
/// Upper bound for reaching quiescence (an animating page would never get there).
const QUIET_DEADLINE: Duration = Duration::from_secs(10);

const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Every committed baseline (file stem under `tests/traces/`). The stale-baseline test checks
/// the directory against this list.
const TRACES: &[&str] = &[
    "counter-cell",
    "appserver-token-state-cell",
    "counter-gui",
    "todo-cell",
    "todo-gui",
    "input-cell",
    "input-gui",
    "input-demo-cell",
    "todo-demo-cell",
    "navigation-cell",
    "navigation-gui",
    "throttle-cell",
    "appserver-cell",
    "appserver-gui",
    "transition-cell",
    "transition-patch-cell",
    "pointer-sans-gui",
];

// ---------------------------------------------------------------------------------------
// Capturing frame sink
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct SinkState {
    /// Number of sink calls so far (frames and ARIA documents); drives the quiet window.
    seq: u64,
    /// Number of COMPLETE frames: a cell frame, or a display list together with its ARIA
    /// document (the ARIA call is the last one the loop makes for a GUI frame).
    frames: u64,
    last_call: Option<Instant>,
    /// Final cell picture of the latest frame (for display lists: the list painted onto cells).
    grid: Option<String>,
    list: Option<DisplayList>,
    aria: Option<String>,
}

struct Shared {
    state: Mutex<SinkState>,
    cv: Condvar,
}

/// Frame sink double: records the latest picture, never swaps its decision (always "sent").
struct TraceSink {
    shared: Arc<Shared>,
    /// Same channel the session's transport writes to (kept for the wire-format check of
    /// every notice, 0x35 included), so all notices stay one ordered stream for `drain_notices`.
    notice_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl TraceSink {
    fn touch(st: &mut SinkState) {
        st.seq += 1;
        st.last_call = Some(Instant::now());
    }
}

impl FrameSink for TraceSink {
    fn send_frame(&mut self, _front: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
        let mut st = self.shared.state.lock();
        st.grid = Some(grid_snapshot(back));
        st.frames += 1;
        Self::touch(&mut st);
        self.shared.cv.notify_all();
        Ok(true)
    }

    fn send_display_list(&mut self, list: &DisplayList) -> anyhow::Result<bool> {
        let mut painted = CellBuffer::new(COLS, ROWS);
        CellSink::paint(list, &mut painted, |_, _, _| {});
        let mut st = self.shared.state.lock();
        st.grid = Some(grid_snapshot(&painted));
        st.list = Some(list.clone());
        Self::touch(&mut st);
        self.shared.cv.notify_all();
        Ok(true)
    }

    fn send_notice(&mut self, notice: oxiterm_renderer::ClientNotice) -> anyhow::Result<bool> {
        // Capture adapter: encode with the real WS encoding, queue on the notice channel. Not a
        // frame: no `frames`/`seq` bump (the harness watches the channel for the quiet window).
        Ok(self.notice_tx.try_send(crate::web::encode_notice(&notice)).is_ok())
    }

    fn send_a11y(&mut self, json: &[u8]) -> anyhow::Result<bool> {
        let mut st = self.shared.state.lock();
        st.aria = Some(String::from_utf8_lossy(json).into_owned());
        st.frames += 1;
        Self::touch(&mut st);
        self.shared.cv.notify_all();
        Ok(true)
    }
}

// ---------------------------------------------------------------------------------------
// Mock App Server
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct MockInner {
    /// Formatted requests in arrival order.
    requests: Vec<String>,
    /// Connections whose response the harness has not sent yet.
    pending: VecDeque<TcpStream>,
    last_request: Option<Instant>,
}

/// Local HTTP server standing in for the App Server. It records every dispatched action and
/// only answers when the harness says so, which keeps the patch timing under test control.
struct Mock {
    url: String,
    inner: Arc<(Mutex<MockInner>, Condvar)>,
}

impl Mock {
    fn start() -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let inner: Arc<(Mutex<MockInner>, Condvar)> = Arc::new((Mutex::new(MockInner::default()), Condvar::new()));
        let shared = inner.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(stream) = conn else { continue };
                let shared = shared.clone();
                std::thread::spawn(move || Self::handle(stream, shared));
            }
        });
        Mock { url: format!("http://127.0.0.1:{port}/events"), inner }
    }

    fn handle(mut stream: TcpStream, shared: Arc<(Mutex<MockInner>, Condvar)>) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let (header_end, content_len) = loop {
            let Ok(n) = stream.read(&mut chunk) else { return };
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                break (pos + 4, len);
            }
        };
        while buf.len() < header_end + content_len {
            let Ok(n) = stream.read(&mut chunk) else { return };
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let body: serde_json::Value = serde_json::from_slice(&buf[header_end..]).unwrap_or(serde_json::Value::Null);
        // Volatile fields (session_id) are left out on purpose.
        let mut line = format!("action={}", body["action"].as_str().unwrap_or("?"));
        if let Some(tok) = body["app_token"].as_str() {
            let _ = write!(line, " app_token={tok}");
        }
        let state: BTreeMap<String, String> = body["state"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("?").to_string())).collect())
            .unwrap_or_default();
        let state_txt: Vec<String> = state.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let _ = write!(line, " state{{{}}}", state_txt.join(","));

        let (lock, cv) = &*shared;
        let mut g = lock.lock();
        g.requests.push(line);
        g.pending.push_back(stream);
        g.last_request = Some(Instant::now());
        cv.notify_all();
    }

    fn request_count(&self) -> usize {
        self.inner.0.lock().requests.len()
    }

    /// Answers the oldest unanswered request with a JSON body.
    fn answer(&self, body: &str) {
        let (lock, cv) = &*self.inner;
        let deadline = Instant::now() + RENDER_DEADLINE;
        let mut g = lock.lock();
        while g.pending.is_empty() {
            assert!(!cv.wait_until(&mut g, deadline).timed_out(), "mock: no request to answer");
        }
        let mut stream = g.pending.pop_front().unwrap();
        drop(g);
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    }
}

// ---------------------------------------------------------------------------------------
// Trace run
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Web client without display-list caps: cell frames.
    Cell,
    /// Web client with display-list caps: GUI layout, display lists and the ARIA mirror.
    Gui,
}

struct Run {
    mode: Mode,
    session: Arc<ClientSession>,
    _registry: SessionRegistry,
    shared: Arc<Shared>,
    notice_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    thread: Option<std::thread::JoinHandle<()>>,
    mock: Option<Mock>,
    /// Handle of the loop's clock when the run uses the fake clock.
    clock: Option<FakeClock>,
    app_base: PathBuf,
    watch: Vec<String>,
    forbidden: Vec<String>,
    // Bookkeeping between snapshots.
    notices: Vec<String>,
    last_notice: Option<Instant>,
    requests_reported: usize,
    prev_grid: Option<String>,
    prev_list: Option<String>,
    prev_aria: Option<String>,
    steps: usize,
    trace: String,
}

fn key(codepoint: char) -> InputEvent {
    InputEvent::KeyPress(KeyEvent { codepoint, modifiers: KeyModifiers::default(), kind: KeyKind::Press })
}

fn mouse(col: u16, row: u16, action: MouseAction) -> InputEvent {
    InputEvent::MouseEvent(MouseInput { col, row, button: MouseButton::Left, action, modifiers: KeyModifiers::default() })
}

/// Fractional pointer event (`0x03`, display-list sessions): `x`/`y` are 0-based screen cells.
fn pointer(x: f32, y: f32, action: MouseAction) -> InputEvent {
    InputEvent::PointerEvent(PointerInput { x, y, button: MouseButton::Left, action, modifiers: KeyModifiers::default() })
}

impl Run {
    /// Starts a session on `page` (path relative to `app_base`) and waits for the first frame.
    fn start(mode: Mode, app_base: &Path, page: &str, with_mock: bool) -> Run {
        Self::start_inner(mode, app_base, page, with_mock, None, None)
    }

    /// Like `start`, but actions go to an in-process handler instead of the HTTP mock.
    fn start_with_handler(mode: Mode, app_base: &Path, page: &str, handler: Arc<dyn crate::action::ActionHandler>) -> Run {
        Self::start_inner(mode, app_base, page, false, None, Some(handler))
    }

    /// Like `start`, but the loop runs on a fake clock set to `t0_ms`.
    fn start_fake(mode: Mode, app_base: &Path, page: &str, t0_ms: u64) -> Run {
        let clock = FakeClock::new();
        clock.set(MonoTime::from_millis(t0_ms));
        Self::start_inner(mode, app_base, page, false, Some(clock), None)
    }

    fn start_inner(mode: Mode, app_base: &Path, page: &str, with_mock: bool, clock: Option<FakeClock>, handler: Option<Arc<dyn crate::action::ActionHandler>>) -> Run {
        let registry = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 4);
        let session = registry.create_session().unwrap();
        *session.dims.write() = PtyDimensions { cols: COLS, rows: ROWS };
        // Notices are web-only, so both modes are web clients; GUI adds the display-list caps.
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        if mode == Mode::Gui {
            session.display_list_client.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let app_base = app_base.canonicalize().unwrap();
        *session.app_base_dir.write() = Some(app_base.clone());
        let (tx, notice_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4096);
        *session.web_frame_tx.write() = Some((1, tx.clone()));

        let page_path = app_base.join(page);
        let render_mode = if mode == Mode::Gui { RenderMode::Gui } else { RenderMode::Tui };
        let doc = crate::loader::load_thtml_file_mode(&page_path, render_mode).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(1);
        let shared = Arc::new(Shared { state: Mutex::new(SinkState::default()), cv: Condvar::new() });
        let mut el = session.new_event_loop(Arc::new(crate::events::EventBus::new()), output_tx, doc, false);
        el.frame_sink = Box::new(TraceSink { shared: shared.clone(), notice_tx: tx });
        el.source_path = Some(page_path.clone());
        el.base_source_path = Some(page_path);
        if let Some(c) = &clock {
            el.clock = Arc::new(c.clone());
            el.now = el.clock.now();
        }
        let mock = if with_mock {
            let m = Mock::start();
            el.action_handler = Some(Arc::new(HttpActionHandler::new(m.url.clone())));
            Some(m)
        } else {
            None
        };
        if handler.is_some() {
            el.action_handler = handler;
        }
        let thread = std::thread::spawn(move || el.run());

        let mut run = Run {
            mode,
            session,
            _registry: registry,
            shared,
            notice_rx,
            thread: Some(thread),
            mock,
            clock,
            app_base,
            watch: Vec::new(),
            forbidden: Vec::new(),
            notices: Vec::new(),
            last_notice: None,
            requests_reported: 0,
            prev_grid: None,
            prev_list: None,
            prev_aria: None,
            steps: 0,
            trace: String::new(),
        };
        run.wait_render(0);
        run.wait_quiet(QUIET);
        run
    }

    /// Moves the fake clock forward without waiting for anything (the loop is idle).
    fn advance(&self, ms: u64) {
        self.clock.as_ref().expect("fake clock run").advance(Duration::from_millis(ms));
    }

    /// Moves the fake clock forward while the loop is animating and waits for the one frame
    /// that is rendered at the new time.
    fn step_frame(&mut self, ms: u64) {
        self.wait_gap();
        let mark = self.frames();
        self.advance(ms);
        self.wait_render(mark);
        self.wait_quiet(QUIET);
    }

    fn watch(&mut self, keys: &[&str]) {
        self.watch = keys.iter().map(|s| s.to_string()).collect();
    }

    /// Text that must never appear in any recorded picture (grid, display list, ARIA), e.g. a
    /// typed password. The `state:` line is exempt: it reports the bound state itself.
    fn forbid(&mut self, secret: &str) {
        self.forbidden.push(secret.to_string());
    }

    fn frames(&self) -> u64 {
        self.shared.state.lock().frames
    }

    fn drain_notices(&mut self) {
        while let Ok(frame) = self.notice_rx.try_recv() {
            let Some((&tag, payload)) = frame.split_first() else { continue };
            let text = String::from_utf8_lossy(payload).into_owned();
            let line = match tag {
                0x33 => format!("open_url {text}"),
                0x34 if text == "Nie znaleziono strony" => "nav_error".to_string(),
                0x34 if text == "Zwolnij ;)" => "nav_throttle".to_string(),
                0x34 => format!("notice34 {text:?}"),
                0x35 => format!("app_token {text}"),
                // Frames produced by sinks are not part of the semantic notice stream.
                _ => continue,
            };
            self.notices.push(line);
            self.last_notice = Some(Instant::now());
        }
    }

    /// Waits for a complete frame newer than `mark` (a value of `frames()`).
    fn wait_render(&mut self, mark: u64) {
        let deadline = Instant::now() + RENDER_DEADLINE;
        loop {
            {
                let mut st = self.shared.state.lock();
                if st.frames > mark {
                    return;
                }
                let _ = self.shared.cv.wait_until(&mut st, Instant::now() + Duration::from_millis(20));
            }
            self.drain_notices();
            assert!(Instant::now() < deadline, "event loop produced no frame within {RENDER_DEADLINE:?}");
        }
    }

    /// Waits until no sink call, notice or App Server request happened for `quiet`.
    fn wait_quiet(&mut self, quiet: Duration) {
        let deadline = Instant::now() + QUIET_DEADLINE;
        loop {
            assert!(
                Instant::now() < deadline,
                "event loop never went quiet within {QUIET_DEADLINE:?} (animating page?)"
            );
            self.drain_notices();
            let mut latest = self.shared.state.lock().last_call;
            for t in [self.last_notice, self.mock.as_ref().and_then(|m| m.inner.0.lock().last_request)].into_iter().flatten() {
                latest = Some(latest.map_or(t, |l| l.max(t)));
            }
            match latest {
                Some(t) if t.elapsed() < quiet => std::thread::sleep(Duration::from_millis(5)),
                _ => return,
            }
        }
    }

    /// Feeds one event after the GAP rule and settles.
    fn send_with(&mut self, ev: InputEvent, quiet: Duration) {
        self.wait_gap();
        let mark = self.frames();
        let _ = self.session.event_tx.try_send(ev);
        self.wait_render(mark);
        self.wait_quiet(quiet);
    }

    fn send(&mut self, ev: InputEvent) {
        self.send_with(ev, QUIET);
    }

    fn wait_gap(&self) {
        if let Some(t) = self.shared.state.lock().last_call {
            let age = t.elapsed();
            if age < GAP {
                std::thread::sleep(GAP - age);
            }
        }
    }

    /// Waits until the mock has seen exactly `n` requests in total (they arrive on a
    /// dispatcher thread and may land after the quiet window), then settles.
    fn await_requests(&mut self, n: usize) {
        let deadline = Instant::now() + RENDER_DEADLINE;
        while self.mock.as_ref().expect("mock").request_count() < n {
            assert!(Instant::now() < deadline, "App Server request #{n} never arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
        self.wait_quiet(QUIET);
        assert_eq!(self.mock.as_ref().unwrap().request_count(), n, "unexpected extra App Server request");
    }

    /// Answers the oldest pending App Server request with `body` and waits for the frame
    /// the resulting `StatePatched` causes.
    fn answer(&mut self, body: &str) {
        self.wait_gap();
        let mark = self.frames();
        self.mock.as_ref().expect("mock").answer(body);
        self.wait_render(mark);
        self.wait_quiet(QUIET);
    }

    /// What `POST /sessions/{id}/patch` does (web.rs): queue the patch as `PushPatch`; the loop
    /// applies it.
    fn push_patch(&mut self, patch: &str) {
        self.wait_gap();
        let mark = self.frames();
        let patch = oxiterm_proto::input::AppPatch(serde_json::from_str(patch).unwrap());
        let _ = self.session.event_tx.try_send(InputEvent::PushPatch(patch));
        self.wait_render(mark);
        self.wait_quiet(QUIET);
    }

    /// Feeds all events back to back (a flood); settles for longer than the loop's idle
    /// timeout because a limiter-deferred frame may trail the burst.
    fn burst(&mut self, events: Vec<InputEvent>) {
        self.wait_gap();
        let mark = self.frames();
        for ev in events {
            let _ = self.session.event_tx.try_send(ev);
        }
        self.wait_render(mark);
        self.wait_quiet(BURST_QUIET);
    }

    fn press_key(&mut self, c: char) {
        self.send(key(c));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.press_key(c);
        }
    }

    /// Cell (1-based terminal coordinates) at the centre of the first visible node whose
    /// `event-htmx` equals `target`, computed from a layout of the current page that mirrors
    /// the one the loop used (same mode, viewport and state).
    fn locate(&self, target: &str) -> (u16, u16) {
        let page = self.session.current_page.read().clone().expect("current page known after first frame");
        let path = self.app_base.join(page);
        let render_mode = if self.mode == Mode::Gui { RenderMode::Gui } else { RenderMode::Tui };
        let mut doc = crate::loader::load_thtml_file_mode(&path, render_mode).unwrap();
        let state = self.session.state.read();
        EventLoop::inject_initial_state(&mut doc, &*state);
        let mut engine = LayoutEngine::with_mode(render_mode);
        engine.set_viewport_rows(ROWS);
        let layout = engine.compute(&mut doc, COLS, 0, Some(&*state)).unwrap();
        let (ox, oy) = layout.get_centering_offset(&doc, COLS, ROWS);
        let id = doc
            .arena
            .iter()
            .filter(|(_, n)| n.attrs.event_htmx.as_deref() == Some(target))
            .map(|(id, _)| id)
            .find(|id| layout.nodes.get(id).map_or(false, |r| r.cells().width > 0 && r.cells().height > 0))
            .unwrap_or_else(|| panic!("no visible node with event-htmx {target:?}"));
        let r = layout.nodes.get(&id).unwrap().cells();
        (ox + r.x + r.width / 2 + 1, oy + r.y + r.height / 2 + 1)
    }

    /// Screen position (0-based, fractional cells) of the point `dx` cells right of the left
    /// edge and half way down the first visible node whose `event-htmx` equals `target`, taken
    /// from the exact (fractional) rect of a layout that mirrors the loop's. `dx = None` is the
    /// rect's centre.
    fn locate_exact(&self, target: &str, dx: Option<f32>) -> (f32, f32) {
        let page = self.session.current_page.read().clone().expect("current page known after first frame");
        let path = self.app_base.join(page);
        let render_mode = if self.mode == Mode::Gui { RenderMode::Gui } else { RenderMode::Tui };
        let mut doc = crate::loader::load_thtml_file_mode(&path, render_mode).unwrap();
        let state = self.session.state.read();
        EventLoop::inject_initial_state(&mut doc, &*state);
        let mut engine = LayoutEngine::with_mode(render_mode);
        engine.set_viewport_rows(ROWS);
        let layout = engine.compute(&mut doc, COLS, 0, Some(&*state)).unwrap();
        let (ox, oy) = layout.get_centering_offset(&doc, COLS, ROWS);
        let id = doc
            .arena
            .iter()
            .filter(|(_, n)| n.attrs.event_htmx.as_deref() == Some(target))
            .map(|(id, _)| id)
            .find(|id| layout.nodes.get(id).map_or(false, |r| r.width > 0.0 && r.height > 0.0))
            .unwrap_or_else(|| panic!("no visible node with event-htmx {target:?}"));
        let r = layout.nodes.get(&id).unwrap();
        let x = dx.map_or(r.x + r.width / 2.0, |d| if d >= 0.0 { r.x + d } else { r.x + r.width + d });
        (f32::from(ox) + x, f32::from(oy) + r.y + r.height / 2.0)
    }

    /// Fractional pointer press + release `dx` cells inside the left edge (`dx >= 0`) or the
    /// right edge (`dx < 0`) of the node, or at its exact centre (`None`).
    fn click_exact(&mut self, target: &str, dx: Option<f32>) {
        let (x, y) = self.locate_exact(target, dx);
        self.send(pointer(x, y, MouseAction::Press));
        self.send(pointer(x, y, MouseAction::Release));
    }

    /// Mouse press + release at the centre of the node with the given `event-htmx`.
    fn click(&mut self, target: &str) {
        let (col, row) = self.locate(target);
        self.send(mouse(col, row, MouseAction::Press));
        self.send(mouse(col, row, MouseAction::Release));
    }

    /// Records a snapshot of everything observable since the previous one.
    fn snap(&mut self, label: &str) {
        self.wait_quiet(QUIET);
        self.drain_notices();
        self.steps += 1;
        let mut out = String::new();
        writeln!(out, "##### step {}: {}", self.steps, label).unwrap();
        writeln!(out, "page: {}", self.session.current_page.read().clone().unwrap_or_else(|| "-".into())).unwrap();

        if self.notices.is_empty() {
            writeln!(out, "notices: (none)").unwrap();
        } else {
            writeln!(out, "notices:").unwrap();
            for n in self.notices.drain(..) {
                writeln!(out, "  {n}").unwrap();
            }
        }

        if let Some(m) = &self.mock {
            let g = m.inner.0.lock();
            let new = &g.requests[self.requests_reported..];
            if new.is_empty() {
                writeln!(out, "app_server: (none)").unwrap();
            } else {
                writeln!(out, "app_server:").unwrap();
                for r in new {
                    writeln!(out, "  {r}").unwrap();
                }
            }
            self.requests_reported = g.requests.len();
        }

        let state_txt: Vec<String> = {
            let st = self.session.state.read();
            self.watch
                .iter()
                .map(|k| format!("{k}={}", st.get(k).map_or("-".to_string(), |v| format!("{v:?}"))))
                .collect()
        };
        writeln!(out, "state: {}", state_txt.join(" ")).unwrap();

        let frames_start = out.len(); // everything from here on is picture, not state
        let (grid, list, aria) = {
            let st = self.shared.state.lock();
            (st.grid.clone(), st.list.clone(), st.aria.clone())
        };
        let grid_txt = grid.unwrap_or_else(|| "(no frame)\n".into());
        if self.prev_grid.as_deref() == Some(grid_txt.as_str()) {
            writeln!(out, "grid: unchanged").unwrap();
        } else {
            writeln!(out, "grid:").unwrap();
            out.push_str(&grid_txt);
        }
        if self.mode == Mode::Gui {
            let list_txt = list.as_ref().map(list_snapshot).unwrap_or_else(|| "(none)\n".into());
            if self.prev_list.as_deref() == Some(list_txt.as_str()) {
                writeln!(out, "display_list: unchanged").unwrap();
            } else {
                writeln!(out, "display_list:").unwrap();
                out.push_str(&list_txt);
            }
            let aria_txt = aria.as_deref().map(aria_snapshot).unwrap_or_else(|| "(none)\n".into());
            if self.prev_aria.as_deref() == Some(aria_txt.as_str()) {
                writeln!(out, "aria: unchanged").unwrap();
            } else {
                writeln!(out, "aria:").unwrap();
                out.push_str(&aria_txt);
            }
            self.prev_list = Some(list_txt);
            self.prev_aria = Some(aria_txt);
        }
        self.prev_grid = Some(grid_txt);

        for secret in &self.forbidden {
            assert!(!out[frames_start..].contains(secret.as_str()), "secret {secret:?} leaked into the trace at step '{label}'");
        }
        self.trace.push_str(&out);
    }

    /// Stops the loop and returns the finished trace.
    fn finish(mut self) -> String {
        if let Some(m) = &self.mock {
            assert_eq!(m.inner.0.lock().pending.len(), 0, "script left an App Server request unanswered");
        }
        self.session.close();
        if let Some(t) = self.thread.take() {
            t.join().expect("event loop thread panicked");
        }
        std::mem::take(&mut self.trace)
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // A failing script must not leave the loop thread blocked on its channel.
        self.session.close();
    }
}

// ---------------------------------------------------------------------------------------
// Stable text forms
// ---------------------------------------------------------------------------------------

fn color(c: AnsiColor) -> String {
    match c {
        AnsiColor::TrueColor(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        AnsiColor::Color256(i) => format!("c{i}"),
        AnsiColor::Reset => "reset".to_string(),
    }
}

fn style_key(cell: &Cell) -> String {
    let mut s = format!("fg={} bg={}", color(cell.fg), color(cell.bg));
    if cell.bold {
        s.push_str(" bold");
    }
    if cell.underline {
        s.push_str(" underline");
    }
    if cell.italic {
        s.push_str(" italic");
    }
    s
}

/// A glyph as text; control characters (e.g. the CR the predictive echo receives on Enter)
/// are escaped so that baselines stay plain text.
fn vis(c: char) -> String {
    if c.is_control() {
        c.escape_unicode().to_string()
    } else {
        c.to_string()
    }
}

/// Glyph grid, wide-glyph skip cells and per-row style runs (same shape as the renderer's
/// golden cell snapshots).
fn grid_snapshot(buf: &CellBuffer) -> String {
    let mut out = String::new();
    let w = buf.width as usize;
    writeln!(out, "  size {}x{}", buf.width, buf.height).unwrap();
    for y in 0..buf.height as usize {
        let row = &buf.cells[y * w..(y + 1) * w];
        let line: String = row.iter().filter(|c| !c.skip).map(|c| vis(c.ch)).collect();
        writeln!(out, "  |{}|", line.trim_end()).unwrap();
    }
    for y in 0..buf.height as usize {
        let row = &buf.cells[y * w..(y + 1) * w];
        let skips: Vec<String> = row.iter().enumerate().filter(|(_, c)| c.skip).map(|(x, _)| x.to_string()).collect();
        if !skips.is_empty() {
            writeln!(out, "  skip {y}: {}", skips.join(",")).unwrap();
        }
    }
    for y in 0..buf.height as usize {
        let row = &buf.cells[y * w..(y + 1) * w];
        let mut runs = Vec::new();
        let mut start = 0;
        while start < w {
            let key = style_key(&row[start]);
            let mut end = start + 1;
            while end < w && style_key(&row[end]) == key {
                end += 1;
            }
            runs.push(format!("{start}-{} {key}", end - 1));
            start = end;
        }
        writeln!(out, "  style {y}: {}", runs.join(" | ")).unwrap();
    }
    out
}

/// ARIA mirror JSON as one line per node (indented by depth) plus the focus id; lossless
/// except for JSON key order and float formatting, which are fixed here.
fn aria_snapshot(json: &str) -> String {
    fn node(out: &mut String, n: &serde_json::Value, depth: usize) {
        let mut parts = vec![format!("{}", n["role"].as_str().unwrap_or("?"))];
        let mut extras: Vec<String> = Vec::new();
        if let Some(obj) = n.as_object() {
            for (k, v) in obj {
                match k.as_str() {
                    "role" | "children" | "x" | "y" | "w" | "h" => {}
                    _ if v.is_null() => {}
                    _ => extras.push(format!("{k}={v}")),
                }
            }
        }
        extras.sort();
        let f = |k: &str| n[k].as_f64().unwrap_or(0.0);
        parts.push(format!("({:.2},{:.2} {:.2}x{:.2})", f("x"), f("y"), f("w"), f("h")));
        parts.extend(extras);
        writeln!(out, "  {}{}", "  ".repeat(depth), parts.join(" ")).unwrap();
        if let Some(children) = n["children"].as_array() {
            for c in children {
                node(out, c, depth + 1);
            }
        }
    }
    let doc: serde_json::Value = serde_json::from_str(json).expect("ARIA is JSON");
    let mut out = String::new();
    writeln!(out, "  v={} focus={}", doc["v"], doc["focus"]).unwrap();
    if let Some(nodes) = doc["nodes"].as_array() {
        for n in nodes {
            node(&mut out, n, 0);
        }
    }
    out
}

fn rect(r: &oxiterm_renderer::render::display_list::RectF) -> String {
    format!("({:.2},{:.2} {:.2}x{:.2})", r.x, r.y, r.w, r.h)
}

fn mods(m: &oxiterm_renderer::render::display_list::Modifiers) -> String {
    let mut s = String::new();
    if m.bold {
        s.push('b');
    }
    if m.underline {
        s.push('u');
    }
    if m.italic {
        s.push('i');
    }
    s
}

/// One line per display item: kind, node, geometry, colours and text.
fn list_snapshot(list: &DisplayList) -> String {
    let mut out = String::new();
    for item in &list.items {
        match item {
            DisplayItem::Fill { node, rect: r, radius, fg, bg } => {
                writeln!(out, "  Fill n{} {} r={:.2} fg={} bg={}", node.0, rect(r), radius, color(*fg), color(*bg)).unwrap()
            }
            DisplayItem::Border { node, rect: r, radius, fg, bg, .. } => {
                writeln!(out, "  Border n{} {} r={:.2} fg={} bg={}", node.0, rect(r), radius, color(*fg), color(*bg)).unwrap()
            }
            DisplayItem::Path { node, cmds, fill, stroke } => writeln!(
                out,
                "  Path n{} cmds={} fill={:?} stroke={:?}",
                node.0,
                cmds.len(),
                fill.map(color),
                stroke.map(|(c, w)| (color(c), w))
            )
            .unwrap(),
            DisplayItem::Text { node, origin, clip, glyphs, fg, bg, mods: m, .. } => {
                let mut text = String::new();
                let mut row = glyphs.first().map(|g| g.y);
                for g in glyphs {
                    if Some(g.y) != row {
                        text.push('\\');
                        text.push('n');
                        row = Some(g.y);
                    }
                    text.push(g.ch);
                }
                writeln!(
                    out,
                    "  Text n{} @({:.2},{:.2}) clip={} fg={} bg={} mods={} {:?}",
                    node.0, origin.x, origin.y, rect(clip), color(*fg), color(*bg), mods(m), text
                )
                .unwrap()
            }
            DisplayItem::Legacy { node, content } => writeln!(out, "  Legacy n{} {}", node.0, rect(content)).unwrap(),
            DisplayItem::CellGrid { rect: r, cells } => {
                let w = (r.w as usize).max(1);
                writeln!(out, "  CellGrid {} cells={}", rect(r), cells.len()).unwrap();
                for line in cells.chunks(w) {
                    let s: String = line.iter().filter(|c| !c.skip).map(|c| vis(c.ch)).collect();
                    writeln!(out, "    |{}|", s.trim_end()).unwrap();
                }
            }
            DisplayItem::PushClip { rect: r } => writeln!(out, "  PushClip {}", rect(r)).unwrap(),
            DisplayItem::PopClip => writeln!(out, "  PopClip").unwrap(),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Baselines
// ---------------------------------------------------------------------------------------

fn traces_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/traces")
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

fn pages_dir() -> PathBuf {
    traces_dir().join("pages")
}

/// Compares `actual` with `tests/traces/<name>.txt`, or rewrites it when `OXI_TRACE_BLESS` is set.
fn check_baseline(name: &str, actual: &str) {
    assert!(TRACES.contains(&name), "trace '{name}' is not listed in TRACES");
    let path = traces_dir().join(format!("{name}.txt"));
    if std::env::var_os("OXI_TRACE_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing baseline {path:?}; the lead creates it with OXI_TRACE_BLESS=1"));
    if expected == actual {
        return;
    }
    let mut exp = expected.lines();
    let mut act = actual.lines();
    let mut n = 0;
    let mut step = "(before the first step)";
    loop {
        n += 1;
        match (exp.next(), act.next()) {
            (Some(e), Some(a)) if e == a => {
                if e.starts_with("##### ") {
                    step = e;
                }
            }
            (e, a) => panic!(
                "trace {path:?} differs at line {n}, in {step}:\n  expected: {}\n  actual:   {}\n(intended change? the lead re-blesses with OXI_TRACE_BLESS=1)",
                e.unwrap_or("<end of file>"),
                a.unwrap_or("<end of file>")
            ),
        }
    }
}

#[test]
fn trace_baselines_have_no_stale_files() {
    if std::env::var_os("OXI_TRACE_BLESS").is_some() {
        return; // the baselines are being (re)written concurrently by the other tests
    }
    let mut on_disk: Vec<String> = std::fs::read_dir(traces_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("txt")).then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    on_disk.sort();
    let mut expected: Vec<String> = TRACES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(on_disk, expected, "tests/traces/*.txt must match the TRACES list (stale or missing baseline)");
}

// ---------------------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------------------

/// counter.thtml: three buttons clicked by mouse, then by Tab focus + Enter.
fn script_counter(mode: Mode) -> String {
    let mut run = Run::start(mode, &examples_dir(), "demos/counter.thtml", false);
    run.watch(&["counter"]);
    run.snap("initial frame");
    run.click("inc:counter");
    run.snap("click +");
    run.click("inc:counter");
    run.snap("click + again");
    run.click("dec:counter");
    run.snap("click -");
    run.click("clear:counter");
    run.snap("click x");
    run.click("inc:counter");
    run.snap("click + after clear");
    // Keyboard: Tab moves focus through the focusable nodes, Enter activates the focused one.
    run.press_key('\t');
    run.snap("Tab");
    run.press_key('\t');
    run.snap("Tab again");
    run.press_key('\r');
    run.snap("Enter on focused node");
    run.finish()
}

/// pointer_sans.thtml: adjacent proportional buttons whose shared edges fall inside a cell,
/// clicked with the fractional pointer message at their exact centres and at points 0.05 cell
/// inside an edge, in the one cell two buttons share. (A cell event would have sent the
/// whole cell to a single button.)
fn script_pointer_sans() -> String {
    let mut run = Run::start(Mode::Gui, &pages_dir(), "pointer_sans.thtml", false);
    run.watch(&["hit"]);
    run.snap("initial frame");
    run.click_exact("set:hit=i", None);
    run.snap("pointer at the centre of i");
    run.click_exact("set:hit=m", None);
    run.snap("pointer at the centre of mm");
    run.click_exact("set:hit=l", None);
    run.snap("pointer at the centre of ll");
    run.click_exact("set:hit=w", None);
    run.snap("pointer at the centre of Wide one");
    // The shared cell of i and mm: just inside the right edge of i, then just inside the
    // left edge of mm.
    run.click_exact("set:hit=i", Some(-0.05));
    run.snap("pointer 0.05 inside the right edge of i");
    run.click_exact("set:hit=m", Some(0.05));
    run.snap("pointer 0.05 inside the left edge of mm");
    run.finish()
}

#[test]
fn trace_pointer_sans_gui() {
    check_baseline("pointer-sans-gui", &script_pointer_sans());
}

#[test]
fn trace_counter_cell() {
    check_baseline("counter-cell", &script_counter(Mode::Cell));
}

#[test]
fn trace_counter_gui() {
    check_baseline("counter-gui", &script_counter(Mode::Gui));
}

/// `<for>` re-expansion. The loop expands `<for>` blocks at start and on `StatePatched` only,
/// so the list below goes stale after a local `append:` until the next `StatePatched`; the
/// trace records exactly that.
fn script_todo(mode: Mode) -> String {
    let mut run = Run::start(mode, &pages_dir(), "todo_for.thtml", false);
    run.watch(&["task_list"]);
    run.snap("initial frame");
    run.click("append:task_list=milk");
    run.snap("append milk (list not re-expanded yet)");
    run.click("append:task_list=eggs");
    run.snap("append eggs");
    run.send(InputEvent::StatePatched);
    run.snap("StatePatched re-expands the <for> block");
    run.click("append:task_list=milk");
    run.send(InputEvent::StatePatched);
    run.snap("append milk + StatePatched");
    run.click("clear:task_list");
    run.send(InputEvent::StatePatched);
    run.snap("clear + StatePatched");
    run.finish()
}

#[test]
fn trace_todo_cell() {
    check_baseline("todo-cell", &script_todo(Mode::Cell));
}

#[test]
fn trace_todo_gui() {
    check_baseline("todo-gui", &script_todo(Mode::Gui));
}

/// Typing into a plain and a password input (the password stays masked), backspace, and a
/// clear button reached through Tab + Enter.
fn script_input(mode: Mode) -> String {
    let mut run = Run::start(mode, &pages_dir(), "input_pw.thtml", false);
    run.watch(&["name_input", "secret"]);
    run.forbid("hunter"); // also covers "hunter2"
    run.snap("initial frame (first input focused)");
    run.type_text("Ana");
    run.snap("typed Ana");
    run.press_key('\u{7f}');
    run.snap("backspace");
    run.press_key('\t');
    run.snap("Tab to the password input");
    run.type_text("hunter2");
    run.snap("typed the password");
    run.press_key('\u{7f}');
    run.snap("password backspace");
    run.press_key('\t');
    run.snap("Tab to the clear button");
    run.press_key('\r');
    run.snap("Enter clears the name");
    run.finish()
}

#[test]
fn trace_input_cell() {
    check_baseline("input-cell", &script_input(Mode::Cell));
}

#[test]
fn trace_input_gui() {
    check_baseline("input-gui", &script_input(Mode::Gui));
}

/// The shipped todo demo (no `<for>` there): `append:` clicks only update the bound list text.
#[test]
fn trace_todo_demo_cell() {
    let mut run = Run::start(Mode::Cell, &examples_dir(), "demos/todo.thtml", false);
    run.watch(&["task_list"]);
    run.snap("initial frame");
    run.click("append:task_list=new task");
    run.snap("append");
    run.click("append:task_list=new task");
    run.snap("append again");
    check_baseline("todo-demo-cell", &run.finish());
}

/// The shipped input demo: typing into the name field and clearing it by mouse.
#[test]
fn trace_input_demo_cell() {
    let mut run = Run::start(Mode::Cell, &examples_dir(), "demos/input.thtml", false);
    run.watch(&["name_input", "search_input"]);
    run.snap("initial frame");
    run.press_key('\t');
    run.snap("Tab to the name input");
    run.type_text("Ola");
    run.snap("typed Ola");
    run.press_key('\u{7f}');
    run.snap("backspace");
    run.click("clear:name_input");
    run.snap("click Clear name");
    check_baseline("input-demo-cell", &run.finish());
}

/// Page changes: by clicks, by `NavigateTo`, to a missing page and a traversal attempt
/// (both must give the same generic notice), and back. `<page>/../x` paths are recorded
/// as they are (the loop does not normalise `current_page`).
fn script_navigation(mode: Mode) -> String {
    let mut run = Run::start(mode, &examples_dir(), "features.thtml", false);
    run.watch(&["tab", "counter"]);
    run.snap("initial frame (features)");
    run.click("demos/navigation.thtml");
    run.snap("click row -> navigation demo");
    run.click("set:tab=keys");
    run.snap("tab keys (no page change)");
    run.click("set:tab=bind");
    run.snap("tab bind");
    run.click("../features.thtml");
    run.snap("click Back -> features");
    run.send(InputEvent::NavigateTo("no_such_page".to_string()));
    run.snap("NavigateTo missing page");
    run.send(InputEvent::NavigateTo("../../../etc/passwd".to_string()));
    run.snap("NavigateTo traversal");
    run.send(InputEvent::NavigateTo("demos/counter".to_string()));
    run.snap("NavigateTo counter");
    run.click("inc:counter");
    run.snap("click + on the new page");
    run.finish()
}

#[test]
fn trace_navigation_cell() {
    check_baseline("navigation-cell", &script_navigation(Mode::Cell));
}

#[test]
fn trace_navigation_gui() {
    check_baseline("navigation-gui", &script_navigation(Mode::Gui));
}

/// NAV throttle: a flood of failing navigations. The bucket holds 5 tokens, so five
/// `nav_error` notices and exactly one `nav_throttle` notice are expected; after the bucket
/// refilled (the burst settles for > 1 s) navigation works again.
#[test]
fn trace_throttle_cell() {
    let mut run = Run::start(Mode::Cell, &examples_dir(), "demos/counter.thtml", false);
    run.snap("initial frame");
    run.burst((0..7).map(|_| InputEvent::NavigateTo("missing".to_string())).collect());
    run.snap("flood of 7 failing navigations");
    run.send(InputEvent::NavigateTo("input".to_string()));
    run.snap("navigation accepted after refill");
    check_baseline("throttle-cell", &run.finish());
}

/// App Server round trips against a mock: dispatched action + state snapshot, a state patch
/// (reserved keys are refused), `open_url` and `set_app_token` in a patch (the token sync
/// is dispatched again), and the HTTP push-patch path (`POST /sessions/{id}/patch`).
fn script_appserver(mode: Mode) -> String {
    let mut run = Run::start(mode, &examples_dir(), "demos/counter.thtml", true);
    run.watch(&["counter", "banner", "_hacked", "_app_token"]);
    run.snap("initial frame");
    run.click("inc:counter");
    run.await_requests(1);
    run.snap("click + (request pending)");
    run.answer(r#"{"counter": 41, "banner": "hello", "_hacked": "1"}"#);
    run.snap("patch applied");
    run.click("inc:counter");
    run.await_requests(2);
    // No bound key in this patch (historic: before Phase 4 step 3 the token sync raced the patch;
    // `appserver-token-state-cell` now covers a bound key together with `set_app_token`).
    run.answer(r#"{"open_url": "https://example.com/auth?x=1", "set_app_token": "tok-abc", "banner": "second"}"#);
    run.await_requests(3);
    run.snap("patch with open_url + set_app_token (token sync dispatched)");
    run.answer("{}");
    run.snap("token sync answered");
    run.push_patch(r#"{"set_app_token": "tok-push", "counter": 99}"#);
    run.snap("pushed patch with set_app_token");
    // Server-initiated navigation: `navigate` is not stored as state, the rest of the patch is
    // applied first, then the page changes (same containment + NAV throttle as event-htmx).
    run.push_patch(r#"{"navigate": "features.thtml", "banner": "navigated"}"#);
    run.snap("pushed patch with navigate");
    run.push_patch(r#"{"navigate": "../outside.thtml"}"#);
    run.snap("navigate outside the app dir is refused");
    run.finish()
}

#[test]
fn trace_appserver_cell() {
    check_baseline("appserver-cell", &script_appserver(Mode::Cell));
}

#[test]
fn trace_appserver_gui() {
    check_baseline("appserver-gui", &script_appserver(Mode::Gui));
}

/// Regression for the pre-step-3 race: a response that sets a bound key AND `set_app_token`.
/// The loop applies the patch first, so the `app_token_sync` request deterministically carries
/// the patched value (500, not the local increment 1).
#[test]
fn trace_appserver_token_state_cell() {
    let mut run = Run::start(Mode::Cell, &examples_dir(), "demos/counter.thtml", true);
    run.watch(&["counter", "_app_token"]);
    run.snap("initial frame");
    run.click("inc:counter");
    run.await_requests(1);
    run.snap("click + (request pending)");
    run.answer(r#"{"set_app_token": "tok-x", "counter": 500}"#);
    run.await_requests(2);
    run.snap("patch sets counter and token together (token sync carries counter=500)");
    run.answer("{}");
    run.snap("token sync answered");
    check_baseline("appserver-token-state-cell", &run.finish());
}

/// The Phase 4 "no HTTP in the core" criterion in miniature: an in-process `ActionHandler`
/// answers `inc:counter` on the shipped counter demo with a patch; the loop applies it and
/// renders the new value. No server, no thread, no ureq.
#[test]
fn trace_in_process_action_handler() {
    let handler = Arc::new(crate::action::FnActionHandler(|p: &crate::action::DispatchPayload| {
        assert_eq!(p.action, "inc:counter");
        Some(serde_json::json!({ "counter": 77 }))
    }));
    let mut run = Run::start_with_handler(Mode::Cell, &examples_dir(), "demos/counter.thtml", handler);
    run.watch(&["counter"]);
    run.snap("initial frame");
    run.click("inc:counter");
    run.snap("click + answered in-process");
    let trace = run.finish();
    assert!(trace.contains("state: counter=Int(77)"), "handler patch not applied:\n{trace}");
    // The new value must be in the final frame of step 2 (not just the `state:` line).
    let step2 = trace.split("##### step 2").nth(1).expect("step 2");
    assert!(
        step2.lines().filter(|l| !l.starts_with("state:")).any(|l| l.contains("77")),
        "new value not rendered:\n{step2}"
    );
}

/// Two bars with an ease-out and a spring width transition, driven by a button press on a fake
/// clock: the picture at each fake time is deterministic. Time moves only through the script.
fn script_transition() -> String {
    let mut run = Run::start_fake(Mode::Cell, &pages_dir(), "transition_bars.thtml", 1000);
    run.watch(&["prog"]);
    run.snap("initial frame (fake t=0)");
    // Idle loop: moving the clock alone renders nothing; the limiter then allows the next frame.
    run.advance(100);
    // Press only: a release would arrive at frozen time and the limiter would render nothing.
    let (col, row) = run.locate("set:prog=24");
    run.send(mouse(col, row, MouseAction::Press));
    run.snap("press Grow: prog=24 (transitions start, t=0 of the transition)");
    for (i, ms) in [50u64, 50, 100, 100, 100, 100, 100].into_iter().enumerate() {
        run.step_frame(ms);
        run.snap(&format!("transition frame {} (+{ms} ms)", i + 1));
    }
    run.finish()
}

/// Same bars, but `prog` arrives as a pushed state patch (App Server path) instead of a click.
/// Regression for a bug where the StatePatched handler synced the patch into the document before
/// the render, so `restore_baselines` reverted it and no transition ever started.
fn script_transition_patch() -> String {
    let mut run = Run::start_fake(Mode::Cell, &pages_dir(), "transition_bars.thtml", 1000);
    run.watch(&["prog"]);
    run.snap("initial frame (fake t=0)");
    run.advance(100);
    run.push_patch(r#"{"prog": 24}"#);
    run.snap("pushed patch prog=24 (transitions start, t=0 of the transition)");
    for (i, ms) in [50u64, 50, 100, 100, 100, 100, 100].into_iter().enumerate() {
        run.step_frame(ms);
        run.snap(&format!("transition frame {} (+{ms} ms)", i + 1));
    }
    run.finish()
}

#[test]
fn trace_transition_patch_cell() {
    check_baseline("transition-patch-cell", &script_transition_patch());
}

#[test]
fn trace_transition_cell() {
    check_baseline("transition-cell", &script_transition());
}
