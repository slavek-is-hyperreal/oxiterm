//! OxiUI Phase 6.6: local mode and thin-client mode agree.
//!
//! Local mode = the core event loop in-process (a `MemPageSource`, a `FakeClock`, no server, no
//! transport) whose sink keeps the `DisplayList` it is given. Thin-client mode = what a browser
//! sees of the same frame: `wire::decode(wire::encode_for(list, 3))`. A scripted session (first
//! frame, resize, fractional pointer clicks on shaped buttons, a state change, navigation) checks
//! on every frame that
//!
//! * the list survives the wire bit-exactly (every glyph x/y/advance, run size, line height,
//!   baseline, faces, clusters), and
//! * a thin client that clicks where it drew a glyph hits the node the core laid out there: the
//!   same point goes through the core's own hit test (`pointer_to_layout` + `hit_test_point`) and
//!   through a real `PointerEvent`, which must reach that node's action.
//!
//! Needs only std (no server, no sockets, no files: pages are `include_str!`-ed).

use std::sync::Arc;
use std::time::Duration;

use oxiterm_core::action::FnActionHandler;
use oxiterm_core::clock::{FakeClock, MonoTime};
use oxiterm_core::event_loop::EventLoop;
use oxiterm_core::loop_io::{EventSource, LoopHandles, LoopHost, MemoryQueue};
use oxiterm_core::page_source::{MemPageSource, PageSource};
use oxiterm_core::session_state::{PtyDimensions, SessionState};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::input::{InputEvent, KeyModifiers, MouseAction, MouseButton, PointerInput};
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::render::buffer::CellBuffer;
use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
use oxiterm_renderer::render::wire;
use oxiterm_renderer::FrameSink;
use parking_lot::Mutex;

const SANS_PAGE: &str = include_str!("../../oxiterm-server/benches/pages/sans.thtml");
/// The counter demo as it was before the demo pages went proportional in GUI mode: the monospace
/// page of these tests (frozen copy, see `oxiterm-server/tests/traces/pages/examples_mono`).
const COUNTER_PAGE: &str = include_str!("../../oxiterm-server/tests/traces/pages/examples_mono/demos/counter.thtml");
/// The live demo page: proportional in GUI mode.
const DEMO_COUNTER_PAGE: &str = include_str!("../../examples/demos/counter.thtml");
/// A proportional page with a state binding: the counter text is re-shaped when the action's
/// reply patches `c`.
const STATE_PAGE: &str = r#"<style>
.p { font-family: sans-serif; wrap: word; width: 36; fg: #e2e8f0; bg: #102030 }
.b { font-family: sans-serif; font-size: 20px; padding-left: 2; padding-right: 2; border-style: single; margin-top: 1 }
</style>
<box style="flex-direction: column; width: 44; padding: 1;">
<text class="p" bind-state="c">zero</text>
<button class="b" event-htmx="inc:c">Increment the counter</button>
<button class="b" event-htmx="reset:c">Reset</button>
</box>"#;

struct Host;
impl LoopHost for Host {
    fn touch_activity(&self) {}
    fn idle_for(&self) -> Duration {
        Duration::ZERO
    }
    fn loop_exited(&self) {}
}

/// Local-mode sink: keeps every list the core produced.
struct Capture(Arc<Mutex<Vec<DisplayList>>>);
impl FrameSink for Capture {
    fn send_frame(&mut self, _f: &CellBuffer, _b: &CellBuffer) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn send_display_list(&mut self, l: &DisplayList) -> anyhow::Result<bool> {
        self.0.lock().push(l.clone());
        Ok(true)
    }
}

struct Local {
    el: EventLoop,
    session: Arc<SessionState>,
    queue: MemoryQueue,
    frames: Arc<Mutex<Vec<DisplayList>>>,
    actions: Arc<Mutex<Vec<String>>>,
    now: MonoTime,
}

impl Local {
    fn new(first: &str) -> Self {
        let pages = Arc::new(MemPageSource::new([
            ("state.thtml", STATE_PAGE),
            ("sans.thtml", SANS_PAGE),
            ("counter.thtml", COUNTER_PAGE),
            ("demo_counter.thtml", DEMO_COUNTER_PAGE),
        ]));
        let clock = Arc::new(FakeClock::new());
        let session = Arc::new(SessionState::new(0, clock.clone()));
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let on = std::sync::atomic::Ordering::SeqCst;
        session.display_list_client.store(true, on);
        session.display_list_v3.store(true, on);
        session.is_web_client.store(true, on);

        let page = pages.page_at(std::path::Path::new(first));
        let doc = pages.load(&page, RenderMode::Gui).expect("first page loads");
        let queue = MemoryQueue::new(128);
        let handles = LoopHandles {
            state: session.clone(),
            events_in: Arc::new(Mutex::new(queue.clone())),
            events_out: Arc::new(queue.clone()),
            host: Arc::new(Host),
        };
        let frames = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::new(Mutex::new(Vec::new()));
        let mut el = EventLoop::new(handles, Arc::new(oxiterm_core::events::EventBus::new()), Box::new(Capture(frames.clone())), pages, doc);
        el.clock = clock;
        el.source_path = Some(first.into());
        el.base_source_path = Some(first.into());
        // The in-process "App Server": counts clicks of `inc:c` and answers with the new value.
        let log = actions.clone();
        let count = std::sync::atomic::AtomicU32::new(0);
        el.action_handler = Some(Arc::new(FnActionHandler(move |p: &oxiterm_core::action::DispatchPayload| {
            log.lock().push(p.action.clone());
            match p.action.as_str() {
                "inc:c" => {
                    let n = count.fetch_add(1, on) + 1;
                    Some(serde_json::json!({ "c": format!("Clicked {n} times: the quick brown fox jumps over the lazy dog") }))
                }
                "reset:c" => Some(serde_json::json!({ "c": "zero" })),
                _ => None,
            }
        })));
        el.prepare_session();
        Self { el, session, queue, frames, actions, now: MonoTime::from_millis(1000) }
    }

    /// Advances the fake time by `ms`, feeds `event`, drains replies the action handler queued
    /// (the step API does not drain the queue) and renders.
    fn step(&mut self, ms: u64, event: Option<InputEvent>) {
        self.now = self.now + Duration::from_millis(ms);
        if let Some(e) = event {
            self.el.handle_event(e, self.now);
        }
        let mut q = self.queue.clone();
        while let Some(e) = q.try_recv() {
            self.el.handle_event(e, self.now);
        }
        self.el.tick(self.now);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.now = self.now + Duration::from_millis(200);
        self.session.resize_debouncer.write().push(PtyDimensions { cols, rows }, self.now);
        self.el.handle_event(InputEvent::Resize { cols, rows }, self.now);
        self.el.tick(self.now);
        // past the debounce window: the new size applies and a frame at that size is produced
        self.step(101, None);
    }

    fn pointer(&mut self, x: f32, y: f32, action: MouseAction) {
        let p = PointerInput { x, y, button: MouseButton::Left, action, modifiers: KeyModifiers::default() };
        self.step(40, Some(InputEvent::PointerEvent(p)));
    }

    fn click(&mut self, x: f32, y: f32) {
        self.pointer(x, y, MouseAction::Press);
        self.pointer(x, y, MouseAction::Release);
    }

    fn last(&self) -> DisplayList {
        self.frames.lock().last().expect("a frame was produced").clone()
    }
}

fn shaped_items(l: &DisplayList) -> Vec<&DisplayItem> {
    l.items.iter().filter(|i| matches!(i, DisplayItem::Text { shaped: Some(_), .. })).collect()
}

/// Compares two lists with bits for every float of every shaped run (plain `==` would take
/// `-0.0 == 0.0` and not look at NaN payloads).
fn assert_bit_identical(a: &DisplayList, b: &DisplayList, what: &str) {
    assert_eq!(a, b, "{what}: lists differ");
    assert_eq!(a.items.len(), b.items.len());
    for (x, y) in a.items.iter().zip(&b.items) {
        let (
            DisplayItem::Text { origin: oa, glyphs: ga, shaped: sa, clip: ca, .. },
            DisplayItem::Text { origin: ob, glyphs: gb, shaped: sb, clip: cb, .. },
        ) = (x, y)
        else {
            continue;
        };
        assert_eq!((oa.x.to_bits(), oa.y.to_bits()), (ob.x.to_bits(), ob.y.to_bits()), "{what}: origin");
        assert_eq!(
            [ca.x, ca.y, ca.w, ca.h].map(f32::to_bits),
            [cb.x, cb.y, cb.w, cb.h].map(f32::to_bits),
            "{what}: clip"
        );
        assert_eq!(ga.len(), gb.len());
        for (p, q) in ga.iter().zip(gb) {
            assert_eq!(p.ch, q.ch);
            assert_eq!((p.x.to_bits(), p.y.to_bits(), p.advance.to_bits()), (q.x.to_bits(), q.y.to_bits(), q.advance.to_bits()), "{what}: glyph {:?}", p.ch);
        }
        match (sa, sb) {
            (Some(p), Some(q)) => {
                assert_eq!(p.face, q.face);
                assert_eq!(
                    [p.size_px, p.line_height, p.baseline].map(f32::to_bits),
                    [q.size_px, q.line_height, q.baseline].map(f32::to_bits),
                    "{what}: run metrics"
                );
                assert_eq!(p.glyph_faces, q.glyph_faces);
                assert_eq!(p.clusters, q.clusters);
            }
            (None, None) => {}
            _ => panic!("{what}: shaped run lost or gained"),
        }
    }
}

/// What the thin client gets for `list`; checks the bytes are stable too.
fn thin_view(list: &DisplayList, what: &str) -> DisplayList {
    let has_runs = !shaped_items(list).is_empty();
    let bytes = wire::encode_for(list, 3);
    // v3 only when a shaped run exists (mono pages stay byte-identical for every client).
    assert_eq!(bytes[0] == 3, has_runs, "{what}: wire version");
    let decoded = wire::decode(&bytes).expect("thin client decodes");
    assert_bit_identical(list, &decoded, what);
    assert_eq!(wire::encode_for(&decoded, 3), bytes, "{what}: re-encoding the decoded list reproduces the bytes");
    decoded
}

/// The nearest ancestor-or-self of `id` with an `event-htmx`, as the loop's activation does.
fn action_of(el: &EventLoop, mut id: NodeId) -> Option<String> {
    loop {
        if let Some(a) = el.doc.get_node(id)?.attrs.event_htmx.clone() {
            return Some(a);
        }
        id = *el.parent_map.get(&id)?;
    }
}

/// Glyph centres (screen cells) of a decoded shaped run.
fn centres(item: &DisplayItem) -> Vec<(f32, f32)> {
    let DisplayItem::Text { origin, glyphs, shaped: Some(run), .. } = item else { return Vec::new() };
    glyphs
        .iter()
        .filter(|g| g.ch != ' ')
        .map(|g| (origin.x + g.x + g.advance / 2.0, origin.y + g.y + run.line_height / 2.0))
        .collect()
}

/// On the decoded (thin client) list: every glyph centre is hit-tested by the core and lands on
/// the node of the item that drew it, or on something painted above it. Returns how many
/// glyphs were checked and how many landed on the drawing node itself.
fn check_glyph_hits(el: &EventLoop, decoded: &DisplayList) -> (usize, usize) {
    glyph_hit_nodes(el, decoded, true).0
}

/// Hit node of every glyph centre of `list`, in item order (the sequence a thin client and the
/// core must agree on), plus the checked/own counts.
fn glyph_hit_nodes(el: &EventLoop, decoded: &DisplayList, assert_related: bool) -> ((usize, usize), Vec<NodeId>) {
    let mut nodes = Vec::new();
    let dims = *el.session.dims.read();
    let layout = el.layout_engine.last_layout.as_ref().expect("layout");
    let (mut checked, mut own) = (0, 0);
    for item in shaped_items(decoded) {
        let DisplayItem::Text { node, .. } = item else { unreachable!() };
        for (x, y) in centres(item) {
            let (lx, ly) = el.pointer_to_layout(layout, x, y, dims);
            let hit = el.layout_engine.hit_test_point(lx, ly).expect("a drawn glyph is over some node");
            checked += 1;
            nodes.push(hit);
            if !assert_related {
                continue;
            }
            if hit == *node {
                own += 1;
            } else {
                // A child or a sibling painted on top (e.g. text inside the button's box): then
                // the hit node must still belong to the same interactive subtree or be a
                // descendant of the drawing node.
                let mut up = hit;
                let mut found = false;
                while let Some(&p) = el.parent_map.get(&up) {
                    if p == *node {
                        found = true;
                        break;
                    }
                    up = p;
                }
                assert!(found, "glyph at ({x},{y}) drawn by node {node:?} hit unrelated node {hit:?}");
            }
        }
    }
    ((checked, own), nodes)
}

/// The centre of the first glyph run whose interactive owner has action `want`.
fn button_point(el: &EventLoop, decoded: &DisplayList, want: &str) -> (f32, f32) {
    for item in shaped_items(decoded) {
        let DisplayItem::Text { node, .. } = item else { unreachable!() };
        if action_of(el, *node).as_deref() == Some(want) {
            let c = centres(item);
            return c[c.len() / 2];
        }
    }
    panic!("no shaped text of a button with action {want}");
}

#[test]
fn local_and_thin_client_agree_over_a_scripted_session() {
    let mut s = Local::new("state.thtml");

    // 1. first frame
    s.step(0, None);
    assert_eq!(s.frames.lock().len(), 1);
    let first = s.last();
    assert!(shaped_items(&first).len() >= 3, "the page is proportional: {} shaped runs", shaped_items(&first).len());
    let d = thin_view(&first, "first frame");
    // the hit test on the geometry the thin client got equals the one on the core's own list
    assert_eq!(glyph_hit_nodes(&s.el, &first, false).1, glyph_hit_nodes(&s.el, &d, false).1);
    let (n, own) = check_glyph_hits(&s.el, &d);
    assert!(n > 20 && own > 0, "{n} glyphs checked, {own} on their own node");

    // 2. resize wider and taller than the page: a nonzero centering offset moves every glyph
    s.resize(100, 30);
    { let d = *s.session.dims.read(); assert_eq!((d.cols, d.rows), (100, 30)); }
    let resized = s.last();
    assert_ne!(resized, first, "the resized frame is a different picture");
    let d = thin_view(&resized, "resized frame");
    assert_eq!(glyph_hit_nodes(&s.el, &resized, false).1, glyph_hit_nodes(&s.el, &d, false).1);
    let (n, own) = check_glyph_hits(&s.el, &d);
    assert!(n > 20 && own > 0);

    // 3. click the first button where the thin client drew it
    let before = s.frames.lock().len();
    let (x, y) = button_point(&s.el, &d, "inc:c");
    s.click(x, y);
    assert_eq!(*s.actions.lock(), vec!["inc:c".to_string()], "the click reached the button the glyph belongs to");

    // 4. the reply patched the state: the bound text re-shapes, both views agree
    assert!(s.frames.lock().len() > before, "the state change produced a frame");
    let changed = s.last();
    assert_ne!(changed, resized);
    let glyph_text = |l: &DisplayList| -> Vec<String> {
        shaped_items(l)
            .into_iter()
            .map(|i| match i {
                DisplayItem::Text { glyphs, .. } => glyphs.iter().map(|g| g.ch).collect(),
                _ => unreachable!(),
            })
            .collect()
    };
    assert!(glyph_text(&changed).iter().any(|t| t.starts_with("Clicked")), "{:?}", glyph_text(&changed));
    let d = thin_view(&changed, "after the state change");
    check_glyph_hits(&s.el, &d);

    // 5. the second button, found in the new geometry (the text above grew, so it moved down)
    let (x, y) = button_point(&s.el, &d, "reset:c");
    s.click(x, y);
    assert_eq!(s.actions.lock().last().map(String::as_str), Some("reset:c"));
    let d = thin_view(&s.last(), "after reset");
    assert!(glyph_text(&s.last()).iter().any(|t| t == "zero"));
    check_glyph_hits(&s.el, &d);

    // every frame of the session, not just the checked ones
    for (i, l) in s.frames.lock().iter().enumerate() {
        thin_view(l, &format!("frame {i}"));
    }
}

#[test]
fn local_and_thin_client_agree_on_the_sans_fixture_and_a_mono_page() {
    let mut s = Local::new("sans.thtml");
    s.step(0, None);
    let sans = s.last();
    assert!(shaped_items(&sans).len() > 5);
    let d = thin_view(&sans, "sans fixture");
    let (n, own) = check_glyph_hits(&s.el, &d);
    assert!(n > 100 && own > 50, "{n} {own}");
    // a real click on a proportional button of the fixture
    let (x, y) = button_point(&s.el, &d, "dec:counter");
    s.click(x, y);
    assert_eq!(s.actions.lock().last().map(String::as_str), Some("dec:counter"));
    s.resize(120, 40);
    let d = thin_view(&s.last(), "sans fixture, 120x40");
    check_glyph_hits(&s.el, &d);

    // an ordinary monospace page: no shaped run, wire stays v2 for every client, still equal
    s.step(2000, Some(InputEvent::NavigateTo("counter.thtml".into())));
    let mono = s.last();
    assert!(shaped_items(&mono).is_empty(), "the counter demo is monospace");
    assert!(!mono.items.is_empty());
    let bytes = wire::encode_for(&mono, 3);
    assert_eq!(bytes[0], 2);
    assert_eq!(bytes, wire::encode_for(&mono, 2));
    thin_view(&mono, "counter page");
}

/// The shipped counter demo is proportional in GUI mode (Phase 6 closing step): shaped runs, wire
/// v3, local and thin-client frames equal, glyph centres hit the node that drew them, and the
/// proportional buttons activate their actions.
#[test]
fn local_and_thin_client_agree_on_the_proportional_counter_demo() {
    let mut s = Local::new("demo_counter.thtml");
    s.step(0, None);
    let first = s.last();
    assert!(!shaped_items(&first).is_empty(), "the counter demo is proportional in GUI mode");
    assert_eq!(wire::encode_for(&first, 3)[0], 3);
    let d = thin_view(&first, "counter demo");
    let (n, own) = check_glyph_hits(&s.el, &d);
    assert!(n > 50 && own > 20, "{n} {own}");
    let (x, y) = button_point(&s.el, &d, "inc:counter");
    s.click(x, y);
    assert_eq!(s.actions.lock().last().map(String::as_str), Some("inc:counter"));
    s.resize(120, 40);
    let d = thin_view(&s.last(), "counter demo, 120x40");
    check_glyph_hits(&s.el, &d);
}
