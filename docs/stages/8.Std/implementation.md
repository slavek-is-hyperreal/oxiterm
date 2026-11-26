# Stage 8.Std v0 — Standard Library Start: Implementation Log

## Step 1: Std module resolution — done
- Created bundled standard library directory `oxiscript/std/` with `math.oxs`, `string.oxs`, `array.oxs`, `result.oxs`, `collections.oxs`.
- Embedded sources at build time via `include_str!` in `oxiscript/oxiscript-check/src/std_lib.rs` so the CLI, tests, lowering, and wasm32 browser playground all resolve `std/...` identically without filesystem access.
- `oxiscript_check::check_program` intercepts `"std"` and `"std/..."` before delegating to `ModuleLoader`, preventing user shadowing and producing a clear `OXS-E0302` diagnostic listing available std modules on unknown `std/...` imports.
- Added `ModuleLoader::on_std_loaded` callback and wired it into `FsLoader` in `oxiscript-cli/src/project.rs` so `FileId` indices stay synchronized with CLI source files even on syntax errors.
- Added `ModuleInfo::is_std` (set only for bundled std modules loaded from `std_lib::source`, never for user/entry modules).

## Step 2: `@intrinsic` mechanism and `std/math` intrinsics & constants — done
- Added `@intrinsic("name")` attribute validation in `oxiscript-check/src/decl.rs`:
  - Allowed only on non-generic top-level functions inside bundled std modules (`is_std == true`).
  - Rejected with `OXS-E0123` in user code (cannot be forged by naming the entry module `std/math`).
  - Validated against `std_lib::KNOWN_INTRINSICS`.
- Exposed `Checked::intrinsic_of(DefId) -> Option<&str>` in `oxiscript-check/src/output.rs`.
- Lowered `@intrinsic` in `oxiscript-lower` (`call.rs` inline at direct call sites + `driver.rs` in `lower_plain` for first-class function values) directly to single OxiIR instructions (`UnOp::{Abs,Sqrt,Floor,Ceil,Trunc,Nearest,Clz,Ctz,Popcnt}`, `BinOp::{Min,Max}`, `Op::Bitcast`, `Op::StrByte`, `Op::StrCmp`, `Op::StrFromUtf8`).
- Populated `oxiscript/std/math.oxs` with f64/f32 intrinsics, constants (`PI`, `TAU`, `E`, `LN2`, `LN10`, `LOG2E`, `LOG10E`, `SQRT2`, `INF`, `NEG_INF`, `NAN`, `EPSILON`, type `MIN`/`MAX`), `copysign`, predicates (`isNaN`, `isFinite`, `isInfinite`), `clamp`, `sign`, bit rotations (`rotl`/`rotr` 8/16/32/64), and integer `min`/`max`/`abs`/`clamp`/wrapping/saturating helpers.
- Verified in `oxiscript-check/tests/std_lib.rs` (5 tests) and `oxiscript-lower/tests/std_math.rs` (2 tests, zero leaks).

## Step 3: `std/string`, `std/array`, `std/result` — done
- Implemented `oxiscript/std/string.oxs`: `isEmpty`, `charCount`, `byteAt`, `compare`, `fromUtf8`, `fromCodePoint`, `trim`, `trimStart`, `trimEnd`, `split`, `replace`, `replaceAll`, `repeat`, `padStart`, `padEnd`, `toUpperAscii`, `toLowerAscii`, `lastIndexOf`, `parseInt`, `parseInt32`, `parseFloat`, `toRadix`, `toRadixU64`, `toRadixI32`, `toFixed` (pure OxiScript, works in Stub runtime without `str.from_f64`).
- Implemented `oxiscript/std/array.oxs`: `isEmpty`, `first`, `last`, stable bottom-up merge sort `sortBy` (fixed deterministic comparison sequence), `sortI32`, `sortI64`, `sortU32`, `sortU64`, `sortF64` (total order with `-0.0 < +0.0` and NaN last), `sortString`, `isSortedBy`, `reverse`, `reverseInPlace`, `indexOf` & `includes` (over `Eq<T>`), `find`, `findIndex`, `join`, `fill`, `flatMap`.
- Implemented `oxiscript/std/result.oxs`: `isOk`, `isErr`, `unwrapOr`, `unwrapOrElse`, `unwrapErrOr`, `okValue`, `errValue`, `map`, `mapErr`, `mapOr`, `andThen`, `orElse`, `flatten`, `all`.
- Verified in `oxiscript-lower/tests/std_core.rs` (4 tests, zero leaks).

## Step 4: `std/collections` (`Map<K, V>`, `Set<T>`, `Hash`, `Eq`, `HashEq`) — done
- Added builtin `.hash(): u64` (SplitMix64 for scalars, canonicalising `-0.0 -> +0.0` and canonical NaN for floats; FNV-1a 64-bit for strings) and `.equals(other: Self): bool` on all primitive numeric types, `bool`, and `string` in `oxiscript-check` (`builtins.rs`, `typeexpr.rs::find_method_sig`) and `oxiscript-lower` (`helpers.rs`, `call.rs` for both direct calls and monomorphised interface-bound calls).
- Implemented `oxiscript/std/collections.oxs`: interfaces `Hash`, `Eq<T>`, `HashEq<T>`, `hashCombine`, `MapEntry<K, V>`, `Map<K extends HashEq<K>, V>`, `Set<T extends HashEq<T>>` using open addressing (linear probing) over compact insertion-ordered entry arrays with automatic rehashing & compaction. Note: removal method is named `remove` because `delete` is a reserved keyword in OxiScript (`OXS-E0900`).
- Added 9 `law` declarations in `std/collections.oxs` asserting hash/equality consistency (`===` for integers/bool/string; SameValueZero for `f32`/`f64`).
- Verified in `oxiscript-lower/tests/std_collections.rs` including a 2,000-step differential fuzz test against Rust `HashMap` + insertion-order model (both `Map<i32, i32>`, `Map<string, i32>`, and `Set<i32>`) with zero acyclic leaks (reference cycles through `Map`/`Set` are reclaimed only by the cycle collector).

## Step 7: BAD ACTOR fixes (BS01–BS08) — done
- See `docs/stages/8.Std/fixes.md` for full details of the fixes for findings BS01–BS08.

## Step 5: Deterministic transcendental functions in `std/math` — done
- Ported Sun Microsystems FDLIBM / FreeBSD msun / musl libm (MIT / permissive Sun licence, cited in `oxiscript/std/math.oxs`) into pure OxiScript for all 14 transcendental functions (`sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `log`, `log2`, `log10`, `pow`, `cbrt`, `hypot`) plus `scalbn`.
- Pure scalar zero-allocation fast paths for all functions (Payne-Hanek multi-word reduction via 66-word `2/pi` switch table for `|x| >= 823549.6`).
- Verified IEEE-754 special-value matrices and 1,000,000 seeded inputs per function (14,000,000 total) against Rust `std::f64` in `oxiscript-lower/tests/std_math_ulp.rs` (`--release`, 36.04s, zero leaks):
  - 1 ULP max: `sin`, `cos`, `asin`, `acos`, `atan`, `atan2`, `exp`, `log`, `log2`, `pow`, `cbrt`, `hypot`.
  - 2 ULP max: `tan`, `log10`.

## Step 6: Documentation, playground, and quality checks — done
- Created `docs/oxiscript/std.md` covering all 5 std modules, built-in members, `@intrinsic` rules, signatures, measured 10^6-input ULP table, and checked `oxs` examples.
- Updated `oxiscript/oxiscript-cli/templates/common/AGENTS.md` (lead must run `python3 -I oxiscript/oxiscript-cli/tools/sync_docs.py` to regenerate root `AGENTS.md` and `docs/oxiscript/llms.txt`).
- Added `oxiscript-playground` unit test (`std_modules_resolve_and_run_in_playground`) and `oxiscript-cli/tests/std_docs.rs` verifying every `oxs` block in `docs/oxiscript/std.md` (check + fmt) and CLI end-to-end `oxi run`.
- Verified `clippy --all-targets -- -D warnings` on `oxiscript-check`, `oxiscript-lower`, `oxiscript-playground` (default & `run`), and `oxiscript-cli`, and `cargo build -p oxiscript-playground --target wasm32-unknown-unknown`.


