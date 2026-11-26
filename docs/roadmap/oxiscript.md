# OxiScript roadmap

Single roadmap of OxiScript: the language, checker, IR, code generation, runtime and memory, standard library, DevKit (CLI, LSP, VS Code), laws and reports, Rust crate linking, targets and profiles, SIMD/GPU, concurrency, safety, the course and the OxiScript websites content. It replaces the OxiScript parts of `docs/oxi-ui.md` (Phase 8), `docs/HANDOFF.md`, the planning text of `docs/stages/**` and the "next steps" of the research documents. The OxiUI half is [oxiui.md](oxiui.md); the index and the id scheme are in [README.md](README.md); contradictions found while merging and the list of documents to rewrite or delete are in [consistency.md](consistency.md).

Written 2026-11-26 against `main` at 19cf8cd (plus the three unmerged Gemini branches named in section 4). Every status in sections 3 and 4 was checked against git or code; statuses in section 5 are marked `verified-absent`, `partial` or `todo` with the check named. Source ids like `[R3-216]` point to the reader inventories (R1 concept docs, R2 plan and stage notes, R3 OxiScript specs and research, R4 SIMD/GPU and second reading of the plan); they are kept so that nothing is lost and each can be traced (see the coverage ledger, section 9).

## 1. Vision and positioning

OxiScript is a general-purpose, statically and soundly typed language that a TypeScript developer reads without learning it, compiled to Core WebAssembly. One program runs two ways: live (compiled in milliseconds inside the core, hot reload) and compiled for release (native AOT on 64-bit hosts, wasm to C for 32-bit boards, the browser's own engine on the web). OxiUI is its first host, not its limit: the owner's test case is that someone can later write safety-critical software (a rocket) in it. The language has no domain concepts; UI, real time, safety and GPU are profiles, libraries and host APIs ([p-1](#p-1), [p-3](#p-3)).

Positioning (owner 2026-11-26, [d-20](#d-20)): GUI and scripting for devices with megabytes, not gigabytes, of memory (industrial panels, kiosks, home automation, old PCs, toys on ESP32), next to the server-streamed role of OxiUI. The claim goes on the sites only with measurements ([8.Mem](#s-8-mem)). Primary persona: a JS/TS frontend or Node developer who builds a toy with a child on an ESP32: one-command flash, live reload, browser simulator, VS Code ([8.S3](#s-8-s3), [8.S2](#s-8-s2), [8.D1](#s-8-d1)). The second persona is an AI agent: unambiguous grammar, stable reason codes with fixes, `llms.txt`, a deterministic compiler ([d-12](#d-12)).

The two products meet in one place, the stage [8.S1](#s-8-s1) (`<script lang="oxs">` in OxiUI pages), which is the end of the critical path and the earliest moment the repository may be split ([d-7](#d-7), [8.Repo](#s-8-repo)).

## 2. Principles and owner decisions

Numbering scheme (one, used by both roadmaps for OxiScript references): principles `P-1..P-11` and owner decisions `D-1..D-13` are numbered exactly as in the old `docs/oxi-ui.md` Phase 8 text; later owner decisions continue as `D-14...`. Anchors are `#p-N` and `#d-N`. In the old documents the word "decision 4" was used for two different things: the SIMD/GPU research ("decision 4 transparency", "revised for decision 4") means principle 4 = [P-4](#p-4) (data parallelism is general, the GPU is a backend, residency visible in types), while the old decision 4 = [D-4](#d-4) is "GPU in v1: no". Mapping for readers of old text: `decision 4 (transparency)` -> `P-4`; `decision 4 (GPU in v1)` -> `D-4`; `decision 1 / Stage 8.0 measures it` -> `D-1` + `8.Perf` ([consistency.md](consistency.md)).

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
- P-5 (placement report, `@expect`) is only partly built: reason codes, `P` class registry and spans exist; no report is emitted ([8.S4](#s-8-s4), [8.2c](#s-8-2c)).

### 2.3 Owner decisions 1-13 (2026-11-24, D-13: 2026-11-25; D-7 revised 2026-11-26)

<a id="d-1"></a>**D-1.** **Memory model:** reference counting with compiler elision (Perceus-style) plus a cycle collector (Nim ORC-style). GC-like convenience for closures in event handlers, mostly deterministic freeing; the `realtime`/`safety` profiles forbid allocation on hot paths. Stage 8.0 measures it (counter overhead, atomics across threads mitigated by per-isolate data, code size on ESP32) but does not reopen the choice.

<a id="d-2"></a>**D-2.** **Grammar:** our own grammar, files `.oxs`, designed so that a TypeScript developer simply *reads* it without learning it (same keywords, braces, arrow functions, `import`/`export`, type annotations after `:`; differences only where soundness requires them, and then with familiar spellings). Real TypeScript is not parsed (no accepting syntax we then reject).

<a id="d-3"></a>**D-3.** **On-device live engine:** wasmi (pure Rust like the core, fastest interpreter in the treVM measurements). Its larger flash footprint is accepted.

<a id="d-4"></a>**D-4.** **GPU in v1:** no, and the `kernel` profile is not in the MVP either. But the language must be designed so that adding it later is easy, not hard: no feature in the MVP may make data-parallel kernels or device buffers awkward to add (the GPU backend comes in 8.3d).

<a id="d-5"></a>**D-5.** **Tools in v1:** web flasher (WebSerial) yes; blocks editor later.

<a id="d-6"></a>**D-6.** **First standards target:** IEC 62443 and the EU CRA (the business model); IEC 61508 as the first functional-safety standard, later.

<a id="d-7"></a>**D-7.** **Ownership and method:** the OxiUI project owns the compiler, in this repository (`oxiscript-*` crates) **for now** (*revised by the owner 2026-11-26: OxiScript, and possibly the websites, will move to their own repositories, preserving history (`git filter-repo`), probably under a shared GitHub organisation. **The earliest possible time is after 8.S1**, when the OxiUI-OxiScript interface (`<script lang="oxs">`) has settled; it is not scheduled right after 8.S1 and happens only when the owner decides. Until then: keep the two halves separable inside this repo (no new `oxiscript-*` -> `oxiterm-*` dependency except the 8.S1 host interface, separate CI workflows, changelogs and docs directories, separate release tags `oxiscript-v*` / `v*`)*), with the same workflow as the rest (owner decides, agents implement, lead reviews). **After every finished stage a BAD ACTOR agent runs** whose only goal is to break it: escape the sandbox, exhaust resources, crash the compiler or runtime, make it miscompile, defeat the type system. Fuzzing (compiler front end, IR, Wasm output, runtime host API), differential testing (interpreter vs AOT) and adversarial programs are part of admitting a commit, not an afterthought; whatever the bad actor finds becomes a regression test.

<a id="d-8"></a>**D-8.** **npm:** the idea, not compatibility: naming and semver conventions and a `package.json`-like manifest; npm packages are not promised to run.

<a id="d-9"></a>**D-9.** **Hot reload state:** module-local state resets on reload; persistent state lives in the host's StateManager.

<a id="d-10"></a>**D-10.** **No plan B bytecode:** WebAssembly is the only IR backend and distribution format.

<a id="d-11"></a>**D-11.** **Laws (inspired by Bend 2's `LAWS.bend`, without its dependent types).** A `law` is a first-class declaration of a property the code must satisfy, written in ordinary OxiScript and readable by a TS developer:

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
<a id="d-12"></a>**D-12.** **AI-friendliness as a design goal:** an unambiguous grammar; error messages with stable reason codes and a suggested fix, readable by people and models; a stable specification with model-oriented docs (`llms.txt`); the placement report and `@expect`; laws as machine-checkable targets for agents; a deterministic compiler, so an agent can rely on its own experiments.

<a id="d-13"></a>**D-13.** **Pointer width is a target parameter (owner question, 2026-11-25).** v1 targets wasm32: 32-bit addresses and lengths in the ABI (4 GiB of linear memory per module), while `i64`/`f64` arithmetic is native 64-bit. Release builds are still native 64-bit machine code on 64-bit hosts (Wasmtime AOT, or wasm→C then the native compiler); only the per-module heap is 32-bit addressed, which is also what ESP32-class 32-bit MCUs need. Programs that need more than 4 GiB get a later `memory64` target (part of Wasm 3.0): pointer width stays a codegen/ABI parameter, never a deep assumption, and the switch is an ABI version bump, not a language change.


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
| <a id="d-23"></a>D-23 | 2026-11-25 | Versioning: first public version 0.0.1, nothing guaranteed before 1.0.0; the patch number `c` grows by 1 per 5555 changed lines in `oxiscript-*/src`; x and y only by the owner for milestones; tag `oxiscript-vX.Y.C`. | `docs/oxiscript/VERSIONING.md`, commit 8dd11c1 |
| <a id="d-24"></a>D-24 | 2026-11-25 | Wasm exception handling (`exnref`) is "nice to have, later": an opt-in ABI minor version, never required by the language, keeping a lowering without EH. | old plan "Later stages" |
| <a id="d-25"></a>D-25 | 2026-11-26 | Implementation flow: Gemini may write a stage, Sonnet with the advisor reviews it, then BAD ACTOR, then the lead merges. (HANDOFF; pilot done with 5.2, rtlink, std, simd, memres.) Whether Gemini becomes the default implementer is still open ([Q-PROC-1](#q-proc)). | `docs/HANDOFF.md` |
| <a id="d-26"></a>D-26 | 2026-11-25 | Sites in Polish and English first; Latin only after the owner reviews it (in memoriam of his Latin teacher); identity: rust orange + bronze patina, "Oxi cell" mark, mascot Oxi the axolotl. | old plan row 8.W |

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
| <a id="s-8-r"></a>8.R Runtime | wasmi 2.0.3 host, IDL capabilities (`host:log`, `host:state`), fuel, `StoreLimits`, poisoning, instance-swap reload, `oxi_tick` | merge d007c74; crate `oxiscript-runtime` |
| <a id="s-8-m"></a>8.M Memory (crate) | `oxiscript-rc`: RC runtime, deferred destruction queue, Bacon-Rajan cycle collector, RC-balance checker, `oxi_str_from_f64` behind feature `float-fmt`. Real linking into programs is not part of it: see [8.M-link](#s-8-m-link) | merge 5a5929d; BAD ACTOR commit 1fa4853 |
| <a id="s-8-t"></a>8.T Type checker | Sound checker, Maranget exhaustiveness, interned types, budgets (`E0113`), effect summary (`EffectSummary`, partial), laws purity (`E0125`), typed output contract | merge 3cbf1b4; `docs/stages/8.T`; crate `oxiscript-check` |
| <a id="s-8-g"></a>8.G Codegen | OxiIR to Core Wasm, validated, run in wasmi; strict-engine limits (`E0603`, `E0704`, `E0408`) | merge bad1e94; `docs/stages/8.G`; crate `oxiscript-wasm` |
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

One row per stage; the status column uses only `done`, `in-progress`, `todo`, `idea`; the last column is the merge hash or empty. Parsed by `sites/build.mjs` replacement work (owned by `U.Sites` in [oxiui.md](oxiui.md)).

| Stage | Status | Merged |
|---|---|---|
| 8.C | done | 2ba1aa0 |
| 8.P | done | 13b5987 |
| 8.R | done | d007c74 |
| 8.M | done | 5a5929d |
| 8.T | done | 3cbf1b4 |
| 8.G | done | bad1e94 |
| 8.L | done | c792b4f |
| 8.D0 | done | 148b8e8 |
| 8.W1 | done | 0df8681 |
| 8.Run | done | 6dda5de |
| 8.Id | done | 6dda5de |
| 8.HX | done | 1b08076 |
| 8.Rel | done | eed8d35 |
| 8.2a | done | d8fb0f8 |
| 8.M-link | in-progress | |
| 8.Std | in-progress | |
| 8.Perf | todo | |

(The remaining stages of section 5 are all `todo` or `idea` and are listed with their status in each stage block.)

## 4. In progress

Three unmerged branches carry work; all other `worktree-agent-*` branches are fully merged (0 commits ahead of `main`, see `consistency.md` for cleanup). Checked with `git log main..<branch>` on 2026-11-26.

| Branch (worktree) | Stage | State | What is missing before merge |
|---|---|---|---|
| `gemini-rtlink` (3 commits: c5d31f4 feature, da87be6 BAD ACTOR tests, 0cf56d3 merge of targets) | [8.M-link](#s-8-m-link) | Gemini is fixing BAD ACTOR findings BL01-BL10 (`docs/stages/8.M-link/bad-actor.md` on that branch; 18 uncommitted modified files in the worktree) | Fixes of BL01, BL03, BL04, BL06 (as input to 8.Perf), BL08, BL09, BL10; owner/lead decision on the two ABI questions (heap base, trap channel) recorded in `abi.md`; gate run; merge |
| `gemini-std` (2 commits: 90fa662 pre-review snapshot, b416210 BAD ACTOR tests and report) | [8.Std](#s-8-std) | BAD ACTOR findings BS01-BS08 being fixed by Gemini (report on that branch) | Rebase on `main` (the branch base predates the targets merge; merging it as is would delete `oxiscript-wasm/tests/targets.rs` and a gate step), wait for 8.M-link (f64 text), review, std module resolution from a bundled directory |
| `gemini-memres` (1 commit e4b9f7d "Gemini draft, unverified") | input of [8.Perf](#s-8-perf) | `docs/research/oxiscript-memory-performance.md` (about 470 lines, plus `oxiscript-runtime/src/runtime.rs` +17 lines) committed on the branch; Opus verifying | Verification pass (claims marked `[estimate]`/`[unverified]` in the document), merge of the document to `main` so that links to it work; the 17-line runtime change must be reviewed separately |

Excerpt of what the memres draft recommends (to be verified, not adopted yet): a slab plus TLSF/talc allocator for small objects, inline Wasm fast paths for `retain`/`release`/`alloc`, Perceus-style elision and reuse, scalar replacement, a collector that allocates nothing. Target stated: 1.1-1.8x of the bump stub fuel against the measured 6.5-10.5x. These numbers are the draft's estimates.

BAD ACTOR findings of 8.M-link, condensed (from the branch file, for tracing into other stages): BL01 collector needs heap headroom (OOM in `deinit`); BL02 cycles inside one host call never collected, OOM path does not collect first; BL03 exported `oxi_rt_init` accepts a heap base inside program data; BL04 embedded runtime contains the builder's absolute cargo path (reproducibility and privacy); BL05 `build.rs` rerun list and nested cargo flags; BL06 fuel 6.5-10.5x of Stub, default 10M fuel allows about 3k allocations per call instead of 25k; BL07 module 58-60 KB versus 3.5-5 KB, compile 3.4-4.5 ms versus 0.7-2.2 ms; BL08 nothing in `oxiscript-runtime` ever calls `oxi_collect_cycles`; BL09 `${f32}` prints the widened f64 text; BL10 lower tests now run Linked only and the debug-only `stale_object` check is absent. BL06 and BL07 are owned by [8.Perf](#s-8-perf), BL08 by [8.M-link](#s-8-m-link) (host collects between events), BL04 by [8.Sec](#s-8-sec), BL10 by [8.M-link](#s-8-m-link).

