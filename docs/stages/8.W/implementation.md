# 8.W step 1 handoff

Status: step 1 built and gated (see "Gate"). Three sites + playground crate + build/check scripts + DEPLOY.md.

## Layout
- `sites/build.sh` (wasm + pages + checks + cargo test), `sites/build.mjs` (generator, no deps), `sites/check.mjs` (i18n parity, link/anchor/THTML-nav check, HTML nesting, "no Latin leak"), `sites/demo-thtml.mjs` (live-demo THTML template), `sites/DEPLOY.md`.
- `sites/i18n/{en,pl,la}.json`: ALL strings, one file per language (en = source). `sites/shared/site.css` + `site.css.d/<site>.css` + `site.js`. `sites/oxiscript/{examples/*.oxs, ts-compare/*.ts, js/playground.js}`.
- `oxiscript/oxiscript-playground`: `check/format/apply_fix/explain/hosts/registry_count` (JSON strings, UTF-16 ranges, fixes applied in Rust), 7 unit tests + `tests/site_examples.rs` (every example checks and is canonically formatted; `OXS_REFORMAT=1` rewrites; `mistakes.oxs` must fail and its fix must apply).
- Output: `sites/dist/{home,oxiui,oxiscript}` (gitignored, with `sites/.build/` wasm cache).

## Decisions made
- Languages: PL + EN published; Latin written but excluded by default (`SITES_LANGS=pl,en` default; `pl,en,la` enables). Dedication/Latin-note widgets are skipped while `la` is off. `en` is the site root (`/`), others `/pl/`, `/la/`.
- Every page has "view this page in OxiUI": generated THTML (+ `_mobile`, 36 columns) from the same i18n blocks; static copy of the 7.1 web client (`oxiterm-server/assets/index.html`) with local mode forced and URLs rewritten to `./pages.json`; no WebSocket fallback; Google Fonts link removed.
- Live demos are click-to-start iframes (the 3.6 MB wasm is not loaded until asked).
- Roadmap/wave log parsed at build time (originally from docs/history/oxi-ui-plan-history.md, now from docs/roadmap/oxiui.md and oxiscript.md) (status derived from the wave log; if the table format changes, the build fails loudly).
- Home page text is placeholders in `[PLACEHOLDER]` / `[DO UZUPEŁNIENIA]` form; no biography facts, no name, no dedication text invented. Dedication is quiet (serif, no animation, no emoji).

## Latin (DRAFT, awaiting review; not published)
Terms I was unsure about: perlustrator (browser; alternatives navigatrum, interretiale perlustrum), tabula (screen), cafeum (coffee), crustula (cookies), computatrum (computer, used for the build machine), "agentes intellegentiae artificialis" (AI agents), laqueus (trap), remedium (fix), annales undarum (wave log), iter (roadmap), palaestra (playground), inferendi gradus (lowering stage, calque), uniones discriminatae, "classes basales", "serie litterarum" for string, testa (shell, Android), cliens (client), servus (server), "puncta et virgulae" (semicolons), "capsae Rust" (crates), "clicationes" (clicks). Please review the whole file; the code, identifiers and error codes are intentionally untranslated.

## Gate (measured on this machine)
- `cargo test -p oxiscript-playground`: 7 + 1 pass. `cargo clippy -p oxiscript-playground --all-targets -- -D warnings`: clean. wasm32 release builds of oxiterm-web (local) and the playground: ok.
- Sizes: oxiui wasm 3,632,253 B raw / 1,488,377 B gzip; playground wasm 563,950 B raw / 238,951 B gzip. HTML pages 4.5-44 KB raw (2-7 KB gzip), css 10 KB (3 KB gzip), playground.js 6 KB. Per-site dir: home 3.8 MB, oxiui 4.0 MB, oxiscript 4.6 MB.
- Headless Chromium screenshots checked: oxiui home (light), oxiscript playground with the `mistakes` example (diagnostic underline, gutter marker, apply-fix and explain buttons), Latin home on a 390 px viewport, OxiUI demo/docs/todo/showcase pages running in local mode. Lighthouse and an HTML validator (vnu) are NOT installed: not measured; run PageSpeed Insights after deploy.

## Not done / follow-ups
- Run button (needs 8.L). Course lessons (only the landing/outline). Latin review. Lighthouse numbers. Fill in the home placeholders. The playground has no per-call panic isolation (wasm abort would need a page reload; message shown).
- Edits outside my dirs: none (only new files under `sites/` and `oxiscript/oxiscript-playground/`; no change to oxiscript-check/syntax).
