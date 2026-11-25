# HANDOFF 8.T (type checker) - keep updated after every milestone

Worktree: `.claude/worktrees/agent-a12e57caa0984dfa6`. No git commands. CARGO_BUILD_JOBS=2.
Verify: `cd <worktree> && CARGO_BUILD_JOBS=2 cargo test -p oxiscript-check` (and `--release`),
`cargo clippy -p oxiscript-check --all-targets -- -D warnings`,
`cargo build -p oxiscript-check --target wasm32-unknown-unknown`,
`cargo test -p oxiscript-syntax -p oxiscript-abi` (must stay green).

## Status
- [x] M0 orientation, advisor consult, corpus triage (below)
- [~] M1 DONE so far: Cargo.toml (deps syntax+abi), src/ty.rs (interner, subst), src/output.rs (Checked API), registry codes E0107-E0126, E0302, E0303, W0004 added (abi tests green). TODO: lib.rs Checker + driver, decl.rs, expr.rs, stmt.rs, flow.rs, builtins.rs; contracts.md 8.T section
- [ ] M2 declarations/resolution/IDL hosts/modules
- [ ] M3 expressions, statements, numerics
- [ ] M4 narrowing, switch exhaustiveness, definite assignment, termination
- [ ] M5 generics, closures, Result/?, laws, attributes, reserved features
- [ ] M6 corpus fixes + 60 ill-typed + soundness + fuzz tests
- [ ] M7 speed measure, clippy, wasm build, report

## Decisions
- No `ena`: no inference variables. Local bidirectional checking; generic calls use one-way
  matching of param types against arg types, TS-style two phases (non-lambda args first, then
  context-sensitive lambdas with the partly-solved substitution). `salsa`: evaluate only.
- Types interned (`Ty(u32)`), O(1) equality; depth and node budgets (E0113-ish) bound pathological input.
- Prelude lives in an outer scope; module items shadow it without E0301. `Result`-ness is
  recognised structurally (union with variants ok{value} / err{error}), so a user `Result` works.
- Narrowing: facts on paths rooted at locals (and struct `this`), killed by assignment; names
  assigned inside closures are never narrowed; module `let` never narrowed; no narrowing through
  class fields or array elements (copy to a const first; fix text says so).
- Arrays are reference objects (ir.md `array.set` mutates in place); struct copies are shallow
  for array fields (documented in contracts.md).
- Check only when the parse has no errors ("do not check" = has errors).

## Corpus triage (syntax/tests/corpus/valid; files are parse-oriented, many are not closed programs)
Fix by editing the real files (keeping the parse feature each exercises); syntax tests must stay green.
Needs host/module stubs from the harness: 02 (host:ui), 03/45 (host:canvas), 16 (./util, std/math).
Undefined names: 12 parseInt, 17 sort/isSorted, 19 pairs, 21 compute, 25 Other, 26 someFunction,
27 shape/foo, 31 ns/x, 36 Item/process/register/handle, 43 S.
No context (E0103): 13 obj, 22 empty, 37 o/a.
Other: 06 i32 ternary cond, 08 `new` on struct, 11 string+i32, 14 ternary branch types, 17 `x === x` on
unbounded T, 18 unknown attr `@a` and `@@inline`, 25 interface as value type, 27 `as Array<i32>`/`as (u8|null)`,
30 `(a ?? b) || 1` on u32, 45 `for (const p of ...) p.x += ` mutates const struct copy.
(Table to be completed with the real diagnostics once the checker runs; list of changed files goes in the report.)

## Lead note (reuse; apply)
- ena: not used; checker has no inference variables (one-way matching, TS two-phase). State this reason in report.
- Exhaustiveness: implement Maranget usefulness (matrix of constructor rows) in src/exhaust.rs, used by switch stmt/expr.
- Cite Dunfield-Krishnaswami / Pierce-Turner local inference (check vs synth modes) in crate docs.
- Narrowing: TS flow-node model (our Flow state is a flow-node walk); messages mirror TS wording in `detail` strings.
- Report must list reused vs written mechanisms.
