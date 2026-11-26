# Showcase: the "view this page in OxiUI" versions of the three sites (implementation notes)

Owner brief: the OxiUI version of each site was "a plain column of text with `- item` lists and one button", poor for what
OxiUI can do. Stage 7.6 had just restored media in every mode and the identity (rust + patina, Oxi) was applied. Only `sites/**`
changed; no engine crate was touched.

## What each page shows (index page of each site, desktop 80 columns and `_mobile` 36 columns, EN + PL + LA kept in i18n)

| Site | Showcase |
|---|---|
| OxiScript | hero with the vector axolotl (SVG `<img>`); the compiler pipeline `.oxs -> parse -> type check -> OxiIR -> Wasm -> wasmi` as a Mermaid `<diagram>`; "works today / not yet" cards (rounded borders, patina and coral); three tabs (`bind-show` on one state key) with the real `hello`, `errors`, `shapes` examples; an accordion "where it differs from TypeScript" (5 rows, `toggle:`) |
| OxiUI | hero + axolotl; a small desktop of three draggable windows (`draggable`, `drag-handle`, `z-index` raised by click through `bind-z-index`): counter, switch (`bind-show`), dashboard with spring-animated bars (`bind-width` + `transition: width 450ms spring(...)`, three presets); effects gallery: single/double/rounded borders, colour swatches, bold/italic, SVG, Lottie (bell, plays by itself), the Rive toggle (clickable); a section about the same page over SSH |
| slavekm.pl | hero + axolotl; project cards with a closed and an open (pressed) state, quiet |

All other pages (`ts`, `tour`, `how`, ...) still come from the generic block renderer; its lists are now marker + wrapping text rows instead of `- item` lines, and buttons are content-sized (they were stretched to the page width).

## Files

* `sites/showcase-thtml.mjs` (new): the three showcases. `sites/build.mjs`: media plumbing, `thtmlPage` hands the index page to the showcase.
* `sites/shared/showcase/bell.json`, `toggle.riv`: copies of `examples/assets` (the site must not break when the examples change).
* `sites/i18n/{en,pl,la}.json`: `<site>.showcase.*`, `common.mascotAlt`. Removed: `oxiui.demo.*` (the old demo page) .
* Removed `sites/demo-thtml.mjs`: the old OxiUI demo page (counter, toggle, mode) is now part of the OxiUI index page; the "live demo" iframe widget and the app start page both open `<lang>/index.thtml`.
* `asciiThtml()` (identity stage) is no longer used by any THTML page: the ASCII axolotl is replaced by the SVG one on the index pages (the HTML pages still use `asciiHtml()`).

## Media on the static site (the risky part)

* A page's media `src` is relative to the page (7.6 rule). Pages are `en/index.thtml`, so `src="../<hash>/file"` resolves to `<hash>/file` in the virtual root.
* `buildMedia` (build.mjs) writes SVG, `bell.json`, `toggle.riv`, per-language `pipeline-td.<lang>.mmd` (generated from i18n, so the labels are copy) and, for OxiUI, the vendored `lottie_light.min.js` + licence into ONE content-hashed directory `/app/<hash>/`. `headers()` already emits an immutable rule for every `/app/<10 hex>/*`, so no `_headers` change was needed and `check.mjs` (one Cache-Control per path, one CSP per path) passes unchanged.
* `patchClient`: the asset prefix `'/oxiui/local/asset/'` becomes `'./'` (the relative path the core asks for already starts with the hash directory); `'oxiui/vendor/lottie_light.min.js'` becomes `'./<hash>/lottie_light.min.js'`. The stale "no site page uses Lottie" comment was fixed.
* CSP: `/app/*` already allowed `script-src 'self'`, `img-src 'self' data: blob:`, `connect-src 'self'`. Lottie checked under the real CSP: `dataset.lottie === 'playing'`, `currentFrame` advances, no `securitypolicyviolation`, no console message.

## Decisions and engine facts learned

* **Page width 80, not 100 (or 96).** The client grid is 98 columns at a 1200 px window but only 84 in the live-demo iframe of the HTML pages (the iframe is 960 px wide), and a wider page is clipped on the right (first the axolotl at 100, then the dashboard window and gallery cards at 96, found by opening the HTML index and pressing "Start the live demo"). 80 fits both. Phones use the `_mobile` 36 column file (client threshold `innerWidth < 800`).
* **Diagram fit rule** (`render_diagram`): the diagram draws one header line (alt text, node/edge count, size) and fits only when `cols <= width` and `rows <= height - 1` (inside its own border), otherwise it falls back to a minimap of white blocks. A 6 node LR chain is 113 columns wide (does not fit 94), the TD chain is 20 x 30; so TD, box `height: 33` with a rounded border. The alt text is short ("Pipeline") because it is the header and is cut at the box width.
* **No `:hover` in TCSS** (hover exists only for Rive). Project cards use a press state instead: a click toggles `home_<id>` and swaps a quiet card for a double-border rust card with the description and a button. Keyboard users get the engine's focus ring (`>` `<`).
* **Tabs without duplication**: tab A is `set:os_tab=false` with `bind-show="os_tab=false"` (false on first paint, and the Str "false" is falsy); B and C are `set:os_tab=b|c`. State keys are prefixed per site (`os_`, `ui_`, `home_`).
* **Progress bars** animate on click (presets), not by themselves; Lottie is the self-playing piece.
* GUI media overlays draw above `z-index` windows (7.6 limit), so no media is inside the desktop; on phones the windows are stacked cards (no absolute positions).
* `align-self`, hover and `gap` do not exist: the linter (E001) would catch them; 0 errors on all 83 generated pages.

## Verification

Harness (not committed, in /tmp): `pages.json` extracted to a tree that mirrors the static site's virtual root and served with `oxiterm-cli serve <tree>/start.thtml --local-mode` (so the app base is the tree root; serving `en/index.thtml` itself would make `../<hash>/` escape the base and look like an engine bug), plus the dist served with its real `_headers` for local mode. Own builds only (`wasm-pack` for the web bundles, `cargo build -p oxiterm-cli`, `sites/build.sh` full).

* Clicked in local mode: tabs A->B->C->A (line count 84 -> 102 -> 98 -> 84), accordion open/close, counter, presets (bars animate), switch, drag, click raises a window, Rive toggle (turns blue), project cards open/close.
* Lottie under the real CSP: `dataset.lottie === 'playing'`, `currentFrame` advances (42.5 -> 30.0 a second later, loops), no `securitypolicyviolation`, no console message.
* `sites/build.sh` (full): wasm x3, `build.mjs`, `check.mjs` ok, `cargo test -p oxiscript-playground` ok (all suites 0 failed).
* `sites/test/*.mjs`: all pass, including `csp_browser.mjs` (28 pages + demos + playground under the real `_headers`, 0 failures; run alone, it times out when the box is busy with cargo) and `bad_actor_contrast.mjs`. `bad_actor_run_parity.mjs` needs `BAD_ACTOR_CORPUS=<file>` produced by a cargo test: not run (not a page test).
* New colour pairs (not all in `bad_actor_contrast.mjs`) computed separately: fg/muted/pat/coral/acc on card and bg >= 4.5 on all three palettes; `#1a0b03` on the rust title bar 7.02.
* `lint_layout.py` on all 83 generated files (three sites, EN and PL, desktop and mobile, plus the gallery demos): 0 errors, 0 warnings, 89 infos (Polish letters in clickable labels, as before).
* The ~2 MB figure of `common.demoHint`: local wasm 5,005,641 raw / 1,997,161 gzip, client js + css, `lottie_light.min.js` 168,236 raw / 46,334 gzip (lazy, only when a Lottie node is on the page), `pages.json` of OxiUI 184 KB raw / 17.4 KB gzip. The copy said 1.5 MB; now "about 2 MB" in EN, PL, LA.

## Weight (raw bytes, everything except wasm; same metric as the identity notes; the baseline used the same wasm-pack glue, byte-identical)

| Site | Before | After | Growth | Of which |
|---|---|---|---|---|
| home | 213,439 | 220,800 | +7.4 KB | axolotl SVGs 5 KB in `/app/<hash>/`, pages.json |
| oxiui | 454,725 | 654,582 | +199.9 KB | lottie 168 KB + licence, bell 7.8 KB, SVGs, showcase THTML (the old `demo*.thtml` are gone) |
| oxiscript | 453,261 | 506,562 | +53.3 KB | pages.json 230 KB (EN+PL, desktop+mobile), 4 `.mmd` |

## Screenshots

`/tmp/claude-1000/-vectorlegis-ssd-pool-oxiterm-oxiterm/6830e8c3-c411-43eb-ad3c-c468305978e8/scratchpad/showcase-shots/` (30 files): `<site>_local_{1200,390}_{top,mid}.png`, `<site>_local_1200_pl_mid.png` (static dist, local mode), `<site>_srv_{cell,gui,local}_1200_mid.png`, `<site>_srv_cell_{1200,390}_top.png` (OxiTerm server; cell = the web cell client, not a real SSH terminal).

## Engine observations (reported, not fixed)

* GUI: a box's `bg` fills its whole cell rectangle outside the rounded border (dark rectangles around windows and cards).
* The diagram paints its own navy background and always draws a one-line header (alt, counts, size) that is cut at the box width (phones: "(2").
* Web cell mode: stray dots above some diagram node labels ("OxiIR`", "Wa<s with dot>m"): a cell-mode glyph/junction artefact, not seen in GUI or local.
* The wheel scrolls 4 lines per event; page height on phones is long because the diagram needs 30 rows.

## Not done

* No real SSH terminal was used (the web cell client was). The "SSH" copy claims only that the same file is served over SSH.
* TCSS has no `:hover`, so "hover" is not shown (press state instead).
