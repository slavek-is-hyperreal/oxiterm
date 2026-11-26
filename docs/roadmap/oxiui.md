# OxiUI roadmap

Status: 2026-11-26, written against `main` at `19cf8cd`. This is the single roadmap for OxiTerm / OxiUI (terminal, web and native renderer, `oxiterm-core`, server, local mode, OxiDE, GPU sink, accessibility, devices and screens, apps, sites infrastructure, CI). The language side is in [oxiscript.md](oxiscript.md). Index and id rules: [README.md](README.md). Open inconsistencies and stale documents: [consistency.md](consistency.md).

Every row carries a `Sources:` cell. The ids (`R1-nn` ... `R4-nn`) point into the reader inventories the lead produced on 2026-11-26 (`scratch/roadmap/R1..R4.md`); they make it possible to check that nothing was lost.

Contents: 1 Vision | 2 Principles and owner decisions | 3 Done | 4 In progress | 5 Roadmap | 6 Ideas not scheduled | 7 Open questions | 8 Working rules.

---

## 1. Vision and positioning

OxiUI is one markup (THTML) and one style language (TCSS) that renders to a terminal over SSH, to a browser (cell mode and pixel GUI mode), to a Linux framebuffer (OxiDE) and, planned, to a native GPU window and an Android shell. The same page, the same session state and the same App Server protocol drive every target.

Owner positioning (2026-11-26):
- **Simple, interoperable, server-streamed first.** The same page appears in a terminal, a browser canvas and a device. `host:dom` ([8.Dom](oxiscript.md#s-8-dom)) is a separate path for browser-native HTML apps and does not change this.
- **GUI and scripting for devices with megabytes, not gigabytes, of memory** (industrial HMI panels, kiosks, home automation, old PCs via OxiDE, toys on ESP32), next to the server-streamed role. The claim goes on the sites only with measured numbers ([8.Mem](oxiscript.md#s-8-mem), U.Bench below).
- Two execution modes, same app source: **server-driven** (the server renders, a thin client paints) and **local core** (parser, layout and loop compiled into the client: WASM, desktop, Android).
- OxiUI is the first host of OxiScript but not its limit; the language has its own roadmap.

Non-goals (owner, plan section 1): full W3C HTML/CSS, a JavaScript engine, Flutter-level mobile polish before the desktop and web path is solid.

---

## 2. Principles and owner decisions that bind OxiUI

Numbering `O-n` is local to this file (anchors `o-1`...). OxiScript principles and decisions have their own sets (`p-1`.. and `d-1`.. in oxiscript.md).

| Id | Decision | Date | Source |
|---|---|---|---|
| <a id="o-1"></a>O-1 | One language: THTML/TCSS stays the only format; backend-specific styling through `@media (render: tui\|gui)` and silent ignore of unsupported properties. | plan of 2026-11-22 | `docs/oxi-ui.md` section 1 |
| <a id="o-2"></a>O-2 | Zero regression for the TUI: every existing app renders byte-identically over SSH and WebSocket after each phase (golden snapshots gate it). | 2026-11-22 | oxi-ui.md section 1, section 6 |
| <a id="o-3"></a>O-3 | Keep the server and protocol (`/events`, `/sessions/{id}/patch`, dispatcher, state); no rewrite. | 2026-11-22 | oxi-ui.md section 1 |
| <a id="o-4"></a>O-4 | D1/D2: **both execution modes** (server-driven thin client and local core), sharing `oxiterm-core`, a bundled font and positioned glyph runs. Thin client distribution: web first, then as a mode of the native/Android app. | plan section 4 (undated; plan created 2026-11-22) | oxi-ui.md:116-122 |
| <a id="o-5"></a>O-5 | D3 (revised): a custom **wgpu** pipeline (vertex and fragment shaders only: instanced SDF rounded quads, glyph-atlas quads, scissor clips); no raw `ash`, no Vello compute renderer; permissive libraries only (cosmic-text/swash, etagere, lyon, winit); hardware below the wgpu floor (Raspberry Pi 0-3) uses the CPU/framebuffer sink. | 2026-11-22 research, plan section 5 | oxi-ui.md:344-356, `research/vulkan-acceleration.md` |
| <a id="o-6"></a>O-6 | Lightweight GUI target: native window aiming at < 15-25 MB RSS; RSS measured and documented per mode (not yet measurable: no window exists). | 2026-11-22 | oxi-ui.md:11-20, :335-341 |
| <a id="o-7"></a>O-7 | Proportional text arrives in both modes at once (Phase 6, done); fonts **Noto Sans + Noto Sans Mono**, decision delegated to the lead by the owner. | 2026-11-24 | oxi-ui.md:588 |
| <a id="o-8"></a>O-8 | **BAD ACTOR gate**: after every finished stage, before its commit is admitted, a separate adversarial agent tries to break it (fuzzing, path/sandbox escapes, malformed protocol input, resource exhaustion, differential checks between modes); findings are fixed or logged and each becomes a regression test. | 2026-11-24 | oxi-ui.md:137; memory "BAD ACTOR gate" |
| <a id="o-9"></a>O-9 | Shared GPU device: one GPU context (device, queue, residency) for the DisplayList renderer, OxiScript kernels ([8.3c](oxiscript.md#s-8-3c)/[8.3d](oxiscript.md#s-8-3d)) and 3D (9.3); script kernels may write textures that the display list draws ("paint with this kernel"); 5.2 is the foundation, 5.3-5.7 may stay last. wgpu in the browser can accelerate local mode without the native window. | 2026-11-25 | oxi-ui.md:756-762 |
| <a id="o-10"></a>O-10 | Phase 9 (3D in OxiUI) is a draft, not scheduled, phase order unchanged. | 2026-11-25 | oxi-ui.md:742 |
| <a id="o-11"></a>O-11 | IDEAS #8 (server-rendered high-risk surfaces: login, PIN, payment) is an idea, "not scheduled; does not change the phase order". | 2026-11-25 | IDEAS.md:99-109 |
| <a id="o-12"></a>O-12 | Positioning: OxiUI stays simple, interoperable and server-streamed first; GUI and scripting for MB-class devices; measure before claiming (first hardware: the owner's Raspberry Pi 1 Model B+, ARMv6, 512 MB). | 2026-11-26 | oxi-ui.md:650-651 |
| <a id="o-13"></a>O-13 | Repository split (plan decision 7, revised): OxiScript and possibly the sites move to their own repositories, history preserved, earliest after 8.S1, owner decides when. Until then keep the halves separable: no new `oxiscript-*` to `oxiterm-*` dependency except the 8.S1 interface, separate CI workflows, changelogs and tags (`oxiscript-v*` / `v*`). Owned by [8.Repo](oxiscript.md#s-8-repo). | 2026-11-26 | oxi-ui.md:718 |
| <a id="o-14"></a>O-14 | Dependencies: permissive licences only (MIT/Apache/BSD/Zlib/ISC; MPL only if recorded; fonts OFL/Apache; never GPL/AGPL/LGPL). Project is MIT OR Apache-2.0. Every dependency is an SBOM entry (CRA/IEC 62443). | standing | HANDOFF.md section 1; README; memory "GPU libraries policy" |
| <a id="o-15"></a>O-15 | Inform, don't forbid: trade-offs that weaken a guarantee (an `unsafe` crate, analysis scope) are documented in the spec and announced at build time, not banned. | standing | HANDOFF.md; memory |
| <a id="o-16"></a>O-16 | Never let an agent touch `/dev/fb0`, `/dev/dri/*`, `/dev/input/*` (the owner's real display); device output only manually with the owner's OK. | standing | HANDOFF.md section 5; oxi-ui.md:599-600 |
| <a id="o-17"></a>O-17 | Golden and trace baselines are blessed only by the lead (`OXI_BLESS*`); subagents run no git commands; shared files (root `Cargo.toml`, CI workflows, the plan) are edited only by the lead. | 2026-11-24 | oxi-ui.md:329-333; HANDOFF.md section 2 |
| <a id="o-18"></a>O-18 | Report outcomes from independent evidence (check `gh run list` after each push); do not claim a gap without grepping the repo; code beats docs (`ONBOARDING.md`). | 2026-11-25/26 | HANDOFF.md section 1; ONBOARDING.md:11-14 |
| <a id="o-19"></a>O-19 | Decisiveness: taste decisions (fonts, colours, visual calls) are delegated to the lead after screenshots; stop only for licence, money, scope, language semantics. | standing | HANDOFF.md section 1; memory |
| <a id="o-20"></a>O-20 | Sites: Polish and English first, Latin after the owner's review (in memoriam dedication); Polish copy is a draft until the owner reviews; no trackers; HSTS only as a Cloudflare dashboard switch, never preload by default; sites are Cloudflare **Workers static assets**, not Pages (2026-11-26). | 2026-11-25/26 | oxi-ui.md:648; sites/DEPLOY.md; commit ddad8b3 |
| <a id="o-21"></a>O-21 | Language of work: the owner writes Polish and gets Polish answers; code, docs and commits are English. Dev blog posts only by the owner or the lead, each signed by its author. | standing | HANDOFF.md; memory |
| <a id="o-22"></a>O-22 | Hardware: ESP32-S3 4.3" 800x480 touch (N16R8) is bought after 2026-12-10; device work waits for it; no public server yet (a VPS with port-22 SSH comes later). | 2026-11-26 | HANDOFF.md section 5; memory |
| <a id="o-23"></a>O-23 | Compilation targets `device`, `web`, `server` are a build concept distinct from language profiles (owned by [8.2a](oxiscript.md#s-8-2a), done). | 2026-11-26 | oxi-ui.md:655 |

Overtaken by events (not revoked by the owner; owner to confirm, see Q-1):
- **Execution order of 2026-11-22** (5.0-5.1, then 6, then 7, then 5.2-5.7 last, Phase 8 after Phase 7). In fact Phase 8 ran in parallel with Phase 7 from 2026-11-24 (plan "Parallel schedule"), and 5.2 was pulled forward (as the plan itself foresaw in oxi-ui.md:759). 5.3-5.7 remain last. Sources: R2-09, R4-30.

Superseded design decisions from the original concept documents (kept for history, not owner decisions of the current plan):
- CD1 mobile design (User-Agent detection and `/` to `/mobile` 302): replaced by client-side viewport classification (< 800 px) plus the `0x11` viewport sync (CHANGELOG 0.4.0, ARCHITECTURE section 7). Sources: R1-07, R1-48.
- CD1 "keys only, passwords rejected": shipped auth has `OXITERM_PASSWORD`, `--no-auth` demo mode and guest web access. Sources: R1-04, R1-81.
- CD2 "linear fallback is the most serious denial of inclusion": the SSH `--a11y` linear sink shipped anyway; see U.A11y. Sources: R1-24, R1-25.
- CD2 formal parsing (Vest) and PUA/VTM Unicode modifiers as "critical": replaced by fuzzing under the BAD ACTOR gate and by avoiding ambiguous-width glyphs in clickable labels (width cache, load-time warning, linter W006). Recorded as a decision question in U.Term. Sources: R1-21, R1-23.
- D3 "may end up as two sinks (Vello vs custom quads)" in plan section 4: superseded by O-5.

---

## 3. Done (verified)

All hashes below were checked with `git merge-base --is-ancestor <hash> main`. Format chosen so a generator can parse it: first cell starts with the stage id, last cell holds a date and a 7-hex hash.

| Stage | What | Documented in | Merged |
|---|---|---|---|
| 0 | Golden snapshot harness (72 cell baselines, `OXI_BLESS=1`) | oxi-ui.md Phase 0; `oxiterm-renderer/tests/golden` | 2026-11-22, `b77f1a6` |
| 1 | DisplayList inside the renderer (`display_list.rs`, CellSink) | oxi-ui.md Phase 1 | 2026-11-22, `2149847` |
| 2 | f32 cell geometry, length units, `@media (render:)`, `border-radius` | oxi-ui.md Phase 2 | 2026-11-22, `c1ae580` |
| 3a | DisplayList over the wire (`0x36`, zlib), vector painting in the browser | oxi-ui.md Phase 3a | 2026-11-22, `5dff19f` |
| 3b | Fractional layout, border-radius, `Path` items (wire v2) | oxi-ui.md Phase 3b | 2026-11-22, `b088089` |
| 3c | ARIA mirror `0x37` for GUI web clients | oxi-ui.md Phase 3c | 2026-11-22, `b335dd3` |
| 4 | `oxiterm-core` extraction (steps 0-8c; step API `handle_event`/`tick`/`next_deadline`, `MemPageSource`, `ActionHandler`; builds for wasm32 and aarch64-linux-android, CI-checked) | oxi-ui.md Phase 4 | 2026-11-23, `38ba048` |
| 5.0 | Instrumentation: `frame_cost` bench, baseline table | oxi-ui.md Stage 5.0 | 2026-11-24, `66fa334` |
| 5.1 | CPU wins: 5.1a lock-free width table (`6b2ffe6`), 5.1b incremental layout (`7802a03`), 5.1c overlay row diff and ARIA cache | oxi-ui.md Stage 5.1 | 2026-11-24, `a451657` |
| 5.2 | Headless `GpuSink` on wgpu (`oxiterm-gpu`: SDF fills/borders, glyph atlas, clips, CellGrid; PNG goldens on lavapipe; CI `gpu` job) | `docs/stages/5.2`, `oxiterm-gpu/README.md` | 2026-11-26, `05c22ea` (CI `9e4aa49`, `c4ecd2d`) |
| 6 | Proportional text, stages 6.0-6.6: cosmic-text shaping, Noto fonts at `/oxiui/fonts.json`, TCSS font properties, positioned glyph runs (wire v3), fractional pointer (`0x03`), local-vs-thin equality test, wasm32 text test in CI | oxi-ui.md Phase 6 | 2026-11-24, `54a7e19` (6.6; demo pages `d1a2117`) |
| 7.1 | WASM local mode (`?local=1`, `--local-mode`) | oxi-ui.md Stage 7.1 | 2026-11-25, `77f9002` |
| 7.2 | OxiDE software sink `oxiterm-fb` (PNG goldens; `/dev/fb0`/DRM behind a flag) | oxi-ui.md Stage 7.2 | 2026-11-25, `78fcdb5` |
| 7.6 | Media and diagram parity across modes (one resolver `pathsafe::resolve_media`, pure-Rust Rive, vendored lottie-web, 64-bit media keys, wire break `0x20/0x21/0xE0`) | `docs/stages/7.6` | 2026-11-26, `83ba89e` |
| Showcase | OxiUI versions of the three sites (80-column pages, `_mobile` variants, tabs/accordion/drag/spring/Lottie) | `docs/stages/showcase` | 2026-11-26, `782e539` |
| Sites deploy | `oxiui.slavekm.pl` deployed as Workers static assets (wrangler configs) | `sites/DEPLOY.md` (second half) | 2026-11-26, `ddad8b3` |
| Terminal baseline | SSH server (russh), DA1 capability negotiation, Kitty keyboard and SGR mouse decoding, synchronized updates `?2026`, bounded frame channel, XON/XOFF pause, resize debounce, predictive local echo (server side), Kitty/Sixel/half-block images | `CHANGELOG.md` 0.1.0-0.6.0, `docs/ARCHITECTURE.md` | before 2026-11-22 (CHANGELOG) |
| OxiDESK window features | z-index, reverse hit-test, `position: absolute/fixed`, drag and drop, collapse/dock/taskbar, physics transitions (bezier, spring), layout linter | `CHANGELOG.md` 0.6.0 | before 2026-11-22 (CHANGELOG) |
| App Server | `/events`, `/sessions/{id}/patch` (+ `navigate`, `open_url`, `set_app_token`), fail-closed auth, Spotify control center, oxiterm-reader | `docs/app-server-guide.md`, `docs/spotify-demo.md`, `apps/oxiterm-reader/README.md` | before 2026-11-22 (CHANGELOG) |

Sources of the Done rows: R1-05,06,08,10,11,16-19,22,31,43,44,46,49,51,52,53,54,60,97,103,104,105; R2-11..15,23..25,28,33,40..42,44,45,50,126,127,140,143,148,151; R4-30,36.

Notes on dates: the CHANGELOG-based rows predate the plan; no single merge commit exists for them. Items R1-12 (predictive echo, adaptive mode unchecked), R1-20 (resize debouncer only, no subtree dirty skipping) are done only in the part stated here; the rest is in U.Term.

---

## 4. In progress

**No OxiUI work is in progress.** Every 5.x and 7.x stage that was started is merged. The three branches that carry unmerged work are OxiScript stages, listed in [oxiscript.md section 4](oxiscript.md): `gemini-rtlink` ([8.M-link](oxiscript.md#s-8-m-link)), `gemini-std` ([8.Std](oxiscript.md#s-8-std)) and `gemini-memres` ([8.Perf](oxiscript.md#s-8-perf) research). The worktree `.claude/worktrees/roadmaps` holds this documentation work. The remaining 19 `worktree-agent-*` worktrees and `gemini-5.2`, `gemini-simd`, `integ-sites`, `worktree-targets` are fully merged leftovers (cleanup item U.Hygiene). Sources: R2-89, R4-80, R4 "Unmerged worktrees".

---

## 5. Roadmap

Ids: existing plan ids are kept (5.x, 7.x, 9.x). New ids use the prefix `U.` Each stage is a heading with an anchor `s-<id>` (lowercase, "." becomes "-"). Status: `todo`, `idea` (design only, owner may schedule), `blocked`, `conditional`. Every stage ends with a BAD ACTOR pass (O-8) unless it is documentation only. Order within a group is the dependency order; the suggested overall sequence is in 5.12.

Renamed ids (old -> new): oxi-ui.md "Phase 5 stages 5.2-5.7" keep their ids; "Stage 7.6" (absent from the old plan table) now appears in section 3; oxi-ui.md Phase 4 "Still open ... next_deadline" -> U.Idle; "8.3b GPU kernels" (old wording in `docs/stages/5.2`) -> [8.3c-8.3e](oxiscript.md#s-8-3c); OxiDE phases 1-5 of `retro-desktop-environment.md` -> 7.2b / U.OxiDE (the old phase numbers there collide with OxiUI phases and are retired); OxiScreens roadmap steps 1-7 of `iot-screens.md` -> U.Screens-1..4.

### 5.1 Native window and GPU (Phase 5)

<a id="s-5-3"></a>
#### 5.3 Native window, thin-client mode
- Scope: `oxiterm-gui` crate, `winit` window, `--connect wss://` paints the `0x36` stream through `GpuSink` (5.2). The shared `GpuContext` must expose the wgpu device and queue without breaking the [8.3c](oxiscript.md#s-8-3c) API (W-OXS requirement). Configurable pipeline format (swapchain Bgra8/sRGB; `render_into_view` currently returns `InvalidTarget`). Measure RSS of a minimal winit+wgpu window with a driver loaded, wasm size of wgpu, wgpu limits on Pi 4/5.
- Gate: counter demo and oxiterm-reader usable in a native window; RSS and startup measured and written down (O-6); license check (`cargo deny`, U.CI) passes before wgpu/winit are added to a shipping binary.
- Depends: 5.2 (done), U.CI (licence gate).
- Status: todo. Sources: R2-22,29,145,170; R4-31.

<a id="s-5-4"></a>
#### 5.4 Native input and HiDPI
- Scope: pointer, wheel, keyboard/Tab, resize, DPI `scale_factor` to `dp`; `:hover` hooks into U.Hover.
- Gate: clicks hit the right nodes (trace-style test against a live server); two DPI scales checked.
- Depends: 5.3. Status: todo. Sources: R2-22,29,30.

<a id="s-5-5"></a>
#### 5.5 Native local mode
- Scope: `oxiterm-core` in-process, in-process or HTTP `ActionHandler`; thin client mode of the same binary stays. Embeds OxiScript pages once [8.S1](oxiscript.md#s-8-s1) lands (see U.S1).
- Gate: same app offline; frames equal to thin-client mode for the same input.
- Depends: 5.3, 5.4. Status: todo. Sources: R2-22,29.

<a id="s-5-6"></a>
#### 5.6 `Path` via lyon, images as textures
- Scope: `Path` items in `GpuSink` (CPU sink `oxiterm-fb` already draws paths; parity gap today), images and video frames as textures; resolves the "media overlay above z-index windows" limit for GUI mode where possible; glyphs above 512 px (accepted skip, 5.2 B-09) documented.
- Gate: new PNG goldens; CPU/GPU differential within the 5.2 tolerance.
- Depends: 5.2. Status: todo. Sources: R2-29,149,150,153; R1-105,45.

<a id="s-5-7"></a>
#### 5.7 AccessKit and Raspberry Pi 4/5 run
- Scope: feed the `oxiterm-a11y` tree into AccessKit (`accesskit_winit`); check AccessKit's Android support (promised "before Phase 7", never done); run on a Pi 4/5 (record wgpu limits for v3dv).
- Gate: AT tree visible to the OS; app runs on a Pi; RSS recorded.
- Depends: 5.3, 5.4. Status: todo. Sources: R2-22,29,170; R1-25.

<a id="s-u-hover"></a>
#### U.Hover Hover, sub-cell drag, pointer caret
- Scope: `:hover` pseudo-class for both backends (IDEAS #5; any-event mouse `1003h` in the terminal, `InputThrottle`), pointer move events in the web client (today moves are sent only while a button is down), sub-cell drag deltas, caret placement from a pointer press on `<input>`, `.riv` hover/click on fractional cells. TCSS reference gets the selector grammar.
- Gate: trace tests for hover enter/leave in TUI and GUI; `tcss-reference.md` updated; no TUI golden changes unless the page uses `:hover`.
- Depends: 5.4 for native; web client part can start now. Status: todo. Sources: R1-56,57; R2-30,36,140; R4-41; IDEAS #5.

<a id="s-u-webgpu"></a>
#### U.WebGPU Browser performance and GPU painter
- Scope: batch `fillText` per glyph in the Canvas2D painter (runs or glyph cache + `drawImage`); WebGL2 glyph-atlas painter matching the native pipeline; wgpu on wasm32 (WebGPU, WebGL2 fallback) for local mode with 5.2 + 7.1, without the native window; share `oxiterm-web/src/paint.rs::plan()` (border snapping, radius clamping, hidpi alignment) between Canvas2D, WebGL2 and wgpu through one crate; run the 5.2 GL-backend shaders on a GLES3/WebGL2 adapter (never run so far); measure first (the research marks the Canvas2D hotspot as inference, not profiled).
- Gate: profile of the showcase page before/after; golden equality of painters; wasm size recorded for wgpu in the bundle.
- Depends: 5.2, 7.1; coordinates with [8.3c](oxiscript.md#s-8-3c) (Q5 browser wgpu vs thin JS bridge). Status: todo, measure first. Sources: R2-31,144,167,168; R4-39.

<a id="s-u-kerneldraw"></a>
#### U.KernelDraw "Paint with this kernel"
- Scope: display-list item (`KernelDraw { kernel, region, inputs }` or `Texture`) that draws a texture written by an OxiScript kernel without CPU round trip, CPU fallback on terminal sinks; plan section 3.1 enum gets the item.
- Gate: defined by [8.3d](oxiscript.md#s-8-3d); lavapipe golden.
- Depends: 5.2, [8.3c](oxiscript.md#s-8-3c), [8.3d](oxiscript.md#s-8-3d). Status: idea (owner O-9 fixed the design rule, not the item). Sources: R3-176; R4-39.

### 5.2 Mobile, framebuffer and OxiDE (Phase 7 and OxiDE)

<a id="s-7-2b"></a>
#### 7.2b OxiDE runner `oxide run page.thtml`
- Scope: core in local mode on the Linux console without X11/Wayland: output through the 7.2 sink to `/dev/fb0` or DRM, keyboard/mouse/touch from evdev (none in the repo today; `grep evdev` in `oxiterm-fb` finds nothing), step-API loop, actions to an App Server (later a Unix socket). Raw TTY runner on `/dev/tty1`.
- Gate: runs a demo page full screen in a VM or on spare hardware with the owner's OK (never by an agent on the owner's display, O-16); input mapping tested with recorded evdev events.
- Depends: 7.1, 7.2 (done). Status: todo. Sources: R2-46; R4-37; R1-75,79.

<a id="s-7-3"></a>
#### 7.3 Android shell
- Scope: `android/` Kotlin WebView, thin-client URL first, bundled local mode after 7.1; note the 7.6 wire break (server and client ship together) and the media limits (> 16 MiB not sent over WS).
- Gate: `assembleDebug` builds; unit tests; no device run (none attached; SDK 34 + Java 17 at `/vectorlegis_ssd_pool/android_sdk`).
- Depends: 7.1. Status: todo. Sources: R2-47,155; R4-37.

<a id="s-7-4"></a>
#### 7.4 Touch, IME, visualViewport
- Scope: touch to `0x03` pointer, virtual keyboard input, `visualViewport` resize, in the web client (same files as 7.3).
- Gate: coordinate tests; mobile viewport checked in the browser pane. Depends: 7.1, 7.3. Status: todo. Sources: R2-48.

<a id="s-7-5"></a>
#### 7.5 Close Phase 7
- Scope: local-vs-thin equality in CI (partly done for text in 6.6), docs, bundle sizes recorded (7.1 gate item "wasm bundle size recorded" and "counter demo works with the server stopped" are not confirmed in the plan), BAD ACTOR pass.
- Depends: 7.3, 7.4, 7.2b. Status: todo. Sources: R2-44,49.

<a id="s-u-oxide"></a>
#### U.OxiDE OxiDE desktop shell
- Scope: FocusManager with Alt+Tab and Tab cycling inside windows, Super-key launcher, XDG `.desktop` launcher, PTY child windows, system daemon (`oxide-daemon`: supervisor, `/proc` telemetry), cgroup limits, D-Bus (zbus) Notifications/StatusNotifierItem/login1, ALSA/nmcli, lighter media on weak CPUs (replace resvg/rlottie with lodepng/palettes), dirty-rect blitting (see U.Damage), reference `desktop.thtml` fixed for the ambiguous-width glyph rules (emoji in clickable labels). Rewrite `retro-desktop-environment.md` against the shipped state first (its section 3.1 is obsolete).
- Gate: per sub-feature, defined when scheduled; unmeasured RAM/boot claims of the blueprint (18-35 MB, < 0.5 s) removed or measured by [8.Mem](oxiscript.md#s-8-mem).
- Depends: 7.2b, U.Idle. Status: idea (scheduled only after 7.2b). Sources: R1-73..79; R1-62,63.

<a id="s-u-ipc"></a>
#### U.IPC Unix socket patch transport and serial sink
- Scope: Unix Domain Socket IPC for daemon patches (today only HTTP patch API), serial/UART/RS-485 sink (`/dev/ttyS0`), evdev touchscreen driver shared with 7.2b, sensor-to-UI flow (`event-htmx="gpio:relay_1=toggle"`), `no_std`/RTOS port of the core (links [8.S3](oxiscript.md#s-8-s3)).
- Gate: defined when scheduled. Depends: 7.2b; hardware after 2026-12-10 (O-22). Status: idea. Sources: R1-62..66.

### 5.3 Core, server and terminal

<a id="s-u-idle"></a>
#### U.Idle Idle wake-ups
- Scope: `next_deadline` returns `None` when nothing is pending (today `event_loop.rs:1258-1264` always returns now + 16 ms active / 1000 ms idle), so the driver sleeps; a dropped `try_send(Resize)` on a full queue must wake the loop. Prerequisite of battery/idle behaviour in browser-local mode and of OxiDE ([8.S1](oxiscript.md#s-8-s1) needs it for power).
- Gate: idle session measured at zero timer wake-ups; trace baselines unchanged except documented ones (lead blesses).
- Depends: 7.1 (done). Status: todo. Sources: R2-16; R4-34; R1-76.

<a id="s-u-s1"></a>
#### U.S1 OxiUI side of `<script lang="oxs">`
- Scope: parse `<script lang="oxs">` in `oxiterm-renderer`/`oxiterm-core`, the `host:ui` IDL ([8.S1](oxiscript.md#s-8-s1) owns the language and runtime side), instance-swap hot reload for pages (R3-44), page script running in server-driven and in browser-local mode with the same result (equality test), capability-scoped. No new `oxiscript-*` to `oxiterm-*` dependency except this interface (O-13).
- Gate: owned by 8.S1 (counter page driven by OxiScript in both modes; BAD ACTOR on capability escape).
- Depends: [8.S1](oxiscript.md#s-8-s1), [8.M-link](oxiscript.md#s-8-m-link), 7.1 (done), U.Idle. Status: todo. Sources: R2-62; R4-55; R3-44.

<a id="s-u-quirks"></a>
#### U.Quirks Pinned quirks and filesystem users
- Scope: decide each Phase 4 quirk the traces pin (do not fix silently): `<for>` re-expanded only on `StatePatched`, `current_page` not normalised, Enter pushes `'\r'` into predictive echo, `<for>` template node rendered, origin policy of `open_url` and token resync differs between App Server response and push endpoint; remove residual filesystem users (`source_path`/`base_source_path`, `Renderer::read_asset`, attach `page` param check); `0x21 update_document` cleared and resent every GUI frame on media pages (fix needs client + trace baseline change).
- Gate: each quirk resolved with a trace change blessed by the lead, or recorded as intended.
- Depends: none. Status: todo. Sources: R2-19,20,26.

<a id="s-u-cpu"></a>
#### U.Cpu Remaining CPU-side wins
- Scope: drop the redundant `CellGrid` capture in TUI `render_node` (5.1 recommendation 3, "perhaps 10-30% of render_node", needs measurement); skip the CellBuffer pass in GUI mode when no cell client needs it and cache legacy content (research step 1); zlib on the wire decision (5.1 recommendation 5, deferred to the 5.3 wire discussion); the unmeasured costs listed in Stage 5.0 (`apply_state_to_doc`, animation tick, focus rebuild, overlays); root cause of the Taffy stale cache (5.1b: "do not retry partial cache reuse" until isolated).
- Gate: before/after `frame_cost` numbers, goldens unchanged. Depends: none (measure first). Status: todo. Sources: R2-23,25,27,174.

<a id="s-u-damage"></a>
#### U.Damage Damage tracking for display-list sinks
- Scope: diff consecutive display lists into dirty rectangles for the wire, `oxiterm-fb` (partial framebuffer/DRM flush), e-ink and power; golden check that partial and full redraw give identical pixels. Not a GPU feature.
- Gate: triggered only if [8.Mem](oxiscript.md#s-8-mem) shows the full redraw too expensive on the Pi 1 B+.
- Depends: [8.Mem](oxiscript.md#s-8-mem), 7.2. Status: conditional. Sources: R2-14,70,173; R4-40; R1-77.

<a id="s-u-term"></a>
#### U.Term Terminal fidelity decisions
- Scope: close the unchecked terminal items with a decision record each: Unicode width (PUA/VTM modifiers: `insert_vtm_modifier` in `render/unicode.rs:74` has no non-test caller, `OSC 1337;UnicodeVersion` absent; shipped approach is glyph avoidance + width cache + W006), Kitty image chunking and ID reuse, key-release-driven interactions, latency-adaptive predictive echo (only server-side prediction exists), frame-rate tiers per emulator, emulator matrix and the named Linux Mint target, subtree dirty-flag layout, panic-free input policy and unsafe policy (inform, don't forbid, O-15), thousands-of-sessions load test (see U.Bench). Each item: implement, or record as dropped with reason in this file.
- Gate: every listed item has a verdict in section 6 or a test. Depends: none. Status: todo (small). Sources: R1-03,09,10,11,12,13,14,15,16,17,20,21,23,27,28,29.

<a id="s-u-a11y"></a>
#### U.A11y Accessibility beyond the web mirror
- Scope: SSH screen-reader users are on the linear `--a11y` sink only; `oxiterm-a11y` `DBusBridge` is a skeleton without transport. Decide and record: AT-SPI over D-Bus tunnelled through SSH AF_UNIX forwarding to the client's Orca (CD2 proposal) vs semantic skeleton vs linear only; native a11y comes with 5.7. Also Braille/speech.
- Gate: an owner decision (Q-6) then a stage per chosen route. Depends: 5.7 (native). Status: idea / owner to decide. Sources: R1-24,25,47; R4 none.

<a id="s-u-media"></a>
#### U.Media 7.6 leftovers
- Scope: Lottie process sandbox (an in-bounds document that aborts rlottie kills the SSH render path), cap SSH/Kitty/Sixel Rive raster (20 Mpx at 500x200), `Renderer::read_asset` descriptor checks, K3 attach navigation (`NavigateTo` vs `NavigateToAppPath`), `.mmd` size cap, advisory checks for rlottie and lottie-web 5.12.2 in the release checklist, Safari < 15.4 `connect-src 'self'` vs WebSocket, run `scripts/media_matrix.mjs` in CI (currently manual; needs short TMPDIR and PLAYWRIGHT_CHROMIUM); media DOM above z-index windows and whole-cell snapping stay documented limits until 5.6.
- Gate: each item a regression test; media matrix green in CI. Depends: U.CI. Status: todo. Sources: R2-152,153,154,155.

<a id="s-u-text"></a>
#### U.Text Proportional text follow-ups
- Scope: real bold and italic faces (only Regular bundled), lazy font fetch for WASM (fonts embedded, +0.82 MB raw), proportional inputs/caret/predictive echo/focus ring/scroll indicator (still mono CellGrid overlays), sub-cell scroll (`LayoutResult.total_height` is u16), `wrap: word` mono measure on uncollapsed text, focus-ring row off-by-one on fractional layouts ("Not reproduced", note at oxi-ui.md:551), determinism on a real browser engine (checked on WASI only) and DPR 2 in a real browser, showcase engine observations (GUI box background outside rounded border, diagram header cut, cell-mode stray dots above diagram labels, wheel scrolls 4 lines per event, generic code blocks cut with `~`), focus-ring audit of hidden `bind-show` controls. Visual check of every sans page under `?gui=1` is the owner's.
- Gate: per item; goldens re-blessed by the lead. Depends: 5.4 for caret. Status: todo. Sources: R2-34,35,36,37,38,39,140,141; R4-33.

<a id="s-u-apps"></a>
#### U.Apps App Server and apps
- Scope: persist the Spotify session map (300 s expiry, single-process poller), reader limitations (UTF-8 lossy decode, tables flattened, Polish diacritics trigger I007), Home Assistant bridge as an ordinary App Server (`state_changed` to patches, actions to service calls), media as display-list items vs overlay (rejected for now to avoid a wire bump), patch limits and `state` = bound keys documented in `app-server-guide.md`.
- Gate: per app. Depends: none (HA bridge pairs with U.Screens-2). Status: idea. Sources: R1-54,68,69,82,83,84,85,101,102,103,104,45.

<a id="s-u-secure"></a>
#### U.Secure Server-rendered high-risk surfaces
- Scope: IDEAS #8: login/PIN/payment as server-rendered surfaces (zero DOM, no secrets in the browser), WebAuthn bridge (`navigator.credentials` thin-client bridge), shuffled keypad, IEC 62443 kiosks; honest limits (phishing, keylogger, password managers) stay in the text.
- Gate: none; owner explicitly "not scheduled" (O-11). Depends: 7.1 or 5.3. Status: idea. Sources: R1-58.

### 5.4 Devices and screens

<a id="s-u-screens"></a>
#### U.Screens OxiScreens (dumb networked screens)
Sub-stages, all unscheduled until the board arrives (O-22). Phase numbers in `iot-screens.md` use the old OxiUI numbering; this section replaces them.
- **U.Screens-1** server: device identity token to page/size/group, device-to-page routing table, group patches `POST /groups/{name}/patch` (not in the code today), touch mapping, full-frame resend and a "disconnected" indicator, per-device tokens, revocation, LAN-only default. MVP uses tablet kiosks (works today). Gate: two devices in one group updated by one patch. Sources: R1-67,68,72.
- **U.Screens-2** Home Assistant bridge (shares U.Apps). Sources: R1-69.
- **U.Screens-3** ESP32 thin client (Rust, esp-hal/esp-idf): decodes `DiffEngine::encode_binary`, flash bitmap font, FT6236/GT911 touch, capability declaration on connect. Board: 800x480 (the `iot-screens.md` sizing example is 320x240 and predates the purchase). Depends: hardware after 2026-12-10, [8.S3](oxiscript.md#s-8-s3). Sources: R1-71,72.
- **U.Screens-4** e-ink client (partial refresh) and `<scene>` 3D tiers (see 9.x). Sources: R1-70,72; R2-171.
- Status: idea. Sources: R1-67..72; R2-88.

### 5.5 Phase 9: 3D in OxiUI (draft, not scheduled, O-10)

<a id="s-9-1"></a>
#### 9.1 Tier-2 server projection
Server-side projection to `Path`/`Text`, `<model src="x.glb">` element with GUI-only TCSS (`perspective`, `rotate3d`, camera), flat shading, picking as ordinary `event-htmx`. No GPU needed; works in the browser, OxiDE, ESP32 thin clients and coarsely in terminals (half-block/braille). Libraries `gltf`, `glam`. Gate: malicious meshes, polygon floods, NaN geometry (BAD ACTOR). Status: idea. Sources: R2-86; R4-38; R1-70.

<a id="s-9-2"></a>
#### 9.2 Tier-3 mesh and camera
Mesh once plus a camera matrix per frame to capable thin clients; server LOD; server-side headless wgpu `<scene>` tier-1 render + JPEG (software first)/H.264, render only on change, one render per (scene, resolution, camera); Pi 5 has no H.264 hardware encoder (do not repeat the NVENC/patent claims of the research as facts). Depends: 9.1. Status: idea. Sources: R2-86,171; R4-38.

<a id="s-9-3"></a>
#### 9.3 GPU path (local mode and native window)
Needs 5.2 (done) and the shared context (O-9); depends on [8.3c](oxiscript.md#s-8-3c). Status: idea. Sources: R2-86; R4-38,39.

<a id="s-9-4"></a>
#### 9.4 OxiScript 3D API and kernel shaders
Owned jointly: language part in [8.3d](oxiscript.md#s-8-3d). Status: idea. Sources: R2-86; R4-38. Slug-class/MSDF text for scalable or perspective labels only when 9.x needs it (R2-172).

### 5.6 Infrastructure, CI, sites, release

<a id="s-u-ci"></a>
#### U.CI CI and gates
- Scope: pin `runs-on: ubuntu-24.04` for golden stability (the `gpu` job and the others use `ubuntu-latest`); make the `release` job depend on `gpu` and add a tag trigger `v*` with `startsWith(github.ref,'refs/tags/v')` so an `oxiscript-v*` tag can never publish an `oxiterm-cli` release (today there is no `tags:` trigger and the release job is unreachable); `cargo deny check licenses` (allow-list MIT, Apache-2.0, BSD-2/3, Zlib, ISC, Unicode-3.0, MPL-2.0 flagged) and SBOM export (no `deny.toml` exists; wgpu is already a dependency; `cargo audit` checks vulnerabilities, not licences; covers the OxiScript crates too, [8.Sec](oxiscript.md#s-8-sec) links here); `csp_browser.mjs` and `media_matrix.mjs` in CI, the 500 ms Stop assertion flake in `csp_browser.mjs:217`; clippy `-D warnings` without `--no-deps` fails in `oxiterm-proto/src/pathsafe.rs` (check whether still true); watch `chrono` and `paste` advisories; advisory check of the wasm build prerequisites (wasm-pack before `cargo build`, documented in README only).
- Gate: CI green on a fresh push; a deliberately wrong tag does not release. Depends: none. Status: todo. Sources: R2-17,95,135,146,147,154,169; R1-41,55; R3-215.

<a id="s-u-sites"></a>
#### U.Sites Sites infrastructure and deploy
- Scope: rewrite `sites/DEPLOY.md` for Workers static assets (first ~60 lines still describe Pages Direct Upload); deploy `oxiscript.slavekm.pl` and `slavekm.pl` (only `oxiui.slavekm.pl` confirmed deployed); verify Cloudflare `_headers` semantics under Workers (the `check.mjs` rule "one splat per pattern" was recalled, not verified); HSTS (dashboard switch, 6-month max-age, `includeSubDomains` only if all subdomains HTTPS, no preload); PageSpeed Insights after deploy (the 8.W step-1 gate "Lighthouse measured" was not met; Lighthouse/vnu not installed), Search Console sitemap; roadmap generator input (see consistency.md: the generator must read `docs/roadmap/*.md`); decisions on run defaults (`oxi build` output relative to cwd, English truncation marker). Owner gates remain: Polish review (banner "draft"), home About text, Latin review, `--line` 3:1 contrast question. Site content and the course are [8.W2](oxiscript.md#s-8-w2)/[8.W3](oxiscript.md#s-8-w3)/[8.Blog](oxiscript.md#s-8-blog).
- Gate: all three sites live, `sites/check.mjs` and `sites/test/csp_browser.mjs` green, DEPLOY.md matches reality.
- Depends: owner reviews. Status: todo (partly blocked on owner). Sources: R1-39,106..111; R2-67,127,128,129,136,137,142; R4-60.

<a id="s-u-host"></a>
#### U.Host Public server
- Scope: a VPS with port-22 SSH and the web endpoint running `oxiterm-cli serve` (topology: deploy that binary to the admin host, manual restart); then the "no public SSH server yet" copy on the showcase pages and the `ssh app` marketing line become true. Document the deploy topology in the repo (today only in the lead's notes).
- Gate: `ssh` to the demo works from outside; limits (`OXITERM_MAX_SESSIONS`, trusted proxy) reviewed. Depends: owner buys the VPS. Status: blocked (owner). Sources: R2-162; R1-26; R2-140.

<a id="s-u-bench"></a>
#### U.Bench OxiUI measurements
- Scope: measured replacement for every unmeasured claim (RAM 2-15 MB, 0.0-0.1% idle CPU, < 0.5 s boot, 18-35 MB OxiDE, < 1 ms 200x50 diff, thousands of sessions per process, native RSS < 15-25 MB). The cross-product measurements on the Pi 1 B+ are [8.Mem](oxiscript.md#s-8-mem); this stage owns the OxiUI-only numbers and the server load test. Positioning claims on the sites wait for it.
- Gate: numbers table in `docs/research`. Depends: [8.Mem](oxiscript.md#s-8-mem), 5.3. Status: todo. Sources: R1-05,27,62,73; R1-76; R2-02,70; R4-82.

<a id="s-u-ver"></a>
#### U.Ver OxiTerm/OxiUI versioning and releases
- Scope: no OxiTerm release after 0.6.0 although crates changed massively (russh 0.65, core extraction); decide a versioning story (OxiScript has `VERSIONING.md`; `oxiscript/VERSIONING.md` says OxiTerm and OxiUI are versioned separately), tags `v*`, CHANGELOG structure ("Unreleased 2026-11-23" header duplicated).
- Gate: a `v0.7.0` (or owner's choice) released through the fixed CI job. Depends: U.CI. Status: todo (needs owner choice, Q-8). Sources: R1-30,32,170; R3-74.

<a id="s-u-docs"></a>
#### U.Docs Documentation clean-up
- Scope: apply the doc fixes of consistency.md (spring order, tick rates, `position` default, getting-started prerequisites, app-server-guide samples, README layout table, skill corrections, stage notes, HANDOFF). Move the two concept documents out of the repository root (`docs/history/`) with a "superseded" banner; fold `examples/spotify/README.md` (Polish, 6 lines) into `docs/spotify-demo.md`; link or delete the orphan `.agents/rules/how-to-comment-rust.md` (one wall of text); optional one comment-cleanup pass per crate with that rule; index `docs/stages/README.md` completely. Linter `lint_layout.py` stays under the skill.
- Gate: no document contradicts code on the items of consistency.md. Depends: lead review of this roadmap. Status: todo. Sources: R1-40,53,80..95,96,99,100,112,113; R2-01,159,165; R4 stale.

<a id="s-u-hygiene"></a>
#### U.Hygiene Repository hygiene
- Scope: prune merged worktrees (27 linked, 4 locked: `a402933a`, `a5c23966`, `a736d876`, `ad24f942`; only `gemini-rtlink`, `gemini-std`, `gemini-memres` hold unmerged or uncommitted work: do not delete `gemini-memres` before its research file is committed); keep stage notes of 8.C/8.P/8.R/8.M/7.1/7.2 that exist only in worktrees (`docs/stages/README.md`) before pruning, since the 8.S4 gate needs the 8.M findings.
- Gate: `git worktree list` shows only live work. Depends: none. Status: todo. Sources: R2-89; R4-80; R2 inconsistency 8.

### 5.7 Suggested sequence

1. Now, no dependencies: U.CI, U.Idle, U.Quirks, U.Hygiene, U.Docs, U.Term (decisions).
2. After the language critical path unblocks ([8.M-link](oxiscript.md#s-8-m-link) merge): U.S1 with [8.S1](oxiscript.md#s-8-s1); 7.3/7.4 in parallel (web client files); U.Hover (web part).
3. 7.2b, then 7.5; 5.3-5.5 (5.2 foundation done) with U.WebGPU; 5.6, 5.7.
4. After [8.Mem](oxiscript.md#s-8-mem) numbers: U.Bench claims, U.Damage if triggered.
5. After 2026-12-10 hardware: U.Screens-1..3.
6. Unscheduled: U.OxiDE beyond 7.2b, U.IPC, U.A11y, U.Secure, U.KernelDraw, 9.x.

---

## 6. Ideas not scheduled / dropped from the concept docs

| Item | Verdict | Sources |
|---|---|---|
| Whole-line-on-Enter (rlwrap style) input buffering, TLS/gRPC transport replacing SSH, SSH tuning (Compression, ClientAliveInterval, multiplexing) | dropped: prediction was chosen (CD2); owner to confirm | R1-13,14 |
| Bypass Diff directive | dropped: never implemented, CD2 calls it a trap; a width cache exists instead | R1-15 |
| PUA/VTM Unicode modifiers, `OSC 1337;UnicodeVersion` as critical fix | decision record in U.Term (glyph avoidance chosen) | R1-21 |
| Vest/formally verified parser, panic-free and unsafe-ban as stated rules | dropped: replaced by fuzzing + BAD ACTOR; per O-15 not forbidden | R1-23,28 |
| Linux Mint as named primary target, emulator matrix, per-emulator profiles | dropped as a target; capability negotiation kept; decision in U.Term | R1-09,10 |
| Kitty image IDs/chunking, key-release interactions, latency-adaptive prediction, frame-rate tiers, subtree dirty layout, dedicated `mio` thread | U.Term verdict; owner to decide | R1-11,12,16,17,20 |
| `display: none` and `ch`/`lh` layout units in TCSS | dropped for layout (not supported, `tcss-reference.md:195-198`); `ch`/`lh` remain in GUI font properties | R1-03,88,89 |
| AccessKit/AT-SPI tunnel for SSH users | U.A11y, owner to decide | R1-25 |
| OxiDE desktop features beyond 7.2b | U.OxiDE idea | R1-73..79 |
| Embedded blueprint: UART/RS-485 sink, evdev, UDS IPC, `no_std`, use cases (HMI, router console replacing LuCI, automotive NMEA/OBD) | U.IPC idea; use cases informational | R1-62..66 |
| OxiScreens: device routing, groups, HA bridge, `<scene>`, ESP32 and e-ink clients | U.Screens | R1-67..72 |
| `box-shadow` (silently ignored; appears in a TCSS `@media` example) | owner to decide; GUI shadow idea | R1-88 |
| Shared `plan()` crate, WebGL2 painter, glyph batching | U.WebGPU | R2-167,168 |
| Server-side GPU 3D render for screens | 9.2 | R2-171 |
| Slug/MSDF text | only when 9.x needs scalable labels | R2-172 |
| IDEAS #8 server-rendered high-risk surfaces | U.Secure idea | R1-58 |
| Rive as a real runtime: stays a procedural widget by design ("No Rive runtime", honest note) | informational | R1-60,61 |
| Media as display-list items (rejected for now to avoid a wire bump) | U.Apps, owner to decide | R1-45 |
| Mosh-style prediction as a client-side wrapper (CD2) vs server-side buffer shipped | informational; U.Term | R1-12 |
| AI acceleration (`host:ml`, tensor library) | [8.3e](oxiscript.md#s-8-3e) | R2-32; R4-13,14 |
| Phase 9 e-graph, other OxiScript research ideas | W-OXS | R4-15 |
| VS Code extension publishing | [8.Pub](oxiscript.md#s-8-pub) | R2-124 |

---

## 7. Open questions for the owner

| Id | Question | Source |
|---|---|---|
| <a id="q-1"></a>Q-1 | Confirm that the 2026-11-22 execution order (5.2-5.7 last, Phase 8 after Phase 7) is replaced by: Phase 8 and Phase 7 in parallel, 5.2 done, 5.3-5.7 after 7.x and the first OxiScript visible slice. | R2-09; oxi-ui.md:130-135 |
| <a id="q-2"></a>Q-2 | Order of 5.3-5.7 versus 7.3-7.5 and 8.S1: which comes first once 8.M-link merges? | oxi-ui.md:135; R4 plan-side items |
| <a id="q-3"></a>Q-3 | Visual check of every sans-serif page under `?gui=1` (owner's job, still open). | oxi-ui.md:499; R2-39 |
| <a id="q-4"></a>Q-4 | Version number and release story for OxiTerm/OxiUI (U.Ver). | R1-170; R3-74 |
| <a id="q-5"></a>Q-5 | Where does the public roadmap live after the old plan is deleted, and what does `sites/build.mjs` read (see consistency.md)? | R2-136 |
| <a id="q-6"></a>Q-6 | Accessibility route for SSH screen-reader users (linear only, AT-SPI tunnel, other). | R1-25,47 |
| <a id="q-7"></a>Q-7 | Which Unicode-width strategy is official (U.Term decision), and keep the Linux Mint/emulator matrix as a target or drop it? | R1-09,21 |
| <a id="q-8"></a>Q-8 | Review of the Polish copy, the home About text, the Latin copy and the HSTS decision (owner gates of U.Sites). | R1-107,109; R2-128 |
| <a id="q-9"></a>Q-9 | Buy the public VPS with port-22 SSH (U.Host) and when. | R2-162 |
| <a id="q-10"></a>Q-10 | Should OxiScreens, U.OxiDE and Phase 9 stay unscheduled until after the ESP32 board (2026-12-10)? The ESP32 board is 800x480, not the 320x240 of the blueprint. | R1-71; R2-86 |
| <a id="q-11"></a>Q-11 | Repository split timing (decision 7, owner decides after 8.S1; sites may move too). | oxi-ui.md:718; R4-70 |
| <a id="q-12"></a>Q-12 | Keep `box-shadow` silently ignored or schedule a GUI shadow? | R1-88 |
| <a id="q-13"></a>Q-13 | Gemini as default implementer (pilot happened: 5.2, rtlink, std)? | R2-157; HANDOFF.md |
| <a id="q-14"></a>Q-14 | OxiBench methodology approval and API keys (research/oxibench-methodology.md section 10) is on the OxiScript side: [8.OB](oxiscript.md#s-8-ob). | R2-161 |
| <a id="q-15"></a>Q-15 | The unmeasured RAM/CPU/boot claims in `embedded-iot-platform.md` and `retro-desktop-environment.md`: remove until measured (recommended) or keep as estimates? | R1-62,73 |

---

## 8. Working rules that matter for OxiUI

- **BAD ACTOR gate (O-8):** every stage ends with an adversarial agent that writes failing tests and a report and never fixes code; a separate fix agent fixes; findings stay as regression tests. Stage notes go to `docs/stages/<stage>/`.
- **Gates:** `scripts/oxiscript-gate.sh quick|merge` before merge and push; per-package cargo in worktrees (`-p ...`; a workspace build in a worktree fails on the missing web bundle); at most 3 concurrent agents, `CARGO_BUILD_JOBS=2`; nightly fuzz `.github/workflows/oxiscript-fuzz.yml`; check `gh run list` after every push (O-18).
- **Golden files:** agents never set `OXI_BLESS*`; the lead blesses after review (O-17). GPU goldens are blessed only on the verified llvmpipe pairs (LLVM 20 / Mesa 25.2, LLVM 21 / Mesa 26.0); tolerance 2 / 0.25%. Stage changes that alter wire or traces say so explicitly (7.6 was a wire break: server and client ship together).
- **Never** touch `/dev/fb0`, `/dev/dri/*`, `/dev/input/*` from an agent (O-16).
- **Permissive licences only** (O-14); SBOM and `cargo deny` come with U.CI.
- **Clicks that miss:** three independent causes (ambiguous-width glyph in a label, SSH emitter cursor desync, web canvas pixel-to-cell mapping); check each before blaming layout.
- **Scratch and disk:** `/` is nearly full; agent TMPDIR and scratch go to `/vectorlegis_ssd_pool/oxiterm/scratch`, never `/tmp`.
- **Shared files** (plan/roadmap, root `Cargo.toml`, CI workflows) are edited by the lead only; skills are checked against code, docs are not; where a skill and a document disagree, code decides.
- **Deploy:** the server is `oxiterm-cli serve`, deployed by copying that binary to the admin host with a manual restart; sites are Workers static assets deployed by the lead with `wrangler` after the gate.

---

## 9. Traceability appendix

Inventory ids that are not cited in a row above, and where each one is handled. Ids not listed here are cited in the `Sources:` cells. Counts are in the final report of the writer.

| Inventory ids | Destination |
|---|---|
| R1-01, R1-02, R1-18, R1-19, R1-22, R1-29, R1-46, R1-49 | Done, "Terminal baseline" row (thin client SSR, THTML tag set, bounded channel, XON/XOFF, synchronized updates, reattach/takeover, dynamic tick); the three tick numbers are a doc fix in consistency.md; release-loss recovery of SGR 1006 is part of U.Term |
| R1-31, R1-43, R1-44 | Done (7.1, 7.2, 5.2 and ARCHITECTURE sections 3-4 reference; Android shell 7.3, native window 5.3) |
| R1-37, R1-38, R1-40, R2-10, R2-21, R2-156, R2-158, R2-164 | sections 2 and 8 (O-8, O-14, O-17, O-18) |
| R1-42, R1-86, R1-87, R1-90..94, R1-97 | U.Docs and consistency.md (README layout table, guide wording, spring/position/units statements, skills) |
| R1-51, R1-52 | Done, "OxiDESK window features" row |
| R1-59 | U.Screens |
| R1-108, R1-110 | U.Sites (CSP/no trackers kept as O-20; PageSpeed checklist) |
| R1-33, R1-34, R1-35, R1-36, R1-50, R1-98, R1-32 | OxiScript: [oxiscript.md](oxiscript.md) (laws overclaim R1-33 and `oxi` scope R1-34 are in its consistency half) |
| R2-03 | section 1 non-goals |
| R2-04, R2-05, R2-06, R2-07, R2-08 | Done rows 3-4 and 7.1; O-4, O-5; D3 wording in consistency.md |
| R2-18 | U.Media (native media replacement is covered by 7.6; video not) |
| R2-24, R2-41, R2-43, R2-49 | Done rows (5.1, 6), 7.5 |
| R2-26, R2-174 | U.Quirks, U.Cpu |
| R2-51 | Run, identity, showcase: [oxiscript.md](oxiscript.md) (8.Run, 8.Id) and Done "Showcase" |
| R2-52..R2-85, R2-87, R2-90..R2-94, R2-96..R2-99, R2-100..R2-123, R2-125, R2-132..R2-134, R2-138, R2-139, R2-160, R2-163 | OxiScript stages and decisions: [oxiscript.md](oxiscript.md) (R2-87 risks table: its OxiUI risks are all obsolete; new risks: wgpu churn, lavapipe golden drift, Cloudflare header semantics, usage limits) |
| R2-130, R2-131 | [8.W2/8.W3](oxiscript.md#s-8-w2), [8.Blog](oxiscript.md#s-8-blog); infra half in U.Sites |
| R2-166 | O-5, 5.2 Done |
| R2-175 | rule: research numbers marked unverified (NVENC, patents, Zed/Warp/Ghostty details) are not repeated as facts (9.2 text) |
| R4-01..R4-12, R4-16..R4-20, R4-50..R4-54, R4-56..R4-59, R4-61..R4-69, R4-71 | OxiScript (8.3a-e, 8.Bench, 8.M-link, 8.Std, 8.Perf, 8.S*, decisions): [oxiscript.md](oxiscript.md) |
| R4-32 | O-5 |
| R4-35 | U.Quirks (filesystem users), U.Media; CI wasm32/Android job is done (`ci.yml`) |
| R4-36 | Done (7.1, 7.2, 7.6) |
| R4-42 | U.Quirks (`0x21`), U.Cpu (CellGrid capture) |
| R4-81 | section 8 (scratch/disk) |
