# AGENTS.md: writing OxiScript

OxiScript (`.oxs`) reads like TypeScript but is a smaller, stricter language that compiles to WebAssembly. You already know the syntax. This file lists only what differs, and the loop that keeps you correct.

## The loop

```text
edit -> oxi check --json -> apply machine-applicable fixes -> repeat until "ok": true
```

* `oxi check --json` prints one JSON document (schema 1). Every diagnostic has a stable `code` (`OXS-E0005`), a `range`, and `fixes`. Apply fixes whose `applicability` is `machine-applicable` without asking; `oxi check --fix` does exactly that and rewrites the files. `maybe-incorrect` fixes are suggestions: read them. `advice` fixes have no edit.
* Syntax errors hide type errors: after the syntax is clean, expect a second wave. Keep looping.
* `oxi explain OXS-E0100` explains any code with an example. `oxi fmt` formats (no options; it keeps comments, a BOM and CRLF line endings, and writes raw control or invisible characters inside string literals as escapes); run it before finishing.
* Never invent library functions. Everything a program can call is in its own code, the built-in members listed below, and the host interfaces under `idl/` (`import { Log } from "host:log"`).

<!-- oxs-core:begin -->
## What differs from TypeScript

1. **Numbers.** There is no `number`. Integers are `i8 i16 i32 i64 u8 u16 u32 u64`, floats are `f32 f64`. An unsuffixed integer literal takes the type its context needs (default `i32`), a float literal defaults to `f64`; suffixes fix the type (`200u8`, `1.5f32`; `1f32` is wrong, write `1.0f32`). Types never mix implicitly: convert with `as` (`n as f64`); a literal adapts to its context but a typed variable never does (`const x: f64 = n` with `n: i32` fails). `as` binds looser than `+ - * /`, so `a + b as f64` is `(a + b) as f64`: write `(a as f64) + (b as f64)`. Integer `/` yields an integer: to average, convert both operands first. Integer overflow traps; `+%` wraps and `+|` saturates; integer `/` truncates.
2. **`===` and `!==` only.** `==` is `OXS-E0005`. `===` is not defined on structs, arrays or unions: compare fields.
3. **`;` is mandatory** after every statement (`OXS-E0004`). `oxi check --fix` inserts it (`oxi check --fix-only semicolons` inserts only the `;`, and the VS Code extension does that on save; a `;` is never added where it would change the meaning, e.g. before a line that starts with `(` or `[`). `oxi fmt` refuses a file with syntax errors: fix first, then format.
4. **No exceptions.** No `throw`, `try`, `catch`. A function that can fail returns `Result<T, E>` (`Ok(v)`, `Err(e)`); `e?` unwraps `Ok` or returns the `Err` from the current function, which must itself return `Result<_, E>` with the same `E` (there is no automatic error conversion: map a different error with a `switch` first).
5. **`struct` is a value, `class` is a reference.** Data is a `struct` (copied on assignment; a method that assigns `this.x` must be `mutating`). Identity and shared mutation need a `class` (`new Counter(0)`). No `extends`; use `interface` + composition. `interface` describes behaviour only.
6. **No anonymous object types.** An object literal `{ x: 1.0 }` needs a type from context (a declared `struct`, a parameter, a return type). Sum types are TypeScript's discriminated unions: `type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };`. A `switch` over a union, an enum, a `bool` or `T | null` must be exhaustive and has no fall-through.
7. **Capabilities come in through `main`.** There are no globals such as `console`, `fetch`, `Math`, `process`. A program touches the outside world only through values its `export function main(log: Log, ...)` receives; their types come from `import { Log } from "host:log";`. Host calls return `Result<T, i32>` when they can fail (the `i32` is a host status code). In a project, `oxi check` enforces `oxi.toml` `needs`: every `host:` module the program imports must be listed (`OXS-E0400`), and `needs` must equal the host interfaces `main` takes, in parameter order (`OXS-E0402`).
8. **Strings are UTF-8.** `s.byteLength`, not `.length`; no `s[i]`; iterate with `s.chars()` (code points) or `s.bytes()`. Template literals work: `` `n = ${n}` ``. `+` does not join a string and a number.
9. **The `?` whitespace rule.** `f()?` (no space before `?`) is the try operator; `c ? a : b` (space before `?`) is the conditional; `a?.b` is optional chaining on `T | null`. Do not write `f() ?`.
10. **No `any`, `undefined`, `var`, `typeof`, `instanceof`, `for...in`, `async`, `delete`.** Absence is `T | null` (`null`, `??`, `?.`). Declarations need explicit parameter and return types where exported, and an exported function cannot be generic (generic helpers are fine unexported). `i++` is a statement, not a value. Mixing `&`, `|`, `<<`, `??` with other operators needs parentheses (`OXS-E0007`..`E0009`).

## Complete examples

Every block below is a whole module that passes `oxi check`.

```oxs
export function average(total: i64, count: i32): f64 {
  return (total as f64) / (count as f64);
}
```

```oxs
function parsePort(n: i32): Result<u16, string> {
  if (n < 1 || n > 65535) {
    return Err("port out of range");
  }
  return Ok(n as u16);
}

export function describe(n: i32): Result<string, string> {
  const port = parsePort(n)?;
  return Ok(`port ${port}`);
}
```

```oxs
import { Log } from "host:log";
import { State } from "host:state";

function bump(state: State, key: string): Result<i64, i32> {
  const slot = state.open(key)?;
  const next = state.get_i64(slot)? + 1;
  state.set_i64(slot, next)?;
  return Ok(next);
}

export function main(log: Log, state: State): void {
  const r = bump(state, "taps");
  switch (r.kind) {
    case "ok":
      log.write(2, `taps = ${r.value}`);
    case "err":
      log.write(4, `state failed with status ${r.error}`);
  }
}
```

```oxs
struct Point {
  x: f64;
  y: f64;
}

type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };

export function area(s: Shape): f64 {
  switch (s.kind) {
    case "circle":
      return 3.14159 * s.r * s.r;
    case "rect":
      return s.w * s.h;
  }
}

export function origin(): Point {
  return { x: 0.0, y: 0.0 };
}

law area_of_rect_is_commutative(w: f64, h: f64) {
  area({ kind: "rect", w: w, h: h }) === area({ kind: "rect", w: h, h: w })
}
```

A digit parser over code points; note the parentheses around the cast.

```oxs
export function parseDigits(s: string): Result<i32, string> {
  let total: i32 = 0;
  for (const ch of s.chars()) {
    if (ch < 48 || ch > 57) {
      return Err("not a digit");
    }
    total = total * 10 + ((ch - 48) as i32);
  }
  return Ok(total);
}
```

## Frequent mistakes and what the checker says

```oxs-err OXS-E0110
export function digit(total: i32, ch: u32): i32 {
  return total * 10 + (ch - 48) as i32;
}
```

```oxs-err OXS-E0104
function a(): Result<i32, i32> {
  return Ok(1);
}

export function b(): Result<i32, string> {
  const v = a()?;
  return Ok(v);
}
```

```oxs-err OXS-E0005
export function same(a: i32, b: i32): bool {
  return a == b;
}
```

```oxs-err OXS-E0300
export function half(x: number): number {
  return x / 2;
}
```

```oxs-err OXS-E0108
export function size(s: string): i32 {
  return s.length;
}
```

```oxs-err OXS-E0100
export function widen(n: i32): f64 {
  return n;
}
```

```oxs-err OXS-E0103
const p = { x: 1.0, y: 2.0 };
```

## Built-in members (the whole standard library today)

* arrays `T[]`: `length`, `push`, `pop` (returns `T | null`), `map`, `filter`, `reduce`, `forEach`, `slice`, `some`, `every`, `concat`
* strings: `byteLength`, `bytes()`, `chars()`, `slice`, `startsWith`, `endsWith`, `includes`, `indexOf`
* numbers: `toString()`, `x.wrapping<u8>()`, `x.saturating<u8>()`; unions: `x.downcast<Variant>()`
* `Result`, `Ok`, `Err`, and `range(a, b)` for `for (const i of range(0, n))`. Nothing else is global.
<!-- oxs-core:end -->

## Where things live

* `main.oxs`: the program. `oxi.toml`: name, version, `needs` (the host interfaces `main` receives, in parameter order; `oxi check` enforces it). `idl/*.idl`: host interfaces declared by this project (placeholders until the real capabilities ship).
* `oxi explain <code>`: every diagnostic code. Language reference: `docs/oxiscript/grammar.md` in the OxiScript repository (section 6 is the list of differences above, with reasons).
* `oxi run` and `oxi build` are not available until the compiler back end lands; `oxi check` is the gate today.
