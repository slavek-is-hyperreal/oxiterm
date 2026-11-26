# RUN end to end (playground + `oxi run` / `oxi build`)

Status: implemented; see "Verification" for what was run.

## Design

* One Rust execution path, `oxiscript_playground::run` (feature `run`): `compile` (check first, so diagnostics
  are exactly the Check shape, then `oxiscript-lower::compile_source_with_notes`), `run_wasm` (our wasmi
  runtime, `host:log@1` + `host:state@1`, fuel per run, output caps), `run_json` (the page's JSON).
  The browser worker and `oxi run` both call it: same fuel, ABI checks and capability binding.
* `oxiscript-runtime` builds for `wasm32-unknown-unknown` without changes (the default clock is already
  `None` there). So the browser runs OUR runtime, not the browser's own `WebAssembly`.
* Two bundles: the check/format bundle (`--no-default-features`, loaded at once) and the runner bundle
  (default features: + lowering + wasmi), loaded only by the Web Worker on the first Run.
* Page: `sites/oxiscript/js/runner-worker.js` (module worker, hashed file under `/assets/`, same origin, so
  the existing CSP `script-src 'self' 'wasm-unsafe-eval'` covers it through the `worker-src` fallback; no
  `blob:`, no inline script). The bundle URL is sent in the first message. `playground.js` runs Check
  first (problems are shown, nothing starts), then posts the source with a fuel limit (10 M), starts a 5 s
  wall-clock timer when the program starts, and terminates the worker on Stop, timeout or error.
* Traps: the runtime reports `Error::Trap`; the guest kind (`overflow`, `bounds`, `div_by_zero`, ...) wins,
  else the cause (`out_of_fuel`, `stack_overflow`, ...). The page maps the stable id to EN/PL/LA text.
* Output cap: 1000 lines / 64 KiB per run (counted, reported as "N more lines"), the page shows 300 lines.

## What is true today (and said on the site)

* `${x}` with an `f64` traps as `unreachable` in the current Stub runtime (no float formatting). Float
  arithmetic and `as i32` work. The copy says exactly this; there is a native test pinning it.
* Output appears when the program stops (the runtime hands the sink back after `main`), not live.
* Only `host:log@1` and `host:state@1` exist in the playground and in `oxi run`; a program that needs
  another interface gets `OXS-E0400 ... module rejected` (`oxi run` exit 2; `oxi build` still works).

## Examples

Every example now has a `main(log: Log...)`: errors, laws, shapes, structs gained one; `counter` handles a
missing key (`state.get_i64` fails on a key never written) and bumps twice. `mistakes.oxs` still fails check.
`oxiscript-playground/tests/run_examples.rs` compiles and runs them all natively with expected log lines.

## CLI

`oxi run [file] [--fuel N]`, `oxi build [file] [-o out.wasm]`. Exit codes: 0 ok, 1 diagnostics or
compile error, 2 usage/I-O/needs a host interface `oxi run` lacks, 3 reserved (was "not merged yet"),
4 the program trapped. Documented in `oxiscript-cli/SCHEMA.md` and `main.rs`.

## Sizes (wasm-pack, opt-level z, LTO, panic=abort, then wasm-opt -Oz)

| bundle | raw | gzip -9 |
|---|---|---|
| check/format before (and after, loaded at page start) | 568 775 | 241 515 |
| runner (lazy, first Run only) | 2 311 006 | 783 507 |

## Edits outside my files

* `sites/build.mjs`: `playgroundHtml` (status line inside `.pg-bar`, output panel, tip, stop button),
  the label list, and the `site === 'oxiscript'` copy block (runner bundle + worker). Nothing in the
  header/footer/layout code.
* `sites/build.sh`: second wasm-pack build for the runner, `--no-default-features` for the check bundle.
* `sites/test/csp_browser.mjs`: playground Run tests.
* No CSS touched. Reused `.pg-bar`, `.pg-status`, `.note`, `pre.code`. A style for `#pg-output` and a
  max-height for the diagnostics list would be nice follow-ups.

## Verification

* `cargo test -p oxiscript-playground -p oxiscript-cli`: all pass (new: `tests/run_examples.rs` 8 tests, CLI run/build tests).
* `cargo clippy -p oxiscript-playground -p oxiscript-cli --all-targets -- -D warnings` and
  `-p oxiscript-playground --no-default-features`: clean.
* `sites/build.sh` (full: both wasm bundles, pages, `check.mjs`, playground cargo test): passes, no i18n warnings.
* `node sites/test/csp_browser.mjs` (headless Chromium, real `_headers`): 0 failures, 0 CSP violations. Covers
  Check on the clean default example (status next to the buttons, changes with a time stamp), the tip
  (visible on load, loads mistakes.oxs, hides after diagnostics), Run refused with errors, Run hello (log line),
  endless loop (fuel message after about 0.4 s with the worker warm), Stop (status exactly "Stopped" at once,
  fuel message never appears), run again after Stop, hashchange (prompt over unsaved edits), EN and PL.
* `bad_actor_wasm`, `bad_actor_format`, `bad_actor_contrast` run (no failures reported).
* Page fuel is the runner default (10 M, same as `oxi run`; the page passes 0). Not exercised: the 5 s wall-clock
  timeout (fuel always wins headless). Not done: the BAD ACTOR adversarial/fuzz pass for this stage.
