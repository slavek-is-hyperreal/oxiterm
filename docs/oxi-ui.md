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
- Move `EventLoop`, `StateManager`, focus/scroll/drag handling, `inject_initial_state` and animations from `oxiterm-server` into a new `oxiterm-core` crate with **no tokio, russh, hyper or ureq** dependencies.
- Introduce the `ActionHandler` trait in place of the direct `ureq::post` in `dispatcher.rs`. `oxiterm-server` provides `HttpActionHandler` (current behaviour). Local apps can provide in-process Rust handlers.
- Frames leave the core only through `FrameSink`. `ClientSession.web_frame_tx` and the SSH reactor stay in `oxiterm-server` as sink implementations.
- `oxiterm-core` must build for `wasm32-unknown-unknown` and `aarch64-linux-android`. Add both to CI as build-only checks.
- **Done when:** `oxiterm-server` is a thin host around `oxiterm-core`, all existing server tests and golden snapshots pass, and a small example runs the core with an in-process `ActionHandler` in a unit test.

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
