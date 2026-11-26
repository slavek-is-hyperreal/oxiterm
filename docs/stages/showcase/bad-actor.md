# BAD ACTOR: showcase (commit 0177e5d) - report only

Repros: sites/test/bad_actor_show_csp.mjs (CSP/console in all frames, laziness, 3 sites x EN/PL x 1200/390), sites/test/bad_actor_show_ui.mjs
(driver: STEPS/VW/VH/SITE/CPU env). Hostile-i18n builds ran in a scratch copy (/tmp/oxiba-show/repo), never in sites/dist.

| id | sev | finding |
|---|---|---|
| SH-1 | medium | Code samples on the OxiScript tabs lose their indentation when drawn (markup has the spaces; the engine strips leading whitespace of a text node), so "the real examples" show `log.write` at column 0. On phones 20 of 47 code lines are cut with `~` (first line of every sample: `import { Log } from "host~`); desktop 1 of 47. |
| SH-2 | medium (copy) | "This page is a single .thtml file ... `oxiterm-cli serve index.thtml`": the page needs 6 media files in `/app/<hash>/` (`../<hash>/x`) and the file is not offered for download; a reader cannot reproduce it from the copy. SSH serving itself works (checked: `oxiterm-cli serve en/index.thtml --no-auth` + `ssh -tt`, page rendered, images sent as kitty graphics). |
| SH-3 | low (copy) | "Rive toggle": `toggle.riv` is a 12-byte stub (`RIVE\x07\0\0\0\1\2\3\4`, identical to examples/assets), not a real Rive artwork; alt says "drawn by the engine" (honest) but the title and the "Rive" claim oversell. Renders as a blocky 14x3 cell block in GUI. |
| SH-4 | low | `pipelineMmd` filter forgets `\n`: a label with a newline injects whole Mermaid lines (verified: `click A href javascript:alert` and `subgraph X` land in the .mmd). The engine's mermaid parser has no click/href support, so impact is layout only, and i18n is trusted input. Add `\n\r` to the regex. |
| SH-5 | info | Idle CPU 6-7 % TaskDuration (headless, software) on EVERY app page incl. the static home; Lottie visible adds ~3 points (9.9 %), back to 7.1 % when scrolled away (it pauses). Springs idle at baseline after settling. So the showcase adds nothing at idle; the 6 % floor is the client loop (pre-existing). |
| SH-6 | info | Diagram stray dots in cell mode not reproduced: my SSH capture did not reach the diagram (no terminal emulator available). Implementer's rating stands: cosmetic. |

## Held up
* Lottie player in dist = PROVENANCE SHA-256 (23cd2c01...08f3), no `eval(`/`new Function`; no `https://` in client.js/css; only `window.open` is the http(s)/mailto-guarded one; bell.json = examples copy, `assets: []` (no external refs).
* CSP: header set unchanged by the stage (git diff of build.mjs); 0 violations/console messages in 24 loads (3 sites x EN/PL x 1200/390, `/app/` direct) plus the interaction runs (counter, presets, drag, Rive click, Lottie, tabs by Tab+Enter, 820/900 px).
* Injection: 4 hostile payloads (`</text><box event-htmx=..>`, `" event-htmx="open:javascript:..`, `;set:ui_z_c=99 {ui_n} {{x}}`, newline+click) in every showcase/mascotAlt string, EN+PL: tag counts of all generated pages identical to baseline; `tx()` escapes `& < > "`; no `{}` interpolation in THTML text. Mermaid labels with `[](){}"|;<>` abort the build.
* PL: build prints no "missing key" warning; PL showcase pages render; scratch build == dist byte for byte (`diff -rq` empty).
* Laziness: HTML index loads no pages.json/wasm/lottie/riv before "Start the live demo" (oxiscript HTML index loads its playground wasm, pre-existing); bell.json/lottie not fetched at first paint; app first load = pages.json + wasm + one SVG (+ .mmd on OxiScript).
* Behaviour: counter, presets (bars spring), Switch, accordion rows and Tab+Enter work; drag clamps inside the 22-row desktop (a window cannot be lost); click raises; 820 px and 900 px fit 80 columns; phone shows stacked cards. Tab reaches tabs, accordion rows and buttons, no trap seen.
* Regressions: build.mjs + check.mjs ok; lint_layout 0 errors/0 warnings on EN+PL for all three trees; bad_actor_{contrast,format,run_borders,run_loadtimeout,run_rendered,...} ran without failure (run_lib/stress/timeout/states/workercsp/wasm were not read to the end; run_parity needs a corpus; csp_browser not rerun).
* pages.json: oxiui 184,253 B, oxiscript 229,990 B, home 76,200 B raw; only fetched by `/app/`.

## Not done
Real focus-ring audit of hidden (bind-show) controls, wide fuzzing of the engine, full build.sh (wasm), rating of cell-mode diagram dots.
