# 8.P BAD ACTOR fixes: progress (update after every milestone)

Verify: `CARGO_BUILD_JOBS=2 cargo test -p oxiscript-syntax` and `... --release`; ignored oracles:
`cargo test -p oxiscript-syntax --release --test bad_actor_8p_gen -- --ignored`; clippy `-D warnings --all-targets`; `cargo fmt -p oxiscript-syntax -- --check`;
wasm: `CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime --dir=/" cargo test --target wasm32-wasip1 --release -p oxiscript-syntax --test fuzz` (wasm32-wasip1 + wasmtime are installed).
Edit scripts live in the scratchpad (p1..p4.py); tool runs refuse heredoc+cd combos, so write scripts with Write and run `python3 file`.

## Done so far (compiles; tests NOT re-run yet after these edits)
- lexer: shared MAX_DIAGS cap (L1, in diag.rs, parser budget = MAX_DIAGS - lexer diags), `max_nest` in Lexed, CR-only lines end `//` comments and count as newline in block comments (L3 lexer half), L2 messages ("float literal"), W0003 warning for invisible/bidi chars inside literals (`is_invisible`, registry code OXS-W0003 added), `Diagnostic::is_error()`, `LineIndex` treats lone CR as newline.
- parser: `ParseOptions{max_depth,use_thread}`, `parse_with`, `DEFAULT_MAX_DEPTH=100` (to be re-decided after frame measurement), token slices borrowed (no clone on restart), thread chosen from lexer `max_nest>6` or soft restart at depth>12, inline fallback when no thread. `Parsed.max_nest`.
- FormatOptions.parse (ParseOptions); format refuses only on errors (`is_error`), thread only optional.
- X1 (fix span covers whitespace before `?`), X2 (glued `?` only a ternary when a `:` follows; quiet flag avoids double report) in parser/expr.rs; parse_postfix split into out-of-line helpers.
- examples/stack_probe.rs measures min stack per construct (release): paren 3.3 KB/level, call 3.5, block 3.0 (before shrinking). Frame table via objdump: parse_stmt_inner 1.9 KB, parse_binary 760, parse_postfix 600, parse_assign_body 552, parse_cond 360.

## Milestone 2 (all 22 tests of tests/bad_actor_8p.rs pass in --release)
Implemented: iterative Drop (src/drop.rs: Expr, Stmt; AST contract note: no moving fields out of Expr/Stmt), else-if/ternary iteration in parser and formatter (flat postfix chain, flat cond chain), generic_scan bound (S1), seq trailing comment loop (F1 F2 F6), CR handling (L3), F3 parens `(f()?).y` (tests/common shape() unwraps Paren(Try)), F5 escapes, F4 attr args, DEFAULT_MAX_DEPTH=56 with STMT_WEIGHT=2, thread also used when tokens>512 / max_height>120, K1-K3 un-ignored, K4 corrected (nests DEFAULT_MAX_DEPTH-3, release only; reason in the test comment).

## Milestone 3: cargo test -p oxiscript-syntax passes in debug AND release (all bad_actor_8p tests incl. K1-K4); gen oracles: structured_roundtrip 0 failures at BA_ITERS=20000, fixit_sweep ok.
Extra decisions: E0006 fixes: nothing-after-`?` => remove whitespace (try); `a ? b` without colon => remove the dangling `? b` (a whitespace-only fix would leave stray code and the fixit_sweep oracle rejects it); no fix when the operand ends in `as T`. `///` comments that the parser did not attach as docs are printed as `//` when moved. tests/codes_registered.rs added. misc.rs deep test fixed.

## Remaining
1. clippy -D warnings, fmt check, crate docs (bytes/level) in lib.rs, wasm32-wasip1 fuzz run, other oracle seeds, parse_bench rerun (Drop cost), advisor, final report per finding.
