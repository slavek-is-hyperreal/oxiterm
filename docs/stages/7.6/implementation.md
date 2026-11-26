# Stage 7.6 - media and diagram parity across modes (implementation notes)

Owner question: were OxiTerm features lost when OxiUI (display list, local mode) was added? Baseline
(lead, main `1b08076`, headless Chromium 1200x800) and the reproduction by `scripts/media_matrix.mjs`
(before screenshots in the scratchpad `shots/before/`):

| Page / feature | cell (default) | GUI `?gui=1` | local `?local=1` |
|---|---|---|---|
| showcase | OK | OK | OK |
| media: SVG, PNG `<img>` | EMPTY | EMPTY | OK |
| media: Lottie | EMPTY | EMPTY | EMPTY |
| media: Rive | EMPTY | EMPTY | EMPTY |
| diagram | EMPTY | EMPTY | EMPTY |

## Root causes (evidence)

1. **Web cell and GUI, SVG/PNG/Lottie: not a phase 3/4/7.1 regression, broken since Plan 3.0 (`ea5f8ea`,
   2026-07-24).** `WsFrameSink::sync_preamble` (`oxiterm-server/src/web.rs`) has two safety checks per media
   node. The asset-send loop was changed by `ea5f8ea` to check against `app_base_dir`; the coordinate loop
   (0x21) kept checking against the *page directory*. The same commit moved the demos into `examples/demos/`
   with `../assets/...` sources. Result: every sibling-directory asset got its bytes (0x20) but never a
   position (0x21), so the overlay was never created. (Before `ea5f8ea`, `media_demo.thtml` used same-directory
   `mascot.svg`, which passes both checks.) Local mode has its own lexical check (`media_rel_path`), which is
   why SVG/PNG worked there.
2. **Lottie.** Web cell/GUI: cause 1, plus the client loaded lottie-web from a CDN `<script>` (no network =
   no player). Local: `/oxiui/local/asset/` served only images/video; `.json` was deliberately refused, so the
   file never arrived (404).
3. **Rive.** Never worked on the web: the renderer returned early for `profile.is_web` before the `.riv`
   branch (media is a DOM overlay on the web) and the client has no Rive runtime, so even with coordinates the
   `.riv` blob would have fallen through `detectMediaType` to `image/png`. `ARCHITECTURE.md` section 4 claimed
   it is a DOM overlay; that was never true.
4. **Diagram.** Four stacked causes: (a) `diagram.thtml` wrote `src="examples/assets/flow.mmd"`, a path relative
   to the server's cwd, while every other `src` (and the renderer, and local mode) is relative to the page's
   directory; (b) the safety check reported a missing file whose parent directory does not exist as "path
   traversal blocked" and drew nothing; (c) local mode has no `app_base_dir`, no file system
   (`Path::exists()` is always false on wasm) and no `.mmd` in the browser; (d) the demo page puts a header and
   a main box in a `<screen>` whose default direction is row, so the diagram box started at column ~90 and was
   mostly off-screen in every mode.

## Fixes

* One resolver, `oxiterm_proto::pathsafe::resolve_media` (+ `media_rel_path`, `normalize_lexical`): a media
  `src` is relative to the directory of the page that holds it (HTML rule); absolute paths, schemes, backslashes,
  NUL and percent-encoded `.`/`/`/`\` are refused; the result must stay inside the app base lexically, and on
  disk (nearest existing ancestor canonicalised, so symlinks out of the base are refused, also for files that do
  not exist yet). A missing file is a placeholder, not a traversal. An empty app base is the browser's virtual
  root (lexical only). Used by the renderer (img, video, diagram), both loops of `sync_preamble`, the Rive click
  handler in the core (same key as the renderer's playback state) and the local-mode client.
* Rive: `render/rive.rs`, a pure-Rust rasterizer for the procedural toggle (replaces the resvg drawing, same
  geometry). Web clients get it as half-block cells in every mode (cells, GUI `CellGrid`, local); SSH keeps
  Kitty/Sixel/half blocks. `.riv` is not sent as a media overlay.
* Client (`oxiterm-server/assets/index.html`): lottie-web "light" (MIT, SVG renderer) vendored at
  `oxiterm-server/assets/vendor/`, served at `/oxiui/vendor/lottie_light.min.js` (+ licence text), loaded lazily
  and same-origin; no CDN. A coordinate refresh reuses the playing element (animations no longer restart on
  every refresh).
* Local mode: `.json`/`.mmd` are served by the asset route only when an indexed page names exactly that file as
  an `img`/`video`/`diagram` `src`; `LocalSession` asks the page to fetch diagram files (`0xE0`), the page mounts
  them into the core's virtual file system (`mount_asset`) and the core re-renders.
* Pages: `demos/diagram*.thtml` use `../assets/flow.mmd` and a column `<screen>`.

* Client race fixed along the way: the web client wiped the media layer when the *first frame* arrived, but the
  server sends a frame's `0x21` positions *before* its first cell/list message, so overlays vanished about 40% of
  the time (found by the matrix flaking). Media is now cleared at connect (`ws.onopen`), not at the first frame.

## Tests (the regression net)

| Layer | Where | What it pins |
|---|---|---|
| resolver | `oxiterm-proto/src/pathsafe/media_tests.rs` | page-relative rule; `../`, absolute, `%2e`/`%2f`/`%5c`, schemes, backslash, NUL, symlinked file/dir (also for a missing file below it), no base, virtual root |
| cell sink (SSH + web cell) | `oxiterm-renderer/tests/media_parity.rs` | diagram draws the real labels of `flow.mmd`; old root-relative src shows a placeholder; every refused src draws nothing for `diagram`/`img`/`video`; Rive as cells and its state; Kitty `\x1b_G`, Sixel `\x1bP`, half blocks for PNG/SVG/Lottie/Rive; web leaves raster to the overlay |
| web transport | `web.rs::test_ws_frame_sink_media_in_a_sibling_directory_gets_bytes_and_coordinates` | `../assets/x.png` gets both `0x20` and `0x21`; refused sources get neither (the test that would have caught 2026-07) |
| local session | `oxiterm-web/tests/media_local.rs` (`--features local`) | `0xE0` requests and `0x21` positions for svg/jpg/json, none for `.riv`; Rive cells and click toggle; diagram requested, mounted, drawn |
| browser | `scripts/media_matrix.mjs` | page x mode: DOM overlays decoded and sized, Lottie animating (`currentFrame` advances), Rive drawn and toggling, diagram area non-empty, canvas busy, video element present |

SSH terminals: verified at the cell-sink level (the ANSI emitter is not exercised; the sink output the emitter
consumes is). Not verified in a real terminal.
Video: the matrix only asserts the `<video>` element and its size (Playwright's Chromium has no H.264).

## Known limits / not done

* (Fixed after BAD ACTOR, see `fixes.md` F-12) The Rive playback state is per session now.
* (Fixed after BAD ACTOR, see `fixes.md` F-11) The WebSocket `page=` parameter is percent-decoded once by the server.
* Wire change after BAD ACTOR (`fixes.md` F-01/F-09): media keys are 64-bit (0x20/0x21/0xE0 carry 8 key bytes).
* GUI overlays sit above `z-index` windows and snap to whole cells (section 7c of ARCHITECTURE.md).
* The `native-media` off build still draws no SVG/Lottie/video itself (the browser does).
* Goldens that change (not blessed here; the lead blessed them, see `d52bb89`): `oxiterm-renderer/tests/golden/` `demos__diagram@{80x24,120x40}`,
  `demos__diagram_mobile@{80x24,120x40}` (page fixed: header on top, diagram box on screen),
  `demos__media@{80x24,120x40}`, `demos__media_mobile@{80x24,120x40}` (the Rive widget is now drawn; the old
  baselines pinned the empty box).
