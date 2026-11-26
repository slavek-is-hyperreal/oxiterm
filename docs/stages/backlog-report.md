# OxiScript Backlog Stages Report (8.Spec, 8.Sec, 8.Fix)

Completed in order (`8.Spec` -> `8.Sec` -> `8.Fix`) on branch `gemini-fixspecsec` while keeping `oxiscript-rc` internals, `oxiscript-wasm` internals, and the runtime/heap/trap-channel/collector/fuel/f32-text sections of `docs/oxiscript/abi.md` untouched to avoid merge conflicts with `gemini-rtlink`.

Closure lists (one line per roadmap source id):
- `docs/stages/8.Spec/closed.md`
- `docs/stages/8.Sec/closed.md`
- `docs/stages/8.Fix/closed.md`

---

## 1. What Changed per Stage

### Stage 8.Spec — Specification and Agent-Doc Repair
- Repaired OxiScript specs, contracts, and agent guidance across `docs/oxiscript/{grammar.md,contracts.md,abi.md,ir.md,llms.txt}`, `oxiscript/oxiscript-cli/{SCHEMA.md,templates/common/AGENTS.md,tools/sync_docs.py}`, `.claude/skills/oxiscript-authoring/SKILL.md` (hardlinked with `.agents/skills/oxiscript-authoring/SKILL.md`), `docs/research/{oxiscript-language.md,oxiscript-known-attacks.md}`, and `docs/stages/{8.T/implementation.md,8.G/implementation.md,README.md}`.
- Closed sections A and C (OxiScript half) of `docs/roadmap/consistency.md` and every source id in `docs/roadmap/oxiscript.md#s-8-spec`.
- Kept every ```` ```oxs ```` block in `AGENTS.md`, `llms.txt`, `SKILL.md`, and `std.md` compiling cleanly under `cargo test -p oxiscript-cli`.

### Stage 8.Sec — Hardening Backlog
- **Shared terminal/bidi escape sanitiser**: added `oxiscript/oxiscript-abi/src/sanitize.rs` (`is_unsafe_terminal_char`, `sanitize_terminal`, `sanitize_terminal_bounded`) covering ESC, CSI, OSC, 8-bit C1 (`U+0080..=U+009F`), CR, LF, NUL, DEL, and Unicode bidi/format controls (`U+202A..=U+202E`, `U+2066..=U+2069`, `U+200E..=U+200F`, `U+061C`), with non-splitting bounded truncation.
- **Wired into all text output paths**: `oxiscript-cli` (`report.rs`, `check.rs`, `project.rs`, `runcmd.rs`), `oxiscript-playground` (`lib.rs`, `run.rs`), `oxiscript-runtime` (`caps/log.rs`), and `editors/vscode-oxiscript` (`src/convert.ts`, `src/extension.ts`, `test/convert.test.js`).
- **Trojan Source hardening in lexer & formatter**: `oxiscript-syntax/src/lexer.rs` now warns (`OXS-W0003`) on invisible/bidi characters inside `//`, `///`, and `/* ... */` comments (while preserving `\n`, `\r`, `\t`, and escaped `\u{...}`), and `fmt/build.rs` escapes invisible/bidi controls in formatted comments.
- **`unsafe` policy**: added `#![forbid(unsafe_code)]` to `oxiscript-{abi,syntax,ir,check,lower,wasm,runtime}` and `oxiscript-cli`, `#![deny(unsafe_code)]` to `oxiscript-playground` (due to `#[wasm_bindgen]` expansion on `wasm32`), and documented the `oxiscript-rc` exception (linear-memory allocator/RC runtime).
- **Runtime hardening (`oxiscript-runtime`)**:
  - Added wall-clock call limit (`Limits::call_wall_micros`) enforced independently of fuel on host calls and call exit (`limits.rs`, `host.rs`, `runtime.rs`).
  - Charged proportional host-byte fuel on `STATUS_BUFFER_TOO_SMALL` retry probes (`host.rs`).
  - Blocked capability widening on `Runtime::reload` with `OXS-E0400` before `oxi_init` executes (narrowing allowed; subsequent re-widening blocked).
- **Regression suites**: added `sec_8sec.rs` across `oxiscript-syntax`, `oxiscript-runtime`, `oxiscript-playground`, `oxiscript-cli`, and `oxiscript-lower` (including byte-identical IR+Wasm determinism on `device`/`web` and fuel+OOM fault-injection sweeps).

### Stage 8.Fix — Language and Compiler Leftovers
- **Reason codes (append-only)**: added `OXS-W0005` (`Unreachable case label`, excluded by narrowing) to `oxiscript/oxiscript-abi/registry/codes.txt` and `docs/oxiscript/llms.txt`; no existing code was renumbered or altered.
- **Parser & formatter (`oxiscript-syntax`)**:
  - Reserved `decreases`, `ghost`, and `pure` (`OXS-E0900`).
  - Preserved `static`/`mutating`/`private` modifiers on `MemberKind::Constructor` in the AST and formatter.
  - Diagnosed `throw`/`try`/`catch`/`finally` and `OXS-E0004` continuation hazards (`a\n[1]`, `a\n+ b`, `let k = 2 +`) directly in `parser/mod.rs::expect_semi` (pointing at the previous line without a bogus `;` insert fix), keeping CLI `hints.rs` aligned.
  - Fixed formatter F8 comment re-attachment (`a /* b */ + c` keeps `/* b */` before `+`; `[1, // one\n 2, // two\n]`, object literals, and call arguments keep trailing/dangling comments inside delimiters with idempotent output).
  - Added `multi_seed_oracles` (`bad_actor_8p_gen.rs`, heavy in debug, verified in `--release`) running both `structured_roundtrip` (with comments) and `fixit_sweep` across 5 seeds with 0 failures.
- **Type checker (`oxiscript-check`)**:
  - Rejected `static`/`mutating` constructors (`OXS-E0123`), emitted `OXS-W0005` for union switch case labels excluded by narrowing, rejected mutating a struct's `this` inside a closure (`OXS-E0126`), and rejected duplicate literal labels in nullable switches (`OXS-E0116`).
  - Emitted `OXS-E0105` across all nullable/`null` binary, unary, compound-assignment, and `++`/`--` operations.
  - Implemented `OXS-W0001` unused-binding warnings (`let`/`const`/`for..of` without leading `_`, on error-free modules) and added `oxi explain` examples for `OXS-W0001` and `OXS-W0005`.
  - Widened `null`-narrowed locals/globals declared `T | null` back to their declared nullable type when inferring arrow/function return types (`widen_expr`), fixing `() => s` `OXS-E0700`.
  - Attached `.byteLength`/`chars()` fixes on `s.length` (`OXS-E0108`) and `s.bytes()`/`s.chars()` advice on `s[i]` (`OXS-E0128`) directly in `oxiscript-check`.
  - Included clipped sub-expression/statement snippets in `OXS-E0113` budget-exhaustion diagnostics.
- **Runtime & lowering (`oxiscript-runtime`, `oxiscript-lower`)**:
  - Relaxed wasmi's lazy-compilation-only 40 avg-bytes/function rule in `oxiscript-runtime` (`CompilationMode::Eager` + `EnforcedLimits::default()`) while enforcing every structural cap from `EnforcedLimits::strict()` directly in `load.rs::reject_reexports`, allowing 1000-tiny-export modules to compile, load, and run cleanly.
  - Implemented deterministic IEEE 754 float `%` (`f64 % f64`, `f32 % f32`) as OxiIR helpers `@lw.fmod_f64` / `@lw.fmod_f32` in `oxiscript-lower/src/helpers.rs` (zero `oxiscript-ir`/`oxiscript-wasm` changes) and fixed `field_decl_ty` to unwrap `TyKind::Nullable` for `this.x?.f` in constructors on generic types.
  - Added configurable `Options::max_instantiations`, `lower_with_budget`, and `instantiations` count on `Lowered` and `CompileOutput` (`OXS-E0701`).

---

## 2. Final Gate Results

All 10 crates tested one package at a time (`CARGO_BUILD_JOBS=2`):

| Crate | `cargo test --no-fail-fast` | `cargo clippy --all-targets -- -D warnings` | Notes |
|---|---:|---|---|
| `oxiscript-syntax` | **95 passed**, 0 failed, 3 ignored | clean | All 3 ignored oracles (`fixit_sweep`, `structured_roundtrip`, `multi_seed_oracles`) also verified **green** in `--release` |
| `oxiscript-abi` | **54 passed**, 0 failed, 0 ignored | clean | |
| `oxiscript-ir` | **45 passed**, 0 failed, 0 ignored | clean | |
| `oxiscript-rc` | **62 passed**, 0 failed, 2 ignored | clean | Untouched internals (`gemini-rtlink` safe) |
| `oxiscript-runtime` | **90 passed**, 0 failed, 1 ignored | clean | |
| `oxiscript-check` | **194 passed**, 0 failed, 0 ignored | clean | |
| `oxiscript-wasm` | **101 passed**, 0 failed, 2 ignored | clean | Untouched internals (`gemini-rtlink` safe) |
| `oxiscript-lower` | **250 passed**, 0 failed, 5 ignored | clean | |
| `oxiscript-cli` | **65 passed**, 0 failed, 0 ignored | clean | All agent-doc & `std.md` ```` ```oxs ```` blocks compiled cleanly |
| `oxiscript-playground` | **27 passed**, 0 failed, 1 ignored | clean | `cargo build -p oxiscript-playground --target wasm32-unknown-unknown` **green** |
| **Total** | **983 passed**, **0 failed**, 14 ignored | **10 / 10 clean** | `wasm32-unknown-unknown` build **green** |

---

## 3. Lead Follow-Up Actions (Tools Unavailable in Sandbox)

1. **Sync generated agent docs** (`python` unavailable in sandbox):
   ```sh
   python3 -I oxiscript/oxiscript-cli/tools/sync_docs.py
   ```
   (Updates root `AGENTS.md` from `oxiscript/oxiscript-cli/templates/common/AGENTS.md` and verifies `docs/oxiscript/llms.txt` matches `codes.txt`; note `docs/oxiscript/llms.txt` already includes `OXS-W0005`.)
2. **Build and test VS Code extension** (`node`/`npm` unavailable in sandbox):
   ```sh
   cd editors/vscode-oxiscript && npm run compile && npm test
   ```
   (Verifies the shared terminal-escape/bidi sanitiser added to `src/convert.ts`, `src/extension.ts`, and `test/convert.test.js`.)
3. **Online security checks** (network unavailable in sandbox):
   - Run `cargo audit` (and `cargo deny` if configured) for `[R3-201]` / `[R3-216]`.
   - Verify primary-source external URLs for `[R3-200]`.

---

## 4. Open Questions for the Owner

1. **Non-constant `case` labels in `Int`/`Str`/`Nullable` switches (`[R3-53, R2-106]`)**: Valid corpus `oxiscript/oxiscript-syntax/tests/corpus/valid/11_switch_expr.oxs:7` uses a runtime expression `case ("a" === n ? "x" : "y"): 2`. Should `Int`/`Str`/`Nullable` switches keep allowing runtime case labels (with `Union`/`Enum`/`Bool` requiring constants), or should non-constant labels be forbidden everywhere and `11_switch_expr.oxs` changed?
2. **`this` captured by closure inside a `mutating` struct method (`[R3-53, R2-106]`)**: Existing test `oxiscript-lower/tests/programs_hard.rs::p43_closures_capturing_this` pins that reading `this` in a closure inside a `mutating` struct method is valid and captures a value snapshot, while mutating `this` inside the closure is now rejected (`OXS-E0126`). Confirm keeping snapshot-read semantics vs forbidding `this` capture entirely and updating `p43_closures_capturing_this`.
3. **`Q-LANG-3` (`readonly`, `private`, `static` on struct value copies, `[R3-210]`)**: Current enforced checker behavior (pinned by `oxiscript-check/tests/fix_8fix.rs`) keeps `readonly` non-assignable on copies (`OXS-E0109`), `private` inaccessible outside methods (`OXS-E0119`), and `static` type-level only (`OXS-E0108`). Confirm as the v1 rule.
4. **120k-node drop fuel (`[R2-115]`)**: Once `gemini-rtlink` (`8.M-link`) merges and establishes the Linked fuel baseline (BL06), decide whether large graph drops stay bounded by `fuel_per_call` or receive a bounded deferred-drop step in `oxiscript-rc`.
5. **Post-`gemini-rtlink` `oxiscript-wasm` items (`[R3-32, R2-108, R2-113, R4-17]`)**: Wasm local reuse (liveness free list, prerequisite for `8.3a`) and `bad_cast` tag-check elision were intentionally deferred so `oxiscript-wasm` internals remain conflict-free with `gemini-rtlink`.
6. **Runtime `unsafe` exception (`8.Sec`)**: `oxiscript-rc` requires `unsafe` for linear-memory allocation and header manipulation (documented in `docs/stages/8.Sec/closed.md`), and `oxiscript-playground` uses `#![deny(unsafe_code)]` (not `forbid`) because `#[wasm_bindgen]` expands `unsafe` FFI glue on `wasm32`.
7. **Reproducible-build path remapping (`[R2-92 / BL04]` in `8.Sec`)**: Compiler output is verified byte-identical across runs; stripping builder Cargo paths from embedded `oxiscript-rc` Wasm artifacts belongs to `gemini-rtlink` (`oxiscript-wasm/build.rs`).
