# Changelog

## OxiScript 0.0.3 — 2026-11-27

Regular release on the 5555-line step (7110 changed lines since 0.0.2, `carry=1555`, [docs/oxiscript/VERSIONING.md](docs/oxiscript/VERSIONING.md)). Nothing is guaranteed before 1.0.0. Git tag: `oxiscript-v0.0.3`.

### Checker and syntax (stage 8.Fix)
- Closed the open checker/parser backlog: `static`/`mutating` constructors rejected (`OXS-E0123`), excluded union case labels warned (`OXS-W0005`), nullable operands diagnosed on every operator, compound assignment and `++`/`--` (`OXS-E0105`), `throw`/`try`/`catch` get a "no exceptions" message, `.length` / `s[i]` on strings carry their fix directly from the checker, budget-exhaustion errors name the offending expression.
- `decreases`, `ghost` and `pure` are reserved words.
- Formatter keeps comments next to operators and inside arrays, objects and argument lists; no `;` is inserted before a line that starts with a template literal.

### Security hardening (stage 8.Sec)
- One sanitiser for terminal escapes and bidi controls in every place the `oxi` CLI prints user-controlled text (file names, source lines, JSON `file_display`); bidi/invisible characters in comments are warned (`OXS-W0003`).
- `#![forbid(unsafe_code)]` in the compiler and runtime crates.
- Runtime: per-call wall-clock limit (`Limits::call_wall_micros`), fuel and memory fault-injection sweeps, a reload can never widen a module's capabilities, a total-locals cap in the module loader.
- Reproducible compiler output (byte-identical across builds).

### Specification (stage 8.Spec)
- `grammar.md`, `contracts.md`, `abi.md`, `llms.txt` and the authoring skill agree with the implementation (diagnostic codes, switch arm syntax, payload caps, effect flags); `sync_docs.py` checks every reason code the skill cites.

### Standard library
- `Map`/`Set`: re-entrant mutation from user `hash()`/`equals()` traps deterministically; `forEach` has a defined result when its callback mutates the collection (visits the entries present at the start, skips removed ones).
- `parseFloat` costs 15-30x less fuel per digit on long inputs; `repeat` is O(log n).

### Tooling
- OxiPerf (crate renamed `oxiscript-perf`): checksums are validated strictly, a run with a missing or wrong checksum fails.
- Version script: strict tag and `carry` parsing.
- Sites are deployed from release tags by a GitHub workflow that needs the owner's approval.

### Hardened by
- BAD ACTOR round 2026-11-27-1 (Opus: 0 high, 3 medium, 10 low) and round 2026-11-27-2 (Sonnet + Opus advisor: 1 medium, 4 low, 1 info), all fixed with regression tests ([docs/stages/rounds/](docs/stages/rounds/)).

### Known limitations
- `f64` to text still needs the real runtime linked (stage 8.M-link).
- Sorting 30k `i32` needs about 21M fuel (default 10M per call); stage 8.Perf.

## OxiScript 0.0.2 — 2026-11-26

Owner release before the 5555-line step (`owner-release=standard library v0`, [docs/oxiscript/VERSIONING.md](docs/oxiscript/VERSIONING.md)). Nothing is guaranteed before 1.0.0. Git tag: `oxiscript-v0.0.2`.

### Standard library v0 (new)
- `std/math`: Wasm intrinsics (`sqrt`, `floor`, `ceil`, `trunc`, `abs`, `min`, `max`, ...) and 14 transcendental functions (`sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `log`, `log2`, `log10`, `pow`, `cbrt`, `hypot`) in pure OxiScript, ported from FDLIBM/musl: deterministic, no FMA, no host libm, no heap allocation; measured within 2 ULP of the host libm. `round` and `toFixed` round halfway cases to even.
- `std/string`: correctly rounded `parseFloat` (Clinger, Eisel-Lemire, exact fallback; checked against Rust on 201k inputs), integer parsing, radix formatting, exact `toFixed`.
- `std/array`: stable merge sort (`sortBy` and typed sorts), search, `reverse`, `join`, `fill`, `flatMap`.
- `std/collections`: `Map` and `Set` with insertion order, `Hash`/`Eq` interfaces with laws; float keys use SameValueZero (NaN equals NaN, +0 equals -0). Built-in hashes are not seeded: do not use untrusted keys where hash flooding matters.
- `std/result`: predicates, extractors and combinators for `Result`.
- Numbers, `bool` and `string` gained `hash()` and `equals()`. `std` and `std/...` import specifiers are reserved.
- Reference: `docs/oxiscript/std.md`. Hardened by BAD ACTOR (findings BS01-BS08 fixed) and a separate review.

### Also in this release
- Compilation targets `device`, `web`, `server` (`oxi build --target`, manifest `target`, `OXS-E0411`); the runtime refuses modules that need features the host lacks.
- Playground: a program without `main` now says that nothing ran and lists its exported functions.
- Versioning: `oxiscript/std/**/*.oxs` counts toward the 5555-line step; owner override `--owner-release`.

### Known limitations
- `f64` to text still needs the real runtime linked (stage 8.M-link, in progress); `toFixed` works without it.
- Sorting 30k `i32` needs about 21M fuel (default 10M per call); stage 8.Perf.

## OxiScript 0.0.1 — 2026-11-26

First OxiScript release. **Nothing is guaranteed before 1.0.0**: syntax, semantics, ABI, OxiIR, the `oxi` CLI and its JSON schema may change in any release ([docs/oxiscript/VERSIONING.md](docs/oxiscript/VERSIONING.md)). One version for all `oxiscript-*` crates, the `oxi` CLI, the VS Code extension and the playground. Git tag: `oxiscript-v0.0.1`.

### Language
- TypeScript-like syntax with a stricter, typed core: sized integers (`i8`..`u64`) and floats (`f32`, `f64`) with explicit `as` conversion, `===` only, mandatory `;`, `Result<T, E>` with the `?` operator instead of exceptions, `struct` (value) and `class` (reference), `interface`, generics, discriminated unions with exhaustive `switch`, `T | null` for absence, UTF-8 strings with template literals.
- Capabilities enter only through `main` (`host:log`, `host:state`, ...), checked against `oxi.toml` `needs`. `export` is module visibility; host entry points are marked `@host`.
- Small built-in library: arrays, strings, numbers, `Result`, `range`. `law` declarations are parsed and checked.
- Reference: `docs/oxiscript/grammar.md`, `abi.md`, `ir.md`, `contracts.md`; model-oriented spec `docs/oxiscript/llms.txt`.

### Compiler pipeline
- `oxiscript-syntax` (parser, canonical formatter), `oxiscript-check` (type checker, reason codes `OXS-E/W` with machine-applicable fixes, append-only registry), `oxiscript-ir` (OxiIR), `oxiscript-lower` (end-to-end lowering), `oxiscript-wasm` (Wasm code generator), `oxiscript-rc` and `oxiscript-runtime` (reference-counting memory model, sandboxed wasmi runtime with fuel and host capability binding), `oxiscript-abi` (host interface IDL), `oxiscript-playground` (browser build).
- Differential and fuzz testing: about 19.6k differential programs in the lowering stage, no miscompilation found.

### `oxi` CLI
- `oxi check [--json] [--fix] [--fix-only semicolons]`, `oxi fmt [--check]`, `oxi explain <code>`, `oxi new <name> --template blink|touch-button|sensor-dashboard|oxiui-page`, `oxi run`, `oxi build`. `oxi --version` prints the version and the no-guarantee notice. JSON schema 1 (`oxiscript/oxiscript-cli/SCHEMA.md`).

### Tooling and sites
- VS Code extension (`editors/vscode-oxiscript`): highlighting, diagnostics, quick fixes, format on save, snippets and the semicolon assistant.
- Agent support: `AGENTS.md`, the `oxiscript-authoring` skill, `llms.txt`.
- Web playground on the OxiScript site with Check, Format and **Run** (compiled and run in a Web Worker with an instruction limit and a time limit). Three static sites (slavekm.pl, OxiUI, OxiScript) in EN and PL; Latin is an unpublished draft.

### Hardening
- Every stage (parser/formatter 8.P, runtime 8.R, memory 8.M, type checker 8.T with 22 findings, Wasm generator 8.G, lowering 8.L, DevKit 8.D with 17 findings, websites 8.W with 12 findings, Run) passed an adversarial BAD ACTOR review and fuzzing before merge. Findings and fixes are in `docs/stages/`.

### Known limitations
- `f64` cannot be converted to text yet (needs the real 8.M runtime linked; the current Stub runtime traps on float formatting).
- No standard library beyond the built-ins above; Rust crates as libraries (stage 8.N) not yet.
- No language server (LSP) yet; no `<script lang="oxs">` in OxiUI pages yet (8.S1).
- Closure cycles leak until the cycle collector is linked; Wasm local reuse limits very large functions (about 6000 statements).
- In the playground only `host:log` and `host:state` exist; nothing persists between runs.
- Two parser/formatter oracles (comment re-attachment, an E0006 no-op fix) still need a multi-seed check.

## [Unreleased] — OxiTerm / OxiUI work since 2026-11-23 (OxiTerm crate versions unchanged)

- **Phase 4:** `oxiterm-core` extracted as a pure crate (session state, event-loop step API, animations, frame limiter) driven by host traits; builds for wasm32 and Android.
- **Phase 5.2:** headless GPU sink `oxiterm-gpu` (wgpu, SDF fills and borders, glyph atlas matching the CPU renderer, golden and CPU-comparison tests).
- **Phase 6:** proportional text in both modes.
- **Phase 7:** WASM local mode (7.1) and the OxiDE software framebuffer sink (7.2).
- **Phase 7.6:** media and diagram parity across modes (SVG, PNG, Lottie, Rive, diagrams in GUI and local mode; honest missing-media placeholder in the no-native-media renderer).
- Dependency security upgrade (below) and the showcase versions of the sites.

### 🔒 Dependency security upgrade (2026-11-23)
- **russh 0.45 → 0.65** (`russh-keys` merged into `russh::keys`; `ssh-key` 0.7 backend): fixes RUSTSEC-2026-0496, -0493, -0494, -0492, -0154 and russh-cryptovec -0153 (pre-auth allocation / decompression / unbounded-size DoS issues). Also drops `rsa 0.9` (RUSTSEC-2023-0071, now `rsa 0.10`) and `crypto-bigint 0.5` (RUSTSEC-2026-0387, now 0.7).
- **prometheus 0.13 → 0.14** (protobuf 2.x → 3.7.2, RUSTSEC-2024-0437).
- **resvg 0.47 → 0.50** (drops the unmaintained `rustybuzz`/`ttf-parser` chain), **scraper 0.20 → 0.29** (drops unmaintained `fxhash`); `cargo update` of all compatible dependencies.
- SSH server migrated to the new russh handler API (`auth_publickey_offered` pre-check + `auth_publickey` on the verified key, `ChannelOpenHandle`, `run_stream` returns a session future that is now awaited). Authorized keys are compared by key material, so a changed comment no longer matters. The existing PKCS#8 ED25519 `host_key` file still loads unchanged (covered by a new unit test).
- Remaining `cargo audit` notes (unmaintained only, no vulnerabilities): `chrono` (via russh-util) and `paste` (via image/rav1e AVIF encoder).

## [0.6.0] — 2026-09-17

### 🪟 Desktop Environment Primitives & Window Management
- **Stacking Contexts & `z-index`**: Full CSS stacking order implementation in `oxiterm-renderer` and `oxiterm-proto`. Elements with higher `z-index` render on top of lower ones, enabling multi-window desktop layering.
- **Reverse-Order Hit Testing**: `HitTester::find_node` traverses layers in descending paint order, ensuring top-most elements (floating windows, dialogs, popups) reliably receive mouse clicks and interactions first.
- **Positioning (`position: absolute / fixed`)**: Inset offsets `top`, `bottom`, `left`, `right` integrated with Taffy 0.5+ and character cell grid coordinates.
- **Pointer Drag & Drop (`draggable` / `drag-handle`)**: Zero-drift pointer capture state machine in `EventLoop` supporting window movement, coordinate synchronization to `StateManager` (`drag-state-x`, `drag-state-y`), and drop completion actions (`event-drag-end`).
- **OxiDESK Window Features**: Interactive window collapse (shading into titlebar), floating dock mode, and taskbar restore with unified drag coordinate preservation.

### ⚡ Physics-Based Animations & CSS Transitions
- **Non-Linear Timing Solvers**: Integrated numerical Newton-Raphson `CubicBezierSolver` (`ease`, `ease-in`, `ease-out`, `ease-in-out`, `cubic-bezier(...)`) and second-order differential equation `SpringSolver` (`spring(mass, stiffness, damping)`).
- **Declarative Style-State Bindings**: Added `bind-width`, `bind-height`, `bind-top`, `bind-left`, `bind-opacity`, and `bind-z-index` to THTML/TCSS, enabling reactive state actions (`event-htmx`) to trigger smooth physical animations.
- **Target Baseline Restoration**: Implemented `AnimationController::restore_baselines` to eliminate target corruption from intermediate in-flight ticks, ensuring glitch-free interpolation.
- **Adaptive 60 FPS EventLoop**: Dynamic 16ms ticking during active transitions with instantaneous 0.0% CPU drop on idle.

### 🎨 Renderer & DOM Enhancements
- **Button Element Support**: Native intrinsic sizing and text rendering for `<button>` tag elements.
- **Opacity Blending**: Cell-level RGB color blending for translucent elements (`opacity: 0.0` to `1.0`).

### 🛠️ Authoring Tooling & Layout Linter
- **`lint_layout.py` Updates**: Added validation for positioning, `z-index`, opacity, and parenthesized transition arguments (`spring(...)`). Added dynamic dimension awareness for `bind-width` and `bind-height` to prevent false vertical budget (`W005`) errors. All 36 example files pass `--strict` with 0 errors.

### 📖 Architecture Blueprints & Roadmaps
- **`docs/retro-desktop-environment.md`**: Blueprint for turning OxiTerm into a standalone lightweight Desktop Environment (OxiDESK / OxiDE) for vintage Linux hardware (Pentium II/III/4, 64–512 MB RAM).
- **`docs/embedded-iot-platform.md`**: Blueprint for OxiTerm Embedded on IoT SBCs, machine HMIs, and direct `/dev/fb0` Linux framebuffer blitting (`oxiterm-fb`).
- **`docs/IDEAS.md`**: Central roadmap catalog including Reactive Hover States (`:hover`) and Any-Event mouse tracking (`1003h`).

### 🎵 Spotify App Server Modernization & Stability
- **Animated Progress Bar**: Modernized desktop and mobile templates (`examples/spotify/panel.thtml`, `panel_mobile.thtml`) with continuous linear CSS width transitions (`transition: width 1s linear;`) driven by `bind-width="progress_cols"` and `bind-width="progress_cols_mob"` with exact track duration/position labels.
- **Floating Draggable Volume Mixer**: Added an interactive draggable mixer modal (`position: absolute; z-index: 20; draggable="true" drag-handle="true"`) with spring-physics volume animation (`transition: width 250ms spring(120, 12, 1);`, `bind-width="vol_cols"`).
- **Safe Navigation Glyphs**: Replaced ambiguous Unicode arrows (`◄◄`, `►►`) with ASCII equivalents (`&lt;&lt;`, `&gt;&gt;`), achieving 100% linter compliance (`lint_layout.py --strict`) without hit-box displacement.
- **Thread-Safety & Concurrency**: Safeguarded background polling loops against `RuntimeError: dictionary changed size during iteration` by passing thread-safe `dict(active_oxiterm_sessions)` snapshots to worker threads.
- **Graceful Token Expiry Handling**: Updated `get_user_by_session_token` to return `None` when refresh token fails on expired sessions, cleanly breaking polling cycles without 401 error loops.

## [0.5.0] — 2026-07-24

### 🎵 Spotify Control Center Showcase Application (Plan 2.5 / 2.6)
- **Spotify Control Center**: Complete multi-user showcase application (`spotify-app-server/`) featuring Spotify OAuth 2.0 PKCE authentication, live background track playback polling, state patch updates via `POST /sessions/{id}/patch`, and multi-device UI (`spotify_panel.thtml` for Web/SSH and `spotify_panel_mobile.thtml` for Mobile).
- **Renamed Demo Directory**: Renamed `demo/` → `spotify-app-server/` with full git history preservation (`git mv`) to distinguish feature examples (`examples/`) from full application backends.
- **Documentation & Verification Suite**: Comprehensive documentation audit across `docs/` (`app-server-guide.md`, `thtml-reference.md`, `getting-started.md`, `spotify-demo.md`, `ARCHITECTURE.md`), eliminating inaccurate feature claims (D-Bus transport / Rive runtime) and updating script paths (`$BASH_SOURCE`).

### 🛡️ Security Hardening & App Server Protocol (Plans 2.5, 2.5b, 2.5c)
- **Fail-Closed App Authorization**: Both boundary endpoints (`POST /patch` on OxiTerm and `POST /events` on Python/FastAPI) enforce fail-closed authentication. Missing or empty `OXITERM_APP_TOKEN` acts as a circuit breaker, causing `/patch` to return `404 Not Found` (disabled).
- **Mutual Bearer Authentication**: `AppDispatcher` attaches `Authorization: Bearer <token>` to all event dispatches. Application servers verify requests with constant-time string comparison (`secrets.compare_digest`).
- **OAuth State Hardening**: `state` parameter in OAuth flow is time-limited (TTL 600 s) and strictly single-use (`pop()`).
- **Strict Session Isolation**: Eliminated cross-session fallback queries (`ORDER BY last_seen`) and session-token leaks in state patches or server logs.
- **XSS & Template Sanitization**: HTML escape applied to error query parameters on `/callback`.
- **Docker Test Pipeline**: Separated production `Dockerfile` from `Dockerfile.test`. Tests run exclusively inside Docker using an isolated `tmp_path` SQLite database.
- **Versioned Git Hooks**: Integrated `gitleaks` pre-commit hook under `.githooks/pre-commit`.
- **Shared Rust Test Guard**: Extracted `EnvGuard` into `crate::test_env` to serialize environment variable mutations across `web.rs` and `dispatcher.rs` unit tests without race conditions.

## [0.4.0] — 2026-07-19

### 🖥 Web Client & Responsive Design
- **Custom WASM Canvas Renderer**: The browser client is a purpose-built `WebTerminal` (Rust → WASM, `oxiterm-web`) that paints the cell grid onto an HTML `<canvas>` — it does **not** use xterm.js. A single responsive `index.html` is served at `/` for all devices.
- **Mobile-Responsive Layouts**: Client-side viewport classification (`< 800px` = mobile) with a `0x11` viewport-sync protocol, server-side hot swap (`SwitchViewport`), and automatic `_mobile.thtml` template resolution (mobile grid `48x30`).
- **DPR-Aware Rendering**: Device-pixel-perfect canvas sizing; pixel↔cell hit-mapping (clicks, wheel, media overlays) uses the true rendered cell size so hit targets stay aligned on any zoom/DPI.
- **Mouse-Wheel Scrolling (web)**: The wheel scrolls a few lines per notch via the existing wheel-mouse path, throttled to avoid flooding.

### 🔌 Sessions, Identity & App Server (Plan 2 / 2.1 / 2.2)
- **Session Reattachment & Takeover**: Browser sessions persist server-side and can be reattached via a `?session=<token>` token; opening a new tab takes over the session (old connection notified via `0xFF`). Session tokens are moved to `sessionStorage` and stripped from the URL.
- **Auto-Reconnection**: Exponential backoff capped at 8s; reattaching with a different `?page=` navigates the session.
- **Identity in App-Server Payloads**: `event-htmx` dispatch payloads now also carry optional `username` and `auth_method` fields alongside `action` / `state` / `session_id`.
- **Activation Integrity**: The input throttle no longer swallows mouse/keyboard activation (Plan 2.2) — a Press always runs hit-test → activation.

### 🧱 Layout, Rendering & Input Fixes
- **Word Wrapping**: `<text>` now supports `wrap: word`, resolved through Taffy measure functions so height follows the final wrapped width (previously auto-width text never wrapped).
- **`<for>` Loop Templates**: Added the `<for each="listKey">` element — its child template is cloned once per item of a `List` state value, with `{item}` substituted in text.
- **Clickable Hit Boxes**: A clickable `<text>` node's layout width now equals its rendered glyph span (min-width floor), so every visible column activates it.
- **Scrolling**: Vertical centering tracks the true content extent (bottom of the document is now reachable); a consistent position-model status indicator (`line X/Y` + bar + %); auto-scroll-to-focused fires only when focus changes, so it no longer fights manual scrolling.
- **Text Input**: Fixed a regression where character input was gated behind the Enter key — typing now updates the focused input's `bind-value` state on every keystroke again.
- **Ambiguous-Width Glyph Handling**: Hardened the whole "clickable label drifts off its hit box" class across all render paths — unambiguous focus-ring/label glyphs, an SSH-emitter cursor re-anchor after ambiguous glyphs, and the web cell-size fix above. A load-time warning and a test guard flag ambiguous-width glyphs used inside clickable labels.

### 🧩 Engine, Media & Accessibility (mid-cycle, since 0.3.0)
- **Input Primitives**: `<input>` with `bind-value`, real-time `TextInput`, and the `AppDispatcher` that POSTs `event-htmx` actions + state to an external App Server and applies returned JSON state patches.
- **Reactive StateManager Growth**: Dynamic tab highlighting and multi-action `event-htmx` chains (`a;b` / `a,b`); state patching applied back into the live session.
- **Scroll & WASM Pipeline**: Server-side scroll rendering and the WASM web rendering path (the "v0.4 pipeline"), including a 6-byte binary cell-diff protocol shared with the web client.
- **Accessibility Layer**: `LinearFrameSink` (`--a11y`) renders the document as a linear text tree for screen readers. The AT-SPI/D-Bus API surface (`DBusBridge`, `register_at_spi`, `update_focus`) is a **skeleton without transport implementation** — no D-Bus connections are established at runtime.
- **Media Buffering**: Frame buffering for video/animation media alongside the SVG/asset caches.
- **Parser & Server Robustness**: CSS-comment stripping, HTML-entity decoding, SSH hot-reload, resilient file watching, and a fix for a navigation focus leak on hidden (`bind-show`) nodes.
- **Web Fixes**: Resolved coordinate translation / connection-init hang and a blank-screen bug (canvas clear-dimension init + handshake packet ordering).
- **Plan 2 App Primitives**: Audit repairs, identity plumbing, and app-server primitives (see identity fields above); expanded contract/unit test coverage.

## [0.3.0] — 2026-05-22

### 🎨 Vector Graphics & Interactive Animations
- **High-Fidelity SVG Rendering**: Integrated `resvg` and `tiny-skia` for rendering SVG vectors.
- **Lottie & Rive Procedural Rendering**:
  - Implemented procedural vector loading spinner animation for `.json` Lottie loops.
  - Implemented procedural Rive toggle switch reacting dynamically to hover and click coordinates.
- **Playback Registry & Event Loop Ticking**:
  - Designed `PlaybackRegistry` to track active animation states, timelines, and hover/click variables.
  - Updated the event loop to scale timeout from `5ms` to `66ms` (15 FPS) during active animations.
- **Interactive Mouse Mapping**: Automated cell-relative mapping for click and hover events within layout bounds.
- **Fast Sixel Encoder**: Pre-allocated 256-color palette quantization in Sixel codec to optimize rendering performance.
- **Caching Layer**: Implemented thread-safe `SvgCache` and resolution-indexed `AssetCache`.

### 🌐 Web Console & Raster Graphics
- **Web Console (Canvas)**: Added the browser console with Canvas-based rendering over WebSocket, letting apps run in a browser with no install.
- **Sixel & Kitty Graphics**: Added raster image rendering via the Kitty Graphics Protocol and a Sixel codec, with automatic terminal-capability negotiation and fallback.
- **QA Hardening**: Resolved QA Audit v3 issues across the server and parser (resource base-directory resolution, edge-case fixes).

### 🚀 Examples & Integration
- **Vector Demo Showcase**: Added `mascot.svg`, `loader.json`, and `toggle.riv` to `examples/`.
- **Linked Dashboard**: Updated default `hello.thtml` page to link to the new Vector & Animation Demo page.

## [0.2.0] — 2026-05-09

### 🚀 Major Architecture Upgrade
- **StateManager v2**: Introduced a fully reactive, subscription-based state management system. 
  - Supports typed values: `Int`, `Str`, `Bool`, and `List`.
  - Precise re-rendering: Only nodes affected by a state change are updated.
  - Native HTMX-style actions: `inc`, `dec`, `toggle`, `set`, `append`, `clear`.
- **TCSS Inline Styles**: Integrated the TCSS engine directly into THTML.
  - Use `style` attribute on any node (e.g., `<box style="fg: red; padding: 1;">`).
  - Support for named colors and TUI-friendly aliases (`fg`, `bg`).
  - Integrated into the `THTMLParser` pipeline.

### 🛡 Security & Stability
- **SSH Hardening**: Fixed a critical vulnerability where unauthorized users could log in if no password was configured.
- **HTMX Sanitization**: Added strict validation for `event-htmx` and `bind-state` to prevent path traversal and ANSI injection.
- **Memory Safety**: Resolved complex borrow checker issues in the rendering loop for better performance and reliability.
- **Improved Testing**: Added 22 comprehensive unit tests covering the state manager, parser, and renderer.

### 🛠 CLI & Developer Tools
- **Hot Reload Improvements**: More robust file watching for THTML development.
- **Optimized Release**: Binned binaries are now significantly smaller and faster.

## [0.1.0] — 2026-05-08

Initial foundation — the "TUI as a website" engine (Sprints 0-6) and first CLI release.

### 🏗 Core Engine & SSR Pipeline
- **Workspace Layout**: Split into `oxiterm-proto`, `oxiterm-renderer`, `oxiterm-server`, `oxiterm-a11y`, `oxiterm-web`, and `oxiterm-cli` crates.
- **THTML Parser**: `nom`-based parser building a DOM arena from THTML markup, with tag/attribute sanitisation.
- **TCSS Styling**: Terminal-CSS engine (colors, flexbox properties, padding/margin, borders) with a `<style>` block cascade.
- **Layout Engine**: Flexbox layout via **Taffy**, mapping the DOM onto a terminal cell grid.
- **Rendering & Diffing**: `CellBuffer` frame rendering and a `DoubleBuffer` diff engine that emits a minimal ANSI escape stream (cursor moves, colors) per frame.

### 🔌 Transport, Input & Resilience
- **SSH Server (russh)**: Asynchronous SSH daemon negotiating PTY dimensions and terminal capabilities (Kitty Graphics, SGR mouse) via DA1.
- **Input Decoding**: `InputStateMachine` parsing Kitty/SGR keyboard and mouse byte streams, driven by a non-blocking Resilient Reactor Thread (RRT).
- **Backpressure & Rate Limiting**: `BoundedFrameChannel` congestion control (safe frame dropping for slow clients) and a connection rate limiter.
- **Metrics**: Prometheus metrics endpoint.

### ♿ Accessibility & 🛠 CLI
- **Accessibility Groundwork**: AT-SPI / a11y layer foundations for screen-reader output.
- **CLI Release**: `oxiterm-cli` with `serve` (host a `.thtml` app over SSH + Web), `demo`, and `check` (template validation) commands.
- **Production Hardening**: Security-audit fixes across the transport layer and stabilised builds.

---
*This changelog was reconstructed from the full commit history; pre-0.2.0 entries summarise the foundational sprints.*

