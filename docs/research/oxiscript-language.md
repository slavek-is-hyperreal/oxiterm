# OxiScript: language and execution-stack research

> **Superseded in part (2026-11-26):** the phased roadmap, the estimates and the architecture table (it names `oxiscript-front`) are superseded by [../roadmap/oxiscript.md](../roadmap/oxiscript.md); the research findings and evidence tags remain valid input. Two claims about relaxed SIMD were removed (see section on engines).
>
> **Note (2026-11-26, plan-cleanup):** this document predates the roadmaps. Where it cites `oxi-ui-plan-history.md` with line numbers or section names, it means the old OxiUI plan file as it was at commit `3017fd5` (its Phase 1-7 text is kept in `docs/history/oxi-ui-plan-history.md`; its Phase 8 text, principles `P-n` and decisions `D-n` are now in [../roadmap/oxiscript.md](../roadmap/oxiscript.md), the OxiUI half in [../roadmap/oxiui.md](../roadmap/oxiui.md)).

Research input for the design of **OxiScript**, a statically and soundly typed, TypeScript-feeling, general-purpose language for the OxiUI project. Written 2026-11-24.

**Evidence tags used throughout**

- **[V]** verified: read in a primary source or official documentation this session (link given). Where a claim reached me only through a search-engine summary of a source, it is tagged **[S]** instead.
- **[S]** per search summary: plausible and sourced, but I did not read the underlying page myself. Re-verify before relying on it.
- **[P]** project estimate: a number or schedule that is my estimate for this project, not a measurement.
- **[O]** opinion: my recommendation or judgement.
- **[U]** unknown or not researched. I say so rather than guess.

Not deeply researched (one-sentence treatment only): Chapel, Pony, Hack/HHVM, Vale, aWsm, Gleam, Koto, Cython, Go's runtime internals, DWARF-in-Wasm debugging details. The Grain memory model and MoonBit licence could not be confirmed (see below).

---

## 1. Executive summary

### 1.1 Recommended architecture (with confidence)

| Layer | Recommendation | Confidence |
|---|---|---|
| **Front end** | Own front end written in Rust, as a library crate (`oxiscript-front`): lexer, parser, query-based incremental type checker, formatter and LSP share it. It must itself build for wasm32 (the "write and run live inside the core" requirement means the compiler ships inside `oxiterm-core`, including the browser build). Build, do not adopt: no existing project combines sound TS-like typing, a Wasm-in-linear-memory target, permissive licence and a profile mechanism (section A, comparison). | Medium-high [O] |
| **Type system** | Sound, fully static (no gradual typing, no `any`, no unchecked `as`). Nominal structs/classes, TS-style discriminated unions with exhaustive narrowing, one absence value (`T \| null`), explicit numeric types (`i32`, `i64`, `f32`, `f64`, `u8`...; no `number`), UTF-8 strings, monomorphised generics with a dictionary/`dyn` escape, bidirectional *local* inference with annotations required on exported signatures, `Result`+`?` instead of exceptions. | High on soundness and no-`any`; medium on the rest [O] |
| **Memory model** | Value semantics for records/arrays (Hylo-style mutable value semantics, `inout` parameters), reference counting with compiler elision (Perceus/Lobster style) for heap objects, explicit arenas/regions as opt-in, handles (generational indices) for host-owned objects such as UI nodes, weak references for cycles, optional cycle collector only in the default (non-real-time) profile. | **Medium-low: the owner's biggest decision.** Counter-evidence: Static TypeScript found tracing GC about 2x faster than their RC [V]. Prototype both in phase 0. |
| **IR** | A typed, monomorphised SSA IR owned by OxiScript (`OxiIR`) is the real IR. **Core WebAssembly (MVP+multi-value+bulk-memory+SIMD128, tail calls only as an optimisation) is the primary backend and distribution format**, with all language values in linear memory. WasmGC, threads, exception handling, function-references are *not* used: the engines we need lack them (section D). | High for "Wasm linear memory as first backend"; medium for "Wasm is the only IR we ever want" |
| **Live engine** | wasmi (pure Rust, `no_std`, Apache-2.0/MIT, fuel metering, lazy translation) inside the core. In the browser this is wasm-in-wasm (interpreter under the browser JIT). **On a device, live reload needs an on-device interpreter** (AOT output is native and cannot be swapped without reflashing), so dev mode on ESP32 is wasmi or WAMR and release is AOT. Which of the two is an open decision: in treVM, wasmi was the fastest interpreter (CoreMark 17.3 on ESP-WROOM-32) but largest in flash; WAMR is smaller, an official Espressif component, but its fast interpreter ran out of memory on one Embench test on 3 of 4 boards [V]. | High for wasmi in core/browser; **medium-low for the on-device choice** |
| **Release engines** | Per target, same `.wasm` input: **browser engine** (web), **Wasmtime/Cranelift AOT** (x86_64, aarch64 only: Cranelift has no 32-bit backend [V]), **wasm→C (w2c2, MIT) plus the target C compiler** (32-bit SBCs, MCUs, and the likely safety-critical path), Pulley/wasmi interpreter as portable fallback. | High that wasm→C is needed; medium on exact engine mix |
| **SIMD / parallel** | Three layers: (0) default: data-parallel constructs (`map`/`reduce`/`parallel for` over arrays with value-semantics bodies) that the compiler may vectorise and the runtime may parallelise, with semantics **identical to sequential execution**; (1) explicit portable `Simd<T,N>` types; (2) opt-in schedules (Halide-style hints that never change results), tasks, isolates with message passing. Not shared-memory Wasm threads. Default on wasmi and MCUs: sequential. SIMD lowering decided per target profile. | Medium-high on the model; medium on schedules |
| **Transparency ("automatic, never silent")** | Every build writes a versioned placement report (SIMD/multicore/GPU/scalar, transfers, hot-path allocations, with a stable reason code when the fast path was declined); the LSP shows it as inlay hints; `@expect(simd|gpu|no_alloc|...)` assertions turn silent de-optimisation into compile errors and double as safety evidence. Assert only language-defined *facts*, report heuristic *choices* (section L). One remarks API from the first optimisation pass. | Medium-high [O] |
| **Single-source GPU/CPU kernels** | Ordinary functions under a compiler-checked `kernel` profile (same mechanism as the safety profile), typed `View`/`Image`/`Grid` buffers, residency visible in types (`DeviceBuffer<T>`, async transfers), results classed exact/approximate. Back ends in order: CPU scalar/SIMD/threads, WGSL (WebGPU, browsers and wgpu), SPIR-V/MSL via naga (MIT OR Apache-2.0 [S]). GPU is a `host:gpu` capability, not a language feature; graphics is a library on top (section K). | Medium (research-grade; must not block MVP) |
| **Host API** | Host interfaces declared in a small IDL (WIT-like, own syntax), imported as typed modules; capabilities passed as values, no ambient authority; UI is just the first host module. Component Model is an optional later adapter, not a dependency. | High on capability style; medium on avoiding Component Model |
| **Developer experience (primary persona: JS/TS dev with a kid and an ESP32)** | Compile on the host, run on-device as a precompiled module; `oxi flash` one command; live reload over USB/WiFi with atomic swap (Jaguar reaches <2 s [V]); browser simulator built on the same wasm32 core; VS Code via LSP; host-side REPL; manifest with package.json-like semantics but **no promise that npm packages run**. DX tools live around the compiler so a safety build drops them without removing language features (section J.3-J.4). | Medium-high [O] |
| **Real-time / safety** | **Profiles**: compiler-enforced restriction sets over *one* language with *one* semantics (Ravenscar/SPARK principle), never dialects. Decide the items in the day-1 column of section I.9 (no UB, effect summaries, explicit allocation in the IR, a written spec, deterministic compiler output, capabilities as values). Everything else can wait. | High |

### 1.2 What the research most strongly says

1. **Engine feature intersection is the hidden constraint.** wasmi lacks GC, threads, exception handling and function-references [V]; WAMR lists GC only as a post-MVP item without saying which modes support it [V]; w2c2 mentions no GC [V]. So language semantics must lower to core Wasm in linear memory. AssemblyScript is the cautionary tale: closures blocked for years on GC/function-references [V].
2. **Cranelift is not a universal AOT story.** No 32-bit backend [V]. wasm→C is the universal path.
3. **Parallelism must be semantics-preserving.** wasmi has no threads, browsers need cross-origin isolation for shared memory [V], shared-everything-threads is unimplemented in Wasmtime [V], most MCUs are single-core. So parallel constructs must mean the same thing run sequentially. This is also the determinism and certification story.
4. **Soundness is cheapest on day 1.** TypeScript explicitly lists soundness as a non-goal [V]; Dart's migration to sound null safety left mixed programs unsound until fully migrated [V]; Swift 6 strict-concurrency migration reports are full of pain [S].
5. **Auto-vectorisation is not a programming model** [V: Pharr; Maleki et al.].
6. **A later safety-critical profile is feasible if and only if** the language has no undefined behaviour, a written specification, explicit allocation points, and effect summaries from the start (section I).

### 1.3 Top-10 "avoid early" decisions (details in section H)

1. Unsound escape hatches (`any`, unchecked `as`).
2. An ambiguous `number` type.
3. Depending on a Wasm proposal some target engine lacks.
4. No LSP/formatter at launch.
5. A slow or non-incremental compiler.
6. Async/concurrency bolted on late.
7. Shared-memory concurrency as the default.
8. Reference counting without elision (and RC on every call).
9. Hot reload as an afterthought.
10. Trusting auto-vectorisation.

Plus the transparency one: optimisation passes without a decision/reason log, and reports in compiler-internal terms that nobody can act on (GCC, ISPC lessons, section L).

Plus the heterogeneous-compute one: do not let the language grow a separate shader dialect or implicit host/device transfers (section K).

Plus the safety-driven additions: undefined behaviour, implicit allocation, ambient authority, nondeterministic float/NaN behaviour, no written spec, non-reproducible compiler output (section H, items 11-16), and the persona-driven one: licensing or onboarding choices that break the "kid + ESP32 in five minutes" experience (section J; Moddable XS, Toit VM and Jaculus are GPL/LGPL, DeviceScript was archived).

### 1.4 Main open questions for the owner (full list in section 13)

1. Memory model: RC with elision vs tracing GC. Is a 1-2 week prototype of both acceptable before committing?
2. How TS-compatible must the *syntax* be? (Parse real `.ts`? Or TS-looking but its own grammar?) Recommendation: own grammar.
3. Is a "write once, run on MCU" promise in scope for v1, or only "same compiler, MCU backend later"?
4. Which safety standard is the first realistic customer target (it changes what to design for)?
5. Compiler team size and licence of any formal-verification back end.
6. Persona priority: MakeCode-style blocks editor and web flasher in or out of v1? (section J.3)
7. GPU scope: is "kernel profile + WGSL back end" wanted in v1, or is CPU SIMD/multicore enough for the first release? (section K)

---

## 2. Section A: Prior art (typed, TS-like or embeddable, compiled)

### A.0 Build versus adopt

| Candidate | Licence | Why not adopt as OxiScript |
|---|---|---|
| AssemblyScript | Apache-2.0 [U: not re-checked] | No closures, no iterators, limited unions, own GC runtime [V, status page]; no profile mechanism; TS-*like* but intentionally not a TS subset. Good as a *quick prototype* for UI scripts, not as the foundation. |
| Static TypeScript (MakeCode/pxt) | MIT [U] | Targets ARM Thumb only; unsound casts; C++ runtime tied to MakeCode [V]. Best *design* reference, not reusable code. |
| Luau | MIT [S: Wikipedia via search summary] | Typed Lua, sandbox-first, native codegen for x64/arm64 (1.5x-2.5x [S]) but types are advisory (incorrect annotations must not affect correctness [S]), not TS-like, no 32-bit/MCU story. Strong sandbox reference. |
| Roto (NLnet Labs) | [U] | Statically typed, JIT-compiled, hot-reloadable, Rust-embedded [V]; no hashmaps, no generic functions, no formatter/LSP yet [V]. Closest *shape* to what we want in the "live" half; no Wasm, no MCU. |
| Rhai / Rune / Koto | MIT/Apache typical [U] | Dynamically typed (Rhai) or VM-based (Rune) [S]; fail requirement 1 (sound static types) and 2 (native). |
| Porffor | MIT [U] | JS/TS AOT to Wasm/C, still described by its author as a research project [S]; JS semantics (dynamic) dominate. |
| Wasmnizer-ts (Intel) | Apache-2.0 w/ LLVM exception [S] | TS strict subset to WasmGC via Binaryen; needs WasmGC runtime; WAMR AOT for GC was "under development" in a 2023 thread [S]. Conflicts with the engine-intersection constraint. |
| MoonBit | **Unverified.** One search result claimed "permissive SSPL" which is self-contradictory (SSPL is not OSI-approved) [U]. | Owner must check the compiler licence at the primary source before any dependency; the licence policy makes this matter. Main target is WasmGC [S]. |
| Mojo | Apache-2.0 w/ LLVM exception since 2026-08-18; outside contributions to the compiler not yet accepted [S: Modular blog, Phoronix] | Python-like, AI/GPU oriented, not TS-like, not MCU. Interesting for its explicit SIMD/parallel model (section E). |

**Conclusion [O]:** build an own front end, but borrow aggressively: Static TypeScript's subset philosophy, Roto's Rust embedding, Luau's sandbox, Dart's hot-reload and null-safety rules.

### A.1 Static TypeScript (MakeCode)

Source: Moskal, Ball, de Halleux, MPLR 2019 ([paper](https://www.microsoft.com/en-us/research/uploads/prod/2019/09/static-typescript-draft2.pdf), [blog](https://www.microsoft.com/en-us/research/blog/rocket-fast-embedded-typescript-for-makecode-arcade/)).

- Compiles a TS subset in the browser straight to ARM Thumb, linked with a C++ runtime; devices as small as 16 kB RAM (micro:bit) [V/S].
- Dropped `eval`, `with`, prototype inheritance [V]. Classes are **nominal** (TS is structural) [S]. Interface calls ~2x slower than class calls (hashed vtable), dynamic maps ~6x slower [V].
- Numbers: 31-bit tagged ints, other values boxed doubles [V]. This is the "TS `number` problem" paid for with tagging.
- **Casts get no runtime check** [V]. That is an unsoundness we should not copy.
- Memory: replaced reference counting by a non-moving precise mark-and-sweep; **"switching to GC roughly doubled speed"** [V]. Their RC updated counts on every call with no elision [V]; hence this is evidence against *naive* RC, not against elided RC [O].
- Performance: object-heavy code at least 20x faster than MicroPython/Duktape; a bytecode interpreter fallback (for platforms forbidding generated code) ~5x slower than native but still faster than other interpreters [V]. Complex games at ~30 fps on Cortex-M4 [V].
- What to take: subset-by-design, nominal classes, ahead-of-time in a browser-sized compiler, precomputed C++ runtime, an interpreter fallback. What not to take: unchecked casts, tagged `number`.

### A.2 AssemblyScript

Sources: [status page](https://www.assemblyscript.org/status.html), [closures issue #173](https://github.com/AssemblyScript/assemblyscript/issues/173) [S].

- Not a TS subset by intent: no `any`/`undefined`, union types only for nullable classes, no closures capturing locals, no iterators/`for..of`, no rest parameters, no async, exceptions abort [V].
- Runtime: TLSF allocator + incremental tri-colour mark-sweep (ITCMS) by default, with minimal and stub alternatives [S].
- **Closures wait on Wasm GC / function-references** (capturing an environment needs GC support) [S, issue #173]. Years of waiting on a spec proposal is the lesson: do not make a core feature depend on an engine proposal. We implement closures as `{fn_index, env_ptr}` pairs in linear memory with `call_indirect` [O].
- What worked: familiar syntax, Binaryen optimisation, small output. What hurt: users keep hitting missing TS features; the project calls itself not-TS on purpose.

### A.3 Static Hermes (Meta)

- Adds static types and optional native compilation to Hermes; slides state that **sound** types are required for performance, that Static Hermes *modifies some JS semantics* to allow efficient sound typing, and that ordinary TS annotations are unsound and give no runtime guarantee [S: Mikov decks, [announcement](https://speakerdeck.com/tmikov2023/static-hermes-react-native-eu-2023-announcement)].
- Lesson: if types drive codegen, the types must be sound, and semantics must deviate from the untyped language where needed. That is exactly our position; do not try to remain a TS superset.

### A.4 Gradual typing to native: Typed Racket, mypyc, Sorbet, Hack

- **Typed Racket**: soundness via higher-order contracts at typed/untyped boundaries; POPL 2016 "Is Sound Gradual Typing Dead?" measured overheads of over 100x in some configurations; a 2017 follow-up showed a tracing JIT (Pycket) removes most of it [S: [paper](https://users.cs.northwestern.edu/~dmf082/papers/popl16-tfgnvf.pdf)]. Lesson: avoid gradual typing; our boundary with untyped data (JSON, host) is an explicit checked-decode, paid once at the edge [O].
- **mypyc**: compiled mypy is about 4x faster; annotations are *enforced at runtime* and type errors block compilation; semantics differ from Python (no `__dict__`, no monkey patching) [S: [docs](https://mypyc.readthedocs.io/en/latest/differences_from_python.html) ]. Lesson: types that drive codegen must be enforced, and the compiled language is a different language.
- **Sorbet**: `sig` also adds runtime checks, generics erased at runtime [S: [docs](https://sorbet.org/docs/runtime)].
- **Hack/HHVM**: gradual types with runtime-enforced parameter/return types. [U: not deeply researched.]

### A.5 Dart (JIT + AOT, sound null safety)

- Soundness is explicit and exploited by the compiler: non-null types allow dropping null checks and smaller/faster code; **mixed null-safe and legacy programs are not fully sound until everything is migrated** [V: [Dart docs](https://dart.dev/null-safety/understanding-null-safety)].
- Development uses JIT with hot reload, release uses AOT [S]. Hot-reload semantics in section F.
- Lesson: ship soundness in version 1; a migration costs years.

### A.6 Kotlin/Native, Swift, Go/TinyGo, Zig, Nim, Roc

- **Kotlin/Native**: the legacy memory model required *freezing* objects to share across threads, with `InvalidMutabilityException` etc.; the new manager lifted this and freezing is deprecated [V: [migration guide](https://kotlinlang.org/docs/native-migration-guide.html)]. The current collector is concurrent mark-and-sweep, non-generational [V: [docs](https://kotlinlang.org/docs/native-memory-manager.html)]. Lesson: a restrictive sharing discipline that users must manage by hand does not survive contact with real programs; our isolates must make the safe thing the easy thing.
- **Swift**: *Embedded Swift* shrinks firmware from ~500 kB to under 10 kB in one forum report [S: [forum](https://forums.swift.org/t/embedded-swift/67057)]; generics specialisation increases binary size [S]. Swift 6 strict concurrency migrations produced anecdotes of tens of thousands of warnings [S: [Swift forums](https://forums.swift.org/t/my-experience-attempting-a-migration-to-swift-6/79069)]. Lesson: a restricted subset (profile) can reach MCU sizes; concurrency safety retrofitted late is painful.
- **Go / TinyGo**: TinyGo uses a conservative mark-sweep GC, a cooperative scheduler on MCUs, and lists multi-core use as a non-goal [S: [InfoQ](https://www.infoq.com/news/2020/04/tinygo-go-iot-microcontrollers/)]. Lesson: on MCUs cooperative single-thread scheduling is the norm; goroutine-style APIs can still be offered if they degrade gracefully.
- **Zig**: async/await was removed from the language in the 0.11 cycle mainly because the self-hosted compiler needed to bootstrap without it, then redesigned around an `io` interface in 2025 [S: [commit](https://git.jakstys.lt/motiejus/zig/src/commit/9f0b9b8da1a111d16eb8d1254212ff98a8b4be08), [LWN](https://lwn.net/Articles/1049006/)]. Zig also passes allocators explicitly, which is the model we want in the real-time profile. Lesson: async is cross-cutting; decide its shape before the compiler core hardens.
- **Nim**: ARC (non-atomic reference counting via destructors and moves) and ORC (ARC plus a trial-deletion cycle collector); ORC "gives up some determinism" [S: [Nim docs](https://nim-lang.org/docs/mm.html)]. Directly relevant: this is our candidate memory model.
- **Roc**: rewrote ~300,000 lines of compiler from Rust to Zig over 487 days; main stated reason was Rust compile times, plus allocator-passing fit [S: [gist](https://gist.github.com/rtfeldman/77fb430ee57b42f5f2ca973a3992532f)]. Lesson: compiler build time and architecture are product features; plan crate splitting from the start.
- **Gleam, Koto, Cython, Vale, Pony, Chapel**: not deeply researched [U].

### A.7 WASM-first languages: Grain, MoonBit

- **Grain**: memory-management status could not be confirmed from a primary source [U]; its authors expect WasmGC to help [S].
- **MoonBit**: targets WasmGC by default, with a native/LLVM path; size claims are promotional and unverified [S]. Licence unverified (see A.0).
- Lesson: WASM-first languages are converging on WasmGC *for browsers*; that is a browser-only backend for us, because wasmi, w2c2 and (apparently) most MCU engines lack it [V].

### A.8 Mun (hot-reload typed language)

- Built "with hot reloading in mind"; Rust-hosted, LLVM-based, dual MIT/Apache-2.0; requires LLVM 14; "we do not provide support for milestone releases" [V: [repo](https://github.com/mun-lang/mun)].
- Its own announcements say a v0.3 plan "bit off more than we could chew" and development is in a core contributor's spare time [S: [Open Collective](https://opencollective.com/mun/updates/mun-v0-3-0-released)]. I found **no statement of why it stalled**, only indirect signals [U]. The most defensible lesson: hot reload plus a custom type system plus an LLVM dependency was a lot for a part-time team; and pinning an old LLVM is a maintenance trap.
- Mun's internal struct-migration mechanism: [U, from memory only, not verified].

### A.9 Rust embeddables: Rhai, Rune, Koto, Roto

Covered in A.0. The benchmark repo [script-bench-rs](https://github.com/khvzak/script-bench-rs) compares Rhai, Rune, mlua, rquickjs, boa and wasmi [S]. Interesting data point for us: **wasmi itself is on that benchmark list as an embeddable scripting back end** [S].

---

## 3. Section B: Type system

### B.1 Soundness versus TypeScript

TypeScript's design goals list "apply a sound or provably correct type system" as a **non-goal** and "add or rely on run-time type information" as another [S: [wiki](https://github.com/microsoft/TypeScript/wiki/TypeScript-Design-Goals)]. Its own soundness page gives type assertions and parameter bivariance as examples [S].

For compile-to-native, unsoundness is not cosmetic: the compiler would lay out memory based on a type that may be false. Static Hermes and mypyc both resolve this by making types binding [S].

**Decisions [O]:**
- No `any`; provide `unknown` plus checked narrowing and a typed decode API for untyped input. *(MVP status 2026-11-26: `unknown` was not adopted in the MVP and is reserved as `OXS-E0900` in `grammar.md`; JSON/untyped decode comes via crates in 8.N; final owner decision tracked in `Q-LANG-5`.)*
- `as` is either a *checked* downcast (`as?` returning `T | null`, or trapping `as!`) or a numeric conversion with defined semantics. No assertion that merely claims a type.
- Arrays and function parameters are covariant/contravariant exactly as soundness requires (arrays of mutable elements invariant). This will reject some TS programs; accept that.
- Non-nullable by default; `T | null` with flow narrowing; **one** absence value (no `undefined`).

### B.2 Structural versus nominal

STS chose nominal classes for speed (direct field offset) [V]; interface calls cost ~2x [V]. Go shows a middle path: structural *interfaces* satisfied implicitly, but concrete types nominal [O, general knowledge].

**Recommendation [O]:** nominal `struct`/`class`; structural *interface satisfaction* (so TS-style duck typing for behaviour works); anonymous object *types* allowed only when fully known (compile to a struct, no dynamic property bags); `Map<K,V>` and `Record` as library types, not language magic.

### B.3 Unions and narrowing

Keep TS discriminated unions (`kind: "circle"`) as sum types with exhaustive `switch`; represent as tag + payload in linear memory. Drop arbitrary `string | number` unions with typeof-narrowing *unless* both arms are disjoint and tag-representable (then it is just a sum type with implicit tags) [O]. AssemblyScript supports only nullable unions [V]; STS likewise subset. Exhaustiveness checking is also needed later by safety profiles.

### B.4 Generics: monomorphisation versus boxing

- Monomorphisation: fastest, simplest sound semantics, but code size grows (Embedded Swift forum notes specialisation size [S]).
- Dictionary-passing / erased generics: smaller, slower, harder with value types.
- **Recommendation [O]:** monomorphise by default; allow `dyn Trait`-style erased use explicitly; give profiles a code-size knob (`max_instantiations`) and report instantiation counts in the compiler (the cheapest defence against bloat). Roto still lacks generic functions [V], a sign of how much work this is; scope generics narrowly in the MVP (type parameters on functions/structs/interfaces with simple bounds, no higher-kinded types, no conditional/mapped types).

### B.5 Numbers (the TS `number` problem)

TS `number` is one IEEE double. STS pays tagged ints + boxed doubles for it [V]; AssemblyScript replaced `number` with explicit wasm types and BigInt with i64 [V].

**Recommendation [O]:** explicit `i8..i64, u8..u64, f32, f64`, plus aliases `int = i32` (or pointer-sized?) and `float = f64`; decide `int` width once (i32 is the natural Wasm/MCU width). *(MVP status 2026-11-26: `int`/`float` aliases were not adopted in `grammar.md` 1.1; explicit `i32`/`f64` etc. are used instead; owner confirmation tracked in `Q-LANG-5`.)* Literal inference defaults to `int`/`float`; a literal adapts to context when unambiguous. Overflow: **checked (trap) by default**, with explicit `+%` (wrapping), `+|` (saturating), `checked_add` returning `Result`. Rationale: determinism and no UB; cost is a branch, which profiles/optimisers can remove with range analysis or an explicit unchecked operator in a `trusted` block. Division by zero, shifts out of range, array bounds: all defined (trap or `Result`), never UB (section I).

### B.6 Strings

JS/TS strings are UTF-16; Wasm and the Rust core are UTF-8. **Recommend UTF-8 strings** [O]: zero-copy with the Rust core, 2x smaller for ASCII, matches linear-memory layout. Cost: `.length`/index semantics differ from JS; expose `bytes()`, `chars()` (code points), `graphemes()` (library), and forbid O(1) code-point indexing. WasmGC's JS-string builtins would only help a browser-only backend [V: Wasm 3.0 notes].

### B.7 Inference and compile speed

- TS itself uses local, bidirectional inference with structural subtyping; its full type-checker is slow enough that Microsoft ported it to Go for about 10x speed [S: [announcement coverage](https://www.infoq.com/news/2025/05/new-typescript-compiler-10x-fast)]. The port was explicitly a port, not a rewrite, and gained speed from native code and parallelism [S].
- **Recommendation [O]:** bidirectional local inference; require annotations on exported/public function signatures; infer locals, closures bodies and generic arguments at call sites; no global Hindley-Milner across modules (error messages degrade and compile time becomes non-local). Query-based incremental design (Salsa-style, [U: not researched here]) so the LSP and hot reload re-check only changed functions.

### B.8 What to drop or change to stay sound and fast, yet "feel like TS"

| TS feature | Decision |
|---|---|
| `any`, unchecked `as`, non-null `!` assertion | Drop; checked forms only |
| `undefined` and `null` both | One: `null` |
| `number`, `bigint` | Explicit numeric types |
| Prototype inheritance, `this` rebinding, `eval`, `with`, `arguments` | Drop (also dropped by STS [V]) |
| Structural everywhere | Nominal types, structural interface satisfaction |
| Conditional/mapped/template-literal types | Drop initially; maybe a small subset of generics bounds later |
| Declaration merging, namespaces | Drop; use modules |
| Exceptions (`throw/catch`) | `Result` + `?`, panics trap |
| `enum` | Real sum types; plain C-like enums as sugar |
| Getters/setters, decorators | Defer; properties as methods |
| Syntax: arrow functions, `let/const`, template strings, destructuring, optional chaining, `async/await`, `for..of`, classes, interfaces, generics | Keep (these are what "feels like TS") |

**What makes it feel like TS [O]:** syntax and idioms, inference, editor behaviour (hover types, narrowing, quick fixes), error message tone. Not: semantics.

### B.9 Error message quality

[U: not researched with sources.] Opinion: budget for diagnostics from day one (spans, labels, suggestions, error codes with docs), in line with Rust/Elm practice; Roto's author calls its new Rust-like syntax a response to "surprise" [V].

---

## 4. Section C: Memory management

### C.1 Options and evidence

| Approach | Evidence | Fit for OxiScript |
|---|---|---|
| Tracing GC in linear memory (AssemblyScript ITCMS, TinyGo, Kotlin/Native CMS) | AS ships TLSF + incremental GC [S]; V8 explains linear-memory GCs cannot see the Wasm stack (need shadow stack or collect only between event-loop turns) and cannot form fine-grained cycles with JS [V: [V8 blog](https://v8.dev/blog/wasm-gc-porting)]; STS got ~2x over its RC [V] | Simplest for cyclic TS-style code; pauses and shadow-stack cost; poor for hard real-time |
| WasmGC (engine GC) | Shipped in Chrome 119, Firefox 120, Safari 18.2 [S: caniuse/Chrome blog]; part of Wasm 3.0 [V]; Wasmtime tier 1 [V]; **wasmi: not supported** [V]; V8: fannkuch 2.3 kB vs 6.1-9.6 kB for C/Rust, but ports need new toolchains, no interior pointers, LLVM unsupported [V] | Optional browser-only release backend later; breaks the single-semantics goal and the live engine |
| Reference counting with elision (Perceus in Koka) | "Garbage free" precise RC; reuse analysis enables in-place functional updates; Koka tree insertion within 10% of in-place C++ std::map [S: [Perceus](https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/)]; cycles not collected | Strong fit: deterministic, no pauses, small runtime. Needs cycle policy |
| ARC/ORC (Nim) | ARC non-atomic, no stop-the-world; ORC adds trial-deletion cycle collector but loses some determinism; recommended default for new Nim code [S] | Proven model: ARC in real-time profile, ORC in default profile |
| Swift ARC | Atomic RC overhead is well known [U: no source fetched]; Embedded Swift keeps ARC but drops many features [S] | Evidence RC can reach MCU sizes |
| Ownership/borrowing (Rust) | Safe, no runtime cost; steep learning curve [O] | Too hard for "as easy as TS" |
| Mutable value semantics (Hylo/Val) | Independence of values, second-class references, safety checked by abstract interpretation over an IR [S: [Hylo papers](https://arxiv.org/pdf/2106.12678)] | Good for records/arrays; combine with RC for shared objects |
| Regions / scoped memory (RTSJ, Verona) | RTSJ: scoped/immortal memory hard to use correctly; heap-touching checks at runtime; Safety-Critical Java then prohibits the heap [S]; Verona: regions with cowns and behaviours [S: [OOPSLA 2023](https://www.microsoft.com/en-us/research/publication/reference-capabilities-for-flexible-memory-management/)] | Explicit arenas as opt-in; full region typing is research-grade |

### C.2 Recommended model [O, medium-low confidence]

1. **Value types by default** for `struct`, tuples, fixed-size arrays; copy-on-assign semantically, in-place optimised (mutable value semantics), `inout` parameters for mutation. No hidden aliasing -> data-race freedom for parallel constructs (section E).
2. **Reference types** (`class`, `Array<T>` buffers, closures, strings) are RC-managed, non-atomic *within an isolate*; crossing an isolate moves or copies. Compiler elides RC with Perceus-style ownership inference; reuse where uniqueness is proven.
3. **Cycles**: weak references and explicit `Handle<T>` (generational index into a host-owned or script-owned slab). UI nodes are the typical cyclic structure; they are *host objects reached by handle*, not script heap objects, so the script rarely builds cycles [O].
4. **Arenas**: `arena { ... }` blocks and per-frame arenas for temporaries; allocations inside are freed wholesale. Zig-style explicit allocator parameters in the standard library for real-time profile [O].
5. **Optional cycle collector** (ORC-style trial deletion) *only* in the default profile; real-time profile forbids creating cyclic RC types (checked by type shape) or requires `weak`.
6. Reserve a **WasmGC backend** as a later, browser-only option; do not let language semantics depend on it.

### C.3 Honest uncertainty and what to prototype

- STS's measured 2x for GC over RC [V] is the best direct evidence *against* RC; its RC lacked elision [V], but I have not seen a published like-for-like comparison with Perceus-style elision on a TS-like language [U].
- Linear-memory tracing GC needs a shadow stack and cooperative safepoints; with wasmi and `wasm2c` there is no stack scanning, so GC runs between events (V8 notes this limitation [V]). This works for event-driven UI, poorly for long-running loops.
- Phase-0 experiment [P: ~2-3 weeks]: implement both RC-with-elision and a simple mark-sweep over the same IR for three workloads (UI event handlers with closures, a drawing/game loop, a data-parallel kernel) and measure pause, throughput, RAM, code size on wasmi and native.

### C.4 Determinism and pause times

RC: deterministic frees, but a free can cascade (large graph drop); mitigate with deferred/bounded destruction queues [O]. Tracing: pauses scale with heap. UI at 60 Hz has a 16.7 ms budget [P: arithmetic]; MCU real-time has WCET constraints (section I). Arenas and no-alloc subsets give truly bounded behaviour.

---

## 5. Section D: Execution engines and IR

### D.1 Is WebAssembly the right IR?

**Pros [mostly V]:** validated, sandboxed, memory-safe-by-construction bytecode with an independent spec validator; runs in every browser; many engines (interpreter, AOT, wasm→C) with different footprints; Wasm 3.0 includes a deterministic profile [V]; output can be checked by tools we did not write (a certification argument, section I); Wasefire notes Wasm can be validated whereas Pulley bytecode cannot [S: [Google blog](https://opensource.googleblog.com/2025/11/secure-by-design-firmware-development-with-wasefire.html)].

**Cons [O unless marked]:** lowering to Wasm loses high-level information (loop parallelism, aliasing, SIMD intent), so those must be encoded in OxiIR and either lowered or dropped *before* Wasm; no stack scanning, no threads in wasmi [V]; all memory accesses are bounds-checked (software on MCUs without guard pages) [U: figures not collected]; structured control flow; JIT/AOT engines re-optimise poorly if we emit naive code (use Binaryen `wasm-opt`, Apache-2.0 [V: repo page]).

**Alternatives considered:**

| Alternative | Verdict [O] |
|---|---|
| Own bytecode + interpreter (Luau/Roto/Rune style) | Fastest to iterate live, but we then own the sandbox proof, an AOT backend, and tooling; no browser JIT help. Keep as the *fallback* if wasmi translation latency or footprint fails (STS shipped a custom interpreter ~5x slower than compiled [V]) |
| Cranelift directly (Roto-style JIT) | Great native speed and fast compile (about 10x faster than LLVM, code up to ~2x slower on some benchmarks, 2020 comparison [S]); but no 32-bit backend [V], no browser, no MCU |
| LLVM | Best code, huge, slow, pinned version risk (Mun needs LLVM 14 [V]); Apache-2.0 w/ exceptions fine for licence; reject as primary |
| C as target (Nim, Porffor `2c`) | Excellent MCU reach and certification story (qualified C compilers exist); lose sandbox unless via Wasm→C; strong *second* backend directly from OxiIR later [O] |
| MLIR | Academic fit for multi-level lowering; heavy C++ dependency; reject for now. A 2025 paper (WAMI) compiles to Wasm through MLIR [S: [arXiv](https://arxiv.org/html/2506.16048v1)] |
| TPDE-style fast backend | Research: 4.27x faster than Cranelift, 1.74x slower than Winch compile time in Wasmtime [S: [TPDE](https://arxiv.org/pdf/2505.22610)]; watch, do not adopt |

**Conclusion [O]:** Wasm is the right *distribution and sandbox format* and first backend; OxiIR (typed SSA) is the right *compiler IR*. Keep the door open for a direct C backend and for a custom bytecode.

### D.2 wasmi

- Register-based IR since 0.32: claims up to 5x faster than the old stack machine, Coremark 657 -> 1457 on an EPYC 7763; startup improved by lazy translation; validation is 20-40% of startup; strongest on AMD server chips, weaker on Apple silicon [V (project's own claims): [wasmi 0.32](https://wasmi-labs.github.io/blog/posts/wasmi-v0.32)].
- Current 2.0.x: Apache-2.0/MIT, `no_std`, fuel metering, supports tail calls, multi-memory, memory64, SIMD, relaxed-SIMD, wide-arithmetic; **no function-references, GC, threads, exception-handling** [V: [README](https://docs.rs/crate/wasmi/latest/source/README.md)]; WASI p1 via `wasmi_wasi` [V]. **Component Model: not listed** [V]; I found no MCU size data for Component Model [U].
- MCU study treVM (IEEE DCOSS-IoT 2026): on four boards wasmi was the fastest interpreter (CoreMark 4.8-17.3) but had the **largest code size** among the compared runtimes; WAMR smallest; Wasmtime had one of the smallest RAM footprints; authors chose Wasmtime without SIMD as balance [V: [arXiv 2604.27570](https://arxiv.org/html/2604.27570v1)]. Their Wasmtime on 32-bit MCUs can only be the Pulley interpreter (Cranelift has no 32-bit target [V]); the paper summary does not say so explicitly, so treat this as my inference [O].
- **Translation latency for a small module (needed for "compile in milliseconds")**: not found in published form [U]. The wasmi-benchmarks repository has `startup/` categories (compile+instantiate) but the README has no numbers [V: [repo](https://github.com/wasmi-labs/wasmi-benchmarks/blob/main/README.md)]. Must be measured in phase 0: our own front-end time is a [P] estimate (target: tens of ms for a 1-2k line script; unmeasured).
- **Fuel overhead and no_std flash size**: unknown [U]. Wasmtime's fuel overhead has been reported at 15-40% in a 2022 issue vs ~10% for epochs [S].

### D.3 Other engines

- **Wasmtime**: Cranelift tier 1 on x86_64, aarch64, s390x, riscv64; **no 32-bit**; Winch baseline x86_64 (aarch64 in development); Pulley portable interpreter for other targets (pulley32 exists); `no_std` partial (runtime, gc, component-model, pulley, async) with precompiled modules; GC, SIMD, relaxed-SIMD, tail-call, EH, memory64, function-references tier 1; threads tier 2 with known issues; shared-everything-threads unimplemented [V: [platform](https://docs.wasmtime.dev/stability-platform-support.html), [proposals](https://docs.wasmtime.dev/stability-wasm-proposals.html)]. Fuel (deterministic, more overhead) vs epochs (cheaper, nondeterministic) [S: [docs](https://docs.wasmtime.dev/examples-deterministic-wasm-execution.html)].
- **WAMR**: Apache-2.0 w/ LLVM exception; modes classic interpreter, fast interpreter, AOT, Fast JIT, LLVM JIT; core VM text on Cortex-M4F ~58.9 kB fast interp, ~56.3 kB classic, ~29.4 kB AOT runtime [V: [README](https://github.com/bytecodealliance/wasm-micro-runtime)]. The fast interpreter was designed for speed; Intel reports ~150% better CoreMark vs classic [S]. In treVM it used far more RAM on some tests and ran out of memory on one benchmark on 3 of 4 boards [V].
- **wasm3**: fast interpreter, MIT [U: licence not re-checked]; README notes "minimal maintenance phase" in some mirrors [S]; Arduino library needs ~64 kB flash and ~10 kB RAM minimum [S]. Treat as legacy; do not build on it.
- **wasm2c (WABT)** and **w2c2**: w2c2 is MIT, "Coremark ~7% slower than native", supports WebAssembly 2.0, threads/atomics, SIMD via SIMDe, no GC mentioned [V: [README](https://github.com/turbolent/w2c2)]. Wastrel (2025) reports 2-5% slower than native on Coremark and w2c2 slightly slower than that [S: [Wingolog](https://wingolog.org/archives/2025/10/30/wastrel-a-profligate-implementation-of-webassembly)]. A 2026 benchmark found wasm2c competitive with Wasmer/Wasmtime on libsodium but without fuel, preemption or Component Model [S: [00f.net](https://00f.net/2026/07/08/webassembly-compilation-to-c-2026/)]. The generated C has no engine to ship; sandboxing relies on guard pages or explicit checks.
- **Wasefire** (Google): custom in-place interpreter with very small footprint, no compute-heavy applets; alternative is Wasmtime+Pulley with larger footprint and unvalidatable bytecode [S: [blog](https://opensource.googleblog.com/2025/11/secure-by-design-firmware-development-with-wasefire.html)]; licence [U].
- **aWsm, wasmer**: not researched [U].

### D.4 Browser engines

The release path in the browser is: instantiate a second Wasm module with the browser's engine; host calls cross through JS import glue. Core itself is already wasm32, so the *live* path is **wasm-in-wasm** (wasmi interpreting under the browser JIT), which is slower but acceptable for UI event handlers [O]. Because both modules are linear-memory, the script module can import or share the core's memory, and UTF-8 strings need no conversion; WasmGC objects live in the JS heap and would need copying at every host call [O].

### D.5 Engine comparison table

| Engine | Licence | no_std / footprint | Interp speed | AOT | SIMD | Threads | Metering | MCU support | GC |
|---|---|---|---|---|---|---|---|---|---|
| wasmi 2.x | Apache-2.0 or MIT [V] | no_std; largest flash of compared runtimes in treVM (figure not in text) [V] | Fastest interpreter in treVM CoreMark [V] | No | Yes (incl. relaxed) [V] | No [V] | Fuel [V] | Yes (nRF52840, RP2350, ESP32, ESP32-C6 in treVM) [V] | No [V] |
| WAMR | Apache-2.0 w/ LLVM exc. [V] | ~29-59 kB core on Cortex-M4F [V] | Fast-interp ~1.6-2x classic in treVM [V] | Yes; JIT too [V] | Listed post-MVP feature [V] | pthread, wasi-threads [V] | [U] | Yes (Espressif component) [V] | Listed, mode support unclear [V] |
| wasm3 | MIT [U] | ~64 kB flash/10 kB RAM min [S] | Fast, historically; maintenance mode [S] | No | No [U] | No [U] | No [U] | ESP32/RP2040/Arduino [S] | No |
| Wasmtime (Cranelift) | Apache-2.0 [V: repo page; the page did not mention an LLVM exception, so I do not claim one] | std mostly; partial no_std w/ precompiled modules [V] | n/a (JIT/AOT) | Yes, 64-bit targets only [V] | Yes [V] | Tier 2 [V] | Fuel + epoch [V] | Only via Pulley [V] | Yes [V] |
| Wasmtime Winch | same | x86_64 only [V] | baseline code | Yes | Partial [V] | | Fuel | no | partial |
| Pulley | same | no_std portable; RP2350 bare-metal reports [S] | "not the fastest interpreter" [S] | n/a | Yes (treVM tested) | | | Yes | Yes |
| w2c2 | MIT [V] | Generates C; no engine | ~7% slower than native [V] | Yes (via C compiler) | SIMDe [V] | atomics [V] | No (DIY) | Any C99 target [V] | No |
| wasm2c (wabt) | Apache-2.0 [U] | Generates C | ~native-ish [S] | Yes | Yes [S] | partial [U] | No | Any C target | No |
| Browser engines | n/a | n/a | JIT | JIT | SIMD128 yes; relaxed SIMD support differs by engine and is not relied on [U] | Needs cross-origin isolation [V] | No | n/a | Yes |
| Wasefire interpreter | [U] | Very small [S] | Slow; "not for computation-heavy" [S] | No | [U] | No | [U] | nRF52840, OpenTitan [S] | No |

Deterministic profile: Wasm 3.0 defines one (deterministic NaN, relaxed SIMD) [V: [Wasm 3.0](https://webassembly.org/news/2025-09-17-wasm-3.0/)]; wasmi supports relaxed-SIMD [V], but we exclude it by default (section I.5).

---

## 6. Section E: SIMD, SWAR and parallelism

### E.1 Why automatic vectorisation alone is the wrong contract

- Maleki et al. (PACT 2011): GCC 4.7, ICC 12, XLC 11 vectorised 45-71% of a 151-loop synthetic suite but only 18-30% of loops from real applications [S: [MSR](https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers)]. A 2025 TSVC2 study: GCC ~54%, ICX ~50%, Clang ~46%; no compiler clearly best [S: [arXiv 2502.11906](https://arxiv.org/html/2502.11906v1)].
- Pharr (ispc): vectorisation "can fail, and it fails unpredictably", failures are silent, and programmers resort to guesswork; SPMD makes parallelism implicit but *predictable* [V: [post](https://pharr.org/matt/blog/2018/04/18/ispc-origins)]. In the retrospective: ispc got 3-4x on 4-wide Intel but 2x on the ARM CPU tested; he regrets lack of explicit vectors alongside SPMD and flags `unmasked` as dangerous [V: [retrospective](https://pharr.org/matt/blog/2018/04/29/ispc-retrospective)]. Note: that retrospective does not itself discuss auto-vectorisation; the argument is in the earlier post [V].
- Halide separates algorithm from schedule so the same code can be retargeted by changing only the schedule [S: [PLDI 2013](https://people.csail.mit.edu/jrk/halide-pldi13.pdf)].
- Futhark: uniqueness types allow race-free in-place updates in a data-parallel functional language; moderate flattening and sequentialisation of excess parallelism [S: [PLDI 2017](https://www.futhark-lang.org/publications/pldi17.pdf)].
- Rayon: data parallelism made safe by Rust's `Send`/`Sync`; a captured non-atomic counter in a `par_iter` closure fails to compile [S: [Rayon docs](https://docs.rs/crate/rayon/latest/source/src/./lib.rs)].
- Swift 6: late-added data-race safety caused large migration cost [S]. Kotlin/Native: freezing model dropped [V].
- Chapel, Pony, Verona (cowns/behaviours, [S]), Erlang actors, Go: only mentioned; not deeply researched [U].

### E.2 WASM facts that constrain the design

- SIMD128: shipped broadly (Firefox >= 89, Safari >= 16.4 [S: Emscripten docs]); relaxed SIMD availability differs by engine (an earlier claim that all three major engines ship it was not verified and has been removed); relaxed SIMD results may differ by implementation, so it is excluded from the deterministic profile [V].
- SIMD is **not always faster on MCU interpreters**: in treVM "Wasmtime No SIMD" beat the SIMD build on 3 of 4 boards (RP2350 18.6 vs 13.2) [V]. So SIMD lowering must be decided per target profile from one front end, not by shipping one universal `.wasm` [O].
- Threads: Wasm threads need shared memory; in browsers `SharedArrayBuffer`/shared `WebAssembly.Memory` require a secure, cross-origin-isolated context (COOP/COEP) [V: [MDN](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer)]. wasmi: no threads [V]. Wasmtime threads tier 2, known issues with resource limiting [V]. wasi-threads is being superseded by shared-everything-threads, which Wasmtime lists as unimplemented [V/S]. Rust `wasm32-wasip1-threads` exists [S].
- Single-core MCUs: cooperative scheduling (TinyGo) [S].

### E.3 Recommended model [O]

**Principle: parallel and SIMD constructs have sequential semantics.** A program has one meaning; targets choose *how fast*, never *what*.

1. **Layer 0, default and automatic.** Data-parallel combinators over arrays/ranges (`map`, `reduce`, `scan`, `for_each`, and `parallel for i in 0..n`) whose bodies are free of cross-iteration writes by construction: value semantics, uniqueness (Futhark-style) and `inout` slices enforce it in the type system. The compiler may (a) vectorise (SIMD128 where the target profile enables it), (b) hand chunks to a runtime scheduler. Where the program is not provably independent it is *not* parallelised, and the compiler can report why (an optimisation report, replacing LLVM's silent failure).
2. **Layer 1, explicit SIMD.** Portable `Simd<f32, 4>`/`Simd<i16, 8>` with lane ops, masks, shuffles; lowered to `v128` or scalar loops; semantics identical either way (no relaxed ops, no NaN-propagation differences in the deterministic profile). This is the ispc lesson of "mix explicit vectors with SPMD" [V].
3. **Layer 2, explicit control as annotations that cannot change results.** `@schedule(chunk: 256, vector: 4, tile: [32, 32])` à la Halide, `@layout(soa)` for data layout, `@no_parallel`. Opt-in thread pools/tasks: `spawn`/`join` structured (child cannot outlive scope), isolates (actors) for long-lived concurrent state, message passing by move/copy.
4. **Transparency** (section L): every placement decision is reported with a reason, shown as editor hints, and optionally asserted with `@expect(...)`.
5. **Runtime backends** (by target): native host: work-stealing threads (Rayon-like); browser: worker instances with transferred buffers (works *without* COOP/COEP if no shared memory is used; shared-memory backend optional when isolated) [O]; wasmi/MCU: sequential or cooperative tasks.
5. **Data races impossible by construction**: no shared mutable state across tasks except through isolates/channels or atomics in a small `sync` library; parallel closures capture only immutable values or uniquely-owned slices.

**Failures and successes:** auto-vectorisation alone fails (Maleki, Pharr [S/V]); Rayon, ispc, Halide succeeded by making the *programming model* explicit and the *mapping* automatic [S/V]; Swift 6 and Kotlin/Native show costs of adding or changing safety late [S/V].

**Graceful degradation on single-core MCUs [O]:** the same source compiles to sequential loops; SIMD profile off by default; cooperative `async` tasks for I/O concurrency; `parallel for` compiles to a plain `for`. The cost of keeping this guarantee is that some *explicitly* parallel-only algorithms (e.g. producer-consumer needing real concurrency to avoid deadlock) are forbidden in the portable subset: tasks must be deadlock-free under sequential execution.

---

## 7. Section F: Interop and host API

### F.1 Component Model / WIT / WASI

- Component Model enabled by default in Wasmtime (tier 1) but "not at Phase 4"; C API gaps remain [V: Wasmtime proposals page].
- WASI 0.3.0 released 2026-06-11 per wasi.dev, rebasing on Component Model native async (`stream<T>`, `future<T>`); Wasmtime >= 46 runs it by default; cooperative threading is a future extension [S: [WASI 0.3 announcement](https://bytecodealliance.org/articles/WASI-0.3)] (one secondary source gives February 2026; I use the official date).
- **wasmi does not list Component Model support** [V]; size cost on MCUs: unknown [U].
- **Recommendation [O]:** define OxiScript host interfaces in our own minimal IDL that compiles to plain Wasm imports/exports (typed, flat ABI: ints, floats, pointers+lengths into linear memory, handles). Keep the IDL *convertible to WIT* so a Component Model adapter can be generated for Wasmtime/browser later. Do not require Component Model on the live path.

### F.2 Capability sandboxing

Pattern: no ambient authority. A script's `main(host: Host)` receives capability values (`Ui`, `Clock`, `Storage`, `Net`); a script cannot import what it was not given. Wasm's import mechanism enforces it at the engine level; the language additionally enforces it statically so profiles can reason (e.g. real-time profile forbids `Net`). Luau sandboxing (limited API surface by default) [S] and Rhai ("the sandbox is the set of registered functions") [S] are the same idea.

### F.3 Not baking a domain into the language

- UI = a host module (`import { Canvas, Pointer } from "host:ui"`). Gestures, drag-and-drop, drawing are library code atop `Canvas` + events.
- Built-ins in the language: numbers, strings, arrays, maps, options/results, closures, tasks. Everything else is a library or a host interface.
- Test of generality: the same compiler builds a pure-computation module with *no* host imports, a UI script, and a bare-metal control loop with `host:gpio` [O].

### F.4 FFI to Rust

Roto's experience: registering Rust types/functions via a `library!` macro was a reworked API after real use; passing lists between Rust and Roto was "the hardest challenge" [V: [NLnet Labs](https://blog.nlnetlabs.nl/one-year-of-roto-the-compiled-scripting-language-for-rust)]. Our design [O]: a `#[oxiscript::host]` derive/attribute macro generating both the IDL and the Wasm import shims (wasmi `Linker` functions), with a small, fixed set of ABI-safe types and handles for everything else. Strings and byte slices as (ptr,len) in linear memory. Avoid passing Rust references into the guest.

### F.5 Async/await versus callbacks versus coroutines

- Wasm stack switching is Wasmtime tier 3, x86_64 Linux only [V]; not available in wasmi. So `async` must be a **compile-time state-machine transform** in OxiIR (as Rust and TinyGo-on-MCU do [S: TinyGo's goroutines use coroutine-style lowering]).
- UI events: callbacks registered by closures (non-escaping where possible) plus `async` for sequencing (e.g. a drag gesture as `for await` over pointer events).
- Zig's removal of async [S] and Swift's late concurrency [S] argue for deciding async semantics *now*, even if the implementation lands in phase 2. Structured concurrency: tasks cannot outlive their scope; cancellation is cooperative and checked at `await` points.
- Avoid function colouring pain by making `async` an inferred/propagated *effect summary* of the function (the same mechanism as the real-time profile's effects) rather than a type-level split of every API [O].

### F.6 Hot-reload state preservation

Reference designs:

- **Dart VM**: methods immutable with late binding; fields retained; renames are *not* migrated (only added/removed fields visible); reload accepted atomically or rejected; some class-kind/type-argument changes rejected; stale call-stack pitfalls (super calls, static/instance switches) [V: [Dart hot reload doc](https://dart.googlesource.com/sdk/+/stable/runtime/docs/hot-reload.md)]. Reload only changes behaviour going forward [S].
- **Erlang code upgrade**, **Mun**, **Live++**: not researched in this session [U]; Mun's own requirement of hot-reload-aware codegen is stated but not detailed [V].
- **WASM reality**: a reload creates a *new instance*; there is no in-place patching. Therefore persistent state cannot live in module globals.

**Recommendation [O]:** reload-surviving state lives in **host-managed, schema-versioned state** (OxiUI's `StateManager` already holds typed app state), accessed from the script as typed `state` handles with explicit migration functions on schema change; module-local state (statics, closures, caches) resets on reload. A reload is atomic: compile, validate, instantiate; swap on success, keep the old instance on failure (Dart's accept-or-reject). This avoids Mun-style layout migration and cleanly composes with the deterministic profile (reload = restart of pure code with restored inputs).

---

## 8. Section G: Tooling lessons

- **LSP from day one.** Roto still lacks formatter and LSP [V]; the TypeScript team's big move in 2025 was making type-checking and editor startup about 10x faster by porting to a native compiler [S]. A front end designed as a query-based library serves compiler, LSP, formatter and REPL from one codebase [O].
- **Compile speed is a feature**: Roc left Rust for Zig mainly for compile times [S]; TS port [S]; Cranelift/Winch/TPDE show backends can be 10x faster than LLVM at lower code quality [S]. Our compile speed budget [P]: front end + OxiIR + Wasm emission should be linear and parallel per module; the wasmi translation cost is separate and unmeasured [U].
- **Placement report, inlay hints and `@expect` assertions** (section L) belong to tooling from the first optimisation pass.
- **Formatter** (one canonical style), **test runner** (language-level `test` blocks, run on wasmi in-process), **package manager** (lockfile with content hashes, offline vendoring, semver; permissive licences only), **docs generator**. [O]
- **Debugging**: DWARF in Wasm and source maps are the standard approach for browser debugging of compiled languages [U: not verified this session]. Plan: source maps from day one, DWARF later; a debugger protocol (DAP) through the same LSP-style host.
- **Stable spec and versioning**: see section I.7 (editions, compatibility promise).
- **What projects regretted not doing early** [S/O]: Swift/Kotlin/Zig sharing/async/concurrency model; Dart null safety; AssemblyScript closures dependency; Roto's missing LSP.

---

## 9. Section H: Mistakes others made, decisions that avoid them

Each row: what happened (source) / early decision / cost for us. "Cost" is [P] or [O].

| # | What happened | Early decision | Cost for us |
|---|---|---|---|
| 1 | TS has `any` and unchecked assertions; unsoundness is a stated non-goal [V]. STS casts have no runtime check [V]. | No `any`, no unchecked `as`; `unknown` + checked decode | Some TS code will not port; mitigations: good diagnostics, codemods |
| 2 | TS `number` forces tagging/boxing (STS) [V]. | Explicit numeric types; define `int`/`float` aliases, checked overflow | Slightly less "JS-like" literals; small |
| 3 | AssemblyScript closures blocked on Wasm GC/function-references [S]; wasmi lacks GC/threads/EH/function-references [V]. | Lower everything to core Wasm in linear memory; engine-feature matrix checked in CI | We write our own closure/RC/async lowering |
| 4 | Roto still has no formatter or LSP [V]; TS team spent heavily on editor speed [S]. | Compiler as a query-based library from the first commit; LSP+formatter in MVP | ~15-25% of MVP effort [P] |
| 5 | Roc left Rust partly for compile times [S]; TS 7 native port ~10x [S]. | Incremental, parallel per-module compile; fixed compile-time budget tracked in CI | Architecture discipline; some design (no global inference) |
| 6 | Zig removed async over self-hosting [S]; Rust async colouring pain [O, no source]. | Decide async/task/cancellation semantics in the spec before the IR hardens; implement as state-machine transform | Spec work now, implementation later |
| 7 | Swift 6 retrofit of data-race safety hurt; Kotlin/Native freezing dropped [S/V]. | Isolates + value semantics + uniqueness; no shared mutable state by default | Some patterns (shared caches) need explicit `sync` types |
| 8 | STS RC had no elision and was ~2x slower than GC [V]. | Ownership-based RC elision in the IR (Perceus-like) or choose tracing GC; measure in phase 0 | 2-3 weeks prototype + ongoing complexity |
| 9 | Mun built hot reload around a custom runtime, LLVM 14 pin, part-time team [V/S]. | Hot reload = new Wasm instance + host-managed schema-versioned state; atomic swap | Users must put persistent state in `state` handles |
| 10 | Auto-vectorisation unreliable [S/V]; Intel `#pragma simd` weakness [V]. | Explicit data-parallel constructs with sequential semantics + explicit `Simd` types + optimisation report | New syntax, runtime scheduler |
| 11 | JS/TS/C programs have undefined or implementation-defined corners (e.g. evaluation order, NaN bit patterns) [O]. | Written spec: left-to-right evaluation, defined overflow/shift/bounds, no UB, deterministic NaN in default profile | Small perf cost on some targets |
| 12 | Implicit allocation (closures, string concat, boxing) makes real-time impossible to retrofit; RTSJ scoped memory proved hard to use [S]; Safety-Critical Java dropped the heap [S]. | Allocation is explicit in OxiIR; every function has an `alloc` effect summary; allocator is an explicit parameter in the stdlib | IR design constraint, little code |
| 13 | Dynamic dispatch and recursion block WCET analysis; aiT needs loop bounds and recursion depth [S]. | Effect summaries include `recursive`, `unbounded_loop`; `dyn` is closed-world checkable | Same mechanism as #12 |
| 14 | Ambient authority makes sandbox claims hard to audit [O]. | Capabilities as values; host IDL declares them | Slightly more verbose `main(host)` |
| 15 | Unwritten language semantics: Rust needed Ferrocene to write the spec for qualification [S, unverified details]. | Spec (executable reference interpreter + prose) alongside the implementation, editions policy | Ongoing 5-10% of effort [P] |
| 16 | Non-reproducible tool output undermines qualification and supply-chain claims [O]. | Compiler output byte-for-byte deterministic; hash-locked packages; SBOM from the package manager | Low if done early, high to retrofit |
| 17 | Pinning an old LLVM (Mun, LLVM 14) [V] or an obscure engine (wasm3 minimal maintenance [S]). | Depend on a small set of well-maintained engines behind one backend trait | Small abstraction layer |
| 18 | A "full JS" engine on MCUs needs LGPL/GPL or commercial licences (Moddable XS, Toit VM, Jaculus) [V/S]. | OxiScript runtime MIT/Apache-2.0 only | Own engine/runtime work |

---

## 10. Section I: Safety-critical and hard real-time readiness

Owner's test: "tomorrow someone may want to run a space rocket on it". Aim: keep the easy TS-like default, make a later safety-critical *profile* possible, add nothing UI-specific.

### I.1 What other ecosystems do

| Ecosystem | Mechanism | Lesson for us |
|---|---|---|
| **Ada/SPARK + Ravenscar** | `Profile (Ravenscar)` is a bundle of restrictions (no implicit heap allocation, restricted tasking) over the same Ada language; SPARK excludes access types in general and proves absence of run-time errors [S: [Ravenscar](https://en.wikipedia.com/wiki/Ravenscar_profile), [SPARK slides](https://secdev.ieee.org/wp-content/uploads/2019/09/SPARK-Tutorial-Slides.pdf)] | **Profiles are restriction sets over one language, not dialects.** Subset only removes features; semantics never change |
| **MISRA C** | Coding subset enforced by static analysis [U: no source fetched] | Enforceable subset is useful even without proofs |
| **Rust no_std/no-alloc** | Crates opt out of std/alloc; allocation explicit [O, well known] | Libraries layered by capability (`core` / `alloc` / `std`) |
| **Zig** | Explicit allocator parameters [S] | Same for our stdlib real-time layer |
| **RTSJ / Safety-Critical Java** | Scoped and immortal memory; no-heap real-time threads with runtime checks; difficult to use correctly; SCJ restricts further and bans the heap [S] | Do not rely on dynamic checks for memory regions; make "no heap" a *static* profile property |
| **Erlang** | Soft real-time with per-process GC [O, no source] | Isolates with private heaps give bounded GC pauses per isolate |
| **WCET analysis (aiT etc.)** | Works on binaries; needs loop bounds and recursion depth bounds; automatic bounds only for simple loops [S: [AbsInt](https://www.absint.com/a3/gallery.htm), [TU Dortmund](https://ls12-www.cs.tu-dortmund.de/daes/en/research/wcet-aware-compilation/motivation/timing-analysis.html)] | Language should provide *bounded* loop forms (`for i in 0..N`, `for x of array`) so bounds are derivable; keep unbounded `while` allowed in default profile only |

### I.2 Profile mechanism

A **profile** is a named set of compile-time restrictions declared in the package manifest and per module (`#profile(realtime)` style attribute); the compiler enforces it by checking the **effect summaries** it already computes for every function. Examples of restrictions: `no_alloc` (after init), `bounded_alloc(N bytes)`, `no_recursion`, `bounded_loops`, `no_panic` (every trap point proven unreachable or replaced by `Result`), `no_dyn`, `no_float` or `deterministic_float`, `no_async`, `closed_world`, `no_host(Net)`. Profiles **only remove** capabilities; the same source compiles in the default profile unchanged. This is the Ravenscar principle [S]. Profile composition: a call from a `realtime` function to a non-conforming function is an error.

Default profile = TS-like convenience (RC allocation, closures, strings, arrays, panics trap the isolate). `realtime` profile = bounded, allocation-free steady state. `safety` profile = realtime + restricted language subset + contracts required + a smaller trusted standard library [O].

### I.3 Bounded execution

- **Compiler-inserted budget checks** (opt-in per profile): a counter decremented at loop back-edges and calls, trapping or yielding at zero. Works on *every* backend (wasmi, browser, wasm2c) unlike engine fuel, which wasmi and Wasmtime provide but wasm2c and browsers do not [V]. Overhead is unmeasured for us; Wasmtime fuel overhead was reported 15-40% in an old issue [S] and epoch interruption ~10% [S]. Placing checks only at back-edges and recursion entries is cheaper [O].
- Host side: wasmi fuel and Wasmtime fuel for deterministic limits; epochs for wall-clock watchdogs [V].

### I.4 Error handling

No exceptions (stack unwinding, EH proposal absent in wasmi [V], unbounded cost). `Result<T,E>`, `?`, `Option`. **Panics** (bounds, overflow, `unwrap`, explicit `panic`) trap the current isolate; the host policy decides restart, safe state or reset. A profile can require `no_panic` proofs for a function (arithmetic via checked/saturating/proven ranges; array access via `get` or proven indices). Policy for library code: panics only for programmer errors, never for I/O or data errors.

### I.5 Numeric determinism

- Integers: fixed widths, defined overflow (trap / wrap / saturate explicit), defined shifts and division edge cases. No platform-dependent `int`.
- Floats: IEEE 754 binary32/64 only; no implicit FMA contraction; no `x87`; NaN payload and sign canonicalised in the deterministic profile (Wasm 3.0 provides a deterministic profile [V]); relaxed SIMD excluded (results implementation-defined [V]); `f32`/`f64` math functions taken from our own libm (not host libm) in the deterministic profile so results agree across wasmi, browser, wasm2c [O]. Cost: slower transcendental functions; fine.
- Evaluation order: left-to-right, specified. Hash-map iteration order: insertion order or explicitly unspecified-but-deterministic (seeded constant) [O].

### I.6 Verification friendliness

Lessons: Liquid Haskell keeps refinements in a decidable fragment (quantifier-free, linear integer arithmetic with uninterpreted functions) for predictable solving, and notes that quantifiers are the main source of trouble in Dafny-style tools [S: [LH blog](https://ucsd-progsys.github.io/liquidhaskell-blog/2019/10/20/why-types.lhs), [Explicit Refinement Types](https://arxiv.org/pdf/2311.13995)]. SPARK removes pointers/aliasing to make proofs tractable [S]. Dafny/F* brittleness data for 2025-26: not found [U].

**Design now [O]:** (a) no UB and a formal core (small-step semantics for a core language kept in the spec); (b) value semantics and no aliasing of mutable data, which also helps parallelism; (c) reserve syntax: `requires`, `ensures`, `invariant`, `decreases`, `ghost`, `pure` (parsed and *type-checked*, executed as runtime assertions in debug, erased in release); (d) keep expressions in contracts within a decidable fragment (integers, arrays with bounds, no quantifiers initially); (e) translate OxiIR + contracts to an SMT back end later (Z3 is MIT and cvc5 is BSD-3 per my knowledge [U: not re-verified]; Why3 is LGPL [U], so avoid linking, call as a separate tool).

### I.7 Specification, versioning, stability

Rust's editions and Go's compatibility promise are the reference models [O, from memory, not re-fetched: [go1compat](https://go.dev/doc/go1compat)]. For us: a written spec per edition; semantic version of the compiler; `edition = "2027"` in the manifest; deprecations over editions; profiles versioned too. An executable reference interpreter (the wasmi path or a tiny OxiIR interpreter) doubles as spec oracle and differential-testing partner.

### I.8 Certification evidence and tool qualification

- DO-330 defines tool qualification objectives with levels TQL-1 to TQL-5; tools whose output becomes airborne software (e.g. compilers) generally need stricter levels than verification tools [S: [Rapita](https://rapitasystems.com/do-330)].
- **Ferrocene** is the existence proof that an open-source compiler can be qualified: qualified for ISO 26262 ASIL D, IEC 61508 SIL 3 and IEC 62304 Class C by TUV SUD (2025 press release), plus an IEC 61508 SIL 2 certification of a *subset* of the core library [S: [Ferrous Systems](https://ferrous-systems.com/blog/officially-qualified-ferrocene/), [BusinessWire](https://secure.businesswire.com/news/home/20251203818102/en/Ferrous-Systems-Achieves-IEC-61508-SIL-2-Certification-for-Rust-Core-Library-Subset)]. Lesson: qualification wraps process, a specification and test evidence around a compiler; a small, specified, deterministic compiler is far cheaper to qualify.
- **Structural advantage of Wasm [O]:** the compiler's output can be checked by an independent validator and run on independent engines (differential testing); Wasefire stresses that Pulley bytecode cannot be validated like Wasm [S]. Qualification can lean on "tool output is verified" (lower TQL-style argument) rather than "tool is flawless" [O, to confirm with an assessor].
- **Likely rocket path [O]:** OxiScript (safety profile) -> validated Wasm -> w2c2/wasm2c -> a qualified C compiler for the target. That reuses existing C-compiler qualification; a direct OxiIR->C backend is the cleaner variant. A third path (own native backend) is far costlier.
- IEC 62443 and CRA: CRA reporting duties apply from 2026-09-11, main obligations from 2027-12-11 [S: [Skadden](https://www.skadden.com/insights/publications/2026/09/get-ready-reporting-faqs-and-checklist)]. The CRA's SBOM expectation: from my memory of Annex I, not re-verified [U].
- **IEC 62443**: the business model's anchor, yet I did not research it here [U]. Follow-up: IEC 62443-4-1 (secure development lifecycle) and 62443-4-2 (component requirements) map onto the spec, reproducible builds, SBOM, vulnerability handling and capability-based sandboxing described in this section.
- **Reproducible builds + SBOM**: no sources found in this session on DO-330 tie-in [U]. Design: compiler output deterministic (sorted maps, no timestamps/paths), lockfile with content hashes, `oxi sbom` emitting CycloneDX/SPDX, builds in hermetic mode, signed releases [O].

### I.9 What must be decided at the start versus what can wait

| Decide at the start (hard or impossible to retrofit) | Can wait |
|---|---|
| No undefined behaviour; defined overflow, shifts, bounds, evaluation order | A formal proof of the core semantics |
| Written spec + editions policy | Full verification tool / SMT back end |
| Static types, sound; no `any` | Refinement types / dependent types |
| Allocation explicit in OxiIR; allocator abstraction in stdlib | Region/lifetime type system |
| Effect summaries per function (alloc, panic, recursion, loops, async, host capabilities) | Effect polymorphism for users |
| Capabilities as values; no ambient authority | WASI/Component Model adapter |
| Errors as values; panic policy (trap isolate) | `no_panic` proof assistance |
| Deterministic compiler output; hash-locked packages | SBOM/signing UX, DO-330 documents |
| Isolates + value semantics (race freedom by construction) | Work-stealing scheduler, schedules |
| Sequential semantics for parallel/SIMD constructs | Auto-parallelisation heuristics |
| Reserve contract syntax; budget-check instrumentation point in IR | Contract checking implementation, WCET tooling |
| Async design decided (state-machine, structured) | Full async implementation |
| Own libm and canonical NaN story (deterministic profile) | Hardware-FPU fast path |

Honest expectation [O]: a rocket-grade deployment would still need years of evidence work and independent assessment. The point of the early decisions is that nothing in the language has to be *changed* to get there.

---

## 11. Section J: Developer experience for JS/TS developers on microcontrollers

Primary persona (owner): **a frontend or Node backend developer building a toy with their kid on an ESP32 and feeling at home.** DX for this persona has priority; safety remains possible.

### J.1 Prior art

| Project | Onboarding and workflow | Simulator / ecosystem | Performance and memory on ESP32-class | Licence | Outcome and lessons |
|---|---|---|---|---|---|
| **MakeCode / Static TypeScript** | Browser editor, no install; blocks <-> TypeScript toggle; flash by drag-and-drop UF2 or WebUSB [S: [makecode.com](https://makecode.com/language)] | In-browser simulator; large extension (package) library on GitHub [S] | Native Thumb code, 16 kB RAM micro:bit up to ~100 kB Cortex-M4 arcade; ~30 fps games [V] | pxt open source, MIT [U: not re-checked] | Succeeded in education; kids start in blocks and graduate to TS. Target is ARM Thumb only, not ESP32 [S]. **Closest DX model.** |
| **Moddable SDK / XS** | Install SDK + toolchains (heavier setup); `mcconfig` build and flash; `xsbug` source-level debugger [V] | Desktop simulator for screens; module ecosystem; TC53 standards effort [S] | Full ECMAScript (XS 15 supports ES2024 [S]); bytecode runs from ROM, XS engine ~170 kB on ESP32 in 2018 build, "comfortably under 400 kB" full [S]; typical device ~45 kB free RAM, 1 MB flash [V]; needs preload/precompile on host to save RAM [S] | SDK: GPL-3.0, LGPL-3.0, Apache-2.0, CC-BY-4.0 mix, plus commercial option [V]: **mixed; does not meet our licence policy as a dependency** | Succeeded in showing JS can run on ESP32-class with careful engineering; not beginner-onboarding-first |
| **Espruino** | Web IDE over Web Bluetooth/USB; REPL; send minified JS to RAM [S] | Large module library; Pico/Puck boards; ESP32 only secondary; Espressif does not support it [S] | Designed for ~8 kB RAM minimum; interpreter slow; project size limited by RAM since code lives in RAM [S] | MPL-2.0 [S]; **acceptable under our policy if recorded** (unlike the GPL/LGPL projects here) | Beloved for the REPL/instant feedback; performance and RAM ceiling limit serious projects |
| **Toit / Jaguar** | `jag` CLI; update and restart ESP32 apps over WiFi on file save; README claims <2 s [V]; VS Code extension and language server [V]; containers installable into firmware [V] | Package registry; own typed language, not JS [V] | VM on ESP32; sample snapshots 142-187 kB [V]; real-time-ish containers [S] | Compiler and VM LGPL-2.1, std libs MIT, examples 0BSD [V]: **LGPL VM is a licence-policy conflict** | Best prior art for live reload over WiFi; not JS so adoption slower [O]. Maintenance status not stated [U] |
| **DeviceScript** (Microsoft Research) | TypeScript compiled to custom bytecode, VS Code extension, simulator, Jacdac hardware modules [S: [Adafruit](https://learn.adafruit.com/devicescript/what-is-devicescript)] | Jacdac ecosystem | ESP32, RP2040 targets [S] | MIT [V] | **Repository archived** (page banner 2025-12-08 [V]; another source said 2025-08-13 [S]); no official reason found [U]. Lesson [O]: research-lab projects need an owner and a stable platform story; same team as MakeCode |
| **Jaculus** | CLI + JS/TS runtime for ESP32; thesis-origin [S] | Small | Duktape-based; Duktape compiled code lives in RAM (ESP32 forum) [S] | ESP32 port GPL-3.0 [S] | Student project; shows demand; GPL |
| **MicroPython / CircuitPython** | REPL over USB; CircuitPython exposes a `CIRCUITPY` USB drive: copy `code.py` and it runs; Mu editor; Adafruit libraries [S: [Adafruit](https://learn.adafruit.com/micropython-basics-what-is-micropython)] | Huge library bundles, Wokwi supports both [S: [Wokwi docs](https://docs.wokwi.com/guides/esp32)] | Interpreter: RAM-hungry but workable on ESP32 [U: no figures] | MIT [U] | Won beginners through zero-toolchain REPL and drag-copy workflow, not speed [O] |
| **Arduino** | One IDE, one button, huge board and library coverage [O, no source fetched] | Wokwi simulates Arduino/ESP32 in browser [S] | Native C++ | LGPL/GPL tooling [U] | Won through one-click build and a massive library ecosystem [O] |
| **Wokwi** | Browser simulator for ESP32, Pico, Arduino with hardware parts; free tier, paid features [S] | Shareable project links | Simulation only | closed/commercial [U] | Defines the expectation of "try without hardware" |

### J.2 Cross-cutting lessons [O, grounded in the rows above]

1. **Time-to-first-blink under 5 minutes, ideally without installing anything** (MakeCode, Wokwi, CircuitPython).
2. **Instant feedback loop**: REPL and save-to-run (Espruino, CircuitPython, Jaguar <2 s).
3. **Browser simulator**: the same core runs in the browser, so OxiUI's wasm32 core can host a simulator for free. Strong, unique advantage.
4. **Libraries are the real moat** (Arduino, Adafruit, MakeCode extensions).
5. **Bring-your-own-JS-ecosystem expectations**: persona expects `npm`, VS Code, TypeScript hover types, Prettier-like formatting, `console.log`.
6. **Licences**: Moddable XS (GPL/LGPL/commercial mix [V]), Toit VM (LGPL [V]), Jaculus (GPL [S]) conflict with our permissive-only policy; they are inspiration, not dependencies.
7. **Research-lab projects stall without an owner** (DeviceScript archive [V]; Mun [S]).
8. **Performance is secondary for onboarding** but ceilings matter for "toy that runs efficiently": STS-style compiled output gives 20x over interpreters [V].

### J.3 Minimum DX at launch for the persona [O]

| Feature | Launch priority | Notes |
|---|---|---|
| `oxi new`, `oxi run` (host or simulator), `oxi flash` (one command, auto-detect port, ESP-IDF/esptool hidden behind it) | Must | Hide toolchains; ship prebuilt runtime firmware for common ESP32 boards |
| **Live reload over USB first, WiFi second**: save -> push `.wasm` (or source) to device, atomic swap (section F.6) | Must | Compile on the *host* (the compiler does not need to fit in an ESP32); device runs wasmi/AOT'd module. Jaguar shows the target latency (<2 s) [V] |
| **Browser simulator** using the OxiUI wasm32 core + wasmi, virtual GPIO/screen/sensors | Must | Share via URL; no hardware needed |
| VS Code extension via LSP (hover types, diagnostics, completions) | Must | Same front-end library |
| **On-device interpreter choice (wasmi vs WAMR) with prebuilt firmware** | Must (decision) | Live reload on a device requires an interpreter on the device; AOT is for release. Prebuilt firmware keeps ESP-IDF off the user's machine. See executive summary Live-engine row |
| REPL on host and over the serial/WiFi link | Should | Needs incremental compile of snippets; wasmi live path suits it |
| Formatter and `oxi test` | Should | |
| Package manager `oxi add`; **npm-compatible?** | Should: **recommend "no for installs, yes for naming/semver"** | npm packages are JS and cannot be used; but the persona knows `package.json` semantics. Use `oxi.toml` or `package.json`-like manifest; registry can mirror npm *names* only. Offer a `ts-to-oxiscript` codemod later. Avoid false promises of "runs any npm package" [O] |
| Blocks editor for kids (MakeCode-style) | Later | Pre-1.0 optional; MakeCode proves value but is a separate product |
| Debugger (breakpoints) | Later | Source maps first |
| Web flasher (WebSerial/WebUSB) | Should | Matches MakeCode's no-install experience |

### J.4 Which DX choices conflict with safety-critical, and how to keep both

| DX choice | Conflict | Resolution |
|---|---|---|
| Live reload/hot code swap on device | Not allowed in certified runtime (code identity, determinism) | Reload is a *host feature* absent from safety profile builds; the safety build is AOT-only and signed |
| Implicit allocation, closures, strings everywhere | Breaks no-alloc, WCET | Profile flags; default profile unaffected |
| REPL/eval on device | Needs compiler on device, dynamic code | Host-side REPL only; no `eval` in the language |
| Auto-install of packages from a registry | Supply-chain risk | Lockfile with hashes, vendoring, offline mode, allow-list in safety profile |
| Friendly integer arithmetic (wrap or promote) | Silent errors | Trap by default (already chosen); explicit wrapping operators |
| `console.log` printf debugging | Allocation/IO in real-time loops | Logging is a capability; compiled out or ring-buffered in profile |
| Panics restart quickly in dev | Not acceptable in flight | Panic policy is a host/profile setting |

Principle [O]: **DX tools live around the compiler (host side), not in the language**, so removing them for a safety build removes no language feature.

---

## 11b. Section K: Single-source heterogeneous computing (CPU / SIMD / multicore / GPU) as a general feature

Owner's frustration: the wall between application language, graphics API, separate shader language and glue. Goal: **data-parallel kernels are ordinary OxiScript functions**, mapped over arrays, images or grids; the compiler/runtime picks GPU (WebGPU/WGSL, SPIR-V), multicore+SIMD CPU, or plain loops on MCUs; control is opt-in. No graphics concept enters the language: graphics is a host module plus kernels.

### K.1 Prior art and lessons

| System | What it is | Lesson |
|---|---|---|
| **CUDA single-source** | C++ with `__global__` kernels in the same file | Won on ergonomics and tooling, but vendor-locked [O, no source fetched] |
| **SYCL / oneAPI** | Standard C++ single-source heterogeneous model | Frequently criticised as complex (queues, buffers/accessors, USM) [U: no source found]; avoid exposing that surface |
| **C++ AMP** | Microsoft's `restrict(amp)` single-source GPU C++ | Headers deprecated from VS 2022 17.0 and now produce build errors [S: [MS docs](https://learn.microsoft.com/en-us/cpp/parallel/amp)]. I found **no stated reason**; the common reading (tied to DirectX, no ecosystem pull) is my opinion [O] |
| **OpenMP offload / OpenACC** | Pragmas that offload loops | Implicit `map` data movement costs: putting `map` inside a loop re-transfers every iteration (Intel guide); mapping errors are a main bug source and unified memory does not remove them [S: [Intel](https://www.intel.com/content/www/us/en/docs/oneapi/optimization-guide-gpu/2024-0/memory-allocation.html), [LLVM thread](https://lists.llvm.org/pipermail/llvm-dev/2021-September/152474.html), [ARBALEST](https://vufind-test.katalog.k.utb.cz/SummonRecord/FETCH-LOGICAL-i203t-660ebd762a801ab27f415865fd3d51f7a1c916333f5b2bdf01e55179efcaa0343)]. **Make residency visible in types** |
| **Taichi** | Python-embedded kernels JIT-compiled to CUDA, Vulkan, Metal, OpenGL ES, multicore CPU; spatially sparse and differentiable features [S: [ROCm docs](https://rocm.docs.amd.com/projects/taichi/en/latest/what-is-taichi.html)]; repo last push 2025-04-30 (activity unclear) [S] | Closest existing model of "kernels as ordinary functions, many backends". Kernels are a restricted subset of Python (typed, no recursion-style dynamism) [O, from general knowledge] |
| **CubeCL (Rust)** | `#[cube]` Rust functions compiled on demand to CUDA, HIP, Metal, SPIR-V, WGSL and CPU SIMD; used by Burn; version 0.x, "alpha" per one summary [S: [docs.rs](https://docs.rs/crate/cubecl)] | The most direct recent proof that one restricted-subset function can target all of them; shows the restricted-subset approach works without a separate shader language. Licence [U] |
| **rust-gpu / rust-cuda** | Rust compiled to SPIR-V / PTX | Needs nightly Rust and pinned compiler versions; rust-gpu's graphics-only focus "did not materialize the necessary contributors"; Embark's repo archived 2025-10-31 and moved to community; rust-cuda is fully volunteer-driven [V/S: [rust-gpu transition](https://rust-gpu.github.io/blog/transition-announcement), [rust-cuda update](https://rust-gpu.github.io/blog/2025/03/18/rust-cuda-update)]. Lesson: compiling a *full* host language to GPU through its existing compiler stack is a maintenance trap; a restricted kernel subset with our own backend is sustainable |
| **Slang** | Shading language with modules, generics, interfaces, built-in autodiff; backends SPIR-V, HLSL, GLSL, WGSL, MSL, plus CUDA/CPU; Khronos-hosted since Nov 2024; spec in progress [S: [Khronos](https://www.khronos.org/news/permalink/khronos-group-launches-slang-initiative-hosting-open-source-compiler-contributed-by-nvidia)] | Shows modules/generics are wanted in shader code and that a spec + open governance matter. Candidate *optional* back end or reference; not a dependency of the core language (licence [U]) |
| **Halide** | Algorithm/schedule separation | Schedules are the right opt-in control surface (section E) [S] |
| **Futhark** | Pure data-parallel functional language to GPU; uniqueness types for in-place updates; sequentialises excess parallelism [S] | Closest on type-system ideas (uniqueness, size types) |
| **ISPC** | SPMD on SIMD; predictable | Retrospective: want explicit vectors beside SPMD [V] |
| **Mojo GPU, Julia KernelAbstractions.jl, Apple Metal C++** | | **Not researched / no results** [U] |

### K.2 Back end infrastructure

- **WebGPU** is now supported in Chrome, Edge, Firefox and Safari (Safari 26), per Google's late-2025 article [S: [web.dev](https://web.dev/blog/webgpu-supported-major-browsers)]; compute pipelines are part of the API [S]. I have no 2026 Safari-specific compute notes [U].
- **naga** (wgpu's translator): inputs SPIR-V, WGSL (fully validated), GLSL (secondary); outputs SPIR-V, Metal, HLSL (primary), WGSL and GLSL (secondary) [V: [README](https://github.com/gfx-rs/wgpu/tree/trunk/naga)]; licence MIT OR Apache-2.0 per the Cargo manifest [S]. wgpu/Dawn are the portable runtimes (Dawn licence [U]).
- **Recommended pipeline [O]:** OxiIR kernel subset -> **WGSL text** (stable, browser-native, one portable target) -> naga for SPIR-V/MSL/HLSL on native; the same OxiIR kernel -> Wasm scalar/SIMD128 loops (CPU/MCU) -> native threads. Do not emit SPIR-V/MSL directly at first. Risk: naga's Rust API is not a stability promise [O]; consuming WGSL text is the safest interface.

### K.3 The `kernel` profile (same mechanism as the safety profile)

A function marked `kernel` (or inferred when used with `map_grid`) must satisfy a profile checked from effect summaries, exactly like `realtime`:

- no recursion, no allocation, no closures that capture heap data, no `dyn`, no host calls, no panics that need unwinding (bounds checks become clamped/trapping per backend rule), bounded or structured loops (loops allowed; divergence reported);
- **no pointers**: access through typed views `View<T>`/`Image<T>`/`Grid<T,N>`; writes only through uniquely-owned `inout` views (uniqueness gives no-alias, Futhark-style [S]); read-only views may alias;
- **uniform vs varying** is inferred, not annotated: values derived from the invocation index are *varying*, everything else *uniform*; the compiler uses it for divergence warnings and for rejecting barriers/derivatives in non-uniform control flow. WGSL itself has uniformity analysis requirements [O, from general knowledge, spec not fetched];
- numeric types limited to the portable intersection: `i32`, `u32`, `f32` (+ `f16` where available), small vectors/matrices; `i64`/`f64` not portable to WebGPU [O, unverified: check WGSL type support] so they are kernel-profile errors on the GPU target and fine on CPU targets, with a lint that tells you *which* target rejects them;
- workgroup/shared memory and atomics via a small standard `kernel` library, not syntax.

### K.4 Memory model and transfer cost

- Types: `Array<T>` (host), `DeviceBuffer<T>` (resident on a device). Transfers are **explicit calls that return `async` values** (`gpu.upload(a)`, `await buf.download()`) because WebGPU read-back is asynchronous anyway [O]; this ties K to the async decision in H item 6.
- Every device decision and transfer appears in the placement report (section L), and `@expect(no_transfer)` can forbid it.
- "Automatic" mode (opt-in per call site and via a `Gpu` capability): the runtime may run a kernel on the GPU only when inputs are already device-resident or small enough to amortise transfer, and it always produces a visible report. Hidden transfers are exactly what OpenMP/OpenACC users got wrong [S]; we make them visible in the types and the report instead.
- On MCUs and wasmi (no `Gpu` capability), a kernel call compiles to a plain nested loop: same semantics, no code change [O].

### K.5 Determinism

GPU floating point is not bit-reproducible across vendors (WGSL itself acknowledges implementation-dependent behaviour [S: [WGSL spec](https://www.w3.org/TR/2026/CRD-WGSL-20260702/)]; I could not retrieve its accuracy rules [U]). Rule [O]: each kernel has a result class: **exact** (integer/fixed-point and fixed-order reductions: identical on every target, enforced by the compiler) or **approximate** (float transcendental, unordered reductions: results may differ by target within a documented tolerance). The `deterministic` and safety profiles allow only exact kernels, run on the CPU reference path.

### K.6 How graphics falls out of the general mechanism

- A *pixel kernel* is `kernel (x: u32, y: u32) -> Rgba8` mapped over an `Image<Rgba8>`; a *particle update* is a kernel over `Array<Particle>`; image filters and game-of-life likewise. None needs a graphics concept in the language.
- Fixed-function rasterisation (vertex/fragment pipelines, depth, blending) is a **host API** (`host:gpu` module) that accepts kernels as stage functions; it is a library, as in F.3.
- OxiUI integration: a display-list item such as `KernelDraw { kernel, region, inputs }`, evaluated on the GPU sink or, on terminal/framebuffer sinks, by the CPU backend of the same kernel. This keeps one source across TUI/web/native, matching the OxiUI plan [O].

### K.7 Minimal design that keeps the core domain-free

1. Language: ordinary functions + profile `kernel` + `View`/`Image`/`Grid` types + `parallel for` / `map_grid` (already Layer 0 in section E.3). Nothing GPU-specific in the grammar except possibly the `kernel` modifier.
2. Standard library: `std.kernel` (thread ids, shared memory, atomics, vector math).
3. Host module: `host:gpu` capability (device, buffers, upload/download, dispatch, optional raster pipeline).
4. Backends: CPU scalar/SIMD/threads first (free from sections E, I); WGSL second; SPIR-V/MSL via naga third. Slang/CubeCL as references, not dependencies.
5. Defer: auto-offload heuristics, autodiff, sparse data structures (Taichi), tensor-core paths, multi-GPU.

Risks: WGSL type limits; driver variance and shader-compiler bugs; async read-back complicates the sequential-semantics rule (the CPU fallback must behave the same including `await` points); GPU backends cannot be exercised on MCU CI; honest scope: this is a research-grade feature set, so it must not block the MVP [O].

---

## 11c. Section L: "Automatic, but never silent" (placement report, editor hints, performance assertions)

Owner's principle: defaults are automatic, but after compiling the compiler says **what it chose, where, and why when the fast path was not taken**. This is a first-class design principle, referenced from sections E, G and K.

### L.1 Prior art and what it teaches

| Tool | What it does | Lesson |
|---|---|---|
| **LLVM optimization remarks** (`-Rpass`, `-Rpass-missed`, `-fsave-optimization-record`, opt-viewer/opt-diff/opt-stats) | Typed remarks (Passed, Missed, Analysis, Failure) with pass, function, debug location, hotness, args; YAML or versioned bitstream; viewer tools [S: [LLVM Remarks](https://llvm.org/docs/Remarks.html)] | Structured, tool-consumable and diffable is the right form. I found no formal stability guarantee of the YAML schema [U]; the string-table YAML variant was removed [S], i.e. formats do change |
| **GCC `-fopt-info`** | `-fopt-info-vec-missed` prints reasons such as "relevant phi not supported", "control flow in loop", "not consecutive access" [S: [gcc-help](https://gcc.gnu.org/pipermail/gcc-help/2023-October/142961.html)] | Reasons are expressed in *compiler-internal* terms users cannot act on; one bug shows a "not vectorized" message although vector code was generated [S]. Reasons must be in source terms and correct |
| **Intel `-qopt-report`, Intel Advisor** | Not retrieved [U] | |
| **Julia `@code_warntype`, JET.jl `@report_opt`, Cthulhu** | Shows inferred types, red for non-concrete types; JET analyses the whole call graph for performance pitfalls; results depend on Julia version; a clean report is not a guarantee [S: [JET docs](https://aviatesk.github.io/JET.jl/dev/optanalysis), [Julia perf tips](https://docs.julialang.org/en/v1/manual/performance-tips)] | Type-level diagnostics beat LLVM-level ones for users; a *language-level* notion ("type-stable") is more durable than backend behaviour. `@code_warntype` does not recurse [S], so whole-program tools are needed |
| **ISPC performance warnings** | Warns on gathers/scatters and slow conversions; silenceable with `#pragma ignore warning(perf)` [S: [SIGGRAPH slides](https://pt.slideshare.net/IntelSoftware/advanced-single-instruction-multiple-data-simd-programming-with-intel-implicit-spmd-program-compiler-siggraph-2019-technical-sessions)] | Works because SPMD makes the cause obvious. Weaknesses reported by users: false positives when a stride becomes a runtime variable (issue #2786) and warnings differing between debug and release builds [S: [issue](https://www.Github.com/ispc/ispc/issues/2786), [thread](https://groups.google.com/g/ispc-users/c/W-I85zA_I0I/m/0O-4R4cHAAAJ)]. Reports must be computed on stable IR facts, not after backend-specific optimisation |
| **Halide** | Prints the loop nest/schedule [U: not fetched; from memory only] | Making the schedule inspectable is the cheap form of transparency |
| **Rust** | No vectorisation or inlining guarantees; even explicit guaranteed tail calls (`become`) are still an incomplete, unstable feature (RFC 3407 open) [S: [RFC 3407](https://github.com/rust-lang/rfcs/pull/3407), [docs](https://doc.rust-lang.org/stable/core/keyword.become.html)] | Promising an optimisation is a language-level commitment that is hard to implement across backends: **only assert what the language itself defines and our own lowering implements** |
| **Zig comptime, GHC Core dumps, Mojo/MLIR, .NET JIT disasm, JetBrains/Visual Studio hints** | Not researched [U] | |

### L.2 What makes reports useful or ignored [O, derived from the rows above]

1. **Actionable reasons in source terms** ("loop-carried dependence on `acc` at line 12; fix: use `reduce`") instead of pass-internal wording.
2. **Low noise**: report only declined fast paths in code that asked for or implies parallelism (`parallel for`, `kernel`, hot paths), not every scalar loop.
3. **Same answer in debug and release**: decisions come from the IR facts, not from backend optimisation level.
4. **Stable identifiers and schema** so tools, diffs and assertions do not break on harmless upgrades.
5. **No false promises**: say "not vectorised on this target" separately from "cannot be vectorised by language rules".

### L.3 Design

1. **Decision log from the first optimisation pass.** Every pass that makes or declines a placement decision must emit it through one `remarks` API (unit, source span, decision, stable reason code, suggested fix). Retrofitting is the hard part: LLVM remarks are per pass opt-in [S]. This is the decision that has to be made early.
2. **Two classes of decisions** (key to stability):
   - **Semantic facts**, defined in the language spec and computed by the type/effect system: *data-parallel* (iterations independent), *allocation-free*, *bounded*, *kernel-eligible*, *no-transfer*. Stable across compiler versions within an edition; assertable.
   - **Heuristic choices**: SIMD width, chunk size, CPU vs GPU for a given size. Reported with the reason, but not assertable except through capability bounds (`min_width`).
3. **Build report**: `oxi build` always writes `placement.json` (versioned schema, stable reason codes such as `P0102`) and prints a short human summary: per function/loop/kernel call: scalar / SIMD(N) / multicore(chunking) / GPU; **transfers** (bytes, direction, per call or per frame); **allocations in hot paths**; budget-check sites; and for every declined fast path the reason and fix. `--report=full` lists everything.
4. **Editor**: the LSP serves the same data as inlay hints and code lenses next to the code (for example `simd4`, `scalar: loop-carried dependence`, `3.1 MB upload/frame`, `alloc`), hover shows the reason, quick fixes apply the suggested change.
5. **Performance assertions** as attributes: `@expect(simd)`, `@expect(parallel)`, `@expect(gpu)`, `@expect(no_alloc)`, `@expect(no_transfer)`, `@expect(bounded)`. A failed assertion is a **compile error that prints the same reason as the report**. They are evaluated per target profile (`@expect(simd, on: "browser")`), and use exactly the profile machinery of sections I and K (effect summaries), so a performance assertion and a safety restriction are the same kind of object.
6. **Certification use**: assertions plus the archived report give traceable evidence (requirement -> assertion -> compiler-checked fact) for `no_alloc`, `bounded` and `no_panic` claims. This is my opinion on how to use it; whether an assessor accepts it must be confirmed [U].
7. **Compatibility policy**: semantic facts only get *stronger* within an edition (something provable stays provable); weakening needs an edition bump. Reason-code IDs are never reused. CI keeps golden reports for the example corpus and reviews diffs (opt-diff style [S]).
8. **Suppression requires a reason string** (`@allow(P0102, "tiny N, not worth it")`), unlike ISPC's bare pragma, so suppressions remain reviewable.

### L.4 Cost

Small if designed in at the start (single remark API, source spans carried through OxiIR, stable codes); large if retrofitted. Maintaining reason quality is ongoing work [P: perhaps 10% of compiler effort in the performance phases].

---

## 12. Risks

| Risk | Likelihood / impact [O] | Mitigation |
|---|---|---|
| Scope: sound TS-like language + compiler + LSP + profiles is a multi-year effort; Mun and Roto show how long small teams take [V/S] | High / high | Strict MVP (section 14); reuse Binaryen/wasmi/w2c2; defer profiles' proofs |
| Memory model choice (RC vs tracing) proves wrong | Medium / high | Phase-0 prototype on both; keep OxiIR ownership-explicit so backends can swap |
| wasmi translation latency or footprint worse than hoped (unmeasured [U]) | Medium / medium | Measure in week 1; fallback is own bytecode interpreter or wasm→native on desktop only |
| wasmi has no threads/GC/EH; Wasm proposals shift | Medium / low | Core-Wasm-only policy; CI engine matrix |
| wasm-in-wasm in the browser is slow for heavy scripts | Medium / medium | Release path uses browser engine; live path only for dev/UI handlers |
| SIMD/parallel semantics promise leaks (programs depend on timing) | Medium / medium | Race freedom by construction; sequential-equivalence tests in CI (run every program sequentially and in parallel and diff) |
| "Feels like TS" expectations broken by dropping features | High / medium | Clear migration guide, codemods, error messages that explain the TS difference |
| Licence taint (MoonBit unverified, LGPL/GPL in Toit/Moddable/Jaculus) | Low / high | Permissive-only dependency policy; audit of every crate/tool; record MPL exceptions |
| Safety claim overreach | Medium / high | Never claim certification; claim "designed to be qualifiable"; engage an assessor early |
| Compiler size in the core (wasm32 build) bloats the browser/embedded core | Medium / medium | Feature-gate the compiler; precompiled modules for MCU builds |

## 13. Open questions for the owner

1. **Memory model**: accept a 2-3 week prototype to compare RC-with-elision and tracing GC before freezing the design?
2. **Syntax**: real TypeScript parsing (large, tsc-compat pressure) versus TS-looking own grammar (recommended)?
3. **Numbers**: default `int` width i32? Default overflow = trap (recommended)?
4. **Strings**: UTF-8 with no O(1) code-point index acceptable for the JS-developer persona?
5. **MCU promise at v1**: "same compiler, MCU via wasm->C" or "live on-device interpreter as well" (wasmi flash is the largest in treVM [V])?
6. **Which MCU class first**: ESP32 with PSRAM only (matches Moddable/Toit class) or also 64-256 kB class (STS class)? It decides the interpreter and RC/GC choice.
7. **First safety target** (DO-178C, IEC 61508, ISO 26262, IEC 62443): affects spec/evidence design.
8. **npm positioning**: naming/semver compatibility only (recommended) or attempt TS-to-OxiScript source compatibility?
9. **Blocks editor**: in scope for education positioning?
10. **Team and funding**: compiler front end is the critical path; who owns it long term (DeviceScript/Mun lessons)?
11. **Hot-reload scope**: do you accept "module-local state resets on reload; persistent state through host StateManager"?
12. **Own bytecode fallback**: keep as plan B?
13. **On-device engine** for ESP32 dev mode: wasmi (fast, big flash) or WAMR (small, official Espressif component, RAM risk)? Needs a 2-week bake-off on a real board.

## 14. Proposed phased roadmap (all sizes are [P] estimates with wide uncertainty, +-2x)

Assumes 2 experienced compiler engineers; scale linearly for other staffing (not a promise). The primary persona (JS/TS developer, kid, ESP32) gets a **thin device slice inside Phase 1**, so that the first usable release already blinks an LED on a real board.

| Phase | Content | Size [P] |
|---|---|---|
| **0. Spike and decisions** | Spec skeleton (core semantics, no UB, evaluation order), decisions record; tiny front end (ints, floats, structs, functions, closures, arrays) -> Wasm -> wasmi; **measure**: compile+translate time, RC vs mark-sweep, wasmi vs browser wasm-in-wasm; **2-week bake-off wasmi vs WAMR on a real ESP32 board** (flash, RAM, speed, reload time); remarks API stub | 8-12 weeks |
| **1. MVP language, live mode and thin ESP32 slice** | Sound type system (generics, unions, null, Result, modules), RC with elision, host IDL + Rust derive, OxiUI host module, LSP, formatter, `oxi run`, hot reload with host-managed state, **browser simulator** on the wasm32 core; **prebuilt interpreter firmware for one ESP32 board, `oxi flash`, USB push of compiled `.wasm` with atomic swap** (compile on host); allocation report + inlay hints (first slice of section L) | 9-14 months (grew by ~3-5 months for the device slice) |
| **2. Release, more boards, profiles v0** | AOT: Wasmtime/Cranelift (64-bit) and w2c2 + C compiler (32-bit/ESP32/MCU), WiFi live reload, more boards, size/RAM report, budget-check instrumentation, profiles v0 (`no_alloc`, `no_recursion`, `bounded_loops`), web flasher | 4-6 months |
| **3. Performance and transparency** | `Simd<T,N>`, data-parallel combinators with sequential semantics, native thread pool, worker backend, schedule annotations, **placement report for SIMD/parallel, `@expect` assertions, stable reason codes** | 4-6 months |
| **3b. Kernels / GPU (research-grade, optional)** | `kernel` profile, `View`/`Image`/`Grid`, `DeviceBuffer`, WGSL back end via WebGPU, then SPIR-V/MSL via naga; transfer reporting and `@expect(no_transfer)` | 4-8 months, high uncertainty |
| **4. Concurrency and ecosystem** | async/tasks/isolates, package manager + registry, test runner, debugger/source maps, docs | 4-6 months |
| **5. Safety profile (deferred)** | Executable spec/reference interpreter, contracts checking, deterministic libm, deterministic profile, SBOM/signing, qualification plan with an assessor; verification back end optional | 12+ months, uncertain |

Not in scope initially: Component Model, WasmGC backend, own native backend, blocks editor, refinement types, auto-offload heuristics.

## 15. References

Verified [V] unless marked [S]. Only items actually cited above are listed.

**Language prior art**
- Static TypeScript paper: https://www.microsoft.com/en-us/research/uploads/prod/2019/09/static-typescript-draft2.pdf ; blog https://www.microsoft.com/en-us/research/blog/rocket-fast-embedded-typescript-for-makecode-arcade/ ; MakeCode languages https://makecode.com/language [S]
- AssemblyScript status https://www.assemblyscript.org/status.html ; closures issue https://github.com/AssemblyScript/assemblyscript/issues/173 [S]
- Static Hermes talk https://speakerdeck.com/tmikov2023/static-hermes-react-native-eu-2023-announcement [S]
- Typed Racket: https://users.cs.northwestern.edu/~dmf082/papers/popl16-tfgnvf.pdf [S]; mypyc https://mypyc.readthedocs.io/en/latest/differences_from_python.html [S]; Sorbet https://sorbet.org/docs/runtime [S]
- Dart null safety https://dart.dev/null-safety/understanding-null-safety ; Dart hot reload https://dart.googlesource.com/sdk/+/stable/runtime/docs/hot-reload.md
- Kotlin/Native https://kotlinlang.org/docs/native-migration-guide.html , https://kotlinlang.org/docs/native-memory-manager.html
- Zig async: https://git.jakstys.lt/motiejus/zig/src/commit/9f0b9b8da1a111d16eb8d1254212ff98a8b4be08 [S], https://lwn.net/Articles/1049006/ [S]
- Nim memory management https://nim-lang.org/docs/mm.html [S]; Roc rewrite https://gist.github.com/rtfeldman/77fb430ee57b42f5f2ca973a3992532f [S]
- Embedded Swift https://forums.swift.org/t/embedded-swift/67057 [S]; Swift 6 migration https://forums.swift.org/t/my-experience-attempting-a-migration-to-swift-6/79069 [S]
- TinyGo https://www.infoq.com/news/2020/04/tinygo-go-iot-microcontrollers/ [S]
- Mun https://github.com/mun-lang/mun ; https://opencollective.com/mun/updates/mun-v0-3-0-released [S]
- Roto https://blog.nlnetlabs.nl/one-year-of-roto-the-compiled-scripting-language-for-rust ; script-bench-rs https://github.com/khvzak/script-bench-rs [S]
- Luau https://luau.org/ [S]; Wasmnizer-ts https://github.com/web-devkits/Wasmnizer-ts [S]; Porffor https://github.com/CanadaHonk/porffor [S]; MoonBit https://moonbitlang.com/blog/first-announce/ [S]
- TypeScript design goals https://github.com/microsoft/TypeScript/wiki/TypeScript-Design-Goals [S]; TS native port https://www.infoq.com/news/2025/05/new-typescript-compiler-10x-fast [S]
- Mojo open source https://modular.com/blog/mojo-open-source [S]

**Memory**
- Perceus https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/
- Hylo/MVS https://arxiv.org/pdf/2106.12678 [S]; Verona https://www.microsoft.com/en-us/research/publication/reference-capabilities-for-flexible-memory-management/ [S]
- V8 WasmGC https://v8.dev/blog/wasm-gc-porting ; Wasm 3.0 https://webassembly.org/news/2025-09-17-wasm-3.0/

**Engines**
- wasmi README https://docs.rs/crate/wasmi/latest/source/README.md ; v0.32 https://wasmi-labs.github.io/blog/posts/wasmi-v0.32 ; benchmarks https://github.com/wasmi-labs/wasmi-benchmarks
- Wasmtime platform https://docs.wasmtime.dev/stability-platform-support.html ; proposals https://docs.wasmtime.dev/stability-wasm-proposals.html ; deterministic execution https://docs.wasmtime.dev/examples-deterministic-wasm-execution.html [S]
- WAMR https://github.com/bytecodealliance/wasm-micro-runtime ; w2c2 https://github.com/turbolent/w2c2
- treVM (MCU benchmark) https://arxiv.org/html/2604.27570v1 ; Wasm on IoT (Has et al.) https://arxiv.org/abs/2512.00035v1 (no figures in text)
- Wasefire https://opensource.googleblog.com/2025/11/secure-by-design-firmware-development-with-wasefire.html [S]; wasm to C https://00f.net/2026/07/08/webassembly-compilation-to-c-2026/ [S]; Wastrel https://wingolog.org/archives/2025/10/30/wastrel-a-profligate-implementation-of-webassembly [S]
- TPDE https://arxiv.org/pdf/2505.22610 [S]; WASI 0.3 https://bytecodealliance.org/articles/WASI-0.3 [S]
- SharedArrayBuffer requirements https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer

**SIMD / parallel**
- Pharr ispc https://pharr.org/matt/blog/2018/04/18/ispc-origins , https://pharr.org/matt/blog/2018/04/29/ispc-retrospective
- Maleki et al. https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers [S]; TSVC2 study https://arxiv.org/html/2502.11906v1 [S]
- Halide https://people.csail.mit.edu/jrk/halide-pldi13.pdf [S]; Futhark https://www.futhark-lang.org/publications/pldi17.pdf [S]; Rayon https://docs.rs/crate/rayon/latest/source/src/./lib.rs [S]

**Safety and certification**
- Ravenscar https://en.wikipedia.com/wiki/Ravenscar_profile [S]; SPARK slides https://secdev.ieee.org/wp-content/uploads/2019/09/SPARK-Tutorial-Slides.pdf [S]
- RTSJ/SCJ https://arxiv.org/pdf/1805.10710 [S]
- WCET: https://www.absint.com/a3/gallery.htm [S]; https://ls12-www.cs.tu-dortmund.de/daes/en/research/wcet-aware-compilation/motivation/timing-analysis.html [S]
- Liquid Haskell https://ucsd-progsys.github.io/liquidhaskell-blog/2019/10/20/why-types.lhs [S]; https://arxiv.org/pdf/2311.13995 [S]
- Ferrocene https://ferrous-systems.com/blog/officially-qualified-ferrocene/ [S]; https://secure.businesswire.com/news/home/20251203818102/en/Ferrous-Systems-Achieves-IEC-61508-SIL-2-Certification-for-Rust-Core-Library-Subset [S]
- DO-330 https://rapitasystems.com/do-330 [S]; CRA reporting https://www.skadden.com/insights/publications/2026/09/get-ready-reporting-faqs-and-checklist [S]
- Go 1 compatibility https://go.dev/doc/go1compat [not fetched]

**Developer experience**
- Moddable https://github.com/Moddable-OpenSource/moddable ; https://moddable.com/faq [S]
- Toit https://github.com/toitlang/toit ; Jaguar https://github.com/toitlang/jaguar [S]
- DeviceScript https://github.com/microsoft/devicescript ; https://learn.adafruit.com/devicescript/what-is-devicescript [S]
- Jaculus https://github.com/C2Coder/Jaculus-esp32 [S]; Espruino https://www.espruino.com/FAQ?print [S]
- MicroPython/CircuitPython https://learn.adafruit.com/micropython-basics-what-is-micropython [S]; Wokwi https://docs.wokwi.com/guides/esp32 [S]

**Heterogeneous compute (section K)**
- Taichi https://rocm.docs.amd.com/projects/taichi/en/latest/what-is-taichi.html [S]; CubeCL https://docs.rs/crate/cubecl [S]
- rust-gpu transition https://rust-gpu.github.io/blog/transition-announcement ; rust-cuda https://rust-gpu.github.io/blog/2025/03/18/rust-cuda-update [S]
- Slang/Khronos https://www.khronos.org/news/permalink/khronos-group-launches-slang-initiative-hosting-open-source-compiler-contributed-by-nvidia [S]
- naga https://github.com/gfx-rs/wgpu/tree/trunk/naga ; WebGPU support https://web.dev/blog/webgpu-supported-major-browsers [S]; WGSL spec https://www.w3.org/TR/2026/CRD-WGSL-20260702/ [S]
- C++ AMP deprecation https://learn.microsoft.com/en-us/cpp/parallel/amp [S]; OpenMP offload data movement https://www.intel.com/content/www/us/en/docs/oneapi/optimization-guide-gpu/2024-0/memory-allocation.html [S], https://lists.llvm.org/pipermail/llvm-dev/2021-September/152474.html [S]

**Transparency (section L)**
- LLVM remarks https://llvm.org/docs/Remarks.html [S]; GCC missed-vectorization messages https://gcc.gnu.org/pipermail/gcc-help/2023-October/142961.html [S]
- JET.jl https://aviatesk.github.io/JET.jl/dev/optanalysis [S]; Julia performance tips https://docs.julialang.org/en/v1/manual/performance-tips [S]
- ISPC warnings: https://www.Github.com/ispc/ispc/issues/2786 [S], https://groups.google.com/g/ispc-users/c/W-I85zA_I0I/m/0O-4R4cHAAAJ [S]
- Rust explicit tail calls https://github.com/rust-lang/rfcs/pull/3407 [S]

**Licence checks done this session:** Wasmtime "Apache-2.0" (repo page; LLVM exception not mentioned there) https://github.com/bytecodealliance/wasmtime ; Binaryen "Apache-2.0" https://github.com/WebAssembly/binaryen

**Sources deliberately not relied on:** personal-notes sites (tamnd.github.io), promotional DEV/Medium posts, the wasmz benchmark page (its 7 MB wasmi binary is likely a default-feature std build, not a no_std footprint).
