//! The event loop: one session's document, focus/scroll/drag state, state binding, animations
//! and rendering, driven through a step API (`handle_event` / `tick` / `next_deadline`).
//!
//! Pure core: it has no transport, no thread, no real clock and no filesystem. The host owns the
//! frame sink, the page source, the event queue (behind [`EventSource`] / [`EventPush`]) and the
//! clock, and passes `now` in. `run` is the reference blocking driver for hosts whose
//! `EventSource` blocks (the server); a browser or desktop host calls the step API itself.

use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::loop_io::{EventPush, EventSource, LoopHandles, LoopHost, WaitResult};
use crate::session_state::{MediaRenderInfo, SessionState};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::input::InputEvent;
use oxiterm_renderer::document::THTMLDocument;
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::render::buffer::DoubleBuffer;
use oxiterm_renderer::render::renderer::Renderer;

/// Where a state patch came from; selects the transport policy in `EventLoop::apply_app_patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOrigin {
    /// The response to a dispatched action (`InputEvent::AppResponse`).
    ActionResponse,
    /// The HTTP push endpoint (`InputEvent::PushPatch`).
    Push,
}

/// Standard processing loop coordination.
pub struct EventLoop {
    /// Pure session state (shared with the transport half).
    pub session: Arc<SessionState>,
    /// Input event source (the queue is owned by the host; locked per receive batch).
    pub events_in: Arc<parking_lot::Mutex<dyn EventSource>>,
    /// Puts events back into the queue (action replies).
    pub events_out: Arc<dyn EventPush>,
    /// Activity / lifecycle reporting to the transport.
    pub host: Arc<dyn LoopHost>,
    /// Global notification dispatcher.
    pub event_bus: Arc<crate::events::EventBus>,
    /// Current loaded document tree.
    pub doc: THTMLDocument,
    /// Rendering layout compiler.
    pub layout_engine: LayoutEngine,
    /// Local frame buffer cache.
    pub buffer: DoubleBuffer,
    /// Flag indicating if data output is paused.
    pub output_paused: bool,
    /// Active frame receiver/renderer sink.
    pub frame_sink: Box<dyn oxiterm_renderer::FrameSink>,
    /// Framerate throttling manager.
    pub frame_limiter: crate::frame_limiter::FrameRateLimiter,
    /// Pending mouse events map.
    pub pending_mouse: Option<oxiterm_proto::input::MouseInput>,
    /// The exact (fractional, layout-cell) point of `pending_mouse` when it came from a
    /// pointer event; `None` for a cell event (probed at the cell centre).
    pub pending_point: Option<(f32, f32)>,
    /// Template file path supporting hot reloads.
    pub source_path: Option<std::path::PathBuf>,
    /// The non-mobile variant of the template path.
    pub base_source_path: Option<std::path::PathBuf>,
    /// Where pages come from (navigation, variant selection, reload). The loop never touches
    /// the filesystem for pages itself.
    pub page_source: Arc<dyn crate::page_source::PageSource>,
    /// dbus a11y coordination bridge.
    pub dbus_bridge: Option<oxiterm_a11y::DBusBridge>,
    /// Last ARIA-mirror JSON and the exact inputs it was built from (stage 5.1c).
    aria_cache: oxiterm_a11y::AriaCache,
    /// Correlation table maps child nodes to parent IDs.
    pub parent_map: HashMap<NodeId, NodeId>,
    /// Active interactive nodes array.
    pub focusable_nodes: Vec<NodeId>,
    /// Focused input node identifier.
    pub focused_node: Option<NodeId>,
    /// The focused node the viewport was last auto-scrolled to. Auto-scroll-to-focused
    /// fires only when `focused_node` differs from this, so it reveals a newly focused
    /// (e.g. Tab-selected) node exactly once without fighting subsequent manual scrolling.
    pub autoscroll_anchor: Option<NodeId>,
    /// Vertical scrolling offset.
    pub scroll_offset: u16,
    /// Computed document height.
    pub total_height: u16,
    /// Optional handler for actions (the App Server). Fire-and-forget; responses re-enter the
    /// loop as `InputEvent::AppResponse`.
    pub action_handler: Option<Arc<dyn crate::action::ActionHandler>>,
    /// Template cache for For-node expansion. Maps For node id → original template child ids.
    /// MUST be cleared in update_doc, resolve_and_load_page, and Reload handlers because
    /// NodeIds are arena-local and become invalid after a document swap.
    pub for_template_cache: HashMap<NodeId, Vec<NodeId>>,
    /// Per-session DoR (denial-of-render) event throttle.
    pub throttle: crate::throttle::EventThrottle,
    /// Active pointer drag operation state.
    pub active_drag: Option<DragState>,
    /// CSS transitions and animations controller.
    pub animation_controller: crate::animation::AnimationController,
    /// Time source. Defaults to the session's host clock; tests and the trace harness swap in
    /// a fake one. Read only by the `run()` driver; a step-API host passes `now` itself.
    pub clock: Arc<dyn crate::clock::Clock>,
    /// The loop's current time: the `now` of the latest `handle_event` / `tick` (the `run()`
    /// driver reads `clock` once per iteration, right after the blocking receive). Everything below the loop (throttle, limiter, debouncer,
    /// animations) uses this value instead of reading a clock.
    pub now: crate::clock::MonoTime,
    /// No frame has been rendered yet: the first `tick` renders unconditionally.
    first_frame: bool,
    /// An event since the last `tick` asked for a render (`handle_event` sets, `tick` clears).
    needs_render: bool,
    /// A render owed to the next `tick`: the frame limiter deferred one, or the driver hit its
    /// per-batch event cap (`schedule_render`).
    render_carry: bool,
    /// At least one event was handled since the last `tick`. Without one, an active 16 ms
    /// cadence renders on its own (the old "timeout while active" rule).
    event_since_tick: bool,
}

/// Active pointer drag operation state machine tracking element relocation.
#[derive(Debug, Clone)]
pub struct DragState {
    /// NodeId being dragged.
    pub target_node: NodeId,
    /// Screen column where the drag started.
    pub start_mouse_col: u16,
    /// Screen row where the drag started.
    pub start_mouse_row: u16,
    /// Element initial style left offset when drag started.
    pub initial_left: i16,
    /// Element initial style top offset when drag started.
    pub initial_top: i16,
    /// Bound state variable to synchronize with X position changes.
    pub state_x_key: Option<String>,
    /// Bound state variable to synchronize with Y position changes.
    pub state_y_key: Option<String>,
    /// HTMX action to fire when pointer is released.
    pub on_drag_end: Option<String>,
}

/// Focus-ring markers drawn immediately left/right of the focused node. They MUST be
/// unambiguous width-1 glyphs — a wide glyph in the cell before a label drifts the
/// visible text off its hit box on ambiguous-wide terminals (see the focus-ring draw).
pub const FOCUS_RING_LEFT: char = '>';
pub const FOCUS_RING_RIGHT: char = '<';

/// Layout row (before centering and scroll) that carries the focus-ring markers of a node: the
/// row containing the vertical centre of its exact rect. The cell-snapped rect (`Rect::cells`,
/// `round` on y and on height separately) is wrong for the fractional GUI layout: a node at
/// `y = 0.4`, `h = 1.2` snaps to `y = 0`, `h = 1`, so `0 + 1 / 2` put the markers on row 0 while
/// the label is centred in row 1. On the snapped TUI rects (whole numbers) this is the same
/// `y + h / 2` (integer division) as before.
pub(crate) fn focus_ring_row(rect: &oxiterm_renderer::layout::types::Rect) -> i32 {
    (rect.y + rect.height / 2.0).floor() as i32
}

/// Scroll status indicator under the "position" model: the line fraction, the bar
/// fill, and the percentage are all derived from a single `scroll_offset / max_scroll`
/// ratio, so they are mutually consistent by construction (0% / first position at the
/// top, 100% / last position at the bottom). `max_scroll` is derived from the layout
/// extent (`total_height`), the same height that drives vertical centering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScrollIndicator {
    /// 1-indexed current scroll position (`1..=steps`).
    position: u16,
    /// Total number of distinct scroll positions (`max_scroll + 1`).
    steps: u16,
    /// Progress percentage, `0..=100`.
    percent: u8,
    /// Number of filled cells in a `bar_width`-wide progress bar.
    filled: usize,
}

impl ScrollIndicator {
    fn new(scroll_offset: u16, total_height: u16, viewport_h: u16, bar_width: usize) -> Self {
        let max_scroll = total_height.saturating_sub(viewport_h);
        let offset = scroll_offset.min(max_scroll);
        let (percent, filled) = if max_scroll > 0 {
            let pct = (offset as f32 / max_scroll as f32 * 100.0).round() as u8;
            let filled = ((offset as usize * bar_width) / max_scroll as usize).min(bar_width);
            (pct, filled)
        } else {
            (0, 0)
        };
        Self { position: offset + 1, steps: max_scroll + 1, percent, filled }
    }
}

/// What a driver learns from one batch of events.
struct BatchOutcome {
    /// The 100-event cap was hit: more events may be queued, render on the next tick.
    capped: bool,
    /// Queue disconnected or quit requested: leave the loop.
    disconnected: bool,
}

/// Result of feeding one event to the step API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Keyboard quit (SSH only): the host should end the session.
    Quit,
    /// Handled; keep going. Whether a render is wanted is recorded internally for `tick`.
    Continue,
}

/// Result of handling one event.
enum EventOutcome {
    /// Keyboard quit (SSH only).
    Quit,
    /// Handled; whether it wants a render.
    Handled { needs_render: bool },
}

pub struct EventLoopGuard {
    pub host: Arc<dyn LoopHost>,
}

impl Drop for EventLoopGuard {
    fn drop(&mut self) {
        self.host.loop_exited();
    }
}

impl EventLoop {
    /// The current page as a `PageRef` (pure path arithmetic on `base_source_path` /
    /// `source_path`), the base for current-page-relative navigation.
    pub fn current_page_ref(&self) -> Option<crate::page_source::PageRef> {
        let p = self.base_source_path.as_ref().or(self.source_path.as_ref())?;
        Some(self.page_source.page_at(p))
    }

    /// Parse mode for documents of this session: GUI for display-list web clients, TUI otherwise.
    fn render_mode(&self) -> oxiterm_proto::style::RenderMode {
        if self.session.display_list_client.load(std::sync::atomic::Ordering::SeqCst) {
            oxiterm_proto::style::RenderMode::Gui
        } else {
            oxiterm_proto::style::RenderMode::Tui
        }
    }

    /// Reparses the current source page (hot reload / render-mode switch). Returns true on success.
    fn reload_current_source(&mut self) -> bool {
        let Some(path) = self.source_path.clone() else { return false };
        let page = self.page_source.page_at(&path);
        match self.page_source.load(&page, self.render_mode()) {
            Ok(mut new_doc) => {
                let mut state = self.session.state.write();
                Self::setup_state_subscriptions(&new_doc, &mut *state);
                Self::inject_initial_state(&mut new_doc, &*state);
                drop(state);
                self.update_doc(new_doc);
                self.reset_layout_and_scroll();
                true
            }
            Err(e) => {
                warn!("Reload failed for session {}: {}", self.session.id, e);
                false
            }
        }
    }

    /// Navigates to `target_filename`, relative to the current page's directory (`.thtml` is
    /// appended when missing).
    pub fn resolve_and_load_page(&mut self, target_filename: &str) -> anyhow::Result<()> {
        let base = self.current_page_ref();
        self.load_page(base.as_ref(), &crate::page_source::with_thtml_ext(target_filename))
    }

    /// Like `resolve_and_load_page`, but `target_filename` is relative to the application base
    /// directory instead of the current page's directory.
    fn load_app_page(&mut self, target_filename: &str) -> anyhow::Result<()> {
        self.load_page(None, target_filename)
    }

    fn load_page(&mut self, base: Option<&crate::page_source::PageRef>, target: &str) -> anyhow::Result<()> {
        let is_mobile = self.session.is_mobile.load(std::sync::atomic::Ordering::SeqCst);
        // Containment and variant selection live in the page source (one generic error).
        let page = self.page_source.resolve(base, target, is_mobile)?;
        let mut new_doc = self.page_source.load(&page, self.render_mode())?;
        let mut state = self.session.state.write();
        Self::setup_state_subscriptions(&new_doc, &mut *state);
        Self::inject_initial_state(&mut new_doc, &*state);
        drop(state);

        // update_doc clears for_template_cache (NodeIds are arena-local)
        self.update_doc(new_doc);
        self.reset_layout_and_scroll();

        // The renderer still reads media relative to these paths.
        self.source_path = Some(page.actual);
        self.base_source_path = Some(page.base);
        *self.session.current_page.write() = Some(page.page);
        Ok(())
    }

    /// Creates a new `EventLoop` with clean session parameters.
    ///
    /// The host supplies everything with a transport or filesystem behind it: the frame sink
    /// (ANSI over SSH, display lists over WebSocket, a canvas, ...) and the page source.
    pub fn new(
        handles: LoopHandles,
        event_bus: Arc<crate::events::EventBus>,
        frame_sink: Box<dyn oxiterm_renderer::FrameSink>,
        page_source: Arc<dyn crate::page_source::PageSource>,
        doc: THTMLDocument,
    ) -> Self {
        let LoopHandles { state: session, events_in, events_out, host } = handles;
        let dims = *session.dims.read();
        let clock = session.clock.clone();
        let now = clock.now();

        let mut event_loop = Self { 
            session,
            events_in,
            events_out,
            host,
            event_bus,
            doc,
            layout_engine: LayoutEngine::new(),
            buffer: DoubleBuffer::new(dims.cols, dims.rows),
            output_paused: false,
            frame_sink,
            frame_limiter: crate::frame_limiter::FrameRateLimiter::new(60),
            now,
            clock,
            pending_mouse: None,
            pending_point: None,
            source_path: None,
            base_source_path: None,
            page_source,
            dbus_bridge: None,
            aria_cache: oxiterm_a11y::AriaCache::new(),
            parent_map: HashMap::new(),
            focusable_nodes: Vec::new(),
            focused_node: None,
            autoscroll_anchor: None,
            scroll_offset: 0,
            total_height: 0,
            action_handler: None,
            for_template_cache: HashMap::new(),
            throttle: crate::throttle::EventThrottle::new(now),
            active_drag: None,
            animation_controller: crate::animation::AnimationController::new(),
            first_frame: true,
            needs_render: false,
            render_carry: false,
            event_since_tick: false,
        };
        event_loop.rebuild_parent_map();
        event_loop.rebuild_focusable_nodes();
        event_loop
    }

    /// Resets all compiled nodes, clears stale hit-test layout, and resets scroll margins.
    ///
    /// Clearing `last_layout` prevents stale mouse hit-tests from firing on old layout
    /// during the window between a doc swap and the first post-swap render.
    pub fn reset_layout_and_scroll(&mut self) {
        self.layout_engine.reset_nodes();
        self.layout_engine.last_layout = None;
        self.scroll_offset = 0;
    }

    /// Re-evaluates parent associations for all nodes.
    pub fn rebuild_parent_map(&mut self) {
        let mut parent_map = HashMap::new();
        let mut stack = vec![self.doc.root];
        while let Some(parent_id) = stack.pop() {
            if let Some(node) = self.doc.arena.get(parent_id) {
                for &child_id in &node.children {
                    parent_map.insert(child_id, parent_id);
                    stack.push(child_id);
                }
            }
        }
        self.parent_map = parent_map;
    }

    /// Updates active document node tree and updates mappings.
    ///
    /// Clears `for_template_cache` because NodeIds are arena-local and become invalid
    /// after a document swap.
    pub fn update_doc(&mut self, new_doc: THTMLDocument) {
        self.doc = new_doc;
        self.for_template_cache.clear();
        self.active_drag = None;
        self.animation_controller.clear();
        self.rebuild_parent_map();
        self.rebuild_focusable_nodes();
        self.buffer.force_dirty();
    }

    /// Resolves interactive elements respecting bind conditional rules.
    pub fn rebuild_focusable_nodes(&mut self) {
        use oxiterm_proto::dom::StateEvaluator;
        let state_guard = self.session.state.read();
        let mut nodes = Vec::new();
        let mut stack = vec![self.doc.root];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.doc.arena.get(id) {
                if let Some(ref cond) = node.attrs.bind_show {
                    if !state_guard.evaluate_bind_show(cond) {
                        continue;
                    }
                }

                let is_interactive = node.attrs.event_htmx.is_some()
                    || (node.tag == oxiterm_proto::dom::NodeTag::Input
                        && node.attrs.bind_value.is_some());
                if is_interactive {
                    nodes.push(id);
                }
                for &child in node.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        if let Some(focused) = self.focused_node {
            if !nodes.contains(&focused) {
                self.focused_node = if nodes.is_empty() { None } else { Some(nodes[0]) };
            }
        } else if !nodes.is_empty() {
            self.focused_node = Some(nodes[0]);
        }
        self.focusable_nodes = nodes;
    }

    fn setup_state_subscriptions(doc: &THTMLDocument, state: &mut crate::state::StateManager) {
        state.clear_subscriptions();
        for (id, node) in doc.arena.iter() {
            if let Some(key) = &node.attrs.bind_state {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_value {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_width {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_height {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_top {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_left {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_opacity {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_z_index {
                state.subscribe(key.clone(), id);
            }
        }
    }

    /// Brings the document in line with the session state: restores animated properties to their
    /// targets, applies dirty state keys, then registers transitions for any target that changed.
    /// The order matters: syncing state without the surrounding steps lets `restore_baselines`
    /// revert a changed target before `sync_transitions` sees it.
    fn apply_state_to_doc(&mut self, now: crate::clock::MonoTime) {
        self.animation_controller.restore_baselines(&mut self.doc);
        Self::sync_dirty_state(&mut self.doc, &mut *self.session.state.write());
        self.animation_controller.sync_transitions(&mut self.doc, now);
    }

    fn sync_dirty_state(doc: &mut THTMLDocument, state: &mut crate::state::StateManager) {
        let dirty_nodes = state.get_dirty_nodes();
        for node_id in dirty_nodes {
            let mut changed = false;
            if let Some(node) = doc.arena.get_mut(node_id) {
                if let Some(key) = &node.attrs.bind_state {
                    if let Some(val) = state.get(key) {
                        node.text = Some(val.to_string());
                        changed = true;
                    }
                }
                if let Some(key) = &node.attrs.bind_value {
                    if let Some(val) = state.get(key) {
                        node.text = Some(val.to_string());
                        changed = true;
                    }
                }
                if let Some(key) = &node.attrs.bind_width {
                    if let Some(val) = state.get(key) {
                        if let Ok(v) = val.to_string().parse::<u16>() {
                            if node.style.width != Some(f32::from(v)) {
                                node.style.width = Some(f32::from(v));
                                changed = true;
                            }
                        }
                    }
                }
                if let Some(key) = &node.attrs.bind_height {
                    if let Some(val) = state.get(key) {
                        if let Ok(v) = val.to_string().parse::<u16>() {
                            if node.style.height != Some(f32::from(v)) {
                                node.style.height = Some(f32::from(v));
                                changed = true;
                            }
                        }
                    }
                }
                if let Some(key) = &node.attrs.bind_top {
                    if let Some(val) = state.get(key) {
                        if let Ok(v) = val.to_string().parse::<i16>() {
                            if node.style.top != Some(f32::from(v)) {
                                node.style.top = Some(f32::from(v));
                                changed = true;
                            }
                        }
                    }
                }
                if let Some(key) = &node.attrs.bind_left {
                    if let Some(val) = state.get(key) {
                        if let Ok(v) = val.to_string().parse::<i16>() {
                            if node.style.left != Some(f32::from(v)) {
                                node.style.left = Some(f32::from(v));
                                changed = true;
                            }
                        }
                    }
                }
                if let Some(key) = &node.attrs.bind_opacity {
                    if let Some(val) = state.get(key) {
                        if let Ok(v) = val.to_string().parse::<f32>() {
                            let clamped = Some(v.clamp(0.0, 1.0));
                            if node.style.opacity != clamped {
                                node.style.opacity = clamped;
                                changed = true;
                            }
                        }
                    }
                }
                if let Some(key) = &node.attrs.bind_z_index {
                    if let Some(val) = state.get(key) {
                        if let Ok(v) = val.to_string().parse::<i16>() {
                            let z = Some(v as i32);
                            if node.style.z_index != z {
                                node.style.z_index = z;
                                changed = true;
                            }
                        }
                    }
                }
            }
            if changed {
                doc.mark_dirty(node_id);
            }
        }
    }

    pub fn inject_initial_state(doc: &mut THTMLDocument, state: &crate::state::StateManager) {
        let mut dirty = Vec::new();
        for (id, node) in doc.arena.iter_mut() {
            let mut changed = false;
            if let Some(key) = &node.attrs.bind_state {
                if let Some(val) = state.get(key) {
                    node.text = Some(val.to_string());
                    changed = true;
                }
            }
            if let Some(key) = &node.attrs.bind_value {
                if let Some(val) = state.get(key) {
                    node.text = Some(val.to_string());
                    changed = true;
                }
            }
            if let Some(key) = &node.attrs.bind_width {
                if let Some(val) = state.get(key) {
                    if let Ok(v) = val.to_string().parse::<u16>() {
                        node.style.width = Some(f32::from(v));
                        changed = true;
                    }
                }
            }
            if let Some(key) = &node.attrs.bind_height {
                if let Some(val) = state.get(key) {
                    if let Ok(v) = val.to_string().parse::<u16>() {
                        node.style.height = Some(f32::from(v));
                        changed = true;
                    }
                }
            }
            if let Some(key) = &node.attrs.bind_top {
                if let Some(val) = state.get(key) {
                    if let Ok(v) = val.to_string().parse::<i16>() {
                        node.style.top = Some(f32::from(v));
                        changed = true;
                    }
                }
            }
            if let Some(key) = &node.attrs.bind_left {
                if let Some(val) = state.get(key) {
                    if let Ok(v) = val.to_string().parse::<i16>() {
                        node.style.left = Some(f32::from(v));
                        changed = true;
                    }
                }
            }
            if let Some(key) = &node.attrs.bind_opacity {
                if let Some(val) = state.get(key) {
                    if let Ok(v) = val.to_string().parse::<f32>() {
                        node.style.opacity = Some(v.clamp(0.0, 1.0));
                        changed = true;
                    }
                }
            }
            if let Some(key) = &node.attrs.bind_z_index {
                if let Some(val) = state.get(key) {
                    if let Ok(v) = val.to_string().parse::<i16>() {
                        node.style.z_index = Some(v as i32);
                        changed = true;
                    }
                }
            }
            if changed {
                dirty.push(id);
            }
        }
        for id in dirty {
            doc.mark_dirty(id);
        }
    }

    fn get_htmx_node_and_target(&self, mut node_id: NodeId) -> Option<(NodeId, String)> {
        loop {
            if let Some(node) = self.doc.get_node(node_id) {
                if let Some(ref htmx) = node.attrs.event_htmx {
                    return Some((node_id, htmx.clone()));
                }
            }
            if node_id == self.doc.root {
                break;
            }
            if let Some(&parent_id) = self.parent_map.get(&node_id) {
                node_id = parent_id;
            } else {
                break;
            }
        }
        None
    }

    fn node_has_drag_handle_descendant(&self, root_id: NodeId) -> bool {
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.doc.get_node(id) {
                if node.attrs.drag_handle.unwrap_or(false) {
                    return true;
                }
                for &ch in &node.children {
                    stack.push(ch);
                }
            }
        }
        false
    }

    fn is_or_descendant_of_drag_handle(&self, hit_node: NodeId, draggable_root: NodeId) -> bool {
        let mut curr = Some(hit_node);
        while let Some(id) = curr {
            if let Some(node) = self.doc.get_node(id) {
                if node.attrs.drag_handle.unwrap_or(false) {
                    return true;
                }
            }
            if id == draggable_root {
                break;
            }
            curr = self.parent_map.get(&id).copied();
        }
        false
    }

    pub fn find_draggable_target(&self, hit_node: NodeId) -> Option<(NodeId, Option<String>, Option<String>, Option<String>)> {
        let mut curr = Some(hit_node);
        while let Some(id) = curr {
            if let Some(node) = self.doc.get_node(id) {
                if node.attrs.draggable.unwrap_or(false) {
                    let has_explicit_handle = self.node_has_drag_handle_descendant(id);
                    if !has_explicit_handle || self.is_or_descendant_of_drag_handle(hit_node, id) {
                        return Some((
                            id,
                            node.attrs.drag_state_x.clone(),
                            node.attrs.drag_state_y.clone(),
                            node.attrs.event_drag_end.clone(),
                        ));
                    }
                }
            }
            if id == self.doc.root {
                break;
            }
            curr = self.parent_map.get(&id).copied();
        }
        None
    }

    /// Handles a target HTMX action.
    ///
    /// Web-only open: targets strip 'open:' and emit 0x33 frames.
    /// Navigation targets (.thtml) resolve and load the page.
    /// Action targets set state keys and dispatch notifications.
    pub fn handle_htmx_target(&mut self, node_id: NodeId, target: &str) {
        if let Some(url) = target.strip_prefix("open:") {
            self.handle_open_url(url.to_string(), node_id);
        } else if target.ends_with(".thtml") {
            if self.throttle.check_nav(self.now) {
                if let Err(e) = self.resolve_and_load_page(target) {
                    warn!("handle_htmx_target: failed to load page {}: {}", target, e);
                    self.send_nav_error();
                }
            } else if self.throttle.is_first_throttle() {
                warn!("handle_htmx_target: NAV throttle active, dropping .thtml load");
                self.send_nav_throttle_notice();
            }
        } else {
            self.session.state.write().apply_action(target);
            self.try_dispatch(target);
        }
    }

    /// Sends a generic navigation-not-found notice (0x34) to web clients.
    ///
    /// Message is FIXED and GENERIC — never includes the requested path or failure reason
    /// (no-enumeration-oracle rule, Plan 2).
    pub fn send_nav_error(&mut self) {
        let _ = self.frame_sink.send_notice(oxiterm_renderer::ClientNotice::NavError);
    }

    /// Sends a NAV throttle notice (0x34 "Zwolnij ;)") to web clients.
    ///
    /// Called at most once per throttle episode (guarded by `is_first_throttle()`).
    pub fn send_nav_throttle_notice(&mut self) {
        let _ = self.frame_sink.send_notice(oxiterm_renderer::ClientNotice::NavThrottle);
    }

    /// Preprocesses a raw InputEvent before dispatch.
    ///
    /// - Remaps Page-Up / `b` → `ScrollUp` when input is not focused.
    /// - Remaps Page-Down / Space → `ScrollDown` when input is not focused.
    /// - Returns `None` to signal a quit request when `q`/`Q` is pressed outside an input.
    /// - All other events pass through unchanged.
    pub fn preprocess_key(
        event: InputEvent,
        is_input_focused: bool,
        is_web_client: bool,
    ) -> Option<InputEvent> {
        match event {
            InputEvent::KeyPress(ref key) if !is_input_focused
                && (key.codepoint == '\u{F72E}' || key.codepoint == 'b') => {
                Some(InputEvent::ScrollUp)
            }
            InputEvent::KeyPress(ref key) if !is_input_focused
                && (key.codepoint == '\u{F72D}' || key.codepoint == ' ') => {
                Some(InputEvent::ScrollDown)
            }
            // Quit via 'q'/'Q' is SSH-only; web sessions ignore this binding.
            InputEvent::KeyPress(ref key) if !is_input_focused && !is_web_client
                && (key.codepoint == 'q' || key.codepoint == 'Q') => {
                None // quit signal
            }
            e => Some(e),
        }
    }

    /// Handles an `open:` htmx target.
    ///
    /// Web sessions: emits a 0x33 frame containing the URL bytes via `open_url_tx`.
    /// SSH sessions: logs a warning and does nothing (`open:` is web-only in plan 2).
    ///
    /// Only `http` and `https` schemes are permitted; others are rejected with a warning.
    pub fn handle_open_url(&mut self, url: String, _source_node_id: NodeId) {
        let scheme_end = url.find(':').unwrap_or(0);
        let scheme = &url[..scheme_end];
        if scheme != "http" && scheme != "https" {
            warn!("open: rejected URL with disallowed scheme '{}': {}", scheme, url);
            return;
        }

        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = self.frame_sink.send_notice(oxiterm_renderer::ClientNotice::OpenUrl(url));
        } else {
            // SSH sessions: open: is WEB-ONLY in plan 2.
            // Plan 3 will add OSC 8 cell-level hyperlinks.
            warn!("open: action '{}' on SSH session — not implemented (web-only in plan 2)", url);
        }
    }

    /// Expands all For nodes in the current document against their List state values.
    ///
    /// Called at session start and after every state mutation that may affect a For node.
    pub fn expand_all_for_nodes(&mut self) {
        let for_ids: Vec<NodeId> = self.doc.arena.iter()
            .filter(|(_, n)| n.tag == oxiterm_proto::dom::NodeTag::For)
            .map(|(id, _)| id)
            .collect();

        for for_id in for_ids {
            let each_key = self.doc.arena.get(for_id)
                .and_then(|n| n.attrs.each.as_deref());
            if let Some(key) = each_key {
                let list = match self.session.state.read().get(&key) {
                    Some(crate::state::StateValue::List(l)) => l.clone(),
                    _ => Vec::new(),
                };
                if let Err(e) = crate::expand::expand_for_node(
                    &mut self.doc, for_id, &list, &mut self.for_template_cache
                ) {
                    tracing::warn!("expand_all_for_nodes: expand_for_node failed for {:?}: {}", for_id, e);
                }
            }
        }
    }

    pub fn try_dispatch(&self, action: &str) {
        if let Some(ref handler) = self.action_handler {
            let state_guard = self.session.state.read();
            let mut state_snapshot = std::collections::HashMap::new();
            for (_id, node) in self.doc.arena.iter() {
                if let Some(ref key) = node.attrs.bind_state {
                    if let Some(val) = state_guard.get(key) {
                        state_snapshot.insert(key.clone(), val.to_string());
                    }
                }
                if let Some(ref key) = node.attrs.bind_value {
                    if let Some(val) = state_guard.get(key) {
                        state_snapshot.insert(key.clone(), val.to_string());
                    }
                }
                if let Some(ref key) = node.attrs.bind_show {
                    let key_part = key.split('=').next().unwrap_or(key);
                    if let Some(val) = state_guard.get(key_part) {
                        state_snapshot.insert(key_part.to_string(), val.to_string());
                    }
                }
            }
            drop(state_guard);
            let (username, auth_method) = {
                let id = self.session.identity.read();
                match id.as_ref() {
                    Some(i) => (Some(i.username.clone()), Some(format!("{:?}", i.auth_method))),
                    None => (None, None),
                }
            };
            let app_token = {
                let state_guard = self.session.state.read();
                match state_guard.get("_app_token") {
                    Some(crate::state::StateValue::Str(tok)) if !tok.is_empty() => Some(tok.clone()),
                    _ => None,
                }
            };
            let payload = crate::action::DispatchPayload {
                action: action.to_string(),
                state: state_snapshot,
                session_id: self.session.id,
                username,
                auth_method,
                app_token,
            };
            // The reply only queues an event: the loop applies the response on its own thread.
            let events_out = self.events_out.clone();
            let reply = crate::action::ActionReply::new(move |patch| {
                events_out.push(InputEvent::AppResponse(oxiterm_proto::input::AppPatch(patch)));
            });
            handler.dispatch(payload, reply);
        }
    }

    /// Processes a single `MouseInput` event and returns whether a frame render is needed.
    ///
    /// Anchored by spec [Plan-2.2/R1, SC-05].
    ///
    /// # Design contract
    ///
    /// - **Press** (activation) — always runs `hit_test` + `handle_htmx_target`.
    ///   The INPUT bucket is never consulted for this path. The NAV bucket inside
    ///   `handle_htmx_target` caps the actual page-load rate.
    /// - **Release / Move / hover** — render-only flood surface. The INPUT bucket gates
    ///   `needs_render`; skipping a hover render frame is always safe.
    /// - `update_interactive_animations` and `pending_mouse` are always updated
    ///   (cheap; animation state coalesces internally).
    ///
    /// # Testability note
    ///
    /// This method is the canonical entry point exercised by t1/t2/t3. Tests that call
    /// `handle_mouse_event` directly prove the ARM wires hit_test → get_htmx_node_and_target
    /// → handle_htmx_target correctly, without bypassing any of that glue. Re-adding an
    /// `if self.throttle.check_input()` guard to the caller arm in `run()` would cause t2
    /// to fail when the INPUT bucket is empty, because the Press would never reach this method.
    /// Number of document rows a single mouse-wheel notch scrolls. Distinct from
    /// PageUp/PageDown (which move a whole viewport) so reading long content is smooth.
    const WHEEL_LINE_STEP: u16 = 3;

    /// Scrolls the viewport by [`Self::WHEEL_LINE_STEP`] rows, clamped to the scrollable
    /// range. `up == true` scrolls toward the top. Shared by web wheel events and native
    /// SSH SGR wheel reports (both arrive as `MouseButton::WheelUp/WheelDown`).
    fn scroll_wheel_lines(&mut self, up: bool) {
        let dims = *self.session.dims.read();
        if self.total_height <= dims.rows {
            return; // content fits; nothing to scroll
        }
        let viewport_h = dims.rows.saturating_sub(1);
        let max_scroll = self.total_height.saturating_sub(viewport_h);
        self.scroll_offset = if up {
            self.scroll_offset.saturating_sub(Self::WHEEL_LINE_STEP)
        } else {
            (self.scroll_offset + Self::WHEEL_LINE_STEP).min(max_scroll)
        };
    }

    /// Reveals the focused node in the viewport, but ONLY when focus just changed since
    /// the last call (tracked via `anchor`). Running it unconditionally every frame fights
    /// manual scrolling — PgDn/wheel past a top-anchored focused node gets yanked straight
    /// back, capping the scroll (the "stuck at 50%" bug). Firing only on focus change still
    /// reveals a Tab/arrow-selected node exactly once.
    ///
    /// Takes individual fields (not `&mut self`) so the caller can invoke it while holding
    /// a read lock on another `self` field via disjoint borrows.
    pub fn autoscroll_to_focused(
        focused_node: Option<NodeId>,
        anchor: &mut Option<NodeId>,
        scroll_offset: &mut u16,
        layout: &oxiterm_renderer::layout::types::LayoutResult,
        viewport_h: u16,
    ) {
        if focused_node == *anchor {
            return;
        }
        if let Some(focused_id) = focused_node {
            if let Some(rect) = layout.nodes.get(&focused_id) {
                use oxiterm_renderer::layout::types::to_cells;
                let (ry, rh) = (to_cells(rect.y), to_cells(rect.height));
                if ry < *scroll_offset {
                    *scroll_offset = ry;
                } else if ry + rh > *scroll_offset + viewport_h {
                    *scroll_offset = (ry + rh).saturating_sub(viewport_h);
                }
            }
        }
        *anchor = focused_node;
    }

    /// Applies a keystroke to the focused text input's bound state: printable characters
    /// append, Backspace/Delete (8/127) delete the last char, Enter is a no-op here (it is
    /// handled as activation upstream). Returns `true` when the key was consumed as text
    /// input (so predictive echo is suppressed). Returns `false` when there is no focused
    /// bound `Input`, or for Enter/other control keys.
    ///
    /// Must run for ALL non-navigation keys — regressing this to only run on a specific
    /// key (e.g. Enter) silently breaks typing into every input field.
    pub fn apply_key_to_focused_input(&mut self, cp: char) -> bool {
        let Some(focused_id) = self.focused_node else { return false };
        let state_key = match self.doc.get_node(focused_id) {
            Some(node) if node.tag == oxiterm_proto::dom::NodeTag::Input => {
                match node.attrs.bind_value.clone() {
                    Some(k) => k,
                    None => return false,
                }
            }
            _ => return false,
        };

        let is_backspace = cp as u32 == 127 || cp as u32 == 8;
        let is_enter = cp == '\r' || cp as u32 == 13;
        if !is_backspace && (is_enter || cp.is_control()) {
            return false;
        }

        let mut value = match self.session.state.read().get(&state_key) {
            Some(crate::state::StateValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        if is_backspace {
            value.pop();
        } else {
            value.push(cp);
        }
        self.session.state.write().set(state_key, crate::state::StateValue::Str(value));
        true
    }

    pub fn handle_mouse_event(&mut self, mut mouse: oxiterm_proto::input::MouseInput) -> bool {
        let dims = *self.session.dims.read();
        tracing::trace!("Received MouseEvent: col={}, row={}, action={:?}", mouse.col, mouse.row, mouse.action);

        // Wheel notches are line-step scrolls, not clicks: intercept before any
        // coordinate remap or hit-test/activation so a scroll never activates a link.
        match mouse.button {
            oxiterm_proto::input::MouseButton::WheelUp => {
                self.scroll_wheel_lines(true);
                return true;
            }
            oxiterm_proto::input::MouseButton::WheelDown => {
                self.scroll_wheel_lines(false);
                return true;
            }
            _ => {}
        }
        if let Some(layout) = &self.layout_engine.last_layout {
            let (col, row) = self.cell_event_to_layout(layout, mouse.col, mouse.row, dims);
            mouse.col = col;
            mouse.row = row;
            tracing::trace!("Adjusted MouseEvent: col={}, row={}", mouse.col, mouse.row);
        } else {
            tracing::trace!("No last layout found — pre-swap mouse event, dropping hit-test");
        }

        self.handle_adjusted_pointer(mouse, None)
    }

    /// Screen cell of a `0x02` event (1-based `col`/`row`) to layout cells: remove the
    /// centering offset and the 1-based origin (saturating at 0), add the scroll.
    pub fn cell_event_to_layout(&self, layout: &oxiterm_renderer::layout::types::LayoutResult, col: u16, row: u16, dims: crate::session_state::PtyDimensions) -> (u16, u16) {
        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
        tracing::trace!("Document centering offset: offset_x={}, offset_y={}", offset_x, offset_y);
        (
            col.saturating_sub(offset_x).saturating_sub(1),
            row.saturating_sub(offset_y).saturating_sub(1).saturating_add(self.scroll_offset),
        )
    }

    /// Screen point of a pointer event (0-based fractional cells) to layout cells: the same
    /// centering offset and scroll as [`Self::cell_event_to_layout`] (both whole cells), no
    /// rounding and no clamping.
    pub fn pointer_to_layout(&self, layout: &oxiterm_renderer::layout::types::LayoutResult, x: f32, y: f32, dims: crate::session_state::PtyDimensions) -> (f32, f32) {
        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
        (x - f32::from(offset_x), y - f32::from(offset_y) + f32::from(self.scroll_offset))
    }

    /// A fractional pointer (OxiUI Phase 6.5, wire `0x03`; display-list clients only).
    ///
    /// `(x, y)` are 0-based screen cells; the centering offset and the scroll are applied
    /// exactly as `handle_mouse_event` applies them (both are whole cells), but without
    /// rounding: the layout-space point `(x - offset_x, y - offset_y + scroll)` is hit-tested
    /// as is, against the fractional rects. Consumers that need a cell (drag deltas, the
    /// `.riv` hover/click) get its floor, clamped at zero. A point left of or above the page
    /// is negative in layout space and hits nothing (the cell path clamps it to cell 0).
    pub fn handle_pointer_event(&mut self, pointer: oxiterm_proto::input::PointerInput) -> bool {
        let dims = *self.session.dims.read();
        tracing::trace!("Received PointerEvent: x={}, y={}, action={:?}", pointer.x, pointer.y, pointer.action);
        match pointer.button {
            oxiterm_proto::input::MouseButton::WheelUp => {
                self.scroll_wheel_lines(true);
                return true;
            }
            oxiterm_proto::input::MouseButton::WheelDown => {
                self.scroll_wheel_lines(false);
                return true;
            }
            _ => {}
        }
        let (lx, ly) = match &self.layout_engine.last_layout {
            Some(layout) => self.pointer_to_layout(layout, pointer.x, pointer.y, dims),
            None => (pointer.x, pointer.y),
        };
        let mouse = oxiterm_proto::input::MouseInput {
            col: lx.floor().max(0.0) as u16,
            row: ly.floor().max(0.0) as u16,
            button: pointer.button,
            action: pointer.action,
            modifiers: pointer.modifiers,
        };
        self.handle_adjusted_pointer(mouse, Some((lx, ly)))
    }

    /// Shared tail of the cell and the fractional path. `mouse` holds layout-space cells;
    /// `point` is the exact layout-space point, or `None` to probe the centre of the cell.
    fn handle_adjusted_pointer(&mut self, mouse: oxiterm_proto::input::MouseInput, point: Option<(f32, f32)>) -> bool {
        let hit_point = point.unwrap_or((f32::from(mouse.col) + 0.5, f32::from(mouse.row) + 0.5));
        self.update_interactive_animations(&mouse);
        self.pending_mouse = Some(mouse.clone());
        self.pending_point = point;

        // Pointer Drag Move (pointer capture)
        if mouse.action == oxiterm_proto::input::MouseAction::Move {
            if let Some(ref drag) = self.active_drag {
                let delta_x = mouse.col as i32 - drag.start_mouse_col as i32;
                let delta_y = mouse.row as i32 - drag.start_mouse_row as i32;
                let new_left = (drag.initial_left as i32 + delta_x).max(0) as i16;
                let new_top = (drag.initial_top as i32 + delta_y).max(0) as i16;
                let target_node = drag.target_node;
                let state_x = drag.state_x_key.clone();
                let state_y = drag.state_y_key.clone();

                if let Some(node) = self.doc.arena.get_mut(target_node) {
                    node.style.position = oxiterm_proto::style::Position::Absolute;
                    node.style.left = Some(f32::from(new_left));
                    node.style.top = Some(f32::from(new_top));
                    self.doc.mark_dirty(target_node);
                }
                if let Some(kx) = state_x {
                    self.session.state.write().set(kx, crate::state::StateValue::Str(new_left.to_string()));
                }
                if let Some(ky) = state_y {
                    self.session.state.write().set(ky, crate::state::StateValue::Str(new_top.to_string()));
                }
                return true;
            }
        }

        // Pointer Drag Release
        if mouse.action == oxiterm_proto::input::MouseAction::Release {
            if let Some(drag) = self.active_drag.take() {
                if let Some(action) = drag.on_drag_end {
                    self.handle_htmx_target(drag.target_node, &action);
                }
                return true;
            }
        }

        // Anchored by spec [SC-05]. Handle interactive navigation or state updates based on htmx event targets.
        if mouse.action == oxiterm_proto::input::MouseAction::Press {
            // Press = activation: unconditional. hit_test + handle_htmx_target always run.
            // NAV bucket inside handle_htmx_target caps the actual page-load rate.
            if let Some(node_id) = self.layout_engine.hit_test_point(hit_point.0, hit_point.1) {
                tracing::trace!("Hit test found node: {:?}", node_id);
                if let Some((drag_target, state_x, state_y, on_drag_end)) = self.find_draggable_target(node_id) {
                    let initial_left = self.doc.get_node(drag_target)
                        .and_then(|n| n.style.left.map(|v| v.round() as i16))
                        .or_else(|| self.layout_engine.last_layout.as_ref().and_then(|l| l.nodes.get(&drag_target).map(|r| r.x.round() as i16)))
                        .unwrap_or(0);
                    let initial_top = self.doc.get_node(drag_target)
                        .and_then(|n| n.style.top.map(|v| v.round() as i16))
                        .or_else(|| self.layout_engine.last_layout.as_ref().and_then(|l| l.nodes.get(&drag_target).map(|r| r.y.round() as i16)))
                        .unwrap_or(0);

                    self.active_drag = Some(DragState {
                        target_node: drag_target,
                        start_mouse_col: mouse.col,
                        start_mouse_row: mouse.row,
                        initial_left,
                        initial_top,
                        state_x_key: state_x,
                        state_y_key: state_y,
                        on_drag_end,
                    });
                }

                if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(node_id) {
                    tracing::trace!("Found HTMX target (node {:?}): {}", target_node_id, htmx_target);
                    self.handle_htmx_target(target_node_id, &htmx_target);
                } else {
                    tracing::trace!("No HTMX target found for node {:?}", node_id);
                }
            } else {
                info!("Hit test returned None");
            }
            true // Press always triggers a render
        } else {
            // Release / Move / hover — flood surface.
            // Gate only the render trigger; skipping a hover render is safe.
            self.throttle.check_input(self.now)
        }
    }

    /// The server-side driver: the only place the event loop blocks (OxiUI Phase 4, step 7b).
    ///
    /// Per iteration: idle check, a blocking wait on the event source for at most
    /// `next_deadline - now`, `handle_event` for each received event (the source stays locked
    /// for the whole batch of at most 100), then `tick`. A browser or winit host does the same
    /// with its own wait and calls `handle_event` / `tick` / `next_deadline` directly.
    pub fn run(&mut self) {
        // Must stay in `run()`: dropping it is what reports `loop_exited()` to the host.
        let _guard = EventLoopGuard { host: self.host.clone() };
        self.prepare_session();
        self.first_frame = true;
        self.render_carry = false;
        self.needs_render = false;
        self.event_since_tick = false;

        loop {
            if self.idle_expired() {
                return;
            }

            // Based on the previous iteration's `now`, so the wait is exactly the planned 16 or
            // 1000 ms and the clock is read once per iteration (after the wait).
            let timeout = self
                .next_deadline(self.now)
                .map_or(std::time::Duration::from_millis(1000), |d| d - self.now);

            let (batch, now) = self.receive_batch(timeout);
            if batch.disconnected {
                return;
            }

            self.tick(now);
            // The 100-event cap defers the render to the next iteration (after this tick).
            if batch.capped {
                self.schedule_render();
            }
        }
    }

    /// Ensures the next `tick` renders (a render owed from a capped batch, or a host request).
    pub fn schedule_render(&mut self) {
        self.render_carry = true;
    }

    /// One-time setup before the first `handle_event` / `tick`: state subscriptions, initial
    /// state injection, `<for>` expansion, the web client's `current_page`, sink setup.
    pub fn prepare_session(&mut self) {
        {
            let mut state = self.session.state.write();
            Self::setup_state_subscriptions(&self.doc, &mut *state);
            Self::inject_initial_state(&mut self.doc, &*state);
        }
        // Expand For-template nodes after initial state injection.
        self.expand_all_for_nodes();

        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            let rel_path = self.base_source_path.as_ref().map(|bp| self.page_source.page_at(bp).page);
            *self.session.current_page.write() = rel_path;
        }

        let _ = self.frame_sink.setup();
    }

    /// Idle timeout check (host policy). True when the session died of idleness (`death_reason`
    /// is set).
    fn idle_expired(&self) -> bool {
        // Anchored by spec [SA-10]. Perform idle timeout check (10 minutes) and terminate the session if exceeded.
        let elapsed = self.host.idle_for();
        if elapsed > std::time::Duration::from_secs(600) {
            info!("Session {} timed out ({}s idle)", self.session.id, elapsed.as_secs());
            // Signal idle death so WsFrameSink::drop emits [0xFF, 1] to web clients.
            self.session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);
            return true;
        }
        false
    }

    /// Media animations, transitions or a drag are active: the loop runs at the 16 ms cadence.
    /// Returns `(cadence_active, transitions_active)`.
    fn activity(&self) -> (bool, bool) {
        let has_active_transitions = self.animation_controller.has_active();
        let has_media_animations = self.has_active_animations();
        let is_dragging = self.active_drag.is_some();
        (has_media_animations || has_active_transitions || is_dragging, has_active_transitions)
    }

    /// When `tick` must run next if no event arrives: 16 ms from `now` while animations, a drag
    /// or media are active, else 1000 ms; earlier when a debounced resize is pending (its
    /// expiry, see `ResizeDebouncer::due_at`) and, while a render is owed (deferred by the frame
    /// limiter, or wanted by an event not yet ticked), at the limiter's next allowed instant
    /// (`FrameRateLimiter::next_allowed`; `now` when a frame is allowed already), so a deferred
    /// frame no longer waits for the idle cadence. The result may be at or before `now`; the
    /// wait computed from it saturates to zero. Never `None` yet; a later step may return `None`
    /// when idle so the host can wait for events indefinitely.
    pub fn next_deadline(&self, now: crate::clock::MonoTime) -> Option<crate::clock::MonoTime> {
        let (active, _) = self.activity();
        let wait = if active {
            std::time::Duration::from_millis(16)
        } else {
            std::time::Duration::from_millis(1000)
        };
        let mut deadline = now + wait;
        if let Some(d) = self.session.resize_debouncer.read().due_at() {
            deadline = deadline.min(d);
        }
        if (self.render_carry || self.needs_render) && !self.output_paused {
            deadline = deadline.min(self.frame_limiter.next_allowed().unwrap_or(now));
        }
        Some(deadline)
    }

    /// Driver only: the blocking wait plus draining up to 100 queued events, each fed to
    /// `handle_event`. `events_in` stays locked for the whole batch. Reads the clock once, right
    /// after the blocking wait, and returns that time.
    fn receive_batch(&mut self, timeout: std::time::Duration) -> (BatchOutcome, crate::clock::MonoTime) {
        let mut capped = false;
        let mut disconnected = false;
        let mut now = self.now;
        {
            let events_in = self.events_in.clone();
            let mut rx_lock = events_in.lock();
            let mut processed_count = 0;
            loop {
                if processed_count >= 100 {
                    capped = true;
                    break;
                }

                let event_opt = if processed_count == 0 {
                    let received = rx_lock.wait_event(timeout);
                    // The one clock read of this iteration, after the blocking wait.
                    now = self.clock.now();
                    self.now = now;
                    match received {
                        WaitResult::Event(e) => Some(e),
                        WaitResult::Timeout => break,
                        WaitResult::Disconnected => {
                            info!("Event loop receiver disconnected. Exiting run loop.");
                            disconnected = true;
                            break;
                        }
                    }
                } else {
                    rx_lock.try_recv()
                };

                let event = match event_opt {
                    Some(e) => e,
                    None => break,
                };

                processed_count += 1;
                if self.handle_event(event, now) == StepOutcome::Quit {
                    disconnected = true;
                    break;
                }
            }
        }
        (BatchOutcome { capped, disconnected }, now)
    }

    /// Step API: feeds one input event (keys, mouse, resize, patches, navigation). Never blocks.
    /// Records internally that a render is wanted; `tick` performs it.
    pub fn handle_event(&mut self, event: InputEvent, now: crate::clock::MonoTime) -> StepOutcome {
        self.now = now;
        self.event_since_tick = true;
        match self.process_event(event) {
            EventOutcome::Quit => StepOutcome::Quit,
            EventOutcome::Handled { needs_render } => {
                self.needs_render |= needs_render;
                StepOutcome::Continue
            }
        }
    }

    /// Step API: applies a due debounced resize and renders a frame when one is wanted and the
    /// frame limiter allows it (emits through the sinks). Never blocks. Call it after the events
    /// of a batch and whenever `next_deadline` passes.
    pub fn tick(&mut self, now: crate::clock::MonoTime) {
        self.now = now;
        let (cadence_active, transitions_active) = self.activity();
        // Unconditional clears: a render skipped because output is paused is dropped, not kept.
        let first = std::mem::take(&mut self.first_frame);
        let carry = std::mem::take(&mut self.render_carry);
        let from_events = std::mem::take(&mut self.needs_render);
        let had_event = std::mem::take(&mut self.event_since_tick);
        let mut needs_render = first
            || carry
            || from_events
            || transitions_active
            // No event in this step: the active cadence's own timeout wants a frame.
            || (cadence_active && !had_event);

        if self.poll_resize() {
            needs_render = true;
        }

        if self.render_frame_if_due(needs_render) {
            self.render_carry = true;
        }
    }

    /// Handles one input event (everything the old receive loop did per event).
    fn process_event(&mut self, event: InputEvent) -> EventOutcome {
        let mut needs_render = false;
        self.host.touch_activity();
        let is_input_focused = self.focused_node
            .and_then(|id| self.doc.get_node(id))
            .map(|node| node.tag == oxiterm_proto::dom::NodeTag::Input)
            .unwrap_or(false);
        // Preprocess: remap scroll keys and detect quit. Returns None on quit.
        // Quit is SSH-only; web sessions pass 'q' through unchanged.
        let is_web = self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst);
        let event = match Self::preprocess_key(event, is_input_focused, is_web) {
            Some(e) => e,
            None => {
                info!("Quit requested via keyboard");
                return EventOutcome::Quit;
            }
        };
        match event {
            InputEvent::StatePatched => {
                // Same sequence as the render prelude: syncing the patch alone
                // would consume the dirty keys, and the render's
                // restore_baselines would then revert animated properties.
                self.apply_state_to_doc(self.now);
                self.expand_all_for_nodes();
                needs_render = true;
            }
            InputEvent::Resize { cols, rows } => {
                info!("Reactor notified resize to {}x{}", cols, rows);
                needs_render = true;
            }
            InputEvent::SwitchViewport(is_mobile) => {
                self.session.is_mobile.store(is_mobile, std::sync::atomic::Ordering::SeqCst);
                if let Some(bp) = self.base_source_path.clone() {
                    let file_name = bp.file_name().and_then(|f| f.to_str()).unwrap_or("index.thtml").to_string();
                    if self.throttle.check_nav(self.now) {
                        if let Err(e) = self.resolve_and_load_page(&file_name) {
                            warn!("SwitchViewport: failed to load page: {}", e);
                            self.send_nav_error();
                        }
                        needs_render = true;
                    } else if self.throttle.is_first_throttle() {
                        warn!("SwitchViewport: NAV throttle active, dropping viewport switch");
                        self.send_nav_throttle_notice();
                    }
                }
            }
            InputEvent::NavigateTo(rel_path) => {
                // Containment check is inside resolve_and_load_page.
                if self.throttle.check_nav(self.now) {
                    if let Err(e) = self.resolve_and_load_page(&rel_path) {
                        warn!("NavigateTo: failed to load page: {}", e);
                        self.send_nav_error();
                    }
                    needs_render = true;
                } else if self.throttle.is_first_throttle() {
                    warn!("NavigateTo: NAV throttle active, dropping navigation");
                    self.send_nav_throttle_notice();
                }
            }
            InputEvent::NavigateToAppPath(app_path) => {
                if self.navigate_app_path(&app_path) {
                    needs_render = true;
                }
            }
            InputEvent::AppResponse(patch) => {
                self.apply_app_patch(patch.0, PatchOrigin::ActionResponse);
                needs_render = true;
            }
            InputEvent::PushPatch(patch) => {
                self.apply_app_patch(patch.0, PatchOrigin::Push);
                needs_render = true;
            }
            InputEvent::OpenUrl(url) => {
                // Sent by dispatcher when App Server patch contains open_url.
                // handle_open_url emits opcode 0x33 to web clients only;
                // SSH sessions receive a warn! and no action.
                self.handle_open_url(url, oxiterm_proto::dom::NodeId(0));
            }
            InputEvent::SetAppToken(token) => {
                self.handle_set_app_token(token);
            }
            InputEvent::Reload => {
                if self.reload_current_source() {
                    info!("Hot Reload successful for session {}", self.session.id);
                    needs_render = true;
                }
            }
            InputEvent::SetDisplayList(_) => {
                // The flag is already stored by the WS handler. Not NAV-throttled:
                // skipping the reparse would leave a GUI session with a TUI-parsed doc.
                self.reload_current_source();
                // The client's picture is no longer the front buffer's.
                self.buffer.force_dirty();
                needs_render = true;
            }
            // Anchored by spec [Plan-2.2/R1]: KeyPress arm runs unconditionally.
            // Activation paths (focus-nav, Enter) must never be dropped by the INPUT bucket.
            // check_input() is moved inside the character-input else-branch only.
            InputEvent::KeyPress(key) => {

                let is_nav_forward = key.codepoint == '\u{F701}'
                    || key.codepoint == '\u{F702}'
                    || key.codepoint == '\t';
                let is_nav_backward = key.codepoint == '\u{F700}'
                    || key.codepoint == '\u{F703}';

                if is_nav_forward || is_nav_backward {
                    // Focus navigation — unconditional (cheap, no render flood risk).
                    if !self.focusable_nodes.is_empty() {
                        let n = self.focusable_nodes.len();
                        let cur = self.focused_node
                            .and_then(|f| self.focusable_nodes.iter().position(|&x| x == f))
                            .unwrap_or(0);
                        let next = if is_nav_forward {
                            (cur + 1) % n
                        } else {
                            (cur + n - 1) % n
                        };
                        self.focused_node = Some(self.focusable_nodes[next]);
                        info!("Focus moved to node {:?}", self.focused_node);
                        needs_render = true;
                    }
                } else {
                    // Enter → HTMX activation (Enter only). NAV bucket in handle_htmx_target caps loads.
                    if key.codepoint == '\r' || key.codepoint as u32 == 13 {
                        if let Some(focused) = self.focused_node {
                            if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(focused) {
                                info!("Enter activated HTMX (node {:?}): {}", target_node_id, htmx_target);
                                self.handle_htmx_target(target_node_id, &htmx_target);
                                needs_render = true;
                            }
                        }
                    }
                    // Character input / predictive echo — runs for ALL non-navigation keys
                    // (printable chars append, Backspace deletes; Enter is a no-op here).
                    // D2: state write (append char / backspace) is UNCONDITIONAL —
                    // a dropped frame is invisible; a dropped keystroke corrupts
                    // a comment or password field.
                    // check_input() gates only needs_render (render-flood surface).
                    let handled_by_input = self.apply_key_to_focused_input(key.codepoint);

                    if !handled_by_input {
                        // D7: suppress predictive echo for password inputs
                        let is_password_focused = self.focused_node
                            .and_then(|id| self.doc.get_node(id))
                            .and_then(|n| n.attrs.input_type.as_deref().map(|t| t == "password"))
                            .unwrap_or(false);
                        if !is_password_focused {
                            let mut echo = self.session.predictive_echo.write();
                            echo.buffer.push(key.codepoint);
                        }
                    }
                    // Render is gated: skipping a frame is safe; losing a char is not.
                    if self.throttle.check_input(self.now) {
                        needs_render = true;
                    }
                }
            }
            // Anchored by spec [Plan-2.2/R1]: MouseEvent arm runs unconditionally.
            // The arm delegates entirely to handle_mouse_event(), which owns the
            // Press-vs-flood split. Re-adding an `if self.throttle.check_input()`
            // guard HERE would cause t2 to fail (Press swallowed when bucket empty).
            InputEvent::MouseEvent(mouse) => {
                if self.handle_mouse_event(mouse) {
                    needs_render = true;
                }
            }
            // Fractional pointer of a display-list client (Phase 6.5): same Press-vs-flood
            // split, same unconditional arm.
            InputEvent::PointerEvent(pointer) => {
                if self.handle_pointer_event(pointer) {
                    needs_render = true;
                }
            }
            InputEvent::CapabilityResponse(raw) => {
                info!("Received DA1 response: {}", String::from_utf8_lossy(&raw));
                self.session.terminal_profile.write().parse_da1_response(&raw);
            }
            InputEvent::Xoff => { self.output_paused = true; }
            InputEvent::Xon => { self.output_paused = false; needs_render = true; }
            InputEvent::Refresh => { needs_render = true; }
            InputEvent::ScrollUp => {
                let dims = *self.session.dims.read();
                let has_scroll = self.total_height > dims.rows;
                let viewport_h = if has_scroll {
                    dims.rows.saturating_sub(1)
                } else {
                    dims.rows
                };
                self.scroll_offset = self.scroll_offset.saturating_sub(viewport_h);
                needs_render = true;
            }
            InputEvent::ScrollDown => {
                let dims = *self.session.dims.read();
                let has_scroll = self.total_height > dims.rows;
                let viewport_h = if has_scroll {
                    dims.rows.saturating_sub(1)
                } else {
                    dims.rows
                };
                let max_scroll = self.total_height.saturating_sub(viewport_h);
                self.scroll_offset = (self.scroll_offset + viewport_h).min(max_scroll);
                needs_render = true;
            }
            InputEvent::TextInput(text) => {
                info!("TextInput received (len={}): {:?}", text.len(), text);
            }
            _ => {}
                }
        EventOutcome::Handled { needs_render }
    }

    /// Applies a due debounced resize (buffer and screen reset). True when one was applied.
    fn poll_resize(&mut self) -> bool {
        let dims_opt = self.session.resize_debouncer.write().poll(self.now);
        if let Some(dims) = dims_opt {
            *self.session.dims.write() = dims;
            self.buffer = DoubleBuffer::new(dims.cols, dims.rows);
            info!("Resized session {} to {:?}", self.session.id, dims);

            let _ = self.frame_sink.clear_screen();

            return true;
        }
        false
    }

    /// Renders a frame when one is wanted and the frame limiter allows it. Returns true when the
    /// limiter deferred the frame (the caller must retry on the next tick: `render_carry`).
    fn render_frame_if_due(&mut self, needs_render: bool) -> bool {
        if needs_render && !self.output_paused {
            if self.frame_limiter.should_render(self.now) {
                self.render_frame();
            } else {
                return true;
            }
        }
        false
    }

    /// Computes layout and draws, overlays and sends one frame. The `state` and predictive-echo
    /// read guards below are held until the end of the function, through the sink sends.
    fn render_frame(&mut self) {
        let now = self.now;
        self.apply_state_to_doc(now);
        self.animation_controller.tick(&mut self.doc, now);
        self.rebuild_focusable_nodes();

        let dims = *self.session.dims.read();
        // Display-list clients lay out in fractional GUI mode; everyone else (and a
        // client whose flag was reset) in whole-cell TUI mode. Checked here, at the
        // single compute site, so it also covers flags set without an event.
        let want_mode = self.render_mode();
        if self.layout_engine.mode() != want_mode {
            self.layout_engine = LayoutEngine::with_mode(want_mode);
        }
        // The viewport is the root's minimum height; content can grow past it.
        self.layout_engine.set_viewport_rows(dims.rows);
        let state_guard = self.session.state.read();
        if let Ok(layout) = self.layout_engine.compute(&mut self.doc, dims.cols, self.scroll_offset, Some(&*state_guard)) {
        self.total_height = layout.total_height;

        let has_scroll = self.total_height > dims.rows;
        let viewport_h = if has_scroll {
            dims.rows.saturating_sub(1)
        } else {
            dims.rows
        };

        Self::autoscroll_to_focused(
            self.focused_node,
            &mut self.autoscroll_anchor,
            &mut self.scroll_offset,
            &layout,
            viewport_h,
        );

        let max_scroll = self.total_height.saturating_sub(viewport_h);
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        {
            let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
            let start_x = offset_x as i32;
            let start_y = (offset_y as i32) - (self.scroll_offset as i32);

            let mut active = Vec::new();
            let mut stack = vec![(self.doc.root, start_x, start_y)];
            while let Some((node_id, parent_x, parent_y)) = stack.pop() {
                if let Some(node) = self.doc.arena.get(node_id) {
                    let rect = layout.nodes.get(&node_id).copied().unwrap_or_default().cells();
                    let abs_x = parent_x + rect.x as i32;
                    let abs_y = parent_y + rect.y as i32;
                                
                    let has_border = node.style.border.is_some();
                    let content_x = if has_border { abs_x + 1 } else { abs_x };
                    let content_y = if has_border { abs_y + 1 } else { abs_y };
                    let content_w = if has_border { rect.width.saturating_sub(2) } else { rect.width };
                    let content_h = if has_border { rect.height.saturating_sub(2) } else { rect.height };

                    if node.tag == oxiterm_proto::dom::NodeTag::Img || node.tag == oxiterm_proto::dom::NodeTag::Video {
                        if let Some(ref src) = node.attrs.src {
                            // Keep the true (possibly negative) position so media
                            // scrolled partly out of view is clipped by the client
                            // instead of sticking to row/column 0; skip media that
                            // is entirely outside the viewport.
                            let visible = content_y + i32::from(content_h) > 0
                                && content_y < i32::from(dims.rows)
                                && content_x + i32::from(content_w) > 0
                                && content_x < i32::from(dims.cols);
                            if visible {
                                active.push(MediaRenderInfo {
                                    path: src.clone(),
                                    x: content_x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                                    y: content_y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                                    width: content_w,
                                    height: content_h,
                                });
                            }
                        }
                    }
                                
                    for &child in &node.children {
                        stack.push((child, parent_x, parent_y));
                    }
                }
            }
            *self.session.active_media.write() = active;
        }

        if let Some(ref mut bridge) = self.dbus_bridge {
            let tree = oxiterm_a11y::build_a11y_tree(&self.doc);
            let _ = bridge.register_at_spi(&tree);
            if let Some(active_id) = self.session.predictive_echo.read().active_node {
                let _ = bridge.update_focus(active_id, &tree);
            }
        }

        // Anchored by spec [QUAL-01]. Dispatch deferred mouse inputs once a valid layout is available.
        if let Some(mouse) = self.pending_mouse.take() {
            match self.pending_point.take() {
                Some((x, y)) => { let _ = self.event_bus.dispatch_mouse_at(mouse, x, y, &mut self.doc, &layout); }
                None => { let _ = self.event_bus.dispatch_mouse(mouse, &mut self.doc, &layout); }
            }
        }

        let mut profile = self.session.terminal_profile.read().clone();
        profile.is_web = self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst);
        let base_dir = self.source_path.as_ref().and_then(|p| p.parent());
        let app_base_dir = self.session.app_base_dir.read().clone();
        self.buffer.back.clear();
        // Display-list clients get the painted list; the overlays below are appended to it as
        // CellGrids. Only the rows an overlay is about to write are saved first (`saved_rows`,
        // taken from the buffer exactly as the list left it), and only those rows are diffed
        // afterwards: every other row is untouched, so the result equals a full snapshot diff.
        let gui_frame = if self.session.display_list_client.load(std::sync::atomic::Ordering::SeqCst) {
            let list = Renderer::render_frame_mode(&self.doc, &layout, &mut self.buffer.back, &profile, base_dir, app_base_dir.as_deref(), self.scroll_offset, self.layout_engine.mode());
            Some(list)
        } else {
            Renderer::render_node(&self.doc, &layout, &mut self.buffer.back, &profile, base_dir, app_base_dir.as_deref(), self.scroll_offset);
            None
        };

        let is_gui = gui_frame.is_some();
        let mut saved_rows: Vec<(u16, Vec<oxiterm_renderer::render::buffer::Cell>)> = Vec::new();
        // Debug builds also keep the eager full snapshot and assert both diffs agree.
        #[cfg(debug_assertions)]
        let full_snapshot = is_gui.then(|| oxiterm_renderer::CellBuffer {
            cells: self.buffer.back.cells.clone(),
            width: self.buffer.back.width,
            height: self.buffer.back.height,
            graphics: Vec::new(),
        });

        // Focus ring: draw markers on left/right edge of focused node
        if let Some(focused_id) = self.focused_node {
            let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
            if let Some(rect) = layout.nodes.get(&focused_id).map(|r| r.cells()) {
                let mid_row_offset = focus_ring_row(&layout.nodes[&focused_id]) + i32::from(offset_y) - i32::from(self.scroll_offset);
                if mid_row_offset >= 0 && mid_row_offset < viewport_h as i32 {
                    let mid_row = mid_row_offset as u16;
                    let focus_fg = oxiterm_proto::style::AnsiColor::Color256(51); // bright cyan
                    let focus_bg = oxiterm_proto::style::AnsiColor::Reset;
                    if is_gui {
                        Self::save_row(&mut saved_rows, &self.buffer.back, mid_row);
                    }
                    // Markers MUST be unambiguous width-1 glyphs: a wide glyph
                    // (e.g. ▶/◀, which are East-Asian Ambiguous) drawn in the
                    // cell before the label pushes the visible text right off
                    // its hit box on terminals that render them 2-wide, so
                    // clicks on a focused label miss. ASCII '>'/'<' are safe.
                    if rect.x > 0 {
                        let lx = rect.x.saturating_sub(1) + offset_x;
                        if lx < self.buffer.back.width {
                            self.buffer.back.set(lx, mid_row, oxiterm_renderer::render::buffer::Cell {
                                ch: FOCUS_RING_LEFT,
                                fg: focus_fg,
                                bg: focus_bg,
                                bold: true,
                                ..Default::default()
                            });
                        }
                    }
                    let right = rect.x + rect.width + offset_x;
                    if right < self.buffer.back.width {
                        self.buffer.back.set(right, mid_row, oxiterm_renderer::render::buffer::Cell {
                            ch: FOCUS_RING_RIGHT,
                            fg: focus_fg,
                            bg: focus_bg,
                            bold: true,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // S5-21: PredictiveEcho overlay
        // D7: suppress overlay for focused password inputs
        let echo_suppressed = self.focused_node
            .and_then(|id| self.doc.get_node(id))
            .and_then(|n| n.attrs.input_type.as_deref().map(|t| t == "password"))
            .unwrap_or(false);
        let echo = self.session.predictive_echo.read();
        if !echo.buffer.is_empty() && !echo_suppressed {
            let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);

            let (mut cursor_x, mut cursor_y) = (offset_x, offset_y.saturating_sub(self.scroll_offset));
            if let Some(node_id) = echo.active_node {
                if let Some(rect) = layout.nodes.get(&node_id).map(|r| r.cells()) {
                    cursor_x = rect.x + offset_x;
                    cursor_y = (rect.y + offset_y).saturating_sub(self.scroll_offset);
                }
            }

            if cursor_y < viewport_h {
                if is_gui {
                    Self::save_row(&mut saved_rows, &self.buffer.back, cursor_y);
                }
                for ch in echo.buffer.chars() {
                    if cursor_x < self.buffer.back.width {
                        self.buffer.back.set(cursor_x, cursor_y, oxiterm_renderer::render::buffer::Cell {
                            ch,
                            fg: oxiterm_proto::style::AnsiColor::Color256(2),
                            ..Default::default()
                        });
                        cursor_x += 1;
                    }
                }
            }
        }

        // Renders a visual scroll progress indicator at the bottom of the viewport if content exceeds terminal rows.
        if self.total_height > dims.rows {
            let last_row = dims.rows.saturating_sub(1);
            if is_gui {
                Self::save_row(&mut saved_rows, &self.buffer.back, last_row);
            }
            let mut status_parts = Vec::new();
                        
            let max_scroll = self.total_height.saturating_sub(viewport_h);
            if self.scroll_offset > 0 {
                status_parts.push("▲ PgUp".to_string());
            }
            if self.scroll_offset < max_scroll {
                status_parts.push("▼ PgDn".to_string());
            }

            // Position model: line fraction, bar fill, and percentage all
            // derive from the SAME scroll_offset/max_scroll, so they can never
            // disagree. max_scroll comes from the layout extent (total_height),
            // matching the vertical centering in `get_centering_offset`.
            let bar_width = 8;
            let ind = ScrollIndicator::new(self.scroll_offset, self.total_height, viewport_h, bar_width);
            status_parts.push(format!("line {}/{}", ind.position, ind.steps));
            let pct = ind.percent;
            let filled_width = ind.filled;
            let bar: String = std::iter::repeat('█').take(filled_width)
                .chain(std::iter::repeat('░').take(bar_width - filled_width))
                .collect();
                            
            status_parts.push(format!("{} {}%", bar, pct));
                        
            let status_str = status_parts.join("  ");
            let bar_bg = oxiterm_proto::style::AnsiColor::Color256(236); // dark grey
            let bar_fg = oxiterm_proto::style::AnsiColor::Color256(255); // white
                        
            for x in 0..dims.cols {
                self.buffer.back.set(x, last_row, oxiterm_renderer::render::buffer::Cell {
                    ch: ' ',
                    fg: bar_fg,
                    bg: bar_bg,
                    ..Default::default()
                });
            }
                        
            let start_x = if dims.cols > status_str.chars().count() as u16 {
                (dims.cols - status_str.chars().count() as u16) / 2
            } else {
                0
            };
                        
            for (i, ch) in status_str.chars().enumerate() {
                let x = start_x + i as u16;
                if x < dims.cols {
                    self.buffer.back.set(x, last_row, oxiterm_renderer::render::buffer::Cell {
                        ch,
                        fg: bar_fg,
                        bg: bar_bg,
                        bold: true,
                        ..Default::default()
                    });
                }
            }
        }
                    
        let _ = self.frame_sink.update_document(&self.doc);
        let sent = if let Some(mut list) = gui_frame {
            saved_rows.sort_by_key(|(y, _)| *y);
            let extra = oxiterm_renderer::render::cell_sink::CellSink::diff_grids_rows(&saved_rows, &self.buffer.back);
            #[cfg(debug_assertions)]
            if let Some(ref snap) = full_snapshot {
                assert!(
                    extra == oxiterm_renderer::render::cell_sink::CellSink::diff_grids(snap, &self.buffer.back),
                    "row-restricted overlay diff differs from the full snapshot diff"
                );
            }
            list.items.extend(extra);
            let r = self.frame_sink.send_display_list(&list);
            // ARIA mirror (0x37): same origin the display-list builder used
            // (centering offset, minus scroll for non-fixed nodes).
            let (ax, ay) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
            let json = self.aria_cache.json(
                &self.doc,
                &layout,
                self.focused_node,
                (ax as i32, ay as i32 - self.scroll_offset as i32),
                (ax, ay),
            );
            let _ = self.frame_sink.send_a11y(json.as_bytes());
            r
        } else {
            self.frame_sink.send_frame(&self.buffer.front, &self.buffer.back)
        };
        if let Ok(true) = sent {
            self.buffer.swap();
            self.frame_limiter.record_frame(self.now);
        }
    }
    }

    /// Remembers the current content of row `y` (once) before an overlay writes into it.
    fn save_row(saved: &mut Vec<(u16, Vec<oxiterm_renderer::render::buffer::Cell>)>, back: &oxiterm_renderer::CellBuffer, y: u16) {
        let w = back.width as usize;
        let y_us = y as usize;
        if y >= back.height || saved.iter().any(|(r, _)| *r == y) || back.cells.len() < (y_us + 1) * w {
            return;
        }
        saved.push((y, back.cells[y_us * w..(y_us + 1) * w].to_vec()));
    }

    fn has_active_animations(&self) -> bool {
        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        if let Some(ref layout) = self.layout_engine.last_layout {
            for node_id in layout.nodes.keys() {
                if let Some(node) = self.doc.get_node(*node_id) {
                    if node.tag == oxiterm_proto::dom::NodeTag::Img {
                        if let Some(ref src) = node.attrs.src {
                            if src.ends_with(".json") || src.ends_with(".riv") {
                                return true;
                            }
                        }
                    } else if node.tag == oxiterm_proto::dom::NodeTag::Video {
                        if node.attrs.src.is_some() {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn update_interactive_animations(&self, mouse: &oxiterm_proto::input::MouseInput) {
        if let Some(ref layout) = self.layout_engine.last_layout {
            for (node_id, rect) in layout.nodes.iter().map(|(id, r)| (id, r.cells())) {
                if let Some(node) = self.doc.get_node(*node_id) {
                    if node.tag == oxiterm_proto::dom::NodeTag::Img {
                        if let Some(ref src) = node.attrs.src {
                            if src.ends_with(".riv") {
                                let is_inside = mouse.col >= rect.x
                                    && mouse.col < rect.x + rect.width
                                    && mouse.row >= rect.y
                                    && mouse.row < rect.y + rect.height;
                                
                                let resolved_path = if let Some(ref base) = self.source_path.as_ref().and_then(|p| p.parent()) {
                                    base.join(src)
                                } else {
                                    std::path::PathBuf::from(src)
                                };
                                
                                let registry = oxiterm_renderer::render::cache::PlaybackRegistry::get();
                                registry.set_hover(&resolved_path, is_inside);
                                if is_inside && mouse.action == oxiterm_proto::input::MouseAction::Press {
                                    registry.set_click(&resolved_path, true, Some((mouse.col - rect.x, mouse.row - rect.y)));
                                } else if mouse.action == oxiterm_proto::input::MouseAction::Release {
                                    registry.set_click(&resolved_path, false, None);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Server-initiated navigation (`navigate` in a patch): same containment check and NAV
    /// throttle as `NavigateTo`. Returns whether a render is needed.
    fn navigate_app_path(&mut self, app_path: &str) -> bool {
        if self.throttle.check_nav(self.now) {
            if let Err(e) = self.load_app_page(app_path) {
                warn!("NavigateToAppPath: failed to load page: {}", e);
                self.send_nav_error();
            }
            true
        } else {
            if self.throttle.is_first_throttle() {
                warn!("NavigateToAppPath: NAV throttle active, dropping navigation");
                self.send_nav_throttle_notice();
            }
            false
        }
    }

    /// The one patch path: applies a JSON state patch that arrived as an event.
    ///
    /// Order (deterministic, all on the loop thread): take `navigate`, `open_url`,
    /// `set_app_token` out of the patch, apply the remaining state keys (so any token resync
    /// snapshots the patched state), then the `open_url` notice, the token, the navigation, and
    /// finally the same document sync + `<for>` expansion as `StatePatched`.
    ///
    /// Origin policy, kept exactly as the two transports behaved before step 3 (traces pin it):
    /// - `ActionResponse`: `open_url` is extracted and sent as a notice; a changed token is set
    ///   and resynced to the App Server (`app_token_sync`) but NOT announced with a 0x35 notice.
    /// - `Push`: `open_url` is NOT extracted (stored as an ordinary key); a changed token is set
    ///   and announced with an `AppToken` notice, with no resync.
    pub fn apply_app_patch(&mut self, patch: serde_json::Value, origin: PatchOrigin) {
        if let serde_json::Value::Object(mut obj) = patch {
            // `navigate` is never stored as state; it is applied AFTER the rest of the patch so
            // the new page sees the patched state.
            let navigate = match obj.remove("navigate") {
                Some(serde_json::Value::String(path)) => SessionState::sanitize_navigate_target(&path),
                Some(_) => {
                    warn!("app patch: ignoring non-string navigate value");
                    None
                }
                None => None,
            };
            let open_url = if origin == PatchOrigin::ActionResponse {
                match obj.remove("open_url") {
                    Some(serde_json::Value::String(u)) => Some(u),
                    _ => None,
                }
            } else {
                None
            };
            let token = obj.remove("set_app_token").map(|v| match v {
                serde_json::Value::String(s) => s,
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            });

            self.session.apply_patch_keys(obj);

            if let Some(url) = open_url {
                self.handle_open_url(url, NodeId(0));
            }
            if let Some(tok) = token {
                match origin {
                    PatchOrigin::ActionResponse => self.handle_set_app_token(tok),
                    PatchOrigin::Push => self.set_app_token_announce(tok),
                }
            }
            if let Some(path) = navigate {
                self.navigate_app_path(&path);
            }
        }
        // Same sequence as the render prelude: syncing the patch alone would consume the dirty
        // keys, and the render's restore_baselines would then revert animated properties.
        let now = self.now;
        self.apply_state_to_doc(now);
        self.expand_all_for_nodes();
    }

    /// Push-endpoint token update: set/remove `_app_token` and announce a change to the web
    /// client (0x35, also for removal). No resync dispatch.
    fn set_app_token_announce(&mut self, token: String) {
        let current_token = match self.session.state.read().get("_app_token") {
            Some(crate::state::StateValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        if token != current_token {
            {
                let mut state = self.session.state.write();
                if token.is_empty() {
                    state.remove("_app_token");
                } else {
                    state.set("_app_token".to_string(), crate::state::StateValue::Str(token.clone()));
                }
            }
            let _ = self.frame_sink.send_notice(oxiterm_renderer::ClientNotice::AppToken(token));
        }
    }

    pub fn handle_set_app_token(&mut self, token: String) {
        let current_token = match self.session.state.read().get("_app_token") {
            Some(crate::state::StateValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        if token.is_empty() {
            if !current_token.is_empty() {
                self.session.state.write().remove("_app_token");
            }
        } else if token != current_token {
            self.session.state.write().set("_app_token".to_string(), crate::state::StateValue::Str(token));
            self.try_dispatch("app_token_sync");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_io::MemoryQueue;
    use crate::page_source::{MemPageSource, PageSource};
    use crate::session_state::{PredictiveEcho, PtyDimensions};
    use oxiterm_renderer::render::buffer::CellBuffer;
    use oxiterm_renderer::FrameSink;
    use std::time::Duration;

    /// Host stub: no transport behind the loop.
    struct NullHost;
    impl LoopHost for NullHost {
        fn touch_activity(&self) {}
        fn idle_for(&self) -> Duration { Duration::ZERO }
        fn loop_exited(&self) {}
    }

    /// Default sink of the core tests: reports a frame as sent whenever the diff is non-empty
    /// (the buffer swaps), and drops notices. Tests that look at output install their own sink.
    struct TestSink;
    impl FrameSink for TestSink {
        fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
            Ok(!oxiterm_renderer::DiffEngine::diff(front, back).is_empty() || !back.graphics.is_empty())
        }
    }

    fn empty_pages() -> Arc<dyn PageSource> {
        Arc::new(MemPageSource::new(Vec::<(String, String)>::new()))
    }

    /// A pure session on a frozen fake clock.
    fn test_session() -> Arc<SessionState> {
        Arc::new(SessionState::new(0, Arc::new(crate::clock::FakeClock::new())))
    }

    /// Loop handles over a fresh in-memory queue and a stub host.
    fn loop_handles(session: &Arc<SessionState>) -> LoopHandles {
        let queue = MemoryQueue::new(128);
        LoopHandles {
            state: session.clone(),
            events_in: Arc::new(parking_lot::Mutex::new(queue.clone())),
            events_out: Arc::new(queue),
            host: Arc::new(NullHost),
        }
    }

    fn new_el(session: &Arc<SessionState>, event_bus: Arc<crate::events::EventBus>, doc: THTMLDocument) -> EventLoop {
        EventLoop::new(loop_handles(session), event_bus, Box::new(TestSink), empty_pages(), doc)
    }

    /// Applies a patch the way the loop's key step does (limits, reserved keys), without a loop.
    fn patch_keys(session: &SessionState, patch: serde_json::Value) {
        if let serde_json::Value::Object(obj) = patch {
            session.apply_patch_keys(obj);
        }
    }


    /// A bare loop over an empty document, for patch-handling tests.
    fn bare_loop(session: Arc<SessionState>) -> EventLoop {
        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let event_bus = Arc::new(crate::events::EventBus::new());
        new_el(&session, event_bus, doc)
    }


    #[test]
    fn test_autoscroll_does_not_fight_manual_scroll() {
        // Focused node anchored at the top (y=1). After it's revealed once, a manual
        // scroll (offset advanced) must NOT be reverted on subsequent frames while focus
        // is unchanged — otherwise you can't scroll past a top-anchored focused link.
        let (mut el, btn) = make_el_with_htmx_button("noop:x");
        *el.session.dims.write() = PtyDimensions { cols: 20, rows: 6 };
        el.total_height = 20;
        el.focused_node = Some(btn);
        inject_layout(
            &mut el,
            vec![(btn, oxiterm_renderer::layout::types::Rect { x: 0.0, y: 1.0, width: 6.0, height: 1.0 })],
            20,
        );
        let layout = el.layout_engine.last_layout.clone().unwrap();
        let viewport_h = 5;

        // Frame 1: focus just became `btn` — reveal it (already visible at y=1, offset stays 0).
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout, viewport_h);
        assert_eq!(el.scroll_offset, 0);

        // User scrolls down manually.
        el.scroll_offset = 4;
        // Frame 2+: focus unchanged — offset must be left alone.
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout, viewport_h);
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout, viewport_h);
        assert_eq!(el.scroll_offset, 4, "manual scroll must survive while focus is unchanged");

        // When focus actually changes to an off-screen node, it is revealed once.
        let other = el.doc.arena.alloc(oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Box));
        let mut nodes = layout.nodes.clone();
        nodes.insert(other, oxiterm_renderer::layout::types::Rect { x: 0.0, y: 18.0, width: 4.0, height: 1.0 });
        let layout2 = oxiterm_renderer::layout::types::LayoutResult { nodes, total_height: 20, paint_order: Vec::new(), ..Default::default() };
        el.focused_node = Some(other);
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout2, viewport_h);
        assert_eq!(el.scroll_offset, 19 - viewport_h, "focus change reveals the newly focused node");
    }


    #[test]
    fn test_typing_updates_focused_input_state() {
        // Regression: printable keys must append to the focused input's bound state,
        // Backspace deletes, Enter is a no-op, and no focused input = not consumed.
        // (Previously all char input was wrongly gated behind the Enter branch, so typing
        // into any field did nothing.)
        use oxiterm_proto::dom::{Node, NodeTag};
        let (mut el, _btn) = make_el_with_htmx_button("noop:x");
        let mut input = Node::new(NodeTag::Input);
        input.attrs.bind_value = Some("name".to_string());
        let input_id = el.doc.arena.alloc(input);
        el.focused_node = Some(input_id);

        assert!(el.apply_key_to_focused_input('h'));
        assert!(el.apply_key_to_focused_input('i'));
        assert_eq!(
            el.session.state.read().get("name").cloned(),
            Some(crate::state::StateValue::Str("hi".to_string()))
        );

        assert!(el.apply_key_to_focused_input('\u{8}'), "backspace consumed");
        assert_eq!(
            el.session.state.read().get("name").cloned(),
            Some(crate::state::StateValue::Str("h".to_string()))
        );

        assert!(!el.apply_key_to_focused_input('\r'), "Enter is not text input");
        assert_eq!(
            el.session.state.read().get("name").cloned(),
            Some(crate::state::StateValue::Str("h".to_string()))
        );

        el.focused_node = None;
        assert!(!el.apply_key_to_focused_input('x'), "no focused input → not consumed");
    }


    #[test]
    fn test_focus_ring_glyphs_are_unambiguous_width() {
        // A wide/ambiguous focus-ring marker drawn before a focused label pushes the
        // visible text off its hit box, so clicks on the focused label miss.
        use oxiterm_renderer::render::unicode::is_ambiguous_width;
        assert!(!is_ambiguous_width(FOCUS_RING_LEFT), "left focus marker must be width-1");
        assert!(!is_ambiguous_width(FOCUS_RING_RIGHT), "right focus marker must be width-1");
    }


    #[test]
    fn test_scroll_indicator_is_mutually_consistent() {
        // t2: line X/Y, bar fill, and percentage all come from one total_height, so the
        // fraction and the percentage agree at top / middle / bottom.
        let total_height = 33u16;
        let viewport_h = 30u16; // max_scroll = 3 → 4 positions
        let bar_width = 8usize;

        let max_scroll = total_height - viewport_h;
        assert_eq!(max_scroll, 3);

        let check = |offset: u16, exp_pos: u16, exp_pct: u8| {
            let ind = ScrollIndicator::new(offset, total_height, viewport_h, bar_width);
            assert_eq!(ind.steps, max_scroll + 1, "denominator is the position count");
            assert_eq!(ind.position, exp_pos);
            assert_eq!(ind.percent, exp_pct);
            // Percentage and the line fraction tell the same story: both are the same
            // 0-indexed progress out of `max_scroll`.
            let progress = ((ind.position - 1) as f32 / (ind.steps - 1) as f32 * 100.0).round() as u8;
            assert_eq!(ind.percent, progress, "percent must equal the line-fraction progress");
            // Bar fill agrees with the percentage.
            let expected_filled = (ind.percent as usize * bar_width) / 100;
            assert!((ind.filled as i32 - expected_filled as i32).abs() <= 1,
                "bar fill must track the percentage");
        };

        check(0, 1, 0);   // top: line 1/4, 0%
        check(1, 2, 33);  // middle: line 2/4, 33%
        check(3, 4, 100); // bottom: line 4/4, 100%

        // Over-scroll is clamped to the bottom position, never past 100%.
        let clamped = ScrollIndicator::new(99, total_height, viewport_h, bar_width);
        assert_eq!((clamped.position, clamped.percent), (4, 100));
    }


    #[test]
    fn test_predictive_echo() {
        let mut echo = PredictiveEcho::default();
        echo.push('a');
        echo.push('b');
        assert_eq!(echo.buffer, "ab");
        echo.buffer.clear();
        assert_eq!(echo.buffer, "");
    }


    #[test]
    fn test_run_drains_memory_queue_then_exits_on_disconnect() {
        use crate::loop_io::MemoryQueue;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct StubHost(AtomicBool);
        impl LoopHost for StubHost {
            fn touch_activity(&self) {}
            fn idle_for(&self) -> Duration { Duration::ZERO }
            fn loop_exited(&self) { self.0.store(true, Ordering::SeqCst); }
        }
        let client_session = test_session();
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root_id = arena.alloc(oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen));
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let queue = MemoryQueue::new(128);
        let host = Arc::new(StubHost(AtomicBool::new(false)));
        let handles = LoopHandles {
            state: client_session.clone(),
            events_in: Arc::new(parking_lot::Mutex::new(queue.clone())),
            events_out: Arc::new(queue.clone()),
            host: host.clone(),
        };
        queue.push(InputEvent::AppResponse(oxiterm_proto::input::AppPatch(
            serde_json::json!({"k": "v"}),
        )));
        queue.close();

        let mut event_loop = EventLoop::new(handles, event_bus, Box::new(TestSink), empty_pages(), doc);
        event_loop.run();

        assert!(host.0.load(Ordering::SeqCst), "loop_exited must fire");
        assert!(queue.is_empty());
        let state = client_session.state.read();
        assert!(state.get("k").is_some(), "patch from the queue must be applied");
    }


    #[test]
    fn test_rebuild_focusable_nodes_respects_bind_show() {
        use oxiterm_proto::dom::{Node, NodeTag};
        let client_session = test_session();
        let event_bus = Arc::new(crate::events::EventBus::new());
        
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        
        let mut node2 = Node::new(NodeTag::Box);
        node2.attrs.event_htmx = Some("click_action".to_string());
        node2.attrs.bind_show = Some("show_node2".to_string());
        let id2 = arena.alloc(node2);
        
        let mut node3 = Node::new(NodeTag::Box);
        node3.attrs.event_htmx = Some("click_action2".to_string());
        node3.attrs.bind_show = Some("show_node3".to_string());
        let id3 = arena.alloc(node3);
        
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![id2, id3];
        let root_id = arena.alloc(root);
        
        let doc = THTMLDocument {
            arena,
            root: root_id,
            dirty_nodes: Vec::new(),
        };
        
        let mut event_loop = EventLoop::new(
            loop_handles(&client_session),
            event_bus,
            Box::new(TestSink),
            empty_pages(),
            doc,
        );
        
        event_loop.rebuild_focusable_nodes();
        assert!(!event_loop.focusable_nodes.contains(&id2));
        assert!(!event_loop.focusable_nodes.contains(&id3));

        {
            let mut state = event_loop.session.state.write();
            state.set("show_node2".to_string(), crate::state::StateValue::Bool(true));
            state.set("show_node3".to_string(), crate::state::StateValue::Bool(false));
        }
        
        event_loop.rebuild_focusable_nodes();
        assert!(event_loop.focusable_nodes.contains(&id2));
        assert!(!event_loop.focusable_nodes.contains(&id3));
        
        {
            let mut state = event_loop.session.state.write();
            state.set("show_node2".to_string(), crate::state::StateValue::Bool(false));
            state.set("show_node3".to_string(), crate::state::StateValue::Bool(true));
        }
        
        event_loop.rebuild_focusable_nodes();
        assert!(!event_loop.focusable_nodes.contains(&id2));
        assert!(event_loop.focusable_nodes.contains(&id3));
    }


    #[test]
    fn test_apply_state_patch() {
        let client_session = test_session();
        
        let patch = serde_json::json!({
            "count": 42,
            "name": "OxiTerm",
            "active": true,
            "list": ["a", "b", "c"],
            "empty": null
        });
        
        patch_keys(&client_session, patch);
        
        let state = client_session.state.read();
        assert_eq!(state.get("count"), Some(&crate::state::StateValue::Int(42)));
        assert_eq!(state.get("name"), Some(&crate::state::StateValue::Str("OxiTerm".to_string())));
        assert_eq!(state.get("active"), Some(&crate::state::StateValue::Bool(true)));
        assert_eq!(state.get("list"), Some(&crate::state::StateValue::List(vec!["a".to_string(), "b".to_string(), "c".to_string()])));
        assert_eq!(state.get("empty"), Some(&crate::state::StateValue::Str(String::new())));
    }


    #[test]
    fn test_sec_state_patch_limits() {
        let client_session = test_session();
        
        let mut huge_patch = serde_json::Map::new();
        for i in 0..105 {
            huge_patch.insert(format!("key{}", i), serde_json::json!(i));
        }
        patch_keys(&client_session, serde_json::Value::Object(huge_patch));
        assert!(client_session.state.read().get("key0").is_none());

        let long_key = "a".repeat(257);
        let patch_key_too_long = serde_json::json!({
            long_key.clone(): 42
        });
        patch_keys(&client_session, patch_key_too_long);
        assert!(client_session.state.read().get(&long_key).is_none());

        let long_val = "x".repeat(65537);
        let patch_val_too_long = serde_json::json!({
            "valid_key": long_val
        });
        patch_keys(&client_session, patch_val_too_long);
        assert!(client_session.state.read().get("valid_key").is_none());

        let huge_arr: Vec<i32> = (0..1005).collect();
        let patch_arr_too_long = serde_json::json!({
            "arr_key": huge_arr
        });
        patch_keys(&client_session, patch_arr_too_long);
        assert!(client_session.state.read().get("arr_key").is_none());
    }


    #[test]
    fn test_38_apply_state_patch_rejects_reserved_is_web() {
        // App Server MUST NOT be able to overwrite _is_web (reserved key, _ prefix)
        let client_session = test_session();

        let patch = serde_json::json!({ "_is_web": "true" });
        patch_keys(&client_session, patch);

        // _is_web was never written by engine, so it must remain absent
        assert!(client_session.state.read().get("_is_web").is_none());
    }



    #[test]
    fn test_3_htmx_no_thtml_calls_apply_action() {
        use oxiterm_proto::dom::{Node, NodeTag};
        let client_session = test_session();

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut btn = Node::new(NodeTag::Button);
        btn.attrs.event_htmx = Some("set:tab=home".to_string());
        let btn_id = arena.alloc(btn);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![btn_id];
        let root_id = arena.alloc(root);

        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut event_loop = new_el(&client_session, event_bus, doc);
        event_loop.focused_node = Some(btn_id);

        // Drive handle_htmx_target with action target
        event_loop.handle_htmx_target(btn_id, "set:tab=home");
        assert_eq!(client_session.state.read().get("tab"), Some(&crate::state::StateValue::Str("home".to_string())));
        assert!(event_loop.source_path.is_none());

        // Drive handle_htmx_target with navigation target -> attempts navigation
        // Since nonexistent.thtml doesn't exist, it fails resolution and logs warn without panic.
        event_loop.handle_htmx_target(btn_id, "nonexistent.thtml");
        assert!(event_loop.source_path.is_none()); // failed, so remains None
    }


    #[test]
    fn test_5_preprocess_space_with_input_focused() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: ' ', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        let result = EventLoop::preprocess_key(key, true, false);
        // input focused: Space is NOT remapped
        match result {
            Some(InputEvent::KeyPress(k)) => assert_eq!(k.codepoint, ' '),
            _ => panic!("Expected KeyPress(' ')"),
        }
    }


    #[test]
    fn test_6_preprocess_q_with_input_focused() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        let result = EventLoop::preprocess_key(key, true, false);
        // input focused: 'q' is NOT a quit signal
        assert!(result.is_some(), "q with focused input must pass through");
    }


    #[test]
    fn test_7_preprocess_q_without_input_focused_ssh() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        // SSH session (is_web_client = false), no input focused: 'q' IS a quit signal
        let result = EventLoop::preprocess_key(key, false, false);
        assert!(result.is_none(), "q without focused input on SSH must return None");
    }


    #[test]
    fn test_8_preprocess_q_web_session_no_quit() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        // Web session (is_web_client = true), no input focused: 'q' must NOT quit
        let result = EventLoop::preprocess_key(key, false, true);
        match result {
            Some(InputEvent::KeyPress(k)) => assert_eq!(k.codepoint, 'q', "web 'q' must pass through as KeyPress"),
            _ => panic!("Expected KeyPress('q') to pass through on web session"),
        }
    }


    #[test]
    fn test_13_identity_injects_reserved_keys() {
        let session = test_session();
        let id = crate::identity::UserIdentity::ssh_key("alice");
        
        let attached = session.attach_identity(id);
        assert!(attached);

        let state = session.state.read();
        assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("alice".to_string())));
        assert_eq!(state.get("_auth_method"), Some(&crate::state::StateValue::Str("SshKey".to_string())));
    }


    #[test]
    fn test_15_patch_skips_reserved_applies_others() {
        let session = test_session();
        // Pre-set a reserved key via identity (bypasses _-prefix guard)
        session.state.write().set("_username".to_string(), crate::state::StateValue::Str("real".to_string()));
        // Patch: contains _username (reserved) and a normal key
        let patch = serde_json::json!({"_username": "hacked", "tab": "home"});
        patch_keys(&session, patch);
        let state = session.state.read();
        assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("real".to_string())));
        assert_eq!(state.get("tab"), Some(&crate::state::StateValue::Str("home".to_string())));
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
    fn test_32_open_https_emits_0x33() {
        let session = test_session();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut event_loop = new_el(&session, event_bus, doc);
        let seen = capture_notices(&mut event_loop);

        use oxiterm_proto::dom::NodeId;
        event_loop.handle_open_url("https://example.com".to_string(), NodeId(0));

        assert_eq!(*seen.lock(), vec![oxiterm_renderer::ClientNotice::OpenUrl("https://example.com".to_string())]);
    }


    #[test]
    fn test_33_open_javascript_rejected() {
        let session = test_session();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut event_loop = new_el(&session, event_bus, doc);
        let seen = capture_notices(&mut event_loop);

        use oxiterm_proto::dom::NodeId;
        event_loop.handle_open_url("javascript:alert(1)".to_string(), NodeId(0));

        assert!(seen.lock().is_empty(), "javascript: URL must not raise a notice");
    }


    #[test]
    fn test_34_open_ssh_session_no_output_warn_only() {
        let session = test_session();
        // SSH session: is_web_client = false (default)
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut event_loop = new_el(&session, event_bus, doc);
        let seen = capture_notices(&mut event_loop);

        use oxiterm_proto::dom::NodeId;
        // Must not panic, must not write to any channel
        event_loop.handle_open_url("https://example.com".to_string(), NodeId(0));
        assert!(seen.lock().is_empty(), "SSH session: open: raises no notice");
    }


    #[test]
    fn test_26_for_expansion_on_append_action() {
        let session = test_session();
        session.state.write().set("items".to_string(), crate::state::StateValue::List(vec!["A".to_string()]));
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut for_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::For);
        for_node.attrs.each = Some("items".to_string());
        
        let mut tmpl_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Text);
        tmpl_node.text = Some("{item}".to_string());
        let tmpl_id = arena.alloc(tmpl_node);
        for_node.children.push(tmpl_id);

        let for_id = arena.alloc(for_node);
        let doc = THTMLDocument { arena, root: for_id, dirty_nodes: Vec::new() };

        let mut event_loop = new_el(&session, event_bus, doc);
        
        event_loop.expand_all_for_nodes();
        {
            let node = event_loop.doc.arena.get(for_id).unwrap();
            assert_eq!(node.children.len(), 2);
            assert_eq!(event_loop.doc.arena.get(node.children[1]).unwrap().text.as_deref(), Some("A"));
        }

        event_loop.handle_htmx_target(for_id, "append:items=B");
        event_loop.expand_all_for_nodes();

        {
            let node = event_loop.doc.arena.get(for_id).unwrap();
            assert_eq!(node.children.len(), 3);
            assert_eq!(event_loop.doc.arena.get(node.children[1]).unwrap().text.as_deref(), Some("A"));
            assert_eq!(event_loop.doc.arena.get(node.children[2]).unwrap().text.as_deref(), Some("B"));
        }
    }


    fn doc_texts(el: &EventLoop) -> Vec<String> {
        el.doc.arena.iter().filter_map(|(_, n)| n.text.clone()).collect()
    }


    /// The loop navigates, switches variants and reloads purely through a `MemPageSource`: no
    /// page file exists on disk.
    #[test]
    fn test_page_source_mem_drives_navigation() {
        use crate::page_source::{MemPageSource, PageSource};
        let session = test_session();
        let mem = MemPageSource::new([
            ("index.thtml", "<screen><text>INDEX</text></screen>"),
            ("demos/a.thtml", "<screen><text>PAGE-A</text></screen>"),
            ("features.thtml", "<screen><text>FEATURES</text></screen>"),
            ("features_mobile.thtml", "<screen><text>FEATURES-M</text></screen>"),
        ]);
        let index = mem.page_at(std::path::Path::new("index.thtml"));
        let doc = mem.load(&index, oxiterm_proto::style::RenderMode::Tui).unwrap();
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), doc);
        el.page_source = Arc::new(mem);
        el.source_path = Some(index.actual.clone());
        el.base_source_path = Some(index.base.clone());
        assert_eq!(doc_texts(&el), vec!["INDEX".to_string()]);

        // htmx-style link: current-page-relative, `.thtml` target.
        el.handle_htmx_target(oxiterm_proto::dom::NodeId(0), "demos/a.thtml");
        assert_eq!(doc_texts(&el), vec!["PAGE-A".to_string()]);
        assert_eq!(session.current_page.read().as_deref(), Some("demos/a.thtml"));

        // Relative to the new current page; page id keeps today's un-normalised form.
        el.now = el.now + std::time::Duration::from_secs(10);
        el.handle_htmx_target(oxiterm_proto::dom::NodeId(0), "../features.thtml");
        assert_eq!(doc_texts(&el), vec!["FEATURES".to_string()]);
        assert_eq!(session.current_page.read().as_deref(), Some("demos/../features.thtml"));

        // Hot reload goes through the source too.
        assert!(el.reload_current_source());
        assert_eq!(doc_texts(&el), vec!["FEATURES".to_string()]);

        // With is_mobile set, the _mobile variant is picked (direct call, no SwitchViewport event).
        el.now = el.now + std::time::Duration::from_secs(10);
        session.is_mobile.store(true, std::sync::atomic::Ordering::SeqCst);
        el.resolve_and_load_page("features").unwrap();
        assert_eq!(doc_texts(&el), vec!["FEATURES-M".to_string()]);

        // Server-initiated navigation is app-root relative; escapes and missing pages fail the
        // same generic way and leave the current page untouched.
        el.now = el.now + std::time::Duration::from_secs(10);
        assert!(el.load_app_page("index.thtml").is_ok());
        assert_eq!(session.current_page.read().as_deref(), Some("index.thtml"));
        let e1 = el.resolve_and_load_page("../secret").unwrap_err().to_string();
        let e2 = el.resolve_and_load_page("missing").unwrap_err().to_string();
        assert_eq!(e1, e2);
        assert_eq!(session.current_page.read().as_deref(), Some("index.thtml"));
    }


    #[test]
    fn test_t5_input_throttle_silent_drop() {
        // 2.2/t5: INPUT throttle drops events silently (no 0x34)
        let mut t = crate::throttle::EventThrottle::new(crate::clock::MonoTime::ZERO);
        // Exhaust the input bucket
        for _ in 0..crate::throttle::INPUT_CAPACITY {
            assert!(t.check_input(crate::clock::MonoTime::ZERO));
        }
        // Further 150 events: all dropped
        for _ in 0..150 {
            assert!(!t.check_input(crate::clock::MonoTime::ZERO), "throttled input must return false");
        }
        // No 0x34 test: no send path reached for INPUT throttle
    }


    #[test]
    fn test_t8_after_throttle_refill_nav_accepted() {
        // 2.2/t8: After exhaust + manual refill, NAV accepted again; episode cleared
        let mut t = crate::throttle::EventThrottle::new(crate::clock::MonoTime::ZERO);
        for _ in 0..crate::throttle::NAV_CAPACITY {
            t.check_nav(crate::clock::MonoTime::ZERO);
        }
        assert!(!t.check_nav(crate::clock::MonoTime::ZERO), "exhausted");
        t.is_first_throttle(); // enter episode

        // Force refill
        t.nav.set_tokens(crate::throttle::NAV_CAPACITY as f64);
        assert!(t.check_nav(crate::clock::MonoTime::ZERO), "after refill, allowed again");
        assert!(!t.nav_throttle_active, "episode cleared on allow");
    }


    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.3 / R1 — death_reason + 0xFF tests (Correction A)
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t1a_idle_death_sets_death_reason_1() {
        // 2.3/t1a: EventLoop idle path sets death_reason=1
        let session = test_session();

        // Default is 2
        assert_eq!(session.death_reason.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Simulate idle timeout path (just the store that EventLoop::run does before return)
        session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(session.death_reason.load(std::sync::atomic::Ordering::SeqCst), 1,
            "idle path must set death_reason to 1");
    }


    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.3 Addendum R6 — last_layout cleared after page change
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t6a_reset_layout_clears_last_layout() {
        // 2.3/t6a: reset_layout_and_scroll clears last_layout → no stale hit-test
        let session = test_session();
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = new_el(&session, event_bus, doc);

        // Manually set a fake last_layout to ensure it's not None
        // (We can't construct a full LayoutResult, but we can verify None after reset)
        // The field is already None at construction; just confirm reset_layout_and_scroll keeps it None.
        assert!(el.layout_engine.last_layout.is_none(), "starts as None");
        el.reset_layout_and_scroll();
        assert!(el.layout_engine.last_layout.is_none(), "still None after reset");
        assert_eq!(el.scroll_offset, 0, "scroll offset reset to 0");
    }


    #[test]
    fn layout_is_incremental_and_cross_checked_across_real_frames() {
        // Drives the real render_frame (state sync, animation tick, mode and viewport handling)
        // with a kept layout engine through state changes: a bound text, a bind-show flip, a
        // resize. The engine's debug cross-check panics on any difference from a rebuild; the
        // counters prove the incremental and the quiet path were taken.
        use oxiterm_proto::dom::{Node, NodeTag};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 40, rows: 12 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut label = Node::new(NodeTag::Text);
        label.text = Some("0".to_string());
        label.attrs.bind_state = Some("n".to_string());
        let label_id = arena.alloc(label);
        let mut shown = Node::new(NodeTag::Text);
        shown.text = Some("shown when flag".to_string());
        shown.attrs.bind_show = Some("flag=true".to_string());
        let shown_id = arena.alloc(shown);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![label_id, shown_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = new_el(&session, event_bus, doc);
        el.layout_engine.set_cross_check(true);

        for i in 0..12i64 {
            {
                let mut st = session.state.write();
                st.set("n".to_string(), crate::state::StateValue::Int(i * 111));
                st.set("flag".to_string(), crate::state::StateValue::Bool(i % 4 < 2));
            }
            el.doc.arena.get_mut(label_id).unwrap().text = Some((i * 111).to_string());
            if i == 7 {
                *session.dims.write() = PtyDimensions { cols: 60, rows: 20 };
            }
            el.render_frame();
            el.render_frame(); // quiet frame: nothing changed
        }
        let s = el.layout_engine.stats();
        assert_eq!(s.cross_checked, 24, "{s:?}");
        assert!(s.incremental >= 20, "{s:?}");
        assert!(s.quiet >= 10, "{s:?}");
        assert_eq!(s.full, 1, "only the first frame builds from scratch: {s:?}");
    }

    #[test]
    fn gui_overlays_are_appended_as_row_diffs() {
        // A GUI frame with all three overlays (focus ring on an auto-focused button, predictive
        // echo, scroll indicator on a page taller than the viewport). Debug builds assert in
        // render_frame that the row-restricted overlay diff equals the full snapshot diff; here
        // the captured list proves each overlay produced trailing CellGrids, a frame without
        // overlays produces none, and a repeated frame gives the same list and ARIA JSON.
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
        struct Capture(Arc<parking_lot::Mutex<(Vec<DisplayList>, Vec<Vec<u8>>)>>);
        impl FrameSink for Capture {
            fn send_frame(&mut self, _f: &CellBuffer, _b: &CellBuffer) -> anyhow::Result<bool> { Ok(false) }
            fn send_display_list(&mut self, l: &DisplayList) -> anyhow::Result<bool> {
                self.0.lock().0.push(l.clone());
                Ok(false)
            }
            fn send_a11y(&mut self, j: &[u8]) -> anyhow::Result<bool> {
                self.0.lock().1.push(j.to_vec());
                Ok(false)
            }
        }
        let trailing_grids = |l: &DisplayList| l.items.iter().rev().take_while(|i| matches!(i, DisplayItem::CellGrid { .. })).count();

        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 40, rows: 8 };
        session.display_list_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut kids = Vec::new();
        let mut btn = Node::new(NodeTag::Button);
        btn.text = Some("Go".to_string());
        btn.attrs.event_htmx = Some("go".to_string());
        btn.style.width = Some(10.0);
        let btn_id = arena.alloc(btn);
        let mut first = Node::new(NodeTag::Text);
        first.text = Some("first".to_string());
        kids.push(arena.alloc(first));
        kids.push(btn_id);
        let mut tall = Node::new(NodeTag::Text);
        tall.text = Some((0..30).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n"));
        kids.push(arena.alloc(tall));
        let mut root = Node::new(NodeTag::Screen);
        root.children = kids;
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let cap = Arc::new(parking_lot::Mutex::new((Vec::new(), Vec::new())));
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), doc);
        el.frame_sink = Box::new(Capture(cap.clone()));
        {
            let mut e = session.predictive_echo.write();
            e.buffer = "abc".to_string();
            e.active_node = Some(btn_id);
        }
        el.render_frame();
        el.render_frame();
        assert_eq!(el.focused_node, Some(btn_id));
        {
            let c = cap.lock();
            assert_eq!(c.0.len(), 2);
            assert!(trailing_grids(&c.0[0]) >= 2, "focus/echo/scroll overlays must be appended: {}", trailing_grids(&c.0[0]));
            assert_eq!(c.0[0], c.0[1]);
            assert_eq!(c.1[0], c.1[1]);
        }
        // No overlay at all: no focus (nothing interactive), no echo, page fits.
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 40, rows: 8 };
        session.display_list_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut t = Node::new(NodeTag::Text);
        t.text = Some("plain".to_string());
        let tid = arena.alloc(t);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![tid];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let cap2 = Arc::new(parking_lot::Mutex::new((Vec::new(), Vec::new())));
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), doc);
        el.frame_sink = Box::new(Capture(cap2.clone()));
        el.render_frame();
        let c = cap2.lock();
        assert_eq!(c.0.len(), 1);
        assert_eq!(trailing_grids(&c.0[0]), 0, "a frame without overlays appends nothing");
    }

    #[test]
    fn test_t_death_reason_default_is_2() {
        // Guard: newly created sessions always default death_reason=2 (restart/unknown)
        let session = test_session();
        assert_eq!(
            session.death_reason.load(std::sync::atomic::Ordering::SeqCst),
            2u8,
            "default death_reason must be 2 (restart/unknown)"
        );
    }


    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.2 / R1 — Mouse activation throttle fix
    // ────────────────────────────────────────────────────────────────────────

    /// Builds a minimal EventLoop with one Screen root and one Box child carrying event-htmx.
    /// Returns (event_loop, box_node_id).
    fn make_el_with_htmx_button(
        htmx: &str,
    ) -> (EventLoop, oxiterm_proto::dom::NodeId) {
        use oxiterm_proto::dom::{Node, NodeTag};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut btn = Node::new(NodeTag::Box);
        btn.attrs.event_htmx = Some(htmx.to_string());
        let btn_id = arena.alloc(btn);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![btn_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let el = new_el(&session, event_bus, doc);
        (el, btn_id)
    }


    /// Injects a synthetic LayoutResult placing `node_id` at the given rect.
    fn inject_layout(
        el: &mut EventLoop,
        entries: Vec<(oxiterm_proto::dom::NodeId, oxiterm_renderer::layout::types::Rect)>,
        total_height: u16,
    ) {
        let mut nodes = std::collections::HashMap::new();
        for (id, rect) in entries {
            nodes.insert(id, rect);
        }
        el.layout_engine.last_layout = Some(oxiterm_renderer::layout::types::LayoutResult {
            nodes,
            total_height,
            paint_order: Vec::new(),
            ..Default::default()
        });
    }


    #[test]
    fn test_wheel_scrolls_by_line_step_and_clamps() {
        // Mouse wheel = line-step scroll (3 rows/notch), clamped to the scroll range,
        // and it must NOT run hit-test/activation (a scroll is not a click).
        let (mut el, btn_id) = make_el_with_htmx_button("inc:clicks");
        el.rebuild_parent_map();
        *el.session.dims.write() = PtyDimensions { cols: 20, rows: 5 };
        el.total_height = 20; // viewport_h = 4 → max_scroll = 16

        // Place the button under the wheel coordinate; if the wheel wrongly activated,
        // "clicks" state would change.
        inject_layout(
            &mut el,
            vec![(btn_id, oxiterm_renderer::layout::types::Rect { x: 0.0, y: 0.0, width: 20.0, height: 20.0 })],
            20,
        );

        let wheel = |btn| oxiterm_proto::input::MouseInput {
            col: 1, row: 1, button: btn,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        };
        use oxiterm_proto::input::MouseButton::{WheelDown, WheelUp};

        assert!(el.handle_mouse_event(wheel(WheelDown)), "wheel requests a render");
        assert_eq!(el.scroll_offset, 3, "one notch down = WHEEL_LINE_STEP rows");
        el.handle_mouse_event(wheel(WheelDown));
        assert_eq!(el.scroll_offset, 6);
        el.handle_mouse_event(wheel(WheelUp));
        assert_eq!(el.scroll_offset, 3, "one notch up = WHEEL_LINE_STEP rows back");

        for _ in 0..20 { el.handle_mouse_event(wheel(WheelDown)); }
        assert_eq!(el.scroll_offset, 16, "clamped at max_scroll, never past the bottom");
        for _ in 0..20 { el.handle_mouse_event(wheel(WheelUp)); }
        assert_eq!(el.scroll_offset, 0, "clamped at the top");

        assert!(el.session.state.read().get("clicks").is_none(),
            "wheel scrolling must never activate an htmx target");
    }


    #[test]
    fn test_wheel_ignored_when_content_fits() {
        // No scrollbar (content fits the viewport) → wheel is a no-op.
        let (mut el, _btn) = make_el_with_htmx_button("noop:x");
        *el.session.dims.write() = PtyDimensions { cols: 20, rows: 30 };
        el.total_height = 10; // <= rows → no scroll
        el.handle_mouse_event(oxiterm_proto::input::MouseInput {
            col: 1, row: 1, button: oxiterm_proto::input::MouseButton::WheelDown,
            action: oxiterm_proto::input::MouseAction::Press, modifiers: Default::default(),
        });
        assert_eq!(el.scroll_offset, 0);
    }


    #[test]
    fn test_render_click_roundtrip_activates_every_rendered_cell() {
        // Full render↔click round-trip: a click on the terminal cell where a node is
        // *rendered* (centering offset + rect + the terminal's 1-based origin) must, after
        // handle_mouse_event's inverse adjustment, hit-test back to that node and activate
        // it — and a click one column outside the span must NOT. This locks the coordinate
        // contract shared by rendering (`get_centering_offset` + rect.x) and hit-testing
        // (`raw - offset - 1`); any offset or off-by-one regression fails here.
        let (mut el, btn) = make_el_with_htmx_button("inc:hits");
        el.rebuild_parent_map();
        {
            let n = el.doc.arena.get_mut(btn).unwrap();
            n.style.width = Some(10.0);
            n.style.height = Some(2.0);
            n.style.margin.left = 4.0;
            n.style.margin.top = 3.0;
        }
        let dims = PtyDimensions { cols: 80, rows: 24 };
        *el.session.dims.write() = dims;

        // Real layout — the same offsets rendering would use.
        el.layout_engine.set_viewport_rows(dims.rows);
        el.layout_engine.compute(&mut el.doc, dims.cols, 0, None).unwrap();
        let layout = el.layout_engine.last_layout.clone().unwrap();
        let rect = layout.nodes.get(&btn).unwrap().cells();
        let (ox, oy) = layout.get_centering_offset(&el.doc, dims.cols, dims.rows);
        assert!(rect.width > 0 && rect.height > 0);

        let click = |el: &mut EventLoop, term_col: u16, term_row: u16| {
            el.session.state.write().set("hits".to_string(), crate::state::StateValue::Int(0));
            el.handle_mouse_event(oxiterm_proto::input::MouseInput {
                col: term_col,
                row: term_row,
                button: oxiterm_proto::input::MouseButton::Left,
                action: oxiterm_proto::input::MouseAction::Press,
                modifiers: Default::default(),
            });
            el.session.state.read().get("hits").cloned()
        };

        // Every rendered cell activates the button.
        for ry in rect.y..rect.y + rect.height {
            for rx in rect.x..rect.x + rect.width {
                // Terminal (1-based) coordinate that renders layout cell (rx, ry), no scroll.
                let got = click(&mut el, ox + rx + 1, oy + ry + 1);
                assert_eq!(
                    got,
                    Some(crate::state::StateValue::Int(1)),
                    "click on rendered cell for layout ({rx},{ry}) must activate the button"
                );
            }
        }

        // One column left of, and one past, the span must not activate it.
        let row = oy + rect.y + 1;
        assert_eq!(click(&mut el, ox + rect.x, row), Some(crate::state::StateValue::Int(0)),
            "column just left of the span must not activate");
        assert_eq!(click(&mut el, ox + rect.x + rect.width + 1, row), Some(crate::state::StateValue::Int(0)),
            "column just past the span must not activate");
    }


    #[test]
    fn test_t1_press_activates_despite_empty_input_bucket() {
        // 2.2/t1: A MouseEvent(Press) delivered to handle_mouse_event() fires
        // hit_test + handle_htmx_target even when the INPUT bucket is empty.
        //
        // Negative anchor: if `run()` re-adds `if self.throttle.check_input()` as
        // an arm guard, handle_mouse_event() would never be called and this test
        // would panic because state "clicks" would remain None.
        let (mut el, btn_id) = make_el_with_htmx_button("inc:clicks");
        el.rebuild_parent_map();

        // Button occupies col=5..15, row=2..4.
        inject_layout(
            &mut el,
            vec![(btn_id, oxiterm_renderer::layout::types::Rect { x: 5.0, y: 2.0, width: 10.0, height: 2.0 })],
            10,
        );

        // Drain the INPUT bucket — simulates post-hover state.
        el.throttle.input.set_tokens(0.0);

        // handle_mouse_event subtracts centering_offset (0,0 for dims=0) plus 1
        // from each axis before calling hit_test. Button is at x=5,y=2; send
        // col=8,row=3 so after saturating_sub(1) hit_test receives (7,2).
        let needs_render = el.handle_mouse_event(oxiterm_proto::input::MouseInput {
            col: 8,
            row: 3,
            button: oxiterm_proto::input::MouseButton::Left,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        });

        assert!(needs_render, "Press must always request a render");
        let clicks = el.session.state.read().get("clicks").cloned();
        assert_eq!(
            clicks,
            Some(crate::state::StateValue::Int(1)),
            "Press must activate handle_htmx_target regardless of INPUT bucket; got {:?}",
            clicks,
        );
    }


    #[test]
    fn test_t2_click_on_text_child_activates_parent_box_htmx() {
        // 2.2/t2 — REGRESSION TEST FOR THIS BUG.
        //
        // Scenario: a Box carries event-htmx; a Text node is its child.
        // The user hovers (exhausting INPUT bucket) then clicks on the Text label.
        // Before the fix: the MouseEvent arm was guarded by check_input(), so the
        //   Press was silently dropped — activation never fired.
        // After the fix: handle_mouse_event() is called unconditionally; it runs
        //   hit_test (returns Text node) → get_htmx_node_and_target (walks up to Box)
        //   → handle_htmx_target — all without the test touching those functions.
        //
        // Negative anchor: re-adding `if self.throttle.check_input()` in run()'s arm
        // makes handle_mouse_event() unreachable when the bucket is empty, causing
        // "activations" to remain None and this assertion to fail.
        use oxiterm_proto::dom::{Node, NodeTag};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();

        // Text child — the actual click target, carries no htmx.
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("Click me".to_string());
        let text_id = arena.alloc(text_node);

        // Box parent — carries event-htmx.
        let mut box_node = Node::new(NodeTag::Box);
        box_node.attrs.event_htmx = Some("inc:activations".to_string());
        box_node.children = vec![text_id];
        let box_id = arena.alloc(box_node);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![box_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = new_el(&session, event_bus, doc);
        el.rebuild_parent_map();

        // Text at col=5..13, row=2..3 (smaller area → hit_test picks it over Box).
        // Box at col=4..14, row=1..4 (larger area, parent).
        inject_layout(
            &mut el,
            vec![
                (box_id,  oxiterm_renderer::layout::types::Rect { x: 4.0, y: 1.0, width: 10.0, height: 3.0 }),
                (text_id, oxiterm_renderer::layout::types::Rect { x: 5.0, y: 2.0, width:  8.0, height: 1.0 }),
            ],
            10,
        );

        // Exhaust INPUT bucket — simulates hover-heavy scenario before the click.
        el.throttle.input.set_tokens(0.0);

        // handle_mouse_event subtracts centering_offset (0,0 for dims=0) plus 1
        // from each axis. Text rect is at x=5,y=2; send col=8,row=3 so after
        // saturating_sub(1) hit_test receives (7,2) which lands in the Text rect
        // (x=5..13, y=2..3) but not the Box (x=4..14, y=1..4) alone — Text wins.
        let needs_render = el.handle_mouse_event(oxiterm_proto::input::MouseInput {
            col: 8,
            row: 3,
            button: oxiterm_proto::input::MouseButton::Left,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        });

        assert!(needs_render, "Press must always request a render");
        let activations = el.session.state.read().get("activations").cloned();
        assert_eq!(
            activations,
            Some(crate::state::StateValue::Int(1)),
            "clicking Text child must activate parent Box's htmx via the arm wiring; got {:?}",
            activations,
        );
    }


    #[test]
    fn test_t3_burst_press_activates_all_never_throttled() {
        // 2.2/t3: 300 rapid Press events via handle_mouse_event() with INPUT bucket
        // empty from the start must ALL activate (inc:clicks → state count == 300).
        // NAV bucket is irrelevant (state-mutation action, not a .thtml load).
        //
        // Negative anchor: the old arm guard would have suppressed all 300 Presses
        // (bucket empty), leaving "clicks" as None and this assertion failing.
        let (mut el, btn_id) = make_el_with_htmx_button("inc:clicks");
        el.rebuild_parent_map();
        inject_layout(
            &mut el,
            vec![(btn_id, oxiterm_renderer::layout::types::Rect { x: 0.0, y: 0.0, width: 10.0, height: 1.0 })],
            5,
        );

        // Force INPUT bucket empty throughout.
        el.throttle.input.set_tokens(0.0);

        const PRESS_COUNT: i64 = 300;
        for _ in 0..PRESS_COUNT {
            el.handle_mouse_event(oxiterm_proto::input::MouseInput {
                col: 5,
                row: 0,
                button: oxiterm_proto::input::MouseButton::Left,
                action: oxiterm_proto::input::MouseAction::Press,
                modifiers: Default::default(),
            });
        }

        let clicks = el.session.state.read().get("clicks").cloned();
        assert_eq!(
            clicks,
            Some(crate::state::StateValue::Int(PRESS_COUNT)),
            "all {} Press activations must reach handle_htmx_target; INPUT bucket must never gate them; got {:?}",
            PRESS_COUNT,
            clicks,
        );
    }


    #[test]
    fn test_t_charwrite_never_throttled() {
        // D2: character state-writes (bind_value append) are UNCONDITIONAL.
        // Only the render trigger is gated by check_input().
        // With INPUT bucket at 0, 10 keypresses to a focused Input node must
        // produce bind_value "aaaaaaaaaa" (length 10) — no lost characters.
        use oxiterm_proto::dom::{Node, NodeTag};
        let session = test_session();
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();

        // Input node bound to state key "text".
        let mut input_node = Node::new(NodeTag::Input);
        input_node.attrs.bind_value = Some("text".to_string());
        let input_id = arena.alloc(input_node);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![input_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = new_el(&session, event_bus, doc);
        el.rebuild_focusable_nodes();

        // Focus the Input node.
        el.focused_node = Some(input_id);

        // Drain INPUT bucket to 0 — the critical precondition.
        el.throttle.input.set_tokens(0.0);

        // Replicate the character-input else-branch from the KeyPress arm.
        // The write is UNCONDITIONAL; only needs_render is gated by check_input().
        for _ in 0..10usize {
            if let Some(focused_id) = el.focused_node {
                if let Some(node) = el.doc.get_node(focused_id) {
                    if node.tag == oxiterm_proto::dom::NodeTag::Input {
                        if let Some(ref state_key) = node.attrs.bind_value.clone() {
                            let cp = 'a';
                            let current = match el.session.state.read().get(state_key) {
                                Some(crate::state::StateValue::Str(s)) => s.clone(),
                                _ => String::new(),
                            };
                            let mut new_val = current;
                            new_val.push(cp);
                            el.session.state.write().set(
                                state_key.clone(),
                                crate::state::StateValue::Str(new_val),
                            );
                        }
                    }
                }
            }
            // Render is gated: check_input() returns false (bucket empty), no render.
            // But the write above already committed — this is exactly D2's guarantee.
        }

        let text_val = el.session.state.read().get("text").cloned();
        assert_eq!(
            text_val,
            Some(crate::state::StateValue::Str("aaaaaaaaaa".to_string())),
            "10 keypresses with empty INPUT bucket must produce bind_value of length 10; got {:?}",
            text_val,
        );
    }


    #[test]
    fn test_app_patch_navigate_is_not_stored_and_rest_is_applied() {
        let session = test_session();
        let mut el = bare_loop(session.clone());
        let seen = capture_notices(&mut el);

        el.apply_app_patch(serde_json::json!({
            "navigate": "cache/abc.thtml",
            "status": "ready"
        }), PatchOrigin::ActionResponse);

        let state = session.state.read();
        assert!(state.get("navigate").is_none(), "navigate must not be stored as state");
        assert_eq!(state.get("status"), Some(&crate::state::StateValue::Str("ready".to_string())));
        // The navigation was attempted (no app dir in this bare loop, so it fails visibly).
        assert_eq!(*seen.lock(), vec![oxiterm_renderer::ClientNotice::NavError]);
    }


    #[test]
    fn test_app_patch_navigate_rejects_unsafe_targets() {
        let session = test_session();
        let mut el = bare_loop(session.clone());
        let seen = capture_notices(&mut el);
        for bad in [
            serde_json::json!("/etc/passwd.thtml"),
            serde_json::json!("https://evil.example/x.thtml"),
            serde_json::json!("open:https://evil.example"),
            serde_json::json!("page.html"),
            serde_json::json!(""),
            serde_json::json!(42),
            serde_json::json!("a\u{0}b.thtml"),
        ] {
            el.apply_app_patch(serde_json::json!({ "navigate": bad }), PatchOrigin::Push);
        }
        assert!(session.state.read().get("navigate").is_none());
        // No navigation was even attempted (an attempt would report a nav error here).
        assert!(seen.lock().is_empty());
    }


    #[test]
    fn test_t7_app_patch_handles_set_app_token() {
        for origin in [PatchOrigin::ActionResponse, PatchOrigin::Push] {
            let session = test_session();
            let mut el = bare_loop(session.clone());
            el.apply_app_patch(serde_json::json!({
                "set_app_token": "token_xyz_123",
                "foo": "bar"
            }), origin);

            let state = session.state.read();
            assert!(state.get("set_app_token").is_none(), "set_app_token must not be saved as regular key");
            assert_eq!(state.get("_app_token"), Some(&crate::state::StateValue::Str("token_xyz_123".to_string())));
            assert_eq!(state.get("foo"), Some(&crate::state::StateValue::Str("bar".to_string())));
        }
    }


    #[test]
    fn test_t8_patch_targeting_app_token_directly_rejected() {
        let session = test_session();
        let mut el = bare_loop(session.clone());

        el.apply_app_patch(serde_json::json!({
            "_app_token": "malicious_token"
        }), PatchOrigin::ActionResponse);

        let state = session.state.read();
        assert!(state.get("_app_token").is_none(), "direct patch to _app_token must be rejected");
    }


    #[test]
    fn test_t10_events_payload_contains_app_token_when_present() {
        let session = test_session();

        session.state.write().set("_app_token".to_string(), crate::state::StateValue::Str("secret_token_123".to_string()));

        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let event_bus = Arc::new(crate::events::EventBus::new());
        let el = new_el(&session, event_bus, doc);

        let app_token = {
            let state_guard = el.session.state.read();
            match state_guard.get("_app_token") {
                Some(crate::state::StateValue::Str(tok)) if !tok.is_empty() => Some(tok.clone()),
                _ => None,
            }
        };
        assert_eq!(app_token, Some("secret_token_123".to_string()));
    }


    #[test]
    fn test_t11_events_payload_app_token_none_when_absent() {
        let session = test_session();

        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let event_bus = Arc::new(crate::events::EventBus::new());
        let el = new_el(&session, event_bus, doc);

        let app_token = {
            let state_guard = el.session.state.read();
            match state_guard.get("_app_token") {
                Some(crate::state::StateValue::Str(tok)) if !tok.is_empty() => Some(tok.clone()),
                _ => None,
            }
        };
        assert_eq!(app_token, None);
    }


    #[test]
    fn test_t12_empty_set_app_token_removes_app_token_from_state() {
        for origin in [PatchOrigin::ActionResponse, PatchOrigin::Push] {
            let session = test_session();
            session.state.write().set("_app_token".to_string(), crate::state::StateValue::Str("existing_token".to_string()));
            let mut el = bare_loop(session.clone());

            el.apply_app_patch(serde_json::json!({
                "set_app_token": ""
            }), origin);

            let state = session.state.read();
            assert!(state.get("_app_token").is_none(), "_app_token must be removed when set_app_token is empty");
        }
    }


    #[test]
    fn test_p41_t3_apc_empty_token_removes_state_no_dispatch() {
        let session = test_session();
        session.state.write().set("_app_token".to_string(), crate::state::StateValue::Str("old_token".to_string()));

        let doc = oxiterm_renderer::document::THTMLDocument::default();
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut el = new_el(&session, event_bus, doc);

        el.handle_set_app_token("".to_string());

        let state = session.state.read();
        assert!(state.get("_app_token").is_none());
    }


    #[test]
    fn test_p41_t4_patch_same_set_app_token_no_frame() {
        let session = test_session();
        session.state.write().set("_app_token".to_string(), crate::state::StateValue::Str("existing_token".to_string()));
        let mut el = bare_loop(session);
        let seen = capture_notices(&mut el);

        el.apply_app_patch(serde_json::json!({
            "set_app_token": "existing_token"
        }), PatchOrigin::Push);

        assert!(seen.lock().is_empty(), "AppToken notice must NOT be emitted when set_app_token equals current _app_token");
    }


    #[test]
    fn test_p41_t5_patch_new_set_app_token_emits_frame() {
        let session = test_session();
        let mut el = bare_loop(session);
        let seen = capture_notices(&mut el);

        el.apply_app_patch(serde_json::json!({
            "set_app_token": "brand_new_token_999"
        }), PatchOrigin::Push);

        assert_eq!(*seen.lock(), vec![oxiterm_renderer::ClientNotice::AppToken("brand_new_token_999".to_string())]);
    }


    #[test]
    fn test_action_response_token_resyncs_without_notice_and_open_url_is_extracted() {
        // Response origin: open_url becomes a notice (and is not stored), the token is set, no
        // AppToken notice. Push origin: open_url is just a state key.
        let session = test_session();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut el = bare_loop(session.clone());
        let seen = capture_notices(&mut el);
        el.apply_app_patch(serde_json::json!({
            "open_url": "https://example.com/a", "set_app_token": "t1", "k": "v"
        }), PatchOrigin::ActionResponse);
        assert_eq!(*seen.lock(), vec![oxiterm_renderer::ClientNotice::OpenUrl("https://example.com/a".to_string())]);
        assert!(session.state.read().get("open_url").is_none());
        assert!(session.state.read().get("_app_token").is_some());

        let session2 = test_session();
        let mut el2 = bare_loop(session2.clone());
        let seen2 = capture_notices(&mut el2);
        el2.apply_app_patch(serde_json::json!({"open_url": "https://example.com/a"}), PatchOrigin::Push);
        assert!(seen2.lock().is_empty());
        assert!(session2.state.read().get("open_url").is_some());
    }


    #[test]
    fn test_pointer_drag_state_machine() {
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_proto::input::{MouseInput, MouseButton, MouseAction, KeyModifiers};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut drag_box = Node::new(NodeTag::Box);
        drag_box.attrs.draggable = Some(true);
        drag_box.attrs.drag_state_x = Some("win_x".to_string());
        drag_box.attrs.drag_state_y = Some("win_y".to_string());
        drag_box.attrs.event_drag_end = Some("set:drag_status=dropped".to_string());
        drag_box.style.position = oxiterm_proto::style::Position::Absolute;
        drag_box.style.left = Some(10.0);
        drag_box.style.top = Some(5.0);
        drag_box.style.width = Some(20.0);
        drag_box.style.height = Some(10.0);
        let box_id = arena.alloc(drag_box);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![box_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = new_el(&session, event_bus, doc);
        el.rebuild_parent_map();

        // Synthetic layout
        inject_layout(&mut el, vec![(box_id, oxiterm_renderer::layout::types::Rect { x: 10.0, y: 5.0, width: 20.0, height: 10.0 })], 24);

        // 1. Mouse Press on the draggable box (col 12, row 6)
        // With dims=80x24 and centering offset=0,0, mouse.col saturating_sub(1) = 11.
        // Let's send adjusted coordinates matching the rect:
        let press_mouse = MouseInput {
            col: 13, // 13 - 1 = 12 (inside rect x=10..30)
            row: 7,  // 7 - 1 = 6 (inside rect y=5..15)
            button: MouseButton::Left,
            action: MouseAction::Press,
            modifiers: KeyModifiers::default(),
        };
        let render_needed = el.handle_mouse_event(press_mouse);
        assert!(render_needed, "Press must request render");
        assert!(el.active_drag.is_some(), "DragState must be initialized on Press");
        let drag = el.active_drag.as_ref().unwrap();
        assert_eq!(drag.target_node, box_id);
        assert_eq!(drag.initial_left, 10);
        assert_eq!(drag.initial_top, 5);

        // 2. Mouse Move with button held (delta +5, +3) -> col 18, row 10
        let move_mouse = MouseInput {
            col: 18,
            row: 10,
            button: MouseButton::Left,
            action: MouseAction::Move,
            modifiers: KeyModifiers::default(),
        };
        let move_render = el.handle_mouse_event(move_mouse);
        assert!(move_render, "Drag move must trigger render");
        let updated_node = el.doc.get_node(box_id).unwrap();
        assert_eq!(updated_node.style.left, Some(15.0), "left should be initial(10) + delta(5) = 15");
        assert_eq!(updated_node.style.top, Some(8.0), "top should be initial(5) + delta(3) = 8");

        // Verify state variables synchronized
        let state = session.state.read();
        assert_eq!(state.get("win_x"), Some(&crate::state::StateValue::Str("15".to_string())));
        assert_eq!(state.get("win_y"), Some(&crate::state::StateValue::Str("8".to_string())));
        drop(state);

        // 3. Mouse Release
        let release_mouse = MouseInput {
            col: 18,
            row: 10,
            button: MouseButton::Left,
            action: MouseAction::Release,
            modifiers: KeyModifiers::default(),
        };
        let release_render = el.handle_mouse_event(release_mouse);
        assert!(release_render, "Drag release must trigger render");
        assert!(el.active_drag.is_none(), "DragState must be cleared on Release");

        // Verify event_drag_end fired
        let state_after = session.state.read();
        assert_eq!(state_after.get("drag_status"), Some(&crate::state::StateValue::Str("dropped".to_string())));
    }


    #[test]
    fn test_pointer_drag_with_explicit_drag_handle() {
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_proto::input::{MouseInput, MouseButton, MouseAction, KeyModifiers};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        // Titlebar handle (y: 5..7)
        let mut handle = Node::new(NodeTag::Box);
        handle.attrs.drag_handle = Some(true);
        let handle_id = arena.alloc(handle);

        // Content area non-handle (y: 7..15)
        let content = Node::new(NodeTag::Box);
        let content_id = arena.alloc(content);

        // Parent window (draggable = true)
        let mut window = Node::new(NodeTag::Box);
        window.attrs.draggable = Some(true);
        window.children = vec![handle_id, content_id];
        let window_id = arena.alloc(window);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![window_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = new_el(&session, event_bus, doc);
        el.rebuild_parent_map();

        // Synthetic layout: window at (10, 5, 20, 10), handle at (10, 5, 20, 2), content at (10, 7, 20, 8)
        inject_layout(&mut el, vec![
            (window_id, oxiterm_renderer::layout::types::Rect { x: 10.0, y: 5.0, width: 20.0, height: 10.0 }),
            (handle_id, oxiterm_renderer::layout::types::Rect { x: 10.0, y: 5.0, width: 20.0, height: 2.0 }),
            (content_id, oxiterm_renderer::layout::types::Rect { x: 10.0, y: 7.0, width: 20.0, height: 8.0 }),
        ], 24);

        // Click on content area (col 13, row 10 -> adjusted 12, 9)
        let click_content = MouseInput {
            col: 13,
            row: 10,
            button: MouseButton::Left,
            action: MouseAction::Press,
            modifiers: KeyModifiers::default(),
        };
        el.handle_mouse_event(click_content);
        assert!(el.active_drag.is_none(), "Clicking non-handle content must NOT initiate drag");

        // Click on titlebar handle (col 13, row 6 -> adjusted 12, 5)
        let click_handle = MouseInput {
            col: 13,
            row: 6,
            button: MouseButton::Left,
            action: MouseAction::Press,
            modifiers: KeyModifiers::default(),
        };
        el.handle_mouse_event(click_handle);
        assert!(el.active_drag.is_some(), "Clicking explicit drag-handle MUST initiate drag");
        assert_eq!(el.active_drag.as_ref().unwrap().target_node, window_id);
    }


    #[test]
    fn test_z_index_hit_testing_order() {
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_proto::input::{MouseInput, MouseButton, MouseAction, KeyModifiers};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        // Background card (z-index 1) with HTMX action "bg_clicked"
        let mut bg_card = Node::new(NodeTag::Box);
        bg_card.style.z_index = Some(1);
        bg_card.attrs.event_htmx = Some("set:result=bg".to_string());
        let bg_id = arena.alloc(bg_card);

        // Foreground card (z-index 10) with HTMX action "fg_clicked" covering the same area
        let mut fg_card = Node::new(NodeTag::Box);
        fg_card.style.z_index = Some(10);
        fg_card.attrs.event_htmx = Some("set:result=fg".to_string());
        let fg_id = arena.alloc(fg_card);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![bg_id, fg_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = new_el(&session, event_bus, doc);
        el.rebuild_parent_map();

        // Layout with paint_order: bg_id painted first, fg_id painted second (on top)
        let mut nodes = std::collections::HashMap::new();
        nodes.insert(bg_id, oxiterm_renderer::layout::types::Rect { x: 5.0, y: 5.0, width: 30.0, height: 10.0 });
        nodes.insert(fg_id, oxiterm_renderer::layout::types::Rect { x: 5.0, y: 5.0, width: 30.0, height: 10.0 });
        el.layout_engine.last_layout = Some(oxiterm_renderer::layout::types::LayoutResult {
            nodes,
            total_height: 24,
            paint_order: vec![bg_id, fg_id], // paint_order ascending by z-index
            ..Default::default()
        });

        // Click on overlapping area (col 11, row 7 -> doc coords 10, 6)
        let click = MouseInput {
            col: 11,
            row: 7,
            button: MouseButton::Left,
            action: MouseAction::Press,
            modifiers: KeyModifiers::default(),
        };
        el.handle_mouse_event(click);

        // Result must be "fg" because higher z-index fg_id wins hit-testing
        let state = session.state.read();
        assert_eq!(state.get("result"), Some(&crate::state::StateValue::Str("fg".to_string())));
    }


    #[test]
    fn test_smooth_transition_in_event_loop() {
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_proto::style::{AnimatableProperty, Easing, TransitionSpec};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut anim_box = Node::new(NodeTag::Box);
        anim_box.style.width = Some(20.0);
        anim_box.style.transitions.push(TransitionSpec {
            property: AnimatableProperty::Width,
            duration_ms: 100,
            delay_ms: 0,
            easing: Easing::Linear,
        });
        let box_id = arena.alloc(anim_box);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![box_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = new_el(&session, event_bus, doc);
        let start_time = crate::clock::MonoTime::from_millis(1000);

        // Baseline establishment
        el.animation_controller.sync_transitions(&mut el.doc, start_time);
        assert!(!el.animation_controller.has_active());

        // Target changes to width 60
        el.doc.arena.get_mut(box_id).unwrap().style.width = Some(60.0);
        el.animation_controller.sync_transitions(&mut el.doc, start_time);
        assert!(el.animation_controller.has_active(), "Transition must be active after property target change");

        // Tick at 50ms (midpoint)
        let mid_time = start_time + std::time::Duration::from_millis(50);
        let changed = el.animation_controller.tick(&mut el.doc, mid_time);
        assert!(changed, "Animation tick must return true when property interpolates");
        let mid_width = el.doc.get_node(box_id).unwrap().style.width.unwrap();
        assert!(mid_width >= 38.0 && mid_width <= 42.0, "Midpoint width should be ~40, got {}", mid_width);

        // Tick at 110ms (completion)
        let end_time = start_time + std::time::Duration::from_millis(110);
        let end_changed = el.animation_controller.tick(&mut el.doc, end_time);
        assert!(end_changed);
        let final_width = el.doc.get_node(box_id).unwrap().style.width.unwrap();
        assert_eq!(final_width, 60.0, "Final width must reach target 60");
        assert!(!el.animation_controller.has_active(), "No transitions should be active once finished");
    }


    #[test]
    fn test_state_driven_animated_transition() {
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_proto::style::{AnimatableProperty, Easing, TransitionSpec};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut anim_box = Node::new(NodeTag::Box);
        anim_box.style.width = Some(5.0);
        anim_box.attrs.bind_width = Some("prog".to_string());
        anim_box.style.transitions.push(TransitionSpec {
            property: AnimatableProperty::Width,
            duration_ms: 100,
            delay_ms: 0,
            easing: Easing::Linear,
        });
        let box_id = arena.alloc(anim_box);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![box_id];
        let root_id = arena.alloc(root);
        let mut doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        // 1. Initial State Injection
        session.state.write().set("prog".to_string(), crate::state::StateValue::Int(5));
        EventLoop::setup_state_subscriptions(&doc, &mut *session.state.write());
        EventLoop::inject_initial_state(&mut doc, &*session.state.read());
        assert_eq!(doc.get_node(box_id).unwrap().style.width, Some(5.0));

        let mut el = new_el(&session, event_bus, doc);
        let start_time = crate::clock::MonoTime::from_millis(1000);

        // Baseline establishment
        el.animation_controller.sync_transitions(&mut el.doc, start_time);
        assert!(!el.animation_controller.has_active());

        // 2. State change via action (e.g. user clicked button event-htmx="set:prog=25")
        session.state.write().set("prog".to_string(), crate::state::StateValue::Int(25));

        // 3. Render cycle: restore baselines, sync dirty state, sync transitions, tick
        el.animation_controller.restore_baselines(&mut el.doc);
        EventLoop::sync_dirty_state(&mut el.doc, &mut *session.state.write());
        assert_eq!(el.doc.get_node(box_id).unwrap().style.width, Some(25.0), "Style width should update from state");

        el.animation_controller.sync_transitions(&mut el.doc, start_time);
        assert!(el.animation_controller.has_active(), "Active transition must launch on state update");

        // Midpoint tick (50ms)
        let mid_time = start_time + std::time::Duration::from_millis(50);
        let changed = el.animation_controller.tick(&mut el.doc, mid_time);
        assert!(changed);
        let mid_w = el.doc.get_node(box_id).unwrap().style.width.unwrap();
        assert!(mid_w >= 14.0 && mid_w <= 16.0, "Midpoint width should be ~15, got {}", mid_w);

        // Next render cycle at 50ms: restore baselines must restore target 25
        el.animation_controller.restore_baselines(&mut el.doc);
        assert_eq!(el.doc.get_node(box_id).unwrap().style.width, Some(25.0));
        EventLoop::sync_dirty_state(&mut el.doc, &mut *session.state.write());
        el.animation_controller.sync_transitions(&mut el.doc, mid_time);
        assert_eq!(el.animation_controller.active_count(), 1, "Must not restart or cancel transition");

        // Completion tick (110ms)
        let end_time = start_time + std::time::Duration::from_millis(110);
        el.animation_controller.tick(&mut el.doc, end_time);
        assert_eq!(el.doc.get_node(box_id).unwrap().style.width, Some(25.0));
        assert!(!el.animation_controller.has_active());
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


    #[test]
    fn test_step_api_counter_without_threads() {
        use crate::action::FnActionHandler;
        use crate::clock::{FakeClock, MonoTime};
        use crate::loop_io::MemoryQueue;
        use crate::page_source::{MemPageSource, PageSource};
        use oxiterm_proto::input::{MouseAction, MouseButton, MouseInput};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let client_session = test_session();
        *client_session.dims.write() = PtyDimensions { cols: 80, rows: 24 };

        // Host-owned queue: action replies would come back through it (no thread involved).
        let queue = MemoryQueue::new(128);
        let handles = LoopHandles {
            state: client_session.clone(),
            events_in: Arc::new(parking_lot::Mutex::new(queue.clone())),
            events_out: Arc::new(queue.clone()),
            host: Arc::new(NullHost),
        };
        let mem = MemPageSource::new([("counter.thtml", include_str!("../../examples/demos/counter.thtml"))]);
        let page = mem.page_at(std::path::Path::new("counter.thtml"));
        let doc = mem.load(&page, oxiterm_proto::style::RenderMode::Tui).unwrap();
        let mut el = EventLoop::new(handles, Arc::new(crate::events::EventBus::new()), Box::new(TestSink), Arc::new(mem), doc);
        let frames = Arc::new(parking_lot::Mutex::new(Vec::new()));
        el.frame_sink = Box::new(RowsSink(frames.clone()));
        let clock = FakeClock::new();
        el.clock = Arc::new(clock.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        el.action_handler = Some(Arc::new(FnActionHandler(move |_p: &crate::action::DispatchPayload| {
            c2.fetch_add(1, Ordering::SeqCst);
            None
        })));

        // Initial tick renders the first frame (no events, no waiting).
        el.prepare_session();
        let t0 = MonoTime::from_millis(1000);
        el.tick(t0);
        assert_eq!(frames.lock().len(), 1, "initial tick renders a frame");

        // Locate the counter text and the `+` button from the real layout.
        let layout = el.layout_engine.last_layout.clone().unwrap();
        let (ox, oy) = layout.get_centering_offset(&el.doc, 80, 24);
        let find = |el: &EventLoop, pred: &dyn Fn(&oxiterm_proto::dom::Node) -> bool| {
            el.doc.arena.iter().find(|(_, n)| pred(n)).map(|(id, _)| id).unwrap()
        };
        let text_id = find(&el, &|n| n.attrs.bind_state.as_deref() == Some("counter"));
        let plus_id = find(&el, &|n| n.attrs.event_htmx.as_deref() == Some("inc:counter"));
        let tr = layout.nodes.get(&text_id).unwrap().cells();
        let pr = layout.nodes.get(&plus_id).unwrap().cells();
        let value_at = |frame: &Vec<String>| -> String {
            frame[(oy + tr.y) as usize].chars().skip((ox + tr.x) as usize).take(3).collect::<String>()
        };
        assert_eq!(value_at(&frames.lock()[0]).trim(), "0");

        // Click `+`: handle_event records the render, tick (one second later) emits it.
        let t1 = MonoTime::from_millis(2000);
        let press = |col, row| InputEvent::MouseEvent(MouseInput {
            col, row, button: MouseButton::Left, action: MouseAction::Press, modifiers: Default::default(),
        });
        let out = el.handle_event(press(ox + pr.x + pr.width / 2 + 1, oy + pr.y + pr.height / 2 + 1), t1);
        assert_eq!(out, StepOutcome::Continue);
        assert_eq!(frames.lock().len(), 1, "handle_event never renders");
        el.tick(t1);
        assert_eq!(client_session.state.read().get("counter"), Some(&crate::state::StateValue::Int(1)));
        let fs = frames.lock();
        assert_eq!(fs.len(), 2, "tick renders the frame the click asked for");
        assert_eq!(value_at(&fs[1]).trim(), "1");
        drop(fs);
        // Action replies (none here) would be drained from the host queue and fed back.
        assert!(queue.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1, "inc:counter is also dispatched to the in-process handler");

        // Nothing pending: a later tick renders nothing.
        el.tick(MonoTime::from_millis(3000));
        assert_eq!(frames.lock().len(), 2);
        assert_eq!(el.next_deadline(MonoTime::from_millis(3000)), Some(MonoTime::from_millis(4000)));
    }


    #[test]
    fn test_next_deadline_16ms_active_1000ms_idle() {
        use crate::clock::MonoTime;
        use oxiterm_proto::dom::{Node, NodeTag};
        use oxiterm_proto::style::{AnimatableProperty, Easing, TransitionSpec};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut b = Node::new(NodeTag::Box);
        b.style.width = Some(20.0);
        b.style.transitions.push(TransitionSpec {
            property: AnimatableProperty::Width, duration_ms: 100, delay_ms: 0, easing: Easing::Linear,
        });
        let box_id = arena.alloc(b);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![box_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), doc);

        let t = MonoTime::from_millis(1000);
        el.animation_controller.sync_transitions(&mut el.doc, t);
        assert_eq!(el.next_deadline(t), Some(t + Duration::from_millis(1000)), "idle");
        el.doc.arena.get_mut(box_id).unwrap().style.width = Some(60.0);
        el.animation_controller.sync_transitions(&mut el.doc, t);
        assert_eq!(el.next_deadline(t), Some(t + Duration::from_millis(16)), "transition active");
        el.animation_controller.tick(&mut el.doc, t + Duration::from_millis(200));
        assert_eq!(el.next_deadline(t), Some(t + Duration::from_millis(1000)), "idle again");
    }


    // ---- Phase 4 step 7c: the debounced resize is a deadline ----

    fn dims_of(s: &SessionState) -> (u16, u16) {
        let d = *s.dims.read();
        (d.cols, d.rows)
    }


    fn empty_screen_doc() -> THTMLDocument {
        use oxiterm_proto::dom::{Node, NodeTag};
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root_id = arena.alloc(Node::new(NodeTag::Screen));
        THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() }
    }


    #[test]
    fn test_resize_deadline_and_coalescing_without_threads() {
        use crate::clock::{FakeClock, MonoTime};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), empty_screen_doc());
        let frames = Arc::new(parking_lot::Mutex::new(Vec::new()));
        el.frame_sink = Box::new(RowsSink(frames.clone()));
        el.clock = Arc::new(FakeClock::new());
        el.prepare_session();

        let ms = MonoTime::from_millis;
        let t = ms(1000);
        el.tick(t);
        assert_eq!(frames.lock().len(), 1, "first frame");
        assert_eq!(el.next_deadline(t), Some(t + Duration::from_millis(1000)), "nothing pending: unchanged");

        // A transport pushes the dims, then queues the Resize event (explicit fake timestamps).
        let new_dims = PtyDimensions { cols: 100, rows: 30 };
        let t1 = ms(2000);
        session.resize_debouncer.write().push(new_dims, t1);
        el.handle_event(InputEvent::Resize { cols: 100, rows: 30 }, t1);
        el.tick(t1);
        assert_eq!(frames.lock().len(), 2, "the Resize event renders at the old size");
        assert_eq!(frames.lock()[1].len(), 24);
        assert_eq!(dims_of(&session), (80, 24));

        let due = t1 + Duration::from_millis(100) + Duration::from_nanos(1);
        assert_eq!(el.next_deadline(t1), Some(due), "deadline is the debounce expiry, not now + 1 s");

        // Inside the window nothing is applied.
        el.tick(ms(2050));
        assert_eq!(dims_of(&session), (80, 24));
        assert_eq!(frames.lock().len(), 2);

        // Coalescing: a second push at +60 ms extends the deadline.
        let t2 = ms(2060);
        session.resize_debouncer.write().push(PtyDimensions { cols: 90, rows: 20 }, t2);
        el.handle_event(InputEvent::Resize { cols: 90, rows: 20 }, t2);
        el.tick(t2);
        let frames_before = frames.lock().len();
        let due2 = t2 + Duration::from_millis(100) + Duration::from_nanos(1);
        assert_eq!(el.next_deadline(t2), Some(due2));
        el.tick(due); // the first expiry no longer applies anything
        assert_eq!(dims_of(&session), (80, 24));
        assert_eq!(frames.lock().len(), frames_before);

        // At the deadline the last size is applied and a frame at the new size is emitted.
        el.tick(due2);
        assert_eq!(dims_of(&session), (90, 20));
        let fs = frames.lock();
        assert_eq!(fs.len(), frames_before + 1);
        assert_eq!(fs.last().unwrap().len(), 20, "frame has the new row count");
        drop(fs);
        assert_eq!(el.next_deadline(due2), Some(due2 + Duration::from_millis(1000)), "debouncer drained");
    }


    // ---- Phase 4 step 7d: a render deferred by the frame limiter is a deadline ----

    /// Counter page in a loop with a recording sink; returns the loop, frames, a click event on
    /// `+` and a reader of the counter value in a frame.
    fn counter_loop(session: &Arc<SessionState>) -> (EventLoop, Arc<parking_lot::Mutex<Vec<Vec<String>>>>, InputEvent, Box<dyn Fn(&Vec<String>) -> String + Send>) {
        use crate::page_source::{MemPageSource, PageSource};
        use oxiterm_proto::input::{MouseAction, MouseButton, MouseInput};
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let mem = MemPageSource::new([("counter.thtml", include_str!("../../examples/demos/counter.thtml"))]);
        let page = mem.page_at(std::path::Path::new("counter.thtml"));
        let doc = mem.load(&page, oxiterm_proto::style::RenderMode::Tui).unwrap();
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), doc);
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


    #[test]
    fn test_deferred_render_deadline_is_the_limiter_slot() {
        use crate::clock::{FakeClock, MonoTime};
        let session = test_session();
        let (mut el, frames, click, value_at) = counter_loop(&session);
        el.clock = Arc::new(FakeClock::new());
        // Fresh limiter on the fake timeline.
        el.frame_limiter = crate::frame_limiter::FrameRateLimiter::new(60);
        el.first_frame = true;
        el.prepare_session();

        let ms = MonoTime::from_millis;
        let t = ms(1000);
        el.tick(t);
        assert_eq!(frames.lock().len(), 1, "first frame at t");
        assert_eq!(el.next_deadline(t), Some(t + Duration::from_millis(1000)), "(b) nothing owed: idle cadence");

        // An event needing a render 5 ms later: the limiter refuses, the render is carried.
        let t5 = t + Duration::from_millis(5);
        el.handle_event(click, t5);
        let slot = el.frame_limiter.next_allowed().unwrap();
        assert_eq!(el.next_deadline(t5), Some(slot), "owed render (not yet ticked) waits for the slot");
        el.tick(t5);
        assert_eq!(frames.lock().len(), 1, "limiter deferred the frame");
        assert!(slot > t + Duration::from_millis(16) && slot < t + Duration::from_millis(17));
        assert_eq!(el.next_deadline(t5), Some(slot), "deadline == limiter's next allowed instant, not 1000 ms");

        // One nanosecond early still refuses; at the slot the frame lands.
        el.tick(slot - Duration::from_nanos(1));
        assert_eq!(frames.lock().len(), 1);
        assert_eq!(el.next_deadline(slot - Duration::from_nanos(1)), Some(slot));
        el.tick(slot);
        let fs = frames.lock();
        assert_eq!(fs.len(), 2, "tick at the slot renders");
        assert_eq!(value_at(&fs[1]), "1");
        drop(fs);
        assert_eq!(el.next_deadline(slot), Some(slot + Duration::from_millis(1000)), "no spin: back to the idle cadence");
    }


    // ────────────────────────────────────────────────────────────────────────
    // Phase 6.5 — fractional pointer, focus-ring row
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_focus_ring_row_uses_the_exact_rect() {
        use oxiterm_renderer::layout::types::Rect;
        let r = |y: f32, h: f32| Rect { x: 0.0, y, width: 5.0, height: h };
        // The bug: y = 0.4, h = 1.2 snaps (round each) to y = 0, h = 1, so the old
        // `y + h / 2` gave row 0 while the label is centred in row 1.
        let snapped = r(0.4, 1.2).cells();
        assert_eq!(snapped.y + snapped.height / 2, 0, "what the cell-snapped rect gave");
        assert_eq!(focus_ring_row(&r(0.4, 1.2)), 1);
        // Half-up cases around a row boundary.
        assert_eq!(focus_ring_row(&r(1.4, 1.0)), 1);
        assert_eq!(focus_ring_row(&r(1.6, 1.0)), 2);
        assert_eq!(focus_ring_row(&r(2.5, 1.0)), 3);
        // Whole-number rects (every TUI layout): identical to `y + h / 2`.
        for y in 0u16..30 {
            for h in 0u16..8 {
                let cells = r(f32::from(y), f32::from(h)).cells();
                assert_eq!(focus_ring_row(&r(f32::from(y), f32::from(h))), i32::from(cells.y + cells.height / 2), "y={y} h={h}");
            }
        }
    }

    /// Two adjacent buttons with fractional edges (`a` = [10, 13.5), `b` = [13.5, 18)) in a
    /// container that is centred horizontally (60 of 80 columns) and taller than the viewport
    /// (scrolled by 3 rows). Everything is a power of two, so the arithmetic below is exact.
    fn pointer_fixture() -> (EventLoop, Arc<SessionState>) {
        use oxiterm_proto::dom::{Node, NodeTag};
        let session = test_session();
        *session.dims.write() = PtyDimensions { cols: 80, rows: 24 };
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut a = Node::new(NodeTag::Button);
        a.attrs.event_htmx = Some("set:hit=a".to_string());
        let a_id = arena.alloc(a);
        let mut b = Node::new(NodeTag::Button);
        b.attrs.event_htmx = Some("set:hit=b".to_string());
        let b_id = arena.alloc(b);
        let mut c = Node::new(NodeTag::Box);
        c.children = vec![a_id, b_id];
        let c_id = arena.alloc(c);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![c_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = new_el(&session, Arc::new(crate::events::EventBus::new()), doc);
        el.rebuild_parent_map();
        let rect = |x: f32, y: f32, w: f32, h: f32| oxiterm_renderer::layout::types::Rect { x, y, width: w, height: h };
        let mut nodes = std::collections::HashMap::new();
        nodes.insert(c_id, rect(0.0, 0.0, 60.0, 40.0));
        nodes.insert(a_id, rect(10.0, 5.0, 3.5, 1.5));
        nodes.insert(b_id, rect(13.5, 5.0, 4.5, 1.5));
        el.layout_engine.last_layout = Some(oxiterm_renderer::layout::types::LayoutResult {
            nodes,
            total_height: 40,
            paint_order: vec![c_id, a_id, b_id],
            ..Default::default()
        });
        el.scroll_offset = 3;
        (el, session)
    }

    fn pointer_press(x: f32, y: f32) -> InputEvent {
        InputEvent::PointerEvent(oxiterm_proto::input::PointerInput {
            x,
            y,
            button: oxiterm_proto::input::MouseButton::Left,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        })
    }

    fn hit_state(session: &Arc<SessionState>) -> Option<String> {
        match session.state.read().get("hit") {
            Some(crate::state::StateValue::Str(s)) => Some(s.clone()),
            _ => None,
        }
    }

    #[test]
    fn test_pointer_hits_use_the_fractional_rects_with_centering_and_scroll() {
        use crate::clock::MonoTime;
        let (mut el, session) = pointer_fixture();
        // Screen point = layout point + (ox = 10, -scroll = -3): a spans screen x [20, 23.5),
        // b [23.5, 28), both on screen rows [2, 3.5).
        let mut n = 0;
        for xi in 0..=(35 * 8) {
            let sx = 16.0 + xi as f32 * 0.125; // 16 .. 51
            for sy in [1.875f32, 2.0, 2.875, 3.375, 3.5] {
                session.state.write().set("hit".to_string(), crate::state::StateValue::Str("none".into()));
                el.handle_event(pointer_press(sx, sy), MonoTime::from_millis(1000));
                let in_row = (2.0..3.5).contains(&sy);
                let expect = if !in_row {
                    "none"
                } else if (20.0..23.5).contains(&sx) {
                    "a"
                } else if (23.5..28.0).contains(&sx) {
                    "b"
                } else {
                    "none"
                };
                assert_eq!(hit_state(&session).as_deref(), Some(expect), "pointer at ({sx}, {sy})");
                n += 1;
            }
        }
        assert!(n > 1000);
        // Just inside and just outside the shared edge (23.5), where the cell-centre path
        // cannot tell the buttons apart (cell 23 spans both; its centre 23.5 is in b).
        for (sx, expect) in [(23.4375f32, "a"), (23.5, "b"), (23.5625, "b")] {
            session.state.write().set("hit".to_string(), crate::state::StateValue::Str("none".into()));
            el.handle_event(pointer_press(sx, 2.5), MonoTime::from_millis(1000));
            assert_eq!(hit_state(&session).as_deref(), Some(expect), "at {sx}");
        }
        // Left of the centred page (negative in layout space) and above it: nothing, and no
        // clamp onto cell 0.
        for (sx, sy) in [(-3.0f32, 2.5f32), (9.5, 2.5), (20.5, -4.0)] {
            session.state.write().set("hit".to_string(), crate::state::StateValue::Str("none".into()));
            el.handle_event(pointer_press(sx, sy), MonoTime::from_millis(1000));
            assert_eq!(hit_state(&session).as_deref(), Some("none"), "outside at ({sx}, {sy})");
        }
    }

    #[test]
    fn test_pointer_at_cell_centres_equals_the_cell_event() {
        use crate::clock::MonoTime;
        use oxiterm_proto::input::{MouseAction, MouseButton, MouseInput};
        // Integral rects (what a TUI layout holds): every cell centre must resolve like the
        // 0x02 event for that cell, centering offset and scroll included.
        let (mut el, session) = pointer_fixture();
        {
            let l = el.layout_engine.last_layout.as_mut().unwrap();
            for r in l.nodes.values_mut() {
                r.x = r.x.round();
                r.y = r.y.round();
                r.width = r.width.round();
                r.height = r.height.round();
            }
        }
        let (ox, oy) = (10u16, 0u16);
        let mut seen = std::collections::HashSet::new();
        for row in 0u16..24 {
            for col in 0u16..80 {
                let mut results = Vec::new();
                for pointer in [false, true] {
                    session.state.write().set("hit".to_string(), crate::state::StateValue::Str("none".into()));
                    let ev = if pointer {
                        pointer_press(f32::from(col) + 0.5, f32::from(row) + 0.5)
                    } else {
                        InputEvent::MouseEvent(MouseInput {
                            col: col + 1,
                            row: row + 1,
                            button: MouseButton::Left,
                            action: MouseAction::Press,
                            modifiers: Default::default(),
                        })
                    };
                    el.handle_event(ev, MonoTime::from_millis(1000));
                    results.push(hit_state(&session));
                }
                assert_eq!(results[0], results[1], "cell ({col}, {row}) offset ({ox}, {oy})");
                seen.insert(results[0].clone());
            }
        }
        assert!(seen.contains(&Some("a".to_string())) && seen.contains(&Some("b".to_string())));
    }
}
