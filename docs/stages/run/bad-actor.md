# BAD ACTOR: RUN + IDENTITY (branch integ-sites)

Work in progress; findings are appended per surface. Bundle provenance: `sites/.build/pkg_*` were copied from the run
worktree (`agent-a460e6951f1910b61`, HEAD e896888); `git diff <that HEAD> HEAD -- oxiscript/` is empty, so the browser
bundle matches this branch's Rust source. Pages were rebuilt here with `node sites/build.mjs` (no cargo).

Reproducers: `oxiscript/oxiscript-playground/tests/bad_actor_run.rs`, `sites/test/bad_actor_run_{lib,lazy,stress,timeout,parity}.mjs`
(PLAYWRIGHT_CORE=... node ...; parity needs BAD_ACTOR_CORPUS from the Rust test), `bad_actor_run_{rendered,states,borders,disabled,workercsp,bigpaste,leak}.mjs`, `oxiscript/oxiscript-cli/tests/bad_actor_run.rs` (5 `#[ignore]`d reproducers, all fail today). `cargo test -p oxiscript-playground` and `cargo clippy -p oxiscript-playground -p oxiscript-cli --all-targets -- -D warnings` are green with the new files (the 20 s fuzz test is `#[ignore]`d).

## Surface 1-3: browser runaway, isolation, parity (done)

* HELD: runner is lazy (no runner file, no `modulepreload` before the first Run; after Run: runner-worker.js + runner js + 2.3 MB wasm).
* HELD: 9 runaway programs (string doubling, 100M array pushes, endless recursion, O(n^2) concat, `while(true)`, host-call loop, state-open loop,
  i64 array, array self-concat): every one ends in 50-330 ms with the right stop reason (fuel / out_of_memory / stack_overflow), zero main-thread long tasks, Run usable again.
* HELD: Stop spam, Run x3 clicks, 15 run/stop cycles, a fresh Run after Stop works.
* HELD: the 5 s timer path, exercised for real with a patched page constant (route rewrite): the worker is killed, status says "Stopped after N s", the next Run respawns a worker and works (cold run ~480 ms).
  No real program reaches 5 s (fuel 10 M always wins in <= 330 ms); the timer is only the safety net for compile time.
* HELD: nesting bombs are refused by check (OXS-E0011 at 56 levels / 1000 chained) before anything runs, so a 240 KB chain and 20 000 nested parens cost 60-130 ms on the main thread
  (longest main-thread task 73 ms for the chain, 169 ms for a 194 KB / 4000-function file).
* HELD: log text `<img onerror>`, `<script>`, RTL override, ESC/BEL/NUL: rendered with `textContent`, no element injected, `window.__pwn` undefined; ESC/BEL/NUL arrive pre-escaped (`\x1b`, `\x07`, `\x00`) from the runtime. 0 CSP violations, 0 page errors.
* HELD: isolation (27 hostile programs, native): reserved export names `oxi_init`/`memory`/`oxi_abi` rejected (OXS-E0702), duplicate export names rejected, `host:clock/fs/log@2` rejected (E0400), `rust:`, `./x`, `../../etc/passwd`, `/etc/passwd`, `std/math` unresolved (E0302), `1 as Log`, `{h:1}` as Log and `new Log()` rejected (E0111/E0120/E0119), forged state slot rejected by type (E0100). Oversized key/message (> 64 KiB) trap as host_denied.
* HELD: mutation fuzz, 4000 mutated example programs (fuel 200 k), 0 panics (20 s, debug build).
* HELD: parity. 67 programs (7 site examples + 60 generated: arithmetic with overflow/wrapping/saturating, casts, f64 to i32, slices of a multibyte string, indexing, `${f64}`) through native `run_json` and the REAL runner wasm in Chromium: 67/67 byte-identical JSON including `fuelUsed`; and `oxi run` (its own compile path via `project::compile_entry`, no manifest) on the same 67 files: stdout lines, exit code (0 / 4 / 1) and the `[OXS-E04xx]` trap code identical in 67/67 (`oxi run` stdout carries levels 0-2 only) (clean 17, rejected 11, overflow 19, bounds 13, unreachable 5, bad_utf8 2).

## Surface 4: CLI (`oxi run` / `oxi build`), debug build from this branch

* HELD: `--fuel` garbage (negative, > u64) is a clap error (exit 2); `--fuel u64::MAX` accepted (user's choice; the CLI has no wall-clock limit).
* HELD: FIFO and `/dev/zero` as the entry or as an import: "not a regular file", exit 2 immediately; absolute / `../` imports and a symlink to `/dev/zero` inside the root: E0302 (the 8.D F1/F15 fixes did not regress under `run`/`build`).
* HELD: ESC / OSC 52 / OSC 0 / CSI / C1 (0x9b, 0x85) / CR in guest log text arrive escaped (`\x1b]52;...`) on stdout and stderr: no terminal injection.
* HELD: missing host interface from a project `idl/` (blink template): `oxi run` exit 2 with the "use oxi build" hint; `oxi build` works. 300 KB source accepted by the CLI (the page caps at 200 000 chars).
* HELD: non-UTF-8 `-o` name written byte-exact (`bad\xff\xfe.wasm`); `-o` a directory: exit 2; `-o /dev/full`: exit 2.

## Surface 5: identity (rendered, headless Chromium, real `_headers`)

* HELD: CSP and 28 pages + demos + playground, 0 failures (`csp_browser.mjs`); `bad_actor_contrast`, `bad_actor_wasm` (349 runs), `bad_actor_format` all pass, no regression of the 8.W/8.D fixes (import root escape, `/dev/zero`, FIFO).
* HELD: rendered contrast, not token arithmetic (`bad_actor_run_rendered.mjs`): 50 page loads (3 sites, every page incl. PL and 404, light + dark), 4116 visible text elements vs their effective background: 0 below 4.5 (3 for large text); elements with a background image are skipped, which includes the gradient hero h1 on oxiscript (endpoints fg 15.97 / acc 5.73 light, 15.54 / 6.80 dark, so every point of the gradient is >= 5.7); the only hits are the transparent `.pg-mirror` overlay (intentional). Control borders as rendered (`bad_actor_run_borders.mjs`, WCAG 1.4.11): 0 below 3:1 (oxiscript controls 4.08 light / 4.62 dark, accent buttons 5.2-7.0); the 8.W F9 note is fixed. 8.W F2 (placeholder text on home) is gone from the built pages. Focus ring: solid 3 px, 5.7-6.8:1 against the page, in both schemes, on every site (`bad_actor_run_states.mjs`). Disabled Run: opacity .55 (exempt, still legible). Gradient hero h1 stays visible with `forced-colors: active` (Chromium).
* HELD: no inline `style=`, `<style>`, inline script, `on*=` on any of the 25 pages; the 5 brand SVGs have no script/foreignObject/style/href/use/image; they are `<img>` only. 0 horizontal overflow at 320 and 390 px on every page, also with a 60 000-character log line in the output panel. No running animations at all (reduced motion moot). THTML (84 generated files incl. ASCII axolotl): `lint_layout.py` 0 errors, 0 warnings (70 info: pre-existing ambiguous-width PL letters in labels).
* HELD (Chromium only): the worker script is served without its own CSP header, yet a `fetch` to another origin from inside it was blocked (0 hits on a probe server), so in this browser it is constrained by the page policy; Firefox/WebKit untested. Recommendation: an explicit CSP header on `/assets/*/runner-worker.js` as defence in depth. No `message` listener on the page; the worker is reachable only from its creator.
* Page weight before the first Run: 624 KB raw (check/format wasm 568 775 B + 55 KB html/css/js/svg); after Run: +2 311 006 B wasm (783 507 gzip) + 12 KB.

## Surface 6: honesty of copy

Checked against this build: "instruction limit and a 5 second time limit" (true, limit tested), "in a worker" (true), "Output appears when the program stops" (true), "`log` and `state` available" (true), "f64 cannot be converted to text" (true, parity corpus shows `unreachable`), "Other host interfaces are not available in the playground" (true: E0400), "Loading the compiler (about 0.5 MB)" (true: 568 775 B), "No trackers, no cookies, no third-party scripts, nothing stored in your browser" (true: 0 storage use in site JS; `check.mjs` enforces it for pages), "compiles to WebAssembly" (true: `oxi build`).
Not true / misleading: see F7 below.

## Findings

| id | sev | surface | one line | repro |
|---|---|---|---|---|
| F1 | medium | CLI build | A repo controls where a plain `oxi build` (no `-o`) writes. (a) Strongest form: git keeps symlinks, so a project can ship `demo.wasm -> ../victim` beside `name = "demo"`; `oxi build` follows it and overwrites `victim` (verified, a file outside the project is replaced by wasm bytes). (b) `[package] name` is not validated: `name = "../../escape"` creates `../../escape.wasm` (only `*.wasm` names, since the suffix is appended). SCHEMA.md says the default is "in the current directory". | `cargo test -p oxiscript-cli --test bad_actor_run -- --ignored plain_build_does_not_write_through_a_shipped_symlink build_default_output_stays_inside_cwd` |
| F2 | low-med | CLI run | Output goes through `println!`/`eprintln!`: `oxi run prog.oxs \| head -1` or `> /dev/full` panics ("failed printing to stdout: Broken pipe"), exit 101 plus a panic banner. 101 is not in the exit-code table (0/1/2/4). | `... --ignored run_closed_stdout_is_not_a_panic`; shell: `oxi run flood.oxs \| head -1; echo ${PIPESTATUS[0]}` |
| F3 | low | CLI build | An explicit `-o` is written with `std::fs::write`: follows a symlink and overwrites its target, truncates any existing file, blocks forever on a FIFO with no reader (like `cc -o`; but `oxi fmt` refuses symlinks/special files, `build` does not). | `... --ignored build_output_does_not_follow_symlink` |
| F4 | low | CLI vs browser | `--fuel 0`: browser maps 0 to the default budget (`fuel >= 1 ? fuel : DEFAULT`), `oxi run --fuel 0` clamps to 1 and traps `out_of_fuel` on any program (exit 4). The "same fuel" claim holds only for the default. | `... --ignored run_fuel_zero_matches_browser_meaning` |
| F5 | low | browser, main thread (self-inflicted: only a paste triggers it, `#ex=` loads built-in examples only) | The input handler repaints gutter + mirror and then `check` runs on the main thread: a 200 000-char paste of newlines (the most `maxlength=200000` allows) freezes the tab 5.2 s (50 000 newlines 1.7 s; 66 000 lines of `a;` 2.0 s + 0.8 s check; realistic 8 000-line / 200 KB file 0.33 s + 0.24 s). Programmatic 1 MB: 26 s. Never crashes, recovers. | `node sites/test/bad_actor_run_bigpaste.mjs` |
| F6 | low | browser | The 5 s timer starts after the runner is `ready`; the 2.3 MB runner download/compile has no timeout. A stalled fetch leaves "Loading the runner (once)..." with Run disabled indefinitely; Stop recovers it and a later Run works (a hard fetch failure shows "could not start"). | `node sites/test/bad_actor_run_timeout.mjs` (case B) |
| F7 | low | copy | (a) oxiscript `/` and `/pl/`: the bullet "Running programs. Run compiles the program and runs it ..." sits under the heading "What does not work yet" (reads as "does not work"), and "What works today" does not list Run/compile. (b) info: the tour heading "State that survives reload" above `counter` is ambiguous next to a Run button: the playground's `state` is in memory and created per Run (`StateStore::new` in each `run_wasm`; read from code, `counter` not run twice in the browser), nothing survives a page reload (grammar.md means hot reload in a real host). (c) The roadmap table (generated from docs/history/oxi-ui-plan-history.md) still shows 8.W and 8.D as "planned". | read `sites/i18n/en.json` index blocks[4] / tour blocks[9]; `sites/dist/oxiscript/roadmap/` |
| F8 | low | runtime text | Trap `host_denied` is shown as "the program used a capability it was not given [OXS-E0423]" but it also fires for an over-large argument (a 70 000-byte log message or 1 MiB state key: `HostBadArgument`); a log line > 1024 bytes is truncated silently (60 000-char line shows 1024 chars, no note). | `cargo test -p oxiscript-playground --test bad_actor_run isolation -- --nocapture` (lines "state key 1 MiB", "70000 bytes"); `bad_actor_run_lazy.mjs` |
| F9 | info | compiler | `@host @export_name("main")` and `"__heap_base"` are accepted (reserved set is oxi_init/memory/oxi_abi...); harmless today, the ABI list should probably reserve them. | same test, lines "export_name main", "__heap_base" |
| F10 | info | docs | implementation.md says a program needing another interface gets E0400 "and `oxi build` still works". True only when a project `idl/` declares it (blink template: run exit 2, build ok). An undeclared `host:clock` fails check for both: exit 1 (E0400+E0300), SCHEMA's "exit 2 for a host interface `oxi run` lacks" is reachable only via project IDL. | `oxi run clk.oxs` |
| F11 | info | identity | Mascot SVG body (`#f5a399`..`#ee8a6e`) on the light backgrounds is 1.9-2.3:1; it is decorative (`alt=""`), so no WCAG failure, but the "coral is text-safe" row of the identity table is about the text token only. | arithmetic |
| F12 | info | output | Bidi override U+202E and zero-width chars pass into the output panel unchanged (contained to their own line, `pre`); ESC/BEL/NUL are escaped by the runtime. | `bad_actor_run_lazy.mjs` |

## Numbers

* Browser runaway: 9 programs + 6 oversized sources + 3 flood/line cases; every run ended in 50-420 ms (fuel 10 M burns in ~130 ms); no main-thread long task from a run; 0 CSP violations, 0 page errors.
* Leak: 4 programs x 100 runs in ONE runner instance: wasm memory flat at 34 MiB, 16-95 ms per run.
* Timer: never reached by a real program; exercised with a patched constant (kill, "Stopped after N s", cold respawn run in ~480 ms). 15 run/stop cycles + click spam fine.
* Native: 27 isolation programs, 4 000 mutation-fuzz programs (20 s, debug), 0 panics. Parity: 67 programs, 67 byte-identical JSON between native and the real wasm bundle (including `fuelUsed`).
* Page weight: 624 KB before Run, +2.31 MB (0.78 MB gzip) after; runner NOT fetched before Run and no `modulepreload`.
* Rendered colour scan: 4116 text elements, 0 failures.

## Not done / limits

Browser results come from the run worktree's `pkg_run` bundle (source-identical to this branch: empty `git diff` under `oxiscript/`), not a fresh wasm-pack build. No Firefox/WebKit. The real OxiUI render of the THTML ASCII axolotl (terminal/canvas) was linted, not rendered. Real-user paste of 200 000 newlines was not timed (Chromium's own textarea edit is very slow there, > 200 s via CDP insertText); the page's own handler time is what is reported in F5.
