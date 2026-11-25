# BAD ACTOR 8.P handoff (updated at milestone: all findings below have failing tests)

Harness: tests/bad_actor_8p.rs (per-finding tests), tests/bad_actor_8p_gen.rs (ignored fuzz oracles:
structured generator w/ comments; fixit sweep). Scratchpad probes (stack tables, perf, ev tool, wasm probe):
/tmp/claude-1000/-vectorlegis-ssd-pool-oxiterm-oxiterm/ea9c7764-805e-4a30-a817-028d63200a95/scratchpad/badactor-8p/
(probe/src/bin/{ev,gen,perf,perf2,wp}.rs, minstack.sh, table100.txt native stack table, wasm_table.txt).
Run: `CARGO_BUILD_JOBS=2 cargo test -p oxiscript-syntax --release --test bad_actor_8p` ; ignored ones with `-- --ignored --exact NAME`.

## Findings (test name -> repro, root cause)
- F1 HIGH f1_fmt_line_comment_then_block_comment_corrupts_code: `let x = 1 // a\n/* b\nc */;` formats to `let x = 1; // a /* b\nc */` (does not parse). Also f1b (import list), f1c (comments merge into `//`). Root: fmt/build.rs `seq` trailing-comment loop (~l.183-192) pushes t(" ") after a line comment and appends further comments on the same line.
- F2 MED f2_fmt_not_idempotent_multiline_block_comments. Same loop.
- F3 HIGH f3_fmt_try_then_member_becomes_optional_chain: `f()?\n  .y` -> `f()?.y` (meaning changes to optional chaining). f3b: `f()?\n  ?.y` -> `f()??.y` unparsable. Root: fmt/build.rs:1194 ExprKind::Try printed with no parens when it is the object of Member/optional Member.
- F4 LOW f4: `@a(/* c */ 1) function f(){}` comment moves into body.
- F5 MED f5: escapes \u{202E},\u{2066},\u{200B},\u{FEFF},\u{2028},\u{61C},\u{85} expanded to raw chars (Trojan Source). Root: fmt/build.rs:68 and :88 `c => o.push(c)`.
- F6 MED f6 (DoS): trailing same-line comments quadratic (32000 `/**/` after a stmt = 1.4 s). Root: build.rs:186 newlines_between(hi, c.lo) rescans each time.
- X1 MED x1: E0006 fix "remove the space before `?`" is a no-op (replaces `?` by `?`, span q). Root: parser/expr.rs:~203 Fix::replace(.., q, "?").
- X2 LOW x2: `a?b;` gives two contradictory E0006 (parse_postfix Question branch calls parse_ternary_rest which reports again).
- S1 MED s1: `f(a0 < b0, ..., a150 < b150)` (also array literal) -> false E0012; threshold n=128. Root: per-file spec_cap = 24*toks+20000 (parser/mod.rs attempt) vs try_generic_args (expr.rs:727) scanning to end of list per `<`.
- S2 MED s2/s2b: else-if ladder >~125 branches and ternary chains >128 -> E0011 (valid flat input). Root: stmt.rs:81/471 enter() on else branch; expr.rs ternary else enter().
- L1 MED l1/l1b: lexer diagnostics uncapped (100000 junk bytes -> 100000 diags); MAX_DIAGS only for parser (lexer.rs:313/358).
- L2 LOW: `1.`/`1e` messages say "integer literal". L3 LOW: CR-only line endings: `//` comment runs past CR; formatter emits bare CR.
- K1 HIGH(ESP32) k1 (ignored, aborts, verified SIGABRT): 30 nested parens don't fit 64 KB (parser ~3 KB/level native release, ~18 KB/level debug).
- K2 MED k2 (ignored, verified abort at 32 KB; passes at 64 KB): Drop of 900-term `a + a ...` chain on caller stack needs 32-64 KB.
- K3 (ignored, verified abort at 32 KB): same drop inside format(); formatter itself on wasm: 271 KB for 900-long `.b` chain.
- K4 k4 (re-exec under ulimit -v 110000 so no thread): 100 nested parens inline on 256 KB overflow (release ~303 KB).
- Info: wasm32-wasip1 release default stacks OK at cap for all constructs; wasm DEBUG: 90+ nested parens trap (out of bounds memory = 1 MB shadow stack); existing fuzz.rs passes on wasi release; existing misc.rs `deep_but_legal_input_formats_on_a_small_stack` unwraps thread spawn -> fails on wasi (test portability, not parser).
- Info: diag.rs cites tests/codes_registered.rs which does not exist (all 18 used codes ARE registered, but unenforced).

## Held up
No panics; spans in range; must-parenthesise fixes correct; `>>`/`>=`/`>>=` splitting, `?.5`, generic-call-vs-`<`, arrow/ternary rule 2, switch arms OK; comment-free structured fuzz: 24863 clean programs all idempotent + AST-preserving; speculation linear on other shapes; no lexer hangs; every used code registered.

## Remaining (was doing)
Re-running wasm32-wasip1 debug fuzz suite (interrupted); n=cap stack table (max accepted n per construct, native debug/release and wasm; wasm table measured at n=100 in wasm_table.txt, e.g. paren 107 KB, switchx 131 KB, member/call 900 fmt 271 KB engine stack). Then final report via SubagentHandback.
