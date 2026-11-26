# Stage 7.6 BAD ACTOR fixes (notes, updated after every fix)

Order follows `bad-actor.md` recommendations. All tests release, `--offline`, `CARGO_BUILD_JOBS=2`.

## F-13 Rive web raster (done)

* `oxiterm-renderer/src/render/rive.rs`: `toggle_half_block_image(w, h, bits)` caches the half-block raster per
  `(w, h, bits)` (content-only key: no path, no session; 64 entries, cleared when full) and caps the raster at
  1.5 Mpx (`WEB_RASTER_BUDGET_PX`) by lowering the oversampling above it. Normal widgets keep 8x16 per cell, so goldens and
  the exact-colour parity asserts do not move. `renderer.rs` web branch uses it.
* Test: `rive.rs::web_raster_is_capped_and_cached`; `bad_actor_76::rive_at_the_largest_viewport_is_bounded`
  (`--nocapture`).
* Numbers, 496x198 cells, web profile: **before 0.68 s then 0.67 s; after 92 ms then 1.9 ms**. SSH/Kitty path unchanged
  (0.97 s once, then 1.5 ms from the asset cache; its raster is not capped, see "not done").

## F-01 / F-09 media keys (done)

* One key: `oxiterm_proto::pathsafe::media_key` (FNV-1a 64) of the file's path **relative to the app base**
  (`pathsafe::app_rel` of the resolved path). Used by `web.rs::hash_path` (server), `oxiterm-web/src/local.rs::hash_path`
  (local mode: of the page-resolved `rel`, which it already used for `requested_assets`), and the browser.
* Wire change (**protocol break, server and client ship together**): 0x20 = `[0x20][key:u64le][bytes]`, 0x21 =
  `[0x21][key:u64le][x i16][y i16][w u16][h u16]` (17 bytes, was 13), 0xE0 (local) = `[0xE0][key:u64le][rel]`.
* `sync_preamble` resolves first, then compares/sends by the resolved key (`sent_assets: HashSet<u64>`,
  `sent_coordinates: HashMap<u64,_>`). Side effect: a refused `src` no longer makes `coords_changed` true on every
  frame (the old length comparison counted refused entries).
* `index.html`: `mediaKey()` ("lo:hi" string), `resetMediaCache()` (revokes object URLs) on `ws.onopen`; no per-navigation
  reset is needed because the key names the file.
* Tests: server `same_src_on_another_page_gets_its_own_bytes`, `distinct_srcs_have_distinct_keys`; proto
  `media_key_is_64_bit_and_path_based`, `app_rel_is_below_the_base`; web `hash_matches_the_server_function`,
  `media_local.rs` (offsets/keys). Harness change (not a weakening): the `drain` helper and the web.rs unit tests read
  an 8-byte key (`[1..9]`, payload from 9) because the wire key is 8 bytes now.

## F-02 / F-03 / F-04 / F-07 / F-08 / F-10 / F-05 (done)

* `oxiterm-proto/src/pathsafe.rs` (libc added as a unix-only dependency; wasm32 build unaffected):
  * `open_media_checked` / `read_media_checked`: `symlink_metadata` regular-file pre-check, open with
    `O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC`, then on the descriptor: regular file, size <= `MAX_MEDIA_BYTES` (16 MiB),
    not multiply linked (`meta_multi_linked`, moved here, `OXITERM_ALLOW_HARDLINKS=1` honoured; the server re-exports it),
    and `/proc/self/fd/N` must lie below the canonical base (covers a swapped directory in the middle of the path);
    fallback without `/proc`: canonicalise again and compare dev/inode. Read is `take(max+1)`.
  * `resolve_media` also refuses an existing multiply-linked file (F-02; a missing file has nothing to check).
  * F-05: `MAX_SRC_BYTES = 4096` in `media_src_is_plain`, and `canonical_prefix` is one pass (one `symlink_metadata`
    per component until the first missing one, one `canonicalize`). A dangling symlink as the last existing component
    now fails closed.
* `web.rs::sync_preamble`: reads bytes only through `read_media_checked`, and only for names accepted by
  `local_mode::media_name_servable` (no segment starting with `.`; the extensions of the asset route plus `.json`).
  Deviation noted: **one allow-list for every element type** (`MediaRenderInfo` has no element tag; only `img`/`video`
  reach `active_media`, so `.mmd`/`.riv` are not in it).
* F-10: `local_mode::open_asset` (the same opener, `MAX_ASSET_BYTES`) used by the `/oxiui/local/asset/` route through
  `spawn_blocking` + `tokio::fs::File::from_std`.
* Test corrections (the original lines did the unsafe operation themselves, so no product change could pass):
  * proto `resolve_then_read_is_not_raceable`: `std::fs::read(&p)` replaced by `read_media_checked` (what the sink does);
    added a swapped-directory case.
  * server `asset_route_symlink_swap_between_resolve_and_open`: `File::open` + metadata-only `check_opened_asset`
    replaced by `lm::open_asset`; added a swapped-directory case.
  * server `fifo_does_not_block_the_sink`: the cleanup `OpenOptions::write(true).open(fifo)` blocks forever when nobody
    reads (that is what hung the whole test binary once the sink stopped blocking); it now runs only if the sink blocked.
* Tests: proto `bad_actor_76` (F-02, F-03, F-05), `media_tests::{src_length_is_capped, checked_open_refuses_fifo_oversize_hard_link_and_non_files}`;
  server `bad_actor_76` (F-02, F-04, F-07, F-08, F-10).

## F-12 per-session playback state (done)

* `PlaybackRegistry` is keyed by `(scope, path)`; **every setter and `get_or_create` takes the scope** (the unscoped
  forms are gone, so the core cannot forget it). Scope = session id: `TerminalProfile::playback_scope` (new, `serde(skip)`, set
  per frame in `event_loop.rs::render_frame`) for the renderer, `session.id` in `update_interactive_animations` for hover/click.
  Local mode runs the same core code (its click test in `oxiterm-web` stays green). `EventLoopGuard` carries the scope and calls
  `drop_scope` when the loop ends; the map is capped at 4096 entries (at the cap other scopes are dropped).
* Still shared, content-only: `AssetCache` (Kitty/Sixel bytes by path+size+state bits), the Rive web raster (size+bits), `LottieCache`
  (parsed document, path).
* Test adaptation: `bad_actor_76::rive_state_is_per_session` (A clicks in scope 1, B renders in scope 2; also asserts A sees its own
  click) and `media_parity::rive_widget_is_drawn_as_cells...` (clicks in scope 0, the scope of `web()`), because the setters take a scope.
* Behaviour change: Lottie pacing (`start_time`) is per session now; Rive toggle state no longer survives an event-loop restart.

## F-06 Lottie guard (done, process abort not sandboxed)

* `render/cache.rs`: `validate_lottie` runs before rlottie sees a document (size <= 4 MiB; serde_json recursion limit rejects the bracket
  bomb; `w`/`h` in 1..=4096; finite `fr/ip/op`, <= 1e6 frames; <= 20 000 layers; `refId` graph acyclic via iterative DFS, depth <= 64;
  <= 200 000 expanded layer instances, memoised, so a doubling DAG is refused; <= 5 000 vertices per bezier path and 40 000 in all,
  <= 1 000 star points). `parse_lottie_guarded` = validate + `rlottie::Animation::from_data`.
* Measured with the child harness: one path of 1k..10k vertices 0.01..0.06 s, 20k vertices >100 s (a cliff), hence the vertex cap
  (the 4 MiB size cap alone is not enough: 19k points is a ~0.5 MB document).
* `LottieCache`: parsed **outside** every shared lock; keyed by path, revalidated by `(len, mtime)`; a rejected file is remembered and not
  re-parsed twice per frame per session (it used to retry forever); 64 entries. The Lottie state left `PlaybackState`, so the
  global registry mutex is never held across a parse or a render. A render holds only that animation's own mutex.
* Test: `bad_actor_76::lottie_hostile_documents_in_a_child` (child process; the child now calls `parse_lottie_guarded`, so it tests the
  product guard instead of bypassing it; `REJECTED` is a pass; two new cases `dag_blowup`, `points_near_cap`), unit tests
  `cache.rs::lottie_guard_tests`.
* Review round: cycle detection now starts from **every** asset (an unreachable self-referencing precomp is refused) and duplicate asset
  ids are refused (which asset a `refId` means would be the engine's choice). Child harness outcomes (`LOTTIE` lines, `--nocapture`):
  `huge_dims` TooComplex("size"), `frames_1e9` TooComplex("time"), `deep_precomp` RefCycle (the 3000-deep chain is stopped by the
  64-level depth cap and reported under the cycle label), `many_layers`/`bracket_bomb`/`huge_shape_points` TooLarge (> 4 MiB),
  `nan_inf`/`truncated` Malformed, `layer_ref_cycle` RefCycle, `unreachable_cycle` RefCycle, `duplicate_id` Malformed,
  `dag_blowup` TooComplex("expanded layers"), `points_near_cap` TooComplex("path vertices"). (Whether rlottie itself aborts on the
  two new shapes was not tested; they are refused regardless.) The harness used to print empty outcomes because libtest prefixes the
  child's line; it now matches with `contains`.
* Left: no process sandbox. A document inside every bound that still aborts rlottie (C++ `abort`/`SIGABRT` cannot be caught in-process) would
  still kill the server; the known abort (self-referencing precomp) and the known hang are pre-validated. `Renderer::read_asset` still
  uses `std::fs::read` for the SSH renderer path (containment comes from `resolve_media`, no descriptor checks there).

## F-11 `page=` (done)

* `web.rs::decode_page_param`: the WS `page=` value is percent-decoded exactly once by the server (`local_mode::percent_decode`), then
  refused when it is empty, absolute, has a NUL or backslash, or a `..` segment; the existing `is_within_base` check still runs on the
  result (symlinks). `%252e` decodes to the literal text `%2e` (a plain file name), never to `.`.
* Test: `web.rs::test_decode_page_param_decodes_once_and_refuses_encoded_traversal` (`demos%2Fmedia.thtml` accepted, `..%2fx`,
  `%2e%2e/x`, `%2E%2E%2F`, `a%2f..%2f..%2fx`, `%2fetc%2fpasswd`, `%5c`, `%00`, malformed escapes refused, `%252e...` stays literal).
  Browser check: `scripts/bad_actor_76_deeplink.mjs` (see the verification section).

## F-14 security headers (done)

* `web.rs::apply_security_headers` on every 200 of the static routes: `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: SAMEORIGIN`; `/` and `/index.html` get a CSP, `/oxiui/vendor/*` gets `default-src 'none'; frame-ancestors 'self'`.
* CSP of `/`: **no `'unsafe-inline'`**. The page has exactly one inline `<script type="module">` and one inline `<style>`, both allowed by
  SHA-256 computed at serve time from the bytes sent (`inline_block_hashes`; works with the web-test injection too). No inline event
  handler attributes and no `style=` attributes exist in the HTML (only `el.onclick = fn` and CSSOM `.style` writes, which CSP does not
  restrict). What the policy has to allow, and why: `'wasm-unsafe-eval'` (the WebAssembly core / local mode), `https://fonts.googleapis.com`
  (style) and `https://fonts.gstatic.com` (fonts) because the page links the Google Fonts stylesheet, `blob:` for media object URLs
  (`img-src`, `media-src`), `data:` for images, `connect-src 'self'` (same-origin fetch and WebSocket; Safari before 15.4 does not treat
  `'self'` as covering `ws:`). Also `object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'`.
  Not checked in a browser other than the matrix's Chromium.
* Test: `test_security_headers_on_index_and_vendor`.

## F-15 (done)

* Lottie sniffing (`index.html::detectMediaType`) skips a UTF-8 BOM and leading whitespace before the `{`.
* `mount_asset` (local mode): at most 64 files and 4 MiB in all (plus the 256 KiB per file), via `VIRTUAL_FS.insert_capped`.
* `VIRTUAL_FS`: direct `get` / `contains` / `insert_capped` on the global `RwLock` (no whole-map clone); `read_asset` and `asset_exists`
  use them. `VirtualFsProxy::with` (used by the playground `mount_asset` and a few tests) is unchanged and still clones.

## Provenance (done)

* `oxiterm-server/assets/vendor/PROVENANCE.txt`: lottie-web 5.12.2 (version string read from the file), upstream URL, MIT, SHA-256 of the
  file and of the licence, with the explicit statement that the hash was **not** compared with upstream (no network).

## F-11 follow-up: the redundant client navigate (found while verifying in Chromium)

Decoding `page=` made the server start straight on `demos/media.thtml`. Two things then tried to navigate to the page the session was
already on, relative to the *current page's directory* (`demos/demos/media.thtml`, "blocked", error chip): the attach path
(`K3`, which sent `NavigateTo` before the loop was spawned) and the client's own 0x31 right after connecting. Fixed in `web.rs`: K3 only
navigates a loop that is already running; a 0x31 naming the current page is dropped. Result with `scripts/bad_actor_76_deeplink.mjs`:
the announce is `demos/media.thtml` first, the log has no "page parameter blocked" / "blocked path traversal" lines (the matrix runs
used to log 94 of them), `?page=..%2F..%2Fetc%2Fpasswd` is refused and the start page loads.

## F-16 matrix flake (investigated)

* Cause (consistent with every observation, not reproduced before the fix): the Rive toggle/hover/click state was one process-global map.
  The matrix drives 12 page loads against one server and clicks the widget in each mode; any leftover or concurrent click (a previous
  context still closing, the click of the previous mode) flipped the state a later row had just read, so "did not toggle on click"
  appeared about 1 in 4 full runs. Now every WebSocket/local session has its own scope (F-12).
* `scripts/media_matrix.mjs` now also fails a row on any console "Refused to ..." / "Content Security Policy" message, so the CSP (F-14) is
  validated by the same runs.
* **Five full runs in a row on the final binary: 12/12 OK each (after the last code change)** (plus one earlier green run on the first build of the new CSP and
  keys). Chromium 1193 via `PLAYWRIGHT_CHROMIUM`; the matrix needs a short `TMPDIR` (the browser profile path in the long scratchpad
  directory makes Chromium die with SIGTRAP: unix socket path length), so the matrix runs used `TMPDIR=/tmp/oxi76`.

## Verification (release, `--offline`, `CARGO_BUILD_JOBS=2`)

| Command | Result |
|---|---|
| `cargo test --no-fail-fast -p oxiterm-proto` | 28 passed (20 lib + 8 bad_actor_76) |
| `-p oxiterm-renderer` | 189 passed, 0 ignored (incl. golden_cell_snapshots, media_parity, bad_actor_76) |
| `-p oxiterm-core` | 98 passed |
| `-p oxiterm-server` | 189 passed, 0 ignored |
| `-p oxiterm-web --features local` | 48 passed |
| `-p oxiterm-cli` | rc 0 |
| `cargo build --release -p oxiterm-core --target wasm32-unknown-unknown` | OK |
| `wasm-pack build ... oxiterm-web -- --no-default-features` and `... --out-name oxiterm_local ... -- --features local` | both OK (the bundles in `oxiterm-server/assets/pkg` are rebuilt: the local client speaks the 64-bit keys) |
| goldens | unchanged: no file under `oxiterm-renderer/tests/golden/` was touched, `golden_cell_snapshots` passes against the committed baselines. The 8 goldens listed in `implementation.md` (diagram/media, 80x24 and 120x40, plain and mobile) are still the lead's to bless. |
| `scripts/media_matrix.mjs` x5 | 12/12 OK, 5 of 5 |

Clippy (`--all-targets`, default lints plus the repo's pedantic set): no warning in the code added by this round after fixing `open_media_checked`; the remaining
warnings in touched files are pre-existing (`pathsafe.rs` `is_within_base` let-else, the BAD ACTOR repro files `bad_actor_76.rs` format!-in-iterator and by-value
argument, `bad_actor_local.rs`, `test_rlottie.rs`).

## Behaviour changes to know about (WS media path)

* Media files over 16 MiB (videos included) no longer reach web clients over the WebSocket; the refusal is logged as one WARN per file per
  connection (`warned_assets`), a missing file stays a debug line. Local mode serves up to the same cap through its asset route.
* A symlink as the *last* path component (a file symlink inside the base) is refused by `O_NOFOLLOW`, so it is not sent to web clients while SSH
  still renders it. Directory symlinks that stay inside the base keep working.
* A hidden (`.x`) or non-media `src` is no longer sent (one WARN per connection, not per frame).
* `resolve_media` refuses an existing multiply hard-linked file for every consumer (renderer too), unless `OXITERM_ALLOW_HARDLINKS=1`.
* `Renderer::read_asset` (SSH render path, SVG/Lottie/PNG) now needs a regular file of at most 32 MiB (`metadata`, follows symlinks), so a FIFO named
  `x.svg` no longer blocks the render thread or the `SvgCache` lock. This is a plain metadata check, not a descriptor check: a path swapped to a FIFO
  between the check and the read still blocks (write access to the app directory needed).

## Ship together

`oxiterm-server/assets/index.html`, both rebuilt bundles in `oxiterm-server/assets/pkg` (`oxiterm_web*`, `oxiterm_local*`) and the server binary (the
0x20/0x21/0xE0 key is 8 bytes now). `Cargo.lock`: `libc` (unix-only dependency of `oxiterm-proto`), `sha2` and `base64` (server, CSP hashes),
`serde_json` behind the renderer's `native-media` feature. No page in `sites/`, `examples/` or `docs/` embeds the live server in an `<iframe>`
(grep), so `frame-ancestors 'self'` / `X-Frame-Options: SAMEORIGIN` breaks no known embedding; an external site that framed the app will now be blocked.

## Not done / left

* Lottie: no process sandbox; an in-bounds document that still aborts rlottie would take the SSH render path down (browser web clients draw
  Lottie in lottie-web, untouched; a hostile Lottie hanging the *browser tab* was never in scope).
* SSH/Kitty/Sixel Rive raster is not capped (0.97 s once, then the asset cache; 20 Mpx at 500x200 cells, the terminal size is chosen by the SSH client).
* `Renderer::read_asset` (SSH render path): metadata check only (above), no descriptor checks; containment comes from `resolve_media` (now incl. hard links).
* K3 (`web.rs`, attach to a *running* loop with a sub-directory `page=`) still sends `InputEvent::NavigateTo`, which resolves relative to the current page's
  directory (`demos/demos/...`); `NavigateToAppPath` is the app-root-relative event. Pre-existing; not changed. No automated WS-level test pins the
  F-11 follow-up (checked manually in Chromium with `scripts/bad_actor_76_deeplink.mjs`).
* `.mmd` diagrams are read by the renderer with no size cap (parser caps 64 KiB / 256 nodes).
* The web client still treats duplicate files on one page (the same file twice at different positions) as one key (pre-existing).
* An advisory check for rlottie and lottie-web 5.12.2 is for the release checklist (no network here).
* `Safari < 15.4`: `connect-src 'self'` may not cover the WebSocket; not tested.
