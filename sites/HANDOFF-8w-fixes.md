# 8.W BAD ACTOR fixes (F1-F12)

Scope: only `sites/**` and `oxiscript/oxiscript-playground/**`. No git, no network, no deploy.

| id | status | what / where |
|---|---|---|
| F1 | done | `sites/i18n/pl.json` `meta.status = "draft"` (banner shown). Flip to `"reviewed"` after review (documented in DEPLOY.md "Owner review gates"). |
| F2 | done | Home copy is neutral; About block behind flag `home.about` (the Latin note and dedication are separate widgets, only rendered when Latin is enabled) (`SITES_FLAGS=home.about`), honoured in HTML and THTML. `check.mjs` fails on `[PLACEHOLDER` / `[DO UZUPE` in any text output unless `SITES_ALLOW_PLACEHOLDERS=1`. |
| F3 | done | `apply_fix(src, diag, fix, code, start, end, title)` in the playground crate re-checks and verifies code + UTF-16 span + fix title, else `{ok:false, kind:"stale"}`. `playground.js` disables fix buttons on every `input`. |
| F4 | done | `setText()` edits through `execCommand('insertText'/'delete')` on the changed middle only: Ctrl+Z undoes Format and Apply fix (browser-tested). Example switch with edits shows an inline confirm (`#pg-confirm`), no `window.confirm`. |
| F5 | done | `check.mjs` resolves GitHub `blob|tree/main/<path>` links against the working tree, checks file vs directory. `repo:llms.txt` -> `repo:docs/oxiscript/llms.txt` (en/pl/la). Allowlist `PENDING_8D` = `AGENTS.md`, `docs/oxiscript/llms.txt` (TODO: delete after the 8.D merge). |
| F6 | done | `_headers`: CSP per HTML page + `/app/` + `/app/*` (non-overlapping, because Cloudflare merges matching rules), nosniff, `Referrer-Policy: strict-origin-when-cross-origin`, Permissions-Policy. 7.1 client's inline script/style moved to `/app/<hash>/client.js|css`. HSTS step in DEPLOY.md. `test/csp_browser.mjs`: 28 pages + demos + playground in headless Chromium with the real `_headers`: 0 CSP violations, 0 console errors. |
| F7 | done | `patchClient()` maps every `localStorage`/`sessionStorage` to an inert `noStore`; 0x33 `window.open` restricted to http(s)/mailto. `check.mjs` fails if the output names Web Storage / cookies / IndexedDB. Footer wording now "...and nothing is stored in your browser" (true, enforced by the check). |
| F8 | NOT fixed here | Formatter comment re-attachment is an `oxiscript-syntax` bug: record for **8.P**. Repro: `node sites/test/bad_actor_format.mjs` (`[1, // one\n 2, // two\n]` -> `// two` ends up after the statement; `/* b */` jumps over `+`). Meaning of code never changed. |
| F9 | done | New token `--ctl` (form-control border, >= 3:1 on bg/card/code-bg in all 6 modes); oxiui light `--acc` `#15803d` -> `#137536` (>= 4.5 on code-bg); roadmap h3 -> `h2.sub` (h1 -> h2); `div.bar.projects` -> `<nav aria-label>`. `bad_actor_contrast.mjs`: all pairs pass. |
| F10 | done | `safeUrl()` (http, https, mailto, relative; strips control chars/space, lowercases; rejects `//`) used by `resolveLink` (so `inline()` and `links` blocks and THTML `open:`). `check.mjs` also scans dist for non-allowed schemes. event-htmx: the runtime splits on `;`/`,` and has NO escape syntax (`oxiterm-core/src/state.rs`), so `demo-thtml.mjs` fails the build on a `set:` label containing either. |
| F11 | done | Wasm `apply_fix` takes `f64` indices (wasm-bindgen's integer glue would coerce NaN/1e300 to 0) and validates finite/integer/in range (`kind:"range"`). UI messages localised (`staleFix`, `fixFailed`, `tooLargeTitle`, `tooLarge`, `discardAsk`, ...) incl. "Source too large" via `kind:"too_large"`; PL/LA pages show one line "compiler messages are English". |
| F12 | done | DEPLOY.md: keep-list (MX, TXT incl. SPF/DKIM/DMARC + verification, CAA with the CA caveat, NS, SRV), checks for Redirect Rules / Bulk Redirects (account level) / Page Rules / Workers routes / Origin+Transform rules, exact apex instructions (Pages custom domain on the apex, CNAME flattening). |
| fmt | done | `cargo fmt -p oxiscript-playground` (pre-existing line-240 failure gone). |

## Test flips (assertions that asserted the defect)
- `oxiscript/oxiscript-playground/tests/bad_actor_8w.rs::stale_fix_index_applies_a_different_fix` now asserts `ok:false, kind:"stale"` for the stale button, plus positive cases (unchanged text fixes the right line; an edit below still works; a wrong fix title is refused).
- `sites/test/bad_actor_wasm.mjs`: `apply_fix` calls use the new 7-argument signature; the NaN / 1e300 / 2**53 / -1 / 0.5 / Infinity tail now FAILS the run if any index is accepted, and checks matching-ok / stale-refused.
- `sites/test/bad_actor_contrast.mjs`: the pair `line on bg >= 3` was retargeted to the control-border token (`ctl` on bg, card, code-bg >= 3). `--line` is the decorative card/rule colour (exempt from WCAG 1.4.11); all other pairs untouched. Decision for the lead: if you prefer `--line` itself to be >= 3:1 everywhere, say so (it makes every card border heavier).
- `unit test fix_is_applied_in_rust` and `tests/site_examples.rs` use the new `apply_fix_json(src, diag: f64, fix: f64, &FixExpect)` signature (API change: `FixExpect` is exported).

## New
- `sites/test/csp_browser.mjs` (needs `playwright-core`; default path `/my_data/VectorLegis/.ocr-wasm-work/web/node_modules/playwright-core`, override `PLAYWRIGHT_CORE`; system `/usr/bin/chromium`).

## Owner / lead decisions
- PL stays `draft` until the owner reads it. About block stays off until the owner writes it.
- After the 8.D merge: remove `PENDING_8D` in `sites/check.mjs` and the "(being added by the DevKit stage)" label suffix.
- HSTS: Cloudflare dashboard only; do not enable Preload.
- The `/app/` rule is duplicated for `/app/` and `/app/*` (identical value) because it is not certain Cloudflare's `*` matches an empty remainder; an identical duplicate policy is harmless.

## Final verification (all green)
- `sites/build.sh` full (wasm + pages + check + cargo test), then `build.sh --pages` after the last edits; `check.mjs` ok (also with `SITES_LANGS=pl,en,la SITES_ALLOW_PLACEHOLDERS=1`).
- `cargo test`, `clippy --all-targets -D warnings`, `fmt --check` for oxiscript-playground: ok. wasm32 release builds ok.
- `test/csp_browser.mjs`: 28 pages + demos + playground, 0 CSP violations, 0 console errors; stale-click, Ctrl+Z (Format, Apply fix), inline confirm all pass on `/` and `/pl/`. A real 404 gets its CSP from a `<meta>` in 404.html (Cloudflare matches `_headers` on the requested path).
- `test/bad_actor_wasm.mjs` (349 runs, 0 traps), `bad_actor_contrast.mjs` (0 FAIL, plus assertion that controls use `--ctl`), `bad_actor_format.mjs` (only the known F8 behaviour).
- Weights (raw / gzip): home index 4,810 / 1,774; oxiui index 5,424 / 2,224; oxiscript index 9,399 / 3,710; site.css 10,045 / 3,110; playground.js 9,507 / 3,561; app/index.html 2,277 / 834; app client.js 56,370 / 13,755; client.css 9,042 / 2,244; pages.json (oxiui) 141,439 / 14,732; oxiui wasm 3,632,253 / 1,490,286; playground wasm 564,624 / 240,420. Per site dir: home 3,822,969; oxiui 4,048,178; oxiscript 4,578,528.
