# OxiScript Stages 8.3a-8.3e (SIMD, Parallel and Transparent GPU Execution): Research & Architecture Design

> [Revised 2026-11-26 for decision 4 transparency] Title and stage names changed; the original title was "Stages 8.3 (SIMD & Parallel) and 8.3b (GPU Kernels)".

> **Draft by Gemini; verified and corrected by Claude (Opus) on 2026-11-26. See the verification ledger: [oxiscript-simd-gpu-design-verification.md](oxiscript-simd-gpu-design-verification.md).** Every correction is marked inline as **[Corrected 2026-11-26: ...]**. Read the ledger's section A (10 findings that change the design) before using this report.
>
> **Stage naming. [Revised 2026-11-26 for decision 4 transparency]** `docs/oxi-ui.md` ("Later stages") now uses this report's five slices directly, so there is no mapping any more:
> * **8.3a** explicit SIMD; **8.3b** auto-vectoriser, layer-0 combinators on CPU, placement report; **8.3c** `GpuContext` compute upgrade and isolation; **8.3d** one source to CPU and GPU (kernel profile, WGSL variants, residency types, executor choice); **8.3e** low-level `host:gpu`, `std/tensor`, AI demo.
> * The plan's former "8.3" is 8.3a-8.3b; its former "8.3b GPU kernels" is 8.3c-8.3e. Older text that says "8.3b GPU" (`oxi-ui.md` sections on 3D and the shared GPU device, the research notes) means 8.3d unless it is about the shared device (then 8.3c).
> * "Stage 8.3b" in this report means the auto-vectoriser slice.

> **[Revised 2026-11-26 for decision 4 transparency] Design alignment.** The first version made GPU use explicit and low-level (guest code compiles kernels, allocates buffers and dispatches through `host:gpu@1`). `docs/oxi-ui.md` Phase 8 decision 4 says the opposite: *data parallelism is general, GPU is a backend; parallel and SIMD constructs have the semantics of sequential code; device residency and transfers are visible in types.* This revision therefore (1) adds section 4.6 "One source, several executors" as the normal GPU path, (2) demotes `host:gpu` (section 4.4) to a low-level capability for libraries and experts, (3) rewrites the staged plan (section 7.2) and the open questions (section 9). Safety, determinism and target findings are kept unchanged (every inline correction marker from the verification pass is still in place; the revision adds only `[Revised 2026-11-26 for decision 4 transparency]` markers). Where a statement below still describes explicit dispatch as the default, section 4.6 wins.

> **Status:** Design specification (research stage, 2026-11-26). No product code changed.
> **Scope:** Stage 8.3 (SIMD types, auto-vectorisation, `@expect(simd)`, placement report) and Stage 8.3b (`kernel` profile, OxiIR → WGSL, `host:gpu`, shared `GpuContext` integration with `oxiterm-gpu` Stage 5.2, AI/ML foundation, benchmarks, and staged delivery plan).
> **Revised scope [Revised 2026-11-26 for decision 4 transparency].** Adds: layer-0 data-parallel combinators and `kernel` functions compiled to a CPU version always and a WGSL version when eligible, a `Resident<T>` type for visible residency, a deterministic executor policy, `@expect(gpu)` and runtime placement remarks.
> **Evidence tags:** **[V]** verified directly in local repo/crate sources (`~/.cargo/registry`, `Cargo.lock`) or primary upstream specifications; **[S]** per web search summary of cited external source (verify before relying on exact numbers); **[P]** project design estimate; **[O]** architectural recommendation/opinion; **[U]** unverified / open unknown.

---

## Executive Summary & Recommendations

OxiScript's core compiler pipeline (`8.C`–`8.L`, `8.M`, `8.R`) and `oxiterm-gpu` (`5.2`) are already in place and deliberately prepared for data parallelism. This report designs **Stage 8.3** (explicit SIMD, predictable auto-vectorisation, placement reporting, `@expect(simd)`) and **Stage 8.3b** (the `kernel` profile, OxiIR → WGSL, `host:gpu@1`, and shared-device integration with `oxiterm-gpu`), keeping OxiScript readable by TypeScript developers, deterministic by default, and permissive-only in licensing.

### Key Architectural Decisions Recommended

| Area | Recommendation | Rationale |
|---|---|---|
| **1. Wasm SIMD target** | Target **Wasm fixed-width SIMD128** (ABI minor bump `1.1`, opt-in per target profile); keep **Relaxed SIMD off by default** (explicit opt-in only under non-deterministic profiles). | Supported in `wasmi 2.0.3` (behind the `simd` Cargo feature, no cost on non-SIMD code per the wasmi 2.0.0 CHANGELOG **[V]**), Wasmtime/Cranelift (Tier 1), `wasm2c`/`w2c2` (via MIT-licensed SIMDe), and all major browsers. Relaxed SIMD breaks cross-target determinism (`ir.md` §6). **[Corrected 2026-11-26: `wasm_relaxed_simd(false)` is mandatory, not optional. wasmi's `relaxed_madd` is *fused* even with `deterministic`, while the Wasm 3.0 deterministic profile requires *unfused* (ledger A4). Relaxed SIMD in Safari is still behind a JSC flag (ledger B21).]** |
| **2. ESP32-S3 PIE SIMD** | Do **not** rely on `wasm2c`/GCC auto-vectorisation for Xtensa PIE; expose PIE through linked C/assembly intrinsics (`esp-dsp`/`esp-nn`) or an explicit OxiIR-to-PIE-C pattern matcher for `i8`/`i16` dot/MAC kernels later. | Upstream GCC/LLVM do not auto-vectorise into ESP32-S3's proprietary `ee.*` PIE instructions **[S: community evidence only]**. MCU profiles default to scalar Wasm **[O]**. **[Corrected 2026-11-26: treVM does *not* show that SIMD code regresses on MCUs. It measured scalar CoreMark on Wasmtime/Pulley built with vs without SIMD support (the cost of enabling the feature), on boards that exclude the ESP32-S3. wasmi 2.0 no longer has that cost. The scalar default rests on flash and handler-size arguments and must be benchmarked (ledger A5).]** |
| **3. Explicit SIMD syntax** | Built-in generic value type **`Simd<T, N>`** (fixed 128-bit lanes first: `f32x4`, `f64x2`, `i32x4`, `u32x4`, `i16x8`, `u16x8`, `i8x16`, `u8x16`, plus `SimdMask<T, N>`). Ordinary operators (`+`, `-`, `*`, `+&`/`+%`/`+|`), static constructors (`Simd.splat`, `Simd.load`), no untyped `v128` bitbag. | Readable as typed generics by any TS developer; aligns with `contracts.md` §3 (`Simd<f32, 4>`) and Rust `std::simd`; requires zero new grammar; lowers 1:1 to OxiIR `Ty::Vec` and either Wasm `v128` or identical-semantics scalar loops. |
| **4. Auto-vectoriser v0** | Narrow, honest, predictable loop vectoriser on OxiIR SSA for **countable unit-stride loops** (`parallel for`, `for (const i of range(0, n))`) over contiguous `[T]`/inline-struct arrays, plus explicit reductions (`reduce`) and branchless `select`. Emit structured `OXS-P01xx` remarks for every declined loop. | Compilers vectorise 45–71% of synthetic loops but "only a few loops from the real applications" (Maleki et al. 2011 abstract **[V]**). **[Corrected 2026-11-26: the earlier "fails on 70–82%" had no source.]** Making `@expect(simd)` assert only language-defined vectorisability facts (`assertable yes`) and reporting heuristics (`assertable no`) avoids brittle builds. |
| **5. GPU kernel lowering** | Lower `#kernel` OxiIR functions directly to **WGSL text** (validated offline in dev/CI via `naga` `wgsl-in`). Do **not** emit SPIR-V directly or pass `ShaderSource::Naga` in browsers. | Verified in `wgpu-30.0.2/src/backend/webgpu.rs` **[V]**: `ShaderSource::Wgsl` passes straight to WebGPU browsers with zero Naga bundle cost, while native `wgpu` compiles WGSL via its bundled Naga to Vulkan SPIR-V, Metal MSL, and DX12 HLSL automatically. **[Corrected 2026-11-26: "zero Naga" holds only for a WebGPU-only wasm32 build. A browser build that keeps the WebGL2 fallback needs the `webgl` feature, which pulls in `wgpu-core` + `wgpu-hal` + naga (ledger A9).]** [Revised 2026-11-26 for decision 4 transparency] WGSL is emitted for layer-0 combinators and `kernel` functions alike, always next to a CPU version of the same code (section 4.6). |
| **6. Shared `GpuContext`** | Upgrade `oxiterm-gpu`'s `GpuContext` with **tiered capability negotiation**. When `DownlevelFlags::COMPUTE_SHADERS` is set, request compute limits starting from `Limits::downlevel_defaults()` clamped with `.or_worse_values_from(&adapter.limits())`; otherwise fall back to `downlevel_webgl2_defaults()` (UI only; kernels run on CPU SIMD/scalar). **[Corrected 2026-11-26: the draft's `Limits::defaults().using_resolution(adapter)` does not clamp. `using_resolution` copies only the texture dimensions, so device creation fails on adapters below the WebGPU defaults (ledger A1).]** | Preserves OxiUI's GLES 3.0 / WebGL2 compatibility floor while unlocking WebGPU/Vulkan/Metal/DX12 compute on one shared `wgpu::Device` & `wgpu::Queue`, enabling zero-copy kernel-to-UI texture handoff (`Rgba8Unorm`). |
| **7. Safety & Error Isolation** | Wrap every script GPU operation in `GpuContext::with_error_scopes`, forbid script-registered uncaptured handlers, enforce per-instance VRAM/buffer/dispatch quotas, inject **static loop trip-count guards** in emitted WGSL, and chunk large dispatches to stay well below the 2 s OS TDR watchdog. | Directly addresses Stage 5.2 BAD ACTOR B-01/B-03 findings (`wgpu` error scopes are thread-local; uncaptured handler is device-wide **[V]**) and prevents runaway user kernels from freezing the UI or triggering OS GPU device resets. **[Corrected 2026-11-26: wgpu 30's `ErrorScopeGuard` already pops on `Drop`. The hole that remains is that `GpuContext::device()` is `pub`, so any Rust co-user can replace the device-wide uncaptured handler. Close it by API design (ledger A3).]** [Revised 2026-11-26 for decision 4 transparency] The same rules apply to the executor-chosen GPU path (section 4.6), not only to `host:gpu`. |
| **8. AI on top** | Keep AI out of the core language; ensure `kernel` + `host:gpu@1` support **tiled `f32` GEMM** (workgroup shared memory + barriers), **conv2d**, **reductions** (softmax/layernorm), **attention**, **backward passes** (for live MLP training), plus optional **`f16`** (`shader-f16`) and **`i8`/`u8` packed dequant** with `i32`/`f32` accumulation. Skip native `bf16` (not in WGSL **[S]**). **[Corrected 2026-11-26: subgroups are native-only in wgpu 30 and cannot be used through wgpu in the browser (ledger A2).]** | Satisfies the "Hero" course goal (perceptron → live-trained digit recogniser → tiny LM in browser) using the same general-purpose kernel stack. [Revised 2026-11-26 for decision 4 transparency] `std/tensor` is written once in layer-0/`kernel` code and runs on whichever executor fits; `host:gpu` is only the expert escape hatch. |

### Transparent data parallelism (decision 4) [Revised 2026-11-26 for decision 4 transparency]

| Topic | Recommendation |
|---|---|
| **One source, several executors** | Layer-0 code (`map`/`zip`/`reduce`/`parallel for` over arrays, images, grids) and `kernel` functions always compile to a CPU version (scalar + SIMD128 per target) and, when the code satisfies the `kernel` profile, also to WGSL. The runtime picks the executor. Full design: section 4.6. |
| **Residency is a type** | `Resident<T>` (proposal; runner-up `Gpu<T>`; `Device<T>` rejected because `--target device` already means ESP32-class devices). Transfers are visible calls (`upload`, `download`). The runtime never inserts a transfer. |
| **Executor policy** | A pure function of the static residency types of the inputs, the host compute tier and the profile. Host arrays run on CPU (SIMD or scalar); `Resident` inputs run on the GPU when the host has the Compute tier, otherwise on host memory with the same code; mixing is a compile error. Element-count thresholds only produce report hints. |
| **Same results** | Integer results identical on every executor (GPU-eligible code is trap-free by construction); floats within the section 4.5 contract; `safety` keeps float code on the CPU. |
| **Transparency window** | The placement report states, for every loop and kernel, where it runs or ran and why; `@expect(simd)` and the new `@expect(gpu)` assert only eligibility (`assertable yes`); actual placement is logged at run time in the same format. |
| **`host:gpu`** | Survives as the low-level capability for libraries and experts, with all corrected safety rules. Pure compute through the executor needs no `main` parameter. |

---

## 1. SIMD Across Execution Engines & Targets

OxiScript targets one portable bytecode format (Core WebAssembly) executed across very different engines: the embedded `wasmi` interpreter (local core, browser live mode, ESP32 dev mode), browser JIT engines (web release), Wasmtime/Cranelift (64-bit native release), and Wasm-to-C (`wasm2c`/`w2c2`) + native C compilers (32-bit SBCs, ESP32 release, safety-critical builds). Because SIMD capability and performance vary drastically across these engines, **SIMD lowering must be a per-target backend choice from one semantic OxiIR**, never a single mandatory `.wasm` binary.

### 1.1 `wasmi` (Exact Version in `Cargo.lock`: `2.0.3`)

Direct inspection of `Cargo.lock` and `~/.cargo/registry/src/*/wasmi-2.0.3` / `wasmi_core-2.0.3` establishes the exact capabilities of our live interpreter **[V]**:

1. **Proposal support & versions:** Upstream `wasmi` added fixed-width `simd` in **v0.43.0** and `relaxed-simd` in **v0.44.0** ([wasmi GitHub](https://github.com/wasmi-labs/wasmi), [docs.rs/wasmi](https://docs.rs/wasmi/)) **[S]**. Both proposals are fully present in our pinned **`wasmi 2.0.3`** **[V]**.
2. **Feature gating:** Both proposals sit behind a single opt-in Cargo feature `simd` (disabled in `wasmi`'s `default` features) **[V, `wasmi-2.0.3/Cargo.toml:83`; corrected line number]**:
   ```toml
   simd = ["wasmi_core/simd", "wasmi_ir/simd", "wasmparser/simd"]
   ```
   Currently `oxiscript-runtime/Cargo.toml:18` enables `features = ["validate", "deterministic", "stable", "auto-dispatch"]` (no `simd`). At runtime, `wasmi::Config` provides separate toggles `c.wasm_simd(bool)` and `c.wasm_relaxed_simd(bool)` (gated on `cfg(all(feature = "simd", feature = "validate"))`; both default to `true` when the feature is compiled in, so `oxiscript-runtime` **must explicitly set `c.wasm_relaxed_simd(false)`** when enabling `simd`) **[V, `wasmi-2.0.3/src/engine/config.rs:324-338`]**.
3. **Zero penalty on non-SIMD code in `wasmi 2.0`:** In wasmi 0.43/0.44, the CHANGELOG warned that "enabling the `simd` crate feature may regress Wasm execution and memory consumption" **[V]**. (The "~8% CoreMark" figure in the draft has no source: **[U]**.) In `wasmi 2.0.3`, `ValType::V128` occupies **2 cells** (`wasmi-2.0.3/src/engine/translator/utils.rs:12-18`) of fixed 64-bit cells **[V]**. The 2.0.0 CHANGELOG states: "Enabling the `simd` crate feature no longer affects memory consumption or execution performance of non-`simd` Wasm code" **[V: [CHANGELOG](https://github.com/wasmi-labs/wasmi/blob/main/CHANGELOG.md)]**.
4. **How `wasmi 2.0.3` executes SIMD (interpreted performance):** Inspection of `wasmi_core-2.0.3/src/simd.rs:252-259` confirms **there are no `core::arch` hardware SIMD intrinsics** inside `wasmi_core`. Instead, every lane operation (`lanewise_binary`, `lanewise_unary`, `lanewise_comparison`) executes a **plain Rust fixed-bound array loop** (`for i in 0..Self::LANES { lhs[i] = f(lhs[i], rhs[i]); }`) over `#[repr(transparent)] struct([T; N])` **[V]**.
   - On desktop x86_64/aarch64 in `--release`, `rustc`/LLVM can auto-vectorise simple integer lane loops inside `wasmi_core`'s instruction handlers, and—more importantly—**1 interpreter dispatch processes 4–16 lanes instead of 4–16 interpreter dispatches**. Because dispatch overhead dominates interpreted loops, Wasm SIMD128 in `wasmi` on desktop CPUs is expected to outperform interpreted scalar loops (~1.5×–3.5× depending on lane width and memory ratio **[P/U: unmeasured; benchmark in Stage 8.3a]**), though still much slower than JIT/AOT native SIMD (ratio **[U]**: [wasmi-benchmarks](https://github.com/wasmi-labs/wasmi-benchmarks) publishes no numbers and has no SIMD case; corrected 2026-11-26).
   - On 32-bit MCUs without 128-bit vector registers, wider `V128` traffic (2 cells + 16-byte copies) and larger handler code *may* hurt cache/RAM **[O, unmeasured]**. **[Corrected 2026-11-26]** The treVM MCU study ([arXiv:2604.27570](https://arxiv.org/html/2604.27570v1), Table I) measured *scalar* CoreMark on Wasmtime (Pulley) built with and without SIMD support. The no-SIMD build scored higher on all 4 boards: RP2350 18.6 vs 13.2, nRF52840 5.1 vs 3.5, ESP32-C6 12.3 vs 9.9, and ESP-WROOM-32 11.7 vs 11.6 (marginal) **[V]**. That is the cost of enabling the feature in Pulley, not evidence that SIMD *code* is slower. The ESP32-S3 was not tested, and wasmi's SIMD setting is not stated. wasmi 2.0 removed the equivalent cost (item 3).
5. **Determinism interaction in `wasmi 2.0.3`:** Verified in `wasmi_core-2.0.3/src/simd.rs:938-980`, `wasm.rs:205-221`, and `value.rs:406-418,605-655` **[V]**: SIMD float operations (`f32x4_add/sub/mul/div/sqrt/min/max`, `f64x2_*`) delegate directly to `wasm::f32_*` → `Float::*`, which calls `.canonicalize_nan_if_enabled()` on every lane when `features = ["deterministic"]` is enabled. Thus `wasmi 2.0.3` already provides bit-deterministic canonical NaNs (`0x7FC0_0000` / `0x7FF8_0000_0000_0000`) for fixed-width SIMD128 float ops! Note also that `wasmi` lowers relaxed-SIMD ops in software with a fixed choice (e.g. `f32x4_relaxed_min` forwards to `f32x4_min`), but JIT/AOT hosts do **not** guarantee the same choice, so relaxed SIMD must remain disabled in the default/deterministic ABI. **[Corrected 2026-11-26]** wasmi's fixed choice is **not** the Wasm 3.0 deterministic profile. `f32x4_relaxed_madd`/`f64x2_relaxed_madd` call `Float::mul_add` (std `mul_add` / `libm::fma`, i.e. **fused**) even with `deterministic` (`wasmi_core-2.0.3/src/simd.rs:1399-1402`, `value.rs:472-475, 550-553, 710-713`). The spec says: "In the deterministic profile, the unfused behaviour is used" ([numerics, relaxed operations](https://webassembly.github.io/spec/core/exec/numerics.html)). min/max/swizzle/laneselect/trunc do match the profile. So `wasm_relaxed_simd(false)` is **mandatory** in `oxiscript-runtime`, and the madd divergence is worth reporting upstream. Also note the `neg`/`abs` lane ops are sign-bit operations and are not canonicalised (allowed by the spec; `simd.rs:974-977`).

### 1.2 AOT, Wasm-to-C, and Browser Engines

* **Wasmtime / Cranelift (64-bit native AOT/JIT):** Tier 1 production support for both fixed-width **SIMD128** and **Relaxed SIMD** on `x86_64`, `aarch64`, `s390x`, and `riscv64` ([Wasmtime Proposals](https://docs.wasmtime.dev/stability-wasm-proposals.html), [Platform Support](https://docs.wasmtime.dev/stability-platform-support.html)) **[V]**. Lowers Wasm `v128` directly to SSE/AVX2/AVX-512, NEON, or RVV. **No 32-bit backend** (32-bit falls back to the Pulley interpreter) **[V]**. Cranelift is not known to auto-vectorise loops. **[Corrected 2026-11-26: the cited page does not say so. It says only that Cranelift lacks "the aggressive optimization stance … of LLVM or gcc" and avoids "advanced alias analysis", so this is [U].]** Either way, OxiScript must emit Wasm `v128` ops itself. **Added:** Wasmtime offers `Config::relaxed_simd_deterministic` and `Config::cranelift_nan_canonicalization` ([docs.rs](https://docs.rs/wasmtime/latest/wasmtime/struct.Config.html)) **[V]**. The native release path should set both. This removes the `bitselect` canonicalisation only for an artifact built for Wasmtime alone; a `.wasm` that also runs in browsers still needs it. Whether Wasmtime's canonical NaN is the positive `0x7fc00000` that `ir.md` §6 requires is **[U]**.
* **Browsers (V8 / SpiderMonkey / JavaScriptCore):** Fixed-width **SIMD128** is baseline everywhere (Chrome ≥ 91, Firefox ≥ 89, Safari ≥ 16.4; [WebAssembly Roadmap](https://webassembly.org/features/), [Emscripten SIMD docs](https://emscripten.org/docs/porting/simd.html)) **[S]**. **Relaxed SIMD** is supported in Chrome ≥ 114 and Firefox ≥ 145. **Safari requires the JavaScriptCore flag `useWebAssemblyRelaxedSIMD`** ([features.json](https://github.com/WebAssembly/website/blob/main/features.json)) **[V]**. **[Corrected 2026-11-26: the draft said "all three engines"; `docs/research/oxiscript-language.md` carries the same error.]** Browser optimising compilers (V8 TurboFan, SpiderMonkey Ion, JSC OMG) lower `v128` directly to SSE/AVX or NEON; like Cranelift, they do **not** reliably auto-vectorise scalar Wasm loops.
* **`wasm2c` (WABT) and `w2c2` (AOT via C compiler):** Both support WebAssembly fixed-width **SIMD128** by emitting calls to **[SIMDe](https://github.com/simd-everywhere/simde)** (MIT-licensed header-only C library; `<simde/wasm/simd128.h>`) ([WABT `wasm2c` README](https://github.com/WebAssembly/wabt/tree/main/wasm2c), [`w2c2` README](https://github.com/turbolent/w2c2)) **[V/S]**. On x86, ARM NEON, AltiVec, and RISC-V RVV, SIMDe maps those intrinsics to native hardware SIMD headers; on targets without a SIMDe backend, SIMDe falls back to portable C99 scalar loops. Relaxed SIMD status in `wasm2c`/`w2c2` is partial/unverified **[U]**—another reason not to require relaxed SIMD.
* **WAMR (optional MCU runtime):** Lists fixed-width SIMD as a post-MVP feature on x86-64/AArch64 AOT/JIT ([WAMR README](https://github.com/bytecodealliance/wasm-micro-runtime)) **[V]**; not available as hardware SIMD on Xtensa.

### 1.3 ESP32-S3 (Xtensa LX7) & PIE SIMD: Can an AOT Path Use It?

The ESP32-S3 uses a dual-core **Xtensa LX7** with Espressif's proprietary **PIE (Processor Instruction Extensions)**—128-bit vector registers (`q0`–`q7`) with packed `i8`/`i16`/`i32` multiply-accumulate, saturated arithmetic, and dot-product instructions prefixed `ee.*` ([ESP32-S3 Technical Reference / PIE docs](https://docs.espressif.com/projects/esp-dsp/en/latest/esp32s3/index.html)) **[S]**. Hand-coded PIE assembly in `esp-nn` yields **3.5×–14.2×** per kernel over ANSI C on ESP32-S3: conv 1×1 14.24×, fully connected 5.32×, elementwise mul 3.51× ([esp-nn README](https://github.com/espressif/esp-nn)) **[V]**. Model-level `invoke()` gains are 42–104×, but those include more than SIMD. **[Corrected 2026-11-26: the draft's "5×–44×" cited esp-dsp, whose README gives no numbers.]**

However, **neither upstream GCC (`xtensa-esp32s3-elf-gcc`) nor LLVM auto-vectorises standard C loops or SIMDe fallback code into Xtensa PIE instructions**, and GCC exposes no PIE intrinsics **[S: community statement only, [epdiy #289](https://github.com/vroland/epdiy/discussions/289); no official Espressif toolchain doc found, so re-check before relying on it]**. Why:
1. PIE uses vendor-specific Tensilica TIE encodings, strict 128-bit alignment rules, and custom accumulator registers (`accx`) not modeled by GCC's tree vectoriser or LLVM's LV.
2. SIMDe has **no Xtensa PIE backend** **[U: verified absent in standard SIMDe target list; re-check before relying]**, so `wasm2c`/`w2c2` + SIMDe compiles Wasm `v128` on ESP32-S3 down to **scalar C loops** (worse than plain scalar Wasm due to struct packing/unpacking overhead).

**Design consequence for ESP32-S3 AOT:**
1. **Default ESP32-S3 target profile:** Compile OxiScript to **scalar Wasm** (disabling Wasm SIMD128 emission) for both `wasmi` live mode and `w2c2` release AOT, avoiding `v128` unpacking overhead.
2. **Using PIE on ESP32-S3 (two clean paths):**
   - **Short-term (Stage 8.N / `host:ml` / `host:dsp`):** Expose Espressif's permissive Apache-2.0 **[V: both READMEs]** **`esp-dsp`** and **`esp-nn`** kernels as host capabilities or linked native shims on ESP32-S3. OxiScript's standard tensor/DSP library dispatches `dot`, `matmul_i8`, `fir`, `conv2d` to the host capability when present and falls back to OxiScript SIMD/scalar otherwise, with the placement report recording `placed: host:dsp (esp32-s3 PIE)`.
   - **Long-term (direct OxiIR → C backend):** Because OxiIR preserves `Ty::Vec { elem, lanes }` and `kernel` semantics before Wasm erasure, a future OxiIR → C emitter can emit `ee.*` inline assembly / Espressif intrinsics directly for `Simd<i8, 16>` / `Simd<i16, 8>` MAC patterns without going through Wasm `v128` at all.

### 1.4 Target × SIMD Capability Matrix

| Target / Runtime | Wasm SIMD128 | Wasm Relaxed SIMD | Hardware Vector ISA | How OxiScript Uses SIMD | Expected Speedup vs Scalar | Notes / Caveats |
|---|---|---|---|---|---|---|
| **Local dev / OxiUI live (`wasmi 2.0.3`, desktop x86_64/arm64)** | Yes (`simd` feature + `wasm_simd(true)`) **[V]** | Yes (must disable: `wasm_relaxed_simd(false)`) **[V]** | Emulated via Rust `[T; N]` loops in `wasmi_core` (LLVM may auto-vec handlers) **[V]** | Emit Wasm `v128` when target profile = `simd128`; scalar otherwise | **~1.5×–3.5×** vs interpreted scalar (fewer dispatches) **[P/U]** | Zero overhead on non-SIMD code in `wasmi 2.0` (`V128` = 2 cells) **[V]**; `deterministic` canonicalises lane NaNs **[V]**; relaxed `madd` is fused even then, so relaxed must be off **[V]** |
| **Browser release (Chrome / Firefox / Safari Wasm engine)** | Yes (universal baseline) **[V]** | Chrome 114+, Firefox 145+; Safari behind a flag **[V, corrected]** | x86 SSE/AVX, ARM NEON via browser JIT | Emit Wasm `v128`; scalar fallback build optional | **2×–4×** (`f32`/`i32`), **4×–8×** (`u8`/`i16`) **[P]**; measured references: TF.js 1.7–4.5×, MediaPipe 14–15 → 38–40 FPS **[V]** | Relaxed SIMD kept off in deterministic mode |
| **Browser live mode (`wasmi` compiled to wasm32)** | Yes (wasm-in-wasm) **[V]** | Disabled | Emulated inside wasm32 `wasmi` | Scalar Wasm preferred for fast compile/small size; `v128` optional | Modest dispatch savings **[P/U]** | Heavy compute uses browser Wasm release or WebGPU |
| **Native 64-bit release (Wasmtime / Cranelift)** | Yes (Tier 1) **[V]** | Yes (Tier 1) **[V]** | x86_64 SSE4.2/AVX2/AVX-512, aarch64 NEON, riscv64 RVV | Emit Wasm `v128` (Cranelift does **not** auto-vectorise scalar Wasm **[S]**) | **2.5×–4×** (`f32`/`i32`), up to **8×+** (`i8`/`i16`) **[P/S]** | 64-bit hosts only; no 32-bit Cranelift **[V]** |
| **32-bit Linux SBC (RPi 4/5 ARMv7/ARMv8, `w2c2`/`wasm2c` + GCC/Clang)** | Yes (via SIMDe, MIT) **[V/S]** | Unverified **[U]** | ARM NEON (via SIMDe → `<arm_neon.h>`) | Emit Wasm `v128` → `w2c2` + SIMDe → GCC/Clang `-O3 -mfpu=neon` | **2×–4×** (`f32`/`i32`) **[P]** | Must verify C compiler flags; `w2c2` requires C99 for SIMD **[V]** |
| **ESP32-S3 live (`wasmi` `no_std`)** | Feature exists, **turn OFF** **[O]** | Off | None used by `wasmi` | Emit **scalar Wasm** | Unknown: treVM measured Wasmtime/Pulley, not wasmi or the S3 **[U, corrected]** | Saves flash and handler code size **[O]**; benchmark on the board before fixing the default |
| **ESP32-S3 AOT (`w2c2`/`wasm2c` + `xtensa-esp32s3-elf-gcc`)** | Scalar C via SIMDe (no PIE) **[S/U]** | No | **Xtensa PIE** (`ee.*`, 128-bit `i8`/`i16`/`i32`) **[S]** | Emit **scalar Wasm**; dispatch DSP/NN ops to `esp-dsp`/`esp-nn` capability, or future direct OxiIR→C PIE intrinsics | **3.5×–14×** per kernel via `esp-nn` PIE **[V]** | GCC/LLVM do not auto-vectorise PIE **[S, community]** |
| **Safety-critical target (qualified C compiler via `w2c2`/OxiIR→C)** | Optional (scalar fallback guaranteed identical) | **Forbidden** | Target-dependent | Scalar fallback or strictly verified deterministic `v128` (no FMA/relaxed ops) | Deterministic parity > peak speed | NaN canonicalised per `ir.md` §6 |


**[Revised 2026-11-26 for decision 4 transparency] Target x executor availability (derived from the matrix above, section 5.7 and the profile/target rules of `oxi-ui.md` 8.2; [O]).** This is what the runtime's executor policy (section 4.6) chooses from; the language semantics are identical in every row.

| `--target` / host | Scalar | SIMD128 | WGSL variant runs | Threads (8.4) |
|---|---|---|---|---|
| `device` (ESP32-class, wasmi) | yes (default) | off by default, provisional (Q9) | no (no GPU; the Resident type is host memory) | no |
| `web` (browser local mode) | yes | yes | only with WebGPU and `ComputeTier::Compute`; WebGL2-only browsers get host memory | later (isolates / Wasm threads) |
| `server` (native, Wasmtime/AOT) | yes | yes | with a native compute-capable adapter (Vulkan/Metal/DX12); lavapipe in CI | later |

---

## 2. Language Design for Explicit SIMD

OxiScript's design rule (`grammar.md`) is strict: **a TypeScript developer must read the code without learning a foreign dialect**, while semantics remain sound, statically typed, and deterministic (`ir.md` §6). Below we survey 11 existing SIMD designs and synthesize the cleanest fit for OxiScript.

### 2.1 Comparison of Existing SIMD Language Designs

| Language / System | Vector Type Surface | Width Model | Lane Access & Shuffles | Masks & Predication | Portable Fallback | Lessons for OxiScript |
|---|---|---|---|---|---|---|
| **Rust `std::simd`** (`portable_simd`, nightly) ([tracking](https://github.com/rust-lang/portable-simd)) **[S]** | `Simd<T, const N: usize>` (e.g. `Simd<f32, 4>`) | Fixed compile-time lane count `N`, 1 ≤ N ≤ 64, no power-of-two rule [V, corrected] | `v[i]` (dynamic or const), `simd_swizzle!(v, [3,2,1,0])` (const indices) | Dedicated `Mask<M, N>` (`m.select(a, b)`, `all()`, `any()`) | Yes (LLVM lowers arbitrary `N` to chunks/scalars) | **Closest model**: clean generic type, distinct typed mask prevents bitmask/int confusion, const-only swizzles map to hardware. |
| **C# `Vector128<T>` / `Vector<T>`** ([.NET Intrinsics](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.intrinsics.vector128-1)) **[S]** | `Vector128<float>` (fixed bit-width) & `Vector<float>` (hardware-width) | Both: fixed bit-width (`64/128/256/512`) and opaque `Vector<T>.Count` | `v.GetElement(i)`, `Vector128.Shuffle(v, idx)` | Same type (`Vector128<T>` all-1s/0s) + `ConditionalSelect` | Yes (JIT software emulation) | Fixed width (`Vector128`) won over variable `Vector<T>` for predictable algorithms & serialization; untyped all-1s float masks are error-prone. |
| **Java Vector API** (JEP 448/469/508) ([OpenJDK JEP 489](https://openjdk.org/jeps/489)) **[S]** | `FloatVector` + `VectorSpecies<Float>` | Runtime species (`SPECIES_PREFERRED` or `SPECIES_128`) | `v.lane(i)`, `v.rearrange(VectorShuffle)` | Dedicated `VectorMask<Float>` (`a.blend(b, mask)`) | Yes (pure Java fallback) | Explicit `Species` is verbose and awkward in static AOT; confirms dedicated `Mask` type and `select`/`blend` ergonomics. |
| **Zig `@Vector`** ([Zig docs](https://ziglang.org/documentation/master/#Vectors)) **[S]** | `@Vector(4, f32)` | Arbitrary compile-time length `N` | `v[i]`, `@shuffle(f32, a, b, mask)`, `@splat(x)`, `@reduce(.Add, v)` | `@Vector(N, bool)` + `@select(T, mask, a, b)` | Yes (LLVM lowers) | First-class operators (`+`, `*`, `+%`, `+|`) make vector math read like scalar math! Builtin `@reduce` is super clean. |
| **Mojo `SIMD`** ([Modular docs](https://docs.modular.com/mojo/stdlib/builtin/simd/SIMD/)) **[S]** | `SIMD[DType.float32, 4]` (scalars = `SIMD[T, 1]`) | Power-of-2 compile-time `size` | `v[i]`, `v.shuffle[...]()`, `v.reduce_add()` | `SIMD[DType.bool, N]` + `mask.select(a, b)` | Yes (MLIR lowers) | Unifying scalar and vector syntax is slick for AI, but `DType` metaprogramming is alien to TS developers. |
| **ISPC** ([Pharr 2018](https://pharr.org/matt/blog/2018/04/29/ispc-retrospective)) **[V]** | Implicit `varying float` across gang (`uniform` vs `varying`) | Target-selected gang width (`4/8/16`) | Implicit per-instance; short vectors (`float<4>`) bolted on | Implicit execution mask (`if` predication) | Yes (compiles to target width) | SPMD handles control flow well; Pharr's retrospective lists explicit vectors mapped to SIMD lanes as a wish ("It would be nice to be have support for explicit vectors") **[V; corrected from "regrets"]**. |
| **Halide** ([PLDI 2013](https://people.csail.mit.edu/jrk/halide-pldi13.pdf)) **[S]** | Scalar algorithm `Func`; `.vectorize(x, 4)` in schedule | Schedule parameter | N/A (stencil indexing) | Predicated tail loops | Yes (CPU/GPU) | Decoupling algorithm from vector width is ideal for Layer 2 schedules, not for manual Layer 1 SIMD. |
| **Futhark** ([PLDI 2017](https://www.futhark-lang.org/publications/pldi17.pdf)) **[S]** | No explicit SIMD types; pure data-parallel arrays (`map`, `reduce`, `scan`) | Compiler-chosen | Array indexing | Pure functional `if` | Yes (sequential/multicore/GPU) | Uniqueness types enable race-free in-place updates for Layer 0 combinators; lacks Layer 1 manual SIMD escape hatch. |
| **WGSL** ([W3C WGSL](https://www.w3.org/TR/WGSL/)) **[V/S]** | `vec2<f32>`, `vec3<f32>`, `vec4<f32>` | Fixed small geometric widths (2, 3, 4) | `.x/.y/.z/.w`, `.xyzw` swizzles, `v[i]` | `vecN<bool>` + `select(f, t, cond)` (note arg order!) | N/A (GPU native) | Component swizzles (`v.xy`) are great for graphics/kernels; `vec3` padding pitfalls (16-byte alignment in storage buffers) must be handled carefully. |
| **JS `SIMD.js` (abandoned)** ([V8 features/simd](https://v8.dev/features/simd), [TC39 archive](https://github.com/tc39/ecmascript_simd)) **[V]** | `SIMD.Float32x4` value objects, `SIMD.Float32x4.add(a, b)` | Fixed 128-bit | `extractLane`, `replaceLane`, `swizzle`, `shuffle` | `SIMD.Bool32x4` + `SIMD.Float32x4.select` | JS polyfill (very slow) | **Why it died:** the TC39 README says it was removed from Stage 3 because WebAssembly is "a better vehicle", and implementing it reliably outside asm.js "creates a lot of complexity" **[V]**. **[Corrected 2026-11-26: the "~10% V8 bloat" figure has no source (the cited v8.dev/blog/simd is a 404): [U].]** Also: static method syntax (`Float32x4.add(a, b)`) is unreadable compared to operators. |
| **AssemblyScript `v128`** ([AS SIMD docs](https://www.assemblyscript.org/stdlib/builtins.html#simd)) **[S]** | Single untyped `v128` + intrinsics `f32x4.add(a, b)` | Fixed 128-bit (Wasm only) | `f32x4.extract_lane(v, 0)` | Untyped `v128` bitmask + `v128.bitselect` | **None** (requires Wasm SIMD128) | Zero type safety between `f32x4` and `i32x4` (both are `v128`!), verbose intrinsic calls, cannot compile to scalar Wasm for MCUs. |

### 2.2 Recommended OxiScript SIMD Surface, OxiIR Lowering & Determinism

#### Language Surface (Readable by TypeScript Developers)

Following `contracts.md` §3 and `grammar.md` §5, **`Simd<T, N>` requires zero new syntax**—it is a built-in generic **value type** (like a non-heap `struct` with copy semantics, always copied by value, never RC-allocated) plus a matching typed mask **`SimdMask<T, N>`** (or `Mask<N>` where lane bit width is inferred from element width in IR).

1. **Supported types (v1 — exact 128-bit Wasm SIMD128 shapes + portable aliases):**
   - `Simd<f32, 4>` (alias `f32x4`), `Simd<f64, 2>` (`f64x2`)
   - `Simd<i32, 4>` (`i32x4`), `Simd<u32, 4>` (`u32x4`), `Simd<i64, 2>` (`i64x2`), `Simd<u64, 2>` (`u64x2`)
   - `Simd<i16, 8>` (`i16x8`), `Simd<u16, 8>` (`u16x8`), `Simd<i8, 16>` (`i8x16`), `Simd<u8, 16>` (`u8x16`)
   - Restricting `N` in v1 to exact 128-bit combinations (`sizeof(T) * N === 16`) guarantees a 1:1 mapping to Wasm `v128` registers without hidden multi-register splitting. Non-128-bit widths (e.g., `Simd<f32, 8>` for future AVX2/GPU) can be accepted later by splitting in lowering without breaking source compatibility.
   - `N` is a compile-time integer literal type argument (type-checker extends `TypeExpr` to accept `INT` literals in builtin `Simd<T, N>` positions—already parsed by `LiteralType = STRING | INT | ...` in `grammar.md` §3.1 **[V]**). **[Corrected 2026-11-26: the grammar annotates `LiteralType` as "discriminants only" (`grammar.md:247`). Parsing needs no change, but the checker must newly accept an INT literal as a type argument of the builtin `Simd`.]**

2. **Operators (same overflow/determinism rules as OxiScript scalars!):**
   - Arithmetic: `+`, `-`, `*`, `/` (floats only for `/`, since Wasm SIMD128 has no integer vector division; integer `/` on `Simd` is a clear `OXS-E0110` diagnostic suggesting scalar loop or multiply-shift).
   - **Critical overflow rule (`grammar.md` §1.3 & `ir.md` §3):**
     - On `Simd<f32, 4>` / `Simd<f64, 2>`: `+`, `-`, `*`, `/` are IEEE-754 lane-wise (no FMA contraction!).
     - On integer `Simd`: Wasm SIMD128 instructions `i32x4.add`, `i16x8.add`, `i8x16.add`, `i32x4.mul`, etc. are **wrapping** (`+%`, `-%`, `*%`), and `i8x16.add_sat_s/u`, `i16x8.add_sat_s/u` are **saturating** (`+|`, `-|`). Emitting a per-lane overflow trap check on plain `+` would destroy SIMD throughput (3–5 extra vector instructions + `any_true` branch per op). **Design recommendation [O] (corrected 2026-11-26 to remove a contradiction with §9 Q1):** **forbid** plain `+`/`-`/`*` on integer `Simd` (`OXS-E0110` with a machine-applicable quick-fix to `+%` or `+|`). Every integer vector operator then maps to one Wasm SIMD128 instruction, with no hidden per-lane trap sequence, and the scalar fallback uses the same wrapping/saturating ops, so semantics stay identical. Precedent: Rust `std::simd` integer arithmetic wraps rather than panicking. The draft first recommended keeping trapping `+` and listed forbidding it as an alternative; §9 Q1 recommended the opposite. One answer is given now, and the owner decides (Q1). For *auto-vectorised scalar loops*, trapping `+` stays trapping and blocks vectorisation (`OXS-P0108`).
   - Bitwise (integer `Simd` and `SimdMask`): `&`, `|`, `^`, `~`, `<<`, `>>` (uniform scalar shift amount, matching Wasm `i32x4.shl(v, i32)`).
   - Comparisons: **Do NOT overload `===`/`<` to return a `SimdMask`** (it violates TS intuition, where `a < b` is `bool`; Rust `std::simd` likewise uses methods such as `simd_lt` returning `Mask` [V], while Zig returns a bool vector from `<` [V], so this is a taste call [O]). Instead use explicit lane comparison methods: `a.eq(b)`, `a.ne(b)`, `a.lt(b)`, `a.le(b)`, `a.gt(b)`, `a.ge(b)` returning `SimdMask<T, N>`, plus `a.allEq(b): bool` / `mask.all(): bool` / `mask.any(): bool` / `mask.bitmask(): u32`.

3. **Construction, memory load/store, lane access, shuffles, reductions & conversions:**
   *Illustrative proposed syntax (not current OxiScript): static calls on the type name such as `Simd.splat<...>` are a proposed builtin surface; today's stdlib has no such namespace.*
   ```ts
   import { Simd } from "std/simd"; // or prelude builtin

   export function saxpy(a: f32, xs: f32[], ys: f32[], out: f32[]): void {
     const va = Simd.splat<f32, 4>(a);
     let i: i32 = 0;
     const n = xs.length;
     while (i + 4 <= n) {
       const vx = Simd.load<f32, 4>(xs, i);   // single bounds check: 0 <= i && i + 4 <= xs.length
       const vy = Simd.load<f32, 4>(ys, i);
       const vr = va * vx + vy;               // IEEE-754 f32x4.mul + f32x4.add (no FMA)
       Simd.store<f32, 4>(out, i, vr);
       i += 4;
     }
     while (i < n) {
       out[i] = a * xs[i] + ys[i];
       i += 1;
     }
   }
   ```
   - **Constructors:** `Simd.splat<T, N>(val)`, `Simd.from<f32, 4>(a, b, c, d)`, `Simd.zero<T, N>()`.
   - **Bounds-checked memory ops:** `Simd.load<T, N>(arr: T[], idx: i32): Simd<T, N>` and `Simd.store<T, N>(arr: T[], idx: i32, v: Simd<T, N>): void`. Checks `idx >= 0 && idx <= arr.length - N` once (traps `bounds` otherwise), avoiding 4 separate bounds checks.
   - **Lane access & shuffles:** `v.lane(0)` / `v.withLane(0, val)` (constant index `0..N-1` checked at compile time → Wasm `extract_lane` / `replace_lane`; dynamic index falls back cleanly or errors with `OXS-E0117` in v1), `v.swizzle(3, 2, 1, 0)` (compile-time lane indices → `i8x16.shuffle`), `Simd.shuffle(a, b, ...)` (compile-time `0..2N-1`).
   - **Mask & selection:** `mask.select(ifTrue, ifFalse)` (maps to Wasm `v128.bitselect` because mask lanes are guaranteed all-0s or all-1s by construction), `mask.all()`, `mask.any()`, `mask.none()`, `mask.bitmask()`.
   - **Math & reductions:** `v.abs()`, `v.sqrt()`, `v.min(w)`, `v.max(w)` (IEEE NaN-propagating matching `ir.md` §3 `min`/`max` → Wasm `f32x4.min`/`max`), `v.floor()`, `v.ceil()`, `v.trunc()`, `v.nearest()`, `v.sum()` (defined left-to-right tree order `((v0 + v1) + (v2 + v3))` so scalar fallback and Wasm SIMD produce **bit-identical** floating-point sums!), `v.dot(w)` (for `i16x8` → `i32x4` widening dot product matching `i32x4.dot_i16x8_s`).
   - **Conversions:** `v.toF32()` (`i32x4` ↔ `f32x4`), `v.saturating<i32>()` (Wasm `i32x4.trunc_sat_f32x4_s`; ABI 1.0 bans scalar `trunc_sat` (`abi.md:5`). Allowing the vector form in ABI 1.1 is an **owner decision** (see §9 Q7): it is deterministic, but it widens the feature set that every engine and `wasm2c` path must accept. The alternative is an explicit clamp + `trunc` sequence.), `v.widenLow<i32>()` / `v.widenHigh<i32>()` (`extend_low`/`extend_high`), `v.narrowSat<i16>(hi)` (`narrow_i32x4_s/u`), `v.bitcast<u32>()` (canonicalises float NaNs before bitcasting to int lanes, per `ir.md` §6!).

#### OxiIR Representation & ABI 1.1

Per `contracts.md` §3 and `ir.md` §7, `Ty` and `Op` are already `#[non_exhaustive]`:
- **New IR types (`oxiscript-ir/src/types.rs`):**
  - `Ty::Vec { elem: VecElem, lanes: u8 }` (where `VecElem` is `I8 | U8 | I16 | U16 | I32 | U32 | I64 | U64 | F32 | F64`, with `elem.bytes() * lanes == 16` enforced by `verify`).
  - `Ty::VecMask { elem_bits: u8, lanes: u8 }` (distinct from integer vectors in IR so `verify` guarantees canonical all-0/all-1 lane masks before `vec.select`).
  - Both are **unmanaged value types** (`Ty::is_managed() == false`, no RC header, no retain/release).
- **New IR ops:**
  - `Op::VecSplat { val }`, `Op::VecConst([u8; 16])`, `Op::VecExtract { vec, lane: u8 }`, `Op::VecReplace { vec, lane: u8, val }`, `Op::VecShuffle { a, b, lanes: [u8; 16] }`.
  - Reuse `Op::Bin`, `Op::Un`, `Op::Cmp` (when operands are `Ty::Vec`, `Cmp` yields `Ty::VecMask`), `Op::VecSelect { mask, a, b }`, `Op::VecBitmask { mask }`, `Op::VecAllTrue { mask }`, `Op::VecAnyTrue { mask }`.
  - `Op::VecLoad { arr, idx }` and `Op::VecStore { arr, idx, val }` (typed array ops—never raw pointers—so bounds checking and safety remain verified in OxiIR!).
  - `Op::VecConv { ovf, a }`, `Op::VecBitcast { a }`, `Op::VecDot { a, b }` (widening integer dot), `Op::VecWiden { a, high: bool }`, `Op::VecNarrow { lo, hi, ovf }`.
- **ABI 1.1 (minor bump, backwards-compatible):**
  - `ABI_MINOR = 1`; `WasmTy::V128` added to `oxiscript-abi` (`WasmTy` is already `#[non_exhaustive]`).
  - `oxi.manifest` gains an optional `features simd128` line so an engine without SIMD128 rejects cleanly with `OXS-E0403` before instantiation.
  - Inline struct fields of type `Simd<T, N>` use `FieldKind::Inline { size: 16, align: 16, ptr_slots: &[] }`—already supported by `layout::try_layout_fields` (`oxiscript-abi/src/layout.rs:231`) **[V]**. Note one subtle alignment detail: `OBJECT_ALIGN` is 8 (`layout.rs:23`), so heap objects containing a 16-byte aligned field either bump object alignment to 16 (via a descriptor flag `TYPE_FLAG_ALIGN16` and `oxi_alloc` honoring it) or emit unaligned `v128.load align=2` (Wasm `v128.load` supports any power-of-2 alignment hint `0..=4` and executes cleanly on x86/ARM/wasmi even at 4- or 8-byte alignment, with only a tiny penalty on unaligned cache-line splits). **Recommendation [O]:** Emit `v128.load` with `align=2` (4-byte) for `[f32]`/`[i32]` array data (since `arr.data` is 8-aligned and `idx` may be odd) and `align=3` (8-byte) for struct fields—avoiding any change to `OBJECT_ALIGN = 8` or `oxi_alloc` in ABI 1.1!

#### Scalar Fallback & Determinism Guarantees (`ir.md` §6)

1. **Identical semantics across SIMD128 and scalar fallback:** A pre-codegen IR lowering pass `oxiscript_ir::lower_simd_to_scalar(&mut Module)` rewrites every `Ty::Vec` into `N` scalar SSA values (and `VecLoad`/`VecStore` into one hoisted bounds check + `N` scalar `array.get`/`array.set` ops) when targeting scalar Wasm (e.g., ESP32-S3 or non-SIMD hosts). Because every OxiIR vector op has exact lane-wise semantics and reductions have a fixed binary-tree reduction order, **differential testing** (`simd128` in `wasmi`/Wasmtime vs `scalar` in `wasmi`) must produce **bit-for-bit identical outputs**, including edge cases (`-0.0`, `Inf`, `NaN`, overflow traps).
2. **NaN canonicalisation:** Same rule as `ir.md` §6: NaN payloads/signs are unobservable.
   - `VecBitcast` (float vec → int vec) canonicalises NaN lanes (`0x7FC0_0000` / `0x7FF8_0000_0000_0000`) via `v128.bitselect(canonical_nan, v, v.eq(v))` (3 vector instructions, branchless!).
   - Under `safety` profile, canonicalisation is additionally emitted after `f32x4.min`/`max` and before `VecStore` (note: in `wasmi 2.0.3` with `deterministic`, every arithmetic float lane op is already canonicalised **[V]**. Browsers need the `bitselect` sequence at `bitcast`/host boundaries; a Wasmtime-only artifact can instead rely on `cranelift_nan_canonicalization` **[V]** (sign of its canonical NaN **[U]**)).
3. **No Relaxed SIMD by default:** Wasm Relaxed SIMD (`f32x4.relaxed_madd`, `f32x4.relaxed_min`, `i8x16.relaxed_swizzle`) explicitly allows hardware-dependent rounding (FMA vs unfused) and out-of-range lane results ([WebAssembly Relaxed SIMD spec](https://github.com/WebAssembly/relaxed-simd)) **[V/S]**. Even wasmi, the reference engine, does not follow the Wasm 3.0 deterministic choice for `relaxed_madd` (§1.1 item 5). It is forbidden under `default`, `realtime`, and `safety` profiles; it may only be enabled in a future explicit `@profile(fast_math)` or opt-in flag that is surfaced in the placement report and manifest.

---

## 3. Auto-Vectorisation, `@expect(simd)` and Placement Reporting

### 3.1 Prior Art & Why OxiScript Takes an Honest, Narrow Approach

* **LLVM Loop Vectorizer (LV) & SLP Vectorizer** ([LLVM Vectorizers docs](https://llvm.org/docs/Vectorizers.html), [VPlan](https://llvm.org/docs/Proposals/VectorizationPlan.html)) **[S]**: LV widens countable inner loops across iterations using Scalar Evolution (SCEV), alias analysis (emitting runtime overlap checks), and a VPlan cost model; SLP packs isomorphic straight-line ops within a basic block. Despite decades of engineering, empirical studies show compilers vectorise **45–71% of synthetic-benchmark loops but "only a few loops from the real applications"** (Maleki et al., PACT 2011, [MSR](https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers)) **[V; corrected 2026-11-26 from an unsourced "18%–30%"]**, and **46%–54% of TSVC2 loops** on x86 (GCC 54%, ICX 50%, Clang 46%; [arXiv:2502.11906](https://arxiv.org/html/2502.11906v1)) **[V]**, failing silently with compiler-internal excuses (`-fopt-info-vec-missed`: "relevant phi not supported").
* **Cranelift** ([cranelift.dev](https://cranelift.dev/)): avoids "the aggressive optimization stance" of LLVM/gcc and "advanced alias analysis" **[V]**. That it has no loop auto-vectoriser is consistent with this but not stated there **[U]**. OxiScript should not rely on Wasmtime or browser JITs to vectorise.
* **ISPC (SPMD)** ([Pharr 2018](https://pharr.org/matt/blog/2018/04/18/ispc-origins)) **[V]**: Eliminates auto-vectorisation guesswork by making the parallel contract explicit (`foreach` / program instances across SIMD lanes, handling control flow via masks). Its performance warnings (`gather`/`scatter` alerts) are a useful precedent for placement remarks **[O]**. **[Corrected 2026-11-26: the cited ispc #2786 is about a false-positive undefined-behaviour warning, not about warnings differing between debug and release. Claim removed.]**
* **Halide** ([PLDI 2013](https://people.csail.mit.edu/jrk/halide-pldi13.pdf)) **[S]**: Separates pure functional algorithms from explicit schedules (`.vectorize(x, 4)`), guaranteeing predictability.

**Takeaway for OxiScript [O]:** Do **not** build a fragile LLVM-style speculative auto-vectoriser that tries to guess aliasing on arbitrary pointer-chasing `while` loops. Instead, build a **small, rock-solid, predictable OxiIR vectoriser v0** aimed at **explicitly structured data-parallel loops** (`parallel for`, `for (const i of range(0, n))`, and standard array combinators `map`/`zip`/`reduce`), backed by exact source-level placement diagnostics.

### 3.2 Loop Shapes Handled by Auto-Vectoriser v0 (on OxiIR SSA)

OxiIR is already a typed SSA IR with **block parameters** (no phi nodes), explicit `array.len`/`array.get`/`array.set`, explicit `Op::may_trap()`, and `#effects` attributes (`oxiscript-ir/src/types.rs`) **[V]**. A clean natural-loop pass over OxiIR (using the existing Cooper-Harvey-Kennedy dominator tree already in `oxiscript-ir/src/verify.rs` **[V]**) can reliably vectorise **exactly five well-specified shapes**:

| # | Loop Shape | Required Semantic Facts (Checked on OxiIR) | Lowered Vector Form (Width `W = 16 / sizeof(T)`) |
|---|---|---|---|
| **S1** | **Countable unit-stride map/zip** (`for (const i of range(0, n)) { out[i] = f(a[i], b[i]); }`, or `parallel for`) | Single induction block param `i: i32` starting at `0`, step `+1`, exit `i < n` (`n` loop-invariant, `n <= len` proven or hoisted); body uses only pure vectorisable ops (`Bin`/`Un`/`Cmp`/`Select`/`Conv`) on uniform element type `T` (`f32/f64/i32/u32/i16/u16/i8/u8`); **no `alloc`**, **no calls** (except `@inline`d pure helpers), **no RC ops**, **no traps inside the loop** (bounds hoisted; integer ops use `+%`/`-%`/`*%`/`+|`/`-|` or range-proven no-overflow); output array `out` does not alias input arrays (`out !== a && out !== b`, checked statically for fresh/distinct params or via a 1-instruction pointer-inequality guard before the loop). | Guarded `vec_loop` (`i + W <= n`) using `VecLoad` + vector ALU + `VecStore`, followed by scalar epilogue `tail_loop` (`i < n`). |
| **S2** | **Scalar-broadcast + affine read** (`out[i] = k * a[i] + c`) | Loop-invariant scalars `k`, `c` hoisted to `VecSplat` in loop preheader. | Same as S1 with preheader splats. |
| **S3** | **Branchless conditional (if-conversion)** (`out[i] = a[i] > 0.0 ? a[i] : 0.0`) | Diamond CFG (`BrIf` → two side-effect-free, trap-free blocks → single merge with block param) with no memory writes or calls in branches. | Flattened to `Cmp` (`Ty::VecMask`) + `VecSelect` (both arms evaluated; safe only because trap-free!). |
| **S4** | **Horizontal reductions** (`sum += a[i]`, `min`/`max`) | Single reduction block param `acc` updated by an associative, commutative op: integer `+%`, `&`, `|`, `^`, `min`, `max` **always**; floating-point `+` **only** via explicit `parallel reduce` / `a.reduceSum()` (because float addition is non-associative, changing `(a+b)+c+d` to `(a+b)+(c+d)` changes rounding and would violate sequential equivalence on an ordinary `for` loop!). | Vector accumulator `vacc: Ty::Vec` initialized to identity (`0`, `~0`, etc.), lane-wise accumulate in loop, fixed binary-tree horizontal reduction after loop + scalar tail. |
| **S5** | **Inline-struct field loops (AoS-to-vector or SoA)** | Once inline structs land (`contracts.md` §3 `Array<Particle>`), strided accesses of constant small stride (2 or 4 fields, e.g. `Particle { x, y, vx, vy }`) or SoA arrays (`xs: f32[], ys: f32[]`). **v0 recommends SoA first** (unit-stride S1) and defers de-interleaving shuffles (`i8x16.shuffle`) for AoS to v1, emitting `OXS-P0109` suggesting SoA layout! | Unit-stride for SoA; deinterleave/reinterleave shuffle pair for stride-2/4 AoS in v1. |

**Key safety rule on traps:** Because OxiScript integer `+`/`-`/`*` trap on overflow (`Ovf::Trap`), a loop containing `a[i] + b[i]` on `i32` **cannot** silently drop overflow checks. In v0:
- `f32`/`f64` arithmetic never traps → vectorises cleanly.
- Integer `+%`/`-%`/`*%`/`+|`/`-|` and bitwise `& | ^` never trap → vectorises cleanly.
- Trapping integer `+`/`-`/`*` inside a loop is **not** vectorised in v0 and emits a crisp diagnostic (`OXS-P0108`: "integer `+` can trap on overflow; use `+%` (wrapping) or `+|` (saturating) to vectorise"). This makes the performance model 100% transparent!

### 3.3 Placement Report, `@expect(simd)` and Reason Codes (`OXS-P01xx`)

Existing registry (`oxiscript-abi/registry/codes.txt:620-650`) **[V]** already establishes:
- `OXS-P0101` (`assertable no`): *Loop not vectorised: loop-carried dependence*
- `OXS-P0102` (`assertable yes`): *Loop not data-parallel*
- `OXS-P0201` (`assertable yes`): *Allocation on hot path*
- `OXS-P0301` (`assertable no`): *Host transfer per call*

**Critical contract (`abi.md` §6, `oxiscript-language.md` §L.3):**
- **`assertable yes` (semantic facts):** Computed purely from OxiIR structure and effects, **identical in debug and release**, and guaranteed to only get *stronger* within an edition. `@expect(simd)` checks that the loop satisfies the language-defined vectorisable shape (S1–S4) **and** has no disqualifying semantic blocker (`P0102`–`P0108`).
- **`assertable no` (target/heuristic choices):** Target lacks SIMD128 (e.g., ESP32-S3 scalar build), trip count too small (`n < 8`), or heuristic cost model declines. Reported in `placement.json` and LSP inlay hints (`scalar: target has no SIMD128`), **never breaks `@expect(simd)`** unless target-scoped (`@expect(simd, on: "wasm-simd128")`).

**Proposed append-only `OXS-P01xx` codes for slice 8.3b (the vectoriser; plan stage 8.3), never modifying the existing `P0101`/`P0102`.** Note: `P0101` (loop-carried dependence) is `assertable no` in the registry, so a dependence the language can prove (not a heuristic) should be reported under `P0102` (`assertable yes`).

| Code | `assertable` | Title | Source-Level Message & Actionable Fix |
|---|---|---|---|
| `OXS-P0103` | `yes` | Loop not vectorised: call or allocation in body | `loop at {loc} calls `{callee}` (effect: {effect}); inline a pure helper or hoist allocation` |
| `OXS-P0104` | `yes` | Loop not vectorised: unsupported control flow | `loop at {loc} has `{kind}` (early exit, inner loop, or multi-exit branch); use a counted loop with branchless `? :`` |
| `OXS-P0105` | `yes` | Loop not vectorised: non-unit or indirect indexing | `access `{arr}[{idx}]` at {loc} is not contiguous unit-stride `i`; use contiguous arrays or explicit `Simd.load`` |
| `OXS-P0106` | `yes` | Loop not vectorised: non-associative float reduction | `loop at {loc} accumulates float `{var}` sequentially; use `parallel reduce` to allow tree-order SIMD summation` |
| `OXS-P0107` | `yes` | Loop not vectorised: unsupported element type | `type `{ty}` at {loc} is a managed reference or non-vectorisable aggregate; store inline scalars or SoA arrays` |
| `OXS-P0108` | `yes` | Loop not vectorised: trapping arithmetic | `operator `{op}` on `{ty}` at {loc} traps on overflow; use wrapping (`+%`, `*%`) or saturating (`+|`) ops to vectorise` |
| `OXS-P0109` | `no` | Loop not vectorised: strided struct layout (AoS) | `struct array `{arr}` at {loc} has stride {stride} B; use Structure-of-Arrays (SoA) or explicit `Simd` for full bandwidth` |
| `OXS-P0110` | `no` | Loop vectorised (`simd{width}`) | `loop at {loc} vectorised to `Simd<{ty}, {width}>` (guard: {alias_guard})` — positive placement fact for inlays & `placement.json` |
| `OXS-P0111` | `no` | Vectorised loop lowered to scalar for target | `loop at {loc} is SIMD-eligible (`Simd<{ty}, {width}>`) but target `{target}` disables Wasm SIMD128` |
| `OXS-P0112` | `no` | Runtime alias guard inserted | `loop at {loc} emits a fast-path check `{a} !== {b}` (scalar fallback if arrays alias)` |

**How `@expect(simd)` works end-to-end:**
1. Parser/checker already accepts `@expect(...)` on statements and functions (`grammar.md` §4 **[V]**) and preserves `#expect(simd)` in OxiIR (`ir.md` §1 **[V]**).
2. The vectoriser pass runs on **all** `parallel for` loops, loops annotated `@expect(simd)`, and countable inner loops (only reporting declined remarks for `parallel for` and `@expect(simd)` loops by default to keep noise low; `--report=full` reports all loops).
3. If an `@expect(simd)` loop hits any `assertable yes` rejection (`P0102`–`P0108`), compilation fails with the exact code, source span, and fix suggestion—turning silent performance regressions into immediate, actionable compile errors.

**[Revised 2026-11-26 for decision 4 transparency] Runtime phase.** The same `Remark` format also carries run-time placement facts (which executor actually ran a loop, and why); `@expect(gpu)` and the new reason codes are in section 4.6.7.

---

## 4. GPU Kernels (Stages 8.3d-8.3e): Profile, Lowering, Transparent Execution, Buffers & Determinism [Revised 2026-11-26 for decision 4 transparency]

**[Revised 2026-11-26 for decision 4 transparency]** Read section 4.6 first: it is the normal way to use the GPU. Sections 4.2-4.5 define the profile, the WGSL lowering and the numeric contract that 4.6 relies on; section 4.4 (`host:gpu`) is the low-level layer underneath, for libraries and experts.

OxiScript's principle (`oxi-ui.md` Phase 8, `contracts.md` §3): **no separate shader language glued on through strings**. GPU kernels are ordinary OxiScript functions marked `kernel function` (or `@profile(kernel)`), type-checked by the same front end, lowered to the same OxiIR with a `#kernel` attribute, and compiled to **either** WGSL (for GPU execution) **or** CPU SIMD/scalar loops (for GPU-less targets, WebGL2/GLES3 fallbacks, and deterministic reference tests).

### 4.1 Prior Art Analysis

| System | Licence | Model & Architecture | Key Takeaway for OxiScript 8.3d-8.3e |
|---|---|---|---|
| **CubeCL / Burn** ([github.com/tracel-ai/cubecl](https://github.com/tracel-ai/cubecl)) **[V: licence + backends]** | **MIT OR Apache-2.0** | `#[cube]` Rust functions expanded to a typed SSA CubeIR, then compiled on demand to WGSL, SPIR-V, CUDA, HIP, Metal, and CPU SIMD. | **Closest architectural analogue.** Proves a restricted typed subset + own SSA IR → WGSL/SPIR-V/CPU works in production for real ML (Burn). |
| **`rust-gpu`** ([transition blog](https://rust-gpu.github.io/blog/transition-announcement)) **[V]** | MIT / Apache-2.0 | Compiles full Rust via `rustc_codegen_spirv` to SPIR-V. Needs a (pinned) nightly `rustc` **[V]**. The Aug 2024 post announces a move from Embark to community ownership **[V]**. **[Corrected 2026-11-26: "Embark archived it in Oct 2025" is not in the cited source.]** | **Warning:** hooking into a general host compiler's internals is fragile. Owning OxiIR + a small `#kernel` profile avoids this completely. |
| **Taichi** ([docs.taichi-lang.org](https://docs.taichi-lang.org/)) **[S]** | **Apache-2.0** | Python `@ti.kernel` restricted static subset JIT-compiled to Vulkan, Metal, CUDA, OpenGL, CPU. | Validates "restricted static subset of the host language" ergonomics for physics, particles, and graphics. |
| **Futhark** ([futhark-lang.org](https://futhark-lang.org/)) **[S]** | ISC | Pure functional array language → `GPUMem` IR → CUDA, OpenCL, and WebGPU (via C + `webgpu.h` + Emscripten). | Uniqueness types give race-free writable buffers without runtime alias checks; WebGPU backend confirms `webgpu` compute viability. |
| **Mojo GPU** ([Modular docs](https://docs.modular.com/mojo/manual/gpu/fundamentals/)) **[S]** | Apache-2.0 w/ LLVM exc. (compiler contributions restricted) **[S]** | Imperative kernels + `LayoutTensor` (compile-time strides/tiling) via MLIR to NVIDIA/AMD. | Explicit stride/tile tensor views (`View2D<T>`) eliminate error-prone flat index arithmetic. |
| **Triton** ([github.com/triton-lang/triton](https://github.com/triton-lang/triton)) **[S]** | **MIT** | Block-level Python DSL targeting PTX/AMDGCN via MLIR; programmer thinks in tiles rather than scalar threads. | Tile-level abstractions shine for GEMM/attention, but require heavy MLIR machinery; OxiScript starts with thread/workgroup SPMD (WGSL-native) and builds tiled GEMM as a library kernel. |
| **Slang** ([shader-slang.org](https://shader-slang.org/), [Khronos](https://www.khronos.org/news/permalink/khronos-group-launches-slang-initiative-hosting-open-source-compiler-contributed-by-nvidia)) **[S]** | **Apache-2.0 w/ LLVM exc.** | Modern shading language with modules, generics, interfaces, autodiff → SPIR-V/WGSL/MSL/HLSL/CUDA. | Great reference for generics-in-shaders and autodiff; too heavy (C++) to embed in OxiScript's wasm32 core. |
| **Halide GPU** ([PLDI 2013](https://people.csail.mit.edu/jrk/halide-pldi13.pdf)) **[S]** | MIT | Algorithm/schedule split targeting OpenCL/CUDA/Metal/WebGPU. | Confirms workgroup/tile parameters belong in declarative schedules/annotations (`@workgroup(16, 16, 1)`). |
| **`wgpu` compute examples** ([wgpu repo](https://github.com/gfx-rs/wgpu/tree/trunk/examples/features/src/boids)) **[V/S]** | **MIT OR Apache-2.0** | Reference WGSL compute shaders (boids particles, hello-compute, ray tracing) over `wgpu` 30. | Directly demonstrates `StorageBuffer`, ping-pong particle buffers, and compute-to-render handoff on one `wgpu::Device`. |

### 4.2 The `kernel` Profile (`#kernel` on Functions)

`kernel` is already reserved in `grammar.md` §1 & §5 (`FnModifiers::kernel`) and `oxiscript-abi/registry/codes.txt` (`OXS-E0500`) **[V]**. A `kernel function` is checked by `oxiscript-check` (8.T) and re-verified by `oxiscript-ir::verify`:

**[Revised 2026-11-26 for decision 4 transparency]** (How such a function is called without dispatch, and how `BufferView` relates to `Resident<T[]>`: section 4.6.2a.) A `kernel function` makes the checks below mandatory. Layer-0 code gets a WGSL variant only if the same checks pass when inferred from the code (section 4.6.2).

1. **Forbidden constructs (`OXS-E0500`):**
   - No managed heap allocation (`alloc`, `array.new`, `array.push`, `str.*`, `closure.new`, `clone`, nullable boxed scalars).
   - No RC instructions (`retain`, `release`, `is_unique`)—all kernel data is plain unmanaged data.
   - No host capability calls (cannot take `Log`, `State`, etc.) and no module `let` mutable globals (constants `const` are allowed and inlined).
   - No recursion (static call-graph cycle check on `#kernel` + called pure helper functions) and no `dyn`/closure indirect calls.
   - No `string`, `class`, or tagged unions containing managed references (plain C-like `enum`s and plain-data `struct`s are allowed!).
   - **Portable numeric subset:** `bool`, `i32`, `u32`, `f32`, `Simd<f32, 4>`/`Simd<i32, 4>`/`Simd<u32, 4>` (mapping to WGSL `vec4<T>`), plus optional `f16` (gated by `@require(gpu_f16)`). Narrow `i8`/`u8`/`i16`/`u16` are allowed in packed storage views (explicit unpack/pack via `u32` shifts in WGSL, since WGSL lacks native 8-bit/16-bit integer storage types **[V: [W3C WGSL](https://www.w3.org/TR/WGSL/#scalar-types)]**), while **`i64`/`u64`/`f64` are rejected on GPU kernels** (`OXS-E0500`, because WebGPU/WGSL has no 64-bit scalar types **[V]**; CPU-only kernels may still allow them with a clear target diagnostic).
2. **Bounded / structured loops & uniformity:**
   - All loops in `#kernel` must be structured (reducible CFG, already required by `oxiscript-wasm` `OXS-E0600` **[V]**) and carry either a statically bounded `for (const i of range(0, N))` or an explicit upper bound (`@max_iters(N)`), so the WGSL emitter can guarantee termination (see §5.5).
   - Workgroup barriers (`kernel.barrier()` → WGSL `workgroupBarrier()`) must occur in **uniform control flow** (checked against WGSL's static uniformity rules ([W3C WGSL Uniformity](https://www.w3.org/TR/WGSL/#uniformity)) **[S]**—values derived from `kernel.globalId()`/`localId()` are non-uniform; branches on non-uniform booleans cannot contain barriers).
3. **Parameters & uniqueness (no aliasing):**
   - Kernel parameters are strictly:
     - Read-only device buffers: `BufferView<T>` (maps to WGSL `var<storage, read> : array<T>`)
     - Writable device buffers: `inout BufferView<T>` or `MutBufferView<T>` (maps to WGSL `var<storage, read_write> : array<T>`)
     - Storage textures: `ImageViewOut<Rgba8>` (maps to WGSL `var : texture_storage_2d<rgba8unorm, write>`)
     - Plain-data uniform parameters: scalars (`i32`, `u32`, `f32`) or plain `struct`s packed into a single `@group(0) @binding(0) var<uniform> _params` block!
   - **Alias rule:** At dispatch time, the `host:gpu` runtime rejects binding the same `BufferHandle` both read-write and read (or twice read-write) in the same dispatch (`STATUS_ALIASING_CONFLICT`), matching WebGPU's own validation rule ([WebGPU Spec §21.2](https://www.w3.org/TR/webgpu/)) **[S]**.
   - **Plain-data `struct` layout compatibility:** OxiScript's `layout_fields` uses natural alignment (`abi.md` §2.3), which matches WGSL `std430` storage-buffer layout for `i32`/`u32`/`f32`/`vec2`/`vec4` **except** for 3-component vectors (`vec3` has 16-byte alignment in WGSL) and trailing struct padding! The WGSL emitter verifies `std430` layout compatibility at compile time and pads/rejects incompatible fields explicitly (`OXS-E0500`), ensuring host `Array<Particle>` (inline structs) can upload raw bytes with zero CPU repacking!

### 4.3 Lowering OxiIR → WGSL: Why Emit WGSL Text Directly?

Three backend paths exist for lowering `#kernel` OxiIR to `wgpu`:
1. **Emit WGSL text directly** (`oxiscript-wgsl` crate, ~1,500–2,500 LOC, zero runtime dependencies) → pass `wgpu::ShaderSource::Wgsl`.
2. **Build `naga::Module` IR directly** → pass `wgpu::ShaderSource::Naga`.
3. **Emit SPIR-V binary directly** → pass `wgpu::ShaderSource::SpirV`.

**Direct verification in `wgpu-30.0.2/src/backend/webgpu.rs:1905-2000` [V] settles this decisively:**

| Criterion | Emit WGSL Text (Recommended) | Emit `naga::Module` IR | Emit SPIR-V Binary |
|---|---|---|---|
| **Browser WebGPU (`wgpu` wasm32)** | **Zero overhead:** `ShaderSource::Wgsl` passes straight to `GpuShaderModuleDescriptor::new(code)` **[V]**. No `naga` in WASM bundle! | Forces `wgpu` to pull `naga` + `wgsl-out` into wasm32 and serialize back to WGSL text at runtime **[V]**. | Forces `naga` `spv-in` + `wgsl-out` + `petgraph` into wasm32 **[V]**! WebGPU browsers reject SPIR-V natively. |
| **Native `wgpu` (Vulkan/Metal/DX12)** | `wgpu` already bundles `naga` (`wgsl-in` → `spv-out`/`msl-out`/`hlsl-out`) and compiles WGSL automatically **[V]**. | Slightly skips WGSL lex/parse (~0.2 ms), couples OxiScript tightly to Naga's unstable internal AST across every `wgpu` major release (every few months!). | Still translated through Naga on Metal/DX12; bypasses WGSL uniformity checks. |
| **Debugging, golden tests, AI-friendliness** | Human-readable, diffable text; trivially validated offline in CI using `naga` as a **dev-dependency** (`wgsl::parse_str` + `valid::Validator`). | Opaque Rust structs; hard to snapshot. | Binary format; requires `spirv-dis`. |

**Recommendation [O]:** Implement **`oxiscript-wgsl`** emitting clean, deterministic WGSL text directly from verified `#kernel` OxiIR.
- **Control flow:** Reuse the exact same dominator + structured-CFG ("Beyond Relooper") algorithm already proven in `oxiscript-wasm/src/cfg.rs` **[V]**! Every OxiIR block parameter becomes a WGSL local `var`, and `Br`/`BrIf`/`Switch` map cleanly to `loop`/`if`/`switch`/`break`/`continue`.
- **Bounds checking in WGSL (mandatory):** every array access `buf[idx]` must be emitted as an **explicit guarded read/write** (`if (idx < arrayLength(&buf)) { ... } else { <defined result> }`). **[Corrected 2026-11-26]** WebGPU's robust buffer access does *not* define a single OOB result. An OOB load may return a value from *any in-bounds location* or zero; an OOB store may land at *another in-bounds location* or be discarded; the dispatch may be partly discarded; and implementations differ: naga offers `Restrict` (any in-bounds index), `ReadZeroSkipWrite` and `Unchecked` (its `Default`), and the backend picks the policy (`naga-30.0.2/src/proc/index.rs:42-134`) ([RBA notes](https://kvark.github.io/webgpu-debate/RBA.html)). Without our own guards, `@exact` CPU/GPU parity is impossible. The guarded branch defines OxiScript's kernel OOB rule: the read yields zero, the write is skipped, and in debug builds an atomic `oob_flag` is set and reported as `TrapKind::Bounds` after the dispatch.
- **Offline validation:** In tests and `oxi build --check-gpu`, run `naga::front::wgsl::parse_str` + `naga::valid::Validator` (MIT OR Apache-2.0, already in `Cargo.lock` as `naga 30.0.2` **[V]**) so WGSL validity and uniformity are verified in CI even without a GPU!
- **Packaging kernels inside `.wasm`:** Each compiled `#kernel` function embeds its WGSL source string (plus binding metadata & workgroup size) in a read-only custom section **`oxi.kernels`** (or immutable static data) inside the `.wasm` module, alongside a **compiled CPU fallback function** in the Wasm code section! Thus one `.wasm` file runs identically on WebGPU, Vulkan/Metal/DX12, WebGL2-only browsers (CPU fallback), and ESP32 (CPU fallback).

### 4.4 Buffers (`Buffer<T>`), `host:gpu@1` IDL, Dispatch & Readback [Revised 2026-11-26 for decision 4 transparency]

> **[Revised 2026-11-26 for decision 4 transparency] Low-level layer.** Everything in 4.4 is the *expert* interface: libraries (`std/tensor`, `std/image`) and people who need manual control use it; normal programs never compile kernels, bind buffers or dispatch (section 4.6). It still requires the `Gpu` capability in `main`, and the corrected safety rules (no guest WGSL text, module-verified kernels only, bounds guards, error-scope isolation, quotas) apply unchanged. The executor path of 4.6 is implemented on top of the same `GpuArena`, validation and error-scope code.


Per `contracts.md` §3 and `abi.md` §5, GPU access is an ordinary capability **`host:gpu@1`** (zero ambient authority; scripts without `Gpu` in `main` cannot touch the GPU):

```text
// oxiscript-abi/idl/gpu.idl (proposed for Stage 8.3e; was 8.3d: [Revised 2026-11-26 for decision 4 transparency])
interface Gpu "host:gpu" @1 {
    handle Buffer;
    handle Texture;
    handle Kernel;
    handle Fence;

    // Capability query (0 = CPU-only fallback, 1 = WebGPU compute, bit 1 = f16, bit 2 = subgroups)
    fn caps() -> u32;

    // Lifecycle (all fallible -> result<...>, so quota/OOM/device-lost never silently panics)
    fn create_buffer(byte_len: u32, usage: u32) -> result<Buffer>;
    fn destroy_buffer(buf: Buffer) -> result<void>;
    fn create_texture_rgba8(width: u32, height: u32, usage: u32) -> result<Texture>;
    fn destroy_texture(tex: Texture) -> result<void>;

    // Explicit transfers (visible in types and placement report OXS-P03xx!)
    fn upload(buf: Buffer, byte_offset: u32, data: bytes) -> result<void>;
    // Async readback respecting abi.md §5.3 (no re-entrancy) & WebGPU mapAsync:
    callback OnReadbackReady(fence: Fence, status: i32, byte_len: u32);
    fn request_download(buf: Buffer, byte_offset: u32, byte_len: u32, cb: OnReadbackReady) -> result<Fence>;
    // Guest pulls ready bytes using the standard abi.md §5.2 out-buffer protocol (handles >4 KiB cleanly!)
    fn read_download_chunk(fence: Fence, offset: u32) -> result<bytes>;

    // Kernel compilation & dispatch. [Corrected 2026-11-26] The host compiles only kernels from the
    // calling module's own `oxi.kernels` section, validated with naga before create_shader_module.
    // A guest-supplied WGSL string would bypass the kernel profile, the injected loop fuel and the
    // std430 checks.
    fn compile_kernel(kernel_index: u32) -> result<Kernel>;
    fn dispatch(
        kernel: Kernel,
        wg_x: u32, wg_y: u32, wg_z: u32,
        bindings: bytes,   // packed (binding_idx, kind, handle_id) table
        uniforms: bytes    // packed uniform struct bytes
    ) -> result<void>;
}
```

**Why this IDL fits `abi.md` 1.0/1.1 without breaking any invariant:**
1. **No re-entrancy (`abi.md` §5.3):** `request_download` initiates `copy_buffer_to_buffer` + `map_async` asynchronously on the host and invokes `OnReadbackReady` only at a quiescent point between guest turns (matching both browser WebGPU's mandatory `Promise`-based `mapAsync` and OxiUI's step loop!).
2. **Large transfers (> `HOST_PAYLOAD_MAX` = 4096 B):** Callbacks are limited to 4096 bytes in scratch memory (`abi.md` §5.4 **[V]**), so `OnReadbackReady` delivers **only the fence handle + byte length** (zero payload!), and the guest reads the mapped staging bytes via `read_download_chunk` using the existing out-buffer protocol (`abi.md` §5.2), which supports arbitrarily large guest `[u8]` arrays without touching scratch memory! Similarly, `upload` passes a borrowed `[u8]` slice directly from guest linear memory (zero `HOST_PAYLOAD_MAX` cap on guest-to-host `bytes` borrows).
3. **Typed OxiScript wrapper (`std/gpu`):** Programmers never touch raw byte offsets or `u32` handles.

   *Illustrative proposed syntax, not valid OxiScript today (corrected 2026-11-26). `?` requires `main` to return `Result<_, i32>`, not `void` (AGENTS.md rule 4). The `download` callback body is elided. `kernel function`, `inout`, `@workgroup` and `BufferView` are reserved or proposed.*
   ```ts
   import { Gpu, Buffer } from "host:gpu";

   @workgroup(64)
   kernel function integrate(
     pos: inout BufferView<f32>,
     vel: BufferView<f32>,
     dt: f32
   ): void {
     const i = kernel.globalId().x;
     if (i < pos.length) {
       pos[i] = pos[i] + vel[i] * dt;
     }
   }

   export function main(gpu: Gpu): void {
     const pos = gpu.upload<f32>([0.0, 1.0, 2.0, 3.0])?;
     const vel = gpu.upload<f32>([1.0, 1.0, 1.0, 1.0])?;
     gpu.dispatch(integrate, [pos.length], pos, vel, 0.016f32)?;
     // Async readback (callback in 8.3e; `await` sugar once 8.4 async lands):
     pos.download((result) => { ... })?;
   }
   ```
   When `gpu.caps() === 0` (WebGL2/GLES3/no-GPU host), `std/gpu`'s `dispatch` transparently runs the embedded **CPU SIMD/scalar fallback** for `integrate` on host-side shadow buffers, invoking the same `download` callback—so the exact same program runs everywhere!

### 4.5 Determinism Contract: CPU vs GPU Floating-Point

GPU hardware cannot guarantee bit-identical floating-point results with CPU IEEE-754 (`ir.md` §6):
- **WGSL specification accuracy** ([W3C WGSL Floating Point Evaluation](https://www.w3.org/TR/WGSL/#floating-point-evaluation)) **[S]**: `f32` `+`, `-`, `*` are correctly rounded, `/` is ≤ 2.5 ULP, `sqrt` ≤ 0.5–1 ULP depending on range, transcendentals (`sin`, `exp`, `log`) have bounded ULP tolerances, **subnormals may be flushed to zero (FTZ)** (modelled this way by the WebGPU CTS fp primer), **NaN payloads/signs are unspecified**, and implementations may **reassociate and fuse** (`a * b + c` → FMA). **[Corrected 2026-11-26: WGSL offers *no* way to inhibit fusion or reassociation, so every float kernel is inherently `@approx`.]** The exact ULP figures above are [S]: I could not fetch WGSL §15.7 directly.
- **OxiScript's honest contract (`ResultClass`):**
  1. **`@exact` kernels (integer-only `i32`/`u32` + wrapping/bitwise ops, no float math or unordered atomics):** Bit-for-bit identical across CPU scalar, CPU SIMD128, and every GPU vendor. Permitted under `deterministic` and `safety` profiles (still verifiable against CPU reference!).
  2. **`@approx` kernels (default for float `#kernel`s):** Defined as IEEE-754 `f32` within WGSL ULP bounds + FTZ tolerance (e.g., `abs_tol = 1e-6`, `rel_tol = 1e-5` in differential GPU-vs-CPU test harnesses). Allowed in `default` and `realtime` profiles; **rejected under `safety` profile** (`OXS-E0500`) unless explicitly routed to the CPU deterministic reference path.
  3. **Unordered reductions / atomics:** Floating-point parallel reductions on GPU are explicitly typed as `@approx`; integer atomic adds (`atomicAdd`) are commutative/associative so their final sum is `@exact`.

**[Revised 2026-11-26 for decision 4 transparency] Applying the contract when the runtime chooses the executor.** (1) Integer results are identical on every executor; this is why GPU eligibility excludes trap-capable operations (4.6.2, 4.6.5). (2) Float results of layer-0 code are `@approx` on the GPU executor and bit-identical between scalar and SIMD128 (section 2.2). A program may therefore see different low-order float bits on a host with a GPU than on one without; the contract bounds the difference, and `safety` code never uses the GPU executor for floats (as for any future deterministic profile; `deterministic` is not a profile in `abi.md` today, **UNVERIFIED**). (3) A float reduction has the order of its documented contract (`reduceSum` is tree order; plain sequential float accumulation is not parallelised, `OXS-P0106`), never "whatever the GPU did".

### 4.6 One source, several executors; residency in types [Revised 2026-11-26 for decision 4 transparency]

#### 4.6.1 Model

Decision 4: *parallel and SIMD constructs have the semantics of sequential code; GPU is a backend.* One OxiScript source (layer-0 combinators, `parallel for`, `kernel` functions) is compiled to **executor variants**. A variant is a lowering of the same OxiIR, never a different program.

| Executor | Variant | Exists when | Stage |
|---|---|---|---|
| scalar | scalar Wasm loop | always | 8.3a |
| SIMD128 | `v128` loop (explicit `Simd<T,N>`, or auto-vectorised per section 3) | target has SIMD128 (`web`, `server`; `device` provisional) | 8.3a/8.3b |
| GPU | WGSL compute shader in the `oxi.kernels` section (4.3) | code is GPU-eligible (4.6.2) | 8.3d |
| threads | chunked isolates / Wasm threads | later (decision 4 layer 2) | 8.4 (the Remark format already has an `executor` value for it) |

The runtime chooses among *existing* variants by the policy of 4.6.4. The manifest records which variants a module carries (4.6.6).

#### 4.6.2 When a WGSL variant exists (eligibility, inferred or declared)

A layer-0 body gets a WGSL variant only if it satisfies the `kernel` profile (4.2), checked on the code, not annotated. `kernel function` makes the same check mandatory and an error if it fails. Concretely:
* The function passed to `map`/`zip`/`reduce` must be statically known and inlinable (no closure values from variables, no recursion).
* **No `f64`, `i64`, `u64`** (not in WGSL, 4.2). **An unsuffixed float literal defaults to `f64`** (AGENTS.md rule 1), so most float code is CPU-only unless written with `f32` (`x * 2.0f32`, or typed `f32` operands that make the literal `f32`). The report must say this plainly: `OXS-P0302` names the first `f64` use as the reason.
* No managed allocation, strings, classes, host calls, or capabilities (4.2).
* **Trap-free by construction (v1).** CPU integer `+ - *` overflow, integer `/` and `%` by zero, and out-of-range indexing all trap; on the GPU a guard can only return a value or skip a write (Q4). To keep integer results identical on every executor without a hidden trap channel, v1 eligibility excludes plain trapping integer ops (use `+%`, `+|`, as for SIMD, `OXS-P0108`) and any array index the checker cannot prove in range. **Provable shapes (the proof scope, chosen so that tiled GEMM, im2col/conv2d and attention stay eligible):** (a) the implicit index of `map`/`zip`/`parallel for`; (b) `i` guarded by a dominating `i < xs.length`; (c) **affine indices** `a*u + b*v + c` over `range()` loop variables, `globalId`/`localId` and constants, whose maximum is bounded by a shape guard the checker can see (`row < M && k < K` against `A.length === M * K` stated as a launch precondition); (d) `@shared` tile indices bounded by the constant tile size and `localId` (< workgroup size); (e) a **shaped view** `View2D<T>`/`View1D<T>` (the Mojo `LayoutTensor` idea, 4.1) whose `(row, col)` access is checked against its shape at the launch boundary (one check per launch, not per access), so kernels written against views are eligible without hand-written guards. Anything else (data-dependent gather such as `xs[idx[i]]`) gets `OXS-P0302` reason `unproven index`. WGSL behaviour on integer division by zero or overflow is **UNVERIFIED** in the ledger; the emitter must not rely on it.
* `filter` and other size-changing combinators need stream compaction (a scan). **v1: CPU only**, with `OXS-P0302` reason `filter`; a scan-based variant is future work (Q16).
* `reduce` is eligible only for operations whose parallel order cannot change the result for integers (`+%`, `min`, `max`, bitwise) or for the explicit `reduceSum`/`parallel reduce` float forms (`@approx`).

#### 4.6.2a How a `kernel function` is called (no dispatch in normal code)

* **Launch domain from the call.** A kernel is invoked like a function with a launch shape: `integrate.launch([n], pos, vel, dt)` (explicit range/shape as first argument), or inside `parallel for (const i of range(0, n)) { integrate1(pos, vel, dt, i); }` where the per-element form is an ordinary function and the compiler builds the launch. There is no handle, no binding table, no `dispatch`.
* **Caller-side arguments** are `Resident<T[]>` (`inout` for writes) or host `T[]`, under the residency rules of 4.6.3 (mixing is an error; host arrays run the CPU/SIMD version, `Resident` arguments the GPU version). The call returns after enqueueing; observation of results is through `download` or another combinator.
* **`BufferView<T>`, `ImageViewOut` and `View2D<T>` exist only inside the kernel body and in the WGSL lowering** (4.2): the caller's `Resident<T[]>` argument becomes the view; a parameter list written with views in 4.2 is the callee side of the same call.
* **`@workgroup(...)`, `@shared` sizing hints** affect only the GPU variant; the CPU version ignores them (it runs the same body per element/tile), so results do not depend on them.
* `host:gpu` (4.4) remains the way to compile kernels by index and dispatch manually; the example there is the expert path.

#### 4.6.3 Residency in types: proposal `Resident<T>`

`Resident<T[]>`, `Resident<Image>`, `Resident<f32>` mean "this data lives in the accelerator's memory domain". Reasoning for the name:
* **Not `Device<T>`:** `oxi build --target device` (8.2) already names ESP32-class devices; two meanings of "device" in one language is a trap for readers and for the LSP.
* **Not `Gpu<T>` (runner-up):** more discoverable, but on a host without a GPU the data is plain host memory, and the type must not promise a GPU. `Resident` says what the *type system* tracks (where the data is) and stays true in both cases.

Semantics:
* **Type-checking never depends on runtime availability.** `Resident<T>` has one API everywhere. On a host without the Compute tier it is host memory (the same bytes a `T[]` would have); operations run the CPU version of the same code. No transfer exists in that case, so none is reported.
* **Transfers are visible calls** in `std/resident`, not syntax magic: `upload(xs: T[]): Result<Resident<T[]>, i32>` (fallible: quota) and `download(r: Resident<T>, cb)`; the callback form is required until 8.4 async, because readback is asynchronous (browser `mapAsync`, `abi.md` §5.3 no re-entrancy; section 4.4). Each call is one transfer line in the report.
* **No operation returns host data from resident inputs.** `reduceSum` over `Resident<f32[]>` returns `Resident<f32>`, not `f32` (a host scalar would be a hidden transfer); it reaches the host only through `download`. Likewise `.length`-style metadata is known statically or kept host-side and is not a transfer.
* **Mixing is a compile error:** a combinator or call with both `Resident` and plain `T[]` inputs fails (code to be allocated in the `OXS-E` range, **UNVERIFIED** free; message `mixed residency at {loc}: upload {arg} or download {arg}` with the fix naming the call). A `Resident` input whose body has no WGSL variant is also a compile error (the type demands eligibility; the check is static so it cannot depend on the host).
* Resident values are handles owned by the instance's arena: hot reload and drop free them (section 5.3); after device loss, operations on them fail (Q15).

Illustrative proposed syntax, not valid OxiScript today (fenced `ts` on purpose):
```ts
import { Log } from "host:log";
import { upload, download } from "std/resident";

// Host data: layer-0 runs on CPU (SIMD128 if the target has it, else scalar). Never GPU.
function scaled(xs: f32[], k: f32): f32[] {
  return xs.map((x) => x * k);
}

// Resident data: same combinator, now the GPU executor (or host memory without a GPU).
function energy(xs: Resident<f32[]>, k: f32): Resident<f32> {
  return xs.map((x) => x * k).reduceSum();
}

function run(xs: f32[], log: Log): Result<void, i32> {
  const r = upload(xs)?;                   // visible host -> resident transfer
  const e = energy(r, 0.5f32);             // no transfer here, in any executor
  download(e, (v: f32) => {                // visible resident -> host transfer, callback
    log.write(2, `energy = ${v}`);
  });
  return Ok(null);   // success value for Result<void, i32>: spelling UNVERIFIED
}
```

#### 4.6.4 Executor policy (deterministic, documented)

Hidden transfers killed C++ AMP (decision 4), so **the runtime never inserts a transfer**, and a clause such as "use the GPU when the transfer is amortised" is deliberately *not* part of the default policy (it would be a hidden transfer; see Q12). The policy is a pure function:

`executor = f(residency type of the inputs, host compute tier, profile, target)`

| Inputs (static types) | Host | Executor chosen |
|---|---|---|
| all host `T[]` / `Image` | any | SIMD128 if the target has it and the loop is vectorisable (`OXS-P0110`), else scalar; threads once 8.4 exists |
| all `Resident` | `ComputeTier::Compute`, variant exists, profile allows | GPU |
| all `Resident` | `GraphicsOnly` or no GPU | CPU version of the same code over host memory (no transfer) |
| mixed | any | compile error (4.6.3) |
| float code under `safety` (and a possible future `deterministic` profile, **UNVERIFIED**: `abi.md` lists only `default\|realtime\|safety\|kernel`) | any | scalar or SIMD128 only (4.5); `Resident` float code is then a compile error in these profiles |

* Same inputs (types, tier, profile, target) give the same executor on every run: no hidden state, no timing, no adaptive tuning. Quotas never change the executor: `upload` returns `Err` when the VRAM quota is exceeded; a dispatch cap (section 5.5: invocations per dispatch, dispatches per turn) traps with a `quota` trap kind instead of silently re-routing (proposed; Q15).
* **Size thresholds are report hints only.** A host loop over at least `N_hint` elements emits `OXS-P0305` ("large host loop; consider `Resident`"); a `Resident` operation below `N_small` emits an info that dispatch overhead may dominate. Provisional values `N_hint = 65 536`, `N_small = 4 096` **[P, placeholder: measure in the 8.3d benchmarks, Q13]**.
* The GPU variant dispatches in chunks within the section 5.5 caps; chunking never changes results (elementwise) and reductions fix their tree order independent of chunking.

#### 4.6.5 Same results on every executor

* **Integers:** bit-identical on scalar, SIMD128 and GPU, including the trap behaviour, because eligible code cannot trap (4.6.2). Gate: three-executor differential test (7.2, 8.3d).
* **Floats:** within the section 4.5 contract (`@approx`: `abs_tol = 1e-6`, `rel_tol = 1e-5` in the harness); scalar and SIMD128 bit-identical. Reduction order per 4.5.
* Integer-only `@exact` code may use any executor under any profile.

#### 4.6.6 Authority and manifest

Pure compute on the GPU executor grants no authority, like SIMD: it reads and writes only the program's own `Resident` data, cannot name devices, buffers or shader text, and has no host calls (kernel profile). It therefore needs **no `main` parameter** (AGENTS.md rule 7 is about authority); it is limited by host quotas (VRAM, dispatch caps, section 5.3/5.5). `host:gpu` (4.4) still requires the capability. The manifest records the variants a module carries (proposal, with the ABI 1.1 `simd128` line: `variants wgsl`); a host without the Compute tier **ignores** WGSL variants (unlike a missing Wasm feature, which refuses the module), because a CPU version always exists. This holds only for hosts that understand ABI 1.1: `abi.md` makes older hosts refuse unknown manifest directives, so a module carrying the `variants` line needs the ABI minor bump. Whether `needs`/manifest should also declare "may use GPU" so an operator can forbid it is Q14.

#### 4.6.7 The placement report as transparency window; `@expect(gpu)`

For every loop and kernel the report states where it ran or will run (scalar / SIMD128 / GPU / threads) and why. Remarks come in two phases in one format:

```text
remark { code, severity, phase: "compile" | "run", loc,
         executor: "scalar" | "simd128" | "gpu" | "threads",
         reason, inputs: ["host" | "resident"], transfer_bytes? }
```
* **Compile phase** states what the policy will choose for the compile-time-known tier, and eligibility facts. **Run phase** logs the executor actually used (tier is a host fact) and every transfer, in the same stream.
* **`@expect(gpu)` follows the `@expect(simd)` split.** It asserts only `assertable yes` facts: a WGSL variant exists and the `kernel` profile holds (`OXS-P0302` fails the build with reason and fix). *Where it actually ran* is `assertable no` (target/host facts) and goes only to the run-phase log; `@expect(gpu, on: "webgpu")` scoping works as for simd. `@expect(no_transfer)` asserts that the function body contains no `upload`/`download` call.
* Proposed codes (**numbers checked against `codes.txt`: only `P0101`, `P0102`, `P0201`, `P0301` exist; `P0103`-`P0112` are proposed in 3.3; `P0302`-`P0306` are free; `codes.txt` is not edited by this report**):

| Code | `assertable` | Title |
|---|---|---|
| `OXS-P0302` | yes | No GPU variant: `{reason}` (`f64` use, `filter`, trapping op, unproven index, non-inlinable function, managed type) |
| `OXS-P0303` | no | Placement: `{executor}` ran / will run `{loc}` (positive fact, compile or run phase) |
| `OXS-P0304` | no | GPU executor not used at run time: `{no compute tier / profile / device lost}` |
| `OXS-P0305` | no | Large host loop or tiny resident op: consider the other residency (size hint, 4.6.4) |
| `OXS-P0306` | no | Transfer `{upload / download}` of `{bytes}` at `{loc}` (run phase; the count of these equals the source's transfer calls) |

Existing `OXS-P0301` (host transfer per call) keeps its meaning for host-capability traffic.

---

## 5. Shared Device Integration with `oxiterm-gpu` (Stage 5.2)

Per `docs/oxi-ui.md` ("GPU after OxiScript: one device, several users"), **one `Arc<GpuContext>`** must serve three co-users without CPU round-trips:
1. **`GpuSink` (Stage 5.2+):** DisplayList 2D renderer (SDF fills/borders, glyph atlas, scissor clips).
2. **`OxiScript` GPU kernels (Stages 8.3d-8.3e; was "8.3b") [Revised 2026-11-26 for decision 4 transparency]:** WGSL variants chosen by the executor policy (4.6), `host:gpu@1` compute pipelines for experts, storage buffers, and procedural textures.
3. **3D rendering (Stage 9.3):** Mesh/camera passes and kernel shaders.

### 5.1 Current State of `oxiterm-gpu` (Stage 5.2, Verified in Source)

Reading `oxiterm-gpu/src/context.rs:99-220` and `wgpu-types-30.0.2/src/limits.rs:644-667` **[V]** shows:
- `GpuContextOptions::default()` requests `Features::empty()` and `Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())`.
- Under `downlevel_webgl2_defaults()`, **all compute and storage limits are zero**: `max_storage_buffers_per_shader_stage = 0`, `max_storage_textures_per_shader_stage = 0`, `max_storage_buffer_binding_size = 0`, `max_compute_workgroup_storage_size = 0`, `max_compute_invocations_per_workgroup = 0`, `max_compute_workgroup_size_{x,y,z} = 0`, `max_compute_workgroups_per_dimension = 0` **[V]**.
- Stage 5.2 fixes (`docs/stages/5.2/fixes.md`) already added `GpuContextOptions`, `device.set_device_lost_callback`, `GpuContext::with_error_scopes` (`Internal`/`OutOfMemory`/`Validation` LIFO pop), and `Send` on `GpuSink` **[V]**.

### 5.2 Tiered Capability Negotiation (One Context, Graceful Degradation)

Instead of hard-coding either WebGL2-only or WebGPU-only limits, upgrade `GpuContext` in Stage 8.3c with a **negotiated `GpuCapabilities` profile** derived from `adapter.get_downlevel_capabilities()`, `adapter.features()`, and `adapter.limits()`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeTier {
    /// WebGL2 / GLES 3.0 / downlevel adapter without `DownlevelFlags::COMPUTE_SHADERS`.
    /// `GpuSink` UI rendering works fully; OxiScript kernels fall back to CPU SIMD/scalar.
    GraphicsOnly,
    /// Full WebGPU compute (`Limits::defaults()` clamped to adapter limits):
    /// >= 8 storage buffers/stage, >= 4 storage textures, >= 16 KiB workgroup shared mem,
    /// >= 256 invocations/workgroup, >= 65535 workgroups/dim.
    Compute,
}

pub struct GpuCapabilities {
    pub compute_tier: ComputeTier,
    pub shader_f16: bool,        // wgpu::Features::SHADER_F16
    pub subgroups: bool,         // wgpu::Features::SUBGROUP: native-only in wgpu 30 (features.rs:1130-1142); always false on the wgpu WebGPU backend
    pub timestamp_query: bool,   // wgpu::Features::TIMESTAMP_QUERY
    pub max_storage_buffer_binding_size: u32,
    pub max_compute_workgroup_storage_size: u32,
    pub max_compute_invocations_per_workgroup: u32,
}
```

**Negotiation algorithm inside `GpuContext::new_negotiated_async(opts)`:**
1. Request adapter (`Instance::request_adapter`, with software/lavapipe fallback as today).
2. Inspect `let downlevel = adapter.get_downlevel_capabilities();` and `let adapter_limits = adapter.limits();`.
3. Check `downlevel.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)` and `adapter_limits.max_compute_invocations_per_workgroup >= 64` and `adapter_limits.max_storage_buffers_per_shader_stage >= 4`.
   - **If yes (`ComputeTier::Compute`):** Build `required_limits = wgpu::Limits::downlevel_defaults().or_worse_values_from(&adapter_limits).using_resolution(adapter_limits)`, or request the larger `Limits::defaults()` the same way when every co-user wants them. **[Corrected 2026-11-26]** The draft used `Limits::defaults().using_resolution(adapter_limits)` and claimed that clamps storage/compute limits. It does not: `using_resolution` copies only `max_texture_dimension_{1d,2d,3d}` (`wgpu-types-30.0.2/src/limits.rs:753-760`), so a GLES 3.1 or mobile adapter with 4 storage buffers would make `request_device` fail. `or_worse_values_from` (`limits.rs:931`) is the clamping helper. `downlevel_defaults()` (`limits.rs:557-569`: 4 storage buffers per stage, 16 352 B workgroup memory, 256 invocations) is a safer floor than `defaults()` (8 storage buffers, 16 384 B). Optionally intersect `adapter.features()` with desired optional features (`SHADER_F16`, `SUBGROUP`, `TIMESTAMP_QUERY`) when `opts.enable_optional_compute_features` is true—never failing device creation if an optional feature is absent!
   - **If no (`ComputeTier::GraphicsOnly`):** Fall back cleanly to `wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter_limits)` with `Features::empty()`. `GpuSink` runs identically to Stage 5.2; `host:gpu` reports `caps() == 0` and routes OxiScript kernels to the CPU fallback.
4. Expose `ctx.capabilities() -> &GpuCapabilities` so `GpuSink`, `host:gpu`, and 3D (9.3) all query the exact active tier.

### 5.3 Resource Ownership, Lifetimes & Per-Instance Quotas

* **Owner hierarchy:** `GpuContext` (`Arc<GpuContext>`) owns only the `Instance`, `Adapter`, `Device`, `Queue`, and `GpuCapabilities`. Each subsystem owns its own resources:
  - `GpuSink` owns UI pipelines, glyph atlas, and instance buffers.
  - Each OxiScript runtime instance gets a scoped **`GpuArena`** inside the `host:gpu` capability provider, holding generational `HandleTable`s (reusing `oxiscript-runtime`'s `HandleTable` pattern **[V]**) for `Buffer`, `Texture`, `Kernel` (`ComputePipeline` + `BindGroupLayout`), and `Fence`.
* **Deterministic cleanup on trap / hot reload / drop:** When an OxiScript instance traps, reloads (instance swap, `abi.md` §4), or drops, `GpuArena::drop()` explicitly calls `.destroy()` on every live `wgpu::Buffer` and `wgpu::Texture` owned by that script and cancels pending readback callbacks—mirroring Stage 5.2 fix B-05 (`buf.destroy()`) so a leaking or reloaded script never leaks VRAM.
* **Memory & object quotas (BAD ACTOR hardening):** Every `GpuArena` enforces configurable hard caps (defaults for untrusted/sandboxed scripts: `max_vram_bytes = 128 MiB`, `max_buffers = 256`, `max_textures = 64`, `max_kernels = 64`, `max_pending_readbacks = 8`, `max_texture_dim = 4096`). Exceeding a quota returns `STATUS_QUOTA_EXCEEDED` (`Result::Err`) without ever calling `device.create_buffer` (preventing host OOM and `wgpu` validation errors).

### 5.4 Queue Submission Ordering & Error-Scope Isolation (Fixing Stage 5.2 B-03 Permanently)

**Queue ordering:** `wgpu::Queue` executes submitted `CommandBuffer`s in strict FIFO submission order ([WebGPU Spec](https://www.w3.org/TR/webgpu/#queues)) **[S]**. Within a single OxiUI frame tick:
1. Script event/tick runs → kernel dispatches record into a per-turn `CommandEncoder` (or batch) and submit to `ctx.queue()` **before** `GpuSink::render_into_view`.
2. `wgpu` automatically inserts Vulkan/DX12/Metal pipeline barriers between a compute pass writing a storage texture/buffer and a subsequent render pass sampling that texture (`TEXTURE_BINDING`)—no manual barrier API needed from OxiScript!
3. If `GpuSink` and script kernels ever run on different threads, serialize queue submissions around frame boundaries (or keep both on the render/session thread in local mode and browser single-threaded wasm32) so texture writes complete before UI sampling.

**Error-scope isolation (analyzing Stage 5.2 BAD ACTOR B-03):**
Direct inspection of `wgpu-30.0.2/src/backend/wgpu_core.rs:1840-1850` **[V]** confirms why B-03 occurred:
- `device.push_error_scope(...)` pushes onto a **thread-local stack** keyed by `ThreadId::current()`. A co-user on the same thread that pushes a scope and forgets to pop it swallows errors from subsequent operations on that thread, unless every subsystem pushes and pops its own inner scopes around every `wgpu` call.
- `device.on_uncaptured_error(...)` is **device-wide** (one global callback in `error_sink.uncaptured_handler`). If any co-user replaces it, it hijacks uncaptured errors for the whole process!

**Isolation design for `GpuContext` (Stage 8.3c):**
**[Corrected 2026-11-26]** The current state, checked in the code:
* The sink side of B-03 is already fixed. `with_error_scopes` pushes Internal/OOM/Validation scopes around every sink operation (`docs/stages/5.2/fixes.md`, `context.rs:235-260`), so a foreign *unpopped* scope pushed earlier on the same thread can no longer swallow the sink's errors.
* In wgpu 30, `push_error_scope` already returns an `ErrorScopeGuard` that **pops on `Drop`** and is `!Send`/`!Sync` (`wgpu-30.0.2/src/api/device.rs:957-985`). RAII is built in; only a deliberately leaked guard (`mem::forget`) or a guard held across an `.await` can still leak a scope.
* **The remaining hole:** `GpuContext::device()` is `pub` (`context.rs:295`), and `on_uncaptured_error` *replaces* a single device-wide handler (`wgpu_core.rs:1840-1843`). Any Rust co-user (3D, the `host:gpu` provider) can hijack it, and no documentation rule prevents that.

1. **Promote `with_error_scopes` to `pub`** (currently `pub(crate)`, `context.rs:238` **[V]**) and add an async twin `with_error_scopes_async` for browser wasm32, where `pollster::block_on` cannot block the JS event loop.
2. **Do not hand out `&wgpu::Device` to co-users.** Replace the public `device()`/`queue()` with scoped entry points (`ctx.run_scoped(|dev, queue| ...)`, which always runs inside `with_error_scopes`). Keep raw access `pub(crate)` or behind an explicitly named `unsafe_raw_device()` escape hatch, so that no co-user can call `on_uncaptured_error`. This is an API change in `oxiterm-gpu`, so slice 8.3c owns it.
3. **One handler, owned by `GpuContext`:** install it at device creation. It counts errors in an atomic `uncaptured_error_count` and sets `device_lost` if the device is lost. A BAD ACTOR probe must try to replace it through every public API.
4. **Never expose raw `wgpu::Device` to scripts:** `host:gpu` pre-validates every handle, buffer size, alignment (`COPY_BUFFER_ALIGNMENT = 4`, uniform 16-byte alignment), usage flag, and aliasing rule in Rust *before* calling `wgpu`, and wraps every `wgpu` call (`create_shader_module`, `create_compute_pipeline`, `create_buffer`, `submit`) inside `ctx.with_error_scopes`. A malformed shader or OOM inside one script returns `Err(status)` to that script while leaving `GpuSink` and other scripts completely untouched.

### 5.5 Budgeting, Runaway Kernels, Timeouts & Device-Lost Recovery

Unlike CPU execution (`wasmi` fuel metering), **GPUs have no hardware instruction-level fuel counter**. A runaway compute shader (infinite loop or 10⁹ workgroups) triggers OS GPU watchdogs:
- **Windows WDDM TDR:** Resets the GPU driver after **2 seconds** by default (`TdrDelay = 2`), causing `DXGI_ERROR_DEVICE_HUNG` → `DeviceLost` ([Microsoft TDR docs](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery--tdr-)) **[S]**.
- **macOS Metal / Linux DRM/Vulkan (`amdgpu`/`i915`/`nouveau`):** Kernel GPU hangcheck similarly kills the context after some seconds (the "~2–10 s" range is **[U]**: no source), poisoning the `wgpu::Device` for **all co-users** (including the UI renderer!).

**Five-layer defence against runaway kernels:**
1. **Static loop-bound guard in `oxiscript-wgsl` (mandatory for untrusted/sandboxed kernels):** Every `loop` emitted by `oxiscript-wgsl` gets a compiler-injected iteration counter (`var _fuel: u32 = MAX_ITERS; if (_fuel == 0u) { break; } _fuel -= 1u;`), defaulting to e.g. `65_536` iterations per invocation (configurable via `@max_iters(N)` up to a host cap). This makes infinite loops **structurally impossible** in compiler-generated WGSL!
2. **Dispatch invocation cap:** `host:gpu::dispatch` caps total invocations per single submission (`wg_x * wg_y * wg_z * wg_size <= max_invocations_per_dispatch`, e.g. 4M invocations default) and caps dispatches per event turn (`max_dispatches_per_turn = 32`). Larger jobs must chunk across turns/frames.
3. **Per-frame GPU time accounting (when `TIMESTAMP_QUERY` is available):** Attach `ComputePassTimestampWrites` to script compute passes; measure elapsed GPU nanoseconds asynchronously on completion. If a script's rolling GPU time exceeds its per-frame budget (e.g. **4 ms / 16.7 ms frame** so the UI stays at 60 fps), throttle or reject further dispatches from that script with `STATUS_GPU_BUDGET_EXCEEDED` until the next frame window.
4. **Wall-clock watchdog on blocking readbacks (native):** Never pass `timeout: None` in `device.poll(PollType::Wait { .. })` for script-triggered waits; use bounded timeouts (e.g., 500 ms) so a stalled queue never wedges the host thread.
5. **Device-loss recovery (one device shared with UI):** If `device_lost` fires anyway (driver crash, sleep/resume, GPU reset):
   - `GpuContext::is_device_lost()` immediately flips to `true` (already implemented via `OnceLock` in `context.rs:213-225` **[V]**).
   - Every active `GpuArena` marks all handles invalid (`STATUS_DEVICE_LOST`) and poisons/notifies the script (consistent with `abi.md` §3.4).
   - The host shell (`oxiterm-gui` / browser local core) reconstructs a fresh `Arc<GpuContext>`, rebuilds `GpuSink` (atlas + pipelines rebuild cleanly from the bundled fonts), and either restarts the script instance or degrades `host:gpu` to CPU fallback if device loss repeats twice within 60 s (circuit breaker against crash loops!).

### 5.6 Zero-Copy Kernel-to-UI Texture Handoff

`oxiterm-gpu` renders in **`TextureFormat::Rgba8Unorm`** (`oxiterm-gpu/src/sink.rs:30` **[V]**), which is also a mandatory WebGPU storage-texture format (`texture_storage_2d<rgba8unorm, write>`) supported on every compute-capable Vulkan/Metal/DX12/WebGPU adapter!
- **Flow (zero CPU copies):**
  1. Script allocates `tex = gpu.create_texture_rgba8(w, h, STORAGE_BINDING | TEXTURE_BINDING)` → gets generational `TextureHandle`.
  2. Script dispatches a pixel kernel writing directly to `textureStore(out_tex, coord, rgba)`.
  3. Script binds `tex` to an OxiUI surface element (new `DisplayItem::Texture { node, rect, handle: TextureHandle }` or `KernelSurface` in Stage 5.6/8.3d).
  4. `GpuSink` adds a tiny 3rd pipeline (`image_pipeline`, reusing the exact same textured-quad vertex shader as `glyph.wgsl` but sampling RGBA8 instead of R8!) and looks up `&wgpu::TextureView` from the shared `GpuTextureRegistry` on the same `Arc<GpuContext>`.
  5. On `ComputeTier::GraphicsOnly` (WebGL2/GLES3) or `oxiterm-fb` (software sink), the CPU fallback kernel writes into an RGBA8 byte buffer and uploads via `queue.write_texture` (or blits directly in `oxiterm-fb`), keeping the exact same visual result!

### 5.7 Browser (WebGPU) vs Native Local Mode Differences

| Concern | Native Local Mode (`oxiterm-gui` / CLI) | Browser Local Mode (`wasm32-unknown-unknown`) |
|---|---|---|
| **`wgpu` backend** | Vulkan / Metal / DX12 (GLES3 fallback; lavapipe in CI) | WebGPU (primary when `navigator.gpu` available); WebGL2 fallback (`ComputeTier::GraphicsOnly`) |
| **Async / blocking** | `pollster::block_on` works for headless sync paths; async preferred in window loop | **Cannot block main thread!** `new_negotiated_async`, `pop_error_scope().await`, and `map_async` must all be `async`/callback-driven on the JS microtask queue (`wasm-bindgen-futures`). |
| **Shader compilation** | `wgpu` runs Naga `wgsl-in` → SPIR-V/MSL/HLSL | `wgpu` passes WGSL string straight to `GPUDevice.createShaderModule` (browser Tint/Naga compiles it) **[V]**. |
| **Error scopes** | Thread-local in `wgpu-core` **[V]** | Single-threaded JS event loop; still **must** push/pop symmetrically around every synchronous WebGPU call (do not hold an error scope open across an `.await` point where another subsystem could interleave!). |
| **Bundle** | n/a | WebGPU-only build: no naga (`ShaderSource::Wgsl` passes through). A build *with* the WebGL2 fallback needs the `webgl` feature, which pulls in `wgpu-core` + `wgpu-hal` + naga (`wgpu-30.0.2/Cargo.toml`) **[V, added 2026-11-26]**. Measure both. |
| **WebGPU availability** | Broad on modern desktop/Pi4-5/Android | Chrome 113+ (Win/Mac/ChromeOS), Android 12+ (121), Linux Intel Gen12+ (144) and NVIDIA on Wayland (147); Safari 26; Firefox Windows 141, macOS Apple Silicon 145/147, Intel Mac and Linux Nightly only, Android behind a flag ([gpuweb status](https://github.com/gpuweb/gpuweb/wiki/Implementation-Status), `vulkan-acceleration.md` §10 **[V]**) → **CPU SIMD fallback is mandatory** for universal web reach! |

---

## 6. AI on Top: Minimal Kernels & Data Types for In-Browser Training and Inference

The owner's course goal (`docs/oxi-ui.md` Level 5 "Hero") is **"first AI in OxiScript": perceptron → live-trained digit recogniser (MNIST) → tiny language model**, running directly in the browser (and natively) without pulling ML domain primitives into the language core. Everything below falls out of the exact `#kernel` + `Simd<T, N>` + `host:gpu@1` layers designed in §§2–5.

### 6.1 Prior Art in Browser & Rust WebGPU AI

| Project | Stack & Licence | Key Lessons for OxiScript |
|---|---|---|
| **WebLLM / MLC-AI** ([github.com/mlc-ai/web-llm](https://github.com/mlc-ai/web-llm)) **[S]** | Apache-2.0; TVM-compiled WGSL compute shaders + WebGPU | Proves 0.5B–3B parameter LLMs run interactively in browser tabs using 4-bit/8-bit packed `u32` weights, `shader-f16`, KV-cache buffers, and subgroup reductions; caches compiled pipelines to hide shader compile latency. |
| **Burn + CubeCL** ([github.com/tracel-ai/burn](https://github.com/tracel-ai/burn)) **[S]** | **MIT OR Apache-2.0**; Rust + `wgpu`/WGSL/SPIR-V | Demonstrates both **training** (reverse-mode autodiff in pure library code on top of tiled matmul/conv/reduction kernels) and **inference** in the browser over `wgpu`. |
| **Candle** ([github.com/huggingface/candle](https://github.com/huggingface/candle)) **[S]** | **MIT OR Apache-2.0**; Rust + Wasm SIMD128 (+ WebGPU efforts) | Clean minimal tensor library proving small models (MNIST, Whisper tiny, SmolLM) are practical in Wasm SIMD128 alone when WebGPU is unavailable. |
| **ONNX Runtime Web** ([onnxruntime.ai](https://onnxruntime.ai/docs/tutorials/web/)) **[S]** | **MIT**; WebGPU EP + Wasm SIMD128 (XNNPACK) fallback | Expected lesson **[P]**: WebGPU wins on large matmul/transformers, while Wasm SIMD wins on tiny models and batches because of dispatch and readback latency. **[Corrected 2026-11-26: the cited ORT Web tutorial contains no performance numbers, so the "5×–10×" figure is unsourced.]** The measured basis for the crossover is per-dispatch API cost: 24–36 µs (Vulkan) and 32–71 µs (Metal) natively, Chrome 33–67 µs, Safari 32 µs, **Firefox ~1 ms** ([arXiv:2604.02344](https://arxiv.org/html/2604.02344v1)) **[V]**. OxiScript's dual CPU-SIMD + WebGPU fallback matches this automatically. |
| **Transformers.js** ([huggingface.co/docs/transformers.js](https://huggingface.co/docs/transformers.js)) **[S]** | Apache-2.0; ONNX Runtime Web | Shows progressive feature detection (`webgpu` + `f16` → `webgpu` `f32` → `wasm-simd` → `wasm-scalar`). |

### 6.2 Minimal Kernel Primitives Needed (Only 7 Kernel Families)

**[Revised 2026-11-26 for decision 4 transparency]** `std/tensor` is written once as layer-0 combinators and `kernel` functions over `Resident<f32[]>`; the executor policy (4.6.4) runs it on GPU or host memory, so the AI demo needs `host:gpu` only for expert pieces (workgroup tiling tuned by hand). Everything below stays valid as the kernel inventory.

Every course milestone (perceptron → MNIST MLP/CNN live training → tiny GPT-style LM with 100k–10M params) decomposes into **seven small `#kernel` primitives**, all expressible in Stage 8.3d OxiScript without language changes:

| # | Kernel Family | Required Language / WGSL Features | Used By |
|---|---|---|---|
| **1** | **Elementwise & fused activations** (`add`, `mul`, `relu`, `gelu`, `silu`, `add_bias`, residual add, plus backward `d_relu` / `d_gelu`) | 1D `BufferView<f32>`, `globalId().x`, bounds check | Perceptron, MLP, CNN, Tiny LM |
| **2** | **Tiled Matrix Multiply (`GEMM` / `GEMV`)** (`C = A @ B`, `A^T @ B`, `A @ B^T`) | 2D dispatch, **workgroup shared memory** (`@shared let tileA: FixedArray<f32, 256>` → WGSL `var<workgroup> tileA: array<f32, 256>`), **`kernel.barrier()`** (`workgroupBarrier()`), `Simd<f32, 4>` vectorised loads (`vec4<f32>`) | MLP forward & backprop, Attention projections, LM linear layers |
| **3** | **2D Convolution (`conv2d`)** | Either **im2col + GEMM** (zero extra complex kernel!) or direct 2D stencil kernel with shared-memory halo tile | Digit recogniser (CNN), image filter benchmark |
| **4** | **Row-wise Reductions (`softmax`, `layernorm`, `rmsnorm`, `cross_entropy_loss`)** | Workgroup shared-memory tree reduction (`workgroupBarrier()`), optional **`kernel.subgroupAdd(x)`** when `GpuCapabilities::subgroups` is true (native only through wgpu 30; never in the browser path) | Digit recogniser loss, Transformer attention & normalization |
| **5** | **Scaled Dot-Product Attention** (`softmax(Q @ K^T / sqrt(d)) @ V`) | Composed from kernels #2 + #4 (causal mask in elementwise step); single-pass FlashAttention-style tiled kernel optional later | Tiny LM (Hero level) |
| **6** | **Optimizer step (`sgd_step`, `adamw_step`)** | In-place elementwise update on `inout BufferView<f32>` (`w`, `m`, `v`) with `grad` + scalar hyperparams (`lr`, `beta1`, `beta2`) | Live browser training of digit recogniser & tiny LM! |
| **7** | **Packed Dequantization + MatVec** (`u32`-packed `i8`/`u4` weights × `f32`/`f16` activations) | Bitwise shifts/masks on `u32` (`extractBits` / `(packed >> shift) & 0xFF`), `Simd<i32, 4>` / `dot4I8Packed` (optional) | Quantised inference in browser & ESP32-S3 (`esp-nn` parity) |

**Critical design requirement uncovered here:** Stage 8.3d **must** include two `#kernel`-profile intrinsics from day one—**workgroup shared memory** (`@shared` fixed-size array inside a `kernel function`) and **`kernel.barrier()`** (`workgroupBarrier()`)—because naive global-memory GEMM is much slower than tiled shared-memory GEMM (the "10×–30×" factor is **[U]**: no source given). Both map 1:1 to standard WebGPU/WGSL core (no extensions required!).

### 6.3 Data Types Matrix for AI (`f32`, `f16`, `bf16`, `i8`/`u8`)

| Type | CPU Wasm SIMD128 | WebGPU / WGSL | OxiScript Design Decision |
|---|---|---|---|
| **`f32`** | Native (`f32x4`) **[V]** | Mandatory core **[V]** | **Universal baseline** for training (weights, gradients, optimizer moments) and default inference. Always works without extensions. |
| **`f16`** | Emulated via `f32` conversion (no Wasm `f16x8` in fixed-width SIMD128) | Optional **`shader-f16`** (`enable f16;`), in Chrome ≥ 120 where the adapter supports it ([Chrome blog](https://developer.chrome.com/blog/new-in-webgpu-120)) **[V]**; a web+native feature in wgpu 30 (`features.rs:1690-1700`) **[V]**; naga 30 parses `enable f16;` **[V]**; Safari 26 and per-driver coverage **[U]** | **Include as an explicit storage/compute scalar `f16` (or `half`) in Stage 8.3e**, guarded by `gpu.caps() & CAP_F16` (CPU fallback converts via `f32` using IEEE-754 half conversion, e.g., `half` crate algorithm). Cuts VRAM & bandwidth 2× for tiny LM inference. |
| **`bf16`** | No native Wasm support | **NOT in WGSL** (only provisionally reserved; no browser implements native `bf16` **[S]**) | **Do NOT add as a core language primitive.** If loading `bf16` weights from external checkpoints, provide a 3-line stdlib `u16 ↔ f32` bitshift unpacker (`f32::from_bits(u32(b) << 16)`) inside `#kernel`! |
| **`i8` / `u8` + `i32` accum** | Native (`i8x16`, `i16x8`, `i32x4.dot_i16x8_s`) **[V]** | No native `array<i8>` storage in WGSL **[V]**; pack 4×`i8` per `u32` and unpack via shifts (or WGSL `unpack4xI8` / `dot4I8Packed`, part of the WGSL spec (§17.5.22) and parsed by naga 30 as language extension `packed_4x8_integer_dot_product` **[V]**) | **Support in `std/tensor` via packed `Buffer<u32>`** (`pack4xI8` / `unpack4xI8` helpers in `std/kernel`). Essential for 1–3B tiny LMs in browser VRAM and matches ESP32-S3 PIE `int8` kernels (`esp-nn`). |

For production models larger than the course demos, `docs/oxi-ui.md` §5 already leaves room for an optional `host:ml@1` capability delegating whole ONNX/SafeTensors graphs to `burn`/`candle`/`tract` (native) or `esp-nn` (ESP32-S3), with the placement report stating whether a layer ran as an OxiScript kernel or a host ML backend.

---

## 7. Concrete Staged Delivery Plan (8.3a–8.3e) & Permissive Reuse

All stages follow the repository's process rule (`docs/oxi-ui.md` §5): **measurable gate + BAD ACTOR pass + regression tests before commit**, keeping `oxiscript-*` crates free of `oxiterm-*` dependencies until the integration slice.

### 7.1 Prerequisites & Dependency Map

| Dependency | Current Repo Status | Why Needed |
|---|---|---|
| **8.L Lowering** (`oxiscript-lower`) | **Done** (`c792b4f`) **[V]** | End-to-end `.oxs` → OxiIR → Wasm pipeline exists and passes differential tests. |
| **8.M Memory & runtime linking** (`oxiscript-rc` + `walrus` link) | **8.M crate done (`5a5929d`)**; full runtime linking with `oxiscript-lower` (`RuntimeMode::Import`) needed for non-stub floats (`oxi_str_from_f64`) and real heap benchmarks **[V]**. | Needed before realistic memory-heavy SIMD/GPU benchmarks (8.3a gate can start on `Stub` for numeric kernels, but full benchmarks need 8.M linked). |
| **8.S4 Placement report v0** (`placement.json` schema + remarks API) | Planned (Wave 5) | 8.3b plugs SIMD remarks (`OXS-P0103..P0112`) and `@expect(simd)` directly into the 8.S4 remarks API. (If 8.3a starts before 8.S4, 8.3a needs no placement report yet; 8.3b either depends on 8.S4 or defines the minimal `Remark` struct in `oxiscript-abi` shared with 8.S4.) |
| **5.2 Headless `GpuSink`** (`oxiterm-gpu`) | **Done** (pending golden blessing; B-01..B-10 fixed) **[V]** | Direct foundation for 8.3c (`GpuContext` upgrade) and 8.3d (`host:gpu@1` + zero-copy texture). |
| **Inline value structs in arrays** (`contracts.md` §3 worked example `Array<Particle>`) | Not yet implemented (MVP structs are heap-allocated COW objects) | Required for zero-copy `Array<Particle>` GPU uploads and SoA/AoS benchmarks; can land as an opt-in `@inline_elements` / scalar-only struct-array lowering in **8.3a** or **8.3d**. |
| **8.2 targets (`oxi build --target device/web/server`) and manifest feature lines** [Revised 2026-11-26 for decision 4 transparency] | Planned (8.2) | The executor policy and variant list (4.6.6) are per target; 8.3a can start with a single `web`-like test target and the manifest `features simd128` line. |

### 7.2 Staged Delivery Plan Table

| Stage | Scope | Measurable Gate | Key Risks & Mitigations | Depends On |
|---|---|---|---|---|
| **8.3a: Explicit SIMD + Wasm SIMD128 + Scalar Fallback** | Add `Simd<T, N>` & `SimdMask<T, N>` builtin value types to `oxiscript-check`; add `Ty::Vec`/`Ty::VecMask` + vector ops to `oxiscript-ir` + `verify`; add `lower_simd_to_scalar` pass; emit Wasm `v128` (`ABI 1.1`, `features simd128`) in `oxiscript-wasm` (`wasm-encoder` 0.263 already supports SIMD128 **[V]**); enable `wasmi/simd` in `oxiscript-runtime` with `wasm_simd(true)`, `wasm_relaxed_simd(false)` (mandatory: a test must show a module using `f32x4.relaxed_madd` is rejected); for the native release, set Wasmtime `relaxed_simd_deterministic` + `cranelift_nan_canonicalization`; add `FieldKind::Inline` vector layout support. | (1) All 10 `Simd<T, N>` types compile to both `simd128` and `scalar` Wasm and pass `wasmparser` validation. (2) Differential fuzz test (≥ 5,000 generated vector expressions incl. NaN, `-0.0`, `Inf`, min/max, overflow traps, bitcast) produces **bit-identical results** across `wasmi` SIMD128, `wasmi` scalar, and Wasmtime SIMD128. (3) Benchmark suite (§8) shows **≥ 2.5×** speedup for `f32x4` matmul/SAXPY in Wasmtime/browser vs scalar, and zero regression on non-SIMD benchmarks in `wasmi 2.0.3`. (4) BAD ACTOR pass (out-of-bounds `Simd.load`, misaligned/overflowing indices, NaN payload leak via `bitcast`, fuel exhaustion). | *Risk:* Wasm local count limit (`LIMIT_LOCALS = 12_000`) hit faster when `lower_simd_to_scalar` expands 1 `v128` into 16 `i8` scalars. *Mitigation:* add basic Wasm local reuse (liveness-based free list) in `oxiscript-wasm` or keep scalar-fallback loops compact. | 8.L (done), Benchmarks harness (§8) |
| **8.3b: Auto-Vectoriser v0 + Layer-0 Combinators on CPU + Placement Report + `@expect(simd)`** [Revised 2026-11-26 for decision 4 transparency] | Enable `parallel for` syntax (un-reserve `For*::parallel` in `oxiscript-syntax`); implement natural-loop + dependence/alias analyser on OxiIR SSA for shapes **S1–S4** (§3.2); emit `OXS-P0103..P0112` remarks into `placement.json`; enforce `@expect(simd)` on `assertable yes` codes (compile error with source span + fix); emit positive `OXS-P0110` for LSP inlays. [Revised 2026-11-26 for decision 4 transparency] Also: layer-0 combinators (`map`/`zip`/`reduce`/`reduceSum`, `parallel for` over arrays) lowered to counted loops that feed the vectoriser (CPU executors only here); the shared `Remark` struct gains `phase` (compile/run), `executor` (scalar/simd128/gpu/threads) and `inputs` fields so 8.3d needs no format change. | (1) Corpus of ≥ 25 loops (maps, zips, reductions, branchless `?:`, alias-guarded loops, plus negative tests: loop-carried deps, calls, allocs, trapping int `+`, non-associative float sums) matches expected `OXS-P01xx` codes 100% identically in debug and release. (2) `@expect(simd)` fails compilation with exact code on every negative case and passes on every S1–S4 case. (3) Auto-vectorised SAXPY, image filter, and SoA particle step achieve ≥ 80% of hand-written `Simd<f32, 4>` throughput (Wasmtime/browser). (4) Differential fuzzing verifies bit-identical outputs vs unvectorised loop. (5) BAD ACTOR pass (aliasing arrays passed to same loop, integer overflow on `n`, empty/1-3 element tails, NaN in branchless `?:`). [Revised 2026-11-26 for decision 4 transparency] (6) The report states, for every layer-0 loop, the executor it will use and why (100% of the corpus has an `OXS-P0303`/`P0110`/`P0111` remark). | *Risk:* False aliasing or subtle trap reordering breaks sequential semantics. *Mitigation:* Conservative v0—never vectorise across any `Op::may_trap()` inside the body; require explicit runtime `a !== b` pointer guard whenever arrays aren't provably distinct; verify every case differentially. | 8.3a, 8.S4 (or shared `Remark` type) |
| **8.3c: `GpuContext` Compute Upgrade & Isolation (`oxiterm-gpu`)** | Add `ComputeTier` (`GraphicsOnly` vs `Compute`), `GpuCapabilities`, and `GpuContext::new_negotiated[_async]` to `oxiterm-gpu`, with limits clamped by `or_worse_values_from` (not `using_resolution`); expose `pub with_error_scopes[_async]` / `run_scoped`, **stop exposing raw `device()` to co-users** (so the uncaptured handler cannot be replaced), and add a context-owned uncaptured-error counter; add shared `GpuTextureRegistry` + `image_pipeline` (`Rgba8Unorm` textured quad) for zero-copy texture display; verify `wasm32` build (`WebGPU` + `WebGL2`). | (1) On Vulkan/lavapipe CI, `new_negotiated` negotiates `ComputeTier::Compute` and runs a smoke compute shader writing into an `Rgba8Unorm` storage texture that `GpuSink` draws with **zero CPU readback**, matching a PNG golden. (2) Forced `GraphicsOnly` mode reproduces all Stage 5.2 PNG goldens unchanged. (3) BAD ACTOR pass (co-user leaking error scopes, including `mem::forget` of a guard; a co-user trying to replace the uncaptured handler through every public API; concurrent threads; OOM buffer creation; device destruction mid-compute; recovery via context recreation) passes with 0 panics and 0 leaked errors across subsystems. (4) Negotiation test with an adapter whose limits are below `Limits::defaults()` (e.g. forced GL backend): the device is created. | *Risk:* Browser `wasm32` `async` error scopes cannot use `pollster::block_on`. *Mitigation:* Keep `with_error_scopes_async` native-first and use `pollster::block_on` only in native sync wrappers. | 5.2 (done) |
| **8.3d: One Source to CPU and GPU** [Revised 2026-11-26 for decision 4 transparency] | `kernel` profile + layer-0 combinators lowered to WGSL + residency types + runtime executor choice + report. Un-reserve `kernel function` & `@workgroup(...)` / `@shared` / `@max_iters(...)`; enforce the `#kernel` profile in `oxiscript-check` and `oxiscript-ir::verify` (`OXS-E0500`, `std430` layout checks, uniformity check for `kernel.barrier()`), and infer the same eligibility for layer-0 bodies (`OXS-P0302` with reason); build `oxiscript-wgsl` (OxiIR to WGSL text, mandatory loop-fuel counter, bounds guards); embed `oxi.kernels` (WGSL + CPU version of every GPU-eligible loop) in the `.wasm`; add `Resident<T>` (`std/resident`: `upload`/`download`, mixed-residency error) and `GpuArena` quotas and error-scope isolation for the executor path; implement the deterministic executor policy (4.6.4) and the manifest `variants wgsl` line; `@expect(gpu)`, `@expect(no_transfer)`, run-phase remarks (`OXS-P0302`-`P0306`); inline struct arrays (Q2) for `Resident<Particle[]>`. | (1) Every emitted WGSL variant validates with `naga 30.0.2` offline in CI (0 errors). (2) **Three-executor differential test:** the same program (particle step 100k, 2D Gaussian blur, tiled 256x256 `f32` GEMM, integer `reduce`) run on scalar, SIMD128 and GPU gives identical integer results and `f32` results within the 4.5 contract (`1e-5` rel); scalar and SIMD128 float results bit-identical. (3) **Policy determinism:** a property test (>= 10,000 generated residency/tier/profile combinations) shows the same inputs always give the same executor, and an exhaustive table test covers every row of 4.6.4. (4) **No hidden transfers:** for every test program, the transfers logged at run time (`OXS-P0306`) equal the `upload`/`download` calls executed in the source, on a GPU host and on a host forced to `GraphicsOnly`; `@expect(no_transfer)` rejects a function containing one. (5) `@expect(gpu)` fails the build with the exact `P0302` reason on every negative case (`f64`, `filter`, trapping `+`, unproven index, non-inlinable closure) and passes on every eligible case. (6) Mixed `Resident`/host input and `Resident` input without a WGSL variant are compile errors with the fix. (7) BAD ACTOR pass (infinite-loop kernel stopped by the static `_fuel` counter in < 50 ms without TDR; out-of-range index attempts never reach an eligible variant; aliasing read+write rejected; VRAM quota exceeded returns `Err` from `upload`; dispatch cap traps; hot reload frees all GPU buffers with 0 VRAM leak; device loss poisons handles without hanging the UI). | *Risk:* WGSL uniformity rejections confuse users; `std430` padding; users hit `f64` literal defaults and see "CPU only" surprises; eligibility too narrow to be useful without a trap channel. *Mitigation:* uniformity and `std430` checked in `oxiscript-check` with source diagnostics; `P0302` names the exact reason and fix (`2.0f32`, `+%`); measure how much real code is eligible (Q16) before widening beyond the 4.6.2 proof scope; `naga` validation in CI. | 8.3a, 8.3b, 8.3c, 8.M (linking) |
| **8.3e: Low-Level `host:gpu`, `std/tensor` + AI Demo (Course Level 5)** [Revised 2026-11-26 for decision 4 transparency] | Implement the low-level `host:gpu@1` provider for libraries and experts (IDL of 4.4: `GpuArena` handles, async `OnReadbackReady` + out-buffer chunking, dispatch caps, `caps()`, only module-verified kernels, no guest WGSL), built on the 8.3d arena and validation. Add optional `f16` storage/compute support (`CAP_F16` / `enable f16;`), optional `subgroupAdd` fast path (`CAP_SUBGROUPS`, native only through wgpu 30), and `pack4xI8`/`unpack4xI8` helpers; write `std/tensor` entirely in OxiScript (layer-0 combinators and `kernel` functions over `Resident<f32[]>`; the executor policy runs it, `host:gpu` only for hand-tuned pieces) (7 kernel families of §6.2: elementwise/activations, tiled GEMM, im2col/conv2d, softmax/layernorm, scaled dot-product attention, SGD/AdamW backward pass, packed int8 matvec); build browser + native demos for MNIST digit recogniser (live training) and tiny char-level LM. | (1) MNIST 784→128→10 MLP trains live in browser (WebGPU & Wasm SIMD128 fallback) to ≥ 95% test accuracy in < 10 s (WebGPU) / < 60 s (SIMD128) with differential loss check against CPU scalar. (2) Tiny LM (e.g., 100k–1M params, 2-layer Transformer) generates tokens interactively in browser at ≥ 20 tok/s (WebGPU) / ≥ 5 tok/s (Wasm SIMD128) **[P target]**. (3) `host:gpu` conformance: a hostile guest gets `Err` for guest WGSL text, foreign kernel index, aliasing and quota, with the corrected safety rules unchanged. (4) Zero ML concepts added to core grammar or compiler. (5) BAD ACTOR pass (NaN weights/gradients, huge tensor dims overflowing `u32` byte size, zero-length batch). | *Risk:* Shader compile cold-start latency on first browser load; float precision drift in `f16` training. *Mitigation:* Keep training in `f32`, use `f16`/`i8` for inference only; cache compiled `ComputePipeline`s per kernel hash in `GpuArena`. | 8.3d |

**[Revised 2026-11-26 for decision 4 transparency] Cross-stage gates.** Every slice that changes an executor keeps the same measurable rule: *one program gives identical integer results and in-contract float results on scalar, SIMD128 and GPU executors, and the executor policy is deterministic given inputs* (enforced for scalar/SIMD128 from 8.3b, fully in 8.3d). Stage mapping: 8.3a explicit SIMD + scalar fallback; 8.3b auto-vectoriser + layer-0 on CPU + placement report; 8.3c compute-capable `GpuContext` + isolation; 8.3d one source to CPU and GPU; 8.3e low-level `host:gpu` + `std/tensor` + AI demo.

### 7.3 Permissive-Only Libraries to Reuse (Zero GPL/LGPL)

| Need | Crate / Library | Licence (Verified) | Where Used | Notes |
|---|---|---|---|---|
| **Wasm SIMD128 emission & validation** | `wasm-encoder 0.263`, `wasmparser 0.263` | Apache-2.0 WITH LLVM-exc. OR Apache-2.0 OR MIT **[V, `Cargo.toml`]** | `oxiscript-wasm` (core) | Already in `Cargo.lock`; full `v128` opcode support verified **[V]**. |
| **Interpreted Wasm SIMD128** | `wasmi 2.0.3` (`features = ["simd", "deterministic"]`, plus `Config::wasm_relaxed_simd(false)`, which is mandatory) | MIT OR Apache-2.0 **[V, `Cargo.toml`]** | `oxiscript-runtime` | Already in `Cargo.lock`; zero overhead on non-SIMD code **[V: 2.0.0 CHANGELOG]**. |
| **GPU abstraction (Vulkan/Metal/DX12/WebGPU/GLES3)** | `wgpu 30.0.2`, `bytemuck 1.25.2`, `pollster 1.0.3` | MIT OR Apache-2.0 (bytemuck: Zlib/Apache-2.0/MIT) **[V]** | `oxiterm-gpu`, `host:gpu` | Already in `Cargo.lock`. |
| **Offline WGSL validation (CI / `oxi check`)** | `naga 30.0.2` (`features = ["wgsl-in"]`) | MIT OR Apache-2.0 **[V, `Cargo.toml`]** | `oxiscript-wgsl` (dev/toolchain only—NOT in browser wasm32!) | Already in `Cargo.lock`; validates emitted WGSL + uniformity without a GPU. |
| **Wasm-to-C SIMD128 AOT (32-bit/SBC)** | `w2c2` + `SIMDe` | `w2c2`: MIT **[V]**; `SIMDe`: MIT ([simde LICENSE](https://github.com/simd-everywhere/simde/blob/master/COPYING)) **[S]** | Toolchain (8.2/8.3a AOT) | Header-only C99; maps `v128` to NEON/SSE/AVX/RVV. |
| **IEEE `f16` host conversions (8.3e)** | `half 2.7.1` (corrected) | MIT OR Apache-2.0 **[V, pulled by `naga` in `Cargo.lock`]** | `oxiscript-runtime` / `std/tensor` host | Already in `Cargo.lock`! |
| **Host CPU multi-threading (Layer 2 parallel, later)** | `rayon` | MIT OR Apache-2.0 **[S]** | Native host only (never in `wasm32` core) | Optional parallel execution of CPU fallback kernels on multi-core native hosts. |
| **E-graph algebraic simplification (optional, post-v0)** | `egg` / `egglog` | MIT **[S]** | Toolchain only (`docs/oxi-ui.md` table) | Defer until after 8.3b v0—do **not** block 8.3a/8.3b on e-graphs! |
| **ESP32-S3 PIE DSP/NN (optional device capability)** | `esp-dsp`, `esp-nn` | Apache-2.0 ([esp-dsp](https://github.com/espressif/esp-dsp), [esp-nn](https://github.com/espressif/esp-nn) READMEs) **[V]** | ESP32-S3 firmware only | Used via `host:dsp` / `host:ml` capability, never compiled into portable core. |

*Excluded on licence grounds:* Moddable XS, Toit VM, Jaculus, GCC/libgcc runtime code, Slint embedded, or any GPL/LGPL/AGPL library.

---

## 8. Pre-Implementation Benchmark Suite & Reference Expectations

**Rule: set up the benchmark suite *before* writing Stage 8.3a/8.3b/8.3d code** so every stage measures real before/after deltas against a frozen baseline (mirroring `oxiterm-server/benches/frame_cost.rs` from Stage 5.0 **[V]**).

### 8.1 Harness Design (`oxiscript/oxiscript-bench`, `harness = false`)

Run every workload across **five execution targets** with identical deterministic seed inputs and an automated correctness oracle (exact bitwise check for integers; `max_abs_err` / `max_rel_err` check for floats):
1. **OxiScript Scalar Wasm on `wasmi 2.0.3`** (baseline live interpreter).
2. **OxiScript SIMD128 Wasm on `wasmi 2.0.3`** (measures interpreted SIMD dispatch gain/loss!).
3. **OxiScript Scalar Wasm on Wasmtime (Cranelift) & Browser Wasm** (baseline JIT/AOT).
4. **OxiScript SIMD128 Wasm on Wasmtime (Cranelift) & Browser Wasm** (measures real CPU vector speedup).
5. **OxiScript GPU `#kernel` on `wgpu` (native Vulkan/Metal/DX12 + Mesa `llvmpipe` + Browser WebGPU)** — measured **both** compute-only (resident buffers) **and** end-to-end (upload + dispatch + readback) so transfer overhead is never hidden!
6. **Reference native Rust / WGSL baseline** (compiled with `rustc -O` / hand-written WGSL) to quantify our compiler's efficiency gap (`OxiScript / Rust`).

Record per run (median + p95 over ≥ 30 warm iterations + cold compile/pipeline time, following `frame_cost.rs` **[V]**): wall time (ms), throughput (GFLOP/s or MPix/s or Mparticles/s), compile latency (ms), `.wasm` byte delta, and peak RSS / VRAM.

### 8.2 The Four Benchmark Workloads & Expected Reference Numbers

**[Corrected 2026-11-26] Read every number in this table as a [P] project estimate.** Of the sources the draft cited for these cells, the ORT Web tutorial has no performance numbers, `nuss-and-bolts.com` is a Substack landing page (the article is [zanussbaum.github.io/blog/2024/webgpu](https://zanussbaum.github.io/blog/2024/webgpu), "1 TFLOP+", with GPU and sizes not visible), `web.dev/articles/webassembly-simd` returns 404, and the wgpu boids example states no "1M particles at 60 fps". The only verified reference figures are TF.js Wasm SIMD 1.7–4.5× over plain Wasm (threads a further 1.8–2.9×) ([TF blog](https://blog.tensorflow.org/2020/09/supercharging-tensorflowjs-webassembly.html)); MediaPipe hand tracking 14–15 → 38–40 FPS with SIMD ([v8.dev](https://v8.dev/features/simd)); WebGPU per-dispatch cost 24–71 µs native, 33–67 µs Chrome, ~1 ms Firefox ([arXiv:2604.02344](https://arxiv.org/html/2604.02344v1)); and esp-nn ESP32-S3 3.5–14× per kernel. That is exactly why the suite must be built and measured first.

Reference expectations below assume a typical modern mid-range desktop/laptop CPU (e.g., 4–8 core x86_64/Apple Silicon, single-threaded for Wasm benchmarks) and integrated or mid-range discrete GPU (e.g., Intel Iris Xe / AMD Radeon 780M / Apple M1–M3 / RTX 3060-class). On the repo's older **Intel Core i5-3450 (2012 Ivy Bridge, AVX1 only, no FMA, Mesa `llvmpipe`)** (`docs/oxi-ui.md` Stage 5.0 **[V]**), CPU SIMD128 numbers will be ~2×–3× lower in absolute GFLOP/s (still ~2.5×–3.5× relative speedup over scalar Wasm!) and `llvmpipe` is a CPU Vulkan rasterizer/compute emulator, not a hardware GPU.

| Benchmark | Problem Specification | Scalar Wasm (JIT: Wasmtime / Browser) | Wasm SIMD128 (JIT: Wasmtime / Browser) | `wasmi 2.0.3` (Interp Scalar → SIMD128) | WebGPU / `wgpu` Compute (Hardware GPU, Resident) | Primary Sources & Notes |
|---|---|---|---|---|---|---|
| **1. Matrix Multiply (`f32` GEMM)** | **Small:** 256×256 (`33.5M` FLOPs)<br>**Large:** 1024×1024 (`2.15G` FLOPs)<br>`C = A @ B`, tiled `16×16` workgroups on GPU; `4×4` micro-kernel on SIMD128 | **256×256:** ~15–40 ms (~1–2 GFLOP/s)<br>**1024×1024:** ~1.0–2.5 s (~1–2 GFLOP/s) **[P/S]** | **256×256:** ~4–12 ms (~3–8 GFLOP/s; **3×–4×** vs scalar)<br>**1024×1024:** ~250–600 ms (~3.5–8 GFLOP/s; **3×–4×** vs scalar) **[S/P]** | ~15×–30× slower than JIT (~300–800 ms for 256×256 scalar; ~1.5×–3× faster with SIMD128) **[P/U]** | **256×256:** ~0.1–0.5 ms (dispatch-limited)<br>**1024×1024:** **~2–15 ms** (**150–1,000+ GFLOP/s**; **20×–150×** vs Wasm SIMD128!) **[P; cited sources had no numbers, see note above]** | Classic compute-bound kernel; proves tiled shared memory (`@shared` + `barrier()`). |
| **2. Image Filter (5×5 Gaussian / 3×3 Sobel on RGBA8)** | **1080p** (1920×1080 = 2.07M px) and **4K** (3840×2160 = 8.29M px), RGBA8 input → RGBA8 output (separable 1D or 2D stencil) | **1080p:** ~40–90 ms<br>**4K:** ~160–400 ms (~20–50 MPix/s) **[S/P]** | **1080p:** ~10–35 ms<br>**4K:** ~40–210 ms (**3×–8×** vs scalar JS/Wasm; `u8x16`/`i16x8` widening shines!) **[P; cited web.dev page is a 404]** | ~500–1500 ms for 1080p scalar **[P/U]** | **1080p:** **< 1–3 ms** (zero-copy texture → `GpuSink`!)<br>**4K:** **~2–8 ms** (**10×–40×** vs Wasm SIMD128) **[S]** | Tests narrow `u8`/`i16` SIMD128 lanes AND zero-copy kernel-to-UI texture handoff (§5.6). Note: CPU readback of 4K RGBA8 (33 MB) adds ~3–8 ms—proving zero-copy matters! |
| **3. Particle Physics (Euler + Gravity / Boids)** | **10k** and **100k** `Particle { x, y, vx, vy: f32 }` (16 B each = 1.6 MB for 100k); 100 simulation steps; compares AoS vs SoA (`xs, ys, vxs, vys`) | **100k × 1 step:** ~0.5–1.5 ms (gravity) / ~30–100 ms (grid/boids)<br>**100k × 100 steps (gravity):** ~50–150 ms **[P]** | **100k × 100 steps (SoA):** ~15–45 ms (**3×–3.8×** vs scalar); AoS without deinterleave: ~1.5×–2× **[P/S]** | ~15×–25× slower than JIT **[P/U]** | **100k × 1 step:** **< 0.1–0.3 ms** (scales effortlessly to **1M+ particles at 60 fps** **[S: [wgpu boids](https://github.com/gfx-rs/wgpu/tree/trunk/examples/features/src/boids)]**) | Directly tests `contracts.md` §3 `Array<Particle>` inline structs, SoA vs AoS, and ping-pong GPU storage buffers. |
| **4. Small MLP (MNIST 784→128→10 Forward + Backprop)** | Batch size `B = 64`, `f32`: `FC1 (64×784 @ 784×128)` + ReLU + `FC2 (64×128 @ 128×10)` + Softmax/CrossEntropy + backward + SGD (`~26M` FLOPs/step; 1 epoch ≈ 937 steps) | **1 step (`B=64`):** ~8–25 ms<br>**1 epoch:** ~8–23 s **[P/S]** | **1 step (`B=64`):** ~2–7 ms (**2.5×–4×** vs scalar; matches TF Lite Wasm SIMD **1.7×–4.5×** **[S: [TensorFlow Wasm SIMD](https://blog.tensorflow.org/2020/09/supercharging-tensorflowjs-webassembly.html)]**)<br>**1 epoch:** ~2–7 s | ~150–400 ms/step (fine for 1-sample interactive perceptron demo, too slow for full epoch) **[P/U]** | **1 step (`B=64`, resident weights):** **~0.2–0.8 ms** (dispatch-bound at `B=64`; **~0.5–2 ms** at `B=512`, **5×–15×** vs Wasm SIMD128)<br>**1 epoch:** **< 1 s** **[S/P]** | Validates the Hero course goal (live training in browser) and exposes the small-batch crossover where Wasm SIMD128 beats WebGPU if batch size is `B = 1`! |

**Critical crossover insight for benchmarks [O, corrected 2026-11-26]:** for tiny dispatches (`B = 1` MLP inference, or a few thousand particles), WebGPU's per-dispatch cost (24–71 µs native, 33–67 µs in Chrome, ~1 ms in Firefox, [arXiv:2604.02344](https://arxiv.org/html/2604.02344v1) **[V]**) plus an async readback is likely to make **Wasm SIMD128 faster in latency**. The exact crossover is to be measured. (The draft cited ORT Web for this, but that page has no numbers.) In Firefox the ~1 ms per dispatch makes kernel fusion and batching essential. Keeping the placement report honest about transfer bytes (`OXS-P0301`) and measuring both resident and round-trip times prevents misleading benchmarks.

---

## 9. Open Questions for the Owner

Before starting Stage 8.3a implementation, these calls need the owner's decision (recommended default listed first; questions 7-11 were added by the verifier):

1. **Integer overflow on `Simd<i32, 4>` (`+` vs `+%`/`+|`):**
   - *Option A (Recommended):* Forbid trapping `+`/`-`/`*` on integer `Simd<T, N>` (compile error `OXS-E0110` with quick-fix suggesting `+%` wrapping or `+|` saturating), so every integer SIMD operator maps 1:1 to a single fast Wasm SIMD128 instruction with zero hidden per-lane overflow traps.
   - *Option B:* Allow trapping `+` on integer `Simd` (emitting 4–5 extra instructions + `any_true` trap check) and warn via `OXS-P0108`.
2. **Inline-element arrays (`Array<Particle>`) timing:**
   - Should inline plain-data struct elements in `Array<T>` (`contracts.md` §3 worked example) land in **Stage 8.3a** (unlocking particle/AoS benchmarks and zero-copy GPU uploads immediately) or wait for **Stage 8.3d** (keeping 8.3a strictly to primitive `f32[]`/`i32[]` arrays + SoA)? *Recommendation:* Keep 8.3a to primitive arrays (fast, clean slice), land inline plain-data struct arrays at the start of 8.3d when `std430` layout is needed for `Buffer<Particle>`.
3. **Timing of `8.S4` (placement report) vs `8.3b` (auto-vectoriser):**
   - `8.3b` emits `OXS-P0103..P0112` remarks and enforces `@expect(simd)`. Should `8.S4` land first so `8.3b` plugs into the finalized `placement.json` schema (recommended), or should `8.3a` introduce a minimal `Remark` struct in `oxiscript-abi` early?
4. **GPU OOB semantics in `#kernel` (`clamped/zero` vs `debug trap flag`):**
   - WebGPU/WGSL cannot trap mid-shader, and robust access leaves the OOB result implementation-defined: a read may return any in-bounds value or zero, and a write may land elsewhere in-bounds or be dropped **[corrected 2026-11-26]**. The emitter therefore must guard every access itself (§4.3). The open question is only *which defined result* the guard gives. *Recommendation:* read → 0, write → skipped, plus a debug-build atomic `oob_flag` reported as `TrapKind::Bounds` after the dispatch, documented in `ir.md` and `contracts.md`. The alternative is to always run with the flag and trap the instance after the dispatch, which brings GPU behaviour closer to CPU behaviour at a small cost.
   - **[Revised 2026-11-26 for decision 4 transparency] Revised recommendation.** With transparent executor choice the user cannot see which executor ran, so a debug-only flag would make results depend on the executor, and a read of 0 or a skipped write would differ from the CPU trap. Therefore **v1: GPU-eligible code is trap-free by construction** (no plain trapping integer ops, only provably in-range indices, 4.6.2); the guards remain as robustness for the emitted WGSL and never fire in verified code, so no flag is needed. A later version may add the flag plus a *deferred* trap (checked at the next download or end of turn; the deferral is observable through host effects in between, so it must be documented), which widens eligibility at a semantic cost (Q16). The `host:gpu` expert path keeps the original recommendation (read 0, skip write, debug flag), because there the program asked for GPU semantics explicitly.
5. **Browser `wgpu` vs thin JS WebGPU bridge for `host:gpu@1`:**
   - In browser local mode (`oxiterm-web`, Phase 7.1), `oxiterm-core` + `oxiscript` already run in wasm32. Should `host:gpu@1` in the browser go through `wgpu 30` compiled to wasm32 (shares 100% Rust code with `oxiterm-gpu`, enables zero-copy `GpuSink` in the browser, but adds wasm bundle size—currently unmeasured **[U]**) or call `navigator.gpu` via thin `web-sys` bindings when Canvas2D is the UI sink? *Recommendation:* in a 1-day spike at the start of slice 8.3c, measure the wasm32 gzip size of `wgpu 30` in two variants: `features = ["webgpu","wgsl"]` with `default-features = false` (no naga, since we emit WGSL text), and `["webgpu","webgl","wgsl"]`, which pulls in wgpu-core, wgpu-hal and naga for the WebGL2 fallback **[corrected 2026-11-26]**. Decide only after measuring.
   - [Revised 2026-11-26 for decision 4 transparency] The question now covers the browser *executor* path as well as `host:gpu`: both share one WGSL/arena implementation, so measure once.
6. **Hardware validation target for GPU benchmarks:**
   - The current dev box runs Mesa `llvmpipe` (software Vulkan) on an i5-3450. Will Stage 8.3c/8.3d hardware GPU benchmarks run on a hardware Vulkan/WebGPU machine (e.g., laptop iGPU/dGPU or Raspberry Pi 5 V3D) before closing Stage 8.3d's speedup gate?

*Added by the verifier (2026-11-26):*

7. **Vector `trunc_sat` in ABI 1.1.** ABI 1.0 bans non-trapping float-to-int (`abi.md:5`). Should ABI 1.1 allow `i32x4.trunc_sat_f32x4_*` under the `simd128` feature, or should vector conversions lower to explicit clamp + convert sequences, consistent with the scalar rule?
8. **Multi-core layer.** `oxi-ui.md` says "CPU SIMD + multi-core first", but this report covers multi-core only with a `rayon` row. Should 8.3 include the isolate-based parallel layer (Layer 2 in `oxiscript-language.md` §E: chunked `parallel for` over isolates with copied/moved data, no shared-memory Wasm threads), or is that 8.4?
9. **ESP32-S3 SIMD default.** No measurement exists for wasmi with SIMD on Xtensa (treVM measured Wasmtime/Pulley on other boards). Keep "scalar by default on the S3" as a provisional [O] until the board (expected after 2026-12-10) is benchmarked?
10. **Upstream report.** Should we file the wasmi `relaxed_madd` (fused under `deterministic`) divergence from the Wasm 3.0 deterministic profile upstream?
11. **Repo research fix.** `docs/research/oxiscript-language.md:320,341` also says that relaxed SIMD ships in all three engines (Safari needs a flag). Should it be corrected in the same pass?

*Added by the decision 4 revision (2026-11-26) [Revised 2026-11-26 for decision 4 transparency]:*

12. **Runtime-inserted transfers.** The default forbids them (hidden transfers killed C++ AMP). Should the owner allow an opt-in policy "above `N` elements the runtime may upload host inputs and download results, logged as `OXS-P0306`"? It would make results and timing depend on a threshold and weaken `@expect(no_transfer)`. *Recommendation:* no; keep it a library function (`gpuMap(xs, f)` that does `upload` + `map` + `download` visibly).
13. **Size thresholds.** What are `N_hint` and `N_small` (provisional 65 536 and 4 096 **[P]**)? They only drive report hints, so they can be tuned after the 8.3d benchmarks; may they be per-host (measured at startup) or must they be fixed constants so that the report is reproducible? *Recommendation:* fixed constants per edition.
14. **Residency type name and authority.** Is `Resident<T>` the right name (runner-up `Gpu<T>`; `Device<T>` rejected by the `--target device` clash)? Should a module that may use the GPU executor declare it in `needs`/the manifest so an operator can forbid it (default proposed: no declaration, host quotas only)?
15. **Failure of resident operations.** On device loss or an exceeded dispatch cap, should layer-0 operations on `Resident` data trap (`quota`/`device-lost` trap kind) or return `Result`? A `Result` return makes every combinator fallible and hurts the "same code" goal. *Recommendation:* trap, with `Resident` handles poisoned, and rebuild by re-`upload`.
16. **How wide is GPU eligibility in v1?** Excluding trap-capable ops and `filter` keeps integer results identical but may leave much real code CPU-only. The v1 choice is to *widen the proof scope* (affine indices, `localId`/`@shared` tiles and shaped views, 4.6.2) rather than weaken the gates, so GEMM, conv2d and attention remain on the GPU executor. Data-dependent gathers (`xs[idx[i]]`) stay CPU-only. Measure the eligible fraction on the 8.3d corpus before adding a flag-plus-deferred-trap channel (gathers) or a scan for `filter`.
17. **May the executor differ between runs?** The policy is deterministic for fixed types, tier, profile and target, but the *host tier* (GPU present or not) differs between machines, so low-order float bits can differ between a laptop and a server (within the 4.5 contract). Acceptable by default, with `safety` pinning float code to the CPU, or should the default profile also pin it unless the program opts in to `@approx`? *Recommendation:* accept; record the executor in the report and the run log.
18. **Which layer-0 vocabulary is core?** The combinators with defined parallel semantics (`map`, `zip`, `reduce`, `reduceSum`, `parallel for`, `filter`, image/grid stencils) need a spec pass in `grammar.md`/stdlib before 8.3b. Which are in v1?

---

## 10. References

### Verified Local Repository & Crate Sources (`[V]`)
- Repository specs & architecture: `docs/oxi-ui.md`, `docs/oxiscript/contracts.md`, `docs/oxiscript/ir.md`, `docs/oxiscript/abi.md`, `docs/oxiscript/grammar.md`, `docs/research/oxiscript-language.md`, `docs/research/vulkan-acceleration.md`, `docs/research/oxibench-methodology.md`.
- Stage 5.2 (`oxiterm-gpu`): `docs/stages/5.2/{implementation,review,bad-actor,fixes}.md`, `oxiterm-gpu/Cargo.toml`, `oxiterm-gpu/src/{context.rs,sink.rs}`.
- OxiScript crates: `oxiscript/oxiscript-abi/registry/codes.txt`, `oxiscript/oxiscript-abi/src/{layout.rs,idl.rs}`, `oxiscript/oxiscript-ir/src/types.rs`, `oxiscript/oxiscript-wasm/{Cargo.toml,src/lib.rs}`, `oxiscript/oxiscript-runtime/{Cargo.toml,src/{lib.rs,load.rs,runtime.rs}}`.
- Locked crate sources (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`):
  - `wasmi-2.0.3/{Cargo.toml,src/lib.rs,src/engine/config.rs,src/engine/translator/utils.rs}`
  - `wasmi_core-2.0.3/{Cargo.toml,src/simd.rs,src/wasm.rs,src/value.rs,src/float.rs}`
  - `wasm-encoder-0.263.0/src/core/code.rs`
  - `wgpu-30.0.2/src/{api/shader_module.rs,api/device.rs,backend/webgpu.rs,backend/wgpu_core.rs}`
  - `wgpu-types-30.0.2/src/{limits.rs,features.rs}`
  - `naga-30.0.2/Cargo.toml`

### External Specifications, Papers & Documentation
- **Added by the verifier (2026-11-26):** Wasm 3.0 numerics, relaxed operations / deterministic profile: https://webassembly.github.io/spec/core/exec/numerics.html ; https://webassembly.github.io/spec/core/appendix/profiles.html ; WebAssembly features.json: https://github.com/WebAssembly/website/blob/main/features.json ; wasmi CHANGELOG: https://github.com/wasmi-labs/wasmi/blob/main/CHANGELOG.md ; Wasmtime `Config` (relaxed_simd_deterministic, cranelift_nan_canonicalization): https://docs.rs/wasmtime/latest/wasmtime/struct.Config.html ; WebGPU dispatch overhead (arXiv:2604.02344): https://arxiv.org/html/2604.02344v1 ; esp-nn: https://github.com/espressif/esp-nn ; PIE intro: https://developer.espressif.com/blog/2024/12/pie-introduction/ ; epdiy PIE discussion: https://github.com/vroland/epdiy/discussions/289 ; Rust std::simd: https://doc.rust-lang.org/nightly/std/simd/struct.Simd.html ; WebGPU robust access notes: https://kvark.github.io/webgpu-debate/RBA.html ; WebGPU matmul article: https://zanussbaum.github.io/blog/2024/webgpu ; Local: `wgpu-types-30.0.2/src/limits.rs:557-569,753-760,931`, `features.rs:1130-1142,1690-1700`, `wgpu-30.0.2/src/api/device.rs:957-985`, `wgpu-30.0.2/Cargo.toml [features]`, `wasmi_core-2.0.3/src/{simd.rs:1399-1402,1889-1904, value.rs:472-475,550-553,710-713}`, `naga-30.0.2/src/front/wgsl/parse/directive/{enable_extension.rs,language_extension.rs}`.
- **WebAssembly & Engines:**
  - WebAssembly Features Roadmap: https://webassembly.org/features/
  - WebAssembly 3.0 Deterministic Profile: https://webassembly.org/news/2025-09-17-wasm-3.0/
  - WebAssembly Relaxed SIMD Proposal: https://github.com/WebAssembly/relaxed-simd
  - Wasmi Repository & Docs: https://github.com/wasmi-labs/wasmi ; https://docs.rs/wasmi/ ; https://github.com/wasmi-labs/wasmi-benchmarks
  - Wasmtime Stability (Platforms & Proposals): https://docs.wasmtime.dev/stability-platform-support.html ; https://docs.wasmtime.dev/stability-wasm-proposals.html
  - Cranelift Compiler Backend: https://cranelift.dev/
  - WABT `wasm2c` SIMD support: https://github.com/WebAssembly/wabt/tree/main/wasm2c
  - `w2c2` (MIT): https://github.com/turbolent/w2c2
  - SIMDe (SIMD Everywhere, MIT): https://github.com/simd-everywhere/simde
  - WAMR (WebAssembly Micro Runtime): https://github.com/bytecodealliance/wasm-micro-runtime
  - treVM MCU WebAssembly study (IEEE DCOSS-IoT 2026): https://arxiv.org/html/2604.27570v1
  - ESP32-S3 PIE SIMD & `esp-dsp`: https://docs.espressif.com/projects/esp-dsp/en/latest/esp32s3/index.html ; https://github.com/espressif/esp-dsp
- **SIMD Languages & Auto-Vectorisation:**
  - Rust `portable_simd`: https://github.com/rust-lang/portable-simd
  - C# `Vector128<T>` (.NET Intrinsics): https://learn.microsoft.com/en-us/dotnet/api/system.runtime.intrinsics.vector128-1
  - Java Vector API (JEP 489): https://openjdk.org/jeps/489
  - Zig `@Vector`: https://ziglang.org/documentation/master/#Vectors
  - Mojo `SIMD`: https://docs.modular.com/mojo/stdlib/builtin/simd/SIMD/
  - AssemblyScript `v128`: https://www.assemblyscript.org/stdlib/builtins.html#simd
  - TC39 `SIMD.js` (archived) & V8 Wasm SIMD history: https://github.com/tc39/ecmascript_simd ; https://v8.dev/features/simd ; https://emscripten.org/docs/porting/simd.html
  - LLVM Auto-Vectorizers & VPlan: https://llvm.org/docs/Vectorizers.html ; https://llvm.org/docs/Proposals/VectorizationPlan.html
  - Maleki et al. (PACT 2011), *An Evaluation of Vectorizing Compilers*: https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers
  - TSVC2 modern compiler vectorisation study (2025): https://arxiv.org/html/2502.11906v1
  - Matt Pharr (ISPC): https://pharr.org/matt/blog/2018/04/18/ispc-origins ; https://pharr.org/matt/blog/2018/04/29/ispc-retrospective
  - Halide (PLDI 2013): https://people.csail.mit.edu/jrk/halide-pldi13.pdf
  - Futhark (PLDI 2017 & GPU backends): https://www.futhark-lang.org/publications/pldi17.pdf ; https://futhark-lang.org/
- **GPU Kernels, WebGPU, WGSL & Browser AI:**
  - W3C WebGPU & WGSL Specifications (Floating-Point Evaluation, Scalar Types, Uniformity): https://www.w3.org/TR/webgpu/ ; https://www.w3.org/TR/WGSL/ ; https://www.w3.org/TR/WGSL/#floating-point-evaluation ; https://www.w3.org/TR/WGSL/#uniformity
  - WebGPU Browser Implementation Status: https://github.com/gpuweb/gpuweb/wiki/Implementation-Status
  - WebGPU `shader-f16` (Chrome 120): https://developer.chrome.com/blog/new-in-webgpu-120
  - `wgpu` & `naga` (MIT OR Apache-2.0): https://github.com/gfx-rs/wgpu ; https://github.com/gfx-rs/wgpu/tree/trunk/examples/features/src/boids
  - Windows WDDM TDR (Timeout Detection and Recovery): https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery--tdr-
  - CubeCL & Burn (MIT OR Apache-2.0): https://github.com/tracel-ai/cubecl ; https://github.com/tracel-ai/burn
  - `rust-gpu` transition: https://rust-gpu.github.io/blog/transition-announcement
  - Taichi (Apache-2.0): https://docs.taichi-lang.org/
  - Triton (MIT): https://github.com/triton-lang/triton
  - Slang (Apache-2.0 w/ LLVM exc.): https://shader-slang.org/ ; https://www.khronos.org/news/permalink/khronos-group-launches-slang-initiative-hosting-open-source-compiler-contributed-by-nvidia
  - Mojo GPU (`LayoutTensor`): https://docs.modular.com/mojo/manual/gpu/fundamentals/
  - WebLLM (MLC-AI): https://github.com/mlc-ai/web-llm
  - Candle (Hugging Face): https://github.com/huggingface/candle
  - ONNX Runtime Web & Transformers.js: https://onnxruntime.ai/docs/tutorials/web/ ; https://huggingface.co/docs/transformers.js
  - TensorFlow.js Wasm SIMD & WebGPU benchmarks: https://blog.tensorflow.org/2020/09/supercharging-tensorflowjs-webassembly.html ; https://v8.dev/features/simd

