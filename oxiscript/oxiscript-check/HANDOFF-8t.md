# HANDOFF 8.T (type checker) - keep updated after every milestone

Worktree: `.claude/worktrees/agent-a12e57caa0984dfa6`. No git commands. CARGO_BUILD_JOBS=2.
Verify: `cd <worktree> && CARGO_BUILD_JOBS=2 cargo test -p oxiscript-check` (and `--release`),
`cargo clippy -p oxiscript-check --all-targets -- -D warnings`,
`cargo build -p oxiscript-check --target wasm32-unknown-unknown`,
`cargo test -p oxiscript-syntax -p oxiscript-abi` (must stay green).

## Status
- [x] M0-M5 core checker implemented: src/{lib,ty,output,decl,typeexpr,expr,calls,builtins,stmt,flow,exhaust}.rs; compiles; smoke + valid corpus pass (45/45).
- [x] Valid corpus fixed (real files edited, syntax tests green): 06,08,11,12,13,14,17,18,19,21,22,25,26,27,30,31,36,37,43,45 (script in scratchpad patch4.py; reasons in "Corpus triage"). tests/corpus.rs supplies host IDLs (tests/idl/ui.idl, canvas.idl) and stub modules ./util, std/math.
- [x] Advisor bug list (exports guard, Capture.mutated, definite assignment, Ok(), receiver sets, for-of msg, union fields read-only, effects (EffectSummary + law purity), enum cap, quadratic fixes, E0128 unary code, @allow code check, this_locals) done.
- [x] tests: ill_typed.rs (100 programs, //! markers), soundness.rs, accept.rs, corpus.rs, smoke.rs pass.
- [x] limits.rs done+passing debug/release on 1MB stack (parser caps depth ~50, checker MAX_EXPR_DEPTH=180; probe: debug needs <=512KB). tables.rs, modules.rs, registry.rs files exist: verify they pass.
- [x] fuzz.rs done (4 tests, pass debug,full-debug,release). Remaining: speed example
- [x] contracts.md 8.T paragraph written
- [x] speed 105k lines/s release; clippy clean; wasm32 ok; debug+release green. Remaining: advisor, report
Known TODO polish: arithmetic on nullable should say E0105; W0001 unused not implemented; deep-chain stack tests.

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

## Final state (advisor round 2 done)
- Fixed: reversed alias/declaration chains overflowed 1MB stack -> MAX_SIG_DEPTH=64 guard (E0113) wrapping ensure_*; interface flatten was cubic -> HashSet + MAX_IFACE_METHODS=256; host `result<T>` returned ERROR type (prelude Result lookup used `exports`, now `top`) - real soundness hole, test in soundness.rs.
- Added tests: limits.rs (declaration/reversed/import/bound chains, nested inference non-exponential, mixed nesting), tables.rs all-tables ERROR walker, fuzz.rs.
- All green: debug+release 69 tests, clippy -D warnings, wasm32, syntax+abi 133. Only the report remains.

## Advisor round 3 fixes (done)
- depth counter no longer reset in with_def_env; ensure_* wrappers + lower_type consume it; MAX_EXPR_DEPTH 120 (debug 1MB needs ~384-512KB, release <=256KB).
- sig_overflow writes placeholder sigs per kind; OXS-E0990 (internal poison in tables of ok program) emitted by finish(); fuzz asserts none.
- tests: nested_budgets, guarded_definitions, nested inference n>=20 asserted. Gate green (71 tests debug+release, clippy, wasm32, syntax+abi 133). ONLY THE REPORT REMAINS.
