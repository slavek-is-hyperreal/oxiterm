# OxiScript known-attack corpus (input for BAD ACTOR regression tests)

Compiled 2026-11-25. Purpose: catalogue KNOWN, documented bug/attack classes so adversarial agents can turn them into regression tests for OxiScript slices 8.P (oxiscript-syntax), 8.T (oxiscript-check), 8.L (lowering), 8.G (oxiscript-wasm codegen), 8.M (oxiscript-rc), 8.R (oxiscript-runtime, wasmi).

## Verification legend

- **V** = primary source page was read in this session. (Honest status: no full primary page read succeeded; the one direct fetch failed.)
- **S** = confirmed by web-search result snippets from the named source (id and one-line mechanism seen), page not read in full.
- **R** = recalled from prior knowledge, not re-checked this session. Treat ids as "check before citing". Where an id is not given, the class is well documented but I would not vouch for a number.

Priority: **P0** = would break soundness/sandbox, run first; **P1** = likely bug class, run in the first wave; **P2** = hardening/regression.

Ground rule for all tests: OxiScript promises (a) a type-correct program never faults in linear memory, (b) a guest cannot exceed fuel/memory/depth limits or escape its capability set, (c) formatting and diagnostics never change or hide meaning. Every row's "test idea" is a negative test: the attack must be rejected at compile time, trap deterministically, or be impossible by construction.

---

## 1. TypeScript type-system unsoundness (target mainly 8.T, some 8.L/8.G)

Sources: TS FAQ wiki ("Why Method Bivariance?", covariance entries) [S]; TS design non-goals "not a sound or provably correct type system" in handbook [S]; TS issue #9825 "Type-checking unsoundness: standardize treatment" [S]; TS issue #62666 unsafe mutation through covariant arrays [S]; Effective TypeScript "The Seven Sources of Unsoundness in TypeScript" [S]; TS issue #9998 "Trade-offs in Control Flow Analysis" [R]; compiler flags `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `strictFunctionTypes` [R, well known].

OxiScript equivalents below are written in TS-like syntax; each should be a compile error (E) or a defined runtime trap (T) in OxiScript, never silent type confusion.

| # | Class | Source ids | Mechanism (TS) | Minimal TS hole | OxiScript test idea | Pri |
|---|---|---|---|---|---|---|
| 1.1 | Array covariance | FAQ [S], #62666 [S] | `Dog[]` assignable to `Animal[]`; writing a `Cat` through the alias corrupts the `Dog[]` view | `const d: Dog[]=[new Dog()]; const a: Animal[]=d; a.push(new Cat()); d[1].bark();` | Same program must be E (arrays invariant or write-through-supertype rejected). If read-only views are covariant, assert `ReadonlyArray<Dog>`->`ReadonlyArray<Animal>` OK and mutable one E. Also repeat for Map/Set/object fields/tuples | P0 |
| 1.2 | Method parameter bivariance | FAQ [S] | Methods (not function properties) are compared bivariantly, so a subtype method accepting a narrower arg is allowed | `interface A{f(x:Animal):void} const b:A={f(x:Dog){x.bark()}}; b.f(new Cat())` | E: method params contravariant always; test method shorthand, function-typed property, callbacks, generic interfaces, class `implements`/`extends` overrides, overloads | P0 |
| 1.3 | Unchecked `as` / double assertion | Effective TS [S] | `as` between unrelated types via `unknown`/`any`, or narrowing assertions lie | `const n = ("s" as unknown) as number; n.toFixed()` | Decide policy: either no `as` to non-supertype, or `as` lowered to a runtime-checked cast that traps with defined error. Test: downcast with failing check must trap, never reach field access; cast through `unknown` must check | P0 |
| 1.4 | `any` leaking | Effective TS [S] | `any` disables checking and propagates silently (JSON.parse, catch variable) | `const x:number = JSON.parse("\"s\")` | Language has no `any`; stdlib JSON returns a `Json` union that must be narrowed; `catch` binding typed `unknown`/`Error` only. Test: grep-like AST test that no builtin returns unconstrained top-that-assigns-to-everything | P0 |
| 1.5 | Index signatures omit `undefined` | `noUncheckedIndexedAccess` [R] | `Record<string,T>[k]` and `T[][i]` typed `T` though absent/OOB gives `undefined` | `const a:number[]=[]; a[5].toFixed()` | Index read of array/map returns `T | undefined` (or traps OOB) -- test OOB read, negative index, `NaN` index, 2^32, -0 index, map missing key; must never yield uninitialised memory | P0 |
| 1.6 | Optional props vs explicit `undefined` | `exactOptionalPropertyTypes` [R] | `{a?:number}` accepts `{a:undefined}`; `"a" in o` vs `o.a===undefined` diverge | `const o:{a?:number}={a:undefined}; "a" in o` | Specify one semantics; test that `{a?:T}` cannot hold `undefined` explicitly (or that presence and value are consistent in lowering), including spread `{...o}`, `Partial<T>`, and RC drop of the absent slot | P1 |
| 1.7 | Narrowing invalidated by callbacks/aliasing | #9998 [R] | Narrowing of a property/`let` persists across a function call that mutates it | `let x:string|number="a"; if(typeof x==="string"){ f(()=>{x=1}); x.length }` | E or re-widen: test narrowing of mutable captured variables, object props after any call, array elements, `this.field` across method calls, narrowing of `let` in loops with closures | P0 |
| 1.8 | Narrowing of mutable aliasing (two refs to one object) | #9998 [R] | `if (o.k!==null){ other.k=null; o.k.x }` | as left | Same as 1.7 but with two references to same record; checker must either invalidate on any write through any alias or restrict narrowing to immutable paths | P0 |
| 1.9 | `this` typing | FAQ [R] | `this` unbound when method extracted; polymorphic `this` in callbacks; `this` param lies | `const f=obj.method; f()` | Test extracted methods (bound or E), `this` in closures, `this` in static contexts, class field initialisers reading `this` before init (uninitialised field read) | P1 |
| 1.10 | Class fields use-before-init | TS #...? [R] | Constructor calls overridable method reading uninitialised subclass field | `class A{constructor(){this.f()} f(){}} class B extends A{x=1; f(){this.x.toFixed()}}` | E, or fields have definite-init order guaranteed; test must prove no read of an unset slot (important for RC: uninit slot must not be dropped) | P0 |
| 1.11 | Numeric enum assignability | handbook [R] (TS 5.0 tightened) | Any `number` assignable to numeric enum; reverse mapping; out-of-range values | `enum E{A,B}; const e:E=42; switch(e){case E.A:...case E.B:...}` exhaustive-but-wrong | Enums are closed sums; `number->enum` only via checked `fromInt` returning option. Test exhaustive `match` + codegen default arm unreachable-trap | P1 |
| 1.12 | Overload resolution | handbook [R] | Implementation signature not checked against overloads; first-match picks surprising overload | `function f(x:string):string; function f(x:number):number; function f(x:any):any{return 1}` | Test overload set vs implementation consistency (E when impl returns wrong type), ordering ambiguity, overloads + generics + unions of args | P1 |
| 1.13 | Generic constraint holes | #9825 [S], R | Constraint on `T extends {a:string}` allows subtype with `a: "x"` written back; `T[K]` writes; conditional-type distribution | `function set<T extends {a:string}>(o:T){ o.a="x" } set<{a:"lit"}>(...)` | Writes through constrained generic must require invariance in T; test `T[K]` assignment (union K writes need intersection), higher-order generic inference, recursion in constraint (checker termination) | P0 |
| 1.14 | `readonly` bypass via aliasing | FAQ [R] | `readonly` is a view modifier; mutable alias of the same object mutates it | `const m:{x:number}={x:1}; const r:Readonly<{x:number}>=m; m.x=2; r.x` | Decide: readonly is advisory (document) or enforced via uniqueness. Test the stated semantics; mutation of a readonly-typed value through an alias must be either E or documented-visible; critically readonly must never be assumed by codegen/optimiser for CSE/hoisting | P1 |
| 1.15 | Structural vs nominal confusion | handbook [R] | Class with private fields is nominal-ish, else structural; branded types forgeable | `type USD=number&{__b:1}; const x=5 as USD` | If opaque/branded types are offered, test forging impossible outside defining module; test capability handles (8.R) are nominal and unforgeable from numbers | P0 |
| 1.16 | Function parameter arity/excess-property/freshness | handbook [R] | Fewer-param function assignable; excess property check only on fresh literals | `const f:(a:number,b:number)=>void=(a)=>{}`; `const o={a:1,b:2}; const p:{a:number}=o` | Arity subtyping is sound (OK) -- test it is still sound with rest/optional/default params; width subtyping with RC: dropping extra fields must still free them (layout must not be prefix-assumed wrongly) | P1 |
| 1.17 | Union/intersection weirdness (`string & number` = never, `{a:1}&{a:2}`) | R | Intersection of conflicting props not reduced to never in some cases; `never` assignable everywhere | `declare const n:never; const s:string=n` | Test `never` only inhabitable by unreachable code and that codegen emits `unreachable` trap; discriminated-union exhaustiveness | P1 |
| 1.18 | Tuple/rest/spread length | R | `[...T]` with `T extends any[]` length unknown; `args.length` mismatch; variadic holes | `function f(...a:[number,string]){}; f(...(x as number[]))` | Spread of dynamically-sized array into fixed-arity call must be E or runtime-checked; fuzz arities 0..N and huge N | P1 |
| 1.19 | Type-checker DoS (exponential/infinite instantiation) | TS errors "Type instantiation is excessively deep" [R] | Recursive conditional/mapped types, huge unions, circular constraints | `type R<T>=T extends any?R<T[]>:never` | Checker must carry depth/step budget and report a diagnostic, never hang or overflow stack; test 10^5-member union, 10^3-deep nesting | P0 |
| 1.20 | Variance of `unknown`/`void` return, `Object`/`{}` | R | `void`-returning callbacks accept any returning fn; `{}` accepts primitives | `const f:()=>void=()=>5` | Test callbacks that return values where `void` expected: returned RC value must be dropped (leak test in 8.M) | P2 |
| 1.21 | Definite assignment / TDZ | R | `let x:number; use(x)` after closure assignment; `var` hoisting | `let x:number; function g(){return x} g()` | E on read-before-assign incl. via closures; test captured-before-init locals, recursive `const` initialisers (uninit slot read through closure) | P0 |
| 1.22 | Bigint/number mixing, NaN equality | R | `NaN!==NaN`, `0===-0`, `Object.is` differ; `in`/`includes` use different equality | `[NaN].indexOf(NaN)` vs `.includes(NaN)` | Fix equality semantics in spec (see section 2 -0/NaN); property test that `==` is consistent with hash/Map keys | P1 |

---

## 2. JavaScript engine CVEs (analogous bugs in 8.L / 8.G / 8.M)

OxiScript has no JIT, but its optimiser/lowering/codegen has the same *logical* risks (range analysis, bounds check elision, side-effect modelling, CSE, inlining). RC runtime has the UAF/resize classes. Sources cited by search only unless noted.

| # | Class | Representative ids | Mechanism (one line) | Target slice | OxiScript test idea | Pri |
|---|---|---|---|---|---|---|
| 2.1 | Incorrect side-effect modelling in optimiser | CVE-2020-6418 (V8 TurboFan, Project Zero RCA) [S]; CVE-2023-3420 (V8, GitHub Security Lab write-up) [S]; CVE-2018-17463 (V8, Phrack "Exploiting Logic Bugs in JIT Engines") [S/R] | Optimiser believes an op (e.g. inlined `Array.pop`, property load) has no side effects, so a prior check (map/shape/length) is reused after the object changed | 8.L, 8.G (any CSE/LICM/inlining) | Differential test optimised vs unoptimised IR: calls that mutate array length/shape/kind between a bounds check and use; getter/closure/callback that resizes array while loop holds cached length; check eliminated only if no intervening call/store/alloc/RC-drop (drop can run destructor/finaliser) | P0 |
| 2.2 | Bounds-check elimination from wrong range/type inference | P0 "In-the-wild series: Chrome infinity bug" (typer induction-variable bug) [S]; Phrack JIT logic bugs [S] | Typer infers a loop var range too narrow (e.g. missing Infinity/-0/overflow), so index check is removed and OOB access is possible | 8.G, 8.L | Programs whose loop induction vars hit i32 wrap, `i+=step` with huge step, `i<=len` off-by-one, negative start, `len` modified in body; assert every array access either keeps a check or has a proven, machine-checked-in-test range. Fuzz: compile with checks forced-on vs elision-on, compare traps | P0 |
| 2.3 | Integer overflow in length/size computation | Hermes CVE-2022-32234 (OOB write handling large arrays) [S]; Hermes CVE-2022-35289 (write-what-where via integer overflow) [S]; QuickJS CVE-2025-62496 (BigInt conversion size overflow) [S]; QuickJS CVE-2025-62495 (regex bytecode size overflow) [S] | `count*elem_size`, `len+1`, `cap*2`, bit-count formulas overflow 32-bit then allocate too small, then write full length | 8.M, 8.G, 8.R | Allocate/concat/repeat/`new Array(n)` with n near 2^31, 2^32, 2^32+small, element sizes 8/16; string repeat/concat doubling; wasm32 address overflow (offset+base as i32 wrap). Must trap with OOM/limit error, never allocate undersized | P0 |
| 2.4 | Array growth/resize while iterating (callback re-entrancy) | QuickJS CVE-2025-62490 (print loop caches array length, array shrinks -> UAF) [S]; quickjs-ng CVE-2026-1144 (`ArrayBuffer.resize(0)` in `valueOf` -> stale pointer in Atomics) [S]; Hermes `Array.prototype.sort` comparator adds elements during sort (OOB read, Meta write-up 2022) [S] | Native code caches (ptr,len) across a call-out into user code that can mutate the container | 8.M (runtime container ops), stdlib, 8.G (inlined loops) | Stdlib higher-order fns (`map/forEach/sort/filter/reduce/find`) with callback that `push`es, `pop`s, clears, or reassigns the array being iterated; sort comparator that mutates; `for..of` over array mutated in body; Map/Set mutation during iteration. Must be defined (snapshot or checked) and RC-safe under ASan-style poison | P0 |
| 2.5 | Use-after-free from RC/GC mismanagement across async/closure | QuickJS CVE-2023-48184 (`JS_FreeValueRT`, async fn with closures) [S]; QuickJS CVE-2025-62491 (iterating rejected-promise list after free) [S] | Object freed while an alias in a closure/queue still iterates; drop order during teardown | 8.M, 8.R (hot reload) | Closure captures + drop during its own invocation (`f = ()=>{ f=null; use(captured) }`); self-referential closure cycle freed by cycle collector while on stack; drop of container whose element drop re-enters container; reload of module while old closures alive | P0 |
| 2.6 | Type confusion via element-kind / shape transitions | CVE-2020-6418 [S]; SpiderMonkey CVE-2019-9791 (IonMonkey type inference), CVE-2019-11707 (Array.pop type confusion) [R]; JSC DFG/FTL analogues (ids not verified) | Array changes representation (int<->double<->object) while optimised code assumes one | 8.G, 8.L | OxiScript is statically typed with monomorphic layout; test that generic instantiation (`T[]` with T=i32 vs ref) never shares code that assumes ref-ness: monomorphisation or tagged-layout test where an `i32` payload equal to a valid pointer value must not be traversed by drop/cycle collector | P0 |
| 2.7 | -0 / NaN / Infinity typing and equality | P0 infinity bug [S]; V8 typer `-0`/`Math.expm1`-class bugs (ids not verified) | Optimiser types a value as integer but it can be -0 or NaN; `Object.is` vs `===`; `x|0` normalisation | 8.G, 8.L, 8.T | Constant-fold vs runtime diff for f64 ops: `-0` through `/`, `%`, `*`, `Math.round/floor`, `min/max`, `f64->i32` convert (trapping vs saturating), `NaN` as Map key, sort comparator NaN, `0*-1`. Folding must match wasm runtime bit-for-bit (modulo NaN payload policy) | P1 |
| 2.8 | Wasm `div/rem/shift/convert` edge semantics mis-lowered | R (class; wasm spec) | `i32.div_s(INT_MIN,-1)` traps, `rem_s(INT_MIN,-1)`=0, shift counts mod width, `trunc` traps on NaN/out-of-range | 8.G, 8.L | Table test of all int/float ops at boundary values vs a reference interpreter; constant folder must reproduce trap behaviour (not fold to a value) | P0 |
| 2.9 | Regex engine bugs & ReDoS | QuickJS CVE-2025-62495 (regex bytecode size overflow) [S]; Cloudflare 2019 outage (backtracking) [R]; V8 irregexp backtrack limits [R] | Backtracking blowup; compile-time size overflow; lookbehind/backref memory | stdlib regex (if present), 8.R fuel | If regex ships: must be linear-time (RE2/Thompson) or fuel-metered; test `(a+)+$` on `"a"*40+"b"`, nested quantifiers, huge `{n,m}` counts, pattern 1MB. If no regex: record decision | P1 |
| 2.10 | Property-access type confusion in inline caches / prototype chain | many (V8 map-check bugs) [R] | Cached shape check stale after prototype mutation | 8.G/8.L | OxiScript has no dynamic shapes or prototypes; test proves field access lowers to fixed-offset loads from a statically known layout only, and that no runtime code can alter layout | P1 |
| 2.11 | String/Unicode length vs byte length | R | UTF-16 vs UTF-8 length, lone surrogates, normalisation, index by code unit vs code point | 8.G/8.M | Strings with lone surrogates (if UTF-16 modelled), 4-byte code points, NUL, 1MB; slicing mid-codepoint must not produce invalid UTF-8 in a type claiming valid | P1 |
| 2.12 | Stack overflow from deep recursion (guest and compiler) | many engines; V8 RangeError [R] | Unbounded recursion crashes host or corrupts | 8.R, 8.G, 8.P, 8.T | Guest infinite recursion -> deterministic trap (wasmi call-depth/stack limit), also mutual recursion through closures and through `drop` of a 10^6-long linked list (recursive free overflows native/wasm stack) | P0 |
| 2.13 | Typed-array / ArrayBuffer detach & resize semantics | CVE-2026-1144 [S], V8 detached-buffer class [R] | Backing store invalidated while view in use | 8.M/8.R if byte buffers exposed | Shared/transferred buffers across host call: view must be re-validated after any host call that can `memory.grow` (linear memory base may move: host holds stale pointer) | P0 |

---

## 3. WebAssembly runtime advisories (8.R primarily, 8.G for generated code)

OxiScript runs on wasmi (interpreter, so JIT/guard-page classes are weaker but logic classes apply). Ids from search snippets [S] unless noted.

| # | Class | Source ids | Mechanism | Target | OxiScript test idea | Pri |
|---|---|---|---|---|---|---|
| 3.1 | Runtime memory-safety bug in linear memory impl | wasmi CVE-2025-66627 / GHSA-g4v2-cjqp-rfmq, UAF in linear memory (published 2025-12) [S; exact mechanism and affected range not verified] | Memory object freed/reallocated while reference still held | 8.R | Pin wasmi to a version at or above the fix; add `cargo audit`/`cargo deny` gate in CI; test: memory grow/reset/drop on hot reload while old instance exports memory view | P0 |
| 3.2 | Guest-controlled OOB read/write via guard config | wasmtime CVE-2023-26489 / GHSA-ff4p-7xrq-q5r8 (x86_64 addressing, guard size) [S] | Address computation allowed access past guard region | 8.G (offset/addr emitted), 8.R | wasmi bounds-checks explicitly, but generated code: test `i32.load offset=0xFFFFFFFF` with base near 2^32, `memory.copy/fill/init` with len overflowing, zero-length at exact end/ end+1 (spec: trap iff dst>size even when len=0) | P0 |
| 3.3 | Runtime UB in instance/table/memory state mgmt | wasmtime CVE-2023-30624 (LLVM-level UB in per-instance state) [S] | Rust-level UB in embedder-internal structs | 8.R | Not directly testable; mitigation = dependency pin + cargo-audit gate; keep embedder `unsafe`-free (`#![forbid(unsafe_code)]` in oxiscript-runtime) | P1 |
| 3.4 | Panics/DoS on table/segment ops | wasmtime CVE-2024-30266 (`table.copy` from dropped elem segment on externref table; panic) [S] | Edge case of dropped segments | 8.G, 8.R | Emit tests with `table.copy/init/fill/grow` and `elem.drop`/`data.drop` then re-use; if OxiScript codegen emits tables for closures/vtables, assert it never uses dropped segments; host must contain a wasmi panic (catch_unwind boundary) | P1 |
| 3.5 | Tail calls + stack-trace/host import | wasmtime CVE-2024-47763 (`return_call*` to host fn capturing backtrace) [S] | Feature interaction | 8.G | Do not enable tail-call proposal (OxiScript targets Core Wasm); test: validator rejects `return_call` modules -- proves feature gating | P1 |
| 3.6 | Uninitialised/invalid drop of partial instance | wasmtime CVE-2022-23636 / GHSA-88xq-w8cq-xfg7 (partially-initialised instance drop, externref) [S] | Instantiation failure mid-way (OOM, trap in start) leaves invalid state dropped | 8.R | Instantiate with failing start function, memory limit smaller than module minimum, data segment OOB (spec: instantiation traps, partial writes to memory visible!) -- test no panic, no leaked host resources, hot-reload keeps old instance | P0 |
| 3.7 | Compiler (Winch/Cranelift) sandbox-escape logic bugs | RUSTSEC-2026-0095 Winch memory access (critical) [S]; Wasmtime April 2026 batch of 12 advisories [S] | Backend miscompile | 8.R (n/a for interpreter) | Informational: choosing an interpreter avoids this class. Record decision; if a JIT backend is ever enabled, re-open | P2 |
| 3.8 | Fuel/metering gaps | Wasmtime `hostcall-fuel` option (limits bytes copied per host call) [S]; class from engine docs [R] | Fuel counted per instruction but bulk ops (`memory.fill/copy`, `memory.grow`, table ops), host calls, and allocation are O(n) or free; or fuel not charged on loop back-edges in some tiers | 8.R | Guest: `memory.fill` of max memory in a loop; `memory.copy` self-overlap; host function that does O(n) work (string concat, JSON encode) with n from guest -- must be charged proportionally by host (`consume_fuel` per byte/element); loop with zero-cost body (`loop{}`), `br_table` loop; check fuel exhaustion reported as distinct error and instance poisoned (no resume into half-updated state unless designed) | P0 |
| 3.9 | Fuel exhaustion at arbitrary point corrupts runtime invariants | R (class) | Trap mid-update leaves RC counts/free list inconsistent; resuming or reading memory afterwards is UB-ish | 8.R, 8.M | Force trap at every fuel value 1..N during alloc/free/cycle-collect (fault injection sweep); after trap the instance must be discarded or the host must only read it through checked paths; hot reload must not migrate state from a trapped instance | P0 |
| 3.10 | Memory growth limits | wasm spec `memory.grow` returns -1; limiter/ResourceLimiter semantics [R] | Guest grows to host limit; host allocations scale with guest; max pages absent in module | 8.R, 8.M | `ResourceLimiter` with memory/table/instances caps; test grow to limit, `grow(0)`, grow past 4GiB (`-1`), allocator reaction to grow failure (must return OOM trap/Result not null deref), module declaring no max | P0 |
| 3.11 | Host-call reentrancy | R (embedder class; wasmtime docs on `Caller`) | Host fn called from guest calls back into guest (or another instance) while host holds a borrow of memory/store; stale `&[u8]` into memory after grow | 8.R | Host function invoking a guest callback that does `memory.grow`; host holding slice across it -> must re-fetch; recursion host->guest->host depth cap; capability revoked during a callback; same capability handle used reentrantly | P0 |
| 3.12 | Import/export type confusion at host boundary | R | Host func signature mismatch (wasmi checks at link), but host-side decoding of (ptr,len) pairs from guest is unchecked | 8.R | Every host API arg that is a (ptr,len) must be bounds-checked with overflow-safe math (`ptr.checked_add(len)`); test ptr=0xFFFFFFFF len=2; len=0 at end; unaligned; pointing at RC header; host returning pointer into guest memory that guest later frees | P0 |
| 3.13 | Validation bugs (accepting invalid modules) | historical engine validator bugs [R, ids not verified] | Stack-typing/`br_table`/unreachable-code validation holes | 8.G, 8.R | 8.G output must validate under `wasmparser`/wasm-tools and wasmi, with all optional features OFF (no GC/threads/EH/tail-call/SIMD/ref-types as chosen); fuzz: mutate emitted bytes, runtime must reject or trap, never crash | P1 |
| 3.14 | NaN nondeterminism / float canonicalisation | wasm spec nondeterminism [R] | NaN payload bits vary across engines; breaks hot-reload/replay determinism and hashing | 8.G, 8.R | Programs that hash/serialise f64 bits of NaN results; cross-check wasmi vs second interpreter; canonicalise at host boundary | P2 |
| 3.15 | Indirect call signature check / table OOB | wasm spec traps; engine bugs [R] | `call_indirect` type check or index OOB mishandled; null entry | 8.G | Closure/vtable calls: index > table size, null slot, type mismatch -> trap; fuzz table contents after hot reload (stale table indices from old module version) | P0 |
| 3.16 | WAMR / Wasmer advisories | WAMR and Wasmer CVE lists exist (OOB in interpreter/loader, validation) [R; ids not collected] | Loader/validator memory-safety | 8.R | Not pursued: wasmi chosen. TODO: next session pull WAMR/Wasmer ids from GHSA if comparing runtimes | P2 |

---

## 4. Parser/tooling attacks and language-level classes (8.P, 8.T, 8.L, 8.R diagnostics)

| # | Class | Source ids | Mechanism | Target | OxiScript test idea | Pri |
|---|---|---|---|---|---|---|
| 4.1 | Bidi control characters (Trojan Source) | CVE-2021-42574; CERT/CC VU#999008; Red Hat RHSB-2021-007 [S] | RLO/LRI/RLI/PDI etc. inside comments/strings reorder display so reviewed text differs from tokenised text | 8.P (lexer), formatter | Lexer rejects (error, not warning) U+202A-202E, U+2066-2069 anywhere in source incl. comments and string literals, unless written as escapes; test each code point in comment, string, template, identifier, and via `\u{202E}` escape (escape allowed, raw not) | P0 |
| 4.2 | Homoglyph identifiers | CVE-2021-42694 [S] | Cyrillic/Greek lookalike letters define distinct identifiers | 8.P, 8.T | Identifiers restricted to UAX#31 with NFC normalisation and a confusable policy (UTS#39: reject mixed-script identifiers, or ASCII-only in v1); test `Н` (U+041D) vs `H`, `ａ` fullwidth, combining marks, NFC vs NFD same name resolving to same symbol, zero-width joiner/ZWSP/invisible chars (U+200B-200F, U+2060, U+FEFF mid-file, U+00AD) | P0 |
| 4.3 | Invisible/unprintable characters, BOM, NUL, line separators | R (JS parsers: U+2028/2029) | Hidden chars alter tokens or line counts; CRLF/LF/CR/LS/PS disagreement between parser, formatter, diagnostics | 8.P | Source with NUL, lone CR, U+2028, vertical tab, invalid UTF-8 bytes, overlong encodings, BOM in middle: each rejected with a stable diagnostic; line/col math consistent for every line-terminator kind | P0 |
| 4.4 | Deeply nested input / parser stack overflow | classic (JSON/YAML/XML/JS parsers; many CVEs) [R] | Recursive-descent parser overflows native stack on `((((...` or `[[[[...` | 8.P, 8.T, 8.L, 8.G | Depth limit (e.g. 256-1000) with diagnostic; test 10^6 nested parens/brackets/blocks/unary `!!!!`/`-----`/binary chain `a+a+...` (left-assoc loop vs recursive build; drop of deep AST also recursive!), long `else if` chains, deeply nested types/generics, 10^5-long string concatenation expression; also lowering/codegen/AST Drop on those trees | P0 |
| 4.5 | Algorithmic blow-up (quadratic/exponential) in parser/checker | ReDoS family; TS "excessively deep" [R] | Backtracking in arrow-function vs parenthesised-expr ambiguity, generics `<` vs less-than, type inference | 8.P, 8.T | Ambiguity cases: `(a,b,c,...)=>` with 10^4 params, `f<a,b>(c)` vs comparison chains, regex literal vs division, template-literal nesting, ASI hazards; time-bounded tests (budget assertion) | P0 |
| 4.6 | ReDoS in tooling regexes | ReDoS class [R] | Hand-written lexer/formatter/diagnostics regexes backtrack | all tooling | Grep the workspace for `regex` use in non-test code of syntax/check crates; fuzz with long near-miss inputs; require linear-time engine (Rust `regex` crate is linear) | P2 |
| 4.7 | Formatter changes meaning | known formatter bugs (Prettier/rustfmt/gofmt issues on comments, ASI, parens) [R] | Dropping parens, reordering comments/directives, ASI insertion, string-escape rewriting, number-literal normalisation (`1e400`, `0.1+0.2`, `.5`) | 8.P | Property test: `parse(format(src)) == parse(src)` modulo spans, and `format(format(x))==format(x)`, over corpus + fuzz; specifically: `return\nx`, `a\n(b)`, `+ +a`, `- -a` (token fusion `--`), `a++ +b`, `/` regex vs div, `<` generics, escapes in strings (`\u{...}`), comment inside expression, pragma/`// @ts-ignore`-like directives, shebang, 0-width chars inside strings | P0 |
| 4.8 | Token fusion / re-tokenisation after printing | JS minifier bug class [R] | `a- -b` printed as `a--b` | 8.P printer, 8.L if any codegen-to-text | Printer unit test over all operator pairs | P1 |
| 4.9 | Diagnostics injection (ANSI/terminal escapes, newlines, bidi in messages) | CVE class "terminal escape injection" (e.g. many CLI tools) [R]; Trojan Source applied to diagnostics [S for CVE] | Source snippet/identifier echoed in error message contains ESC `\x1b[`, OSC 8, CR, bidi controls: spoof output, rewrite terminal title, hide text | 8.P/8.T/8.R diagnostics, OxiTerm itself (renders in a terminal!) | Diagnostic renderer must escape/sanitise control chars (C0, C1, DEL, bidi, ZW) in any echoed source/identifier/string; test with `\x1b]0;pwn\x07`, `\x1b[2J`, `\r` overwrite, 8-bit CSI U+009B; this is specifically relevant because OxiTerm is a terminal UI | P0 |
| 4.10 | Source-map / span injection | source-map attacks, path leakage [R] | Spans out of range or mid-codepoint panic in slicing; maps with `../` paths leak/overwrite files | 8.P, 8.R | Spans at byte offsets inside multibyte char; empty file; span at EOF; hot-reload diagnostics for module path with `..`, absolute path, symlink; if a source map format is emitted, disallow external file refs | P1 |
| 4.11 | Prototype pollution | lodash CVE-2019-10744 / CVE-2018-3721 [R]; npm merge/clone family | `obj["__proto__"].x=1` or `constructor.prototype` mutates shared prototype for all objects | 8.T, 8.L, stdlib | Impossible by construction: records have fixed layouts, no prototype chain, no dynamic property creation, `__proto__`/`constructor` are ordinary keys only in `Map<string,T>` (a hash map, not an object). Tests: (1) `Map.set("__proto__",v)` does not affect any other value or the map's own methods; (2) record/spread/`Object.assign`-like merge of untrusted `Map` into a record is a type error or only copies declared fields; (3) JSON parse of `{"__proto__":{"x":1}}` produces plain data; (4) grep AST/IR: no op can write to a shared mutable global through a string key; (5) property test that two independent values share no mutable state unless via explicit reference | P0 |
| 4.12 | Dynamic code execution / eval-like | JS `eval`, `Function` [R] | Runtime code gen bypasses checks | 8.R | No `eval`; hot reload goes through the full 8.P/8.T/8.G pipeline and validation; test that a reloaded module cannot gain capabilities not granted to the slot (compare capability manifest old vs new) | P0 |
| 4.13 | Capability confusion / ambient authority | object-capability literature; JS global leakage [R] | Guest reaches host API through global, import name guessing, or forged handle | 8.R, 8.T | Module imports only declared capabilities; test: wasm import named like a non-granted host function fails link; handle values are opaque indices validated against a per-instance table (forged integer rejected, handle from other instance rejected, use-after-close rejected, generation counter prevents ABA) | P0 |
| 4.14 | Integer literal / numeric parsing edge cases | R | `0x` overflow, `1_000` separators, `1e999`, `-9223372036854775808`, leading zeros/octal, huge digit strings (quadratic bigint parse) | 8.P | Literals beyond i32/i64/f64 range -> diagnostic not wrap; 10^6-digit literal parse time bounded; float literal round-trips through formatter exactly | P1 |
| 4.15 | String literal / template escape handling | R | Invalid escapes, `\u{110000}`, lone surrogate escapes, unterminated at EOF, template nesting depth | 8.P | Reject invalid scalar values; unterminated -> diagnostic with span inside file | P1 |
| 4.16 | Resource limits on source size / token count / identifier length | R | 1GB source, 10^7 identifiers, 10^6-char identifier | 8.P, 8.R (hot reload accepts source over network?) | Hard caps with diagnostics; memory proportional to input; hot-reload endpoint size cap | P1 |

---

## 5. Supply-chain classes (FUTURE; relevant when a package manifest exists)

Not for current slices. All entries [R] unless stated.

| # | Class | Examples | Mechanism | Future test idea | Pri |
|---|---|---|---|---|---|
| 5.1 | Typosquatting | npm `crossenv` (2017), PyPI numerous | Near-miss package names | Manifest resolver edit-distance/confusable warning vs popular names; apply 4.2 confusable rules to package names | future |
| 5.2 | Dependency confusion | Birsan 2021 | Private name also exists on public registry; resolver prefers higher public version | Scoped/namespaced registries pinned per dependency; never fall through to another registry | future |
| 5.3 | Install/build scripts | npm `postinstall` abuse (event-stream 2018, ua-parser-js 2021, Shai-Hulud worm 2025 -- recall, verify) | Arbitrary code at install time | OxiScript manifest has no install scripts; any build step is itself sandboxed OxiScript with declared capabilities | future |
| 5.4 | Capability widening via dependency update | general | Dep update silently requests more host capabilities | Lockfile records capability set per dependency; update that widens it fails without explicit flag; ties to 4.12/4.13 | future |
| 5.5 | Lockfile/hash integrity, version-range resolution | general | Mutable tags, unpinned ranges | Content-addressed deps (hash in lockfile), reproducible compile (same source -> same wasm bytes) | future |
| 5.6 | Malicious manifest parsing | general | Deeply nested/huge manifest, path traversal in `files` entries | Apply section-4 limits to manifest parser; reject `..`/absolute paths | future |

---

## Mandatory attack list per slice (prioritised, <= 15 each)

### 8.P parser / formatter (oxiscript-syntax)
1. Bidi controls raw in comment/string/identifier rejected; escaped form allowed (4.1)
2. Invisible/zero-width/BOM/NUL/U+2028-9/invalid UTF-8/overlong encodings (4.3)
3. Homoglyph and NFC/NFD identifier policy; mixed-script rejection (4.2)
4. Nesting depth: 10^6 parens/brackets/blocks/unary/types; deep left-assoc binary chain; AST Drop also non-recursive or depth-capped (4.4)
5. Formatter round-trip `parse(fmt(x)) == parse(x)` and idempotence over corpus plus fuzz (4.7)
6. ASI/`return\nx`/token-fusion (`- -a`, `a+ +b`) printing hazards (4.7, 4.8)
7. Ambiguity blow-ups: arrow vs paren, generics vs `<`, regex vs `/`, 10^4 params (4.5)
8. Diagnostics: ESC/OSC/CSI/C1/CR/bidi in echoed source are sanitised (4.9)
9. Spans in the middle of multibyte chars, EOF, empty file; CRLF/CR/LS line math (4.10, 4.3)
10. Numeric literal overflow, 10^6-digit literal, `1e999`, separators (4.14)
11. String escape validity (`\u{110000}`, surrogates), unterminated at EOF (4.15)
12. Source size/token/identifier length caps (4.16)
13. Lexer/parser never panics: cargo-fuzz on arbitrary bytes with `catch_unwind` assertion and time budget (4.4-4.6)
14. Comments/pragmas preserved or intentionally dropped; directive comments cannot change semantics (4.7)

### 8.T type checker (oxiscript-check)
1. Array/collection invariance (writing supertype through alias) (1.1)
2. Method param contravariance incl. overrides, callbacks, shorthand vs property (1.2)
3. Casts/`as`: no unchecked unsound assertion; runtime-checked or rejected (1.3, 1.4)
4. Narrowing invalidated by calls/closures/aliased writes (1.7, 1.8)
5. Index access OOB/missing key yields `T|undefined` or trap (1.5)
6. Generic constraint write-back / `T[K]` unions / invariance in T (1.13)
7. Use-before-init: class fields via overridable ctor call, closure-captured `let`, recursive `const` (1.10, 1.21)
8. Unforgeable branded/opaque/capability types (1.15, 4.13)
9. Checker termination budget: recursive types, 10^5-member union, instantiation depth (1.19, 4.4)
10. Closed enums and exhaustive match with unreachable trap (1.11, 1.17)
11. Optional-property semantics vs explicit undefined (1.6)
12. Readonly alias semantics as documented; never relied upon by optimiser (1.14)
13. Overload/impl consistency (1.12)
14. Equality/NaN/-0 spec consistency (1.22, 2.7)
15. Differential: a corpus of "soundness probe" programs from TS playground soundness.ts and FAQ must each be E, with expected diagnostics snapshot (all section 1)

### 8.L lowering (typed AST -> IR)
1. Optimised vs unoptimised IR differential on programs with calls between bounds check and use (2.1)
2. Loop-induction range inference: wrap, huge step, `<=` off-by-one, len mutated in body (2.2)
3. Evaluation order: args, compound assignment `a[i()] += f()`, `a[i] = a.push(..)`, short-circuit with side effects, destructuring defaults, spread (class of "side-effect modelling")
4. Drop/RC insertion at every early exit: `return`/`break`/`continue`/trap-capable op/`?`-propagation/exception-like paths; temporaries in discarded expression statements (1.20, 2.5)
5. Closure capture lowering: captured mutable var boxed once, shared by all closures; capture of loop var per iteration; self-referential closure (2.5)
6. Generic monomorphisation: ref vs scalar instantiation never share layout assumptions (2.6)
7. Constant folding matches wasm runtime semantics incl. traps, -0, NaN, shifts, div overflow (2.7, 2.8)
8. Definite-init: uninitialised slot never read or dropped (1.10, 1.21)
9. Pattern-match/enum exhaustiveness lowers default arm to trap, not fallthrough (1.11)
10. Deep-expression lowering recursion depth (4.4); IR size blow-up from desugaring (nested spread/templates) (4.5)
11. Struct width subtyping/layout: upcast preserves ability to drop all fields (1.16)
12. Narrowing + lowering: refinement used by lowering to elide a check must be invalidated by same rules as checker (1.7)

### 8.G IR -> Wasm codegen (oxiscript-wasm)
1. Every emitted module validates under wasmparser and wasmi with proposals off; mutate bytes fuzz (3.13, 3.5)
2. Address arithmetic: `base+offset` wrap, offset=0xFFFFFFFF, memory.copy/fill/init zero-length at boundary (3.2, 2.3)
3. Integer op boundary table vs reference interpreter (INT_MIN/-1, shifts, trunc NaN, conversions) (2.8)
4. Bounds-check elision proofs: forced-on vs elided-on differential (2.2)
5. call_indirect: OOB index, null entry, signature mismatch, stale table after reload (3.15)
6. Stack depth: deep guest recursion emits trap not host crash; locals count limit (65k+ locals), function size limits, br_table size (2.12)
7. Big modules: 10^5 functions/globals/data segment size/ type count limits (4.16)
8. Allocation size math emitted inline: `n*elem`, `cap*2` with overflow check emitted (2.3)
9. f64 canonicalisation and NaN-boxing (if any) never lets scalar be read as pointer (2.6, 3.14)
10. Deterministic output: same input -> same bytes (5.5, aids reload diffing)
11. Data/element segments: instantiation-time OOB traps leave defined state (3.6)
12. Fuel metering points: every loop back-edge and call counts (if metering injected by codegen rather than wasmi) (3.8)
13. Block/loop/br label depth miscount under nested expression results (classic codegen bug); fuzz structured nesting (2.1)
14. Locals zero-init and slot reuse across scopes: reused local holding stale pointer must not be dropped twice (2.5)

### 8.M RC memory runtime (oxiscript-rc)
1. Size/count overflow on alloc, concat, repeat, grow (2.3)
2. Mutation of container during iteration/callback in every stdlib HOF (2.4)
3. Drop re-entrancy: element drop mutates the container being dropped; drop of self-referencing closure while executing (2.5)
4. Deep structure drop (10^6-node list/tree) without native/wasm stack overflow (iterative free) (2.12, 4.4)
5. Cycle collector: cycles via closures, maps, arrays; cycle with external ref; cycle during collection that triggers drop; weak refs; collector triggered at OOM and at fuel trap (2.5, 3.9)
6. Fault injection sweep: trap at every allocation/fuel point, then verify heap invariants (3.9, 3.10)
7. Refcount saturation/overflow at 2^32 (or underflow on double-drop); behaviour defined (trap or saturating-immortal) (2.3)
8. Allocator: double free, free of interior pointer, header corruption cannot be induced by guest-typed code; red-zone/poison debug mode run in CI (2.5)
9. Conservative scanning absent: i32 payload equal to valid pointer never traversed (2.6)
10. Memory limit: OOM returns defined trap, no null deref, no partial structure left with dangling slot (3.10)
11. Typed layouts: uninitialised slots zeroed or never dropped; partially constructed objects on OOM during construction (1.10)
12. Fragmentation/exhaustion DoS: alloc patterns that defeat free-list; time-complexity bound of alloc/free (algorithmic DoS) (4.5)
13. Host-visible buffers: after `memory.grow`, no runtime-internal cached base pointers (2.13)
14. String ops UTF-8 validity after slice/concat (2.11)

### 8.R wasmi sandbox runtime (oxiscript-runtime)
1. Fuel: exhaustion on `loop{}`, bulk memory ops, host calls with O(n) work charged proportionally (3.8)
2. Memory/table/instance limits via ResourceLimiter; grow to limit and beyond (3.10)
3. Call depth/stack limit: deep recursion and recursion through host calls (2.12, 3.11)
4. Host-call arg validation: (ptr,len) overflow, OOB, unaligned, pointing into runtime metadata (3.12)
5. Host reentrancy with `memory.grow` inside callback; no stale slices; capability revoke during callback (3.11, 2.13)
6. Capability forging/ABA/cross-instance handle/use-after-close (4.13, 1.15)
7. Link-time: unknown/extra imports rejected; module cannot import non-granted host fns; proposals off (3.5, 3.13)
8. Trap hygiene: after any trap (fuel, OOB, unreachable, OOM) instance is discarded; no state migrated; host catches wasmi panics (3.4, 3.9)
9. Instantiation failure paths: failing start fn, data-segment OOB, memory min over limit: no leak, old instance kept (3.6)
10. Hot reload: stale table indices, closures/handles from old version, capability manifest diff blocks widening, atomic swap, concurrent reload during execution (4.12, 3.15)
11. Dependency audit gate: wasmi at or above the CVE-2025-66627 fix; `cargo audit`/deny in CI; `#![forbid(unsafe_code)]` where possible (3.1, 3.3)
12. Diagnostics from runtime (trap messages, guest-supplied log strings) sanitised for terminal (4.9)
13. Wall-clock time limit independent of fuel (host calls that block, I/O capabilities) (3.8)
14. Output/log volume caps (guest printing 10^9 bytes through host) (3.8, 4.16)
15. Determinism: same inputs -> same fuel consumption and results across runs/platforms (3.14)

---

## Gaps / follow-ups for a later pass

- No primary page was fully read this session; promote S/R rows to V by reading: wasmi GHSA-g4v2-cjqp-rfmq (mechanism, fixed version), TS FAQ and issues #9825/#9998/#62666, CERT VU#999008, Project Zero RCAs for CVE-2020-6418, GitHub Security Lab CVE-2023-3420 write-up.
- JSC and SpiderMonkey rows 2.6/2.7 lack verified CVE ids; collect from Project Zero 0day-in-the-wild RCA index.
- WAMR/Wasmer advisories not collected (3.16).
- wasmi-specific fuel/limits advisories: none found by search; absence is not evidence of absence -- check RustSec `wasmi` and GHSA directly.
