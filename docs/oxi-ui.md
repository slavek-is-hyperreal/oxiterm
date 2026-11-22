# 🪟 OxiUI: One THTML/TCSS Source → Terminal, Web, Desktop, Mobile

This document is the implementation plan for evolving **OxiTerm** into **OxiUI**: a rendering engine in which the existing terminal renderer becomes *one backend among several* (TUI, web canvas, native GPU window, mobile), all driven by the same THTML/TCSS documents, the same session/state machinery, and the same App Server protocol.

> **Status:** Phases 0–2 and 3a–3c done; 4–7 planned.

---

## 1. Goals and Non-Goals

**Goals**
- **One language.** THTML/TCSS stays the only markup/style format. No separate "UIhtml/UIcss". Backend-specific styling goes through `@media (render: tui|gui)` and silent-ignore of unsupported properties.
- **Zero regression for TUI.** Every existing app renders byte-identically over SSH and WebSocket after each phase.
- **Keep the server and protocol.** `oxiterm-server` (sessions, dispatcher, state, animations, `/events`, `/sessions/{id}/patch`) is not rewritten.
- **Lightweight GUI target.** Native window via `winit` + `wgpu`/Vello, aiming for a < 15–25 MB RSS footprint.
- **Two execution modes, same app source** (see §3.2):
  - **Server-driven:** the server renders, and a thin client (web, phone, desktop) only paints the DisplayList it receives over WS. This is the current OxiTerm model extended to pixels, comparable to LiveView for native apps.
  - **Local core:** parser, layout, session loop and renderer are compiled into the app (desktop binary, Android, WASM) and work offline. This is the Flutter-like model.

**Non-Goals (for now)**
- Full W3C HTML/CSS compatibility or a JS engine.
- Competing with Flutter on mobile polish (IME, accessibility, gesture physics) before the desktop/web path is solid.
- Proportional text before phase 6 (see §4).

---

## 2. Current Coupling Map (what is tied to the cell grid)

| Location | Coupling | Phase that changes it |
|---|---|---|
| `oxiterm-renderer/src/render/renderer.rs` (`render_node`, `render_single_node`, `safe_set`) | Paints directly into `CellBuffer` cell by cell; borders are `border.chars` glyphs | 1 |
| `oxiterm-renderer/src/render/buffer.rs` (`Cell`, `CellBuffer`, `DoubleBuffer`) | The frame representation | 1 (becomes a *sink*) |
| `oxiterm-renderer/src/render/diff.rs` (`DiffEngine`, `AnsiCommand`, `encode_binary`) | Cell-diff wire format for SSH and WS | 3 (new message type alongside) |
| `oxiterm-renderer/src/render/emitter.rs` (`send_frame(front, back)`) | Emitter trait takes `CellBuffer`s | 3 |
| `oxiterm-renderer/src/layout/types.rs` (`Rect { x,y,w,h: u16 }`, `HitTester::find_node(col,row)`) | Integer cell geometry | 2 |
| `oxiterm-renderer/src/layout/engine.rs` | Feeds Taffy `f32` but sourced from `u16` styles; viewport = `cols` | 2 |
| `oxiterm-proto/src/style.rs` (`width/height: Option<u16>`, edges `u16`) | TCSS lengths are integer cells | 2 |
| `oxiterm-server/src/animation.rs` (lines ~245–263) | Interpolates in `f32` but rounds back to `u16`/`i16` cells | 2 |
| `oxiterm-server/src/events.rs:59,84` | Hit-testing by `(col,row)` | 2 |
| `oxiterm-server/src/session.rs:481,582,775,1836,1933` | Owns `DoubleBuffer`, calls `render_node`, sends diffs | 3 |
| `oxiterm-server/src/web.rs:150,283–300` | Encodes cell diffs for WS | 3 |
| `oxiterm-web/src/lib.rs` (`draw_commands`, `draw_cell`) | Canvas client paints cells | 3 |
| `oxiterm-a11y/src/lib.rs` | Builds an a11y tree from the DOM; `--a11y` linear text mode works; AT-SPI D-Bus transport is a skeleton | 3 (web ARIA mirror), 5 (AccessKit) |

Already backend-agnostic: `oxiterm-proto` DOM and input decoder, THTML/TCSS parsers, the cascade, session state, bindings, the dispatcher, and App Server integration.

---

## 3. Target Architecture

```
THTML + TCSS ──► parser ──► DOM arena + cascade ──► Taffy layout (f32, unit-resolved)
                                                          │
                                                   DisplayList builder
                                                          │
                        ┌─────────────────────────────────┼──────────────────────────────┐
                        ▼                                 ▼                              ▼
                 CellSink (TUI)                    Wire encoder (web)              GpuSink (native)
           DisplayList → CellBuffer          DisplayList → compressed msg      winit + wgpu / Vello
           → DiffEngine → ANSI / WS diff     → oxiterm-web canvas painter      (desktop, later Android)
```

### 3.1 DisplayList (core abstraction)

A flat, paint-ordered list of primitives in `f32` logical units:

```rust
pub enum DisplayItem {
    Rect      { node: NodeId, rect: RectF, bg: Color },
    Border    { node: NodeId, rect: RectF, fg: Color, bg: Color, style: BorderStyle, chars: BorderChars, radius: f32 },
    TextRun   { node: NodeId, origin: PointF, text: String, fg: Color, bg: Color, mods: Modifiers },
    PushClip  { rect: RectF },
    PopClip,
    Image     { node: NodeId, rect: RectF, asset: AssetRef },   // phase 3+
    Legacy    { node: NodeId },                                   // phase 1: img/video/diagram painted by the old path
}
```

Requirements the cell code gets "for free" today and the DisplayList must carry explicitly:
- **Paint order and z-index**: built from `LayoutResult.paint_order`, including `Position::Fixed` offsets.
- **Clipping**: an explicit push/pop pair. Today `safe_set` only clamps to the buffer edge.
- **Border glyph sets**: `Border` carries `chars`, so the TUI sink reproduces `┌─┐` / `╔═╗` / rounded exactly. GUI sinks use `style`/`radius` instead.
- **Node ids on every item**: for GPU hit-testing and a11y.

### 3.2 Two execution modes

```
                    ┌──────────────────────── oxiterm-core (no tokio, no network) ────────────────────────┐
                    │ DOM + cascade + layout + DisplayList + EventLoop + StateManager + animations        │
                    │ ActionHandler trait  ·  FrameSink trait  ·  FontBundle                                │
                    └───────────────▲─────────────────────────────────────────────▲─────────────────────────┘
                                    │                                             │
      SERVER-DRIVEN MODE            │                       LOCAL MODE            │
      oxiterm-server                │                       oxiterm-gui / Android / WASM app
      ├─ SSH  → CellSink → ANSI     │                       ├─ GpuSink (winit + wgpu)
      ├─ WS   → CellSink → diff     │                       ├─ input from winit/touch → EventLoop
      └─ WS   → DisplayList msg ──► thin client paints      └─ ActionHandler:
         ActionHandler = HTTP to App Server (ureq)              ├─ in-process Rust callbacks (offline)
                                                                └─ HTTP to App Server (same protocol)
```

- **One core, two hosts.** `EventLoop` already runs on a std thread with `std::sync::mpsc`. The tokio and network coupling is at the edges: `ClientSession.web_frame_tx`, the SSH reactor, and the `ureq` POST in `dispatcher.rs`. Extracting the loop into `oxiterm-core` behind two traits makes both modes the same code:
  - `FrameSink` (already exists in `oxiterm-renderer`) receives frames: cells for TUI, DisplayList for GUI.
  - `ActionHandler` replaces the direct `ureq` call. In server-driven mode it is the HTTP App Server client. In local mode it is either in-process Rust (offline apps) or the same HTTP client.
- **App source is identical in both modes.** The same `.thtml`/`.tcss` and the same App Server contract (`/events`, state patches). An app can start server-driven and later ship as a local binary without changes to its markup.
- **Text and hit-testing are consistent in both modes (resolves the old D1).**
  - Both modes use the same layout code and the same **bundled font** (`FontBundle`, loaded by `cosmic-text`/`parley`), so a document lays out identically wherever it runs.
  - In server-driven mode the DisplayList carries **already shaped, positioned glyph runs**. The thin client never re-measures text. It draws glyphs at the coordinates it was given, and sends pointer events back in those coordinates. The server hit-tests on the layout it actually sent, so there is no font-metric drift between server and client.
  - In local mode layout, painting and hit-testing happen in one process, so drift is impossible by construction.
- **Thin client = local core minus the session.** A thin client is the GPU/canvas sink plus input forwarding. It reuses `GpuSink` and the web painter from the local build, so the two modes share their whole rendering stack.

---

## 4. Open Decisions (need the owner's call)

**D1 / D2: Server-side vs client-side layout, mobile model. Decided: both.**
The owner chose to support both execution modes. §3.2 describes how: a shared `oxiterm-core`, a bundled font, and positioned glyph runs in server-driven mode. Remaining sub-choices are left to implementation:
- Phases 1–3 stay **monospace** (coordinates `f32` in `ch`/`lh`). Proportional text arrives in phase 6 for both modes at once.
- Which font ships in `FontBundle` by default (one monospace + one proportional family, chosen for small size and wide Unicode coverage).
- Thin-client distribution: web first (no install), then the same thin client as a mode of the native/Android app.

**D3: GPU rasterizer.** Vello needs compute shaders (modern GPUs). A custom instanced-quad renderer (GL 3.3 / GLES 3 / Vulkan) covers old hardware such as GT 220 and the OxiDE retro targets. These may end up as two sinks.

---

## 5. Phased Plan

### Phase 0: Golden snapshot harness (safety net first) — ✅ done

Implemented in `oxiterm-renderer/tests/golden_cells.rs`, with 72 baselines in `oxiterm-renderer/tests/golden/` (36 pages × 2 sizes). Rendering is static, as in the web playground: no state injection, so `bind-show` nodes are visible. Regenerate with `OXI_BLESS=1 cargo test -p oxiterm-renderer --test golden_cells`. Stale baselines for deleted pages also fail the test.

Original scope:
- Add a test (e.g. `oxiterm-renderer/tests/golden_cells.rs`) that renders every `examples/**/*.thtml` (36 files today) at **80×24** and **120×40**.
- Dump each `CellBuffer` to a stable text form (char, fg, bg, bold/underline/italic, skip) under `oxiterm-renderer/tests/golden/`.
- Use an env var (e.g. `OXI_BLESS=1`) to regenerate. Normal runs diff against the committed baseline.
- Media nodes (img/video) use the web-mode placeholder path so the snapshots are deterministic.
- **Done when:** the baseline is committed and `cargo test` passes on it.

### Phase 1: DisplayList inside the renderer (no API change) — ✅ done

Implemented as `render/display_list.rs` (types + normative sink semantics), `render/build.rs` (DOM + layout → DisplayList; Fill, Border and Text items, with positioned glyphs for text, button and input) and `render/cell_sink.rs` (DisplayList → `CellBuffer`). Img, video and diagram still go through `DisplayItem::Legacy`. All 72 golden snapshots are unchanged.

Original scope:
- Add `render/display_list.rs` (types), `render/build.rs` (DOM + layout → DisplayList), and `render/cell_sink.rs` (DisplayList → `CellBuffer`).
- `Renderer::render_node(...)` keeps its **exact signature**. Internally it calls build, then `CellSink`. `session.rs`, `web.rs`, `events.rs` and a11y do not change.
- Migrate only background, border and text (incl. wrap, wide chars, password masking, button text). `img`, `video` and `diagram` stay on the old code via `DisplayItem::Legacy`.
- **Done when:** the golden snapshots and all existing tests pass with zero diffs.

### Phase 2: Units and float geometry — ✅ done

Implemented with these deviations from the original scope:
- Lengths are `f32` in cell units, not a `Length { value, unit }` type. `ch`/`lh` mean cells; `px`/`dp` convert through a provisional 8×16 px reference cell until phase 6.
- `RenderMode { Tui, Gui }` lives in `oxiterm-proto`; `LayoutEngine::with_mode` selects it.
- TUI snapping happens only in the layout engine (`snap`/`snap_size` and rect flattening); Taffy's own rounding is disabled in GUI mode.
- `@media (render: tui|gui)` is applied at parse time via `THTMLParser::parse_with_mode`; `parse` = TUI. `border-radius` is parsed into `ComputedStyle.border_radius` (cells; TUI ignores it); `box-shadow`, `font-family`, `font-size` are ignored silently.
- `LayoutResult.total_height` stays `u16`, because scrolling is still cell-based.
- During transitions, layout now reruns every animation tick because values change below one cell; empty diffs are still not sent.

Original scope:
- TCSS lengths become `Length { value: f32, unit: Ch | Lh | Px | Dp | Percent }`. A bare integer stays `ch` horizontally and `lh` vertically, so existing TCSS is unchanged.
- `Rect` becomes `f32` (or a parallel `RectF`). The TUI sink snaps to cells with one documented rounding rule.
- `HitTester` takes `f32` points. TUI passes `(col + 0.5, row + 0.5)`.
- `animation.rs` stops rounding to cells in the style. Rounding happens only in the TUI sink, which gives GUI sub-cell smoothness for free.
- Add the `@media (render: tui|gui)` parser and evaluation, plus silent-ignore for GUI-only properties (`border-radius`, `box-shadow`, `font-family`, `font-size`).
- **Done when:** golden snapshots are unchanged, and `@media` tests cover both render modes.

### Phase 3: DisplayList over the wire → web canvas becomes real GUI
- New negotiated WS message type: a full DisplayList per frame, serialized and compressed with `flate2` (already a dependency). The cell-diff stream stays as the fallback and default.
- Generalize the `emitter.rs` `send_frame` trait into a frame enum (`Cells(front, back)` | `Display(list)`).
- `oxiterm-web`: add `WebCanvas::draw_display_list` (Canvas2D fill rects, rounded borders, `fillText` with the monospace font, images as positioned DOM/`drawImage` from asset paths).
- Images in GUI are asset references, as in today's web mode, never Kitty/Sixel pixels.
- Damage tracking (rect diffing between lists) is deferred. Measure first.
- **Accessibility:** a `<canvas>` is invisible to screen readers. Mirror `oxiterm_a11y::build_a11y_tree` into hidden, positioned DOM elements with ARIA roles and labels over the canvas, the same way media overlays are positioned. Every DisplayList item carries its `node`, so items map directly to a11y nodes.
- **Done when:** the Spotify demo and `examples/index.thtml` work in the browser in GUI mode with clicks hitting the right targets, and the TUI and the old web client are unaffected.

Phase 3 is the first **server-driven** GUI: the browser is the thin client.

**Phase 3a — ✅ done.** The browser opts in with `?gui=1`. The client then sends `0x12` (capabilities) on every connection, and the server answers with `0x36` + zlib(DisplayList) frames instead of cell diffs. The wire format is versioned (`render/wire.rs`, see ARCHITECTURE.md §7b). Img/video/diagram and session overlays (focus ring, echo, scroll bar) travel as `CellGrid` items. The browser draws borders as vector lines (rounded and double sets included). Geometry is still TUI-snapped and the opt-in is per connection.

**Remaining:**
- **3b (done):**
  - Display-list sessions run layout in GUI mode (fractional geometry).
  - Wire v2 adds `radius` to Fill/Border and a general `Path` item.
  - TCSS `border-radius` (GUI-only) feeds `radius`; the cell sink ignores both.
  - The web client paints rounded fills and borders (ring background clipped to the rounded silhouette) and Paths (fill and/or stroke).
  - Mouse input stays cell-based; sub-cell pointer precision is a later step.
  - Fixed after 3b:
    - **First GUI frame:** a cell frame was painted before the delayed switch to display lists. The handshake now accepts `0x12` before the first `0x10`, and the web client sends it first.
    - **Root height:** the root was `u16::MAX` tall, so root children stretched to 65535 rows. It is now content-sized with the viewport as its minimum height (`LayoutEngine::set_viewport_rows`).
  - Still open: a new size is applied about 1 s late, because the idle event loop polls the resize debouncer only after its 1 s receive timeout.
- **3c (done):** ARIA mirror for screen readers. The server sends the accessibility tree with layout geometry as JSON (`0x37`, only when it changes); the GUI client mirrors it into transparent, accessible DOM over the canvas, keeps focus on the canvas (`aria-activedescendant` + a live region) and turns assistive-tech clicks on mirrored buttons into mouse press/release. Password values are never sent. Any node with an `event-htmx` action is announced as a button (named by its text), and containers take no name from their own text (the parser stores raw markup such as comments there).

### Phase 4: Extract `oxiterm-core` (enables local mode)

**Goal.** Move the session logic (event loop, state, focus/scroll/drag, animations, `<for>` expansion) out of `oxiterm-server` into an `oxiterm-core` crate. The core must **run**, not just compile, in a desktop app, on Android and in `wasm32-unknown-unknown`. `oxiterm-server` becomes a thin host: SSH, WebSocket, HTTP App Server client, metrics.

#### What the code looks like today (surveyed before planning)

| Area | Finding | Consequence |
|---|---|---|
| `session.rs` (4365 lines; ~2225 production, ~2140 tests) | `EventLoop::run()` is one ~570-line blocking loop (`recv_timeout` on the event channel, then render) | Must be turned inside out into a step API |
| Clock | `Instant::now()` in session.rs (resize debouncer, `last_activity`, render), `animation.rs`, `ratelimit.rs`, `throttle.rs`, and the renderer's media `cache.rs` | `Instant::now()` panics on wasm32 at runtime; the core takes time from the host |
| Threads | `dispatcher.rs` spawns a thread per App Server call (ureq); renderer `cache.rs` spawns decode threads | No threads in the core; async work goes through host traits |
| `ClientSession` | Mixes pure session state with transport (`web_frame_tx` tokio sender, `raw_input_tx`/`ReactorMessage`, prometheus `metrics`, token, epoch, connection count) | Split into a pure half (core) and a transport half (server) |
| Direct transport writes in the loop | Nav error (0x34/0x35), throttle notice and `open_url` (0x33) write `web_frame_tx` directly | Route through the sink like display lists and the ARIA mirror |
| Event channel | `backpressure.rs` uses `tokio::sync::Notify` | The host owns the channel and pushes events into the core |
| Page loading | `loader.rs` + `pathsafe.rs` read the filesystem | A `PageSource` trait: filesystem on the server, embedded or virtual FS on wasm |
| Already pure | `state.rs`, `events.rs`, `expand.rs`, `placeholder.rs`, `pathsafe.rs` logic | Move as-is |
| Installed targets | `wasm32-unknown-unknown` and `aarch64-unknown-linux-gnu` are installed; `aarch64-linux-android` is **not** | The Android check needs `rustup target add aarch64-linux-android` (an rlib build needs no NDK linker) |

#### Target API (step-based, host-driven)

```rust
// In oxiterm-core. `now` is a monotonic time supplied by the host; the core never reads a clock.
impl Session {
    fn handle_event(&mut self, ev: InputEvent, now: Millis);   // input, resize, patches, navigation
    fn tick(&mut self, now: Millis);                            // animations, debounces; emits frames via sinks
    fn next_deadline(&self) -> Option<Millis>;                  // when tick() must run again (None = idle)
}
trait FrameSink    { /* cells, display list, a11y (exists) */ fn send_notice(&mut self, n: ClientNotice) -> Result<bool>; }
trait ActionHandler { fn dispatch(&self, payload: DispatchPayload); } // fire-and-forget; the patch comes back as an InputEvent
trait PageSource   { fn load(&self, rel_path: &str, mode: RenderMode) -> Result<THTMLDocument>; }
```

- **Server driver:** a thread that does `recv_timeout(next_deadline - now)` → `handle_event`/`tick`. That is today's loop, outside the core.
- **Wasm and native drivers:** call the same three methods from their own event loop (`requestAnimationFrame`, winit).
- **Side effect:** the debounced resize becomes a deadline instead of waiting for the 1 s idle receive timeout. This fixes the open ~1 s resize lag.

#### Step 0: event-loop trace harness (safety net first)

The golden and display-list equivalence tests call the renderer directly. They do **not** cover the event loop. Before any refactor:
- Drive the real `EventLoop` through scripted input on a few example pages: counter (clicks), todo (`<for>` lists), input (typing, password), navigation (page changes), plus one App Server action with a mocked server.
- Record everything that leaves the loop: cell frames, display-list bytes, ARIA JSON, notices and dispatched actions. Use a capturing sink and a capturing handler.
- Store the traces as committed baselines, and assert them unchanged after every step. Bless only by the lead, never by a subagent.
- Pages with timing-dependent output (transitions) join the harness right after step 1, once the clock is injectable (done: `transition-cell`).

**Status: implemented** in `oxiterm-server/src/event_loop_trace.rs` (test-only module), baselines in `oxiterm-server/tests/traces/*.txt`, fixture pages in `oxiterm-server/tests/traces/pages/`.
- Run: `cargo test -p oxiterm-server --lib event_loop_trace` (about 10 s). A mismatch names the baseline file, the step and the first differing line.
- Bless: `OXI_TRACE_BLESS=1 cargo test -p oxiterm-server --lib event_loop_trace`, then review the diff of `tests/traces/` and rerun WITHOUT `OXI_TRACE_BLESS` (the stale-baseline check is skipped while blessing). **Only the lead blesses; a subagent never does.** Adding a script means adding its name to the `TRACES` list; a stale or missing baseline fails `trace_baselines_have_no_stale_files`. Blessing never deletes: a baseline whose script was removed from `TRACES` must be deleted by hand.
- What a trace holds, per step: current page, notices since the previous step (semantic lines: `open_url`, `nav_error`, `nav_throttle`, `app_token`), actions sent to the mock App Server (action, `app_token`, bound state; no session id), watched state keys, and the final cell grid (glyphs, wide-glyph skips, style runs; "unchanged" when equal to the previous step). GUI runs add the final display list (one line per item) and the ARIA mirror (one line per node).
- Scripts: counter (mouse clicks, Tab + Enter), todo (a fixture page, because `demos/todo.thtml` has no `<for>`: `<for>` re-expansion via `StatePatched`; plus the shipped demo), input (plain + password input, backspace; the password never reaches a frame), the shipped input demo, navigation (clicks, `NavigateTo`, missing page, traversal), NAV throttle flood, and App Server round trips (patch, reserved key refused, `open_url`, `set_app_token`, token sync, HTTP push-patch path). Cell and GUI (display list) variants where meaningful.
- Quiescence, not frame recording: events are fed only 30 ms after the last sink call (so the frame limiter never defers a frame to the loop's 1 s idle timeout), then the harness waits for a complete new frame and a 100 ms window with no sink call, notice or App Server request. Floods settle for 1.4 s.
- Notices are recorded as semantic lines read from the `web_frame_tx` channel. When step 2 routes them through the sink, only the capture adapter changes; baselines stay valid. Likewise step 3 changes only how the mock's reply re-enters the loop.
- Known quirks the traces record (do not "fix" them while refactoring): `<for>` blocks are re-expanded only on `StatePatched`; `current_page` is not normalised (`demos/../features.thtml`); Enter on a button pushes `'\r'` into the predictive echo (shown as `\u{d}` in the grid); the `<for>` template node itself is rendered.
- Known race kept OUT of the traces: the dispatcher thread queues `SetAppToken` before it applies the patch, so the `app_token_sync` request may or may not see the patched state. The App Server script therefore patches no bound key together with `set_app_token`. Step 3 (responses as events) removes the race; then the script can be extended.
- Transitions: covered by the fake-clock trace `transition-cell` (step 1). Not covered: media animations (renderer cache clock, step 8).

#### Steps: one seam per step, in place inside `oxiterm-server`, trace harness green after each

1. **Clock injection.** Replace every `Instant::now()` in the loop, animation, rate limiter, throttle and debouncer with a `now` passed in. The server passes the real clock; tests pass a fake one.
   - **Done.** `oxiterm-server/src/clock.rs`: `MonoTime` (a `Duration` from a host-chosen origin, saturating arithmetic), `Clock` trait, `SystemClock` (the host clock: `Instant` at creation + elapsed; the only `Instant::now()` of the loop's time path) and `FakeClock` (shared handle, tests/harness). `ClientSession.clock` holds the host clock; `EventLoop.clock` defaults to it and `run()` reads it once per iteration, right after the blocking receive, into `EventLoop.now`. Throttle, `FrameRateLimiter`, `ResizeDebouncer` (`push(dims, now)` is stamped by the transports from `session.clock`), `AnimationController` (`start_transition` now uses the passed `now`) take `now` explicitly.
   - New trace `transition-cell` (fixture `tests/traces/pages/transition_bars.thtml`) runs on a `FakeClock`: ease-out and spring width transitions sampled at fixed fake times.
   - **Remaining clock reads:** `ClientSession.last_activity` (`Instant`, behind `touch_activity()` / `idle_for()`; idle reaper in web.rs; moves in step 5), session-token `SystemTime` and `drain_sessions` in the registry (transport), per-IP `RateLimiter` in `ratelimit.rs` (web transport; split from `FrameRateLimiter` in step 8), `SessionMetrics.connected_at`, and the renderer media cache (step 8).
   - **Fixed after Step 1:** the `StatePatched` handler and the render prelude now share `apply_state_to_doc` (restore baselines → sync dirty state → sync transitions), so patched animated properties animate exactly like clicks; trace `transition-patch-cell` covers it. Original finding: `StatePatched` runs `sync_dirty_state` before the render and consumes the dirty keys; the render then calls `restore_baselines`, which reverts the patched value, so on a node with a `transition` a pushed or App Server patch to a bound property (`bind-width` etc.) is lost visually, not just un-animated. Only event-htmx actions (dirty keys survive to the render) animate. The new trace therefore drives the transition with a button press.
   - Design note: `ClientSession.clock` exists because resize pushes come from SSH/WS threads that must stamp the same origin the loop polls with.
2. **Notices through the sink.** Add `FrameSink::send_notice(ClientNotice)` with `NavError`, `NavThrottle`, `OpenUrl` and `AppToken` (and any others found). The loop stops touching `web_frame_tx`.
3. **`ActionHandler` replaces `AppDispatcher`.** The HTTP implementation (thread + ureq) stays in the server. Responses re-enter as an event, e.g. `InputEvent::AppResponse(patch)`; they are no longer applied to `ClientSession` from a foreign thread. Add an in-process handler for tests.
4. **`PageSource`** wraps `loader` + `pathsafe`. The filesystem implementation lives in the server; an in-memory implementation is used for tests and wasm.
5. **Split `ClientSession`.**
   - Pure `SessionState`: dims, state manager, predictive echo, debouncer, mode flags, current page, app base dir, active media, identity as data.
   - Transport `ClientSession`: the rest, holding an `Arc<RwLock<SessionState>>` or similar.
   - `EventLoop` sees only the pure half.
6. **Event queue ownership.** The host owns the channel (tokio `Notify` stays in the server). The core receives events only through `handle_event`.
7. **Invert `run()`** into `handle_event` / `tick` / `next_deadline`. Split the ~570-line loop into named functions first, then move the blocking wait into a server-side driver. This is the riskiest step, so it is done last and in the smallest commits possible.
8. **Move into `oxiterm-core`.** Use `git mv` of the now-pure modules into a new crate. `oxiterm-server` depends on it. Renderer media-cache threads/clock get the same treatment (decode work via a host hook) or are feature-gated out of wasm builds.

#### Tests move with the code
About 2100 lines of `session.rs` tests use `EventLoop` fields and `pub(crate)` methods directly.
- **Logic tests** (keys, mouse, focus, scroll, state, `<for>`) move to `oxiterm-core`, rewritten against the step API where needed.
- **Transport tests** (WS/SSH handshakes, takeover, frames on the wire) stay in `oxiterm-server`.

This will be a large share of the step 8 diff and is budgeted as its own sub-step.

#### Done when
- A test in `oxiterm-core` scans its own `src/` and fails on `Instant::now`, `std::thread`, `recv_timeout`, `std::fs`, `tokio`, `ureq`, `prometheus` or `russh`.
- `oxiterm-core` builds for `wasm32-unknown-unknown` and `aarch64-linux-android` (after installing the target).
- An in-process test drives the step API on `examples/demos/counter.thtml` with an injected clock and an in-process `ActionHandler`. Clicking `+` sets the state to 1 and emits a new frame.
- The trace harness, golden snapshots, display-list equivalence and all server tests are unchanged.

#### Delegation
- Each step is a separate Sonnet task with a mandatory advisor call before the first edit and before reporting.
- The lead reviews and runs the gates between steps.
- Subagents may run **no git commands at all**, read-only ones included.
- Steps 0, 5 and 7 get extra lead review. Step 7 may be split further.

### Phase 5: Native window, both modes (`oxiterm-gui` crate)
- `winit` window + `wgpu` (or Vello, per D3), with a glyph atlas built only from the glyphs in use (`swash`/`fontdue`).
- **Local mode:** `oxiterm-core` runs in-process and paints to `GpuSink`. It works offline, and actions go to an in-process or HTTP `ActionHandler`.
- **Thin-client mode:** the same binary started with `--connect wss://…` paints the phase 3 DisplayList and forwards input. No core session runs locally.
- Input mapping: pointer (incl. hover → `:hover`, IDEAS #5), wheel, keyboard/Tab focus, DPI `scale_factor` → `dp`.
- **Accessibility:** feed the existing `oxiterm-a11y` tree (roles, labels, values, focus) into AccessKit (`accesskit_winit`). This gives AT-SPI on Linux plus Windows/macOS support and replaces the unfinished D-Bus transport in `DBusBridge` instead of completing it by hand. Check AccessKit's Android support before Phase 7.
- **Done when:** the demo apps run in a native window in both modes, and RSS is measured and documented for each.

### Phase 6: Proportional text (both modes at once)
- Ship `FontBundle` (one monospace and one proportional family) with the server, the native app and the WASM build.
- Taffy measure function backed by `cosmic-text`/`parley`, used by `oxiterm-core` wherever it runs.
- The server-driven DisplayList gains positioned glyph runs, so thin clients draw text without re-measuring it (§3.2).
- `font-family`, `font-size`, `line-height` in TCSS, in GUI mode only. TUI ignores them.
- **Done when:** the same document gives identical glyph positions in local mode and thin-client mode (golden test on the DisplayList), and clicks land on the right node in both.

### Phase 7: Mobile and framebuffer
- Android: `winit` Android backend or NDK `SurfaceView`, IME/viewport resize, touch and gesture normalization in the input layer. One APK supports both local and thin-client mode.
- WASM local mode: `oxiterm-core` + the web painter in the browser, offline-capable (alongside the phase 3 thin client).
- **OxiDE**: the same GPU or a software sink writing to DRM/KMS dumb buffers or `/dev/fb0` (see [retro-desktop-environment.md](retro-desktop-environment.md) and [embedded-iot-platform.md](embedded-iot-platform.md)). OxiDE becomes an application/shell on top of OxiUI rather than a separate fork, running in local mode.

---

## 6. Risks

| Risk | Mitigation |
|---|---|
| TUI regressions during the renderer refactor | Phase 0 golden snapshots gate every phase |
| Paint-order / clipping differences between old and new paths | Explicit `PushClip`/`PopClip` and `paint_order` in the DisplayList; snapshot coverage of overlapping/fixed layouts |
| Font-metric drift between server and GUI client | Monospace until phase 6; then a shared `FontBundle` and positioned glyph runs, so thin clients never re-measure |
| The two modes diverge over time | One `oxiterm-core` for both; DisplayList golden tests compare local vs thin-client output |
| `oxiterm-core` extraction breaks the server (`session.rs` is ~4,200 lines) | Phase 4 moves code behind traits without changing behaviour; existing server tests and golden snapshots gate it |
| Bandwidth of full DisplayList frames | Compression now, damage tracking in phase 3+ only if measurements require it |
| Scope creep toward "full browser" | Non-goals in §1; GUI-only CSS added one property at a time with silent-ignore in TUI |

---

## 7. Relationship to Other Ideas

- **[IDEAS #1 OxiDE](retro-desktop-environment.md):** OxiDE becomes a local-mode consumer of OxiUI (phase 7), getting a real pixel renderer instead of a cell grid on a framebuffer.
- **[IDEAS #3 Framebuffer Engine](embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers):** becomes a DisplayList sink.
- **[IDEAS #4 Transitions](tcss-reference.md#10-css-transitions--timing-curves):** gains sub-cell smoothness in GUI once phase 2 removes cell rounding from styles.
- **[IDEAS #7 OxiScreens](iot-screens.md):** the main consumer of server-driven mode. ESP32 thin clients use the cell-diff stream that phase 3 keeps as the fallback; its tier-2 3D needs `Polygon`/`Path` DisplayList items, which phase 3 should add.
- **[IDEAS #5 Hover states](IDEAS.md#5-reactive-hover-states--pseudoclasses-hover-focus):** native in GUI (pointer move events). Phase 5 should land `:hover` for both backends.
