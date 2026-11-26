# Fixes for the BAD ACTOR findings of RUN + IDENTITY (branch integ-sites)

Status per finding (updated after each fix). "Test" = what flips or guards it.

| id | status | fix | files | test |
|---|---|---|---|---|
| F1 | fixed | `oxi build` validates `[package] name` against `[A-Za-z0-9_-]{1,64}` when it is used for the default name (exit 2, nothing written); the default output is written with `write_atomic_bytes`, which refuses a symlink / non-regular file (a shipped `demo.wasm -> ../victim` is refused, exit 2) | `oxiscript/oxiscript-cli/src/runcmd.rs`, `src/project.rs` | `tests/bad_actor_run.rs` `build_default_output_stays_inside_cwd`, `plain_build_does_not_write_through_a_shipped_symlink` (un-ignored) |
| F2 | fixed | every write of `oxi run` goes through `writeln!` on a locked handle: EPIPE ends quietly with exit 0, any other write error (`/dev/full`) exits 2 with `oxi run: cannot write output: ...`; diagnostics and `note:` lines can no longer panic either | `runcmd.rs`, `project.rs` (`note`), `SCHEMA.md` | `run_closed_stdout_is_not_a_panic` (un-ignored) |
| F3 | fixed | `-o` uses the same atomic writer: symlink, directory, FIFO, device refused (exit 2); an existing regular file is replaced (temp file in the same directory + rename) | `runcmd.rs`, `project.rs` | `build_output_does_not_follow_symlink` (un-ignored) |
| F4 | fixed | `--fuel 0` = default budget (10 000 000), as in the browser; help text and SCHEMA.md say so | `runcmd.rs`, `main.rs`, `SCHEMA.md` | `run_fuel_zero_matches_browser_meaning` (un-ignored) |
| F5 | fixed | gutter and mirror paint at most 10 000 lines (line numbers for the first 10 000, no error marks above that) with a visible note (`#pg-big`); the 8 000-line / 200 KB case is unchanged; line counting is an `indexOf` loop, not `split` | `sites/oxiscript/js/playground.js`, `sites/build.mjs`, i18n | `sites/test/bad_actor_run_bigpaste.mjs` (numbers in the report) |
| F6 | fixed | runner load timeout 20 s (`LOAD_TIMEOUT_MS`), separate from the 5 s run timer; message says what happened and that Run retries; the Stop hint is in the status shown while loading (`loadingRunner`) | `playground.js`, i18n `loadTimeout`, `loadingRunner` | new `sites/test/bad_actor_run_loadtimeout.mjs` (constant patched to 2 s: message, Run usable, retry works); `bad_actor_run_timeout.mjs` case B still recovers via Stop |
| F7 | fixed (copy) | Run moved to "What works today" with the true description (state is in memory for one run); the limits (`f64` to text, other interfaces, nothing kept between runs) stay under "does not work yet"; tour heading is now "State through a host capability" (PL, LA too). Roadmap rows: see "For the lead" | `sites/i18n/{en,pl,la}.json` | `node sites/check.mjs`, build |
| F8 | fixed | new stop reason `host_bad_argument` (code OXS-E0423, "Host call rejected an argument") with its own honest text; `host_denied` stays for a real missing capability. A log line over 1024 bytes ends with ` ... (line truncated at 1024 bytes)` (the provider is given 4 bytes of slack so the playground can tell a cut line from a line of exactly 1024) | `oxiscript/oxiscript-playground/src/run.rs`, `oxiscript-cli/src/runcmd.rs` (`explain`), i18n, `sites/build.mjs` | `oxiscript-playground/tests/run_fixes.rs` |
| F9 | fixed | `@export_name` rejects `main`, `memory`, `__heap_base`, `__data_end`, `oxi_*`, `cb:*` with OXS-E0123 (existing code reused, registry unchanged) | `oxiscript/oxiscript-check/src/decl.rs` | `oxiscript-check/tests/export_abi.rs` `reserved_export_names_are_rejected` |
| headers | fixed | explicit per-hash `Cache-Control` rules, per-entry-point revalidate rules, no broad rule sets Cache-Control; `check.mjs` checks every dist path | `sites/build.mjs`, `sites/check.mjs` | `node sites/check.mjs` |
| worker CSP | fixed | `Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'` on `/assets/<hash>/runner-worker.js` | `sites/build.mjs`, `sites/check.mjs` | `sites/test/csp_browser.mjs`, `bad_actor_run_workercsp.mjs` |

## Notes

* F8: the cut can fall inside a `\xNN` / `\u{..}` escape the runtime wrote for a control character; the output is still valid text, just a truncated escape before the marker.
* Headers: the lead's description ("`/app/*.wasm` overrides the immutable rule") does not match the old file: that rule set only `Content-Type`. The old immutable rule `/app/*/*` had two splats, which Cloudflare Pages does not support (one `*` per pattern: recalled, NOT verified, no network), so it probably never matched and the Pages default `max-age=0, must-revalidate` applied. Cloudflare also documents that several matching rules do not replace each other: the values of the same header name are joined with a comma (unverified here). The new design does not depend on either: every path matches exactly one rule that sets `Cache-Control`, and `check.mjs` fails otherwise (also on a pattern with more than one `*`).

## For the lead: rows in docs/oxi-ui.md (not edited here)

`sites/build.mjs` `stateOf(id)` reads the wave-log table (`| Slice | Implementation | BAD ACTOR | Fixes | Findings | Merged |`, under "#### Wave log"): a slice is **done** only if a row starts with its id and its 6th cell (Merged) holds a date or a 7-hex hash and no em dash; no row = planned, a `—` = active.

* 8.W and 8.D have no row, so the roadmap shows them as planned. Add, with the real hashes: `| 8.W Websites + course | ... | ... | ... | ... | <date>, <hash> |` and `| 8.D DevKit | ... | ... | ... | ... | <date>, <hash> |` (date and hash of the real merges; `c291e92` and `320db1b` are the Run and identity merges, NOT the 8.W / 8.D ones).
* 8.L Lowering: its Merged cell is `—` (started 2026-11-25 21:32), so it shows active; Run depends on it and is on this branch. Fill its Merged cell if it is merged.
* 8.P, 8.R, 8.T, 8.G also still have `—` in Merged; set them if merged (the playground runs on all of them).
* The phase heading "Phase 8 ... (planned)" is shown as active because slices started.

## Numbers

* bigpaste (debug machine, loaded): 200 000 newlines handler 130 ms (was 5.2 s; long task after 137 ms), 50 000 newlines 20 ms (was 1.7 s), 66 000 `a;` lines 213 ms + 219/98 ms check, 8 000 lines / 200 KB 302 ms + 306/169 ms check (was 0.33 s + 0.24 s: unchanged).
* parity: 67/67 byte-identical between native and the rebuilt runner wasm. Runner 2 311 644 B, checker 569 242 B (unchanged order of magnitude).
* A 60 000-char log line shows 1059 chars = 1024 + the marker.
* `csp_browser.mjs`: its Check click landed in the same second as the load-time check (the status carries a time with seconds, so the text did not change) and failed at line 185 in 4 of 5 runs once the machine was less loaded: a race in the TEST, fixed by waiting 1.1 s before the click (a test-race fix, nothing weakened). After it: 1 failure at line 217 (the Stop assertion waits only 500 ms, load-sensitive, left untouched) and 3 passes in the last 3 runs. Open flake: line 217.
* Manual CLI checks (target/debug/oxi): `oxi run hello.oxs > /dev/full` exit 2 with `cannot write output`; `oxi run broken.oxs 2>&1 | head -1` exit 1 (no 101); `mkfifo f.wasm; oxi build -o f.wasm` exit 2 at once; `oxi build -o /tmp` exit 2; `oxi build` twice (overwrite of a regular file) exit 0 both; `oxi run --fuel 0` exit 0.
* Full run of all `sites/test/*.mjs` that exist (csp_browser, bad_actor_run_{workercsp,lazy,stress,timeout,leak,states,disabled,borders,rendered,parity,bigpaste,loadtimeout}, bad_actor_{contrast,format,wasm}): all exit 0 (csp_browser as above). Not run: Firefox, WebKit.

## Decisions to confirm

* F1: the default output stays relative to the current directory (as SCHEMA.md always said); a validated name has no separator, so it cannot leave the directory and a repo cannot choose the working directory. If `<project root>/<name>.wasm` is wanted, it is a one-line change.
* F8 marker: ASCII ` ... (line truncated at 1024 bytes)` (not the `…` character), English on every language page because it is part of the program's output text.
* check.mjs rejects a `_headers` pattern with more than one `*` on the strength of the unverified one-splat recollection; the design works either way.
