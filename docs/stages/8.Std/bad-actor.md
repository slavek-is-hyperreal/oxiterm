# BAD ACTOR report: 8.Std v0 (commit 90fa662)

Tests: `oxiscript/oxiscript-lower/tests/bad_actor_8std.rs` (27 tests: 19 pass, 7 fail, 1 ignored/info) and
`oxiscript/oxiscript-check/tests/bad_actor_8std.rs` (10 tests, all pass). Regression runs of `cargo test` and
`cargo clippy --all-targets -D warnings` for all ten oxiscript crates were green on 90fa662 (the only clippy errors seen were in my own scratch tests, since fixed).
ULP checks compare against Rust `std` f64 with a 2-ULP tolerance (Rust's own libm is not correctly rounded), so they confirm only "<= 2", not the documented 1-ULP bounds.

## Failing tests -> findings

### BS01 HIGH: `parseFloat` is not correctly rounded; `f64::MAX` text becomes `inf`
Test `string_parse_float_correct_rounding`. Repro: `parseFloat("1.7976931348623157e308")` returns +inf (bits 0x7FF0...), Rust gives `f64::MAX`.
Also `"0.000000000000000000000000000001"` is 1 ULP below `1e-30`; `"2.4703282292062328e-324"` gives 0 (should round up to 5e-324); a long exact-halfway subnormal text is 1 ULP low.
Expected: correct rounding (docs say "parses decimal floating-point literal"; the language literal lexer and `parseFloat` should agree). Actual: lossy accumulate-and-scale.
Info (undocumented): accepts leading/trailing spaces, `inf`, `nan`, case-insensitively.

### BS02 MEDIUM: `toFixed` rounds the wrong way / gives inexact digits
Test `string_to_fixed_matches_exact_decimal` (ties and "-0" excluded, those follow JS and are fine). Repro: `toFixed(0.045, 2)` = "0.05" (exact value 0.04499999..., correct "0.04"); `toFixed(0.1, 20)` = "0.10000000000000000000" (exact 0.10000000000000000555); `toFixed(0.7, 20)`, `-0.4`, `0.9999999` at 20 digits likewise. Digits beyond ~16 are zero-padded/approximate. std.md claims nothing about this; it says `0..20` digits. Either document the limit or compute exactly.

### BS03 MEDIUM: `toFixed(f64::MAX, n)` traps `overflow`
Test `string_to_fixed_f64_max_does_not_trap`. Repro: `toFixed(1.7976931348623157e308, 2)`. Expected a 312-char string (Rust format) or a documented Err/limit; actual a guest trap.

### BS04 MEDIUM: NaN keys grow `Map`/`Set` forever and are unreachable
Test `map_nan_key_single_entry`. Repro: `m.set(NaN, 1); m.set(NaN, 2); m.size()` is 2 and `m.get(NaN)` is null; `remove(NaN)` can never succeed. Cause: `equals` follows `===` (NaN !== NaN) while the hash is canonicalised. std.md says hash/equals are "consistent with `===`" (true) but does not warn about NaN keys. Expected: either NaN equals NaN as a key (SameValueZero), or documented and rejected.

### BS05 MEDIUM: `math_trig_huge_arguments`: single instance traps `out_of_memory` in `tan` (zero-allocation claim)
Repro (test is the repro): one instance, xorshift64 seed 0x123456789abcdef1, exponent in 1..2040, alternating sign, call `sin(x)` then `tan(x)` per iteration; at iteration 11796 (x = -2.8336086287247696e100, bits 0xd4c9e904ee13df45) `tan` traps `out_of_memory`. The same x on a fresh instance succeeds, and `deinit` reports 0 live objects after 3000 calls of `sin(1e300)`, so memory is exhausted without being counted as a leak (fragmentation or an unfreed Payne-Hanek table array per call). Expected: math functions are pure, repeatable for any call count. (The test also checks <= 2 ULP; the first failure seen was the trap.)

### BS06 LOW: invalid radix is silently clamped
Test `string_invalid_radix_and_digits_do_not_return_garbage`. `toRadix(10, 0|1|-5)` returns "1010" (treated as 2); `toRadix(10, 37)` returns "a" (treated as 10?); docs say "base 2..36" with no error path. `toFixed(1.5, -1)` = "2", `toFixed(1.5, 21|100)` = 20 digits. Expected: trap or documented clamp.

### BS07 LOW (throughput): sorting 30,000 `i32` exceeds the default 10M fuel
Test `array_sort_30k_within_default_fuel`. `sortI32` of 30k elements traps OutOfFuel; ~330 fuel per element. Map/Set at 20k set/has pairs and 50k set+remove churn also need > 10M fuel (they pass here only with raised fuel). Not a correctness bug; the practical limit should be written in std.md.

### BS08 INFO: collisions and cycles
`map_equal_hash_keys_correct` (ignored): 600 keys with a user-constant `hash()` exhaust default fuel (O(n^2) probing); true of any hash table, but built-in `hash()` is unseeded FNV-1a / invertible SplitMix64, so HashDoS on string keys is feasible; std.md should say Map/Set are not DoS resistant.
Reference cycle through a Map (`a.m.set(1, b); b.m.set(1, a)` with `a.next=b; b.next=a`) leaves 14 objects live at deinit: normal RC cycle behaviour, but contradicts the acceptance wording "zero leaks" for programs using Map; document it.

## Passing (attacks that held)
- Math special values (NaN, +-0, +-inf, subnormals, extremes) for all 14 functions on a 25x25 grid: within 2 ULP, zero signs correct; exp/log/pow tails and cross-run determinism OK.
- `std/..` path tricks (`std/../x`, `std//math`, `std/math.oxs`, `STD/math`, ...) all give OXS-E0302; entry module named `std/*` cannot use `@intrinsic`; a user module that obtains the canonical name `std/math` through `./std/math` can neither shadow std nor forge `@intrinsic` (diagnostics fire). std sources contain no `host:`.
- Sort: stable, total order with NaN last and -0 < +0, constant/inconsistent comparator terminates, trapping comparator propagates, empty/one element OK.
- Map/Set: order after remove+reinsert, churn, string keys with equal bytes but different provenance, f32 -0/+0, integer extremes, clear/reuse, mutation in forEach: correct and leak-free.
- Docs: every documented function exists and every export is documented (the `*F32` variants are covered by one prose sentence); AGENTS.md and `llms.txt` are in sync (`sync_docs.py` produced no diff); a Map key without hash/equals or an array key is rejected.
- parseInt/parseInt32 edge cases (sign, overflow, i64::MIN, non-ASCII digits, NBSP), UTF-8 split/replace/pad, huge `repeat`/`padStart` are bounded by traps.
