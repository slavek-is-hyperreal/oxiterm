//! OxiUI Stage 7.1 gate 1: the in-browser local mode produces the frames a thin client gets.
//!
//! `LocalSession` (the Rust glue of `?local=1`: page index -> `MemPageSource`, step API with a
//! host clock, `LocalSink`, input as the web client's own byte messages) is driven with a script
//! of input messages. An independent reference session is built the way a server session is built
//! (`FakeClock`, `MemPageSource`, the three `0x12` flags, a sink that does what the wire does:
//! `decode(encode_for(list, 3))`) and driven with the same script as typed `InputEvent`s, not
//! through the glue's decoder. After every step the frames must be equal and the wire bytes of
//! both must be identical (bit-exact floats). The ARIA mirror JSON, the page announce and the
//! notices are compared too.
//!
//! Native test of the glue: the wasm wrapper (`local_wasm.rs`) only adds the painter call and
//! JS plumbing on top of `LocalSession`.
#![cfg(feature = "local")]
#![allow(clippy::pedantic)]

use std::sync::Arc;
use std::time::Duration;

use oxiterm_core::action::{DispatchPayload, FnActionHandler};
use oxiterm_core::clock::{FakeClock, MonoTime};
use oxiterm_core::event_loop::EventLoop;
use oxiterm_core::loop_io::{EventSource, LoopHandles, LoopHost, MemoryQueue};
use oxiterm_core::page_source::{MemPageSource, PageSource};
use oxiterm_core::session_state::{PtyDimensions, SessionState};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseAction, MouseButton, PointerInput};
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::render::buffer::CellBuffer;
use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
use oxiterm_renderer::render::wire;
use oxiterm_renderer::{ClientNotice, FrameSink};
use oxiterm_web::local::{encode_notice, LocalConfig, LocalSession, PendingAction};
use parking_lot::Mutex;

const COUNTER: &str = include_str!("../../examples/demos/counter.thtml");
const COUNTER_MOBILE: &str = include_str!("../../examples/demos/counter_mobile.thtml");
const STATE_DEMO: &str = include_str!("../../examples/demos/state.thtml");
const SANS: &str = include_str!("../../oxiterm-server/benches/pages/sans.thtml");
/// A proportional page whose text is bound to state an App Server answers.
const APP_PAGE: &str = r#"<style>
.p { font-family: sans-serif; wrap: word; width: 36; fg: #e2e8f0; bg: #102030 }
.b { font-family: sans-serif; font-size: 20px; padding-left: 2; padding-right: 2; border-style: single; margin-top: 1 }
</style>
<box style="flex-direction: column; width: 44; padding: 1;">
<text class="p" bind-state="c">zero</text>
<button class="b" event-htmx="inc:c">Increment the counter</button>
<button class="b" event-htmx="reset:c">Reset</button>
<button class="b" event-htmx="go.thtml">Go</button>
</box>"#;

fn pages() -> Vec<(String, String)> {
    vec![
        ("counter.thtml".into(), COUNTER.into()),
        ("counter_mobile.thtml".into(), COUNTER_MOBILE.into()),
        ("demos/state.thtml".into(), STATE_DEMO.into()),
        ("sans.thtml".into(), SANS.into()),
        ("app.thtml".into(), APP_PAGE.into()),
        ("go.thtml".into(), COUNTER.into()),
    ]
}

// ---- input messages, as the page's JS builds them -------------------------------------------

fn key_msg(cp: u32, flags: u8) -> Vec<u8> {
    let mut v = vec![0x01];
    v.extend_from_slice(&cp.to_le_bytes());
    v.push(flags);
    v
}
fn pointer_msg(x: f32, y: f32, button: u8, action: u8) -> Vec<u8> {
    let mut v = vec![0x03];
    v.extend_from_slice(&x.to_le_bytes());
    v.extend_from_slice(&y.to_le_bytes());
    v.extend_from_slice(&[button, action, 0]);
    v
}
fn resize_msg(cols: u16, rows: u16) -> Vec<u8> {
    let mut v = vec![0x10];
    v.extend_from_slice(&cols.to_le_bytes());
    v.extend_from_slice(&rows.to_le_bytes());
    v
}
fn nav_msg(p: &str) -> Vec<u8> {
    let mut v = vec![0x31];
    v.extend_from_slice(p.as_bytes());
    v
}

/// The same events, typed (the reference does not use the glue's decoders).
fn key_event(cp: char, flags: u8) -> InputEvent {
    InputEvent::KeyPress(KeyEvent {
        codepoint: cp,
        modifiers: KeyModifiers { shift: flags & 1 != 0, ctrl: flags & 2 != 0, alt: flags & 4 != 0, ..Default::default() },
        kind: KeyKind::Press,
    })
}
fn pointer_event(x: f32, y: f32, button: MouseButton, action: MouseAction) -> InputEvent {
    InputEvent::PointerEvent(PointerInput { x, y, button, action, modifiers: KeyModifiers::default() })
}

// ---- the reference: a server-style session ---------------------------------------------------

struct Host;
impl LoopHost for Host {
    fn touch_activity(&self) {}
    fn idle_for(&self) -> Duration {
        Duration::ZERO
    }
    fn loop_exited(&self) {}
}

#[derive(Default)]
struct RefOut {
    frames: Vec<DisplayList>,
    a11y: Vec<Vec<u8>>,
    notices: Vec<ClientNotice>,
    last: Option<Vec<u8>>,
    last_a11y: Option<Vec<u8>>,
}
struct RefSink(Arc<Mutex<RefOut>>);
impl FrameSink for RefSink {
    fn send_frame(&mut self, _f: &CellBuffer, _b: &CellBuffer) -> anyhow::Result<bool> {
        Ok(false)
    }
    /// What the WebSocket sink does, with the receiving end of the wire folded in.
    fn send_display_list(&mut self, l: &DisplayList) -> anyhow::Result<bool> {
        let mut o = self.0.lock();
        let bytes = wire::encode_for(l, 3);
        if o.last.as_deref() == Some(bytes.as_slice()) {
            return Ok(false);
        }
        o.last = Some(bytes.clone());
        o.frames.push(wire::decode(&bytes).expect("thin client decodes"));
        Ok(true)
    }
    fn send_a11y(&mut self, json: &[u8]) -> anyhow::Result<bool> {
        let mut o = self.0.lock();
        if o.last_a11y.as_deref() == Some(json) {
            return Ok(false);
        }
        o.last_a11y = Some(json.to_vec());
        o.a11y.push(json.to_vec());
        Ok(true)
    }
    fn send_notice(&mut self, n: ClientNotice) -> anyhow::Result<bool> {
        self.0.lock().notices.push(n);
        Ok(true)
    }
}

struct Reference {
    el: EventLoop,
    session: Arc<SessionState>,
    queue: MemoryQueue,
    out: Arc<Mutex<RefOut>>,
    actions: Arc<Mutex<Vec<DispatchPayload>>>,
}

impl Reference {
    /// `app`: answers `inc:c` / `reset:c` synchronously (an in-process App Server).
    fn new(first: &str, cols: u16, rows: u16, app: bool) -> Self {
        let clock = Arc::new(FakeClock::new());
        clock.set(MonoTime::from_millis(1000));
        let session = Arc::new(SessionState::new(0, clock.clone()));
        let on = std::sync::atomic::Ordering::SeqCst;
        *session.dims.write() = PtyDimensions { cols, rows };
        session.display_list_client.store(true, on);
        session.display_list_v3.store(true, on);
        session.is_web_client.store(true, on);
        session.attach_identity(oxiterm_core::identity::UserIdentity::guest());
        let pages = Arc::new(MemPageSource::new(pages()));
        let page = pages.resolve(None, first, false).unwrap();
        let doc = pages.load(&page, RenderMode::Gui).unwrap();
        let queue = MemoryQueue::new(256);
        let handles = LoopHandles {
            state: session.clone(),
            events_in: Arc::new(Mutex::new(queue.clone())),
            events_out: Arc::new(queue.clone()),
            host: Arc::new(Host),
        };
        let out = Arc::new(Mutex::new(RefOut::default()));
        let mut el = EventLoop::new(handles, Arc::new(oxiterm_core::events::EventBus::new()), Box::new(RefSink(out.clone())), pages, doc);
        el.clock = clock;
        el.source_path = Some(page.actual.clone());
        el.base_source_path = Some(page.base.clone());
        let actions = Arc::new(Mutex::new(Vec::new()));
        if app {
            let log = actions.clone();
            let n = std::sync::atomic::AtomicU32::new(0);
            el.action_handler = Some(Arc::new(FnActionHandler(move |p: &DispatchPayload| {
                log.lock().push(p.clone());
                match p.action.as_str() {
                    "inc:c" => Some(serde_json::json!({ "c": format!("Clicked {} times", n.fetch_add(1, on) + 1) })),
                    "reset:c" => Some(serde_json::json!({ "c": "zero" })),
                    _ => None,
                }
            })));
        }
        el.prepare_session();
        Self { el, session, queue, out, actions }
    }

    fn step(&mut self, now_ms: u64, event: Option<InputEvent>) {
        let now = MonoTime::from_millis(now_ms);
        if let Some(e) = event {
            self.el.handle_event(e, now);
        }
        let mut q = self.queue.clone();
        while let Some(e) = q.try_recv() {
            self.el.handle_event(e, now);
        }
        self.el.tick(now);
    }

    fn resize(&mut self, now_ms: u64, cols: u16, rows: u16) {
        let now = MonoTime::from_millis(now_ms);
        self.session.resize_debouncer.write().push(PtyDimensions { cols, rows }, now);
        self.step(now_ms, Some(InputEvent::Resize { cols, rows }));
    }
}

// ---- the local side -------------------------------------------------------------------------

struct Local {
    s: LocalSession,
    frames: Vec<DisplayList>,
    a11y_msgs: Vec<Vec<u8>>,
    other_msgs: Vec<Vec<u8>>,
    /// Actions the "page" has not forwarded yet.
    parked: Vec<PendingAction>,
}

impl Local {
    fn new(first: &str, cols: u16, rows: u16, actions: bool) -> Self {
        let cfg = LocalConfig { start_page: first.into(), cols, rows, mobile: false, actions, app_token: String::new() };
        Self { s: LocalSession::new(&cfg, pages(), 1000.0).unwrap(), frames: vec![], a11y_msgs: vec![], other_msgs: vec![], parked: vec![] }
    }

    fn pump(&mut self, now_ms: u64) {
        self.s.pump(now_ms as f64);
        self.frames.extend(self.s.take_frames());
        for m in self.s.take_messages() {
            if m[0] == 0x37 {
                self.a11y_msgs.push(m[1..].to_vec());
            } else {
                self.other_msgs.push(m);
            }
        }
        self.parked.extend(self.s.take_actions());
    }

    fn step(&mut self, now_ms: u64, msg: Option<Vec<u8>>) {
        if let Some(m) = msg {
            self.s.input(&m, now_ms as f64);
        }
        self.pump(now_ms);
    }
}

/// Frames, ARIA mirror and wire bytes equal, from the start up to now.
fn assert_agree(l: &Local, r: &Reference, what: &str) {
    let rf = r.out.lock();
    assert_eq!(l.frames.len(), rf.frames.len(), "{what}: number of frames");
    for (i, (a, b)) in l.frames.iter().zip(&rf.frames).enumerate() {
        assert_eq!(a, b, "{what}: frame {i} differs");
        // bit-exact floats: the wire bytes are the proof
        assert_eq!(wire::encode_for(a, 3), wire::encode_for(b, 3), "{what}: frame {i} wire bytes differ");
    }
    assert_eq!(l.a11y_msgs, rf.a11y, "{what}: ARIA mirror JSON");
}

fn shaped(l: &DisplayList) -> Vec<&DisplayItem> {
    l.items.iter().filter(|i| matches!(i, DisplayItem::Text { shaped: Some(_), .. })).collect()
}

fn action_of(el: &EventLoop, mut id: NodeId) -> Option<String> {
    loop {
        if let Some(a) = el.doc.get_node(id)?.attrs.event_htmx.clone() {
            return Some(a);
        }
        id = *el.parent_map.get(&id)?;
    }
}

/// Centre (reference cells) of a glyph of the shaped run owned by the button with `want`.
fn button_point(el: &EventLoop, list: &DisplayList, want: &str) -> (f32, f32) {
    for item in shaped(list) {
        let DisplayItem::Text { node, origin, glyphs, shaped: Some(run), .. } = item else { unreachable!() };
        if action_of(el, *node).as_deref() == Some(want) {
            let g: Vec<_> = glyphs.iter().filter(|g| g.ch != ' ').collect();
            let m = g[g.len() / 2];
            return (origin.x + m.x + m.advance / 2.0, origin.y + m.y + run.line_height / 2.0);
        }
    }
    panic!("no shaped text of a button with action {want}");
}

#[test]
fn counter_demo_local_equals_thin_over_a_scripted_session() {
    let mut l = Local::new("counter.thtml", 80, 24, false);
    let mut r = Reference::new("counter.thtml", 80, 24, false);

    l.pump(1000);
    r.step(1000, None);
    assert!(!l.frames.is_empty() && !shaped(&l.frames[0]).is_empty(), "the demo is proportional in GUI mode");
    assert_agree(&l, &r, "first frame");
    assert_eq!(l.s.current_page().as_deref(), Some("counter.thtml"));
    assert!(l.other_msgs.iter().any(|m| m[0] == 0x30 && &m[1..] == b"counter.thtml"), "page announce 0x30");

    // resize wider and taller (debounced)
    l.step(1200, Some(resize_msg(100, 30)));
    r.resize(1200, 100, 30);
    l.pump(1301);
    r.step(1301, None);
    assert_agree(&l, &r, "resized");
    assert_eq!(l.s.event_loop().session.dims.read().cols, 100);

    // click the + button where the shaped glyphs were drawn: press and release
    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "inc:counter");
    let before = l.frames.len();
    l.step(1400, Some(pointer_msg(x, y, 0, 0)));
    r.step(1400, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Press)));
    l.step(1440, Some(pointer_msg(x, y, 0, 1)));
    r.step(1440, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Release)));
    assert!(l.frames.len() > before, "the click changed the picture (built-in state action, no server)");
    assert_agree(&l, &r, "after the click");
    // a second and third click on the (re-laid-out) button
    for t in [1600u64, 1800] {
        let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "inc:counter");
        l.step(t, Some(pointer_msg(x, y, 0, 0)));
        r.step(t, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Press)));
        l.step(t + 40, Some(pointer_msg(x, y, 0, 1)));
        r.step(t + 40, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Release)));
    }
    assert_agree(&l, &r, "after three clicks");

    // keyboard: Tab moves the focus ring, Enter activates
    l.step(2000, Some(key_msg(9, 0)));
    r.step(2000, Some(key_event('\t', 0)));
    l.step(2100, Some(key_msg(13, 0)));
    r.step(2100, Some(key_event('\r', 0)));
    assert_agree(&l, &r, "keyboard");

    // wheel and a pointer move
    l.step(2300, Some(pointer_msg(3.5, 4.5, 4, 0)));
    r.step(2300, Some(pointer_event(3.5, 4.5, MouseButton::WheelDown, MouseAction::Press)));
    l.step(2400, Some(pointer_msg(5.25, 6.75, 5, 2)));
    r.step(2400, Some(pointer_event(5.25, 6.75, MouseButton::None, MouseAction::Move)));
    assert_agree(&l, &r, "wheel and move");

    // navigation to a page in a subdirectory (page announce) and to a missing one (notice)
    l.step(3000, Some(nav_msg("demos/state.thtml")));
    r.step(3000, Some(InputEvent::NavigateTo("demos/state.thtml".into())));
    assert_agree(&l, &r, "navigated");
    assert_eq!(l.s.current_page().as_deref(), Some("demos/state.thtml"));
    assert!(l.other_msgs.iter().any(|m| m[0] == 0x30 && &m[1..] == b"demos/state.thtml"));
    let n_before = l.other_msgs.len();
    l.step(5000, Some(nav_msg("../secret.thtml")));
    r.step(5000, Some(InputEvent::NavigateTo("../secret.thtml".into())));
    assert_agree(&l, &r, "failed navigation");
    assert_eq!(r.out.lock().notices, vec![ClientNotice::NavError]);
    assert_eq!(l.other_msgs[n_before..].iter().filter(|m| **m == encode_notice(&ClientNotice::NavError)).count(), 1);

    // every list of the session survives the wire bit-exactly (the thin path), and the cost of
    // that round trip is what local mode saves per frame
    let t = std::time::Instant::now();
    for f in &l.frames {
        let bytes = wire::encode_for(f, 3);
        assert_eq!(&wire::decode(&bytes).unwrap(), f);
    }
    eprintln!("wire encode+decode of {} frames: {:?} total", l.frames.len(), t.elapsed());
}

#[test]
fn sans_fixture_and_mobile_variant_agree() {
    let mut l = Local::new("sans.thtml", 80, 24, false);
    let mut r = Reference::new("sans.thtml", 80, 24, false);
    l.pump(1000);
    r.step(1000, None);
    assert_agree(&l, &r, "sans first frame");
    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "dec:counter");
    l.step(1100, Some(pointer_msg(x, y, 0, 0)));
    r.step(1100, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Press)));
    l.step(1140, Some(pointer_msg(x, y, 0, 1)));
    r.step(1140, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Release)));
    assert_agree(&l, &r, "sans click");

    // `_mobile` variant: the session starts with it and 0x11 switches to it
    let cfg = LocalConfig { start_page: "counter.thtml".into(), cols: 40, rows: 30, mobile: true, actions: false, app_token: String::new() };
    let mut m = LocalSession::new(&cfg, pages(), 1000.0).unwrap();
    m.pump(1000.0);
    let mobile_frame = m.take_frames().pop().unwrap();
    let mut d = Local::new("counter.thtml", 40, 30, false);
    d.pump(1000);
    assert_ne!(&mobile_frame, d.frames.last().unwrap(), "the mobile variant is another page");
    assert_eq!(m.current_page().as_deref(), Some("counter.thtml"), "the page id is the non-mobile name");
}

#[test]
fn app_actions_are_deferred_and_converge_to_the_synchronous_result() {
    let mut l = Local::new("app.thtml", 80, 24, true);
    let mut r = Reference::new("app.thtml", 80, 24, true);
    l.pump(1000);
    r.step(1000, None);
    assert_agree(&l, &r, "first frame");

    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "inc:c");
    l.step(1100, Some(pointer_msg(x, y, 0, 0)));
    r.step(1100, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Press)));
    l.step(1140, Some(pointer_msg(x, y, 0, 1)));
    r.step(1140, Some(pointer_event(x, y, MouseButton::Left, MouseAction::Release)));

    // the loop did not block: the action is parked, unanswered, and carries the payload
    assert_eq!(l.parked.len(), 1);
    assert_eq!(l.s.pending_replies(), 1);
    let sent: DispatchPayload = serde_json::from_str(&l.parked[0].body).unwrap();
    assert_eq!(sent.action, "inc:c");
    assert_eq!(sent.action, r.actions.lock()[0].action);
    assert_eq!(sent.state, r.actions.lock()[0].state, "same state snapshot as the synchronous handler got");
    assert_eq!((sent.username.as_deref(), sent.auth_method.as_deref()), (Some("guest"), Some("Guest")));
    let frames_before_reply = l.frames.len();

    // "the fetch resolves": the App Server answered (the proxy returns the patch verbatim)
    let id = l.parked.remove(0).id;
    l.s.action_reply(id, serde_json::json!({ "c": "Clicked 1 times" }));
    assert_eq!(l.s.pending_replies(), 0);
    assert_eq!(l.frames.len(), frames_before_reply, "nothing is applied from inside the reply call");
    l.pump(1200);
    assert!(l.frames.len() > frames_before_reply, "the reply was applied by the next pump and produced a frame");
    r.step(1200, None);
    // the synchronous reference applied the reply one tick earlier: the final pictures agree
    assert_eq!(l.frames.last(), r.out.lock().frames.last(), "converged picture");
    let text = |f: &DisplayList| -> Vec<String> {
        shaped(f)
            .into_iter()
            .map(|i| match i {
                DisplayItem::Text { glyphs, .. } => glyphs.iter().map(|g| g.ch).collect(),
                _ => unreachable!(),
            })
            .collect()
    };
    assert!(text(l.frames.last().unwrap()).iter().any(|t| t == "Clicked 1 times"));

    // a failed fetch (server down, non-200, bad JSON): no reply, no frame, the page keeps working
    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "reset:c");
    l.step(1300, Some(pointer_msg(x, y, 0, 0)));
    l.step(1340, Some(pointer_msg(x, y, 0, 1)));
    assert_eq!(l.parked.len(), 1);
    let n = l.frames.len();
    let id = l.parked.remove(0).id;
    l.s.action_failed(id);
    l.pump(1400);
    assert_eq!(l.s.pending_replies(), 0);
    assert_eq!(l.frames.len(), n, "a dropped reply changes nothing");
    // a late reply to an unknown id is ignored
    l.s.action_reply(id, serde_json::json!({ "c": "late" }));
    l.pump(1500);
    assert_eq!(l.frames.len(), n);

    // a navigation button (`go.thtml`) is not an action: no payload leaves the page
    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "go.thtml");
    l.step(1600, Some(pointer_msg(x, y, 0, 0)));
    l.step(1640, Some(pointer_msg(x, y, 0, 1)));
    assert!(l.parked.is_empty());
    assert_eq!(l.s.current_page().as_deref(), Some("go.thtml"));
}

#[test]
fn without_an_app_server_nothing_is_parked_and_builtins_still_work() {
    let mut l = Local::new("app.thtml", 80, 24, false);
    l.pump(1000);
    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "inc:c");
    l.step(1100, Some(pointer_msg(x, y, 0, 0)));
    l.step(1140, Some(pointer_msg(x, y, 0, 1)));
    assert!(l.parked.is_empty());
    assert_eq!(l.s.pending_replies(), 0);
}

#[test]
fn unanswered_replies_are_bounded_and_garbage_input_is_ignored() {
    let mut l = Local::new("app.thtml", 80, 24, true);
    l.pump(1000);
    let (x, y) = button_point(l.s.event_loop(), l.frames.last().unwrap(), "inc:c");
    let mut t = 1100u64;
    for _ in 0..200 {
        t += 80;
        l.step(t, Some(pointer_msg(x, y, 0, 0)));
        t += 40;
        l.step(t, Some(pointer_msg(x, y, 0, 1)));
    }
    assert!(l.s.pending_replies() <= 64, "{}", l.s.pending_replies());
    // truncated, unknown and non-finite messages never panic or change the page
    for m in [vec![], vec![0x01], vec![0x03, 0, 0], vec![0x10, 1], vec![0xEE, 1, 2], pointer_msg(f32::NAN, 1.0, 0, 0), key_msg(0xD800, 0)] {
        l.s.input(&m, (t + 10) as f64);
    }
    l.pump(t + 50);
    assert_eq!(l.s.current_page().as_deref(), Some("app.thtml"));
}

#[test]
fn start_page_must_exist_and_stays_inside_the_index() {
    for bad in ["nope.thtml", "../counter.thtml", "/counter.thtml"] {
        let cfg = LocalConfig { start_page: bad.into(), cols: 80, rows: 24, mobile: false, actions: false, app_token: String::new() };
        assert!(LocalSession::new(&cfg, pages(), 0.0).is_err(), "{bad}");
    }
}

#[test]
fn next_delay_follows_the_core_deadline() {
    let mut l = Local::new("counter.thtml", 80, 24, false);
    l.pump(1000);
    let d = l.s.next_delay_ms(1000.0);
    assert!((0.0..=1000.0).contains(&d), "{d}");
    // a resize pending in the debouncer pulls the deadline in
    l.s.input(&resize_msg(90, 30), 1010.0);
    assert!(l.s.next_delay_ms(1010.0) <= 101.0);
}
