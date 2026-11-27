# OxiScript roadmap

Single roadmap of OxiScript: the language, checker, IR, code generation, runtime and memory, standard library, DevKit (CLI, LSP, VS Code), laws and reports, Rust crate linking, targets and profiles, SIMD/GPU, concurrency, safety, the course and the OxiScript websites content. It replaces the OxiScript parts of `docs/history/oxi-ui-plan-history.md` (Phase 8), `docs/HANDOFF.md`, the planning text of `docs/stages/**` and the "next steps" of the research documents. The OxiUI half is [oxiui.md](oxiui.md); the index and the id scheme are in [README.md](README.md); contradictions found while merging and the list of documents to rewrite or delete are in [consistency.md](consistency.md).

Written 2026-11-26 against `main` at 19cf8cd (plus the three unmerged Gemini branches named in section 4). Every status in sections 3 and 4 was checked against git or code; statuses in section 5 are marked `verified-absent`, `partial` or `todo` with the check named. Source ids like `[R3-216]` point to the reader inventories (R1 concept docs, R2 plan and stage notes, R3 OxiScript specs and research, R4 SIMD/GPU and second reading of the plan); they are kept so that nothing is lost and each can be traced (see the coverage ledger, section 9).

## 1. Vision and positioning

OxiScript is a general-purpose, statically and soundly typed language that a TypeScript developer reads without learning it, compiled to Core WebAssembly. One program runs two ways: live (compiled in milliseconds inside the core, hot reload) and compiled for release (native AOT on 64-bit hosts, wasm to C for 32-bit boards, the browser's own engine on the web). OxiUI is its first host, not its limit: the owner's test case is that someone can later write safety-critical software (a rocket) in it. The language has no domain concepts; UI, real time, safety and GPU are profiles, libraries and host APIs ([p-1](#p-1), [p-3](#p-3)).

Positioning (owner 2026-11-26, [d-20](#d-20)): GUI and scripting for devices with megabytes, not gigabytes, of memory (industrial panels, kiosks, home automation, old PCs, toys on ESP32), next to the server-streamed role of OxiUI. The claim goes on the sites only with measurements ([8.Mem](#s-8-mem)). Primary persona: a JS/TS frontend or Node developer who builds a toy with a child on an ESP32: one-command flash, live reload, browser simulator, VS Code ([8.S3](#s-8-s3), [8.S2](#s-8-s2), [8.D1](#s-8-d1)). The second persona is an AI agent: unambiguous grammar, stable reason codes with fixes, `llms.txt`, a deterministic compiler ([d-12](#d-12)).

The two products meet in one place, the stage [8.S1](#s-8-s1) (`<script lang="oxs">` in OxiUI pages), which is the end of the critical path and the earliest moment the repository may be split ([d-7](#d-7), [8.Repo](#s-8-repo)).

## 2. Principles and owner decisions

Numbering scheme (one, used by both roadmaps for OxiScript references): principles `P-1..P-11` and owner decisions `D-1..D-13` are numbered exactly as in the old `docs/history/oxi-ui-plan-history.md` Phase 8 text; later owner decisions continue as `D-14...`. Anchors are `#p-N` and `#d-N`. In the old documents the word "decision 4" was used for two different things: the SIMD/GPU research ("decision 4 transparency", "revised for decision 4") means principle 4 = [P-4](#p-4) (data parallelism is general, the GPU is a backend, residency visible in types), while the old decision 4 = [D-4](#d-4) is "GPU in v1: no". Mapping for readers of old text: `decision 4 (transparency)` -> `P-4`; `decision 4 (GPU in v1)` -> `D-4`; `decision 1 / Stage 8.0 measures it` -> `D-1` + `8.Perf` ([consistency.md](consistency.md)).

The texts below are the owner's, carried over word for word (2026-11-24 unless a date is shown). Where a later document or the code disagrees, the note under the text says so; the text itself is not changed.

### 2.1 Owner goals (2026-11-24)

- **Feels like TypeScript, runs like a compiled language.** TS-like syntax, inference and editor support, but statically and soundly typed.
- **Two ways to run one program:** "write and run" live (compile in milliseconds inside the core, hot reload) and compiled for release, natively, down to microcontrollers.
- **Primary persona:** a JS/TS frontend or Node developer who builds a toy with their kid on an ESP32 must feel at home: one-command flash, live reload, browser simulator, VS Code.
- **No artificial limits.** OxiUI is only the first host. The language must stay general enough that someone could later build safety-critical software (the owner's test case: a rocket) on it.
- **Modern hardware without assembler knowledge:** SIMD/SWAR, multi-core and GPU are used automatically by default, with explicit control for those who want it.
- **No wall between the language and graphics/GPU:** data-parallel code is written as ordinary functions, not in a separate shader language glued on through an API.
- **Automatic, but never silent:** after every build the compiler says what it placed where and why, so nothing slows down by surprise.

### 2.2 Principles (decided 2026-11-24)

<a id="p-1"></a>**P-1.** **The core language has no domain concepts.** UI, real time, safety and GPU are profiles, libraries and host APIs.

<a id="p-2"></a>**P-2.** **Sound static types.** No `any`, no unchecked casts, explicit numeric types (integers distinct from floats, overflow traps by default), one absence value (`T | null`), `Result` with `?` instead of exceptions, UTF-8 strings, discriminated unions with narrowing. Bidirectional local inference keeps it light to write.

<a id="p-3"></a>**P-3.** **One language, compiler-checked profiles, never dialects** (the SPARK/Ravenscar principle). Profiles such as `realtime`, `safety` and `kernel` are restriction sets the compiler checks (no allocation, no recursion, bounded loops, no ambient authority, ...), with the same semantics as unrestricted code.

<a id="p-4"></a>**P-4.** **Data parallelism is general, GPU is a backend.** Parallel and SIMD constructs have the semantics of sequential code. Layers: (0) automatic data-parallel combinators over arrays/images/grids; (1) explicit `Simd<T, N>` types; (2) opt-in schedules, tasks and isolates. `kernel` functions are ordinary functions under the `kernel` profile; backends in order: CPU SIMD + threads, then WGSL/WebGPU, then SPIR-V/MSL via naga. Device residency and CPU↔GPU transfers are visible in types (hidden transfers killed C++ AMP). Graphics (pixel/vertex shaders, a "paint with this kernel" display-list item) is a library on top.

<a id="p-5"></a>**P-5.** **Placement report and performance assertions.** Every build writes a versioned, human-readable report: what went to GPU, multi-core, which SIMD width, scalar, transfers per call, allocations on hot paths, with a stable reason code whenever the fast path was not taken. The LSP shows it as inlay hints next to the code. `@expect(simd | gpu | no_alloc | no_transfer | ...)` turns a silent de-optimisation into a compile error; only language-defined facts are assertable, heuristics are reported but not assertable (so compiler upgrades do not break builds). The same assertions serve as certification evidence.

<a id="p-6"></a>**P-6.** **Capabilities, not ambient authority.** A program sees only the host APIs it is given (no files, network or clock unless passed in), declared in a small own IDL that compiles to plain Wasm imports. The WASM Component Model can be an adapter later.

<a id="p-7"></a>**P-7.** **Own typed SSA IR, Core WebAssembly as the first backend and distribution format.** No dependency on Wasm proposals that some target engine lacks (WasmGC, threads, exception handling: wasmi lacks them; AssemblyScript was hurt by this). Live mode runs the module in an embedded interpreter (wasmi in the core; on the device wasmi or WAMR, see open questions). Release: Wasmtime AOT on 64-bit hosts, wasm→C (w2c2, MIT) plus the target's C compiler for 32-bit boards, ESP32 and the likely safety route, the browser's own engine on the web.

<a id="p-8"></a>**P-8.** **Hot reload by instance swap.** A reload builds a new instance and swaps atomically; module-local state resets, persistent state lives in host-managed, schema-versioned state (OxiUI's StateManager), following Dart's rules.

<a id="p-9"></a>**P-9.** **Determinism and a written spec from day one:** no undefined behaviour, defined overflow and float behaviour across targets, reproducible compiler output. These are what make qualification (DO-178C, IEC 61508, ISO 26262, IEC 62443) possible later.

<a id="p-10"></a>**P-10.** **Tooling is part of the language:** one query-based front end in Rust (also built for wasm32, because the compiler ships inside the core) shared by the compiler, LSP and formatter; LSP and formatter at launch; good error messages; a `package.json`-like manifest without promising that npm packages run. The device tooling (flash, live reload over USB then WiFi, browser simulator on the wasm32 core, host-side REPL) lives around the compiler, so a safety build can drop it without losing language features.

<a id="p-11"></a>**P-11.** **Permissive licences only** for every component (the on-device runtime included). Moddable XS, the Toit VM and Jaculus are GPL/LGPL, so they are references, not dependencies.


Notes (no change of meaning; conflicts go to the lists named):
- P-4 says backends in order "CPU SIMD + threads, then WGSL/WebGPU, then SPIR-V/MSL via naga". The verified SIMD/GPU report (2026-11-26) emits WGSL text only and uses naga as a CI validator, not as a backend; threads are stage [8.4](#s-8-4). Owner to confirm the SPIR-V/MSL tail: [Q-SIMD-5](#q-simd) and [consistency.md](consistency.md) (R4 inconsistency 9).
- P-7 says "on the device wasmi or WAMR, see open questions". [D-3](#d-3) settles the live engine as wasmi; the release engine for ESP32 (wasmi interpreter or w2c2 AOT) is still measured in [8.Mem](#s-8-mem) and [8.0](#s-8-0). There is no longer an "open questions" section in the old plan; the residue is in section 7.
- P-10 says "one query-based front end". The code has separate `oxiscript-syntax` and `oxiscript-check` crates, no Salsa-style incremental queries (whole file re-checked, about 100k lines/s); both build for wasm32. Incremental queries are decided with the LSP in [8.S2](#s-8-s2) [R3-100, R3-145].
- **Exception to P-2 (owner, 2026-11-27, [D-37](#d-37)): implicit lossless widening.** "Explicit numeric types, types never mix implicitly" stays the rule, with one exception: a value may be converted implicitly when the conversion is **exact for every value of the source type**, and only at an **assignment, a call argument or a `return`** (never inside arithmetic or comparison operators, so no C-style promotion). Allowed pairs: a signed integer to a wider signed integer (`i8→i16→i32→i64`); an unsigned integer to a wider unsigned integer (`u8→u16→u32→u64`) or to a strictly wider signed one (`u8→i16`, `u16→i32`, `u32→i64`); `f32→f64`; integers to floats only where every value is representable (`i8, i16, u8, u16 → f32`; `i8, i16, i32, u8, u16, u32 → f64`). Everything else (narrowing, signed to unsigned, `i32→f32`, `i64/u64→f64`, float to integer) still needs `as`. Literals keep their existing context typing. Planned in [8.Fix](#s-8-fix); `AGENTS.md`, `llms.txt`, the skill and `grammar.md` section 6 change when it lands, not before.
- P-5 (placement report, `@expect`) is only partly built: reason codes, `P` class registry and spans exist; no report is emitted ([8.S4](#s-8-s4), [8.2c](#s-8-2c)).

### 2.3 Owner decisions 1-13 (2026-11-24, D-13: 2026-11-25; D-7 revised 2026-11-26)


- <a id="d-1"></a>**D-1.** **Memory model:** reference counting with compiler elision (Perceus-style) plus a cycle collector (Nim ORC-style). GC-like convenience for closures in event handlers, mostly deterministic freeing; the `realtime`/`safety` profiles forbid allocation on hot paths. Stage 8.0 measures it (counter overhead, atomics across threads mitigated by per-isolate data, code size on ESP32) but does not reopen the choice.
- <a id="d-2"></a>**D-2.** **Grammar:** our own grammar, files `.oxs`, designed so that a TypeScript developer simply *reads* it without learning it (same keywords, braces, arrow functions, `import`/`export`, type annotations after `:`; differences only where soundness requires them, and then with familiar spellings). Real TypeScript is not parsed (no accepting syntax we then reject).
- <a id="d-3"></a>**D-3.** **On-device live engine:** wasmi (pure Rust like the core, fastest interpreter in the treVM measurements). Its larger flash footprint is accepted. *(Superseded for microcontrollers by [D-27](#d-27), 2026-11-26: w2c2 native builds on MCUs; wasmi stays on servers, in the browser and the simulator.)*
- <a id="d-4"></a>**D-4.** **GPU in v1:** no, and the `kernel` profile is not in the MVP either. But the language must be designed so that adding it later is easy, not hard: no feature in the MVP may make data-parallel kernels or device buffers awkward to add (the GPU backend comes in 8.3d).
- <a id="d-5"></a>**D-5.** **Tools in v1:** web flasher (WebSerial) yes; blocks editor later.
- <a id="d-6"></a>**D-6.** **First standards target:** IEC 62443 and the EU CRA (the business model); IEC 61508 as the first functional-safety standard, later.
- <a id="d-7"></a>**D-7.** **Ownership and method:** the OxiUI project owns the compiler, in this repository (`oxiscript-*` crates) **for now** (*revised by the owner 2026-11-26: OxiScript, and possibly the websites, will move to their own repositories, preserving history (`git filter-repo`), probably under a shared GitHub organisation. **The earliest possible time is after 8.S1**, when the OxiUI-OxiScript interface (`<script lang="oxs">`) has settled; it is not scheduled right after 8.S1 and happens only when the owner decides. Until then: keep the two halves separable inside this repo (no new `oxiscript-*` -> `oxiterm-*` dependency except the 8.S1 host interface, separate CI workflows, changelogs and docs directories, separate release tags `oxiscript-v*` / `v*`)*), with the same workflow as the rest (owner decides, agents implement, lead reviews). **After every finished stage a BAD ACTOR agent runs** whose only goal is to break it: escape the sandbox, exhaust resources, crash the compiler or runtime, make it miscompile, defeat the type system. Fuzzing (compiler front end, IR, Wasm output, runtime host API), differential testing (interpreter vs AOT) and adversarial programs are part of admitting a commit, not an afterthought; whatever the bad actor finds becomes a regression test.
- <a id="d-8"></a>**D-8.** **npm:** the idea, not compatibility: naming and semver conventions and a `package.json`-like manifest; npm packages are not promised to run.
- <a id="d-9"></a>**D-9.** **Hot reload state:** module-local state resets on reload; persistent state lives in the host's StateManager.
- <a id="d-10"></a>**D-10.** **No plan B bytecode:** WebAssembly is the only IR backend and distribution format.
- <a id="d-11"></a>**D-11.** **Laws (inspired by Bend 2's `LAWS.bend`, without its dependent types).** A `law` is a first-class declaration of a property the code must satisfy, written in ordinary OxiScript and readable by a TS developer:
    ```ts
    law sorted_stays_sorted(xs: i32[]) {
      isSorted(sort(xs))
    }
    ```
    One syntax, three levels of checking:
    1. **Tested (default, every build in dev):** the compiler generates many inputs from the parameter types and searches for a counterexample (QuickCheck/Hypothesis-style property testing, shrinking the failing case). Cheap and immediate; also the BAD ACTOR agent's fuel.
    2. **Proven automatically (`safety` profile):** an SMT solver (Z3, MIT, or cvc5, BSD) tries to prove the law without hand-written proofs, as SPARK, Dafny and Verus do.
    3. **Proofs written by AI or a human, only when the solver cannot:** lemmas and hints are added next to the law. In the `safety` profile an unproven law is a compile error (the Bend 2 mechanism, as the last resort rather than the norm).

    Lesson from Bend 2 kept in mind: laws only protect what is written down (an under-specified law let its demo agent change the game rules), so law review is part of code review. Full dependent types are rejected: they would break "a TS developer simply reads it" and slow compilation.
- <a id="d-12"></a>**D-12.** **AI-friendliness as a design goal:** an unambiguous grammar; error messages with stable reason codes and a suggested fix, readable by people and models; a stable specification with model-oriented docs (`llms.txt`); the placement report and `@expect`; laws as machine-checkable targets for agents; a deterministic compiler, so an agent can rely on its own experiments.

- <a id="d-13"></a>**D-13.** **Pointer width is a target parameter (owner question, 2026-11-25).** v1 targets wasm32: 32-bit addresses and lengths in the ABI (4 GiB of linear memory per module), while `i64`/`f64` arithmetic is native 64-bit. Release builds are still native 64-bit machine code on 64-bit hosts (Wasmtime AOT, or wasm→C then the native compiler); only the per-module heap is 32-bit addressed, which is also what ESP32-class 32-bit MCUs need. Programs that need more than 4 GiB get a later `memory64` target (part of Wasm 3.0): pointer width stays a codegen/ABI parameter, never a deep assumption, and the switch is an ABI version bump, not a language change.


Notes:
- D-1 says "Stage 8.0 measures it". Stage 8.0 was never run. Its "RC overhead with elision" part now belongs to [8.Perf](#s-8-perf) (plan commit 120ac39); the other measurements stay in [8.0](#s-8-0). D-1 itself is not reopened. Evidence that the cost matters: the 8.M-link BAD ACTOR measured 6.5-10.5x fuel versus the bump stub (BL06); the founding research warned RC without elision is about 2x slower than GC in measurements on STS [R3-103].
- D-5: the web flasher is owned by [8.S3w](#s-8-s3w); the blocks editor stays "later" ([Ideas](#s-ideas)).
- D-7 (revision 2026-11-26): the split is stage [8.Repo](#s-8-repo); the version/tag half is in [8.Rel](#s-8-rel); OxiTerm/OxiUI versioning and `v*` tags are owned by [oxiui.md](oxiui.md).
- D-2 and D-10 are implemented (own grammar, WebAssembly the only IR backend). D-9 is implemented in the runtime (instance swap) and `host:state`; the OxiUI side is [8.S1](#s-8-s1).

### 2.4 Owner decisions recorded after the old list (D-14 onward)

Each is stated in the plan rows, specs or commit messages of the date shown; wording tightened.

| Id | Date | Decision | Source |
|---|---|---|---|
| <a id="d-14"></a>D-14 | 2026-11-25 | Rust crates are the main way to usable libraries: `import { X } from "rust:<crate>"`, the crate is compiled for wasm32 and statically linked with the program into one Wasm module (Rust's global allocator = `oxi_alloc`; callbacks into OxiScript closures are plain `call_indirect`); `oxi-bindgen` generates bindings; C/C++ the same way via Wasm without exceptions. Native bindings outside the sandbox only for OS services (kind D), chosen by the host operator. Inform, don't forbid: every build prints a notice per crate (version, `unsafe`, dropped guarantees). | `docs/oxiscript/grammar.md` 2.7, old plan row 8.N |
| <a id="d-15"></a>D-15 | 2026-11-25 | Interface laws: an `interface` may declare laws over its methods; every type satisfying it (as a generic bound) gets them checked at the same level as ordinary laws. A comparator or hash is checked, not trusted. | old plan row 8.S4 |
| <a id="d-16"></a>D-16 | 2026-11-25 | Bug shapes (variant analysis over OxiIR as graph patterns) and a sign vocabulary with requires/gives and vulnerability chains; every BAD ACTOR finding is generalised into a shape. | old plan row 8.S4 |
| <a id="d-17"></a>D-17 | 2026-11-26 | `export` is module visibility exactly like TypeScript; a function the host calls is marked `@host` and exported from the entry module; only `main` and `@host` functions become Wasm exports. Supersedes the earlier rule "entry-module export must fit the ABI" and item 4 of `tests/agent_test/LOG.md` (marked superseded there). | `docs/stages/host-export`, commits 4c7c5ed, 1b08076 |
| <a id="d-18"></a>D-18 | 2026-11-26 | Compilation targets `device`, `web`, `server` are a build setting, not a language construct and not a profile: they choose which Wasm features the code generator may use for the same program; semantics are identical on every target. | `docs/oxiscript/abi.md` 3.6, `docs/stages/targets`, commit d8fb0f8 |
| <a id="d-19"></a>D-19 | 2026-11-26 | A standard library starts before 8.N/8.S5: Wasm intrinsics as `std/math`, own deterministic libm in OxiScript, `std/string`, `std/collections` (Map/Set with insertion order), `std/array` stable sort, `std/result`. | old plan row 8.Std v0 |
| <a id="d-20"></a>D-20 | 2026-11-26 | Positioning: GUI and scripting for devices with megabytes of memory; first real hardware is the owner's Raspberry Pi 1 Model B+; no claim on the sites without measurements. | old plan row 8.Mem |
| <a id="d-21"></a>D-21 | 2026-11-26 | `host:dom` (idea): OxiScript as a capability-scoped alternative to JavaScript on ordinary web pages; OxiUI stays simple, interoperable and server-streamed first. | old plan row 8.Dom |
| <a id="d-22"></a>D-22 | 2026-11-26 | Dev blog on oxiscript.slavekm.pl: posts only by the owner or the lead agent, each signed by its real author; the build rejects unsigned posts. | old plan row 8.W |
| <a id="d-23"></a>D-23 | 2026-11-25 | Versioning: first public version 0.0.1, nothing guaranteed before 1.0.0; the patch number `c` grows by 1 per 5555 changed lines in `oxiscript-*/src`; x and y only by the owner for milestones; tag `oxiscript-vX.Y.C`. *Amended 2026-11-26 (owner): `oxiscript/std/**/*.oxs` counts too; the owner may release the next `c` early (`--owner-release`, carry resets to 0), first for 0.0.2.* | `docs/oxiscript/VERSIONING.md`, commit 8dd11c1 |
| <a id="d-24"></a>D-24 | 2026-11-25 | Wasm exception handling (`exnref`) is "nice to have, later": an opt-in ABI minor version, never required by the language, keeping a lowering without EH. | old plan "Later stages" |
| <a id="d-25"></a>D-25 | 2026-11-26 | Implementation flow: Gemini may write a stage, Sonnet with the advisor reviews it, then BAD ACTOR, then the lead merges. (HANDOFF; pilot done with 5.2, rtlink, std, simd, memres.) Whether Gemini becomes the default implementer is still open ([Q-PROC-1](#q-proc)). | `docs/HANDOFF.md` |
| <a id="d-26"></a>D-26 | 2026-11-25 | Sites in Polish and English first; Latin only after the owner reviews it (in memoriam of the owner's Latin teacher); identity: rust orange + bronze patina, "Oxi cell" mark, mascot Oxi the axolotl. | old plan row 8.W |
| <a id="d-27"></a>D-27 | 2026-11-26 | **Engine on microcontrollers is w2c2, not wasmi; the choice follows the compilation target.** When the build targets a microcontroller (`oxi build --target device` for an MCU board), the output goes through w2c2: no switch, no interpreter option. On MCUs (ESP32 and similar) the program is translated wasm to C (w2c2, MIT) and compiled by the board C compiler into native firmware, for development and for release; the wasmi interpreter is too slow on a microcontroller and is not shipped there. Reflashing on every change is accepted ("flash is not a tragedy"); instant iteration happens in the browser simulator (wasmi on the wasm32 core). Consequences: no fuel on the device, so the compiler inserts budget checks ([8.2b](#s-8-2b)); interpreted and w2c2 builds must give identical results (differential tests). wasmi stays the engine on servers, in the browser and in the playground. Supersedes [D-3](#d-3) for microcontrollers; answers [Q-ENG-1](#q-eng). Spec follow-up for [8.Spec](#s-8-spec): `abi.md` 3.6 still describes `device` as "wasmi on ESP32 and small devices"; it must say that `device` on an MCU means w2c2 native output (Linux boards such as the Raspberry Pi: engine chosen by the [8.Mem](#s-8-mem) measurement). | owner, conversation 2026-11-26 |
| <a id="d-28"></a>D-28 | 2026-11-26 | **Target `micro` split from `device`.** `micro` = microcontrollers (ESP32 and similar): always w2c2 native firmware per [D-27](#d-27), no fuel, compiler budget checks. `device` = small Linux machines (Raspberry Pi, old PCs, kiosks); engine chosen by the [8.Mem](#s-8-mem) measurement. Order `micro` < `device` < `web` < `server`. No separate `wasmi-micro` target: a target chooses machine capabilities, not the engine; the only reason for wasmi on an MCU (loading programs without reflashing: OTA, downloaded scripts, plug-ins) would be a packaging option, kept as an idea. Implemented in [8.2b](#s-8-2b). | owner, conversation 2026-11-26 |
| <a id="d-29"></a>D-29 | 2026-11-26 | **Speedup Lab.** After 8.Bench and 8.Perf a standing lab researches speedups (papers, SWAR and similar techniques), tests them as super-alpha flags, releases a speedup alpha weekly and promotes tested speedups into the language. Rule: a speedup never changes what a program does. Stage [8.Lab](#s-8-lab). | owner, conversation 2026-11-26 |
| <a id="d-30"></a>D-30 | 2026-11-26 | **Batch answers to the open questions** (docs/owner-questions.md), as the lead recommended unless noted: Q-ABI-1 (heap base and single trap channel recorded as an ABI 1.0 note), Q-PERF-1 (memory proposals in the Opus-verified order, 8.Perf), Q-PLAN-1 (8.S4c gate uses the BAD ACTOR findings committed in the repo), Q-SIMD-10/11 (file the wasmi issue, fix the Safari sentence), Q-LANG-1 (keep the whitespace rule for `?`), Q-LANG-2 (G-1..G-6 and C-1..C-11 signed), Q-LANG-3 (`readonly`/`private`/`static` checked by the compiler only, as in TS), Q-LANG-4 (bidi controls in code are an error), Q-LANG-5 (no `int`/`float` aliases; `unknown` with the JSON library), Q-MCU-1 (**no promise until it works: the sites say "planned"**), Q-MCU-2 (first MCU class: ESP32 with PSRAM), Q-DX-7 (size table as an 8.S3 result), Q-SIMD-1..3 and 5..18 as recommended, Q-SCB-1..7 deferred to the first native kind-D module; **floats on non-IEEE executors (VideoCore IV QPUs): (a) exact integer/fixed-point kernels by default plus (b) an explicit `@approx` opt-in with a documented error bound, forbidden in the `safety` profile**. Q-SIMD-4 pending an explanation. | owner, conversation 2026-11-26 |
| <a id="d-31"></a>D-31 | 2026-11-26 | **Gemini is the default implementer** for code and research stages unless there is a stated reason or the owner says otherwise (answers Q-PROC-1). **Gemini may do security audits but never as the only model:** every security review, BAD ACTOR pass or audit that Gemini performs is paired with an independent Claude review. | owner, conversation 2026-11-26 |
| <a id="d-32"></a>D-32 | 2026-11-26 | **No budget for an external auditor or certification body now.** Safety and security work is done with what the owner's Claude Code Pro subscription allows (agents, BAD ACTOR, fuzzing, formal tools with permissive licences); Q-SAFE-1 answered "not now"; claims about standards on the sites only after our own research (Q-SAFE-2). | owner, conversation 2026-11-26 |
| <a id="d-33"></a>D-33 | 2026-11-26 | **BAD ACTOR is Claude: Opus for large stages and releases; Sonnet with Opus consultation (the `advisor` tool) for small changes.** The lead decides which and says so in the stage notes; other models (Gemini) may add passes but never replace the Claude pass (complements D-31). | owner, conversation 2026-11-26 |
| <a id="d-34"></a>D-34 | 2026-11-26 | **BAD ACTOR rounds.** To save cost, several changes may be collected and attacked in one Opus round; changes may merge after their gate and review while waiting, but no release tag while any of its changes waits; large runtime/memory/sandbox/safety stages get their round before merge. Queue: [../stages/bad-actor-rounds.md](../stages/bad-actor-rounds.md). | owner, conversation 2026-11-26 |
| <a id="d-35"></a>D-35 | 2026-11-27 | **Batch answers, as the lead recommended** (docs/owner-questions.md): Q-SIMD-4 (only proved-in-bounds code goes to the GPU on the automatic path; a deferred trap at the next sync point, always the first failing index; debug self-check; `host:gpu` read-0/skip-write + debug flag); Q-EXE-1..6 (accuracy classes E/C/A; `@approx` with integer keys now; canonical float reduction tree on every executor, cost measured in 8.3d; QPU mode decided after the 8.Mem probe, start with mode H + `host:fft`; no guest machine code on the QPU ever; 8.3f an idea row); Q-ASYNC-1..5 (explicit `async`, plain `R` with `Pending<R>`, cancellation = not resumed + `defer`, reserve `defer` now in 8.Fix, reload cancels tasks); Q-DX-2,3,4,6,8,9,10 (MCP inside `oxi`; debugger after LSP; other editors via LSP; git + `rust:` until 1.0; telemetry only manual; DX owned by the lead; `oxi migrate` after 8.S5); Q-SAFE-2,3,4 and Q-PLAN-2 (IEC 62443 research by Gemini + Claude verification; Z3/cvc5; handover docs; 5% threshold); Q-OP-1..7 (OxiPerf methodology approved, v0.1 scope as recommended). OxiBench questions Q-OB-1..5: deferred by the owner. | owner, conversation 2026-11-27 |
| <a id="d-36"></a>D-36 | 2026-11-27 | **Board simulation (answers Q-DX-1):** (1) **support Wokwi as the market leader**: `oxi build --target micro` firmware (w2c2 + ESP-IDF, D-27/D-28) runs in Wokwi's ESP32 simulator, with a ready `wokwi.toml`/`diagram.json` per `oxi new` board template and docs; check Wokwi's terms and free tier before promising CI use; (2) **our own simple simulator** (virtual `host:gpio`/`host:sensor`/`host:ui` in the browser on the wasm32 core, no CPU emulation) **only after we have bought the board and really run something on it** (after the 8.S3 first-light milestone), so it mirrors real behaviour. | owner, conversation 2026-11-27 |
| <a id="d-37"></a>D-37 | 2026-11-27 | **Implicit lossless numeric widening** at assignment, argument and `return` only, for the exact pairs listed under P-2 (exception to P-2); lossy or narrowing conversions keep `as`; no promotion inside operators. Goal: less friction for TypeScript developers with no loss of precision. No "lax mode" or dialect (P-3). Implemented in 8.Fix. | owner, conversation 2026-11-27 |
| <a id="d-38"></a>D-38 | 2026-11-27 | **AI-assisted TS → OxiScript migration (`oxi migrate`), token-frugal; supersedes "after 8.S5" in D-35 for Q-DX-10.** Deterministic pipeline first, AI only on holes: TS parsed by a permissive Rust parser (`oxc_parser`, MIT), deterministic rules rewrite everything certain into an OxiScript AST printed by our formatter, uncertain spots become marked holes with a reason; `oxi check --json` applies machine-applicable fixes; only the remaining holes and errors go to an AI model, one small snippet each (function-level context, the registry reason code, neighbour signatures), never a whole file; loop until zero errors, plus differential tests when the TS source has tests. **Skeleton (parser, holes, check loop, AI hook, token accounting) early as stage [8.Mig](#s-8-mig); rules grown from the 8.S5 port data.** Also a **tree-sitter grammar for OxiScript** in [8.D1](#s-8-d1) (editors Zed/Neovim/Helix, structural queries; parses the whole corpus like our parser). | owner, conversation 2026-11-27 |

### 2.5 Superseded

| What | Superseded by | Source |
|---|---|---|
| Old execution order "5.0-5.1, 6, 7, then 5.2-5.7, Phase 8 after 7" | Phase 8 ran in parallel from 2026-11-24; 5.2 pulled forward and done; the critical path below | old plan lines 130-135, 664-672 |
| Founding research phased roadmap with effort estimates (phase 0: 8-12 weeks ... phase 5: 12+ months) and the "phase 0 spike" | The stage list in section 5; actual throughput was parser through VS Code extension in under two weeks of agent work | R3-177; `docs/research/oxiscript-language.md` section 14 |
| Research premise of `oxiscript-sync-callbacks.md` (8.N native bindings need sibling instances / frozen-region re-entry) | [D-14](#d-14) (crates linked statically); the document stays the reference for kind-D native bindings | R3 section K |
| Rule "entry-module export must fit the ABI" and LOG item 4 | [D-17](#d-17) | R3-85 |
| D-7 original ("OxiUI project owns the compiler in this repository") | D-7 revision 2026-11-26 (already inside the D-7 text) | old plan |
| "Stage 8.0 measures RC overhead with elision" | [8.Perf](#s-8-perf) | R2-84 |
| Plan row "8.2 AOT release path and profiles v0 plus compilation targets" as one stage | split: [8.2a](#s-8-2a) targets (done), [8.2b](#s-8-2b) AOT, [8.2c](#s-8-2c) profiles | R2-73, R4-51 |
| Research use of the name "8.3b" for GPU kernels (`docs/stages/5.2`) | 8.3c-8.3e; 8.3b = auto-vectoriser | R2 inconsistency 7 |
| Research reuse table naming `ena` for 8.T | 8.T did not use it (no inference variables) | R2-104 |

## 3. Done

Only verified items: merge commit found on `main` (`git log -1`), crate or file present. Dates are merge dates. Stage notes: `docs/stages/<stage>/` where they exist. Stages 8.C, 8.P, 8.R and 8.M have no stage notes anywhere in the repository (checked: not in `main` history, not in the worktrees); their evidence is the merge, the crate, its tests and the BAD ACTOR commits. This matters for the gate of [8.S4](#s-8-s4).

| Stage | What | Evidence |
|---|---|---|
| <a id="s-8-c"></a>8.C Contracts | Grammar (`docs/oxiscript/grammar.md`), OxiIR spec and text form (`ir.md`), value/memory layout, host ABI/IDL (`abi.md`, `contracts.md`), reason-code registry (`oxiscript-abi/registry/codes.txt`, 594 lines, append-only) | merge 2ba1aa0 (2026-11-25); crates `oxiscript-abi`, `oxiscript-ir` |
| <a id="s-8-p"></a>8.P Parser and formatter | Lexer, parser with recovery, formatter, `E0011` depth limit, iterative AST drop | merge 13b5987; crate `oxiscript-syntax`; fuzz tests with `OXI_FULL_FUZZ`, `.github/workflows/oxiscript-fuzz.yml` |
| <a id="s-8-r"></a>8.R Runtime | wasmi 2.0.3 host, IDL capabilities (`host:log`, `host:state`), fuel, `StoreLimits`, poisoning, instance-swap reload, `oxi_tick` | merge d007c74; crate `oxiscript-runtime` [R2-56] |
| <a id="s-8-m"></a>8.M Memory (crate) | `oxiscript-rc`: RC runtime, deferred destruction queue, Bacon-Rajan cycle collector, RC-balance checker, `oxi_str_from_f64` behind feature `float-fmt`. Real linking into programs is not part of it: see [8.M-link](#s-8-m-link) | merge 5a5929d; BAD ACTOR commit 1fa4853 [R2-57] |
| <a id="s-8-t"></a>8.T Type checker | Sound checker, Maranget exhaustiveness, interned types, budgets (`E0113`), effect summary (`EffectSummary`, partial), laws purity (`E0125`), typed output contract | merge 3cbf1b4; `docs/stages/8.T`; crate `oxiscript-check` [R2-58] |
| <a id="s-8-g"></a>8.G Codegen | OxiIR to Core Wasm, validated, run in wasmi; strict-engine limits (`E0603`, `E0704`, `E0408`) | merge bad1e94; `docs/stages/8.G`; crate `oxiscript-wasm` [R2-59] |
| <a id="s-8-l"></a>8.L Lowering | Typed AST to OxiIR; end-to-end `.oxs` to Wasm; closures with shared cells; monomorphisation; `E0700`/`E0701` | merge c792b4f; `docs/stages/8.L`; crate `oxiscript-lower` |
| <a id="s-8-d0"></a>8.D0 DevKit v0 | `oxi` CLI (`check`, `fmt`, `explain`, `new` with 4 templates, later `run`, `build`), JSON schema 1 (`SCHEMA.md`), `AGENTS.md`, `llms.txt`, skill, VS Code extension v0 (TextMate grammar, diagnostics, quick fixes, semicolon assistant, format on save), agent test (`tests/agent_test/LOG.md`: 10 programs, 13 runs) | merge 148b8e8; `docs/stages/8.D`; `oxiscript/oxiscript-cli`, `editors/vscode-oxiscript` |
| <a id="s-8-w1"></a>8.W1 Websites step 1 | Home, OxiUI and OxiScript sites, playground with check and format, roadmap generator, support link, PL+EN (LA written, off by default). Gate item "Lighthouse measured" was not met: moved to [8.W2](#s-8-w2) and `U.Sites` | merge 0df8681; `docs/stages/8.W`; `sites/` |
| <a id="s-8-run"></a>8.Run | `oxi run` and `oxi build` wired to `compile_source`; playground Run in a Web Worker (wasmi) with fuel 10M, 5 s wall timer, output caps | merge 6dda5de (branch merge c291e92); `docs/stages/run` |
| <a id="s-8-id"></a>8.Id Site identity | Palette, "Oxi cell" mark, axolotl mascot (SVG and ASCII); Physarum companion deliberately skipped (belongs to 8.S4 as an idea) | merge 6dda5de (branch merge 320db1b); `docs/stages/identity` |
| <a id="s-8-hx"></a>8.HX Host export | `export` = visibility, `@host` marks Wasm exports ([D-17](#d-17)); `E0121` for unfit `@host` signatures | merges 4c7c5ed, 1b08076; `docs/stages/host-export` |
| <a id="s-8-rel"></a>8.Rel Release 0.0.1 | Versions 0.0.1 on all crates, `oxi --version` notice, `scripts/oxiscript-version.sh`, gate step `version-check`, `CHANGELOG.md`, annotated tag `oxiscript-v0.0.1` | commits 8dd11c1, eed8d35; `git tag`; notes in `docs/stages/0.0.1` still say "not tagged" (stale) |
| <a id="s-8-2a"></a>8.2a Compilation targets | `device`/`web`/`server` as manifest `target`, features, `--target`, `OXS-E0411`, `OXS-E0405`, runtime `max_target`, memory64 declarable but refused, no multi-memory, matrix `targets-web` in `scripts/oxiscript-gate.sh` | merge d8fb0f8; `docs/stages/targets`; `oxiscript-wasm/src/target.rs` |
| Showcase sites (OxiUI versions of the sites) | merge 782e539, `docs/stages/showcase` | owned by [oxiui.md](oxiui.md) |

Done, not a stage: 98 reason codes plus 4 `P` codes registered (codes.txt), language invariants in force (reserved words, numeric rules, `?` rule, `@host`), 0.0.1 CHANGELOG, machine-applicable fixes for `==`, `;`, parentheses, float suffix. Specs in force without further work (kept as invariants, not tasks) [R3-03, R3-11, R3-12, R3-13, R3-14, R3-22, R3-23, R3-25, R3-33, R3-38, R3-50, R3-52, R3-57, R3-63, R3-70, R3-71, R3-72, R3-73, R3-75, R3-76, R3-77, R3-81, R3-82, R3-86, R3-88, R3-90, R3-91, R3-92, R3-93, R3-148, R3-203, R3-205, R3-217, R3-248, R2-54..R2-60, R2-77, R2-94, R2-97, R2-98, R2-99, R2-102, R2-103, R2-107, R2-110, R2-117, R2-118, R2-119, R2-123, R2-125, R2-134, R2-138, R4-50, R4-51, R4-63].

### 3.1 Machine-readable status table

One row per stage; the status column uses only `done`, `in-progress`, `todo`, `idea`; the last column is the merge hash or empty. The last cell is `date, hash` or an em dash. Input for the roadmap generator of the sites (owned by [U.Sites](oxiui.md#s-u-sites)).

| Stage | Status | Merged |
|---|---|---|
| 8.C | done | 2026-11-25, 2ba1aa0 |
| 8.P | done | 2026-11-25, 13b5987 |
| 8.R | done | 2026-11-25, d007c74 |
| 8.M | done | 2026-11-25, 5a5929d |
| 8.T | done | 2026-11-25, 3cbf1b4 |
| 8.G | done | 2026-11-25, bad1e94 |
| 8.L | done | 2026-11-25, c792b4f |
| 8.D0 | done | 2026-11-25, 148b8e8 |
| 8.W1 | done | 2026-11-25, 0df8681 |
| 8.Run | done | 2026-11-26, 6dda5de |
| 8.Id | done | 2026-11-26, 6dda5de |
| 8.HX | done | 2026-11-26, 1b08076 |
| 8.Rel | done | 2026-11-26, eed8d35 |
| 8.2a | done | 2026-11-26, d8fb0f8 |
| 8.M-link | done | 2026-11-27, a17d1ef |
| 8.Std | done | 2026-11-26, fd7bf41 |
| 8.Perf | todo | — |
| 8.Lab | todo | — |
| 8.Mig | todo | — |
| 8.Fix | todo | — |
| 8.Spec | todo | — |
| 8.Sec | todo | — |
| 8.0 | todo | — |
| 8.S1 | todo | — |
| 8.S2 | todo | — |
| 8.D1 | todo | — |
| 8.D2 | todo | — |
| 8.S4a | todo | — |
| 8.S4b | todo | — |
| 8.S4c | todo | — |
| 8.N | todo | — |
| 8.Bench | in-progress | harness v0 (dev tier) on branch oxiperf, 2026-11-26 |
| 8.2b | todo | — |
| 8.2c | todo | — |
| 8.3a | todo | — |
| 8.3b | todo | — |
| 8.3c | todo | — |
| 8.3d | todo | — |
| 8.3e | todo | — |
| 8.S3 | todo | — |
| 8.S3w | todo | — |
| 8.Mem | todo | — |
| 8.S5 | todo | — |
| 8.W2 | todo | — |
| 8.W3 | todo | — |
| 8.Blog | todo | — |
| 8.Pub | todo | — |
| 8.OB | todo | — |
| 8.4a | todo | — |
| 8.4 | todo | — |
| 8.5 | todo | — |
| 8.6 | idea | — |
| 8.Dom | idea | — |
| 8.Repo | idea | — |

(`8.OB` is `todo` but blocked on the owner; see its stage block.)

<a id="in-progress"></a>
## 4. In progress

Three unmerged branches carry work; all other `worktree-agent-*` branches are fully merged (0 commits ahead of `main`, see `consistency.md` for cleanup). Checked with `git log main..<branch>` on 2026-11-26.

| Branch (worktree) | Stage | State | What is missing before merge |
|---|---|---|---|
| `gemini-rtlink` (3 commits: c5d31f4 feature, da87be6 BAD ACTOR tests, 0cf56d3 merge of targets) | [8.M-link](#s-8-m-link) | Gemini is fixing BAD ACTOR findings BL01-BL10 (`docs/stages/8.M-link/bad-actor.md` on that branch; 18 uncommitted modified files in the worktree) | Fixes of BL01, BL03, BL04, BL06 (as input to 8.Perf), BL08, BL09, BL10; owner/lead decision on the two ABI questions (heap base, trap channel) recorded in `abi.md`; gate run; merge |
| `gemini-std` (2 commits: 90fa662 pre-review snapshot, b416210 BAD ACTOR tests and report) | [8.Std](#s-8-std) | BAD ACTOR findings BS01-BS08 being fixed by Gemini (report on that branch) | Rebase on `main` (the branch base predates the targets merge; merging it as is would delete `oxiscript-wasm/tests/targets.rs` and a gate step), wait for 8.M-link (f64 text), review, std module resolution from a bundled directory |
| `gemini-memres` (1 commit e4b9f7d "Gemini draft, unverified") | input of [8.Perf](#s-8-perf) | `docs/research/oxiscript-memory-performance.md` (about 470 lines, plus `oxiscript-runtime/src/runtime.rs` +17 lines) committed on the branch; Opus verifying | Verification pass (claims marked `[estimate]`/`[unverified]` in the document), merge of the document to `main` so that links to it work; the 17-line runtime change must be reviewed separately |

Excerpt of what the memres draft recommends (to be verified, not adopted yet): a slab plus TLSF/talc allocator for small objects, inline Wasm fast paths for `retain`/`release`/`alloc`, Perceus-style elision and reuse, scalar replacement, a collector that allocates nothing. Target stated: 1.1-1.8x of the bump stub fuel against the measured 6.5-10.5x. These numbers are the draft's estimates.

BAD ACTOR findings of 8.M-link, condensed (from the branch file, for tracing into other stages): BL01 collector needs heap headroom (OOM in `deinit`); BL02 cycles inside one host call never collected, OOM path does not collect first; BL03 exported `oxi_rt_init` accepts a heap base inside program data; BL04 embedded runtime contains the builder's absolute cargo path (reproducibility and privacy); BL05 `build.rs` rerun list and nested cargo flags; BL06 fuel 6.5-10.5x of Stub, default 10M fuel allows about 3k allocations per call instead of 25k; BL07 module 58-60 KB versus 3.5-5 KB, compile 3.4-4.5 ms versus 0.7-2.2 ms; BL08 nothing in `oxiscript-runtime` ever calls `oxi_collect_cycles`; BL09 `${f32}` prints the widened f64 text; BL10 lower tests now run Linked only and the debug-only `stale_object` check is absent. BL06 and BL07 are owned by [8.Perf](#s-8-perf), BL08 by [8.M-link](#s-8-m-link) (host collects between events), BL04 by [8.Sec](#s-8-sec), BL10 by [8.M-link](#s-8-m-link).

## 5. Roadmap

Ids follow the old plan; existing ids are reused. Renames: old `8.2` (AOT release path, profiles v0 and compilation targets in one row) -> `8.2a` targets (done), `8.2b` AOT, `8.2c` profiles; old `Stage 8.0` -> `8.0` (kept; the "RC overhead with elision" part moved to `8.Perf`); old `8.W` -> `8.W1` (done), `8.W2`, `8.W3`, `8.Blog`; old `8.D` v0/v1/v2 -> `8.D0` (done), `8.D1`, `8.D2`; `8.S4` stays as an umbrella with parts `8.S4a/b/c`; stage directories `run`, `identity`, `host-export`, `targets`, `0.0.1` got ids `8.Run`, `8.Id`, `8.HX`, `8.2a`, `8.Rel`. New ids (no old row): `8.M-link`, `8.Fix`, `8.Spec`, `8.Sec`, `8.Bench`, `8.S3w`, `8.OB`, `8.Pub`, `8.4a`, `8.Repo`. Cross-links to OxiUI stages have the form `oxiui.md#s-5-3` (dots become hyphens).

Gate rule for the catch-all stages `8.Fix`, `8.Spec`, `8.Sec`: every source id listed in the stage is closed by a regression test or by a commit that changes the named document, and the closure list is committed as `docs/stages/<stage>/closed.md` with one line per id.

### 5.1 Order, dependencies and the critical path

Critical path to the first visible result (the old path 8.C -> 8.P -> 8.T -> 8.L -> 8.S1, now with the missing link and the OxiUI side named):

`8.L (done)` -> **`8.M-link`** (in progress) + [U.S1](oxiui.md#s-u-s1) (OxiUI side of the script element) -> **`8.S1`** -> `8.Repo` (owner decides) / `8.S3` (needs hardware, after 2026-12-10).

Gate legend: for stages that had an owner row in the old plan (8.S1-8.S5, 8.D, 8.W, 8.Std, 8.Dom, 8.Mem, 8.N, 8.Perf, 8.3a-e) the gate is the owner's text, with amendments marked (8.S4c); gates of stages that are new in this roadmap (8.M-link, 8.Fix, 8.Spec, 8.Sec, 8.0, 8.Bench, 8.2b, 8.2c, 8.S3w, 8.4a, 8.Pub, 8.Repo) and added dependencies (for example 8.N -> 8.Std) are proposals for owner approval.

**Order agreed with the owner on 2026-11-27:** 8.S1 (now) → 8.2b (compiled path with budget checks, `micro` target, differential interpreter vs compiled: the biggest single speedup, OxiPerf shows wasmi 20-50x slower than native while Wasmtime is ~1.8x of Rust) → 8.Perf (after the Gemini weekly quota refresh, ~2026-12-03).

Parallel to it, in order of unblocking value:

```
8.M-link --+--> 8.S1 (needs oxiui 7.1, done)
           +--> 8.Std (rebase, f64 text) --> 8.W2 --> 8.W3
           +--> 8.Perf (needs memres verified) --> 8.2b --> 8.Mem (+ oxiui 7.2 done)
           +--> 8.N (needs 8.Std for the allocator contract) --> 8.S5 (needs 8.S1)
8.0 (host part now; board part after 8.S3)
8.Fix, 8.Spec, 8.Sec: independent of the above, start now
8.S2 (LSP) --> 8.D1 --> 8.Pub
8.S4a (laws) --> 8.S4b (report) --> 8.2c (profiles) --> 8.5 (safety)
8.Bench --> 8.3a --> 8.3b (needs 8.S4b remark type) --> 8.3d (needs 8.3c, 8.M-link) --> 8.3e
8.Bench + 8.Perf --> 8.Lab (weekly speedup alpha, then promotion into the language)
8.3c (needs oxiui 5.2, done) 
8.4a (async spec, before more IR hardening) --> 8.4 (needs 8.3b, 8.2b) --> 8.Dom (needs 8.S1)
```

Waves (suggested, three agents at most, per the working rules in section 8). The old parallel schedule (waves 1-6) is stale: waves 1-4 are done or overtaken, and it never listed 8.Std, 8.Mem, 8.Perf, 8.3a-e or 8.Dom [R4-64]:
- Wave A, now: finish 8.M-link, 8.Std, verify memres; start 8.Fix (checker, parser and formatter items only), 8.Spec, 8.Sec, 8.S2.
- Wave B, after 8.M-link merges: 8.S1, 8.Perf, 8.N, 8.S4a, 8.Bench, 8.W2.
- Wave C: 8.2c, 8.3a, 8.3c, 8.S4b, 8.D1, 8.0 host measurements.
- Wave D: 8.3b, 8.2b, 8.Mem, 8.S5, 8.4a; board work (8.S3) when the hardware arrives.
- Later: 8.3d, 8.3e, 8.4, 8.5, 8.6, 8.Dom, 8.OB, 8.Repo.

### 5.2 Foundation: finish what is merged

<a id="s-8-m-link"></a>
#### 8.M-link Real runtime linking (in progress, branch `gemini-rtlink`)

- Scope: link every program with the real `oxiscript-rc` runtime into one Wasm module (`RuntimeMode::Linked` default, `heap_base`, one trap channel, f64 text, cycles collected); close the BAD ACTOR findings BL01-BL10 (section 4); record the two ABI decisions in `abi.md` (heap base passed to `oxi_rt_init`, trap cell versus `oxi_trap_kind`) and validate `oxi_rt_init` arguments (BL03) [R2-92, R2-93, R4-52, R3-40]. Resolves the closure-cycle leak L04 and `${f64}` text under the Stub [R2-112, R2-114, R2-132]. Host collects cycles between events (pump/tick calls `oxi_collect_cycles`) or a bounded `oxi_gc_step` is kept in linked modules (BL08, BL02). Keep Stub-versus-Linked parity tests and a way to run the debug `stale_object` check (BL10). Remap the builder's paths out of the embedded runtime so artifacts are byte-identical across machines (BL04, with [8.Sec](#s-8-sec)); decide `${f32}` text (BL09, doc or fix).
- Gate: gate script `merge` green; 208 lower tests plus corpus pass in both modes with identical logs, results and trap kinds; BL01 `collect_cycles_needs_headroom_probe`, BL03 `rt_init_recall_matrix`, BL08 `long_running_instance_leaks_cycles_until_oom_without_host_collect` and BL09 `f32_text_probe` pass; two builds on different `CARGO_HOME` give identical `.wasm`; `oxi run` prints `${1.5}`; fuel and size numbers (BL06, BL07) recorded as the baseline for [8.Perf](#s-8-perf); BAD ACTOR again.
- Depends on: 8.L, 8.M, 8.R (done). Blocks: 8.S1, 8.Std (f64 text), 8.Perf, 8.N, 8.Mem, 8.3d.
- Status: in progress (3 commits, fixes pending).

<a id="s-8-std"></a>
#### 8.Std v0 Standard library (in progress, branch `gemini-std`)

- Scope ([D-19](#d-19)): `std/math` intrinsics as single IR/Wasm ops (`sqrt floor ceil trunc round abs min max copysign`, `clz ctz popcnt rotl rotr`, `bitsOf/fromBits`, constants); deterministic software `sin cos tan asin acos atan atan2 exp log log2 log10 pow cbrt hypot` with a documented max-ulp error and the same bits on every target (own libm, never the host's) [R3-68]; `std/string` (`split trim replace repeat pad*`, `parseInt/parseFloat` returning `Result`, number formatting); `std/collections` `Map<K,V>`, `Set<T>` over interfaces `Hash` + `Eq`, insertion-order iteration, no prototype-pollution class of bug [R3-141, R3-170, R3-220]; `std/array` stable `sort/sortBy` (merge sort with a fixed comparison sequence, so inconsistent comparators cannot panic or diverge) [R3-281 Tier 0]; `std/result`; module resolution of `std/...` from a bundled directory in CLI, playground and tests; every function has an example that `oxi check`s [R3-30, R3-60, R2-68, R2-91]. Optional here: `checked_add` returning `Result` [R3-143].
- Gate: course levels 0-2 and the 8.S1 demo need no missing basic function; sin/cos/exp/log/pow within the documented ulp bound against a reference on 10^6 inputs and bit-identical on `device` and `web`; Map/Set fuzzed against a reference model with zero leaks; BAD ACTOR findings BS01-BS08 fixed; branch rebased so `oxiscript-wasm/tests/targets.rs` and the gate step survive.
- Depends on: 8.L, [8.M-link](#s-8-m-link) (f64 text). Status: in progress.

<a id="s-8-fix"></a>
#### 8.Fix Language and compiler leftovers (todo)

Closes known, owned-by-nobody defects of the merged stages. Each id below is closed by a test or a documented decision.
- Implicit lossless numeric widening ([D-37](#d-37), exception to P-2): checker accepts the exact pairs at assignment/argument/return, lowering inserts the conversion, tests for every allowed pair and for every rejected lossy pair (still `OXS-E0100`/`E0110` with the `as` fix), docs updated in the same change (`grammar.md` section 6, AGENTS template core rule 1, `llms.txt`, skill).
- Checker (8.T): `static constructor` silently dropped; non-constant case labels accepted; no warning for a label excluded by narrowing; closure capturing `this` in a mutating struct method sees the old value [R3-53, R2-106]; arithmetic on a nullable should say `E0105`, warning `W0001` (unused binding) and `W0004` (no effect) emitted [R2-105]; closure returning a narrowed-null local gives `E0700` [R2-111]; `throw/try/catch` diagnosed by the checker, not only by CLI hints [R2-122]; `.length` and `s[i]` get a dedicated diagnostic with a fix hint [R3-249, R3-89, R3-235]; error text names the sub-expression on budget exhaustion [R3-252]; labels and multi-span rendering checked [R3-147]; array invariance, method-parameter contravariance and narrowing-invalidation tests from the 8.T attack list, plus a TS soundness probe corpus [R3-140, R3-209]; `readonly`, `private`, `static` semantics on value copies decided ([Q-LANG-3](#q-lang)) [R3-210]; reserve `decreases`, `ghost`, `pure` so they cannot become identifiers [R3-169].
- Parser and formatter (8.P/8.D): comment re-attachment bug (trailing `// two` moves, `/* b */` jumps over `+`), the E0006 no-op fix oracle, multi-seed run of both 8.P oracles [R2-55, R2-133, R2-163, R3-204]; the E0004 continuation-hazard (`a\n[1]`) reported by the parser with the previous-line range, not only in CLI `hints.rs` [R2-121]; source size, token count and identifier length caps and numeric/escape edge cases (`1e999`, `\u{110000}`) as tests [R3-207, R3-208].
- Lowering and codegen (8.L/8.G): Wasm local reuse (liveness free list) so a function may exceed about 6000 simple statements (limit 12000 locals, `E0603`/`E0704`) [R3-32, R2-108, R2-113, R4-17, R3-61]; tag-check elision for `bad_cast` [R2-108]; the "40 average bytes per function" strict-engine rule and the 1000-tiny-exports `E0408` mismatch resolved by relaxing in 8.R or padding in codegen [R2-109]; float `%` and `x?.f` on `this` in constructors (`E0700`) [R3-31]; dropping 120k nodes exceeds default fuel as `OutOfFuel`: decision on `fuel_per_call` or a bounded drop step [R2-115]; instantiation budget `E0701` becomes a profile knob with counts in the report ([8.2c](#s-8-2c)) [R3-142].
- Gate: closure list `docs/stages/8.Fix/closed.md` complete; checker, lower and wasm suites green on `device` and `web`; no existing code renumbered.
- Depends on: none for the checker, parser and formatter items (start now). The lowering and codegen items (local reuse, tag-check elision, float `%`, drop fuel) touch `oxiscript-ir`, `oxiscript-lower` and `oxiscript-wasm`, which `gemini-rtlink` is editing (24 uncommitted files), so they wait for [8.M-link](#s-8-m-link) to merge; local reuse must land before [8.3a](#s-8-3a), which can expand a v128 into 16 locals [R4-17]. Status: todo.

<a id="s-8-spec"></a>
#### 8.Spec Specification and agent-doc repair (todo)

Documentation and spec decisions that the merged stages left inconsistent. Doc fixes listed for the lead in [consistency.md](consistency.md) are applied here as commits.
- `grammar.md`: wrong codes (`===` on struct is `E0110`, not `E0100`; `?.` on non-null is `E0127`; no-effect statement is `W0004`; missing return is `E0114`), deviation-table numbering gap (rows 14, 15), header and section 7 refreshed with recorded sign-offs [R3 inconsistencies 1-4, 7; R3-24, R3-09, R3-10].
- `contracts.md`: switch-expression arms use `:` not `=>` (decision 2 text), sections 4-5 rewritten as history, convention list order, stdlib line [R3 inconsistency 5; R3-58, R3-60].
- `abi.md`: one statement whether adding EH changes `oxi_abi_version` (ABI minor, [8.6](#s-8-6)); `result<str>` cap by `HOST_PAYLOAD_MAX`; ABI 1.0 notes for heap base and trap channel after [8.M-link](#s-8-m-link) [R3 inconsistency 6; R2-115, R2-93].
- `ir.md`: effect vocabulary (`alloc, panic, loops, recursion`) versus the checker's `EffectSummary`: say which are computed now and which wait for [8.2c](#s-8-2c); `str.from_f64` status after 8.M-link [R3 inconsistency 11; R3-161].
- Agent-facing copies: `oxi run`/`oxi build` "not available" removed from `templates/common/AGENTS.md` and `SKILL.md` and then `sync_docs.py` run; `llms.txt` code list generated by a stated rule (all emitted codes, including `E0400`/`E0402`; `E0500` listed only when emitted); skill error table synced [R3 inconsistencies 9, 10, 23; R3-47, R3-234].
- Research documents marked superseded where the plan overtook them (sync-callbacks premise, dev-experience status column, founding research phases, OxiBench `[u8]` -> `u8[]`, `unknown` and `int/float` alias rejections recorded) [R3 inconsistencies 16-20; R3-128, R3-138].
- `oxi build`/`oxi run` decisions to confirm: default `oxi build` output is relative to the cwd, not the project root; the truncation marker is English [R2-137].
- Release process: first real use of the version script carry arithmetic and the two JSON snapshots per release (candidate 0.0.2) [R2-96]; `docs/stages` notes made current (0.0.1 tagged, 5.2 blessed) [R2 stale list]; the research document `oxiscript-known-attacks.md` gets a status column per attack (test exists or not) [R3 stale].
- Gate: closure list complete; `cargo test -p oxiscript-cli` compiles every `oxs` block; `python3 -I oxiscript/oxiscript-cli/tools/sync_docs.py` leaves no diff. Depends on: none (reasonably after [8.M-link](#s-8-m-link) for the ir/abi notes). Status: todo.

<a id="s-8-sec"></a>
#### 8.Sec Hardening backlog (todo)

The `known-attacks` and BAD ACTOR items with no owner. "Inform, don't forbid" applies to dependencies (notices, not bans); the items below concern our own crates.
- `#![forbid(unsafe_code)]` (or `deny` with a documented exception list) in `oxiscript-runtime` and the other embedder crates; no `unsafe` was found in the runtime, so it is free [R3-216].
- Diagnostics echo sanitised: ESC, OSC, CSI, C1, CR and bidi controls in any echoed source, identifier or string in `oxi` output, the playground and the VS Code extension (OxiTerm is a terminal) [R3-206].
- Bidi and invisible characters in literals and comments: today only warning `OXS-W0003`; owner decides whether to make it an error ([Q-LANG-4](#q-lang)); comments checked [R3-201].
- Promote the `S`/`R` rows of the known-attacks catalogue to verified (primary pages read) [R3-200]; record regex ReDoS decision with [8.N](#s-8-n) [R3-218].
- Dependencies: `cargo audit` in CI exists; check that wasmi 2.0.3 contains the fix for the advisory of 2025-12 (CVE-2025-66627); licence check `cargo deny` is owned by [U.CI](oxiui.md#s-u-ci) and linked here [R3-215].
- Runtime: wall-clock limit independent of fuel and fuel charged proportionally to host bytes; fault-injection sweep at every allocation and fuel point; hot reload cannot widen capabilities (compare the capability manifest old versus new) [R3-213, R3-214, R3-221].
- Reproducible compiler output as a test (two builds byte-identical; BL04) and the Vec-order/no-hash-order rule checked [R3-69, R3-163 first half]; supply-chain classes recorded for the day a registry exists [R3-219].
- Gate: closure list complete; sanitising and forbid covered by tests that hit the failing branch; no new `unsafe`. Depends on: none. Status: todo.

<a id="s-8-0"></a>
#### 8.0 Measurement spike (never run; todo)

Named in the old plan four times but never scheduled. It measures; it does not reopen [D-1](#d-1), [D-3](#d-3) or [D-10](#d-10).
- Part 0a (host, now, after 8.M-link): wasmi translation latency for 1-2k-line modules; fuel overhead per operation; raw wasmi host-call crossing in ns (the missing number for [8.N](#s-8-n)); RC against a tracing collector on three workloads as information; flash/size cost of wasmi `simd` and `memory64` code that the targets work compiles into every runtime build; compile-speed budget tracked by a CI example [R3-103, R3-151, R3-292, R3-159, R2-84, R2-101].
- Part 0b (board, after [8.S3](#s-8-s3)): `no_std` flash size and heap of wasmi on ESP32-S3, wasmi against WAMR bake-off, w2c2 AOT; edit-to-visible latency targets (under 300 ms simulator, under 2 s device) [R3-107, R3-237].
- Gate: a table in `docs/research/oxiscript-measurements.md` with method, hardware and numbers; every number states whether it is measured or estimated. Depends on: 8.M-link (0a), 8.S3 (0b). Status: todo.

<a id="s-8-perf"></a>
#### 8.Perf RC elision and allocation fast path (todo)

- Scope: as in the owner-approved plan row of 2026-11-26: (1) ownership/borrow inference and retain/release elision on OxiIR (Perceus, Lean 4 borrowed parameters); (2) reuse analysis and in-place update for unique values; (3) drop specialisation; (4) allocation fast path inlined by codegen (size-class free-list pop), allocation folding; (5) escape analysis turning non-escaping objects into locals, scalar replacement of COW structs and nullable scalars (contracts decision 10) [R3-59, R3-62, R3-66]; array literal and template-string lowering without repeated reallocation. Input: [research/oxiscript-memory-performance.md](../research/oxiscript-memory-performance.md) and its verification [research/oxiscript-memory-performance-verification.md](../research/oxiscript-memory-performance-verification.md) (Opus, 2026-11-26: 114 claims, 13 wrong). Key verified fact: wasmi charges a region's whole static operator count on entry (regions start at function entry, `loop`, `if`/`else`), and LLVM-compiled dlmalloc `malloc` is one 1651-operator region, so each allocation costs about 1700 fuel whatever path runs; allocation plus final release are about 95% of the BL06 `new class` row, call overhead a few percent. Order of work (verification section A): (a) size-class free lists / slabs in the runtime with the backing allocator out of line, empty slabs returned; (b) free leaf objects directly on last release; (c) move rare paths out of the always-charged regions; (d) format integers straight into a string, array literals as one allocation; (e) inline `is_unique`, `retain`, non-final `release` in generated code, with a debug mode keeping the calls for `stale_object`; (f) the elision passes listed above, with an RC-balance verifier; collector headroom goes to 8.Mem. Elision must preserve RC balance (the 8.M balance checker) and the "unprotected borrow" rule [R3-65]. Also owns the optimised-versus-unoptimised IR differential [R3-211, R2-116] and the BL06/BL07/BL01 numbers.
- Also: sorting 30k `i32` with `std/array` within the default 10M fuel (BS07: needs ~21M on 2026-11-26).
- Gate: fuel per operation within 1.5x of the bump stub on the BL06 table and on the 8.S5 programs, **and wall time measured next to fuel on wasmi and on w2c2** (fuel alone rewards code layout, verification finding); percentage of retain/release removed reported per corpus; Stub-versus-Linked and optimised-versus-unoptimised differential runs identical; leak tests and the `stale_object` check stay green; BAD ACTOR attacks use-after-free through wrong borrow inference.
- Depends on: 8.M-link merged (memres document verified and merged to `main` 2026-11-26, d8cb3b7). Status: todo.

<a id="s-8-mig"></a>
#### 8.Mig `oxi migrate`: AI-assisted TS → OxiScript (todo)

- Scope ([D-38](#d-38)): `oxi migrate file.ts|dir` = TS parse (`oxc_parser`, MIT; licence re-checked at adoption) → deterministic rules (`==`→`===`, `var`→`let`, `undefined`→`null`, `console`→`host:log` + `oxi.toml` needs, numeric types from annotations and use, classes → `struct`/`class` + interfaces, exceptions → `Result` where the call graph is local, imports) → OxiScript AST via our formatter, uncertain spots as holes with a reason → `oxi check --json` + machine-applicable fixes → AI only on remaining holes/errors, one snippet each, via a pluggable model hook (Claude Code / Gemini CLI, no paid API: D-32) → loop to zero errors; differential tests TS vs OxiScript when tests exist. Token accounting per file. Rules extended from the 8.S5 friction log.
- Gate: a corpus of real TS files (permissive licences) migrates to zero `oxi check` errors; tokens per 1000 lines measured against a pure-agent migration of the same corpus (target: a large reduction, reported, not promised); the AI never receives a whole file; every deterministic rule has a test; BAD ACTOR on hostile TS input (parser bombs, prompt injection inside TS comments/strings sent to the AI hook).
- Depends on: 8.D1 (MCP tools, tree-sitter grammar useful), 8.S5 for the rule data (skeleton may start before). Status: todo.

<a id="s-8-lab"></a>
#### 8.Lab Speedup Lab (todo; owner 2026-11-26)

- Scope: a standing, weekly pipeline for performance ideas ([D-29](#d-29)). (1) **Research:** papers and known techniques (SWAR on `i64`, e.g. UTF-8 validation, byte search, string compare, bit counting; division by constants; loop specialisation; jump tables; Hacker's Delight-style tricks; later the SIMD/GPU research). Each idea gets a card in `docs/lab/<name>.md`: source with link, expected gain, which targets (`micro`, `device`, `web`, `server`), why it cannot change a result. Gemini researches, Opus verifies (same flow as the SIMD/GPU and memory reports). (2) **Super-alpha:** each speedup behind `oxi build --unstable-opt=<name>`, off by default. (3) **Tests:** identical results with and without the flag on the whole corpus and the fuzzers (values, trap kinds, order of effects, float bits); fuel and wall time measured on wasmi and w2c2 (and the browser where relevant) with [8.Bench](#s-8-bench); BAD ACTOR tries to find an input where the two differ. (4) **Weekly alpha:** the speedups that passed are released together as `oxiscript-vX.Y.C-speedup.N` with a before/after table. (5) **Promotion:** after one week without findings a flag becomes the default; an opt-out flag stays for one version. Optional public "Speedup Lab" page on oxiscript.slavekm.pl with the table (claims only with measurements, as [D-20](#d-20)).
- Rule (owner): **a speedup never changes what a program does** (results, traps, order of effects, float results; [P-9](#p-9)). Integer SWAR tricks qualify; float reassociation or fast-math only as an explicit opt-in written in the program, never silently. Most value expected on `micro` and `device`, where there is often no SIMD and SWAR on 64-bit integers is the only parallelism ([D-28](#d-28)).
- Gate (per weekly alpha): differential runs identical; every promoted speedup shows a measured gain on at least one target and passes a **non-inferiority test on every other target** (block unless the upper bound of the 95% interval of new/old time is below 1.02; an under-measured, noisy run must not count as "no loss": verification of the OxiPerf methodology); BAD ACTOR report with no open finding; card verified.
- Depends on: [8.Bench](#s-8-bench), [8.Perf](#s-8-perf) (its optimised-versus-unoptimised differential harness). Runs in parallel with the rest of the roadmap afterwards. Status: todo.

### 5.3 The visible slice and the tools around it

<a id="s-8-s1"></a>
#### 8.S1 First visible slice: `<script lang="oxs">` in OxiUI pages (todo; end of the critical path)

- Scope: OxiScript in THTML pages: event handlers and state, server-driven and browser-local (WASM local mode). The language side: the `host:ui` (or equivalent) IDL replacing today's placeholder UI IDL, `@host` handlers per [D-17](#d-17), hot reload by instance swap with host-managed state kept ([D-9](#d-9), [P-8](#p-8)); module-local `let` resets, the host drops old callbacks; schema-versioned persistent state needs explicit migration functions on schema change (not specified anywhere yet) and an atomic accept-or-reject reload that keeps the old instance on failure [R3-26, R3-44, R3-76, R3-157]; timers and animation callbacks on top of the host clock `oxi_tick` [R3-39]; the host (server, wasm32 core) runs `oxiscript-runtime` and collects cycles between events (BL08). The OxiUI side (parsing the element, wiring the script into the event loop, sandboxed state keys) is `U.S1` and the "needs from OxiUI" list in [oxiui.md](oxiui.md#s-u-s1). This is the only stage allowed to add an `oxiscript-*` -> `oxiterm-*` dependency (crate direction rule [R3-51], [D-7](#d-7)).
- Gate: a demo page scripted in both modes; in local mode it works with the server stopped; a reload keeps host state; a script cannot reach anything not passed in through `main`; BAD ACTOR on capability escape and on reload during an event.
- Depends on: [8.M-link](#s-8-m-link), 8.L, 8.R, [7.1](oxiui.md#s-7-1) (done). Status: todo (no `script lang` code). Sources: [R2-62, R4-55].

<a id="s-8-s2"></a>
#### 8.S2 Editor: LSP (todo)

- Scope: `oxiscript-lsp` (new crate): diagnostics, hover, go-to-definition, rename, inlay hints (inferred integer types, later placement report), semantic tokens, CodeLens for laws, code actions from fixes, live reload from VS Code with status `oxi: live`; the decision whether the checker becomes incremental (Salsa-style queries) or stays whole-file at about 100k lines/s is made here with a measurement [R3-78, R3-244, R3-145, R3-100, R2-63, R2-104]. Honest debugger story: v0 print/trace, source maps in [8.D2](#s-8-d2).
- Gate: edit-to-reload loop measured (target under 300 ms to the simulator, under 2 s to a device; research targets, estimates); hover and rename work on the 10-program agent corpus; extension remains installable from `.vsix` with zero configuration.
- Depends on: 8.T, 8.R (done). Can start now. Status: todo (`oxiscript-lsp` absent; the extension shells out to `oxi check --json`).

<a id="s-8-d1"></a>
#### 8.D1 DevKit v1 (todo)

- Scope: `oxi dev` live reload to the browser simulator; MCP server `oxi mcp` (check, format, compile, run sandboxed and fuel-limited, explain_code, apply_fix, report, test_laws, docs_search; licence of `rmcp` read first); `create-oxi` scaffolder; `oxi doctor` basics; first-15-minutes walkthrough timed end to end; the agent test (10 programs from `AGENTS.md` only) repeated clean-room, with the apply-fix half of the loop actually exercised [R3-83, R3-231, R3-232, R3-233, R3-230, R3-238, R3-89, R3-235, R2-120]. Ideas kept in the list: `oxi migrate`, `oxi report-friction`, CI GitHub Action ([Ideas](#s-ideas), [Q-DX-8](#q-dx)).
- Tree-sitter grammar for OxiScript ([D-38](#d-38)): `tree-sitter-oxiscript` (MIT), highlights/locals/injections queries for Zed, Neovim, Helix; test: parses the whole `.oxs` corpus with the same accept/reject result as `oxiscript-syntax`.
- Gate: the walkthrough timing of every step that exists (target under 5 min from `create-oxi` to a toy, estimate); an agent given only `AGENTS.md` writes 10 programs that pass `oxi check`, fix loops counted; MCP run is sandboxed by tests; more than half of the common-delta diagnostics (`==`, `;`, `.length`, `as`) have machine-applicable fixes, measured.
- Depends on: [8.S1](#s-8-s1), [8.S2](#s-8-s2). Status: todo.

<a id="s-8-d2"></a>
#### 8.D2 DevKit v2 (todo)

- Scope: law runner and CodeLens ([8.S4a](#s-8-s4)), placement-report inlays ([8.S4b](#s-8-s4)), crate catalogue and `oxi doctor` ([8.N](#s-8-n)), `oxi flash` ([8.S3](#s-8-s3)), `oxi test` with JSON/JUnit output and shrunk counterexample as a reproducible one-liner, debugger via source maps / DWARF-in-Wasm with DAP and `oxi run --trace`, docs generator, language-level lockfile with content hashes and offline vendoring, edit-to-visible latency [R3-236, R3-245, R3-158, R3-237, R2-120].
- Gate: each tool demonstrated on the toy of the walkthrough; a breakpoint hits in the browser simulator. Depends on: 8.S4, 8.N, 8.S3. Status: todo.

<a id="s-8-s4"></a>
#### 8.S4 Laws, placement report, bug shapes, signs (todo; umbrella, three parts)

Scope of the owner-approved row ([D-11](#d-11), [D-15](#d-15), [D-16](#d-16)), split so each part is measurable. The old gate "the shape set re-finds every BAD ACTOR finding of 8.G, 8.M, 8.T and 8.L on their original repros" cannot be met as written for 8.M (and 8.C/8.P/8.R) because no stage notes exist for them anywhere in the repository (checked 2026-11-26); the amended gate for 8.S4c is: "findings recorded in `docs/stages/8.G`, `8.T`, `8.L`, `8.M-link` and in BAD ACTOR commit messages and tests (1fa4853 for 8.M) are re-found on their repros", pending owner confirmation ([Q-PLAN-1](#q-plan)) [R2 inconsistency 8].
- <a id="s-8-s4a"></a>**8.S4a Laws level 1**: `oxi test` runs `law` declarations as property tests (inputs generated from parameter types, shrinking), emitting `OXS-L0001/L0002` (registered, never emitted today); interface laws (`Ord`: antisymmetry, transitivity, consistency with `===`; hash consistent with equality) checked for every instance; 8.T rejects impure interface laws [R3-15, R3-236, R2-65, R2-83, R4-69]. Gate: laws find seeded bugs; interface laws of an `Ord`-like interface find a seeded inconsistent comparator.
- <a id="s-8-s4b"></a>**8.S4b Placement report v0**: versioned `placement.json` plus human report, one remarks API from the first optimisation pass (the shared `Remark` struct with `phase`, `executor`, `inputs`), stable reason codes of the `P` class with `assertable yes|no` (facts only get stronger within an edition), `@expect` evaluated, golden reports kept in CI, suppression only with a reason [R3-49, R3-112, R3-113, R3-48, R3-20, R4-02, R4-20]. Gate: report stable across builds; the same program gives the same codes in debug and release; an `@expect` failure is a compile error with the code.
- <a id="s-8-s4c"></a>**8.S4c Bug shapes and signs**: shapes as graph patterns over OxiIR (data flow, control flow, effects, RC ops, capabilities, traps) with "a path exists" edges; known-attack catalogue written as shapes; every BAD ACTOR finding generalised (anti-unification) and searched across the corpus; sign vocabulary (level 0 facts, level 1 signs, level 2+ hyper-signs; Datalog rule per sign) with requires/gives and vulnerability chains; mining (gSpan-style) proposes candidate hyper-signs; user-facing: a profile or `@expect` may forbid a shape. Prior art: Code Property Graphs, CodeQL, Coccinelle, MulVAL. The Physarum companion character belongs here as an idea [R2-65, R2-139]. Gate: the amended gate above; at least one cross-stage chain re-rated; one mined hyper-sign accepted by a human with a measured false-positive rate under the owner-set threshold ([Q-PLAN-2](#q-plan)); matching time linear-ish in program size.
- Depends on: 8.L (done); 8.S4b feeds [8.3b](#s-8-3b) and [8.2c](#s-8-2c). Status: todo (grammar and checker accept `law`; no runner, no report: `explain.rs` says "oxi test, 8.S4").

<a id="s-8-n"></a>
#### 8.N Rust libraries linked, with bindgen (todo)

- Scope ([D-14](#d-14), `grammar.md` 2.7): `import { X } from "rust:<crate>"`; `oxi-bindgen` (PyO3-style proc-macro) generates binding and OxiScript declarations; link through the same merge as the runtime; Rust's global allocator = `oxi_alloc`; generic APIs over user types by generated glue; borrow safety across callbacks enforced by rustc through a token (`&Oxi<'a>`, `&mut Oxi<'a>`); fixed hash seeds; tier-1 inversion (iterator/batch, index-permutation sort) as default output [R3-16, R3-155, R3-281, R3-295]; build notice per crate (version, `unsafe`, dropped guarantees) and report entry [R3-17]; profiles may forbid crates or `unsafe` crates; C/C++ compiled to Wasm without exceptions [R3-19, R3-290]; `native` bindings stay a host-operator choice [R3-18]; a how-to for building any wasm32 crate and prebuilt bindings for the persona's libraries (browser and ESP32 have no rustc); crate ranking with the hard filter (builds for `wasm32-unknown-unknown` without wasm-bindgen/js-sys/getrandom-js, no threads or OS) re-checked per crate, licences read [R3-240]; regex ReDoS decision recorded (fuel meters it) [R3-218]; `unicode-segmentation` for graphemes [R3-144]; `serde_json` as the decode path for untyped input until a decode API exists [R3-138]; the `oxi add rust:...` command and crate catalogue feed [8.D2](#s-8-d2).
- Gate: 10+ real crates linked with no hand-written glue (`regex`, `jiff`, `serde_json`, `image`, `glam`, `miniz_oxide`), including one callback-heavy API (`sort_by`/visitor) and one generic over a user type; differential tests against native Rust; build notice and report list every crate; works on server, browser local mode and a wasm2c/ESP32 build; BAD ACTOR attacks the binding layer (borrow across callback, RC of program objects held in Rust, allocator mixing, panics, oversized data, determinism leaks); size and call overhead measured with the crossing number of [8.0](#s-8-0).
- Depends on: 8.L, [8.M-link](#s-8-m-link), [8.Std](#s-8-std) (allocator contract and hashing interfaces). Status: todo.

<a id="s-8-bench"></a>
#### 8.Bench OxiPerf: performance benchmark harness `oxiscript-bench` (harness v0 done, rest todo)

Named **OxiPerf** (owner 2026-11-26); the agent usability benchmark is **OxiBench** ([8.OB](#s-8-ob)). Two parts (Opus verification 2026-11-26): **(1) the harness plus the four SIMD/GPU workloads** (blocks 8.3a/b/d); **(2) the cross-language suite**, gated on the owner approving the methodology (Q-OP-1) and, for w2c2 rows, on 8.2b. v0.1 of (2): the six Are We Fast Yet benchmarks with their upstream ports (MIT; DeltaBlue not copied until its old licence terms are cleared), headline claims only on these six, OxiScript on wasmi and Rust on the dev box; Pi 1 rows (C, wasmi, LuaJIT, Lua, CPython; w2c2 after 8.2b) inside 8.Mem; ESP32 later. Every C and w2c2 build uses `-ffp-contract=off`; JS `f32` workloads use `Math.fround`. Both described with their state in [../bench/README.md](../bench/README.md); fair cross-language method in [../research/cross-language-benchmark-methodology.md](../research/cross-language-benchmark-methodology.md) (in progress).


- Scope: harness over six targets (wasmi scalar and SIMD, Wasmtime scalar and SIMD, browser, GPU resident and round trip, native Rust baseline) and four workloads (GEMM, image filter, particles, MLP); median and p95 over at least 30 warm iterations, compile latency, `.wasm` size delta, RSS/VRAM; replaces the `[P]` estimates of the SIMD/GPU report with measurements; hosts the sort/regex/crossing benchmarks of the sync-callback research when kind-D bindings appear [R4-06, R3-292].
- Gate: one command produces a table for the targets available on the machine and records the hardware (the dev box is i5-3450 with llvmpipe; real GPU hardware: [Q-SIMD-6](#q-simd)); numbers reproducible within a stated tolerance. Depends on: 8.L (done); better after [8.M-link](#s-8-m-link). Blocks 8.3a, 8.3b, 8.3d. OxiUI bench side: [U.Bench](oxiui.md#s-u-bench). Status: harness v0 done 2026-11-26 (`oxiscript/oxiscript-bench`, binary `oxiperf`, [../bench/oxiperf.md](../bench/oxiperf.md)): wasmi, Wasmtime, Rust, C, Node; 2 AWFY + 6 own workloads; dev tier. Todo: SIMD/GPU workloads and targets (part 1), rest of the AWFY six, w2c2 after 8.2b, release tier, `compare` for 8.Lab.

### 5.4 Profiles, release path, targets

<a id="s-8-2a-ref"></a>
#### 8.2a Compilation targets: done (section 3). Remaining items split out below: memory64 server module needs an ABI bump and engine support ([8.2b](#s-8-2b), [D-13](#d-13)); no code path emits a new Wasm feature until [8.3a](#s-8-3a) [R2-100]; the wasmi `simd`/`memory64` flash cost is measured in [8.0](#s-8-0) [R2-101].

<a id="s-8-2c"></a>
#### 8.2c Profiles v0 (todo)

- Scope: `OXS-E0500` ("construct forbidden by profile") emitted; profiles `realtime` and `safety` enforced on the source ([P-3](#p-3)): vocabulary `no_alloc`, `bounded_alloc(N)`, `no_recursion`, `bounded_loops`, `no_panic`, `no_dyn`, `deterministic_float`, `no_async`, `closed_world`, `no_host(...)` and the composition rule (a `realtime` caller cannot call a non-conforming callee); `kernel` accepted only with [8.3d](#s-8-3d) [R3-20, R3-21, R3-117, R3-165, R3-168]; effect summaries computed (alloc, panic, loops, recursion) so that profiles and `@expect(no_alloc)` have something to read [R3-161, R3 inconsistency 11-13]; `@expect` enforced with `P` codes ([8.S4b](#s-8-s4)); `realtime` bans cyclic RC types or requires `weak` (reserved word) [R3-149]; canonical NaN at every store/min/max/conversion under `safety` [R3-67]; instantiation counts and `max_instantiations` knob in the report [R3-142]; `@schedule/@layout/@no_parallel` stay out of the attribute registry until [8.4](#s-8-4) [R3-110].
- Gate: every rule has a positive and a negative test; `E0500` is emitted and `llms.txt` lists it truthfully; `@expect(no_alloc)` fails a build on an allocation with a reason code; the composition rule is tested; BAD ACTOR tries to smuggle a forbidden construct through generics, closures and `@host`.
- Depends on: 8.T, [8.S4b](#s-8-s4). Status: todo (`@profile` validates its argument only; no emitter of `E0500`).

<a id="s-8-2b"></a>
#### 8.2b AOT release path (todo)

- Scope: Wasmtime/Cranelift AOT on 64-bit hosts; wasm to C (w2c2, MIT) plus the target C compiler for 32-bit boards and the likely safety route; Binaryen `wasm-opt` step adopted or rejected after measurement (the research recommended it; no stage owned it) [R3-105]; compiler-inserted budget checks at loop back-edges and calls for engines without fuel (wasm2c, browsers, ESP32 AOT) [R3-166]; differential testing interpreter against AOT as in [D-7](#d-7) and an engine feature matrix in CI [R3-154, R3-108]; `memory64` target for `server` with the ABI bump and parametrised address sites (`Mem::I32`) [D-13, R2-85, R2-108, R3-36]; **new target `micro` (owner 2026-11-26, [D-28](#d-28))**: microcontrollers always build through w2c2 + the board C compiler (no interpreter, no fuel: budget checks above), today's `device` stays for small Linux machines (Raspberry Pi, old PCs, kiosks; engine chosen by [8.Mem](#s-8-mem)); target order `micro` < `device` < `web` < `server`; manifest value, `--target micro`, `OXS-E0411`/`OXS-E0405` and the `targets` test matrix extended.0](#s-8-0) numbers [R3-46]; a Component Model adapter generated from the IDL stays an idea ([Ideas](#s-ideas)).
- Gate: the corpus gives identical results and trap kinds interpreted and AOT; a w2c2 build runs on the owner's Raspberry Pi 1 B+ ([8.Mem](#s-8-mem)); a budget check traps a runaway loop within a stated bound without wasmi fuel; a memory64 module is refused or accepted per target as the manifest says. A `--target micro` build of the corpus produces C that compiles for the ESP32-S3 (Xtensa, ESP-IDF) and gives the same results as the wasmi run of the same program on the host. A small backend trait over the maintained engines (wasmi, Wasmtime, w2c2, browser) keeps the matrix honest [R3-164].
- Depends on: [8.M-link](#s-8-m-link), better after [8.Perf](#s-8-perf). Status: todo.

### 5.5 Data parallelism: SIMD, auto-vectoriser, GPU (8.3a-e)

Design: `docs/research/oxiscript-simd-gpu-design.md` and its verification ledger (verified 2026-11-26, numbers marked `[P]` are estimates). Every slice that touches an executor has the gate "identical integer results and in-contract float results on scalar, SIMD128 and GPU, and a deterministic executor policy" ([P-4](#p-4)). Reason codes `OXS-P0103..P0112` and `P0302..P0306` are allocated append-only when each slice lands [R4-20].

<a id="s-8-3a"></a>
#### 8.3a Explicit SIMD (todo)

- Scope: builtin 128-bit `Simd<T,N>` types and `SimdMask`, vector IR ops (`Ty::Vec`), `lower_simd_to_scalar`, v128 emission on `web`/`server` targets, scalar fallback with identical semantics on `device`; ABI 1.1 `features simd128` and `WasmTy::V128` ([Q-SIMD-7](#q-simd)); wasmi `simd` on with `relaxed_simd` forced off and tested (a `relaxed_madd` module is rejected), Wasmtime `relaxed_simd_deterministic` and NaN canonicalisation; integer vectors forbid plain `+ - *` (`+% +|`, `E0110` with fix) per [Q-SIMD-1](#q-simd); dedicated code for a dynamic lane index [R4-01, R4-07, R4-08, R4-09, R4-19, R4-18, R3-34].
- Rule for the first user of any Wasm feature: it names the minimum target, emits only when the target allows, and ships a `device` lowering with identical semantics; suites run on `device` and `web` [R3-56, R3-34, R3-35].
- Gate: 10 types compile to simd128 and scalar and validate; at least 5000-case differential fuzz bit-identical across wasmi SIMD, wasmi scalar and Wasmtime; at least 2.5x on f32x4 SAXPY/matmul in Wasmtime/browser; no regression on non-SIMD code in wasmi; BAD ACTOR.
- Depends on: [8.Bench](#s-8-bench), 8.L, [8.2a](#s-8-2a), [8.Fix](#s-8-fix) (local reuse). Housekeeping inside: file the wasmi `relaxed_madd` upstream issue ([Q-SIMD-10](#q-simd)); correct the Safari relaxed-SIMD claim in `oxiscript-language.md` ([Q-SIMD-11](#q-simd)). Status: todo.

<a id="s-8-3b"></a>
#### 8.3b Auto-vectoriser v0 and layer-0 combinators (todo)

- Scope: `parallel for` un-reserved; layer-0 combinators on CPU (vocabulary fixed by a spec pass first, [Q-SIMD-18](#q-simd)); natural-loop analysis for shapes S1-S4 (AoS deferred); `OXS-P0103..P0112` remarks; `@expect(simd)` fails the build on assertable codes; trapping integer `+` never vectorised (`P0108`) [R4-02, R4-09]; sequential-equivalence CI that runs every program sequentially and in parallel and diffs [R3-111]; optional e-graph simplification only after v0 ([Ideas](#s-ideas)) [R4-15].
- Gate: 25-loop corpus gives identical codes in debug and release; auto-vectorised code reaches at least 80% of hand-written `Simd` throughput; differential fuzz; BAD ACTOR. Depends on: 8.3a, [8.S4b](#s-8-s4) (or a minimal shared `Remark` type, [Q-SIMD-3](#q-simd)). Status: todo.

<a id="s-8-3c"></a>
#### 8.3c Compute-capable GpuContext (todo; owner of the stage is this roadmap)

- Scope in `oxiterm-gpu`: `ComputeTier{GraphicsOnly,Compute}`, `GpuCapabilities`, `new_negotiated[_async]` with `Limits::downlevel_defaults().or_worse_values_from(adapter)`, public `with_error_scopes` and async twin, no raw `device()/queue()` exposure, context-owned uncaptured-error handler and counter, `GpuTextureRegistry` and `image_pipeline`; browser `wgpu` versus a thin JS WebGPU bridge decided by a one-day size spike ([Q-SIMD-5](#q-simd)) [R4-03]. The sink side (5.2 done, 5.3-5.7) is [oxiui.md](oxiui.md#s-5-3); 5.3 must expose the device through this context without breaking this API.
- Gate: lavapipe negotiates Compute and a compute shader writes an `Rgba8Unorm` storage texture drawn by `GpuSink` with zero readback matching a PNG golden; forced `GraphicsOnly` reproduces the 5.2 goldens; BAD ACTOR (handler replacement through every public API, `mem::forget` guard, OOM, device loss). Depends on: [5.2](oxiui.md#s-5-2) (done). Status: todo (`context.rs` still `pub(crate) with_error_scopes`).

<a id="s-8-3d"></a>
#### 8.3d One source for CPU and GPU (todo)

- Scope: `kernel function` un-reserved with `@workgroup/@shared/@max_iters`; crate `oxiscript-wgsl` emitting WGSL text with loop-fuel counter, bounds guards and std430 layout checks (naga only as offline validator, not a backend, per the report); `oxi.kernels` module section with WGSL and CPU versions; `Resident<T>` in `std/resident` with visible `upload`/`download`, mixing residency a compile error (hidden transfers killed C++ AMP, [P-4](#p-4)); executor = pure function of residency types, compute tier, profile, target; `@expect(gpu)` and `@expect(no_transfer)`; run-phase placement log; inline struct arrays at the start of this stage ([Q-SIMD-2](#q-simd)); reason codes `P0302..P0306`; floats on GPU are approximate, `safety` keeps float code on the CPU; the `kernel` profile enters [8.2c](#s-8-2c) [R4-04, R4-10, R4-11, R4-16, R3-114, R3-174, R3-175, R3-64, R2-74].
- Gate: naga validation 0 errors in CI; three-executor differential (integers identical, f32 within 1e-5 relative, scalar equals SIMD128 bits); executor-policy property test over at least 10,000 combinations; logged transfers equal source calls; negative `@expect(gpu)` cases; BAD ACTOR (infinite loop stopped by `_fuel` in under 50 ms, quota, device loss). Open questions: [Q-SIMD-4, 12-17](#q-simd).
- Depends on: 8.3a, 8.3b, 8.3c, [8.M-link](#s-8-m-link). OxiUI side ("paint with this kernel" `KernelDraw`): [U.KernelDraw](oxiui.md#s-u-kerneldraw). Status: todo.

<a id="s-8-3e"></a>
#### 8.3e Low-level `host:gpu`, `std/tensor`, AI demo (todo)

- Scope: `host:gpu@1` IDL for experts (`compile_kernel(kernel_index)` only from the module's own `oxi.kernels`, never guest WGSL); optional f16, `subgroupAdd` (native only), `pack4xI8`; `std/tensor` in OxiScript (seven kernel families); the AI demo of course level 5 (MNIST 784-128-10 trains live to at least 95% in under 10 s on WebGPU and under 60 s on SIMD128; tiny language model at least 20 tokens/s WebGPU and 5 tokens/s SIMD128; the numbers are `[P]` targets) [R4-05, R2-32]. `host:ml` and ESP32 PIE are ideas ([Ideas](#s-ideas)). Prior art to study for 8.3d/8.3e: CubeCL (Rust kernels compiled to WGSL/SPIR-V/CUDA), the backends of `burn`/`candle`/`tract` (MIT/Apache) for the optional `host:ml` delegation (carried over from the deleted plan, "GPU after OxiScript").
- Gate: the demo targets measured; BAD ACTOR on `compile_kernel` and buffer limits. Depends on: 8.3d. Status: todo.

### 5.6 Boards, memory, ecosystem and content

<a id="s-8-s3"></a>
#### 8.S3 First board: ESP32 (todo; hardware after 2026-12-10)

- Scope: `oxi flash` in one command to an ESP32-S3 4.3" 800x480 touch board (N16R8), fast reflash over USB per edit (no on-device interpreter, [D-27](#d-27)) and instant iteration in the browser simulator; Xtensa toolchain (`espup`, not installed); on-device engine w2c2 + the ESP-IDF C compiler ([D-27](#d-27)), budget checks from [8.2b](#s-8-2b) instead of fuel; real flash and heap numbers for [8.0](#s-8-0) part 0b; SIMD default for the board stays scalar until benchmarked ([Q-SIMD-9](#q-simd)); simulator panel for GPIO/sensors; the OxiScreens thin client (cell-diff decoder) is [U.Screens](oxiui.md#s-u-screens) [R2-64, R3-107, R3-239, R4-08].
- Simulation ([D-36](#d-36)): Wokwi support (firmware runs in Wokwi's ESP32 simulator; `wokwi.toml` + `diagram.json` per board template); our own simple browser simulator of the host capabilities only after first light on the real board.
- Gate: a toy demo on the real board; flash size and edit-to-visible latency measured. Depends on: [8.S1](#s-8-s1), hardware. Status: todo (waiting for the purchase).

<a id="s-8-s3w"></a>
#### 8.S3w Web flasher (todo)

- Scope: WebSerial flasher in the browser ([D-5](#d-5), previously ownerless) [R2-82, R3-127].
- Gate: flashing the board from a page without installing anything. Depends on: 8.S3. Status: todo.

<a id="s-8-mem"></a>
#### 8.Mem Small-memory measurements and positioning (todo)

- Scope ([D-20](#d-20)): peak RSS/heap, binary size, startup and frame time for (1) a screen over SSH, (2) OxiDE on the Linux framebuffer (CPU sink; devices below the wgpu floor), (3) core plus OxiScript runtime as a native process with a small script, (4) the browser local-mode bundle, each with the same 3-5 pages; first hardware is the owner's Raspberry Pi 1 Model B+ (ARMv6, 512 MB; reached over SSH with the owner's consent, dedicated user and key; cross-compiled for `arm-unknown-linux-gnueabihf`); also OxiScript interpreted (wasmi) versus compiled (w2c2 + gcc) on the same scripts; full-redraw cost on the Pi, which triggers [U.Damage](oxiui.md#s-u-damage) if too high; comparison table against LVGL, Slint, Flutter embedded, Qt, Electron from measurements or cited numbers [R2-70].
- VideoCore IV probe (owner consent, spare SD card; research [../research/videocore-iv-acceleration.md](../research/videocore-iv-acceleration.md) and its verification): run GPU_FFT (`hello_fft`) and pi-gemm, record sustained GFLOPS and dispatch latency, test whether disabling only the 3D block keeps the display working; check `libbcm_host` is still shipped. Promote VideoCore to a stage only if the probe beats the w2c2 CPU matrix multiply by about 5x and display + GPU coexist (or a display-less demo is accepted). Smallest demo: GPU_FFT (BSD-3) as a fixed `host:fft` capability marked `@approx` ([D-30](#d-30)), shown as a live spectrum in OxiUI next to the CPU number. The GPU tools need root: the OxiScript host stays unprivileged and calls a small privileged helper.
- Gate: numbers table in `docs/research` (RSS, binary size, startup, frame time per scenario on the Pi and on x86); the positioning claim on the sites only with these numbers. Depends on: [7.2](oxiui.md#s-7-2) (done), [8.M-link](#s-8-m-link). Status: todo.

<a id="s-8-s5"></a>
#### 8.S5 Shakedown and preview std (todo)

- Scope: port real programs and about 50 permissive libraries (JSON, dates, regex, colours, 2D/3D maths, physics, compression, image formats, ESP32 sensor drivers, real TypeScript code) with agents; every friction logged as a structured language issue (silent workarounds forbidden); differential tests against the original; every port a regression test; ports ship as unstable `std-preview`; tiny libraries (base64, hex, colour maths) preferred as ports so laws and effects cover them [R3-241]; the opt-in friction logger is a question ([Q-DX-8](#q-dx)); runs the variant analysis of [8.S4c](#s-8-s4) first.
- Gate: health metrics per run (percentage ported without workaround, linked versus ported, compile errors per 100 lines on first try, TS-to-OxiScript length ratio, performance against the original, compiler bugs found). Depends on: [8.S1](#s-8-s1), [8.N](#s-8-n). Status: todo.

<a id="s-8-w2"></a>
#### 8.W2 Playground lessons and course levels 0-2 (todo)

- Generated from the docs (owner 2026-11-26), never hand-copied: changelog page from `CHANGELOG.md`, standard library reference from `docs/oxiscript/std.md`, one page per reason code from `oxiscript-abi/registry/codes.txt` (linked from playground diagnostics), "differences from TypeScript" from the AGENTS.md template core, `/llms.txt` served as is; English with a note on the Polish site until the course is translated.
- Scope: playground runs code (Run exists, [8.Run](#s-8-run)); 3-5 level-0 lessons then levels 0-2 (Zero, Apprentice, Craftsman) in PL and EN (LA after owner review), each lesson text + editor + run + grading by tests and laws; every solution checked in CI; playground limits fixed: per-call panic isolation, live output, `${f64}` after 8.M-link, compiler messages in the page language [R2-130, R2-132]; Lighthouse and PageSpeed measurements are `U.Sites` ([oxiui.md](oxiui.md#s-u-sites)) [R2-129]. Site infrastructure (deploy, CSP, HSTS, roadmap generator) is [U.Sites](oxiui.md#s-u-sites).
- Gate: level-0 lessons pass the CI check of their solutions; works on a phone. Depends on: [8.Std](#s-8-std), [8.M-link](#s-8-m-link). Status: todo.

<a id="s-8-w3"></a>
#### 8.W3 Course levels 3-5 and OxiScript driving an OxiUI page (todo)

- Scope: levels 3 Engineer (OxiUI app, `Result`, laws), 4 Maker (capabilities, Rust crates, ESP32 simulator then board), 5 Hero (performance, perceptron, digit recogniser trained live, tiny language model; the AI part needs [8.3e](#s-8-3e)); parent-and-child track; step 3: OxiScript driving an OxiUI page live in the playground [R2-67, R2-130].
- Gate: every example and solution compiles in CI; level 3 uses [8.S1](#s-8-s1). Depends on: 8.S1, 8.N, 8.3e (level 5). Status: todo.

<a id="s-8-blog"></a>
#### 8.Blog Dev blog (todo)

- Scope: blog on oxiscript.slavekm.pl; posts only by the owner or the lead agent, each with an `author` field; the build rejects unsigned posts ([D-22](#d-22)) [R2-131]. Content infrastructure with `U.Sites`.
- Gate: build fails on an unsigned post and on a post signed by someone else's name. Depends on: 8.W1 (done). Status: todo.

<a id="s-8-pub"></a>
#### 8.Pub VS Code extension publishing (todo)

- Scope: real extension id (placeholder `oxiterm.oxiscript`), real-VS-Code-window test (never done), publishing to the Marketplace and Open VSX, version tied to the OxiScript version [R3-79, R3-80, R2-124]. Zed, Neovim and Helix come through the LSP ([Q-DX-4](#q-dx)).
- Gate: install from the marketplace on a clean profile; all quick fixes work in a real window. Depends on: [8.S2](#s-8-s2). Status: todo.

<a id="s-8-ob"></a>
#### 8.OB OxiBench: agent usability benchmark (blocked on the owner)

- Scope: `docs/research/oxibench-methodology.md` v0.1: five separated roles; at least 60 test tasks over 12+ categories, about 20 public dev tasks in `bench/dev/`, private set outside the repo with a canary GUID; conditions C0-C3 plus a TS twin; k = 5 samples with pinned model IDs; bootstrap statistics; the site quotes only frozen reports [R3-260..R3-270]. `oxi run` now exists, so C3 is feasible.
- Method details kept from the methodology: five separated roles (author, auditor of a different instance or family, deterministic harness, subjects through the plain API only, script analyst) [R3-261]; task quality gates (reference passes, mutation score at least 80%, independent solver from the prompt, no leakage, determinism under fuel) [R3-263]; conditions C0 cold, C1 docs, C2 docs plus check loop (primary, at most 3 rounds), C3 plus own runs, TS twin with `tsc --noEmit` [R3-264]; k = 5 samples, pinned model IDs, toolchain and `AGENTS.md` hash, retries not counted as failures, exactly one fenced `oxs` block [R3-265]; bootstrap over tasks (10,000), paired comparison, no result dropped, headline pass@1 in C2 and the TS gap [R3-266]; the site may quote only numbers from a frozen report with version, date and model IDs, negative results published, the lead's own model family disclosed as a subject [R3-269].
- Gate: methodology approved by the owner, a freeze commit, API keys, the first frozen report. Depends on: owner approval ([Q-OB-1](#q-ob)), [8.D1](#s-8-d1) for MCP tools. Status: blocked (no `bench/` directory).

<a id="s-8-4a"></a>
#### 8.4a Async specification (todo; before more IR hardening)

- Scope: a written spec of `async` as an inferred effect (state-machine transform, structured concurrency, cooperative cancellation) before the IR hardens further; the IR keeps its `Terminator::Await` slot; `async/await/spawn/isolate` stay reserved (`E0900`) [R3-137, R3-136, R3-05, R3-64].
- Gate: spec reviewed by the owner; a paper lowering of three examples to OxiIR without changing existing IR. Depends on: none. Status: todo (no document).

<a id="s-8-4"></a>
#### 8.4 Concurrency and ecosystem (todo)

- Scope: layer 2 of [P-4](#p-4): tasks, isolates with private heaps and message passing by move or copy, `@schedule/@layout/@no_parallel`, worker-instance browser backend, native pool; multi-core promised in v1 by the plan but not designed in the SIMD report ([Q-SIMD-8](#q-simd)) [R4-12, R3-150, R3-110].
- Gate: parallel results identical to sequential on the corpus; BAD ACTOR on isolate escape. Depends on: 8.3b, [8.2b](#s-8-2b), [8.4a](#s-8-4a). Status: todo (no row, gate or deps before this roadmap).

<a id="s-8-5"></a>
#### 8.5 Safety profile (todo)

- Scope: laws level 2 (SMT, Z3 MIT or cvc5 BSD, not Why3 LGPL linked) and level 3 (written lemmas) in the `safety` profile; `requires/ensures/invariant` contracts as runtime assertions in debug, erased in release; `no_panic` proofs; the evidence package (designed to be qualifiable, never "certified"), SBOM (`oxi sbom`), hermetic builds, signed releases; first standard and the IEC 62443 mapping [R3-169, R3-163, R3-167, R3-171, R3-124]. Questions: [Q-SAFE-1..4](#q-safe).
- The safety build drops the DX tools (reload, simulator, REPL), has no `eval`, logging is a capability and the panic policy is a host/profile setting [R3-173]. Gate: an unproven law in `safety` is a compile error; a sample component with proven laws; evidence report generated. Depends on: [8.2c](#s-8-2c), [8.S4a](#s-8-s4). Status: todo.

<a id="s-8-6"></a>
#### 8.6 Wasm exception handling (idea; nice to have)

- Scope ([D-24](#d-24)): `exnref` EH as opt-in ABI minor; keeps `Result`/`?`; use for C++ crates and `panic = "unwind"` Rust crates; check wasmi, wasm2c and browser support first; keep an EH-free lowering [R3-37, R2-75].
- Gate: identical semantics with and without the feature. Depends on: [8.N](#s-8-n), 8.2b. Status: idea.

<a id="s-8-dom"></a>
#### 8.Dom `host:dom` browser host (idea)

- Scope ([D-21](#d-21)): capability-scoped DOM with batched command lists; async browser APIs wait for 8.4; the OxiUI-streamed path stays the default [R2-69, R4-57].
- Gate: a counter and a form on a plain HTML page driven through `host:dom`, crossings per frame measured, BAD ACTOR on capability escape. Depends on: 8.4, 8.S1. Status: idea.

<a id="s-8-repo"></a>
#### 8.Repo Repository split (idea; owner decides when)

- Scope ([D-7](#d-7)): OxiScript (and possibly the websites) move to their own repositories with history (`git filter-repo`), probably under one GitHub organisation, earliest after 8.S1. Until then: no new `oxiscript-*` -> `oxiterm-*` dependency except 8.S1, separate workflows, changelogs and tags. The roadmap generator on the sites reads this directory, so the split decides where the roadmap lives ([U.Ver](oxiui.md#s-u-ver)) [R2-136, R4-70].
- Gate: both repositories build and pass their gates; sites regenerate. Depends on: 8.S1; owner. Status: idea.

<a id="s-ideas"></a>
### 5.7 Ideas, not scheduled

Nothing here has an owning stage today. Each can be promoted by the owner; the source id stays so nothing is lost.

| Idea | Source ids |
|---|---|
| Unicode identifiers (needs an edition bump, lexer-only; homoglyph policy UAX#31 + NFC + UTS#39 if ever) | R3-02, R3-102, R3-202 |
| "Transcendental magic" (owner idea, 2026-11-26): fast approximations of transcendental functions in the spirit of Quake's inverse square root (bit-trick initial guess + Newton steps; Schraudolph-style fast `exp`, minimax polynomials), as an explicit `std/math/approx` module with a documented and tested error bound per function, never a silent replacement of `std/math` (P-9, D-29 rule); cards in 8.Lab; natural fit for `micro`/`device` and for executors whose hardware already approximates (VideoCore IV SFU `recip`, `rsqrt`, `exp2`, `log2`) | owner 2026-11-26 |
| Firmware packaging with a built-in wasmi interpreter for `micro`, only for loading programs without reflashing (OTA updates, downloaded scripts, user plug-ins); not a separate target ([D-28](#d-28)) | owner 2026-11-26 |
| Host-to-guest bulk data above 4096 bytes (chunking, ABI 1.1) | R3-41 |
| Array destructuring, tuples, fixed arrays `[T; N]` (reserved syntax) | R3-04, R3-05 |
| Interfaces as value types (`dyn`, dynamic dispatch) | R3-06, R3-101, R3-142 |
| Derived structural equality on structs | R3-07 |
| Component Model adapter generated from the IDL | R3-45, R3-115 |
| Direct OxiIR-to-C backend; own bytecode interpreter as plan B (D-10 says no) | R3-153, R3-133 |
| Executable reference interpreter of OxiIR as a spec oracle | R3-162 |
| `int`/`float` aliases (not adopted, not rejected) | R3-128 |
| `unknown` plus a typed decode API for untyped input | R3-138 |
| `trusted{}` unchecked arithmetic | R3-143 |
| A browser script module sharing the core's linear memory | R3-152 |
| `oxi migrate file.ts` TS-delta report; `oxi report-friction`; CI GitHub Action; REPL; blocks editor; web debugger beyond v1 | R3-242, R3-243, R3-246, R3-172 |
| Blame-carrying trap reasons; protocol-typed handles; wasmi `ResumableCall` | R3-293, R3-294 |
| Native `sync callback` ABI 1.1 for kind-D bindings (sibling instance, frozen-region re-entry, `contract {}`, `native_effects`, `scb:` prefix, Lean/TLA+ model, gate benchmarks, BAD ACTOR list) | R3-43, R3-281, R3-282, R3-283, R3-284 (purity rules for `pure fn` closures, `#callback_mode`), R3-285 (runtime enforcement list: `in_sync_cb` flag, depth at most 2, fuel cap, poison flag, `catch_unwind` at every binding entry), R3-286 (panic policy, Q-SCB-4), R3-287 (vendored deterministic algorithms, Q-SCB-3), R3-288 (mechanisation, Q-SCB-6), R3-289 (captures, Q-SCB-5), R3-290, R3-291 (BAD ACTOR attack list for sync callbacks), R3-292, R3-295, R3-296 |
| ESP32-S3 PIE through `host:dsp`/`host:ml` or IR-to-C intrinsics | R4-13 |
| `host:ml@1` delegating ONNX/SafeTensors graphs; AI acceleration tensor library | R4-14, R2-32 |
| e-graph simplification (`egg`) after 8.3b v0 | R4-15 |
| Schedule attributes `@schedule`, `@layout(soa)`, `@no_parallel` | R3-110 |
| Package registry, notebook, WebSerial IDE beyond the simulator (deliberately not built) | R3-219, R3-247 |
| Physarum companion character for the bug-shape engine | R2-139 |
| 64-256 kB MCU class; compiler ownership and team; MoonBit licence check | R3-130, R3-125, R3-135 |
## 6. Research recommendations not adopted (or adopted only later)

Source: the founding research (`oxiscript-language.md`), known-attacks, dev-experience, sync-callbacks, SIMD/GPU reports. "Adopted later" names the stage that now owns it; "rejected" means an owner decision or a written spec sentence turns it down; "owner" means a question in section 7.

| Recommendation | Disposition | Where |
|---|---|---|
| Binaryen `wasm-opt` on emitted code | adopted later, after measurement | [8.2b](#s-8-2b) [R3-105] |
| Schedules `@schedule/@layout/@no_parallel`, structured `spawn/join`, isolates | adopted later | [8.4](#s-8-4) [R3-110, R3-150] |
| `int`/`float` aliases | not adopted, not rejected; owner | [Q-LANG-5](#q-lang) [R3-128] |
| Written async spec before the IR hardens | adopted | [8.4a](#s-8-4a) [R3-137] |
| `unknown` with checked narrowing and a decode API | idea (E0900 reserves `unknown`); JSON via crate | [Ideas](#s-ideas), [8.N](#s-8-n) [R3-138] |
| `checked_add`, `trusted{}` | `checked_add` adopted in [8.Std](#s-8-std); `trusted{}` idea | [R3-143] |
| Realtime profile bans cyclic RC types or requires `weak` | adopted later | [8.2c](#s-8-2c) [R3-149] |
| Browser script module shares the core's memory | idea | [R3-152] |
| Direct OxiIR-to-C backend, own bytecode as plan B | rejected as plan B by [D-10](#d-10); C backend idea | [R3-153, R3-133] |
| Language-level `test` blocks, docs generator, lockfile hashes, source maps | adopted later | [8.D2](#s-8-d2) [R3-158] |
| Computed effect summaries (alloc, panic, recursion, loops, async) | adopted later | [8.2c](#s-8-2c) [R3-161] |
| Executable reference interpreter | idea; differential via AOT meanwhile | [8.2b](#s-8-2b) [R3-162] |
| SBOM, hermetic builds, signed releases | adopted later | [8.5](#s-8-5), [8.Sec](#s-8-sec) [R3-163] |
| Budget checks for engines without fuel | adopted later | [8.2b](#s-8-2b) [R3-166] |
| Reserve `decreases`, `ghost`, `pure` | adopted | [8.Fix](#s-8-fix) [R3-169] |
| `#![forbid(unsafe_code)]` in embedder crates | adopted | [8.Sec](#s-8-sec) [R3-216] |
| Sanitise terminal escapes in diagnostics | adopted | [8.Sec](#s-8-sec) [R3-206] |
| Hot-reload capability-manifest diff | adopted | [8.Sec](#s-8-sec) [R3-221] |
| `oxi migrate`, `oxi report-friction`, GitHub Action, `create-oxi`, REPL, web flasher, blocks editor | `create-oxi` and flasher adopted ([8.D1](#s-8-d1), [8.S3w](#s-8-s3w)); the rest ideas / owner ([Q-DX-8, Q-DX-10](#q-dx)) | [R3-242, R3-243, R3-246, R3-233, R3-172] |
| Salsa-style incremental checker | owner / decided with the LSP | [8.S2](#s-8-s2) [R3-145] |
| Phase-0 measurements (wasmi latency, RC vs GC, wasmi vs WAMR, crossing ns) | adopted | [8.0](#s-8-0), [8.Bench](#s-8-bench) [R3-103, R3-107, R3-151, R3-292] |
| Blame-carrying traps, protocol-typed handles | idea | [R3-293] |
| IEC 62443 mapping; first safety standard | answered by [D-6](#d-6); residue in [Q-SAFE](#q-safe) | [R3-124, R3-171] |
| Sync callbacks: Tier 0/1 default, sibling instance first, ABI 1.1 additions | Tier 0/1 adopted in [8.Std](#s-8-std)/[8.N](#s-8-n); sibling instance and ABI 1.1 only for kind-D bindings (idea) | [R3-281, R3-282, R3-283] |
| SIMD report: Relaxed SIMD off by default, ESP32-S3 scalar default, integer `Simd` no plain `+ - *`, WGSL text not naga IR | adopted into the 8.3 slices as proposals; owner confirms | [Q-SIMD-1, 9](#q-simd) [R4-08, R4-09, R4-10] |
| Test suites per layer of the known-attacks lists (8.P/8.T/8.L/8.G/8.M/8.R) | partly adopted; remainder in the closure lists | [8.Fix](#s-8-fix), [8.Sec](#s-8-sec) [R3-209, R3-211, R3-212, R3-213, R3-214] |

## 7. Open questions for the owner

Ids are prefixed by source so numbers never collide. Each question names its source; none has a recorded answer unless the line says "answered".

### 7.1 Answered by an owner decision (not asked again)

Memory model prototype of both RC and GC [R3-103, R3-121] -> [D-1](#d-1) (RC chosen; [8.0](#s-8-0) measures, does not reopen). Real TypeScript or own grammar [R3-122] -> [D-2](#d-2). On-device engine wasmi against WAMR for the live engine [R3-107] -> [D-3](#d-3) (for microcontrollers superseded by [D-27](#d-27): w2c2). GPU scope in v1 [R3-126] -> [D-4](#d-4). Web flasher and blocks editor [R3-127] -> [D-5](#d-5). First safety standard [R3-124] -> [D-6](#d-6) (IEC 62443 and the EU CRA first, IEC 61508 later). Compiler ownership in this repository [R3-125] -> [D-7](#d-7) (residue [Q-SAFE-4](#q-safe)). npm positioning [R3-131] -> [D-8](#d-8). Hot reload scope [R3-132] -> [D-9](#d-9). Plan B bytecode [R3-133] -> [D-10](#d-10). Default integer width and overflow trap [R3-128]: i32 and trap, in force. UTF-8 strings without O(1) index [R3-129]: in force. Semicolons stay mandatory [Q-DX-5 of the DX research]: in force de facto. Pointer width [D-13](#d-13).

### 7.2 Open

<a id="q-simd"></a>
**SIMD/GPU report (Q-SIMD-n, source `docs/research/oxiscript-simd-gpu-design.md` section 9; all unanswered)** [R4 owner list]
- Q-SIMD-1 Integer `Simd` overflow: forbid plain `+ - *` (recommended) or allow trapping.
- Q-SIMD-2 Inline-element arrays in 8.3a or at the start of 8.3d (recommended).
- Q-SIMD-3 Placement report (8.S4b) before 8.3b, or a minimal `Remark` type early.
- Q-SIMD-4 GPU out-of-bounds semantics: trap-free by construction for the auto path; `host:gpu` keeps zero/skip plus a debug flag.
- Q-SIMD-5 Browser: `wgpu` against a thin JS WebGPU bridge (decided by a size spike at the start of 8.3c); also whether P-4's "SPIR-V/MSL via naga" tail stays.
- Q-SIMD-6 Which real GPU hardware closes the 8.3d speedup gate. The dev box has an AMD Radeon R7 260X (Vulkan via RADV, discovered 2026-11-26), which also drives the owner's display: usable with the owner's consent.
- Q-SIMD-7 Vector `trunc_sat` in ABI 1.1.
- Q-SIMD-8 Multi-core isolate layer in 8.3 or 8.4 (this roadmap: 8.4).
- Q-SIMD-9 ESP32-S3 SIMD default (scalar, provisional until the board is benchmarked).
- Q-SIMD-10 File the wasmi `relaxed_madd` upstream issue.
- Q-SIMD-11 Correct `oxiscript-language.md:320,341` (Safari relaxed SIMD).
- Q-SIMD-12 Runtime-inserted transfers above N elements (recommended: no; library `gpuMap`).
- Q-SIMD-13 Size thresholds (65536, 4096): fixed per edition or per host.
- Q-SIMD-14 `Resident<T>` name; whether the manifest declares "may use GPU".
- Q-SIMD-15 Failure of resident operations: trap (recommended) or `Result`.
- Q-SIMD-16 Width of GPU eligibility in v1 (measure on the 8.3d corpus).
- Q-SIMD-17 May the executor differ between hosts (recommended: accept, record in the report).
- Q-SIMD-18 Which layer-0 vocabulary is core (spec pass before 8.3b).

<a id="q-scb"></a>
**Sync callbacks (Q-SCB-n, source `docs/research/oxiscript-sync-callbacks.md` section 10; premise partly superseded by D-14, kept for kind-D native bindings)** [R3-281..R3-290]
- Q-SCB-1 Default Tier 0/1 inversion with `sync callback` only for irreducible cases. Q-SCB-2 Ship the pinned sibling instance first, frozen-region re-entry later behind a flag. Q-SCB-3 Vendored deterministic algorithms (merge sort) instead of Rust `std` sort. Q-SCB-4 Panic policy: `panic = "unwind"` on the server; abort-only targets refuse `panics: caught` bindings. Q-SCB-5 Class instances in callback captures. Q-SCB-6 Mechanisation (Lean 4 / TLA+ model) scope. Q-SCB-7 C++ rule: compile to Wasm; `native-unsafe` mark for a short audited list.

<a id="q-dx"></a>
**Developer experience (Q-DX-n, source `docs/research/oxiscript-dev-experience.md` section 7)** [R3-250]
- Q-DX-1 Free local simulator (virtual GPIO/sensors) instead of Wokwi. Q-DX-2 MCP server in one binary or separate/remote. Q-DX-3 Debugger timing. Q-DX-4 Editors beyond VS Code (Zed, Neovim, Helix via LSP). Q-DX-5 Semicolons mandatory (in force). Q-DX-6 Git URLs plus `rust:` enough for 1.0 or a registry. Q-DX-7 Size table for image/regex on ESP32. Q-DX-8 Telemetry (`oxi report-friction`) acceptable. Q-DX-9 Name of the DX owner. Q-DX-10 `oxi migrate` as early marketing.

<a id="q-ob"></a>
**OxiBench (Q-OB-n, source `docs/research/oxibench-methodology.md` section 10)** [R3-260, R3-262, R3-267, R3-268]
- Q-OB-1 Approve the methodology (nothing runs until approved and a freeze commit exists). Q-OB-2 Where the private test set lives. Q-OB-3 Which models, and who pays for API keys. Q-OB-4 Is condition C3 in v0.1. Q-OB-5 Agent-mode variant (Claude Code, Gemini CLI, Cursor) and AssemblyScript as a second baseline.

<a id="q-safe"></a>
**Safety and ownership (Q-SAFE-n, source `oxiscript-language.md` sections 1.4 and 13)** [R3-124, R3-125, R3-171]
- Q-SAFE-1 When to engage an assessor (the research says early). Q-SAFE-2 The IEC 62443 mapping was explicitly not researched; commission it before the sites mention it. Q-SAFE-3 Licences of any formal-verification back end (Z3 MIT, cvc5 BSD-3; Why3 LGPL must not be linked) and a re-check of the unverified licence claims. Q-SAFE-4 Who maintains the compiler long term (all work is agent-written; the DeviceScript lesson).

<a id="q-mcu"></a>
**Targets (Q-MCU-n)** [R3-123, R3-130]
- Q-MCU-1 "Write once, run on a microcontroller" as a v1 promise or "same compiler, MCU backend later". Q-MCU-2 First MCU class: ESP32 with PSRAM only (the planned N16R8 board) or also the 64-256 kB class.

<a id="q-eng"></a>
- **Q-ENG-1** ESP32 release engine: wasmi interpreter or w2c2 AOT [R2 inconsistency 11]. **Answered 2026-11-26: w2c2 ([D-27](#d-27)).**

<a id="q-lang"></a>
**Language (Q-LANG-n and the implemented-but-unsigned lists)**
- Q-LANG-1 Whitespace-sensitive `?` (try operator against ternary) is shipped; alternatives `expr!`, `try expr`, dropping the ternary [R3-08]. Q-LANG-2 Sign-off for `grammar.md` section 7 (G-1..G-6: `===` only, mandatory `;`, struct as copy-on-write heap value, TS precedence plus must-parenthesise, ASCII identifiers, braces optional after `if/while/for`) [R3-09, R3-10]; and for `contracts.md` section 4 (C-1..C-11, implemented: among them switch-expression `:` arms, own libm for float determinism (C-9), nullable scalars boxed until scalar replacement (C-10), `i32` array lengths with no `usize` (C-11)) [R3-58]. Q-LANG-3 Semantics of `readonly`, `private`, `static` on value copies (advisory or enforced; the optimiser must never assume it) [R3-210]. Q-LANG-4 Bidi and invisible characters in literals and comments: warning `W0003` today; make it an error [R3-201]. Q-LANG-5 `int`/`float` aliases and `unknown`: record a rejection or adopt [R3-128, R3-138].

<a id="q-plan"></a>
**Plan and process (Q-PLAN-n, Q-PROC-n, Q-ABI-n, Q-PERF-n)**
- Q-PLAN-1 Amend the 8.S4c gate (no stage notes exist for 8.C/8.P/8.R/8.M) [R2 inconsistency 8]. Q-PLAN-2 Threshold for false positives of mined signs ("owner-set"). <a id="q-proc"></a>Q-PROC-1 Is Gemini the default implementer for new stages (pilot done: 5.2, rtlink, std, simd, memres) [R2-157]. Q-ABI-1 Confirm the two linking decisions (heap base, trap channel) that the rtlink code already took, as an ABI 1.0 note or a 1.1 bump [R2-93]. Q-PERF-1 Accept the memres draft's allocator and collector proposals after verification.
- Open owner items carried from HANDOFF that touch OxiScript: OxiBench approval and API keys ([Q-OB-1](#q-ob)); the Polish and Latin site review and the About text are `U.Sites` ([oxiui.md](oxiui.md#s-u-sites)).

## 8. Working rules

Rules that hold for every stage of this roadmap (sources: `docs/HANDOFF.md`, `ONBOARDING.md`, `AGENTS.md`, `VERSIONING.md`, owner decisions and standing memory of the lead).

- **BAD ACTOR gate (D-7)** [R4-66: process rules: BAD ACTOR after every stage, goldens and traces blessed only by the lead, subagents run no git, at most three agents]. After every finished stage an adversarial agent tries to break it (sandbox escape, resource exhaustion, miscompile, defeating the type system); fuzzing, differential testing and adversarial programs are part of admitting a commit; every finding becomes a regression test and a line in `docs/stages/<stage>/bad-actor.md` and `fixes.md`. Applies to OxiUI and OxiScript stages alike. **The BAD ACTOR agent is Claude: Opus for large stages and releases, Sonnet with Opus consultation for small changes** (owner 2026-11-26, [D-33](#d-33)); Gemini may add its own attack pass, never replace it.
- **Gates.** `scripts/oxiscript-gate.sh quick|merge` (owned here); nightly `.github/workflows/oxiscript-fuzz.yml` with date seeds; targets matrix (`OXI_TARGET=web`) for suites; every `oxs` block in `AGENTS.md`, `llms.txt` and the skill is compiled by `cargo test -p oxiscript-cli`; CI files of OxiTerm/OxiUI belong to [U.CI](oxiui.md#s-u-ci). The lead checks CI (`gh run list`) and reports from independent evidence.
- **Flow.** Owner decides, agents implement, lead reviews; Gemini may write a stage, Sonnet with the advisor reviews it, then BAD ACTOR, then the lead merges ([D-25](#d-25)); OxiUI implementation goes to Sonnet subagents that call the advisor; subagents run no git commands; the lead blesses goldens and traces; at most three agents at once, `CARGO_BUILD_JOBS=2`, per-package cargo in worktrees (workspace-wide commands fail there on missing server assets); usage-limit stops handled with HANDOFF files and WIP snapshot commits; every agent wave is timed and the OxiScript estimates re-based on measured throughput.
- **Inform, don't forbid.** Trade-offs (unsafe crates, analysis scope, a project IDL shadowing a built-in) go into the spec and a build notice, not into bans; the notice per linked crate ([D-14](#d-14)) is the model. Dependencies of our own embedder crates are the exception handled in [8.Sec](#s-8-sec).
- **Permissive licences only** ([P-11](#p-11)): MIT/Apache/BSD/ISC/Zlib/Unicode; no GPL/AGPL/LGPL; MPL only if recorded; fonts OFL/Apache. Moddable XS, Toit VM and Jaculus are references. `cargo deny` is [U.CI](oxiui.md#s-u-ci).
- **Reason codes are append-only**: never renumber, reuse or change the meaning; every new code gets an entry in `codes.txt` and a test that triggers it; a diagnostic with a mechanical repair carries a `Fix` (syntax-level fixes `machine-applicable`).
- **Versioning ([D-23](#d-23)).** Version 0.0.x, nothing guaranteed before 1.0.0; the patch number grows by 1 per 5555 changed lines in `oxiscript-*/src` (`scripts/oxiscript-version.sh`, gate step `version-check`, annotated tag `oxiscript-vX.Y.C` with `carry=N`); x and y by the owner for milestones; one version for all `oxiscript-*` crates, the CLI, the VS Code extension and the playground; each release updates the two JSON snapshots in `oxiscript-cli/tests/snapshots` and the CHANGELOG. OxiTerm/OxiUI are versioned separately ([U.Ver](oxiui.md#s-u-ver)); the CI tag trigger so that `oxiscript-v*` can never publish an OxiTerm release is [U.CI](oxiui.md#s-u-ci).
- **Verify before claiming a gap** (grep the repo and `ARCHITECTURE.md`); code beats documents; a safety check must have a test that hits the failing branch (`ONBOARDING.md`).
- **Language.** The owner writes Polish and gets Polish answers; code, documents and commits are English.
- **Crate direction.** No `oxiscript-*` crate depends on an `oxiterm-*` crate except at [8.S1](#s-8-s1); separate workflows, changelogs and docs directories ([D-7](#d-7)).
- **Deploy.** Sites are Cloudflare Workers static assets deployed by the lead with wrangler after the gate; dev blog authorship per [D-22](#d-22); TMPDIR and scratch on `/vectorlegis_ssd_pool/oxiterm/scratch`, never `/tmp` (the root disk is nearly full).
- **Doc sync.** The core block of `AGENTS.md` is copied from `oxiscript-cli/templates/common/AGENTS.md` by `tools/sync_docs.py`; edit there.

## 9. Coverage ledger and risks

Every inventory item whose product is OxiScript or both is either a stage scope above, an idea in section 5.7, a question in section 7, a decision in section 2, a done row in section 3, or one of the lines below. Items that belong to the OxiUI half are in [oxiui.md](oxiui.md); items that are document fixes are in [consistency.md](consistency.md). The mechanical check (extract the ids from the four inventories, grep them in this file, `oxiui.md` and `consistency.md`) was run on 2026-11-26 and is repeated in the report to the lead.

Invariants and specs in force, no task: cyclic imports are an error, no `export default`/re-export/namespaces, default parameters may not reference other parameters [R3-27, R3-28, R3-29]; non-trapping `trunc_sat` not allowed on any target [R3-35]; host never re-enters the guest during an import, bulk payloads at most 4096 bytes [R3-41, R3-42]; checker bounds list (32 type levels, 400k types, 64 union variants, expression depth 120, 100 errors) [R3-54]; extension-point table of `contracts.md` (each row is owned by the stage named in it: kernel and SIMD by 8.3, threads and isolates by 8.4, AOT by 8.2b, EH by 8.6, contracts/SMT by 8.5, placement and `@expect` by 8.S4b/8.2c) [R3-55]; the agent-guidance gaps found by the agent test were folded into `AGENTS.md` [R3-84]; every `oxs` block is compiled by `cargo test -p oxiscript-cli` [R3-87]; MVP grammar implemented [R3-01]; OxiIR plus Core Wasm as primary backend, wasmi as live engine, closure layout `{fn_index, env}`, `as` split into numeric `as` and `downcast`, drop list of TypeScript features, written evaluation order and defined overflow [R3-104, R3-106, R3-118, R3-139, R3-146, R3-160]; licence policy ([P-11](#p-11)) [R3-120]; domain-free core and the rocket test ([P-1](#p-1)) [R3-156]; "avoid early" list of the founding research: the two violated items are the LSP ([8.S2](#s-8-s2)) and RC elision ([8.Perf](#s-8-perf)) [R3-119]; persona DX is partial until 8.S2, 8.D1, 8.S3 land [R3-116]; SIMD research layers 0-2 are [8.3a](#s-8-3a), [8.3b](#s-8-3b), [8.4](#s-8-4) [R3-109]; out of scope initially: Component Model, WasmGC backend, own native backend, blocks editor, refinement types, auto-offload heuristics [R3-178]; "faster than tsc" is not a differentiator [R3-251]; the P1-P3 safety framing (host safety, guest memory safety under re-entry, determinism) governs any native binding [R3-280].

Items owned by the OxiUI half and cited there by content or id (R2-95 CI tag trigger, R2-135 flaky CSP test, R2-142 site vendoring, the 5.x/6.x/7.x stage notes, R1 App Server and reader items) are not repeated here; the id-level check over all four files lists 61 inventory ids that appear in none of the three roadmap files, all OxiUI-owned and reported to W-UI.

Document-only items: HANDOFF and plan status lines stale [R2-159, R2-165]; README says "run next" while Run is live [R1-111]: see [consistency.md](consistency.md).

Risks (from the founding research, still valid): scope; memory model wrong (mitigated by [8.0](#s-8-0), [8.Perf](#s-8-perf)); wasmi translation latency; wasm-in-wasm slowness in the browser; SIMD semantics leaking into the language (mitigated by the identical-results gate); "feels like TS" erosion (measured by [8.S5](#s-8-s5)); licence taint; safety-claim overreach (never claim certification); compiler size inside the core (the checker builds for wasm32); agent-written code with no human maintainer ([Q-SAFE-4](#q-safe)); usage-limit stops (WIP snapshot commits) [R3-179].

Covered by content in the sections above without an inline id (the item is the stage, decision or rule it names): plan header status, execution order, delegation and process rules, shared GPU context, Run/identity stages, owner goals, principles, decisions 1-13, critical path, parallel schedule, wave log, risks, [R2-01, R2-09, R2-10, R2-21, R2-31, R2-51, R2-52, R2-53, R2-76, R2-78, R2-80, R2-81, R2-87, R2-156, R4-39, R4-67, R4-68, R4-71, R1-30, R1-37, R1-40, R1-26]; stage rows [R2-61, R2-66, R2-71, R2-72, R2-90, R2-161, R4-53, R4-54, R4-56, R4-58, R4-59, R4-60, R4-61, R4-62]; reuse table [R2-79, R4-65] in section 10; OxiScript facts of the README and CHANGELOG [R1-32, R1-33, R1-34, R1-35, R1-36, R1-42, R1-50, R1-56, R1-98] in sections 3 and 5; versioning split [R3-74] in section 8.
## 10. Reuse before writing (owner, 2026-11-25)

Solved engineering problems are taken from existing permissive crates; the language and its semantics stay ours. Each crate is checked at adoption (licence, size, wasm32 build). Light dependencies may enter the core (live compiler in the browser/device); heavy ones stay in the developer toolchain. Every dependency is an SBOM entry (CRA / IEC 62443). Additions from the SIMD/GPU report: `naga` (CI validator only), w2c2 with SIMDe, `half`, `rayon` (8.4), esp-dsp/esp-nn (ideas). The memres draft proposes `rlsf` or `talc` for the allocator ([8.Perf](#s-8-perf)).

| Need | Reuse | Slice | Where |
|---|---|---|---|
| Type inference (unification) | `ena` (rustc's union-find): not used, 8.T has no inference variables (reason in `docs/stages/8.T/implementation.md`) | 8.T | core |
| Incremental queries shared by compiler and LSP | `salsa` (rust-analyzer) — evaluate first | 8.T / 8.S2 | toolchain |
| Diagnostic rendering | `ariadne` or `codespan-reporting` | 8.T+ | toolchain |
| Language server | `lsp-server` + `lsp-types` | 8.S2 | toolchain |
| Wasm emission and validation | `wasm-encoder` + `wasmparser` | 8.G | core |
| Wasm transforms / module merging | `walrus` | 8.G / 8.L | toolchain |
| Release optimisation | Binaryen `wasm-opt` | 8.2b | toolchain |
| Guest allocator | `dlmalloc` or `talc` compiled to Wasm, under our RC + cycle collector | 8.M, 8.Perf | module |
| Adversarial testing | `wasm-smith`, `arbitrary` | all BAD ACTOR passes | dev only |
| E-graph optimisation | `egg` / `egglog` (after 8.3b v0, idea) | 8.3b+ | toolchain |
| Bug-shape matching (graph queries over OxiIR) | Datalog in Rust: `ascent` / `crepe` / `datafrog`, or `egglog` shared with 8.3 — evaluate | 8.S4c | toolchain |
| Rust library bindings (proc-macro codegen) and linking | `syn` + `quote` + `proc-macro2`; `wasm-ld` (LLVM, Apache-2.0 with LLVM exception) or `walrus` for linking | 8.N | toolchain |
| SMT for laws level 2 | `z3` / `cvc5` | 8.5 | toolchain |

Not reusable as code: the cycle collector for RC in Wasm linear memory (algorithms from Nim ORC and Koka Perceus) and the type system itself.

## 11. Wave log (history, measured; wall-clock excludes usage-limit pauses)

Kept from the old plan because the timings are measured and exist nowhere else; used to re-estimate OxiScript from measured throughput. Rows for 7.1 and 7.2 are shared with [oxiui.md](oxiui.md). Rows missing from the old log (8.2a targets, 8.Run, 8.Id, showcase, 8.Rel, 5.2, 7.6) were not timed.

| Slice | Implementation | BAD ACTOR | Fixes | Findings | Merged |
|---|---|---|---|---|---|
| 8.C Contracts | 31 min (+3 min lead amendment) | ~15 min | 21 min | 28 (verifier crash, pointer forging, RC use-after-free in the spec, grammar ambiguities) | 2026-11-25, `2ba1aa0` |
| 7.1 WASM local mode | 41 min | ~17 min | ~29 min | 8 (incl. a pre-existing server abort via a 65535×65535 resize) | 2026-11-25, `77f9002` |
| 7.2 OxiDE sink | 30 min | ~10 min | ~1 h incl. 200k-case fuzz run | 21 (NaN/size crashes, f32-counter hangs, uncapped memory, device-guard bypass) + 1 found during fixes | 2026-11-25, `78fcdb5` |
| 8.P Parser + formatter | ~45 min | ~20 min | (not logged) | 16 (formatter changes meaning/breaks code with comments, ~3 KB stack per nesting level, Trojan-Source escapes, quadratic comments) | 2026-11-25, `13b5987` |
| 8.R Runtime | ~26 min | ~25 min | (not logged) | 7, no sandbox escape (sticky revocation, pump budget, subscription scan DoS, guessable handles, shared-store quota, log injection) | 2026-11-25, `d007c74` |
| 8.T Type checker | (not logged) | (not logged) | 2 agents: first stopped by usage limit, resume ~35 min | 22 (17 severe: uninitialised globals/statics, `never` falling off, private bypass, law purity bypass, polymorphic recursion, lossy fix texts) + 7 catalogue | 2026-11-25, `3cbf1b4` |
| 8.G Codegen | (not logged) | (not logged) | agent stopped by usage limit after F1–F6; lead finished verify + clippy | 6 (NaN not canonicalised at exports, quadratic IR verifier, locals limit above engine, ptr_count truncation, raw-ref exports, loop limit), 0 miscompilations | 2026-11-25, `bad1e94` |
| 8.M Memory | (not logged) | (not logged) | (not logged) | see docs/stages (notes not archived) | 2026-11-25, `5a5929d` |
| 8.L Lowering | ~2 h incl. a usage-limit stop | ~16 min | ~40 min | 10 (2 RC leaks, per-iteration loop bindings vs TS, null narrowing, deep-drop recursion, limits); 0 miscompilations in ~19.6k differential programs | 2026-11-26, `c792b4f` |
| 8.D DevKit v0 | ~29 min | ~22 min | ~17 min | 17 (import root escape, write through symlink, /dev/zero and FIFO DoS, `needs` not enforced, workspace-controlled binary path) | 2026-11-26, `148b8e8` |
| 8.W Websites step 1 | ~25 min + usage-limit stop | ~7 min | ~18 min | 12 (no CSP, stale-fix index, placeholder text published, DNS keep-list, contrast) | 2026-11-25, `0df8681` |


Wave 1 (7.1, 7.2, 8.C) closed 2026-11-25. Usage limits, not agent speed, set the pace: two pauses (2h10 and about 4h) in the first nine hours of wall-clock time.
