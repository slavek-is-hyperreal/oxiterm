# OxiScript Standard Library Reference (`std/*`)

Stage `8.Std v0`. All standard-library modules are bundled into the toolchain at build time (`oxiscript/std/*.oxs`), so `import { ... } from "std/<module>";` works identically in `oxi check`, `oxi run`, `oxi build`, tests, and the browser playground without filesystem access.

## Module resolution and `@intrinsic`

* Available modules: `std/math`, `std/string`, `std/array`, `std/result`, `std/collections`.
* Importing an unknown `std/...` module reports `OXS-E0302` with the list of available standard-library modules.
* User projects cannot shadow `std/...` modules.
* **Compiler intrinsics (`@intrinsic("name")`):** functions in bundled `std/*.oxs` files that map 1:1 to OxiIR/Wasm instructions (`abs`, `sqrt`, `floor`, `ceil`, `trunc`, `nearest`, `min`, `max`, `clz`, `ctz`, `popcnt`, `bitcast`, `str_byte`, `str_cmp`, `str_from_utf8`) are marked `@intrinsic("...")`. The checker accepts `@intrinsic` **only** inside bundled `std` modules; using `@intrinsic` in user code is rejected with `OXS-E0123`.

## Built-in members (always available without imports)

* **Arrays `T[]`:** `length`, `push(v)`, `pop()` (`T | null`), `map(f)`, `filter(f)`, `reduce(f, init)`, `forEach(f)`, `slice(lo, hi)`, `some(f)`, `every(f)`, `concat(other)`
* **Strings `string`:** `byteLength`, `bytes()`, `chars()`, `slice(lo, hi)`, `startsWith(p)`, `endsWith(p)`, `includes(p)`, `indexOf(p)`, `hash()` (`u64`), `equals(other)` (`bool`)
* **Numbers (`i8..u64`, `f32`, `f64`):** `toString()`, `wrapping<T>()`, `saturating<T>()`, `hash()` (`u64`), `equals(other)` (`bool`)
* **Booleans `bool`:** `toString()`, `hash()` (`u64`), `equals(other)` (`bool`)
* **Unions:** `downcast<Variant>()`
* **Prelude:** `Result<T, E>`, `Ok(v)`, `Err(e)`, `range(a, b)`

---

## `std/math`

Hardware-mapped float/integer intrinsics, numeric constants, bit rotations, integer helpers, and deterministic software transcendental functions.

### Constants

| Name | Type | Description |
|---|---|---|
| `PI`, `TAU`, `E`, `LN2`, `LN10`, `LOG2E`, `LOG10E`, `SQRT2`, `FRAC_1_SQRT2`, `FRAC_PI_2`, `FRAC_PI_4` | `f64` | Mathematical constants |
| `INF`, `NEG_INF`, `NAN`, `EPSILON`, `F64_MAX`, `F64_MIN`, `F64_MIN_POSITIVE` | `f64` | IEEE-754 binary64 special values and limits |
| `F32_INF`, `F32_NEG_INF`, `F32_NAN`, `F32_EPSILON`, `F32_MAX`, `F32_MIN`, `F32_MIN_POSITIVE` | `f32` | IEEE-754 binary32 special values and limits |
| `I8_MIN`, `I8_MAX`, `U8_MIN`, `U8_MAX` | `i8` / `u8` | 8-bit integer bounds |
| `I16_MIN`, `I16_MAX`, `U16_MIN`, `U16_MAX` | `i16` / `u16` | 16-bit integer bounds |
| `I32_MIN`, `I32_MAX`, `U32_MIN`, `U32_MAX` | `i32` / `u32` | 32-bit integer bounds |
| `I64_MIN`, `I64_MAX`, `U64_MIN`, `U64_MAX` | `i64` / `u64` | 64-bit integer bounds |

### Float intrinsics & helpers (`f64` and `f32`)

* `abs(x: f64): f64`, `absF32(x: f32): f32` — absolute value (Wasm `abs`).
* `sqrt(x: f64): f64`, `sqrtF32(x: f32): f32` — IEEE-exact square root (Wasm `sqrt`).
* `floor(x: f64): f64`, `floorF32(x: f32): f32` — round toward $-\infty$ (Wasm `floor`).
* `ceil(x: f64): f64`, `ceilF32(x: f32): f32` — round toward $+\infty$ (Wasm `ceil`).
* `trunc(x: f64): f64`, `truncF32(x: f32): f32` — round toward zero (Wasm `trunc`).
* `round(x: f64): f64`, `roundF32(x: f32): f32` — round to nearest, ties to even (Wasm `nearest`).
* `min(a: f64, b: f64): f64`, `minF32(a: f32, b: f32): f32` — IEEE minimum, NaN-propagating (Wasm `min`).
* `max(a: f64, b: f64): f64`, `maxF32(a: f32, b: f32): f32` — IEEE maximum, NaN-propagating (Wasm `max`).
* `copysign(x: f64, y: f64): f64`, `copysignF32(x: f32, y: f32): f32` — magnitude of `x` with sign of `y`.
* `bitsOf(x: f64): u64`, `bitsOfF32(x: f32): u32` — bitcast float to integer (NaN canonicalised to `0x7ff8000000000000` / `0x7fc00000`).
* `fromBits(bits: u64): f64`, `fromBitsF32(bits: u32): f32` — bitcast integer to float.
* `isNaN(x: f64): bool`, `isFinite(x: f64): bool`, `isInfinite(x: f64): bool` (and `*F32` variants).
* `clamp(x: f64, lo: f64, hi: f64): f64`, `clampF32(x: f32, lo: f32, hi: f32): f32`.
* `sign(x: f64): f64`, `scalbn(x: f64, n: i32): f64` — sign and exact $x \cdot 2^n$ scaling.

```oxs
import { sqrt, floor, round, clamp, bitsOf, fromBits, PI } from "std/math";

export function circleStep(r: f64): f64 {
  const clamped = clamp(r, 0.0, 100.0);
  const area = PI * clamped * clamped;
  const rt = fromBits(bitsOf(sqrt(area)));
  return round(floor(rt) + 0.5);
}
```

### Integer bit operations & arithmetic helpers

* Bit counts (Wasm `clz`, `ctz`, `popcnt`):
  * `clz8(x: u8): u8`, `ctz8(x: u8): u8`, `popcnt8(x: u8): u8`
  * `clz16(x: u16): u16`, `ctz16(x: u16): u16`, `popcnt16(x: u16): u16`
  * `clz32(x: u32): u32`, `ctz32(x: u32): u32`, `popcnt32(x: u32): u32`
  * `clz64(x: u64): u64`, `ctz64(x: u64): u64`, `popcnt64(x: u64): u64`
  * `clzI32(x: i32): i32`, `ctzI32(x: i32): i32`, `popcntI32(x: i32): i32`
  * `clzI64(x: i64): i64`, `ctzI64(x: i64): i64`, `popcntI64(x: i64): i64`
* Rotations: `rotl8`, `rotr8`, `rotl16`, `rotr16`, `rotl32`, `rotr32`, `rotl64`, `rotr64`.
* Integer min / max / abs / clamp:
  * `minI32`, `maxI32`, `absI32`, `clampI32`
  * `minI64`, `maxI64`, `absI64`, `clampI64`
  * `minU32`, `maxU32`, `clampU32`
  * `minU64`, `maxU64`, `clampU64`
* Wrapping & saturating helpers:
  * `wrappingAddI32`, `wrappingSubI32`, `wrappingMulI32`, `wrappingAbsI32`
  * `saturatingAddI32`, `saturatingSubI32`, `saturatingMulI32`, `saturatingAbsI32`
  * `wrappingAddI64`, `wrappingSubI64`, `wrappingMulI64`, `wrappingAbsI64`
  * `saturatingAddI64`, `saturatingSubI64`, `saturatingMulI64`, `saturatingAbsI64`
  * `wrappingAddU32`, `wrappingSubU32`, `wrappingMulU32`, `saturatingAddU32`, `saturatingSubU32`, `saturatingMulU32`
  * `wrappingAddU64`, `wrappingSubU64`, `wrappingMulU64`, `saturatingAddU64`, `saturatingSubU64`, `saturatingMulU64`

```oxs
import { clz32, popcnt32, rotl32, clampI32, saturatingAddI32, I32_MAX } from "std/math";

export function bitSummary(x: u32, n: i32): i32 {
  const c = (clz32(x) + popcnt32(rotl32(x, 3u32))) as i32;
  return clampI32(saturatingAddI32(n, c), 0, I32_MAX);
}
```

### Deterministic transcendental functions (`f64`)

All 14 transcendental functions are implemented in pure OxiScript (IEEE-754 binary64, no FMA, no host libm) by porting the algorithms and coefficients of Sun Microsystems FDLIBM / FreeBSD msun / musl libm (MIT / permissive Sun licence). Every target (`device`, `web`, `server`) produces bit-identical results.

* `sin(x: f64): f64`
* `cos(x: f64): f64`
* `tan(x: f64): f64`
* `asin(x: f64): f64`
* `acos(x: f64): f64`
* `atan(x: f64): f64`
* `atan2(y: f64, x: f64): f64`
* `exp(x: f64): f64`
* `log(x: f64): f64`
* `log2(x: f64): f64`
* `log10(x: f64): f64`
* `pow(x: f64, y: f64): f64`
* `cbrt(x: f64): f64`
* `hypot(x: f64, y: f64): f64`

Measured maximum ULP error against Rust `std::f64` on $10^6$ seeded inputs per function (see `oxiscript-lower/tests/std_math_ulp.rs`):

<!-- ULP_TABLE_START -->
| Function | Documented bound | Measured max ULP ($10^6$ inputs) | Worst input |
|---|---|---|---|
| `sin` | $\le 1$ ULP | 1 | `x = -4.611825235119253e2` |
| `cos` | $\le 1$ ULP | 1 | `x = -8.092203429416439e86` |
| `tan` | $\le 2$ ULP | 2 | `x = 4.63661124573382e4` |
| `asin` | $\le 1$ ULP | 1 | `x = -5.88260440115772e-1` |
| `acos` | $\le 1$ ULP | 1 | `x = -6.194281456556301e-1` |
| `atan` | $\le 1$ ULP | 1 | `x = 3.6854680519401413e0` |
| `atan2` | $\le 1$ ULP | 1 | `(-2.815916693408974e88, -1.8356021879841668e32)` |
| `exp` | $\le 1$ ULP | 1 | `x = -1.2627807402478473e1` |
| `log` | $\le 1$ ULP | 1 | `x = 7.557200456059845e-1` |
| `log2` | $\le 1$ ULP | 1 | `x = 1.490489118753667e0` |
| `log10` | $\le 2$ ULP | 2 | `x = 1.7286000711864429e0` |
| `pow` | $\le 1$ ULP | 1 | `(1.0852565938554795e0, 4.400500603782193e2)` |
| `cbrt` | $\le 1$ ULP | 1 | `x = -1.517374502826836e40` |
| `hypot` | $\le 1$ ULP | 1 | `(4.3617230549416186e282, 5.902113250311187e284)` |
<!-- ULP_TABLE_END -->

```oxs
import { sin, cos, atan2, exp, log, pow, cbrt, hypot } from "std/math";

export function polarStep(x: f64, y: f64): f64 {
  const r = hypot(x, y);
  const a = atan2(y, x);
  return sin(a) * cos(a) + exp(log(r + 1.0)) + pow(cbrt(r), 3.0);
}
```

---

## `std/string`

UTF-8 string utilities, parsing (`Result`), and deterministic number formatting.

* `isEmpty(s: string): bool` — true if `s.byteLength === 0`.
* `charCount(s: string): i32` — count of Unicode scalar values in `s`.
* `byteAt(s: string, idx: i32): u8` — byte at index `idx` (traps `bounds` if out of range).
* `compare(a: string, b: string): i32` — UTF-8 / code-point lexicographic order (`-1`, `0`, or `1`).
* `fromUtf8(bytes: u8[]): Result<string, string>` — validates UTF-8 and builds a `string`.
* `fromCodePoint(cp: u32): Result<string, string>` — encodes a single Unicode scalar value.
* `trim(s: string): string`, `trimStart(s: string): string`, `trimEnd(s: string): string` — trims ASCII whitespace.
* `split(s: string, sep: string): string[]` — splits on `sep` (`""` splits into individual Unicode scalar values).
* `replace(s: string, from: string, to: string): string` — replaces first occurrence.
* `replaceAll(s: string, from: string, to: string): string` — replaces all non-overlapping occurrences.
* `repeat(s: string, count: i32): string` — repeats `s` `count` times (`""` if `count <= 0`).
* `padStart(s: string, targetLen: i32, pad: string = " "): string` — pads on the left to `targetLen` code points.
* `padEnd(s: string, targetLen: i32, pad: string = " "): string` — pads on the right to `targetLen` code points.
* `toUpperAscii(s: string): string`, `toLowerAscii(s: string): string` — ASCII case conversion (preserves non-ASCII UTF-8 bytes).
* `lastIndexOf(s: string, needle: string): i32` — byte index of last occurrence or `-1`.
* `parseInt(s: string, radix: i32 = 10): Result<i64, string>` — parses signed 64-bit integer in base `2..36`.
* `parseInt32(s: string, radix: i32 = 10): Result<i32, string>` — parses signed 32-bit integer in base `2..36`.
* `parseFloat(s: string): Result<f64, string>` — parses decimal floating-point literal (including `NaN`, `Infinity`, `-Infinity`).
* `toRadix(n: i64, radix: i32): string`, `toRadixU64(n: u64, radix: i32): string`, `toRadixI32(n: i32, radix: i32): string` — formats integer in base `2..36`.
* `toFixed(x: f64, digits: i32 = 0): string` — formats `f64` with fixed decimal places (`0..20`).

```oxs
import {
  trim,
  split,
  toUpperAscii,
  padStart,
  parseInt,
  parseFloat,
  toRadix,
  toFixed,
} from "std/string";

export function formatRecord(line: string): Result<string, string> {
  const parts = split(trim(line), ":");
  if (parts.length < 3) {
    return Err("expected name:id:score");
  }
  const name = toUpperAscii(parts[0]);
  const id = parseInt(parts[1], 10)?;
  const score = parseFloat(parts[2])?;
  const hexId = padStart(toRadix(id, 16), 4, "0");
  return Ok(`${name}#${hexId}=${toFixed(score, 2)}`);
}
```

---

## `std/array`

Stable deterministic merge sort, searching, reversing, and joining.

* `isEmpty<T>(xs: T[]): bool` — true if `xs.length === 0`.
* `first<T>(xs: T[]): T | null`, `last<T>(xs: T[]): T | null` — first/last element or `null`.
* `sortBy<T>(xs: T[], cmp: (a: T, b: T) => i32): T[]` — stable bottom-up merge sort with a fixed comparison sequence (non-mutating).
* `sortI32(xs: i32[]): i32[]`, `sortI64(xs: i64[]): i64[]`, `sortU32(xs: u32[]): u32[]`, `sortU64(xs: u64[]): u64[]` — ascending numeric sorts.
* `sortF64(xs: f64[]): f64[]` — deterministic total-order float sort (`-0.0 < +0.0`, `NaN` last).
* `sortString(xs: string[]): string[]` — lexicographic UTF-8 sort.
* `isSortedBy<T>(xs: T[], cmp: (a: T, b: T) => i32): bool` — verifies sortedness under `cmp`.
* `reverse<T>(xs: T[]): T[]`, `reverseInPlace<T>(xs: T[]): void` — copy-reversing and in-place reversing.
* `indexOf<T extends Eq<T>>(xs: T[], target: T): i32` — first matching index or `-1`.
* `includes<T extends Eq<T>>(xs: T[], target: T): bool` — membership test.
* `find<T>(xs: T[], pred: (x: T) => bool): T | null`, `findIndex<T>(xs: T[], pred: (x: T) => bool): i32`.
* `join(xs: string[], sep: string): string` — joins strings with separator `sep`.
* `fill<T>(value: T, count: i32): T[]` — creates an array of `count` copies of `value`.
* `flatMap<T, U>(xs: T[], f: (x: T) => U[]): U[]` — maps and flattens one level.

```oxs
import { sortI32, reverse, indexOf, includes, find, join } from "std/array";

export function topScore(scores: i32[], labels: string[]): string {
  const desc = reverse(sortI32(scores));
  const best = find(desc, (x: i32): bool => x >= 50) ?? 0;
  const hasHundred = includes(desc, 100);
  const pos = indexOf(labels, "gold");
  return `${join(labels, ",")}|best=${best}|100=${hasHundred}|gold=${pos}`;
}
```

---

## `std/result`

Combinators and helpers for `Result<T, E>`.

* `isOk<T, E>(r: Result<T, E>): bool`, `isErr<T, E>(r: Result<T, E>): bool`
* `unwrapOr<T, E>(r: Result<T, E>, fallback: T): T`
* `unwrapOrElse<T, E>(r: Result<T, E>, f: (err: E) => T): T`
* `unwrapErrOr<T, E>(r: Result<T, E>, fallback: E): E`
* `okValue<T, E>(r: Result<T, E>): T | null`, `errValue<T, E>(r: Result<T, E>): E | null`
* `map<T, U, E>(r: Result<T, E>, f: (v: T) => U): Result<U, E>`
* `mapErr<T, E, F>(r: Result<T, E>, f: (e: E) => F): Result<T, F>`
* `mapOr<T, U, E>(r: Result<T, E>, fallback: U, f: (v: T) => U): U`
* `andThen<T, U, E>(r: Result<T, E>, f: (v: T) => Result<U, E>): Result<U, E>`
* `orElse<T, E, F>(r: Result<T, E>, f: (e: E) => Result<T, F>): Result<T, F>`
* `flatten<T, E>(r: Result<Result<T, E>, E>): Result<T, E>`
* `all<T, E>(rs: Result<T, E>[]): Result<T[], E>`

```oxs
import { map, andThen, unwrapOr, all } from "std/result";
import { parseInt32 } from "std/string";

export function sumParsed(items: string[]): Result<i32, string> {
  const parsed = items.map((s: string): Result<i32, string> => parseInt32(s));
  const vals = all(parsed)?;
  const doubled = andThen(Ok<i32, string>(vals.length), (n: i32): Result<i32, string> => Ok(n * 2));
  const total = unwrapOr(map(doubled, (n: i32): i32 => n + 1), 0);
  return Ok(total);
}
```

---

## `std/collections`

Deterministic open-addressing hash collections (`Map<K, V>` and `Set<T>`) with exact insertion-order iteration.

### Interfaces & helpers

* `interface Hash { hash(): u64; }`
* `interface Eq<T> { equals(other: T): bool; }`
* `interface HashEq<T> extends Hash, Eq<T> {}`
* `hashCombine(seed: u64, value: u64): u64` — mixes two 64-bit hashes for composite structs/classes.
* All primitive numeric types (`i8..u64`, `f32`, `f64`), `bool`, and `string` natively provide `.hash(): u64` and `.equals(other): bool` satisfying `HashEq<Self>` (with `-0.0` canonicalised to `+0.0` and NaN canonicalised, consistent with `===`, asserted by laws in `std/collections.oxs`).

### `MapEntry<K, V>` and `Map<K extends HashEq<K>, V>`

* `struct MapEntry<K, V> { key: K; value: V; }`
* `new Map<K, V>()`
* `size(): i32`, `isEmpty(): bool`
* `has(key: K): bool`, `get(key: K): V | null`, `getOr(key: K, fallback: V): V`
* `set(key: K, value: V): void` — inserts at end if absent, or updates value in place preserving order.
* `remove(key: K): bool` — removes key if present (note: named `remove` because `delete` is a reserved keyword).
* `clear(): void`
* `keys(): K[]`, `values(): V[]`, `entries(): MapEntry<K, V>[]`, `forEach(f: (key: K, value: V) => void): void` — always in exact insertion order.

### `Set<T extends HashEq<T>>`

* `new Set<T>()`
* `size(): i32`, `isEmpty(): bool`
* `has(item: T): bool`, `add(item: T): bool`, `remove(item: T): bool`, `clear(): void`
* `values(): T[]`, `forEach(f: (item: T) => void): void` — always in exact insertion order.

```oxs
import { Map, Set, HashEq, hashCombine } from "std/collections";

struct Point implements HashEq<Point> {
  x: i32;
  y: i32;
  hash(): u64 {
    return hashCombine(this.x.hash(), this.y.hash());
  }
  equals(other: Point): bool {
    return this.x === other.x && this.y === other.y;
  }
}

export function countPoints(xs: Point[]): i32 {
  const counts = new Map<Point, i32>();
  const seen = new Set<string>();
  for (const p of xs) {
    const prev = counts.getOr(p, 0);
    counts.set(p, prev + 1);
    seen.add(`${p.x},${p.y}`);
  }
  return counts.size() + seen.size();
}
```
