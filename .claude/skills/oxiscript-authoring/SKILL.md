---
name: oxiscript-authoring
description: Write or edit OxiScript (.oxs) programs and fix their diagnostics. Use this skill whenever you touch a .oxs file, an oxi.toml or idl/*.idl file, are asked for an OxiScript program, library or OxiUI page, or see an OXS-E/OXS-W code. OxiScript looks like TypeScript but is stricter and different in ways that fail the checker (no `number`, no exceptions, `===` only, mandatory `;`, capabilities through `main`), so do not write it from TypeScript habits.
---

# Authoring OxiScript

OxiScript reads like TypeScript and is checked like Rust. A model that writes TypeScript gets about one error per ten lines. The `oxi` tool tells you exactly what is wrong, with machine-applicable fixes, so the way to be correct is the loop below, not memory.

## 1. The loop (do this after every edit)

```text
oxi check --json            # one JSON document, schema 1
oxi check --fix             # applies every machine-applicable fix, rewrites files
oxi fmt                     # canonical format, no options
oxi run [file] [--fuel N]   # compile and run main in the sandboxed wasmi runtime
oxi build [file] [-o out]   # compile to a .wasm module
oxi explain OXS-E0100       # meaning, example and fix for any code
```

1. Write or edit the `.oxs` file.
2. Run `oxi check --json <file or dir>`. Stop when `"ok": true`.
3. For each diagnostic: if a fix has `"applicability": "machine-applicable"`, apply its `edits` (or run `oxi check --fix`). `maybe-incorrect` fixes are proposals; read the title and decide. `advice` has no edit; follow the text.
4. Syntax errors hide type errors. When the syntax is clean a second wave appears. Repeat until clean.
5. Finish with `oxi fmt`. Never tell the user the code works until `oxi check` is clean; use `oxi run` to execute `main` and `oxi build` to emit Wasm.

Edit positions: `range.start.byte` / `range.end.byte` are UTF-8 byte offsets; `line` is 1-based, `col` counts characters, `utf16` counts UTF-16 units (what editors use).

## 2. Before you write: the differences that bite

Full list with reasons: `docs/oxiscript/grammar.md` section 6. The ones that fail first:

| You would write (TypeScript) | OxiScript | Code |
|---|---|---|
| `let n: number` | `i32` / `i64` / `u8` / `f64` ... (no `number`) | E0300 |
| `a == b` | `a === b` | E0005 (fixable) |
| a statement without `;` | `;` is mandatory | E0004 (fixable) |
| `throw` / `try` / `catch` | return `Result<T, E>`, propagate with `e?` | E0004 then more |
| `s.length`, `s[i]` | `s.byteLength`, `s.chars()`, `s.bytes()` | E0108 / E0128 |
| `const p = { x: 1 }` | needs a `struct` type from context | E0103 |
| `x: i32` used as `f64` | explicit `x as f64` (no implicit numeric mixing) | E0100 / E0110 |
| `a + b as f64`, `n / 10` for a float | `as` binds looser than `+`: `(a as f64) + (b as f64)`; convert both operands before `/` | E0110 / E0100 |
| `@host export function f<T>(...)` | a `@host` (host-called) function cannot be generic; plain `export` of a generic is fine | E0121 |
| `?` across different error types | `?` needs the same `E`; map it with a `switch` | E0104 |
| `class B extends A`, `any`, `var`, `undefined`, `for...in` | not in the language | E0900 / E0001 |
| `console.log`, `Math.*`, `fetch` | none exist; `main` receives capabilities (`log: Log`) | E0300 |
| `f() ?` (space) | `f()?` is try; ` ? a : b` is the conditional | E0006 |
| `a & b === c` | parenthesise: `(a & b) === c` | E0007 (fixable) |

Numbers: integer literals take the type their context needs (default `i32`), floats default to `f64`, suffixes fix it (`200u8`, `1.5f32`, never `1f32`). Overflow traps; `+%` wraps, `+|` saturates.

`struct` = value (copied; methods that assign `this.x` are `mutating`), `class` = reference (`new C()`), sum types are discriminated unions with a `kind` literal, and a `switch` over them must be exhaustive with no fall-through.

## 3. Shapes to copy

A whole program: capabilities come in through `main`, failures are `Result` values, `?` propagates.

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

A tagged union, exhaustive `switch`, and a law (a property the program must satisfy; the last expression has no `;`).

```oxs
type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };

export function area(s: Shape): f64 {
  switch (s.kind) {
    case "circle":
      return 3.14159 * s.r * s.r;
    case "rect":
      return s.w * s.h;
  }
}

law rect_area_commutes(w: f64, h: f64) {
  area({ kind: "rect", w: w, h: h }) === area({ kind: "rect", w: h, h: w })
}
```

A reference type with state, and a numeric conversion.

```oxs
class Counter {
  n: i32 = 0;
  inc(by: i32): void {
    this.n += by;
  }
}

export function percent(done: i32, total: i32): f64 {
  const c = new Counter();
  c.inc(done);
  return (c.n as f64) * 100.0 / (total as f64);
}
```

What the checker says for the commonest slip:

```oxs-err OXS-E0005
export function same(a: i32, b: i32): bool {
  return a == b;
}
```

## 4. Project layout and capabilities

* `oxi new <name> --template blink|touch-button|sensor-dashboard|oxiui-page` creates a project that already passes `oxi check`.
* `oxi.toml`: `[package]` name/version/edition and `needs = ["host:log@1", ...]`, the host interfaces `main` receives, in parameter order.
* Host interfaces are IDL files: built in are `host:log` and `host:state`; a project adds its own in `idl/*.idl` (the UI, GPIO and sensor ones in the templates are placeholders until their slices land). Never call a method that is not in an IDL.
* Built-in members: array `length push pop map filter reduce forEach slice some every concat`; string `byteLength bytes chars slice startsWith endsWith includes indexOf`; number `toString wrapping<T> saturating<T>`; numbers, `bool` and `string` also `hash() equals()`; union `downcast<T>`; `Result Ok Err`. Everything else comes from the standard library by import: `std/math`, `std/string`, `std/array`, `std/collections` (`Map`, `Set`), `std/result` (see `docs/oxiscript/std.md`).

## 5. What not to do

* Do not translate TypeScript idioms mechanically; rewrite them (exceptions to `Result`, object bags to `struct`, `number` to a concrete type).
* Do not add `// @ts-ignore`-style suppressions or unchecked casts: there are none (`@allow("OXS-Pxxxx", "reason")` exists only for placement notes and needs a reason).
* Do not claim a library exists because it exists in TypeScript. Check `idl/` and the built-in list above.
* Do not stop at "looks right": a clean `oxi check --json` is the definition of done.
