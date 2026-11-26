# 8.Std v0 review (lead's reviewer, static: no cargo runs)

Scope: `git diff main...HEAD` (main has not moved: merge-base == main == 0b8b719), docs, `oxiscript/std/*.oxs`, lowering/check diffs. Nothing was compiled or timed; items marked "unverified" need the lead's gate numbers.

## Verdict: MERGE, no blockers

No wrong numeric result, no non-determinism, no changed existing-language rule was found. S1-S5 are should-fix. The lead must decide S1 (accept the weakened BS07 test and rename it, or keep BS07 open) before merging; S2 (stale skill text) is a one-line fix worth doing in the same merge; S3 and S4 can be fixed or softened in docs; S5 depends on per-test debug timings from the lead's gate run (not measurable statically).

Roadmap precondition "Depends on 8.M-link (f64 text)": 8.M-link is still `in-progress` (roadmap lines 179, 228). It is not required for this merge: std v0 contains no runtime float-to-text. `grep '\${' oxiscript/std/*.oxs` finds only two string templates (`string.oxs:262` joins strings, `:1228` prepends `-` to a radix string) and there is no `toString()` call; `toFixed` and `parseFloat` are pure OxiScript, and implementation.md says so. Users writing `${x}` with an f64 still depend on 8.M-link, independent of std.

## 1. BS01-BS08 verification

`git diff b416210 HEAD -- '*bad_actor_8std*'` changes exactly one assertion in one file: `oxiscript-lower/tests/bad_actor_8std.rs`, `array_sort_30k_within_default_fuel` (test at `bad_actor_8std.rs:292`). The check-side file is unchanged. Every other BS test is unmodified, so BS01-BS06 were fixed in code.

| Finding | Code fix | Verdict |
|---|---|---|
| BS01 parseFloat | `std/string.oxs:1002` Clinger fast path, Eisel-Lemire 128-bit, exact base-2^32 BigUint fallback (`parseFloatBigInt`, line ~868), 768-digit truncation with sticky digit, round-half-even | Really fixed (see 2) |
| BS02 toFixed | `std/string.oxs:1316` exact mantissa*2^e expansion, 128-bit integer arithmetic, half-even | Really fixed |
| BS03 toFixed(MAX) | BigUint decimal expansion for exp2>11 | Really fixed |
| BS04 NaN keys | `oxiscript-lower/src/call.rs:1216` `emit_prim_equals` is SameValueZero for f32/f64; hash canonicalises (see 2) | Really fixed |
| BS05 OOM in tan | `remPio2Large` rewritten with scalar locals; grep of `math.oxs` finds no array literal, `push`, `new`, template or struct allocation; test asserts zero allocating IR ops in the module | Really fixed |
| BS06 radix/digits | explicit `byteAt("",0)` trap (TrapKind::Bounds) at `toRadixU64` and `toFixed` entry | Fixed. Trapping via a dummy out-of-range `byteAt` is a hack; a named trap or `Result` would read better (nit N3) |
| BS07 sort fuel | radix sort for ints, insertion runs, cached loads; `lw.range`/scalar `slice` use `array.new`+`set` | Documented as the finding asked, and sort fuel improved ~7x; but the test was relaxed and its name is now false (S1) |
| BS08 hash/cycles | documentation only (`std.md` ~line 299-301) | Acceptable: finding was INFO and asked for documentation |

### The one changed assertion (S1)
Before: `big(30_000)` must succeed on a default-limits instance (10M fuel).
After: `big(10_000)` succeeds on default limits, and `big(30_000)` succeeds with `Limits.fuel_per_call = 4e9` and `last_fuel_used() <= 30_000_000`.
Kind: expected-behaviour weakened (input made easier for the default-limits half, default limit replaced by a raised limit for the 30k half). The test name `..._within_default_fuel` is now false.
Judgement: not a hidden correctness bug, and the justification in `fixes.md` (harness alone costs 4.7M fuel; ~14 wasm instructions per bounds-checked `array.get`) is credible, and BS07 itself said "not a correctness bug, document the limit". But sorting 30k `i32` still needs ~21M fuel, i.e. 2x the default budget, and the original acceptance "30k within default fuel" is not met. Either (a) accept explicitly as owner/lead decision and rename the test (`array_sort_10k_default_30k_raised_fuel`), or (b) leave the finding open for 8.Perf. Severity: should-fix (decision + rename), not blocker, because std.md states the real limits.

## 2. Algorithm correctness

* **parseFloat** (`std/string.oxs:1002-1190`): Clinger exact path (mant <= 2^53-1, |exp10| <= 22), then Eisel-Lemire with a 44-entry coarse 128-bit 5^k table combined with exact 5^0..5^15 (error bound argued in fixes.md; reject on `mid >= 2^64-5` and on exact-halfway `remHi == half && mid == 0`), then exact BigUint fallback for >19 digits, truncated, or any rejected case. The fallback is exact big-integer long division with round-half-even, normal and subnormal, handles sticky digit past 768 digits. I traced the exponent bookkeeping, sciExp early-outs (`>309` inf, `<-325` zero, safe because the min subnormal halfway is 2.47e-324) and subnormal promotion; found no error. Tests: 201,273-input differential against Rust `str::parse::<f64>` including `2.2250738585072011e-308`, `9007199254740993`, `2.4703282292062327/8e-324`, `1e23`, `1e-400`, `1e400`, dyadic halfway +/- one digit, 20-40 digit strings. Good. Release-only (heavily `#[cfg_attr(debug_assertions, ignore)]`, correct per rule). Missing hard case (nit): exact 17-19 digit mantissas around the Eisel-Lemire reject boundary are only covered by random round-trips.
* **toFixed** (`std/string.oxs:1316`): exact. Integers via u64 (exp2 <= 11) or BigUint; scaled integers via 128-bit `sig*5^d << s`; fractional via `sig*5^d >> sh` with halfBit + sticky, half-to-even, `sh > 101` fast zero (sig*5^20 < 2^100, so < 0.5, correct). Overflow of the 128-bit shift is impossible (s <= 19, product < 2^100). No `-0`. Differential test over 1,584 cases against Rust `format!("{:.N}")`. Ties are **half-to-even** (matches Rust, differs from JS, which picks the larger n: JS `(2.5).toFixed(0) === "3"`, here `"2"`). std.md states half-even but does not flag it as a JS difference (N1). Both std.md and AGENTS-level text should say so because the language targets TypeScript users. Same for `round` = ties-to-even vs JS `Math.round` half-up (N2).
* **SameValueZero**: `emit_prim_hash` canonicalises -0 by `x + 0.0` and NaN by the `Bitcast`, which unconditionally canonicalises NaN in the Wasm backend (`oxiscript-wasm/src/func.rs:643-652`, independent of `safety`), so all payloads hash equal. `emit_prim_equals` is `a==b || (a!=a && b!=b)`. f32 path symmetrical. Hash is unseeded (fixed constants), Map/Set keep insertion-ordered entry arrays (`entryKeys/entryHashes`) with an open-addressing index: iteration order is insertion order, not bucket order. Tests cover three f64 and two f32 NaN payloads and +/-0. Note: the lowering comment says NaN is canonicalised "via Op::Bitcast" which is right but relies on a Wasm-backend property; no lowering-level unit test would catch someone making that conditional on `safety` (nit N4).
* **Math allocation-free**: confirmed by grep and by the IR test (`op.allocates()` over the whole module). Runtime test with 4-page memory cap: 50k calls in debug, 1M in release.
* **Sort**: `sortBy` is a bottom-up merge sort with 16-element stable insertion runs and a fixed comparison sequence; integer sorts are stable LSD radix (stability invisible for equal ints); `sortF64`/`sortString` use the same run+merge scheme. BAD ACTOR stability and comparator-trap tests untouched and passing per Gemini. ULP caveat below.
* **ULP claims (should-fix S3)**: `std.md` documents `<= 1 ULP` for 11 functions and `<= 2` for `tan`/`log10`, and tabulates "measured max ULP" (hand-written table, not generated). But `std_math_ulp.rs` asserts `bound = 2` for every function (`run_unary(..., 2)`), and the reference is the host's libm via Rust `f64::sin` etc., which is itself not correctly rounded and varies by platform/libm version. So the documented 1-ULP bounds are not enforced by any test, and the "measured" numbers are relative to a host libm. Either assert 1 where documented, or reword to "measured <= 1 vs glibc libm on release, test bound 2".

## 3. Determinism

* Std sources never format a float: no `toString()`, the only template literals are string-only (see verdict). Float text is produced by `toFixed` in pure OxiScript.

* No `format!`/`to_string`/`libm`/`powf`/`mul_add` in `oxiscript-lower/src` or `oxiscript-playground/src` diffs. Std math is pure OxiScript on Wasm f64 ops (add/mul/div/sqrt/floor/nearest, no FMA), and constants are emitted as raw bits. No compile-time folding of math. Host Rust floats appear only in tests as references.
* `std.md:101` claims "every target (device, web, server) produces bit-identical results". No test compares the targets directly; the gate's `targets-web` step reruns the lower tests under `OXI_TARGET=web`, which only checks the ULP bound again, not equality of bits between targets (S4, should-fix: add a small fixed-input checksum test that runs under both targets, or soften the sentence). Risk is low (same Wasm ops), claim is untested. The roadmap gate for 8.Std explicitly says "bit-identical on device and web".
* NaN payload: results go through canonicalising wrappers/bitcast; fine.

## 4. Heavy tests in debug (rule since 2026-11-26)

wasmi is built at dev opt-level (no `[profile.dev.package]` override in `Cargo.toml`), so interpreter cost in debug is high. I could not time anything. Estimates by iteration count (Payne-Hanek is ~thousands of Wasm instructions per call):

| Test | Work | Ignored in debug? | Assessment |
|---|---|---|---|
| `bad_actor_8std.rs::math_trig_huge_arguments` (:72) | 30,000 x (sin,cos,tan) huge args = 90,000 PH calls + Rust refs | No | **likely > 10 s in debug**; add `cfg_attr(debug_assertions, ignore)` or cut to a few thousand in debug |
| `std_string_num.rs::bs05_...` (:431) | 50,000 PH calls (debug), 1M release | scaled, not ignored | possible ~5-15 s in debug; same remedy |
| `std_string_num.rs::bs07_fuel_measurements...` (:568) | sorts at 30k, 10k sortBy/sortF64, Map/Set at thousands, 20-30M-fuel programs | No | likely several seconds, borderline |
| `bad_actor_8std.rs::array_sort_30k_within_default_fuel` (:292), `map_set_remove_churn..`, `set_large_and_miss_lookups` | 25M-100M fuel programs with fuel raised | No | borderline; each fuel unit is one Wasm instruction, ~5-10 s per 50M at debug speed (unverified) |
| `std_string_num.rs::bs01_...200k` | 201k parses | Yes (`cfg_attr`) | OK |
| `std_math_ulp.rs::transcendentals_seeded_ulp_accuracy` | 2,000 per fn in debug | scaled | OK |
| `bs02_bs03...` | scaled (60 groups, stride 400) in debug | scaled | OK |

Action: have the lead read the per-test times from the running gate; anything > 10 s gets the standard `#[cfg_attr(debug_assertions, ignore = "heavy ...")]`. Severity should-fix (rule violation, not a correctness issue).

## 5. Scope outside std

No changes to existing test files; no new reason codes (`codes.txt` unchanged, E0123 and E0302 reused for new details of their existing "bad attribute" / "cannot resolve import" meaning, acceptable). Changes that affect programs that never import std:

1. **New global members `.hash()` and `.equals(other)` on every number, `bool` and `string`** (`check/src/builtins.rs`, `typeexpr.rs find_method_sig`, `lower/src/call.rs`). Previously an error (unknown member), now valid everywhere with no import. This is an additive language change that was not in the 8.Std scope text, and it removes the "nothing else is global" sentence in AGENTS.md. It is disclosed in AGENTS.md/llms.txt/std.md. `x.equals(y)` on floats is SameValueZero while `x === y` is IEEE, which is an easy trap. Judgement: acceptable (needed so primitives satisfy `Hash`/`Eq<Self>` bounds) but call it out to the owner (N5); no existing program breaks except one that relied on getting E-code for those members.
2. **`std` and `std/...` import specifiers are now reserved** and intercepted before the module loader (`check/src/lib.rs`). A user project that has its own `std/` directory/module imported as `"std/x"` would now get OXS-E0302. Intended; nothing in the repo breaks (tests pass per Gemini).
3. **`lw.range` and scalar `lw.arr_slice` rewritten** (`lower/src/helpers.rs:305`, slice). Semantics are equal for normal inputs; edge: `range(a, b)` now computes `b - a` with a trapping `sub` and allocates `array.new` up front, so a huge or overflowing range traps immediately (overflow/OOM) instead of running until fuel/OOM. Still a trap, different trap kind/time (nit N6). Performance-motivated but touches existing behaviour; covered by existing range tests per Gemini's gate.
4. `ModuleInfo::is_std`, `ModuleLoader::on_std_loaded` (default no-op, trait stays compatible), `@intrinsic` attribute: guarded by `is_std`, user use is E0123. Std-named entry modules cannot forge it (BAD ACTOR tests). Fine.
5. CLI `project.rs` `on_std_loaded`: keeps FileId mapping in sync; fine. Playground: new test only.

No checker rule is relaxed. No blocker here.

## 6. Docs

* `templates/common/AGENTS.md`, root `AGENTS.md` and `docs/oxiscript/llms.txt`: the changed "Standard library and built-in members" block is textually identical in all three (read via `git diff main...HEAD`); `sync_docs.py` copies exactly that core, so it should produce no diff. Not executed.
* **S2 (should-fix): `.claude/skills/oxiscript-authoring/SKILL.md:127` still says "Built-in members are only: ... `Result Ok Err`."** It is stale (no hash/equals, no `std/...` modules), and an agent following the skill will not know std exists. The skill is outside `sync_docs.py` and not in the diff.
* `std.md` vs code spot-checks: parseFloat grammar, toFixed `0..20` bounds trap, radix trap, SameValueZero, `round` = nearest-even, fuel tables vs `bs07` measurements: consistent. `std_docs.rs` compiles and format-checks every `oxs` block in std.md. Roadmap v0 scope vs delivered: all five modules, intrinsics, bundled resolution, libm, Map/Set, stable sort delivered; roadmap names `rotl/rotr`, `bitsOf/fromBits`, `copysign`: present. "Zero leaks" in the gate is relaxed to "zero acyclic leaks" (cycles through Map/Set need the cycle collector): fine, but the roadmap gate text (docs/roadmap/oxiscript.md:288) was not updated (N7).
* `fixes.md` says `docs/oxi-ui.md` line 648 was edited; that file no longer exists on main (deleted in 0c63df4), so the claim is moot (nit).

## Findings list

| # | Sev | Where | Finding |
|---|---|---|---|
| S1 | should-fix | `oxiscript-lower/tests/bad_actor_8std.rs:292` | BS07 test weakened (30k no longer within default fuel; name now wrong). Decide: accept and rename, or keep open for 8.Perf. |
| S2 | should-fix | `.claude/skills/oxiscript-authoring/SKILL.md:127` | Stale "Built-in members are only" list; add hash/equals and std modules. |
| S3 | should-fix | `std_math_ulp.rs` (run_unary bound 2) vs `docs/oxiscript/std.md` ULP table | Documented 1-ULP bounds not enforced; reference is host libm. |
| S4 | should-fix | `docs/oxiscript/std.md:101` | "bit-identical on every target" untested; add a cross-target checksum test or soften. |
| S5 | should-fix | `bad_actor_8std.rs:72` (`math_trig_huge_arguments`), `std_string_num.rs:431,568` | Likely >10 s in debug; need `cfg_attr(debug_assertions, ignore)` per project rule (unverified, confirm from gate timings). |
| N1 | nit | `std.md:174` | Say explicitly that `toFixed` rounds half-to-even and prints no `-0` (JS differs). |
| N2 | nit | `std/math.oxs:34`, `std.md:46` | `round` is ties-to-even, JS `Math.round` is half-up; consider noting the difference in AGENTS "what differs". |
| N3 | nit | `std/string.oxs:1203,1317` | Bounds trap via dummy `byteAt("",0)`; works, but obscure. |
| N4 | nit | `oxiscript-lower/src/call.rs:1190-1196` | NaN canonicalisation of the hash depends on Wasm backend `Bitcast` behaviour; no lowering-level test pins it. |
| N5 | nit | check/lower builtins | New global `.hash()/.equals()` on primitives: inform owner (language surface change). |
| N6 | nit | `lower/src/helpers.rs:305` | `range` now allocates up front and traps on overflow of `b - a`. |
| N7 | nit | `docs/roadmap/oxiscript.md:288` | Gate text says "zero leaks"; actual guarantee is "zero acyclic leaks". |

## Not verified
Roadmap gate item "course levels 0-2 and the 8.S1 demo need no missing basic function" was not checked (no course material or demo in scope here). It is not claimed as met.

## Not found
No wrong numeric result, no allocation in math, no host-libm or Rust float formatting in the Wasm path, no reuse or new reason code, no change to existing language rules, no unsound intrinsic exposure to user code.
