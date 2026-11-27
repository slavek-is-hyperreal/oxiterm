# BAD ACTOR report: 8.Spec + 8.Sec + 8.Fix (branch gemini-fixspecsec, 75d2354)

Copy of the target-1 part of round report `docs/stages/rounds/2026-11-27-1.md` (on branch `ba-round1`). Attacker: Claude Opus 5.5.
Tests (this branch): `oxiscript/oxiscript-{runtime,syntax,check,lower,cli}/tests/bad_actor_round1.rs`, `editors/vscode-oxiscript/test/bad_actor_round1.test.mjs` (needs `npm run compile` and the `oxi` debug binary). Failing tests are findings; `probe_*` tests are attacks that held. Heavy runtime tests: `cargo test --release -p oxiscript-runtime --test bad_actor_round1`.

| Id | Target | Severity | Test | One line |
|---|---|---|---|---|
| R1-03 | 1 | MEDIUM | `oxiscript-cli/tests/bad_actor_round1.rs::r1_03_fmt_prints_raw_escapes_from_file_names` | `oxi fmt` prints file names and syntax-error lines raw: an OSC/CSI/C1 file name drives the terminal |
| R1-04 | 1 | MEDIUM | `oxiscript-cli/tests/bad_actor_round1.rs::r1_04_json_file_names_are_inconsistent`, `editors/vscode-oxiscript/test/bad_actor_round1.test.mjs` | JSON `files[]` raw but `diagnostics[].file` sanitised: the VS Code extension shows no diagnostics for such a file |
| R1-01 | 1 | LOW | `oxiscript-runtime/tests/bad_actor_round1.rs::r1_01_many_functions_with_max_locals_tiny_bodies` (release) | 80 KB module = 0.85 s eager compile (9 900 functions x 29 990 locals) |
| R1-02 | 1 | LOW | `oxiscript-runtime/tests/bad_actor_round1.rs::r1_02_wall_cap_does_not_stop_a_compute_only_loop` | `call_wall_micros` never interrupts a loop without host calls (closure list says "independent of fuel") |
| R1-05 | 1 | LOW | `oxiscript-runtime/tests/bad_actor_round1.rs::r1_05_log_sanitiser_mangles_zwj_and_zwnj_text` | shared sanitiser escapes ZWJ/ZWNJ: emoji sequences, Persian and Indic log text are mangled |
| R1-12 | 1 | LOW | `oxiscript-cli/tests/bad_actor_round1.rs::r1_12_fmt_keeps_raw_bidi_in_line_comments` | `oxi fmt` keeps a raw U+202E in a `//` comment (R2-133 says fmt cannot re-emit Trojan Source controls) |
| R1-11 | 1 | LOW | `oxiscript-syntax/tests/bad_actor_round1.rs::r1_11_continuation_hazard_fix_never_changes_meaning` | E0004 offers (and `--fix-only semicolons` applies) `;` before a line starting with a template literal |
| R1-21 | 1 | LOW (pre-existing, message new) | `oxiscript-check/tests/bad_actor_round1.rs::r1_21_e0105_precision` | `x += 1` / `x++` on a narrowed `T \| null` local: E0105 "`x` may be null here" although `x` is narrowed |

## Findings

### R1-03 MEDIUM: `oxi fmt` bypasses the shared sanitiser
`oxiscript-cli/src/fmtcmd.rs` prints `display` (the file name) and the syntax diagnostics without `sanitize_terminal`: `would reformat {display}`, `formatted {display}`, `error: {display}: {why}`, `error: {e}`, and `syntax_errors()` (`{display}:{l}:{c}: error[..]: {message}`).
Repro: `touch $'esc\e]0;PWN\a.oxs'` with a syntax error (or `ugly\u{9b}31m.oxs` needing formatting), then `oxi fmt --check .` or `oxi fmt .`: stdout/stderr contain the raw ESC ] 0 ; PWN BEL (terminal title rewritten) and the raw 8-bit CSI U+009B.
Expected: every `oxi` output path goes through `sanitize_terminal` (closure list R3-206 says it is wired into `oxi`). Actual: `check`, `run`, `build` are covered, `fmt` is not. Lexer messages already escape the character (`unexpected character \u{1b}`), so the vector is the file name: running `oxi fmt` in a cloned repo with such a file. Other unsanitised echoes (argv only, info): `oxi explain <code>` and `oxi new <name>` print the argument raw.

### R1-04 MEDIUM: sanitised `file` fields no longer match `files[]`
`report::collect` and `diag_at` sanitise `Diag.file` and `EditOut.file`, but `check.rs` pushes the raw display name into `names` (`files[]`) and `file_targets`. The file name is data in JSON (serde already escapes C0), so sanitising it there changes the key.
Repro: a file `soft\u{ad}hy.oxs` containing `a == b`: `oxi check --json soft­hy.oxs` gives `"files": ["soft\u{ad}hy.oxs"]` but `"file": "soft\\xadhy.oxs"` in the diagnostic and its edit. Consequences: (a) the VS Code extension keeps only diagnostics whose `file === report.files[0]`, so it shows **no diagnostic and no quick fix** for such a file (the node test runs the real extension against the real `oxi`: control file gets `OXS-E0005`, the soft-hyphen file gets `[]`); (b) human `oxi check` output loses the source snippet, because `sources` is keyed by the raw name and looked up by the sanitised one. Any of the bidi/invisible characters in a file name hides all its errors in the editor.
Expected: JSON keeps paths verbatim (it is not a terminal), only human output escapes them; or all three fields use the same form.

### R1-01 LOW: eager-compile amplification through declared locals
The new structural caps do not bound the total number of declared locals. 9 900 functions, each `(local 29990 i64)` in one compressed entry (8 bytes per body, wasmi's per-function cap is 30 000 locals), make an 80 033-byte module that costs 0.85 s to load (validation, eager translation and instantiation, `Host::instantiate`) in release (a normal module loads at ~80 ns/byte: 600 KB of nested blocks in 48 ms). `strict()` refused this exact shape (average body < 40 bytes); it would have needed ~400 KB for the same cost, so the class pre-exists, but the claim in 8.Fix/closed.md R2-109 ("every structural cap from strict() is enforced") misses that the dropped rule was also what bounded locals per byte. Loading is not fuel-metered; with hot reload a guest-supplied module can buy ~10 us of host CPU per byte.
Expected: a cap on the sum of declared locals (e.g. 1-2 M per module) in `reject_reexports`. Call-time cost of such functions is metered (wasmi charges fuel for zeroed locals): checked, not a finding.

### R1-02 LOW: `call_wall_micros` reports, does not stop
`check_wall` runs only at host-call entry/exit and after `f.call` returns. A guest loop without host calls under `fuel_per_call = 5M`, `call_wall_micros = 1000` and a clock advancing 500 us per read runs all 5M fuel (OutOfFuel), the wall cap never fires. The limits.rs comment is accurate; the closure list (R3-214: "checked independently of fuel") and the test name `wall_clock_limit_stops_call_independent_of_fuel` overstate it. Expected: document it as a post-hoc / host-boundary cap, or enforce it with wasmi epoch-style interruption (fuel slices).

### R1-05 LOW: ZWJ / ZWNJ escaped in guest log lines
`is_unsafe_terminal_char` includes U+200C and U+200D (range `200b..=200f`). They are neither controls nor bidi overrides; they are required by emoji ZWJ sequences, Persian (ZWNJ) and Indic scripts. `log.write(2, "family 👨‍👩‍👧")` arrives as `family 👨\u{200d}👩\u{200d}👧` in the sink, `oxi run` and the playground (main escaped only `is_control`). Expected: keep ZWJ/ZWNJ (and probably U+00AD) in runtime text; escape them only in identifiers/diagnostic snippets if wanted.

### R1-11 LOW: E0004 `;` before a template-literal line
`const a = b` followed by a line `` `t`; `` gets OXS-E0004 with a machine-applicable `insert ;` at the end of line 1; `oxi check --fix-only semicolons` applies it. JS reads `` b`t` `` as a tagged template, so this is the third classic ASI hazard next to `(` and `[`, which the parser already refuses to fix (AGENTS.md: a `;` is never added where it would change the meaning). Lines starting with `(`, `[`, `-`, `.`, `?.`, `as`, `??`, `<` parse as continuations (no fix): correct.

### R1-21 LOW: compound assignment on a narrowed nullable
`let x: i32 | null = a; if (x !== null) { x += 1; return x; }` and the `x++` twin are rejected: E0105 "`x` may be null here" plus E0100 on `return x`. `x = x + 1` in the same place is accepted. On main the same programs fail with E0110/E0128, so the false positive pre-exists; 8.Fix turned it into a factually wrong message. Expected: compound assignment reads the narrowed type.

### Attacks that held (target 1)
- Loader (`load.rs`): 4 M-target `br_table` (rejected by validation), 200 000 nested blocks (48 ms), 60 000 `local.get` + `local.set` preservation pattern (12 ms), 1 000 element segments x 1 300 items (97 ms); type-section param/result caps; counts saturate; imports of memory/table/global still refused. Hot-reload non-widening: reviewed, `needs` recorded only on success; no bypass found.
- Sanitiser: C0/DEL/C1/bidi/tag characters fully covered; no terminal control outside `is_control`; identifiers are ASCII-only (U+200D, U+3164, U+1D173 give E0014), so invisible identifiers are impossible; playground JSON, `oxi check/run/build`, runtime log: clean.
- `forbid(unsafe_code)`: present in every crate root except the documented `oxiscript-rc` (and `oxiscript-bench`, tooling, info); the playground's `deny` is not overridden anywhere (no `allow(unsafe_code)` in `oxiscript/`).
- `max_instantiations`: budget 1 gives E0701 through both `compile_source` and `compile_program`, budget 0 does not fire without generics, the count is reported (`probe_max_instantiations_both_entry_points`). The `.length` advice `s.chars().length` and the `byteLength` replacement type-check; the replacement is maybe-incorrect, so `--fix` never applies it.
- Formatter F8: 56 comment placements (binary operators both sides, calls, arrays, objects, ternary, unary, `as`, return, assignment, `?`, templates, type positions, unions, statements, items): output parses, is a fixed point, keeps every comment, and the code with comments removed is unchanged (`probe_formatter_comment_oracle_over_placements`).
- throw/try/catch: no false positive on property names, object keys or strings; no `;` fix on statement uses.
- E0126: mutating method call, `++`, `+=`, nested field, nested closure all rejected. E0105: optional chain, `<`, `*`, `<<`, `-=`, `--` all caught. W0001: no false positive (closure, template, `?`, switch subject, compound assign, for-of, law). W0005: no false positive after reassignment.
- Float `%`: 40 000 random bit patterns (subnormal, near-max, inf/NaN, `%=`) bit-exact with Rust on f64 and f32; worst case `f64::MAX % 5e-324` costs 6 447 fuel.

### R1-12 LOW: raw bidi survives `oxi fmt` in line comments
8.Fix R2-133 says the formatter escapes invisible/bidi characters in comments "so `oxi fmt` cannot re-emit raw Trojan Source controls". It does so for `/* */` (`/* c\u{202E}d */`), but `fmtcmd::format_preserving` restores the original text of `//` and `///` comments, so `// a<U+202E>b` keeps the raw override after `oxi fmt`. The lexer still warns (W0003), so it is visible; the closure claim is wrong. Expected: same treatment for line comments, or the claim narrowed.

**Verdict target 1: merge after fixes** (R1-03 and R1-04 must be fixed before merge; the LOW findings can be tracked).

