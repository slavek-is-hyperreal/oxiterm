# OxiIR specification

Status: contract of slice 8.C (2026-11-24). Rust types: `oxiscript/oxiscript-ir/src/types.rs`; printer/parser: `print.rs`, `parse.rs`; verifier: `verify.rs`; builder: `build.rs`. Test programs (the canonical text): `oxiscript/oxiscript-ir/tests/programs/*.oxir`.

OxiIR is the typed SSA intermediate representation between the checked AST (slice 8.L lowers into it), the RC pass (8.M: elision), the placement/optimisation passes (later) and the Wasm backend (8.G). It is *monomorphised* (no generics), *explicit about reference counting* (retain/release are instructions, so elision is an IR pass), and *has no undefined behaviour*: every operation either has a defined result or traps with a [TrapKind](../../oxiscript/oxiscript-abi/src/trap.rs).

Pipeline contract: producers emit **verified** IR (`oxiscript_ir::verify` returns no errors); consumers may rely on it. The text form is the stable debugging/testing/interchange format (`parse(print(m)) == m`).

---

## 1. Structure

```
Module   = name, attrs, types, imports, data, globals, funcs
TypeDecl = struct { fields } | closure { captures } | union { Variant { fields }, ... }
Import   = @name = "<wasm module>" "<field>" Sig            (* typed boundary: params scalar | str | [u8] | fn; results flat scalars *)
Data     = @name = "utf-8 text"                             (* immortal string constant *)
Global   = @name : Ty [= scalar const]                      (* module-local mutable state *)
Func     = [export] @name(%param: [own] Ty, ...) -> rets #attrs { Block+ }
Block    = label(%param: Ty, ...): Inst* Terminator         (* block parameters replace phis *)
```

* **Types** (`Ty`): `bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64` scalars; `ptr` raw wasm32 address (trusted runtime-support modules only: `VerifyOptions::allow_raw_memory`); `str` UTF-8 string object; `ref Name` reference to a declared struct/closure-env/union; `[T]` array object; `fn(P...) -> R` closure value (at most one result); `T?` nullable form of `str`, `ref`, `[T]` or `fn` (null = address 0, the only nullable form: nullable scalars are boxed by the lowering). `void` appears only as "no results". Structs are heap objects (copy-on-write for value semantics, see section 4); there are no first-class aggregates, tuples or inline structs in the MVP.
* **Symbols** (types, functions, imports, data, globals) are identified by name; imports and functions share the `@` namespace. Values (`%name`) and block labels are function-local.
* **Entry block** is the first block; it has no parameters; function parameters are values defined on entry.
* **Canonical value numbering:** `ValueId`s are assigned in textual order (function parameters, then per block its parameters followed by instruction results). The text parser and `build::FuncBuilder` produce this numbering; text round-trips exactly when it holds. **Verification requires canonical numbering** (a module-wide pre-pass rejects anything else), so every pass that inserts, removes or reorders value definitions or blocks must call `build::renumber(&mut func)` afterwards.
* **Forward references:** a value may be used textually before its definition (the printer emits blocks in any order; the parser pre-scans each function and numbers values by definition order); the verifier, not the parser, checks that the definition dominates the use. Block labels may also be forward references.
* **Source locations:** every instruction and terminator may carry `!file:lo-hi` (file id and byte range from the AST span). Lowering must carry spans so traps, reports and reason codes can point at source (the placement report depends on it).
* **Attributes** `#name` / `#name(arg, "arg")` on modules, types and functions are open-ended metadata; unknown attributes must be preserved and ignored by passes. Registered names: module `#profile(default|realtime|safety|kernel)`, `#edition(YYYY)`, `#raw_memory` (declares raw use; it is **not** a grant: the toolchain must verify the module with `VerifyOptions::allow_raw_memory`, and an untrusted verification rejects both the attribute and every `ptr`/`load`/`store`/`intrinsic`); type `#acyclic` / `#cyclic` (cycle-collector colour hint, maps to `TYPE_FLAG_ACYCLIC`); function `#pure`, `#inline`, `#noinline`, and the **effects slot** `#effects(alloc, panic, loops, recursion, host:log, ...)` (arguments are identifiers, optionally joined by single colons, or quoted strings; absent = unknown/any). **Current implementation status:** `oxiscript-check`'s `EffectSummary` currently computes `uses_host`, `reads_globals`, `writes_globals`, `mutates_heap` and `calls_unknown` (used for `law` purity); computing `alloc`, `panic`, `loops` and `recursion` and enforcing profiles/`@expect(no_alloc)` wait for stage `8.2c` / `8.S4b`. `#expect(...)` mirrors source assertions; `#export_name(x)` renames a `@host` export.

## 2. Text form grammar

```
File      = "oxiir" INT Module-header { Decl } ;
Module-header = "module" STRING { Attr } ;
Decl      = TypeDecl | Import | Data | Global | Func ;
TypeDecl  = "type" Ident { Attr } "=" ( "struct" Fields | "closure" Fields
          | "union" "{" Variant { "," Variant } "}" ) ;
Fields    = "{" [ Ident ":" Ty { "," Ident ":" Ty } ] "}" ;
Variant   = Ident [ Fields ] ;
Import    = "import" "@"name "=" STRING STRING Sig ;
Data      = "data" "@"name "=" STRING ;
Global    = "global" "@"name ":" Ty [ "=" Const ] ;
Func      = [ "export" ] "func" "@"name "(" [ Param { "," Param } ] ")" "->" Rets { Attr } "{" Block { Block } "}" ;
Param     = "%"name ":" [ "own" ] Ty ;
Sig       = "(" [ Ty { "," Ty } ] ")" "->" Rets ;
Rets      = "void" | Ty | "(" Ty "," Ty { "," Ty } ")" ;
Ty        = "bool"|"i8"|...|"f64"|"ptr"|"str" | "ref" Ident | "[" Ty "]" | "fn" Sig | "(" Ty ")" | Ty "?" ;
Block     = label [ "(" "%"name ":" Ty { "," ... } ")" ] ":" { Inst } Terminator ;
Inst      = [ Result { "," Result } "=" ] Op [ Loc ] ;       Result = "%"name ":" Ty ;
Terminator = "br" Target | "br_if" Val "," Target "," Target
           | "switch" Val "," "[" [ INT ":" Target { "," INT ":" Target } ] "]" "," "default" Target
           | "ret" [ Val { "," Val } ] | "trap" TrapName ;      (* each may be followed by Loc *)
Target    = label "(" [ Val { "," Val } ] ")" ;
Const     = INT | FLOAT | "true" | "false" | "inf" | "-inf" | "nan:" HEXINT ;
Loc       = "!" INT ":" INT "-" INT ;
Comment   = ";" to end of line (ignored, not printed back)
```
Floats print with Rust's shortest round-trip decimal (`-0.0`, `1e21`, `5e-324`); infinities as `inf`/`-inf`; NaN as `nan:0x<bits>` so every payload survives. Strings use `\" \\ \n \r \t \u{..}` escapes. Result types are written on every result (the parser needs no inference); the verifier checks them.

## 3. Instruction reference

`ovf` suffix: none = **trap**, `.wrap`, `.sat` (only where listed). Operand and result types must match exactly (no implicit conversions).

| Instruction | Operands -> result | Defined behaviour |
|---|---|---|
| `const c` | -> scalar | Constant must fit the result type (`i128` checked). |
| `null` | -> `T?` | Null reference. |
| `add sub mul` [.wrap/.sat] | `T,T -> T` (ints, floats; `ptr` for add/sub with .wrap) | Trap mode: overflow traps `overflow`; `.wrap` two's complement; `.sat` clamps (ints only). Floats: IEEE-754, round-to-nearest-even, **never fused** (no FMA contraction). |
| `div` [.wrap] | `T,T -> T` | int: zero divisor traps `div_by_zero`; `MIN / -1` traps `overflow` (`.wrap`: `MIN`); rounds toward zero. float: IEEE. |
| `rem` [.wrap] | int only | zero traps `div_by_zero`; `MIN % -1` traps `overflow` (`.wrap`: 0); sign of the dividend. |
| `and or xor` | int, bool | bitwise / logical, no overflow. |
| `shl shr` [.wrap] | `T,T -> T` (shift amount has the operand type) | amount `>= bits` or negative traps `bounds`; `.wrap` masks to `bits - 1`; `shr` is arithmetic on signed, logical on unsigned. |
| `min max` | float | any NaN operand gives the canonical NaN; `-0.0 < +0.0`. |
| `neg` [.wrap] | numeric | int: `neg MIN` traps `overflow` (`.wrap`: `MIN`). float: sign flip. |
| `not` | int, bool | bitwise / logical. |
| `abs sqrt floor ceil trunc nearest` | float | IEEE (`nearest` ties-to-even). |
| `clz ctz popcnt` | int | defined for 0 (`bits`, `bits`, 0). |
| `eq ne lt le gt ge` | `T,T -> bool` | floats IEEE (NaN unordered: only `ne` is true); `eq/ne` also on bool, ptr and managed refs (identity); ordering on numerics and `ptr`. |
| `conv` [.wrap/.sat] | numeric `A -> B`, `A != B` | Trap mode: int->int out of range, float->int NaN/out of range after truncation toward zero: traps `overflow`. int->float and f64->f32 round to nearest even, never trap. `.wrap` (int->int): two's complement truncate/extend. `.sat` (to int): clamp, NaN -> 0. |
| `bitcast` | same-width int <-> float, `ptr` <-> `i32/u32` | Reinterprets bits; a NaN is canonicalised first (section 6). The `ptr` forms exist only in trusted raw-memory modules; `ptr` <-> `f32` is never allowed. |
| `select c, a, b` | `bool,T,T -> T` (scalars only) | Both operands evaluated. |
| `alloc T` / `alloc U::V` | -> `ref T` | Zeroed object, rc 1, owned. Managed fields start null and must be initialised before first read. Allocating a union variant fixes its tag. Traps `out_of_memory`. |
| `getfield T.f o` | `ref T -> field type` | Result is **borrowed** from `o` (valid while `o` is alive and the field is not overwritten). For variants: `U::V.f`. Reading a field of the wrong variant is a producer bug; checked code switches on `tag` first. |
| `setfield[.init] T.f o, v` | | **Consumes `v`**; for a managed field **releases the previous value**; `.init` skips that release (field known null: fresh object). |
| `tag u` | `ref Union -> u32` | Variant index. |
| `is_unique o` | managed -> bool | `rc == 1` and not sticky: the copy-on-write test. |
| `clone o` | `ref T` / `[T]` -> same | Shallow copy: new owned object, every child reference retained. |
| `array.new n` | `i32 -> [T]` | `n < 0` traps `bounds`. Elements zero/null. Owned. |
| `array.len a` | -> `i32` | |
| `array.get a, i` | -> `T` | Bounds-checked (`bounds`). Result **borrowed** from `a`. |
| `array.set a, i, v` | | Bounds-checked; **consumes `v`**; releases the old element. |
| `array.push a, v` | | **Consumes `v`**; grows amortised; may reallocate the buffer, never the object; traps `out_of_memory`. |
| `str.const @d` | -> `str` | Immortal object (sticky rc); result **borrowed**. |
| `str.len s` | -> `i32` | Byte length. |
| `str.from_utf8 b, n` | `[u8], i32 -> str` | New owned string from the first `n` bytes of `b`; traps `bounds` if `n` is negative or larger than the array, `bad_utf8` if the bytes are invalid. Used after a host wrote into an out-buffer. |
| `str.concat a, b` | `str, str -> str` | New owned string `a ++ b` (both operands borrowed). Traps `out_of_memory`. |
| `str.eq a, b` | `str, str -> bool` | Bytewise equality (both borrowed). Cannot trap. |
| `str.cmp a, b` | `str, str -> i32` | Bytewise order, which for valid UTF-8 equals code point order; result is exactly `-1`, `0` or `1`. Both borrowed. Cannot trap. |
| `str.slice s, lo, hi` | `str, i32, i32 -> str` | New owned string of the **byte** range `lo..hi`. Traps `bounds` if `lo < 0`, `hi > len` or `lo > hi`, and `bad_utf8` if `lo` or `hi` lies inside a code point (so the result is always valid UTF-8). `s` borrowed. |
| `str.byte s, i` | `str, i32 -> u8` | Byte `i` of the string; traps `bounds` unless `0 <= i < len`. Pure scalar result, no ownership effect. |
| `str.from_f64 x` | `f64 -> str` | New owned string with the shortest round-trip decimal text (`NaN`, `Infinity`, `-Infinity`; NaN payload and sign unobservable). Needs a runtime that implements `oxi_str_from_f64`: the 8.G Stub runtime traps `unreachable`; `oxiscript-rc` (8.M) implements `oxi_str_from_f64` (default feature `float-fmt`), linked into programs by stage `8.M-link` (`gemini-rtlink`). |
| `array.empty` | `-> [T]` | New owned array of length 0 for any element type, **including non-null references** (unlike `array.new`, which only exists for scalar and nullable elements and fills with zeros). Cannot trap except `out_of_memory`. |
| `array.pop a` | `[T] -> T` | Removes and returns the last element; traps `bounds` on an empty array. The result is **owned**: the array's reference moves out to the result (no retain, no release of the element; the array's length drops by one). Unlike `array.get`, the result is not a borrow from `a`. |
| `is_null x` | `T? -> bool` | |
| `opt x` / `unopt x` | `T -> T?` / `T? -> T` | `unopt` traps `null_deref` on null. Both are pure re-typings: they **move the owned count** to the result (section 4). |
| `closure.new E @f(caps...)` | -> `fn(...) -> R` | Allocates the closure object for env type `E` calling `@f`, whose first parameter must be `ref E`, remaining parameters/results define the closure's `fn` type; `@f` has no `own` parameters. **Consumes the captures**, in field order. Owned. |
| `call @f(args)` | -> results | Callee is a function or an import. Arguments are **borrowed** unless the parameter is `own` (then consumed). For an **import**, `str` and `[u8]` arguments are borrowed and a `fn` argument is **consumed** (the host keeps the closure, `abi.md` 5.4). Results are **owned**. Multi-value results bind several results. A value passed to an `own` parameter and also to another parameter of the same call is only valid if its count was at least 2 beforehand (retain first). |
| `call_closure c(args)` | -> result | `c` and the arguments are borrowed; the closure object is passed as the callee's env. Result owned. |
| `intrinsic oxi_name(args)` | ABI-defined | Calls a runtime-support export by name with its raw Wasm signature; trusted raw-memory modules only (`VerifyOptions::allow_raw_memory`). |
| `retain x` / `release x` | managed | +1 / -1 on the count (null is a no-op; sticky counts are unchanged). `release` at 0 frees and releases children. |
| `global.get @g` | -> `T` | Result borrowed from the global. |
| `global.set @g, v` | | Consumes `v`, releases the previous managed value. |
| `load p, off` / `store p, off, v` | `ptr` | Raw memory access at `p + off`; trusted raw-memory modules only; out-of-bounds traps via Wasm (`bounds` from the engine). |

Terminators: `br`, `br_if` (bool), `switch` (integer scrutinee, distinct cases, mandatory `default`), `ret` (consumes its operands; count and types must equal the function's results), `trap kind` (kinds: `unreachable overflow div_by_zero bounds bad_cast null_deref out_of_memory assert bad_utf8 panic`).

**Traps** are the only abnormal control flow: execution of the instance stops, `oxi_trap_kind` holds the kind (see `abi.md`), the host decides what to do. There are no exceptions, no unwinding, no `catch`. A trap does not run destructors: state is discarded with the instance. Anything that can trap is reported by `Op::may_trap()`.

## 4. Ownership and reference counting (contract for 8.L, 8.M and 8.G)

Every SSA value of managed type carries a **static owned count** `c`, a non-negative integer meaning "how many references this value is responsible for releasing". The RC-balance checker (slice 8.M) verifies the rules below; the verifier in this crate does not.

**Initial counts**
| Value | `c` |
|---|---|
| owned function parameter (`own`) | 1 |
| borrowed function parameter | 0 |
| block parameter of managed type | 1 (arguments passed to it are consumed) |
| result of `alloc`, `clone`, `array.new`, `array.empty`, `array.pop`, `str.from_utf8`, `str.concat`, `str.slice`, `str.from_f64`, `closure.new`, `call`, `call_closure` | 1 |
| result of `getfield`, `array.get`, `global.get`, `str.const` | 0 (borrowed) |
| result of `opt` / `unopt` | the operand's count, which moves (operand count becomes 0) |

**Effects**
| Operation | Effect |
|---|---|
| `str.concat`, `str.eq`, `str.cmp`, `str.slice`, `str.byte` | operands are borrowed (no effect on `c`); `str.concat`/`str.slice` results are owned (see the initial counts) |
| `array.pop a` | the array operand is borrowed; the removed element's reference moves out as the owned result |
| `retain x` | `c(x) += 1` |
| `release x` | requires `c(x) >= 1`; `c(x) -= 1` |
| consuming uses: `ret`, branch arguments, `own` call arguments, **`fn` arguments of host imports** (the host keeps the closure), `setfield`/`array.set`/`array.push`/`global.set` value operand, `closure.new` captures | each requires `c >= 1` on the operand and does `c -= 1` (passing the same value twice needs `c >= 2`) |
| all other uses (`getfield` object, `call` borrowed arguments, `call_closure`, `tag`, `is_unique`, `array.len`, ...) | no effect on `c`; the value must be alive: `c >= 1`, **or** it is borrowed from something that is alive (parameters, container fields) |

**Balance rule:** on every path to a `ret`, after the `ret` consumes its operands, every owned count is 0 (`trap` paths are exempt). At a join the counts of each live value must agree on all incoming edges. A borrowed value (`c = 0`) must not be used after the container it was borrowed from is mutated or released, or after a `setfield` that overwrites the field it came from; to keep it, `retain` it (the elision pass removes the retain when provably unnecessary).

**Unprotected borrows.** A borrow derived from a *load of mutable state* - `global.get`, `getfield`, `array.get`, or a closure's own environment - is protected only until the next operation that can run arbitrary code or overwrite state: any `call`, `call_closure`, host call (an import), `setfield`, `array.set`, `array.push`, `global.set`, `release` of a possible alias, or `clone`. Such a borrow must be `retain`ed *before* any of those operations if it is used after them (otherwise the callee or the store may drop the last reference and the later use is a use-after-free). Borrows from **parameters** and from **immutable values the callee owns** stay valid for the whole call, because the caller keeps them alive.

**Elision (8.M)** deletes `retain x` ... `release x` pairs only when **no** `call`, `call_closure`, host call, `setfield`, `array.set`, `array.push`, `global.set` or `clone` lies between them, **or** escape analysis proves that nothing reachable by those operations can alias `x` (any call may drop a reference reachable through a global, field, array element or closure environment, so calls are not transparent by default). It also converts owned-to-borrowed handoffs and may turn `clone` into in-place mutation when `is_unique` is proven. It must preserve the balance rule. `setfield`'s implicit release of the old value is *part of the instruction*, so `x.f = y` is one instruction, not a three-instruction sequence.

**Initialisation and defined behaviour (no uninitialised managed value is observable).** `alloc` zero-fills, so a non-nullable managed field would hold null; the verifier therefore requires, in the same block as the `alloc`, a `setfield.init` of every non-nullable managed field of the allocated type or variant before the object is used for anything else (`setfield.init` is accepted only on an object allocated earlier in the same block). `array.new` is allowed only for scalar or nullable element types (arrays of non-null references are built with `array.push`), and a managed global must be nullable (it starts null). Constructors lowered by 8.L must obey the same rule: no use of `this` before every non-nullable managed field is assigned. Reading a field of a union variant whose tag does not match is **defined**: the access traps `bad_cast` (the backend emits the tag test, and elides it when a dominating `switch` on `tag` proves the variant), so the "no undefined behaviour" claim has no hole here.



**Typed host boundary.** Untrusted IR never contains integer addresses: host imports take `str`, `[u8]`, scalars and consumed `fn` closures. Plain `verify` only checks that parameter *kinds* are legal; the exact signature of each import is fixed by the host IDL (`host::import_sig`), and the runtime (8.R) and code generator (8.G) **must run `host::verify_imports` / `verify_with_interfaces` before linking**, which rejects an import that names an ungranted module (`OXS-E0400`), an unknown function (`OXS-E0401`), or declares the flattened integer shape instead of the typed one.

**Value semantics for `struct`** (decision 1/principle 2): the lowering implements `struct` assignment as sharing (`retain`) and any mutation as `is_unique` ? in place : `clone` first (copy-on-write); see `structs.oxir` (`@move_x`). A later scalar-replacement pass may unbox small structs; nothing in the IR prevents it.

**Mutating struct methods.** Because struct mutation may `clone` (copy-on-write), a `mutating` method cannot update its receiver in place for the caller. Convention for 8.L: a method declared `mutating` is lowered to a function `(own %this: ref S, args...) -> (ref S, results...)` whose first result is the (possibly cloned) updated receiver, and the caller stores that result back into the place it called the method on (variable, field or array element; the borrowed receiver reference passed to the callee is taken with `retain` first so the callee owns one). `@move_x` in `structs.oxir` has exactly this shape. Non-mutating methods take the receiver borrowed and return only their results. Classes (reference types) never need this: the method mutates the shared object. `===` on a struct is rejected by the checker (`grammar.md` 3.3), so the IR needs no struct equality instruction; field-wise comparisons are ordinary `getfield`/`cmp`.

**Cycles:** the cycle collector (8.M) traces through the type descriptors of `abi.md`; the `#acyclic` type attribute lets the compiler mark types that can never form a cycle (they are allocated green and never traced).

## 5. Verifier (what `verify` guarantees)

Never panics on any input (a module-wide pre-pass checks every id, block target and the canonical numbering before anything else runs); types nest at most 64 levels (the parser enforces the same limit, and `Ty` drop stays recursive: programmatic construction is trusted, parsed input is bounded); dominators are computed with Cooper-Harvey-Kennedy over reverse postorder (linear memory), so 30,000-block functions verify in milliseconds. Unique symbol/type/value/label names (and names that print and re-parse: `[A-Za-z0-9_.]` for values and symbols, identifiers for labels and types, dot-free field and variant names; a type name may contain dots because `Type.field` splits at the last dot); all referenced types, symbols and data exist; type validity (`Opt` only on managed references, closures have at most one result, imports use `host:<name>@<N>` modules and the typed boundary; `ptr` and raw memory only under `VerifyOptions::allow_raw_memory`; `own` only on managed types; `#acyclic` is checked (no self-reachable or closure-capable field graph, no `#cyclic` conflict); export names are unique and not ABI-reserved; switch keys fit the scrutinee type; `is_unique` needs a non-null reference; every ptr bitcast is `ptr` <-> `i32`/`u32` and trusted); every value defined once and used only where its definition dominates the use (unreachable blocks excluded); block argument counts and types match block parameters; `ret` matches the function's results; operand/result typing for every instruction (including field and variant existence, closure env/function agreement, array element types, nullability); constants fit their types; raw memory ops and intrinsics only in `#raw_memory` modules with ABI-matching signatures; `closure.new` targets have no `own` parameters. It does **not** check RC balance, termination, or that a union field is read under the right tag.

## 6. Determinism

* **Integers:** all behaviour defined (section 3). Narrow integer values are canonical (sign/zero-extended in `i32` registers) after every operation; backends must re-canonicalise after `.wrap` arithmetic on `i8/i16/u8/u16`.
* **Floats:** IEEE-754 binary32/binary64, round-to-nearest-even, no fused multiply-add, no extended precision, no flush-to-zero. Core Wasm leaves NaN *payloads and signs* nondeterministic across engines. The IR rule: **the bits of a NaN are not observable to IR programs.** The only operations that could reveal them are `bitcast` (float -> int), which defines its result as the canonical quiet NaN (`0x7fc00000`, `0x7ff8000000000000`) when the operand is a NaN, and a float handed to a host call, which the backend canonicalises. Backends (8.G) implement this by canonicalising at those two points; under the `safety` profile they additionally canonicalise at every store of a float to memory and every `min`/`max`/`conv` that can produce a NaN, so linear-memory snapshots are bit-reproducible. `conv.sat` maps NaN to 0 and `conv` traps on NaN. Transcendental functions (`sin`, `pow`, ...) are *not* instructions: the standard library implements them in OxiScript (own libm) so results are identical on every target; `sqrt` is IEEE-exact.
* **Evaluation order:** operands of one instruction are values; the order of side effects is the textual order of instructions. The front end defines source evaluation order (left to right) and lowers accordingly.
* **Compiler output is reproducible:** printing a module and every pass iterating `Vec`s in order (no hash-order dependence) keep the compiler deterministic.

## 7. Extension points (summary; authoritative table in `contracts.md` section 3)

`Ty`, `Op` and `Terminator` are `#[non_exhaustive]`. For the full extension-point contract and stage ownership, see [contracts.md](contracts.md) section 3. Planned IR slots: vector types (`Ty::Vec{elem,lanes}`), device buffers (`Ty::Device([T])`), `#kernel` functions, `Op::ParFor`, `alloc.arena`, effect-polymorphic `#effects`, and `Terminator::Await`.
