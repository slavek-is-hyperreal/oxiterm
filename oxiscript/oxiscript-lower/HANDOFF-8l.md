# HANDOFF 8.L (oxiscript-lower) - update after every milestone

Worktree: `.claude/worktrees/agent-a2eccde77a6c2a250`. No git commands. CARGO_BUILD_JOBS=2, per-package cargo only, one cargo at a time.
Verify: `cargo test -p oxiscript-lower --no-fail-fast` (+ `--release`), `cargo test -p oxiscript-check -p oxiscript-ir -p oxiscript-wasm -p oxiscript-abi`,
`cargo clippy -p oxiscript-lower --all-targets -- -D warnings`, `cargo build -p oxiscript-lower --target wasm32-unknown-unknown`.

## Status
M2 DONE: corpus gate (a) green (44/45 files pass checker... all 45 compile+load; tests/corpus.rs), 39 closed programs pass with zero live objects (tests/programs_{scalar,managed,types,host}.rs). NEXT: more programs (>=40 total new, plus tough RC cases: early return with temps, `?` with pending temps, break/continue with owned locals, nested closures, closures capturing this), tiny-methods module (E0408), differential scalar test, clippy -D warnings, wasm32 build, speed example, advisor, contracts.md 8.L notes, report.
Known gaps: float->string uses oxi_str_from_f64 (Stub traps unreachable); float `%` unsupported (E0703); ctor `this` used as value inside nested control flow (E0700); statics of generic classes; `let x: Ref;` without initialiser for non-str/array managed types.

Edits outside this crate (list in the final report):
- oxiscript-ir: new ops str.concat/eq/cmp/slice/byte/from_f64, array.empty, array.pop (types.rs, print.rs, parse.rs, verify.rs); oxiscript-wasm func.rs codegen for them.
- oxiscript-check: `PRELUDE_SRC` made pub; `Checked.sources`/`Checked.order` added (output.rs, lib.rs).
- oxiscript-abi registry: OXS-E0700..E0704 (phase lower).

## Decisions
- API: `lower(&mut Checked, sources: &[&str]) -> Result<ir::Module, Diagnostics>`; `compile_source(src, interfaces, ...)`.
  Checked does not keep the AST: lowering re-parses sources (NodeIds are deterministic). Prelude src = `oxiscript_check::PRELUDE_SRC` (made `pub`).
  `&mut Checked` because monomorphisation substitutes types (interner append-only).
- RC discipline: own everything. Retain every load of mutable state (field/array/global/cell) immediately; immutable locals/params are borrowed operands. Mutated params are retained at entry. Locals own one ref, released at scope exit.
- SSA locals via block params at joins/loops (only variables assigned in the construct). Captured AND mutated locals live in a generated `Cell` struct.
- Mutating struct method on an SSA local: move the local's reference into the call (no retain), store the returned receiver back.
- Struct COW: local -> is_unique/clone with block param; nested field/element -> is_unique on the borrowed child, clone+setfield, re-load (no borrowed block params).
- Monomorphisation: worklist keyed by (DefId, concrete type args); substitute caller's map in `CallInfo::type_args`; resolve `Res::IfaceMethod` by name on concrete record; budget -> E07xx.
- needs set explicitly from `main` parameters.
- Determinism: never iterate HashMaps for output order.
- Missing IR ops (strings: concat/eq/cmp/slice/from_*, byte access; array pop): to be added in oxiscript-ir + oxiscript-wasm (minimal, listed in report).

## Done
(nothing yet)

## Next
see Status.
