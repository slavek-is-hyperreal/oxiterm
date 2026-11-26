# OxiScript Stages 8.3 (SIMD & Parallel) and 8.3b (GPU Kernels): Research & Architecture Design

> **Status:** Design specification (research stage, 2026-11-26). No product code changed.
> **Scope:** Stage 8.3 (SIMD types, auto-vectorisation, `@expect(simd)`, placement report) and Stage 8.3b (`kernel` profile, OxiIR → WGSL, `host:gpu`, shared `GpuContext` integration with `oxiterm-gpu` Stage 5.2, AI/ML foundation, benchmarks, and staged delivery plan).
> **Evidence tags:** **[V]** verified directly in local repo/crate sources (`~/.cargo/registry`, `Cargo.lock`) or primary upstream specifications; **[S]** per web search summary of cited external source (verify before relying on exact numbers); **[P]** project design estimate; **[O]** architectural recommendation/opinion; **[U]** unverified / open unknown.

---

## Executive Summary & Recommendations

OxiScript's core compiler pipeline (`8.C`–`8.L`, `8.M`, `8.R`) and `oxiterm-gpu` (`5.2`) are already in place and deliberately prepared for data parallelism. This report designs **Stage 8.3** (explicit SIMD, predictable auto-vectorisation, placement reporting, `@expect(simd)`) and **Stage 8.3b** (the `kernel` profile, OxiIR → WGSL, `host:gpu@1`, and shared-device integration with `oxiterm-gpu`), keeping OxiScript readable by TypeScript developers, deterministic by default, and permissive-only in licensing.

### Key Architectural Decisions Recommended

| Area | Recommendation | Rationale |
|---|---|---|
| **1. Wasm SIMD target** | Target **Wasm fixed-width SIMD128** (ABI minor bump `1.1`, opt-in per target profile); keep **Relaxed SIMD off by default** (explicit opt-in only under non-deterministic profiles). | Supported in `wasmi 2.0.3` (behind `simd` Cargo feature, zero overhead on non-SIMD code **[V]**), Wasmtime/Cranelift (Tier 1), `wasm2c`/`w2c2` (via MIT-licensed SIMDe), and all major browsers. Relaxed SIMD breaks cross-target determinism (`ir.md` §6). |
| **2. ESP32-S3 PIE SIMD** | Do **not** rely on `wasm2c`/GCC auto-vectorisation for Xtensa PIE; expose PIE through linked C/assembly intrinsics (`esp-dsp`/`esp-nn`) or an explicit OxiIR-to-PIE-C pattern matcher for `i8`/`i16` dot/MAC kernels later. | Upstream GCC/LLVM do not auto-vectorise into ESP32-S3's proprietary `ee.*` PIE instructions **[S]**. On MCUs, interpreted Wasm SIMD can even regress speed (treVM **[V]**), so MCU profiles default to scalar Wasm. |
| **3. Explicit SIMD syntax** | Built-in generic value type **`Simd<T, N>`** (fixed 128-bit lanes first: `f32x4`, `f64x2`, `i32x4`, `u32x4`, `i16x8`, `u16x8`, `i8x16`, `u8x16`, plus `SimdMask<T, N>`). Ordinary operators (`+`, `-`, `*`, `+&`/`+%`/`+|`), static constructors (`Simd.splat`, `Simd.load`), no untyped `v128` bitbag. | Readable as typed generics by any TS developer; aligns with `contracts.md` §3 (`Simd<f32, 4>`) and Rust `std::simd`; requires zero new grammar; lowers 1:1 to OxiIR `Ty::Vec` and either Wasm `v128` or identical-semantics scalar loops. |
| **4. Auto-vectoriser v0** | Narrow, honest, predictable loop vectoriser on OxiIR SSA for **countable unit-stride loops** (`parallel for`, `for (const i of range(0, n))`) over contiguous `[T]`/inline-struct arrays, plus explicit reductions (`reduce`) and branchless `select`. Emit structured `OXS-P01xx` remarks for every declined loop. | Auto-vectorisation alone fails on 70–82% of real loops (Maleki et al. **[S]**; Pharr **[V]**). Making `@expect(simd)` assert only language-defined vectorisability facts (`assertable yes`) and reporting heuristics (`assertable no`) avoids brittle builds. |
| **5. GPU kernel lowering** | Lower `#kernel` OxiIR functions directly to **WGSL text** (validated offline in dev/CI via `naga` `wgsl-in`). Do **not** emit SPIR-V directly or pass `ShaderSource::Naga` in browsers. | Verified in `wgpu-30.0.2/src/backend/webgpu.rs` **[V]**: `ShaderSource::Wgsl` passes straight to WebGPU browsers with zero Naga bundle cost, while native `wgpu` compiles WGSL via its bundled Naga to Vulkan SPIR-V, Metal MSL, and DX12 HLSL automatically. |
| **6. Shared `GpuContext`** | Upgrade `oxiterm-gpu`'s `GpuContext` with **tiered capability negotiation**: request `Limits::defaults()` (compute + storage buffers) when `DownlevelFlags::COMPUTE_SHADERS` is set, transparently falling back to `downlevel_webgl2_defaults()` (UI-only, kernels run on CPU SIMD/scalar). | Preserves OxiUI's GLES 3.0 / WebGL2 compatibility floor while unlocking WebGPU/Vulkan/Metal/DX12 compute on one shared `wgpu::Device` & `wgpu::Queue`, enabling zero-copy kernel-to-UI texture handoff (`Rgba8Unorm`). |
| **7. Safety & Error Isolation** | Wrap every script GPU operation in `GpuContext::with_error_scopes`, forbid script-registered uncaptured handlers, enforce per-instance VRAM/buffer/dispatch quotas, inject **static loop trip-count guards** in emitted WGSL, and chunk large dispatches to stay well below the 2 s OS TDR watchdog. | Directly addresses Stage 5.2 BAD ACTOR B-01/B-03 findings (`wgpu` error scopes are thread-local; uncaptured handler is device-wide **[V]**) and prevents runaway user kernels from freezing the UI or triggering OS GPU device resets. |
| **8. AI on top** | Keep AI out of the core language; ensure `kernel` + `host:gpu@1` support **tiled `f32` GEMM** (workgroup shared memory + barriers), **conv2d**, **reductions** (softmax/layernorm), **attention**, **backward passes** (for live MLP training), plus optional **`f16`** (`shader-f16`) and **`i8`/`u8` packed dequant** with `i32`/`f32` accumulation. Skip native `bf16` (not in WGSL **[S]**). | Satisfies the "Hero" course goal (perceptron → live-trained digit recogniser → tiny LM in browser) using the same general-purpose kernel stack. |

---

## 1. SIMD Across Execution Engines & Targets

OxiScript targets one portable bytecode format (Core WebAssembly) executed across very different engines: the embedded `wasmi` interpreter (local core, browser live mode, ESP32 dev mode), browser JIT engines (web release), Wasmtime/Cranelift (64-bit native release), and Wasm-to-C (`wasm2c`/`w2c2`) + native C compilers (32-bit SBCs, ESP32 release, safety-critical builds). Because SIMD capability and performance vary drastically across these engines, **SIMD lowering must be a per-target backend choice from one semantic OxiIR**, never a single mandatory `.wasm` binary.

### 1.1 `wasmi` (Exact Version in `Cargo.lock`: `2.0.3`)

Direct inspection of `Cargo.lock` and `~/.cargo/registry/src/*/wasmi-2.0.3` / `wasmi_core-2.0.3` establishes the exact capabilities of our live interpreter **[V]**:

1. **Proposal support & versions:** Upstream `wasmi` added fixed-width `simd` in **v0.43.0** and `relaxed-simd` in **v0.44.0** ([wasmi GitHub](https://github.com/wasmi-labs/wasmi), [docs.rs/wasmi](https://docs.rs/wasmi/)) **[S]**. Both proposals are fully present in our pinned **`wasmi 2.0.3`** **[V]**.
2. **Feature gating:** Both proposals sit behind a single opt-in Cargo feature `simd` (disabled in `wasmi`'s `default` features) **[V, `wasmi-2.0.3/Cargo.toml:69`]**:
   ```toml
   simd = ["wasmi_core/simd", "wasmi_ir/simd", "wasmparser/simd"]
   ```
   Currently `oxiscript-runtime/Cargo.toml:18` enables `features = ["validate", "deterministic", "stable", "auto-dispatch"]` (no `simd`). At runtime, `wasmi::Config` provides separate toggles `c.wasm_simd(bool)` and `c.wasm_relaxed_simd(bool)` (gated on `cfg(all(feature = "simd", feature = "validate"))`; both default to `true` when the feature is compiled in, so `oxiscript-runtime` **must explicitly set `c.wasm_relaxed_simd(false)`** when enabling `simd`) **[V, `wasmi-2.0.3/src/engine/config.rs:324-338`]**.
3. **Zero penalty on non-SIMD code in `wasmi 2.0`:** In `wasmi 1.x`, enabling the `simd` feature widened stack slots globally and regressed non-SIMD benchmarks like CoreMark by ~8% ([wasmi 2.0 architecture notes](https://github.com/wasmi-labs/wasmi)) **[S]**. In `wasmi 2.0.3`, `ValType::V128` simply occupies **2 register slots** (`wasmi-2.0.3/src/engine/translator/utils.rs:15`) instead of bloating every scalar slot **[V]**, so enabling the `simd` Cargo feature imposes no memory or speed regression on non-SIMD modules **[S]**.
4. **How `wasmi 2.0.3` executes SIMD (interpreted performance):** Inspection of `wasmi_core-2.0.3/src/simd.rs:252-259` confirms **there are no `core::arch` hardware SIMD intrinsics** inside `wasmi_core`. Instead, every lane operation (`lanewise_binary`, `lanewise_unary`, `lanewise_comparison`) executes a **plain Rust fixed-bound array loop** (`for i in 0..Self::LANES { lhs[i] = f(lhs[i], rhs[i]); }`) over `#[repr(transparent)] struct([T; N])` **[V]**.
   - On desktop x86_64/aarch64 in `--release`, `rustc`/LLVM can auto-vectorise simple integer lane loops inside `wasmi_core`'s instruction handlers, and—more importantly—**1 interpreter dispatch processes 4–16 lanes instead of 4–16 interpreter dispatches**. Because dispatch overhead dominates interpreted loops, Wasm SIMD128 in `wasmi` on desktop CPUs is expected to outperform interpreted scalar loops (~1.5×–3.5× depending on lane width and memory ratio **[P/U: unmeasured; benchmark in Stage 8.3a]**), though still ~10×–30× slower than JIT/AOT native SIMD **[S: [wasmi-benchmarks](https://github.com/wasmi-labs/wasmi-benchmarks)]**.
   - On 32-bit MCUs without 128-bit vector registers, the opposite occurs: wider `V128` register traffic (2 slots + 16-byte copies) and larger handler code size hurt cache/RAM. In the treVM MCU study ([arXiv:2604.27570](https://arxiv.org/html/2604.27570v1)), Wasmtime (Pulley) *without* SIMD beat Wasmtime *with* SIMD on 3 of 4 MCU boards (e.g., RP2350: 18.6 vs 13.2) **[V]**.
5. **Determinism interaction in `wasmi 2.0.3`:** Verified in `wasmi_core-2.0.3/src/simd.rs:938-980`, `wasm.rs:205-221`, and `value.rs:406-418,605-655` **[V]**: SIMD float operations (`f32x4_add/sub/mul/div/sqrt/min/max`, `f64x2_*`) delegate directly to `wasm::f32_*` → `Float::*`, which calls `.canonicalize_nan_if_enabled()` on every lane when `features = ["deterministic"]` is enabled. Thus `wasmi 2.0.3` already provides bit-deterministic canonical NaNs (`0x7FC0_0000` / `0x7FF8_0000_0000_0000`) for fixed-width SIMD128 float ops! Note also that `wasmi` lowers relaxed-SIMD ops deterministically in software (e.g. `f32x4_relaxed_min` forwards to `f32x4_min`), but JIT/AOT hosts do **not** guarantee the same choice—so relaxed SIMD must remain disabled in the default/deterministic ABI.

### 1.2 AOT, Wasm-to-C, and Browser Engines

* **Wasmtime / Cranelift (64-bit native AOT/JIT):** Tier 1 production support for both fixed-width **SIMD128** and **Relaxed SIMD** on `x86_64`, `aarch64`, `s390x`, and `riscv64` ([Wasmtime Proposals](https://docs.wasmtime.dev/stability-wasm-proposals.html), [Platform Support](https://docs.wasmtime.dev/stability-platform-support.html)) **[V]**. Lowers Wasm `v128` directly to SSE/AVX2/AVX-512, NEON, or RVV. **No 32-bit backend** (32-bit falls back to the Pulley interpreter) **[V]**. Critically, **Cranelift performs no loop auto-vectorisation** ([Cranelift design](https://cranelift.dev/)) **[S]**: if OxiScript emits scalar Wasm, Wasmtime will never vectorise it—OxiScript itself must emit Wasm `v128` ops.
* **Browsers (V8 / SpiderMonkey / JavaScriptCore):** Fixed-width **SIMD128** is baseline everywhere (Chrome ≥ 91, Firefox ≥ 89, Safari ≥ 16.4; [WebAssembly Roadmap](https://webassembly.org/features/), [Emscripten SIMD docs](https://emscripten.org/docs/porting/simd.html)) **[S]**. **Relaxed SIMD** is supported in Chrome ≥ 114, Firefox ≥ 145, and Safari (all three major engines per BCD as of 2026) **[S]**. Browser optimising compilers (V8 TurboFan, SpiderMonkey Ion, JSC OMG) lower `v128` directly to SSE/AVX or NEON; like Cranelift, they do **not** reliably auto-vectorise scalar Wasm loops.
* **`wasm2c` (WABT) and `w2c2` (AOT via C compiler):** Both support WebAssembly fixed-width **SIMD128** by emitting calls to **[SIMDe](https://github.com/simd-everywhere/simde)** (MIT-licensed header-only C library; `<simde/wasm/simd128.h>`) ([WABT `wasm2c` README](https://github.com/WebAssembly/wabt/tree/main/wasm2c), [`w2c2` README](https://github.com/turbolent/w2c2)) **[V/S]**. On x86, ARM NEON, AltiVec, and RISC-V RVV, SIMDe maps those intrinsics to native hardware SIMD headers; on targets without a SIMDe backend, SIMDe falls back to portable C99 scalar loops. Relaxed SIMD status in `wasm2c`/`w2c2` is partial/unverified **[U]**—another reason not to require relaxed SIMD.
* **WAMR (optional MCU runtime):** Lists fixed-width SIMD as a post-MVP feature on x86-64/AArch64 AOT/JIT ([WAMR README](https://github.com/bytecodealliance/wasm-micro-runtime)) **[V]**; not available as hardware SIMD on Xtensa.

### 1.3 ESP32-S3 (Xtensa LX7) & PIE SIMD: Can an AOT Path Use It?

The ESP32-S3 uses a dual-core **Xtensa LX7** with Espressif's proprietary **PIE (Processor Instruction Extensions)**—128-bit vector registers (`q0`–`q7`) with packed `i8`/`i16`/`i32` multiply-accumulate, saturated arithmetic, and dot-product instructions prefixed `ee.*` ([ESP32-S3 Technical Reference / PIE docs](https://docs.espressif.com/projects/esp-dsp/en/latest/esp32s3/index.html)) **[S]**. Hand-coded PIE assembly yields **5×–44×** speedups on `int8`/`int16` DSP and NN inner loops **[S: [esp_simd](https://github.com/espressif/esp-dsp)]**.

However, **neither upstream GCC (`xtensa-esp32s3-elf-gcc`) nor LLVM auto-vectorises standard C loops or SIMDe fallback code into Xtensa PIE instructions** **[S]**. Why:
1. PIE uses vendor-specific Tensilica TIE encodings, strict 128-bit alignment rules, and custom accumulator registers (`accx`) not modeled by GCC's tree vectoriser or LLVM's LV.
2. SIMDe has **no Xtensa PIE backend** **[U: verified absent in standard SIMDe target list; re-check before relying]**, so `wasm2c`/`w2c2` + SIMDe compiles Wasm `v128` on ESP32-S3 down to **scalar C loops** (worse than plain scalar Wasm due to struct packing/unpacking overhead).

**Design consequence for ESP32-S3 AOT:**
1. **Default ESP32-S3 target profile:** Compile OxiScript to **scalar Wasm** (disabling Wasm SIMD128 emission) for both `wasmi` live mode and `w2c2` release AOT, avoiding `v128` unpacking overhead.
2. **Using PIE on ESP32-S3 (two clean paths):**
   - **Short-term (Stage 8.N / `host:ml` / `host:dsp`):** Expose Espressif's permissive Apache-2.0 **[S: verify LICENSE in `esp-dsp`/`esp-nn`]** **`esp-dsp`** and **`esp-nn`** kernels as host capabilities or linked native shims on ESP32-S3. OxiScript's standard tensor/DSP library dispatches `dot`, `matmul_i8`, `fir`, `conv2d` to the host capability when present and falls back to OxiScript SIMD/scalar otherwise, with the placement report recording `placed: host:dsp (esp32-s3 PIE)`.
   - **Long-term (direct OxiIR → C backend):** Because OxiIR preserves `Ty::Vec { elem, lanes }` and `kernel` semantics before Wasm erasure, a future OxiIR → C emitter can emit `ee.*` inline assembly / Espressif intrinsics directly for `Simd<i8, 16>` / `Simd<i16, 8>` MAC patterns without going through Wasm `v128` at all.

### 1.4 Target × SIMD Capability Matrix

| Target / Runtime | Wasm SIMD128 | Wasm Relaxed SIMD | Hardware Vector ISA | How OxiScript Uses SIMD | Expected Speedup vs Scalar | Notes / Caveats |
|---|---|---|---|---|---|---|
| **Local dev / OxiUI live (`wasmi 2.0.3`, desktop x86_64/arm64)** | Yes (`simd` feature + `wasm_simd(true)`) **[V]** | Yes (must disable: `wasm_relaxed_simd(false)`) **[V]** | Emulated via Rust `[T; N]` loops in `wasmi_core` (LLVM may auto-vec handlers) **[V]** | Emit Wasm `v128` when target profile = `simd128`; scalar otherwise | **~1.5×–3.5×** vs interpreted scalar (fewer dispatches) **[P/U]** | Zero overhead on non-SIMD code in `wasmi 2.0` (`V128` = 2 slots) **[V]**; `deterministic` canonicalises lane NaNs **[V]** |
| **Browser release (Chrome / Firefox / Safari Wasm engine)** | Yes (universal baseline) **[S]** | Yes (all 3 engines, 2026) **[S]** | x86 SSE/AVX, ARM NEON via browser JIT | Emit Wasm `v128`; scalar fallback build optional | **2×–4×** (`f32`/`i32`), **4×–8×** (`u8`/`i16`) **[S]** | Relaxed SIMD kept off in deterministic mode |
| **Browser live mode (`wasmi` compiled to wasm32)** | Yes (wasm-in-wasm) **[V]** | Disabled | Emulated inside wasm32 `wasmi` | Scalar Wasm preferred for fast compile/small size; `v128` optional | Modest dispatch savings **[P/U]** | Heavy compute uses browser Wasm release or WebGPU |
| **Native 64-bit release (Wasmtime / Cranelift)** | Yes (Tier 1) **[V]** | Yes (Tier 1) **[V]** | x86_64 SSE4.2/AVX2/AVX-512, aarch64 NEON, riscv64 RVV | Emit Wasm `v128` (Cranelift does **not** auto-vectorise scalar Wasm **[S]**) | **2.5×–4×** (`f32`/`i32`), up to **8×+** (`i8`/`i16`) **[P/S]** | 64-bit hosts only; no 32-bit Cranelift **[V]** |
| **32-bit Linux SBC (RPi 4/5 ARMv7/ARMv8, `w2c2`/`wasm2c` + GCC/Clang)** | Yes (via SIMDe, MIT) **[V/S]** | Unverified **[U]** | ARM NEON (via SIMDe → `<arm_neon.h>`) | Emit Wasm `v128` → `w2c2` + SIMDe → GCC/Clang `-O3 -mfpu=neon` | **2×–4×** (`f32`/`i32`) **[P]** | Must verify C compiler flags; `w2c2` requires C99 for SIMD **[V]** |
| **ESP32-S3 live (`wasmi` `no_std`)** | Feature exists, **turn OFF** **[O]** | Off | None used by `wasmi` | Emit **scalar Wasm** | **Regression** if Wasm SIMD enabled (treVM **[V]**) | Saves flash & avoids 2-slot register pressure |
| **ESP32-S3 AOT (`w2c2`/`wasm2c` + `xtensa-esp32s3-elf-gcc`)** | Scalar C via SIMDe (no PIE) **[S/U]** | No | **Xtensa PIE** (`ee.*`, 128-bit `i8`/`i16`/`i32`) **[S]** | Emit **scalar Wasm**; dispatch DSP/NN ops to `esp-dsp`/`esp-nn` capability, or future direct OxiIR→C PIE intrinsics | **5×–44×** via `esp-dsp`/`esp-nn` PIE **[S]** | GCC/LLVM do not auto-vectorise PIE **[S]** |
| **Safety-critical target (qualified C compiler via `w2c2`/OxiIR→C)** | Optional (scalar fallback guaranteed identical) | **Forbidden** | Target-dependent | Scalar fallback or strictly verified deterministic `v128` (no FMA/relaxed ops) | Deterministic parity > peak speed | NaN canonicalised per `ir.md` §6 |

---

## 2. Language Design for Explicit SIMD

OxiScript's design rule (`grammar.md`) is strict: **a TypeScript developer must read the code without learning a foreign dialect**, while semantics remain sound, statically typed, and deterministic (`ir.md` §6). Below we survey 11 existing SIMD designs and synthesize the cleanest fit for OxiScript.

### 2.1 Comparison of Existing SIMD Language Designs

| Language / System | Vector Type Surface | Width Model | Lane Access & Shuffles | Masks & Predication | Portable Fallback | Lessons for OxiScript |
|---|---|---|---|---|---|---|
| **Rust `std::simd`** (`portable_simd`, nightly) ([tracking](https://github.com/rust-lang/portable-simd)) **[S]** | `Simd<T, const N: usize>` (e.g. `Simd<f32, 4>`) | Fixed compile-time lane count `N` (1..64 powers of 2) | `v[i]` (dynamic or const), `simd_swizzle!(v, [3,2,1,0])` (const indices) | Dedicated `Mask<M, N>` (`m.select(a, b)`, `all()`, `any()`) | Yes (LLVM lowers arbitrary `N` to chunks/scalars) | **Closest model**: clean generic type, distinct typed mask prevents bitmask/int confusion, const-only swizzles map to hardware. |
| **C# `Vector128<T>` / `Vector<T>`** ([.NET Intrinsics](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.intrinsics.vector128-1)) **[S]** | `Vector128<float>` (fixed bit-width) & `Vector<float>` (hardware-width) | Both: fixed bit-width (`64/128/256/512`) and opaque `Vector<T>.Count` | `v.GetElement(i)`, `Vector128.Shuffle(v, idx)` | Same type (`Vector128<T>` all-1s/0s) + `ConditionalSelect` | Yes (JIT software emulation) | Fixed width (`Vector128`) won over variable `Vector<T>` for predictable algorithms & serialization; untyped all-1s float masks are error-prone. |
| **Java Vector API** (JEP 448/469/508) ([OpenJDK JEP 489](https://openjdk.org/jeps/489)) **[S]** | `FloatVector` + `VectorSpecies<Float>` | Runtime species (`SPECIES_PREFERRED` or `SPECIES_128`) | `v.lane(i)`, `v.rearrange(VectorShuffle)` | Dedicated `VectorMask<Float>` (`a.blend(b, mask)`) | Yes (pure Java fallback) | Explicit `Species` is verbose and awkward in static AOT; confirms dedicated `Mask` type and `select`/`blend` ergonomics. |
| **Zig `@Vector`** ([Zig docs](https://ziglang.org/documentation/master/#Vectors)) **[S]** | `@Vector(4, f32)` | Arbitrary compile-time length `N` | `v[i]`, `@shuffle(f32, a, b, mask)`, `@splat(x)`, `@reduce(.Add, v)` | `@Vector(N, bool)` + `@select(T, mask, a, b)` | Yes (LLVM lowers) | First-class operators (`+`, `*`, `+%`, `+|`) make vector math read like scalar math! Builtin `@reduce` is super clean. |
| **Mojo `SIMD`** ([Modular docs](https://docs.modular.com/mojo/stdlib/builtin/simd/SIMD/)) **[S]** | `SIMD[DType.float32, 4]` (scalars = `SIMD[T, 1]`) | Power-of-2 compile-time `size` | `v[i]`, `v.shuffle[...]()`, `v.reduce_add()` | `SIMD[DType.bool, N]` + `mask.select(a, b)` | Yes (MLIR lowers) | Unifying scalar and vector syntax is slick for AI, but `DType` metaprogramming is alien to TS developers. |
| **ISPC** ([Pharr 2018](https://pharr.org/matt/blog/2018/04/29/ispc-retrospective)) **[V]** | Implicit `varying float` across gang (`uniform` vs `varying`) | Target-selected gang width (`4/8/16`) | Implicit per-instance; short vectors (`float<4>`) bolted on | Implicit execution mask (`if` predication) | Yes (compiles to target width) | SPMD handles control flow well, but Pharr's retrospective regrets lacking clean explicit vectors alongside SPMD **[V]**. |
| **Halide** ([PLDI 2013](https://people.csail.mit.edu/jrk/halide-pldi13.pdf)) **[S]** | Scalar algorithm `Func`; `.vectorize(x, 4)` in schedule | Schedule parameter | N/A (stencil indexing) | Predicated tail loops | Yes (CPU/GPU) | Decoupling algorithm from vector width is ideal for Layer 2 schedules, not for manual Layer 1 SIMD. |
| **Futhark** ([PLDI 2017](https://www.futhark-lang.org/publications/pldi17.pdf)) **[S]** | No explicit SIMD types; pure data-parallel arrays (`map`, `reduce`, `scan`) | Compiler-chosen | Array indexing | Pure functional `if` | Yes (sequential/multicore/GPU) | Uniqueness types enable race-free in-place updates for Layer 0 combinators; lacks Layer 1 manual SIMD escape hatch. |
| **WGSL** ([W3C WGSL](https://www.w3.org/TR/WGSL/)) **[V/S]** | `vec2<f32>`, `vec3<f32>`, `vec4<f32>` | Fixed small geometric widths (2, 3, 4) | `.x/.y/.z/.w`, `.xyzw` swizzles, `v[i]` | `vecN<bool>` + `select(f, t, cond)` (note arg order!) | N/A (GPU native) | Component swizzles (`v.xy`) are great for graphics/kernels; `vec3` padding pitfalls (16-byte alignment in storage buffers) must be handled carefully. |
| **JS `SIMD.js` (abandoned)** ([V8 blog](https://v8.dev/blog/simd), [TC39 archive](https://github.com/tc39/ecmascript_simd)) **[S]** | `SIMD.Float32x4` value objects, `SIMD.Float32x4.add(a, b)` | Fixed 128-bit | `extractLane`, `replaceLane`, `swizzle`, `shuffle` | `SIMD.Bool32x4` + `SIMD.Float32x4.select` | JS polyfill (very slow) | **Why it died:** dynamic JS semantics + GC/boxing made a huge static-method API bloat V8 by ~10% without reliable speedups; real users compiled C/C++ → Wasm **[S]**. Also: static method syntax (`Float32x4.add(a, b)`) is unreadable compared to operators. |
| **AssemblyScript `v128`** ([AS SIMD docs](https://www.assemblyscript.org/stdlib/builtins.html#simd)) **[S]** | Single untyped `v128` + intrinsics `f32x4.add(a, b)` | Fixed 128-bit (Wasm only) | `f32x4.extract_lane(v, 0)` | Untyped `v128` bitmask + `v128.bitselect` | **None** (requires Wasm SIMD128) | Zero type safety between `f32x4` and `i32x4` (both are `v128`!), verbose intrinsic calls, cannot compile to scalar Wasm for MCUs. |

### 2.2 Recommended OxiScript SIMD Surface, OxiIR Lowering & Determinism

#### Language Surface (Readable by TypeScript Developers)

Following `contracts.md` §3 and `grammar.md` §5, **`Simd<T, N>` requires zero new syntax**—it is a built-in generic **value type** (like a non-heap `struct` with copy semantics, always copied by value, never RC-allocated) plus a matching typed mask **`SimdMask<T, N>`** (or `Mask<N>` where lane bit width is inferred from element width in IR).

1. **Supported types (v1 — exact 128-bit Wasm SIMD128 shapes + portable aliases):**
   - `Simd<f32, 4>` (alias `f32x4`), `Simd<f64, 2>` (`f64x2`)
   - `Simd<i32, 4>` (`i32x4`), `Simd<u32, 4>` (`u32x4`), `Simd<i64, 2>` (`i64x2`), `Simd<u64, 2>` (`u64x2`)
   - `Simd<i16, 8>` (`i16x8`), `Simd<u16, 8>` (`u16x8`), `Simd<i8, 16>` (`i8x16`), `Simd<u8, 16>` (`u8x16`)
   - Restricting `N` in v1 to exact 128-bit combinations (`sizeof(T) * N === 16`) guarantees a 1:1 mapping to Wasm `v128` registers without hidden multi-register splitting. Non-128-bit widths (e.g., `Simd<f32, 8>` for future AVX2/GPU) can be accepted later by splitting in lowering without breaking source compatibility.
   - `N` is a compile-time integer literal type argument (type-checker extends `TypeExpr` to accept `INT` literals in builtin `Simd<T, N>` positions—already parsed by `LiteralType = STRING | INT | ...` in `grammar.md` §3.1 **[V]**!).

2. **Operators (same overflow/determinism rules as OxiScript scalars!):**
   - Arithmetic: `+`, `-`, `*`, `/` (floats only for `/`, since Wasm SIMD128 has no integer vector division; integer `/` on `Simd` is a clear `OXS-E0110` diagnostic suggesting scalar loop or multiply-shift).
   - **Critical overflow rule (`grammar.md` §1.3 & `ir.md` §3):**
     - On `Simd<f32, 4>` / `Simd<f64, 2>`: `+`, `-`, `*`, `/` are IEEE-754 lane-wise (no FMA contraction!).
     - On integer `Simd`: Wasm SIMD128 instructions `i32x4.add`, `i16x8.add`, `i8x16.add`, `i32x4.mul`, etc. are **wrapping** (`+%`, `-%`, `*%`), and `i8x16.add_sat_s/u`, `i16x8.add_sat_s/u` are **saturating** (`+|`, `-|`). Emitting a per-lane overflow trap check on plain `+` would destroy SIMD throughput (3–5 extra vector instructions + `any_true` branch per op). **Design recommendation [O]:** Keep plain `+`/`-`/`*` on integer `Simd` trapping (preserving the language's universal overflow invariant and matching scalar fallback) **and** teach developers + the placement report (`OXS-P0108`) to use `+%`/`-%`/`*%` and `+|`/`-|` for single-instruction Wasm SIMD integer math! (Alternative for Owner Question #2: forbid trapping `+` on integer `Simd` with a quick-fix suggesting `+%` or `+|`—even more honest, zero performance trap!)
   - Bitwise (integer `Simd` and `SimdMask`): `&`, `|`, `^`, `~`, `<<`, `>>` (uniform scalar shift amount, matching Wasm `i32x4.shl(v, i32)`).
   - Comparisons: **Do NOT overload `===`/`<` to return a `SimdMask`** (violates TS intuition where `a < b` is `bool`). Instead use explicit lane comparison methods: `a.eq(b)`, `a.ne(b)`, `a.lt(b)`, `a.le(b)`, `a.gt(b)`, `a.ge(b)` returning `SimdMask<T, N>`, plus `a.allEq(b): bool` / `mask.all(): bool` / `mask.any(): bool` / `mask.bitmask(): u32`.

3. **Construction, memory load/store, lane access, shuffles, reductions & conversions:**
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
   - **Conversions:** `v.toF32()` (`i32x4` ↔ `f32x4`), `v.saturating<i32>()` (Wasm `i32x4.trunc_sat_f32x4_s` — note ABI 1.0 banned scalar `trunc_sat`, so ABI 1.1 permits vector `trunc_sat` under the SIMD feature flag since it is part of fixed-width SIMD128 and deterministic!), `v.widenLow<i32>()` / `v.widenHigh<i32>()` (`extend_low`/`extend_high`), `v.narrowSat<i16>(hi)` (`narrow_i32x4_s/u`), `v.bitcast<u32>()` (canonicalises float NaNs before bitcasting to int lanes, per `ir.md` §6!).

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
   - Under `safety` profile, canonicalisation is additionally emitted after `f32x4.min`/`max` and before `VecStore` (note: in `wasmi 2.0.3` with `deterministic`, every float lane op is already canonicalised **[V]**, but Wasmtime/browsers need the `bitselect` sequence at `bitcast`/host boundaries).
3. **No Relaxed SIMD by default:** Wasm Relaxed SIMD (`f32x4.relaxed_madd`, `f32x4.relaxed_min`, `i8x16.relaxed_swizzle`) explicitly allows hardware-dependent rounding (FMA vs unfused) and out-of-range lane results ([WebAssembly Relaxed SIMD spec](https://github.com/WebAssembly/relaxed-simd)) **[V/S]**. It is forbidden under `default`, `realtime`, and `safety` profiles; it may only be enabled in a future explicit `@profile(fast_math)` or opt-in flag that is surfaced in the placement report and manifest.

---

## 3. Auto-Vectorisation, `@expect(simd)` and Placement Reporting

### 3.1 Prior Art & Why OxiScript Takes an Honest, Narrow Approach

* **LLVM Loop Vectorizer (LV) & SLP Vectorizer** ([LLVM Vectorizers docs](https://llvm.org/docs/Vectorizers.html), [VPlan](https://llvm.org/docs/Proposals/VectorizationPlan.html)) **[S]**: LV widens countable inner loops across iterations using Scalar Evolution (SCEV), alias analysis (emitting runtime overlap checks), and a VPlan cost model; SLP packs isomorphic straight-line ops within a basic block. Despite decades of engineering, empirical studies show GCC/ICC/Clang vectorise only **18%–30% of real application loops** (Maleki et al., PACT 2011, [MSR](https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers)) and **46%–54% of synthetic TSVC2 loops** ([arXiv:2502.11906](https://arxiv.org/html/2502.11906v1)) **[S]**, failing silently with compiler-internal excuses (`-fopt-info-vec-missed`: "relevant phi not supported").
* **Cranelift** ([cranelift.dev](https://cranelift.dev/)) **[S]**: Deliberately has **no loop auto-vectoriser** to preserve fast compilation and simplicity—meaning OxiScript cannot delegate auto-vectorisation to Wasmtime or browser JITs.
* **ISPC (SPMD)** ([Pharr 2018](https://pharr.org/matt/blog/2018/04/18/ispc-origins)) **[V]**: Eliminates auto-vectorisation guesswork by making the parallel contract explicit (`foreach` / program instances across SIMD lanes, handling control flow via masks). Its performance warnings (`gather`/`scatter` alerts) proved valuable, though users complained when warnings changed between debug and release builds ([ispc issue #2786](https://github.com/ispc/ispc/issues/2786)) **[S]**.
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

**Proposed append-only `OXS-P01xx` codes for Stage 8.3b (never modifying existing `P0101`/`P0102`):**

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

---

## 4. GPU Kernels (Stage 8.3b): Profile, Lowering, Buffers & Determinism

OxiScript's principle (`oxi-ui.md` Phase 8, `contracts.md` §3): **no separate shader language glued on through strings**. GPU kernels are ordinary OxiScript functions marked `kernel function` (or `@profile(kernel)`), type-checked by the same front end, lowered to the same OxiIR with a `#kernel` attribute, and compiled to **either** WGSL (for GPU execution) **or** CPU SIMD/scalar loops (for GPU-less targets, WebGL2/GLES3 fallbacks, and deterministic reference tests).

### 4.1 Prior Art Analysis

| System | Licence | Model & Architecture | Key Takeaway for OxiScript 8.3b |
|---|---|---|---|
| **CubeCL / Burn** ([github.com/tracel-ai/cubecl](https://github.com/tracel-ai/cubecl)) **[S]** | **MIT OR Apache-2.0** | `#[cube]` Rust functions expanded to a typed SSA CubeIR, then compiled on demand to WGSL, SPIR-V, CUDA, HIP, Metal, and CPU SIMD. | **Closest architectural analogue.** Proves a restricted typed subset + own SSA IR → WGSL/SPIR-V/CPU works in production for real ML (Burn). |
| **`rust-gpu`** ([transition blog](https://rust-gpu.github.io/blog/transition-announcement)) **[V]** | MIT / Apache-2.0 | Compiles full Rust via `rustc_codegen_spirv` to SPIR-V. Needs pinned nightly `rustc`; Embark archived it in Oct 2025. | **Warning:** hooking into a general host compiler's internals is fragile. Owning OxiIR + a small `#kernel` profile avoids this completely. |
| **Taichi** ([docs.taichi-lang.org](https://docs.taichi-lang.org/)) **[S]** | **Apache-2.0** | Python `@ti.kernel` restricted static subset JIT-compiled to Vulkan, Metal, CUDA, OpenGL, CPU. | Validates "restricted static subset of the host language" ergonomics for physics, particles, and graphics. |
| **Futhark** ([futhark-lang.org](https://futhark-lang.org/)) **[S]** | ISC | Pure functional array language → `GPUMem` IR → CUDA, OpenCL, and WebGPU (via C + `webgpu.h` + Emscripten). | Uniqueness types give race-free writable buffers without runtime alias checks; WebGPU backend confirms `webgpu` compute viability. |
| **Mojo GPU** ([Modular docs](https://docs.modular.com/mojo/manual/gpu/fundamentals/)) **[S]** | Apache-2.0 w/ LLVM exc. (compiler contributions restricted) **[S]** | Imperative kernels + `LayoutTensor` (compile-time strides/tiling) via MLIR to NVIDIA/AMD. | Explicit stride/tile tensor views (`View2D<T>`) eliminate error-prone flat index arithmetic. |
| **Triton** ([github.com/triton-lang/triton](https://github.com/triton-lang/triton)) **[S]** | **MIT** | Block-level Python DSL targeting PTX/AMDGCN via MLIR; programmer thinks in tiles rather than scalar threads. | Tile-level abstractions shine for GEMM/attention, but require heavy MLIR machinery; OxiScript starts with thread/workgroup SPMD (WGSL-native) and builds tiled GEMM as a library kernel. |
| **Slang** ([shader-slang.org](https://shader-slang.org/), [Khronos](https://www.khronos.org/news/permalink/khronos-group-launches-slang-initiative-hosting-open-source-compiler-contributed-by-nvidia)) **[S]** | **Apache-2.0 w/ LLVM exc.** | Modern shading language with modules, generics, interfaces, autodiff → SPIR-V/WGSL/MSL/HLSL/CUDA. | Great reference for generics-in-shaders and autodiff; too heavy (C++) to embed in OxiScript's wasm32 core. |
| **Halide GPU** ([PLDI 2013](https://people.csail.mit.edu/jrk/halide-pldi13.pdf)) **[S]** | MIT | Algorithm/schedule split targeting OpenCL/CUDA/Metal/WebGPU. | Confirms workgroup/tile parameters belong in declarative schedules/annotations (`@workgroup(16, 16, 1)`). |
| **`wgpu` compute examples** ([wgpu repo](https://github.com/gfx-rs/wgpu/tree/trunk/examples/features/src/boids)) **[V/S]** | **MIT OR Apache-2.0** | Reference WGSL compute shaders (boids particles, hello-compute, ray tracing) over `wgpu` 30. | Directly demonstrates `StorageBuffer`, ping-pong particle buffers, and compute-to-render handoff on one `wgpu::Device`. |

### 4.2 The `kernel` Profile (`#kernel` on Functions)

`kernel` is already reserved in `grammar.md` §1 & §5 (`FnModifiers::kernel`) and `oxiscript-abi/registry/codes.txt` (`OXS-E0500`) **[V]**. A `kernel function` is checked by `oxiscript-check` (8.T) and re-verified by `oxiscript-ir::verify`:

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
- **Bounds checking in WGSL:** Array accesses `buf[idx]` emit WGSL bounds clamping or guarded reads/writes (`if (idx < arrayLength(&buf)) { ... }`), matching WebGPU's robust buffer access and preventing out-of-bounds UB (remember: WGSL has no trap/abort instruction; OOB writes are dropped and OOB reads return zero, documented as the `#kernel` OOB rule vs CPU trap, OR optional atomic trap-flag buffer in debug builds!).
- **Offline validation:** In tests and `oxi build --check-gpu`, run `naga::front::wgsl::parse_str` + `naga::valid::Validator` (MIT OR Apache-2.0, already in `Cargo.lock` as `naga 30.0.2` **[V]**) so WGSL validity and uniformity are verified in CI even without a GPU!
- **Packaging kernels inside `.wasm`:** Each compiled `#kernel` function embeds its WGSL source string (plus binding metadata & workgroup size) in a read-only custom section **`oxi.kernels`** (or immutable static data) inside the `.wasm` module, alongside a **compiled CPU fallback function** in the Wasm code section! Thus one `.wasm` file runs identically on WebGPU, Vulkan/Metal/DX12, WebGL2-only browsers (CPU fallback), and ESP32 (CPU fallback).

### 4.4 Buffers (`Buffer<T>`), `host:gpu@1` IDL, Dispatch & Readback

Per `contracts.md` §3 and `abi.md` §5, GPU access is an ordinary capability **`host:gpu@1`** (zero ambient authority; scripts without `Gpu` in `main` cannot touch the GPU):

```text
// oxiscript-abi/idl/gpu.idl (proposed for Stage 8.3d)
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

    // Kernel compilation & dispatch (kernel_id references module's verified oxi.kernels entry)
    fn compile_kernel(wgsl: str, entry: str) -> result<Kernel>;
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
3. **Typed OxiScript wrapper (`std/gpu`):** Programmers never touch raw byte offsets or `u32` handles:
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
     // Async readback (callback in 8.3d; `await` sugar once 8.4 async lands):
     pos.download((result) => { ... })?;
   }
   ```
   When `gpu.caps() === 0` (WebGL2/GLES3/no-GPU host), `std/gpu`'s `dispatch` transparently runs the embedded **CPU SIMD/scalar fallback** for `integrate` on host-side shadow buffers, invoking the same `download` callback—so the exact same program runs everywhere!

### 4.5 Determinism Contract: CPU vs GPU Floating-Point

GPU hardware cannot guarantee bit-identical floating-point results with CPU IEEE-754 (`ir.md` §6):
- **WGSL specification accuracy** ([W3C WGSL Floating Point Evaluation](https://www.w3.org/TR/WGSL/#floating-point-evaluation)) **[S]**: `f32` `+`, `-`, `*` are correctly rounded, `/` is ≤ 2.5 ULP, `sqrt` ≤ 0.5–1 ULP depending on range, transcendentals (`sin`, `exp`, `log`) have bounded ULP tolerances, **subnormals may be flushed to zero (FTZ)**, **NaN payloads/signs are unspecified**, and backend drivers (Metal/DX12/Vulkan) may contract `a * b + c` to FMA unless explicitly inhibited (and even WGSL's `fma` vs `a*b+c` varies slightly across vendors).
- **OxiScript's honest contract (`ResultClass`):**
  1. **`@exact` kernels (integer-only `i32`/`u32` + wrapping/bitwise ops, no float math or unordered atomics):** Bit-for-bit identical across CPU scalar, CPU SIMD128, and every GPU vendor. Permitted under `deterministic` and `safety` profiles (still verifiable against CPU reference!).
  2. **`@approx` kernels (default for float `#kernel`s):** Defined as IEEE-754 `f32` within WGSL ULP bounds + FTZ tolerance (e.g., `abs_tol = 1e-6`, `rel_tol = 1e-5` in differential GPU-vs-CPU test harnesses). Allowed in `default` and `realtime` profiles; **rejected under `safety` profile** (`OXS-E0500`) unless explicitly routed to the CPU deterministic reference path.
  3. **Unordered reductions / atomics:** Floating-point parallel reductions on GPU are explicitly typed as `@approx`; integer atomic adds (`atomicAdd`) are commutative/associative so their final sum is `@exact`.

---

## 5. Shared Device Integration with `oxiterm-gpu` (Stage 5.2)

Per `docs/oxi-ui.md` ("GPU after OxiScript: one device, several users"), **one `Arc<GpuContext>`** must serve three co-users without CPU round-trips:
1. **`GpuSink` (Stage 5.2+):** DisplayList 2D renderer (SDF fills/borders, glyph atlas, scissor clips).
2. **`OxiScript` GPU kernels (Stage 8.3b):** `host:gpu@1` compute pipelines, storage buffers, and procedural textures.
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
    pub subgroups: bool,         // wgpu::Features::SUBGROUP
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
   - **If yes (`ComputeTier::Compute`):** Build `required_limits = wgpu::Limits::defaults().using_resolution(adapter_limits)` (clamping storage/compute limits down to `adapter_limits` if a mobile/GLES 3.1 adapter is slightly below WebGPU defaults, e.g. 4 storage buffers instead of 8). Optionally intersect `adapter.features()` with desired optional features (`SHADER_F16`, `SUBGROUP`, `TIMESTAMP_QUERY`) when `opts.enable_optional_compute_features` is true—never failing device creation if an optional feature is absent!
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
1. **Promote `with_error_scopes` to `pub` on `GpuContext`** (currently `pub(crate)` in `oxiterm-gpu/src/context.rs:238` **[V]**) and provide an async twin `with_error_scopes_async` (needed for browser wasm32 where `pollster::block_on` cannot block the JS event loop!).
2. **Enforce scope hygiene via RAII:** Wrap `ErrorScopeGuard`s inside a `Drop`-safe helper so even on panic/early return, pushed scopes are popped in LIFO order and never leak onto the thread-local stack.
3. **Lock down `on_uncaptured_error`:** Register **one** `GpuContext`-owned uncaptured error handler at device creation that logs + sets an atomic `uncaptured_error_count` (and sets `device_lost` if lost). Make `GpuContext::device()` documented as "never call `on_uncaptured_error` or leave unpopped error scopes" (or expose scoped helpers `ctx.run_scoped(|dev, queue| ...)` as the primary public API for `host:gpu` and 3D).
4. **Never expose raw `wgpu::Device` to scripts:** `host:gpu` pre-validates every handle, buffer size, alignment (`COPY_BUFFER_ALIGNMENT = 4`, uniform 16-byte alignment), usage flag, and aliasing rule in Rust *before* calling `wgpu`, and wraps every `wgpu` call (`create_shader_module`, `create_compute_pipeline`, `create_buffer`, `submit`) inside `ctx.with_error_scopes`. A malformed shader or OOM inside one script returns `Err(status)` to that script while leaving `GpuSink` and other scripts completely untouched.

### 5.5 Budgeting, Runaway Kernels, Timeouts & Device-Lost Recovery

Unlike CPU execution (`wasmi` fuel metering), **GPUs have no hardware instruction-level fuel counter**. A runaway compute shader (infinite loop or 10⁹ workgroups) triggers OS GPU watchdogs:
- **Windows WDDM TDR:** Resets the GPU driver after **2 seconds** by default (`TdrDelay = 2`), causing `DXGI_ERROR_DEVICE_HUNG` → `DeviceLost` ([Microsoft TDR docs](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery--tdr-)) **[S]**.
- **macOS Metal / Linux DRM/Vulkan (`amdgpu`/`i915`/`nouveau`):** Kernel GPU hangcheck similarly kills the context after ~2–10 s, poisoning the `wgpu::Device` for **all co-users** (including the UI renderer!).

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
| **WebGPU availability** | Broad on modern desktop/Pi4-5/Android | Chrome/Edge (Win/Mac/Android; Linux Intel Gen12+ & Wayland NVIDIA), Safari 26, Firefox Win/Mac (Linux Nightly, Android off) ([gpuweb status](https://github.com/gpuweb/gpuweb/wiki/Implementation-Status), `vulkan-acceleration.md` §10 **[V]**) → **CPU SIMD fallback is mandatory** for universal web reach! |

---

## 6. AI on Top: Minimal Kernels & Data Types for In-Browser Training and Inference

The owner's course goal (`docs/oxi-ui.md` Level 5 "Hero") is **"first AI in OxiScript": perceptron → live-trained digit recogniser (MNIST) → tiny language model**, running directly in the browser (and natively) without pulling ML domain primitives into the language core. Everything below falls out of the exact `#kernel` + `Simd<T, N>` + `host:gpu@1` layers designed in §§2–5.

### 6.1 Prior Art in Browser & Rust WebGPU AI

| Project | Stack & Licence | Key Lessons for OxiScript |
|---|---|---|
| **WebLLM / MLC-AI** ([github.com/mlc-ai/web-llm](https://github.com/mlc-ai/web-llm)) **[S]** | Apache-2.0; TVM-compiled WGSL compute shaders + WebGPU | Proves 0.5B–3B parameter LLMs run interactively in browser tabs using 4-bit/8-bit packed `u32` weights, `shader-f16`, KV-cache buffers, and subgroup reductions; caches compiled pipelines to hide shader compile latency. |
| **Burn + CubeCL** ([github.com/tracel-ai/burn](https://github.com/tracel-ai/burn)) **[S]** | **MIT OR Apache-2.0**; Rust + `wgpu`/WGSL/SPIR-V | Demonstrates both **training** (reverse-mode autodiff in pure library code on top of tiled matmul/conv/reduction kernels) and **inference** in the browser over `wgpu`. |
| **Candle** ([github.com/huggingface/candle](https://github.com/huggingface/candle)) **[S]** | **MIT OR Apache-2.0**; Rust + Wasm SIMD128 (+ WebGPU efforts) | Clean minimal tensor library proving small models (MNIST, Whisper tiny, SmolLM) are practical in Wasm SIMD128 alone when WebGPU is unavailable. |
| **ONNX Runtime Web** ([onnxruntime.ai](https://onnxruntime.ai/docs/tutorials/web/)) **[S]** | **MIT**; WebGPU EP + Wasm SIMD128 (XNNPACK) fallback | Key architectural lesson: **WebGPU beats Wasm SIMD by 5×–10× on large matmul/transformers, while Wasm SIMD beats WebGPU on tiny models/batches (< 10 ms) due to GPU dispatch & readback latency** ([benchmark analysis](https://onnxruntime.ai/docs/tutorials/web/)) **[S]**. OxiScript's dual CPU-SIMD + WebGPU fallback matches this automatically. |
| **Transformers.js** ([huggingface.co/docs/transformers.js](https://huggingface.co/docs/transformers.js)) **[S]** | Apache-2.0; ONNX Runtime Web | Shows progressive feature detection (`webgpu` + `f16` → `webgpu` `f32` → `wasm-simd` → `wasm-scalar`). |

### 6.2 Minimal Kernel Primitives Needed (Only 7 Kernel Families)

Every course milestone (perceptron → MNIST MLP/CNN live training → tiny GPT-style LM with 100k–10M params) decomposes into **seven small `#kernel` primitives**, all expressible in Stage 8.3d OxiScript without language changes:

| # | Kernel Family | Required Language / WGSL Features | Used By |
|---|---|---|---|
| **1** | **Elementwise & fused activations** (`add`, `mul`, `relu`, `gelu`, `silu`, `add_bias`, residual add, plus backward `d_relu` / `d_gelu`) | 1D `BufferView<f32>`, `globalId().x`, bounds check | Perceptron, MLP, CNN, Tiny LM |
| **2** | **Tiled Matrix Multiply (`GEMM` / `GEMV`)** (`C = A @ B`, `A^T @ B`, `A @ B^T`) | 2D dispatch, **workgroup shared memory** (`@shared let tileA: FixedArray<f32, 256>` → WGSL `var<workgroup> tileA: array<f32, 256>`), **`kernel.barrier()`** (`workgroupBarrier()`), `Simd<f32, 4>` vectorised loads (`vec4<f32>`) | MLP forward & backprop, Attention projections, LM linear layers |
| **3** | **2D Convolution (`conv2d`)** | Either **im2col + GEMM** (zero extra complex kernel!) or direct 2D stencil kernel with shared-memory halo tile | Digit recogniser (CNN), image filter benchmark |
| **4** | **Row-wise Reductions (`softmax`, `layernorm`, `rmsnorm`, `cross_entropy_loss`)** | Workgroup shared-memory tree reduction (`workgroupBarrier()`), optional **`kernel.subgroupAdd(x)`** when `GpuCapabilities::subgroups` is true | Digit recogniser loss, Transformer attention & normalization |
| **5** | **Scaled Dot-Product Attention** (`softmax(Q @ K^T / sqrt(d)) @ V`) | Composed from kernels #2 + #4 (causal mask in elementwise step); single-pass FlashAttention-style tiled kernel optional later | Tiny LM (Hero level) |
| **6** | **Optimizer step (`sgd_step`, `adamw_step`)** | In-place elementwise update on `inout BufferView<f32>` (`w`, `m`, `v`) with `grad` + scalar hyperparams (`lr`, `beta1`, `beta2`) | Live browser training of digit recogniser & tiny LM! |
| **7** | **Packed Dequantization + MatVec** (`u32`-packed `i8`/`u4` weights × `f32`/`f16` activations) | Bitwise shifts/masks on `u32` (`extractBits` / `(packed >> shift) & 0xFF`), `Simd<i32, 4>` / `dot4I8Packed` (optional) | Quantised inference in browser & ESP32-S3 (`esp-nn` parity) |

**Critical design requirement uncovered here:** Stage 8.3d **must** include two `#kernel`-profile intrinsics from day one—**workgroup shared memory** (`@shared` fixed-size array inside a `kernel function`) and **`kernel.barrier()`** (`workgroupBarrier()`)—because naive global-memory GEMM is 10×–30× slower than tiled shared-memory GEMM on every GPU **[S]**. Both map 1:1 to standard WebGPU/WGSL core (no extensions required!).

### 6.3 Data Types Matrix for AI (`f32`, `f16`, `bf16`, `i8`/`u8`)

| Type | CPU Wasm SIMD128 | WebGPU / WGSL | OxiScript Design Decision |
|---|---|---|---|
| **`f32`** | Native (`f32x4`) **[V]** | Mandatory core **[V]** | **Universal baseline** for training (weights, gradients, optimizer moments) and default inference. Always works without extensions. |
| **`f16`** | Emulated via `f32` conversion (no Wasm `f16x8` in fixed-width SIMD128) | Optional **`shader-f16`** (`enable f16;`), widely shipped in Chrome ≥ 120, Safari 26, modern Vulkan/Metal/DX12 ([Chrome blog](https://developer.chrome.com/blog/new-in-webgpu-120)) **[S]** | **Include as an explicit storage/compute scalar `f16` (or `half`) in Stage 8.3e**, guarded by `gpu.caps() & CAP_F16` (CPU fallback converts via `f32` using IEEE-754 half conversion, e.g., `half` crate algorithm). Cuts VRAM & bandwidth 2× for tiny LM inference. |
| **`bf16`** | No native Wasm support | **NOT in WGSL** (only provisionally reserved; no browser implements native `bf16` **[S]**) | **Do NOT add as a core language primitive.** If loading `bf16` weights from external checkpoints, provide a 3-line stdlib `u16 ↔ f32` bitshift unpacker (`f32::from_bits(u32(b) << 16)`) inside `#kernel`! |
| **`i8` / `u8` + `i32` accum** | Native (`i8x16`, `i16x8`, `i32x4.dot_i16x8_s`) **[V]** | No native `array<i8>` storage in WGSL **[V]**; pack 4×`i8` per `u32` and unpack via shifts (or WGSL `unpack4xI8` / `dot4I8Packed` in newer browsers **[S]**) | **Support in `std/tensor` via packed `Buffer<u32>`** (`pack4xI8` / `unpack4xI8` helpers in `std/kernel`). Essential for 1–3B tiny LMs in browser VRAM and matches ESP32-S3 PIE `int8` kernels (`esp-nn`). |

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

### 7.2 Staged Delivery Plan Table

| Stage | Scope | Measurable Gate | Key Risks & Mitigations | Depends On |
|---|---|---|---|---|
| **8.3a: Explicit SIMD + Wasm SIMD128 + Scalar Fallback** | Add `Simd<T, N>` & `SimdMask<T, N>` builtin value types to `oxiscript-check`; add `Ty::Vec`/`Ty::VecMask` + vector ops to `oxiscript-ir` + `verify`; add `lower_simd_to_scalar` pass; emit Wasm `v128` (`ABI 1.1`, `features simd128`) in `oxiscript-wasm` (`wasm-encoder` 0.263 already supports SIMD128 **[V]**); enable `wasmi/simd` in `oxiscript-runtime` with `wasm_simd(true)`, `wasm_relaxed_simd(false)`; add `FieldKind::Inline` vector layout support. | (1) All 10 `Simd<T, N>` types compile to both `simd128` and `scalar` Wasm and pass `wasmparser` validation. (2) Differential fuzz test (≥ 5,000 generated vector expressions incl. NaN, `-0.0`, `Inf`, min/max, overflow traps, bitcast) produces **bit-identical results** across `wasmi` SIMD128, `wasmi` scalar, and Wasmtime SIMD128. (3) Benchmark suite (§8) shows **≥ 2.5×** speedup for `f32x4` matmul/SAXPY in Wasmtime/browser vs scalar, and zero regression on non-SIMD benchmarks in `wasmi 2.0.3`. (4) BAD ACTOR pass (out-of-bounds `Simd.load`, misaligned/overflowing indices, NaN payload leak via `bitcast`, fuel exhaustion). | *Risk:* Wasm local count limit (`LIMIT_LOCALS = 12_000`) hit faster when `lower_simd_to_scalar` expands 1 `v128` into 16 `i8` scalars. *Mitigation:* add basic Wasm local reuse (liveness-based free list) in `oxiscript-wasm` or keep scalar-fallback loops compact. | 8.L (done), Benchmarks harness (§8) |
| **8.3b: Auto-Vectoriser v0 + Placement Report + `@expect(simd)`** | Enable `parallel for` syntax (un-reserve `For*::parallel` in `oxiscript-syntax`); implement natural-loop + dependence/alias analyser on OxiIR SSA for shapes **S1–S4** (§3.2); emit `OXS-P0103..P0112` remarks into `placement.json`; enforce `@expect(simd)` on `assertable yes` codes (compile error with source span + fix); emit positive `OXS-P0110` for LSP inlays. | (1) Corpus of ≥ 25 loops (maps, zips, reductions, branchless `?:`, alias-guarded loops, plus negative tests: loop-carried deps, calls, allocs, trapping int `+`, non-associative float sums) matches expected `OXS-P01xx` codes 100% identically in debug and release. (2) `@expect(simd)` fails compilation with exact code on every negative case and passes on every S1–S4 case. (3) Auto-vectorised SAXPY, image filter, and SoA particle step achieve ≥ 80% of hand-written `Simd<f32, 4>` throughput (Wasmtime/browser). (4) Differential fuzzing verifies bit-identical outputs vs unvectorised loop. (5) BAD ACTOR pass (aliasing arrays passed to same loop, integer overflow on `n`, empty/1-3 element tails, NaN in branchless `?:`). | *Risk:* False aliasing or subtle trap reordering breaks sequential semantics. *Mitigation:* Conservative v0—never vectorise across any `Op::may_trap()` inside the body; require explicit runtime `a !== b` pointer guard whenever arrays aren't provably distinct; verify every case differentially. | 8.3a, 8.S4 (or shared `Remark` type) |
| **8.3c: `GpuContext` Compute Upgrade & Isolation (`oxiterm-gpu`)** | Add `ComputeTier` (`GraphicsOnly` vs `Compute`), `GpuCapabilities`, and `GpuContext::new_negotiated[_async]` to `oxiterm-gpu`; expose `pub with_error_scopes[_async]` with RAII scope guards and locked-down uncaptured-error accounting; add shared `GpuTextureRegistry` + `image_pipeline` (`Rgba8Unorm` textured quad) for zero-copy texture display; verify `wasm32` build (`WebGPU` + `WebGL2`). | (1) On Vulkan/lavapipe CI, `new_negotiated` negotiates `ComputeTier::Compute` and runs a smoke compute shader writing into an `Rgba8Unorm` storage texture that `GpuSink` draws with **zero CPU readback**, matching a PNG golden. (2) Forced `GraphicsOnly` mode reproduces all Stage 5.2 PNG goldens unchanged. (3) BAD ACTOR pass (co-user leaking error scopes, concurrent threads, OOM buffer creation, device destruction mid-compute, recovery via context recreation) passes with 0 panics and 0 leaked errors across subsystems. | *Risk:* Browser `wasm32` `async` error scopes cannot use `pollster::block_on`. *Mitigation:* Keep `with_error_scopes_async` native-first and use `pollster::block_on` only in native sync wrappers. | 5.2 (done) |
| **8.3d: `kernel` Profile + `oxiscript-wgsl` + `host:gpu@1`** | Un-reserve `kernel function` & `@workgroup(...)` / `@shared` / `@max_iters(...)`; enforce `#kernel` profile rules in `oxiscript-check` & `oxiscript-ir::verify` (`OXS-E0500`, plain-data `std430` layout checks, uniformity check for `kernel.barrier()`); build `oxiscript-wgsl` (OxiIR → WGSL text + mandatory loop-fuel counter); embed `oxi.kernels` custom section + CPU fallback in `.wasm`; implement `host:gpu@1` provider (`GpuArena` quotas, async `OnReadbackReady` + out-buffer chunking, dispatch caps, CPU fallback when `caps() == 0`); emit `OXS-P03xx` transfer remarks and `@expect(gpu)` / `@expect(no_transfer)`. | (1) Every emitted WGSL kernel validates cleanly with `naga 30.0.2` offline in CI (0 errors). (2) Particle physics (100k particles), 2D Gaussian blur (zero-copy texture → `GpuSink` PNG), and tiled 256×256 `f32` GEMM run on Vulkan/lavapipe **and** CPU fallback, agreeing within `@exact` (0 ULP int) or `@approx` (`1e-5` rel tol float). (3) BAD ACTOR pass (infinite loop kernel stopped by static `_fuel` counter in < 50 ms without TDR; OOB buffer index; aliasing read+write buffer rejected; VRAM quota exceeded returns `Err`; hot reload frees all GPU buffers with 0 VRAM leak). | *Risk:* WGSL uniformity analysis rejections confuse users; `std430` padding mismatches between host arrays and WGSL structs. *Mitigation:* Check uniformity & `std430` layout in `oxiscript-check` with clear source diagnostics; keep `naga` validation in CI tests. | 8.3a, 8.3c, 8.M (linking) |
| **8.3e: AI Demo Kernels (`std/tensor` + Course Level 5)** | Add optional `f16` storage/compute support (`CAP_F16` / `enable f16;`), optional `subgroupAdd` fast path (`CAP_SUBGROUPS`), and `pack4xI8`/`unpack4xI8` helpers; write `std/tensor` entirely in OxiScript (7 kernel families of §6.2: elementwise/activations, tiled GEMM, im2col/conv2d, softmax/layernorm, scaled dot-product attention, SGD/AdamW backward pass, packed int8 matvec); build browser + native demos for MNIST digit recogniser (live training) and tiny char-level LM. | (1) MNIST 784→128→10 MLP trains live in browser (WebGPU & Wasm SIMD128 fallback) to ≥ 95% test accuracy in < 10 s (WebGPU) / < 60 s (SIMD128) with differential loss check against CPU scalar. (2) Tiny LM (e.g., 100k–1M params, 2-layer Transformer) generates tokens interactively in browser at ≥ 20 tok/s (WebGPU) / ≥ 5 tok/s (Wasm SIMD128) **[P target]**. (3) Zero ML concepts added to core grammar or compiler. (4) BAD ACTOR pass (NaN weights/gradients, huge tensor dims overflowing `u32` byte size, zero-length batch). | *Risk:* Shader compile cold-start latency on first browser load; float precision drift in `f16` training. *Mitigation:* Keep training in `f32`, use `f16`/`i8` for inference only; cache compiled `ComputePipeline`s per kernel hash in `GpuArena`. | 8.3b, 8.3d |

### 7.3 Permissive-Only Libraries to Reuse (Zero GPL/LGPL)

| Need | Crate / Library | Licence (Verified) | Where Used | Notes |
|---|---|---|---|---|
| **Wasm SIMD128 emission & validation** | `wasm-encoder 0.263`, `wasmparser 0.263` | Apache-2.0 WITH LLVM-exc. OR Apache-2.0 OR MIT **[V, `Cargo.toml`]** | `oxiscript-wasm` (core) | Already in `Cargo.lock`; full `v128` opcode support verified **[V]**. |
| **Interpreted Wasm SIMD128** | `wasmi 2.0.3` (`features = ["simd", "deterministic"]`) | MIT OR Apache-2.0 **[V, `Cargo.toml`]** | `oxiscript-runtime` | Already in `Cargo.lock`; zero overhead on non-SIMD code **[V]**. |
| **GPU abstraction (Vulkan/Metal/DX12/WebGPU/GLES3)** | `wgpu 30.0.2`, `bytemuck 1.25.2`, `pollster 1.0.3` | MIT OR Apache-2.0 (bytemuck: Zlib/Apache-2.0/MIT) **[V]** | `oxiterm-gpu`, `host:gpu` | Already in `Cargo.lock`. |
| **Offline WGSL validation (CI / `oxi check`)** | `naga 30.0.2` (`features = ["wgsl-in"]`) | MIT OR Apache-2.0 **[V, `Cargo.toml`]** | `oxiscript-wgsl` (dev/toolchain only—NOT in browser wasm32!) | Already in `Cargo.lock`; validates emitted WGSL + uniformity without a GPU. |
| **Wasm-to-C SIMD128 AOT (32-bit/SBC)** | `w2c2` + `SIMDe` | `w2c2`: MIT **[V]**; `SIMDe`: MIT ([simde LICENSE](https://github.com/simd-everywhere/simde/blob/master/COPYING)) **[S]** | Toolchain (8.2/8.3a AOT) | Header-only C99; maps `v128` to NEON/SSE/AVX/RVV. |
| **IEEE `f16` host conversions (8.3e)** | `half 2.5` | MIT OR Apache-2.0 **[V, pulled by `naga` in `Cargo.lock`]** | `oxiscript-runtime` / `std/tensor` host | Already in `Cargo.lock`! |
| **Host CPU multi-threading (Layer 2 parallel, later)** | `rayon` | MIT OR Apache-2.0 **[S]** | Native host only (never in `wasm32` core) | Optional parallel execution of CPU fallback kernels on multi-core native hosts. |
| **E-graph algebraic simplification (optional, post-v0)** | `egg` / `egglog` | MIT **[S]** | Toolchain only (`docs/oxi-ui.md` table) | Defer until after 8.3b v0—do **not** block 8.3a/8.3b on e-graphs! |
| **ESP32-S3 PIE DSP/NN (optional device capability)** | `esp-dsp`, `esp-nn` | Apache-2.0 ([github.com/espressif/esp-dsp](https://github.com/espressif/esp-dsp)) **[S: confirm LICENSE file before adoption]** | ESP32-S3 firmware only | Used via `host:dsp` / `host:ml` capability, never compiled into portable core. |

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

Reference expectations below assume a typical modern mid-range desktop/laptop CPU (e.g., 4–8 core x86_64/Apple Silicon, single-threaded for Wasm benchmarks) and integrated or mid-range discrete GPU (e.g., Intel Iris Xe / AMD Radeon 780M / Apple M1–M3 / RTX 3060-class). On the repo's older **Intel Core i5-3450 (2012 Ivy Bridge, AVX1 only, no FMA, Mesa `llvmpipe`)** (`docs/oxi-ui.md` Stage 5.0 **[V]**), CPU SIMD128 numbers will be ~2×–3× lower in absolute GFLOP/s (still ~2.5×–3.5× relative speedup over scalar Wasm!) and `llvmpipe` is a CPU Vulkan rasterizer/compute emulator, not a hardware GPU.

| Benchmark | Problem Specification | Scalar Wasm (JIT: Wasmtime / Browser) | Wasm SIMD128 (JIT: Wasmtime / Browser) | `wasmi 2.0.3` (Interp Scalar → SIMD128) | WebGPU / `wgpu` Compute (Hardware GPU, Resident) | Primary Sources & Notes |
|---|---|---|---|---|---|---|
| **1. Matrix Multiply (`f32` GEMM)** | **Small:** 256×256 (`33.5M` FLOPs)<br>**Large:** 1024×1024 (`2.15G` FLOPs)<br>`C = A @ B`, tiled `16×16` workgroups on GPU; `4×4` micro-kernel on SIMD128 | **256×256:** ~15–40 ms (~1–2 GFLOP/s)<br>**1024×1024:** ~1.0–2.5 s (~1–2 GFLOP/s) **[P/S]** | **256×256:** ~4–12 ms (~3–8 GFLOP/s; **3×–4×** vs scalar)<br>**1024×1024:** ~250–600 ms (~3.5–8 GFLOP/s; **3×–4×** vs scalar) **[S/P]** | ~15×–30× slower than JIT (~300–800 ms for 256×256 scalar; ~1.5×–3× faster with SIMD128) **[P/U]** | **256×256:** ~0.1–0.5 ms (dispatch-limited)<br>**1024×1024:** **~2–15 ms** (**150–1,000+ GFLOP/s**; **20×–150×** vs Wasm SIMD128!) **[S: [WebGPU matmul](https://www.nuss-and-bolts.com/), [ORT Web](https://onnxruntime.ai/docs/tutorials/web/)]** | Classic compute-bound kernel; proves tiled shared memory (`@shared` + `barrier()`). |
| **2. Image Filter (5×5 Gaussian / 3×3 Sobel on RGBA8)** | **1080p** (1920×1080 = 2.07M px) and **4K** (3840×2160 = 8.29M px), RGBA8 input → RGBA8 output (separable 1D or 2D stencil) | **1080p:** ~40–90 ms<br>**4K:** ~160–400 ms (~20–50 MPix/s) **[S/P]** | **1080p:** ~10–35 ms<br>**4K:** ~40–210 ms (**3×–8×** vs scalar JS/Wasm; `u8x16`/`i16x8` widening shines!) **[S: [Wasm SIMD image filter bench](https://web.dev/articles/webassembly-simd)]** | ~500–1500 ms for 1080p scalar **[P/U]** | **1080p:** **< 1–3 ms** (zero-copy texture → `GpuSink`!)<br>**4K:** **~2–8 ms** (**10×–40×** vs Wasm SIMD128) **[S]** | Tests narrow `u8`/`i16` SIMD128 lanes AND zero-copy kernel-to-UI texture handoff (§5.6). Note: CPU readback of 4K RGBA8 (33 MB) adds ~3–8 ms—proving zero-copy matters! |
| **3. Particle Physics (Euler + Gravity / Boids)** | **10k** and **100k** `Particle { x, y, vx, vy: f32 }` (16 B each = 1.6 MB for 100k); 100 simulation steps; compares AoS vs SoA (`xs, ys, vxs, vys`) | **100k × 1 step:** ~0.5–1.5 ms (gravity) / ~30–100 ms (grid/boids)<br>**100k × 100 steps (gravity):** ~50–150 ms **[P]** | **100k × 100 steps (SoA):** ~15–45 ms (**3×–3.8×** vs scalar); AoS without deinterleave: ~1.5×–2× **[P/S]** | ~15×–25× slower than JIT **[P/U]** | **100k × 1 step:** **< 0.1–0.3 ms** (scales effortlessly to **1M+ particles at 60 fps** **[S: [wgpu boids](https://github.com/gfx-rs/wgpu/tree/trunk/examples/features/src/boids)]**) | Directly tests `contracts.md` §3 `Array<Particle>` inline structs, SoA vs AoS, and ping-pong GPU storage buffers. |
| **4. Small MLP (MNIST 784→128→10 Forward + Backprop)** | Batch size `B = 64`, `f32`: `FC1 (64×784 @ 784×128)` + ReLU + `FC2 (64×128 @ 128×10)` + Softmax/CrossEntropy + backward + SGD (`~26M` FLOPs/step; 1 epoch ≈ 937 steps) | **1 step (`B=64`):** ~8–25 ms<br>**1 epoch:** ~8–23 s **[P/S]** | **1 step (`B=64`):** ~2–7 ms (**2.5×–4×** vs scalar; matches TF Lite Wasm SIMD **1.7×–4.5×** **[S: [TensorFlow Wasm SIMD](https://blog.tensorflow.org/2020/09/supercharging-tensorflowjs-webassembly.html)]**)<br>**1 epoch:** ~2–7 s | ~150–400 ms/step (fine for 1-sample interactive perceptron demo, too slow for full epoch) **[P/U]** | **1 step (`B=64`, resident weights):** **~0.2–0.8 ms** (dispatch-bound at `B=64`; **~0.5–2 ms** at `B=512`, **5×–15×** vs Wasm SIMD128)<br>**1 epoch:** **< 1 s** **[S/P]** | Validates the Hero course goal (live training in browser) and exposes the small-batch crossover where Wasm SIMD128 beats WebGPU if batch size is `B = 1`! |

**Critical crossover insight for benchmarks [S/O]:** For tiny dispatches (`B = 1` MLP inference, or `< 2,000` particles), WebGPU's submit + readback overhead (~0.2–2 ms) makes **Wasm SIMD128 faster in latency than WebGPU** ([ONNX Runtime Web](https://onnxruntime.ai/docs/tutorials/web/) **[S]**). Keeping the placement report honest about transfer bytes (`OXS-P0301`) and measuring both resident and round-trip times prevents misleading benchmarks.

---

## 9. Open Questions for the Owner

Before starting Stage 8.3a implementation, six concrete calls need the owner's decision (recommended default listed first):

1. **Integer overflow on `Simd<i32, 4>` (`+` vs `+%`/`+|`):**
   - *Option A (Recommended):* Forbid trapping `+`/`-`/`*` on integer `Simd<T, N>` (compile error `OXS-E0110` with quick-fix suggesting `+%` wrapping or `+|` saturating), so every integer SIMD operator maps 1:1 to a single fast Wasm SIMD128 instruction with zero hidden per-lane overflow traps.
   - *Option B:* Allow trapping `+` on integer `Simd` (emitting 4–5 extra instructions + `any_true` trap check) and warn via `OXS-P0108`.
2. **Inline-element arrays (`Array<Particle>`) timing:**
   - Should inline plain-data struct elements in `Array<T>` (`contracts.md` §3 worked example) land in **Stage 8.3a** (unlocking particle/AoS benchmarks and zero-copy GPU uploads immediately) or wait for **Stage 8.3d** (keeping 8.3a strictly to primitive `f32[]`/`i32[]` arrays + SoA)? *Recommendation:* Keep 8.3a to primitive arrays (fast, clean slice), land inline plain-data struct arrays at the start of 8.3d when `std430` layout is needed for `Buffer<Particle>`.
3. **Timing of `8.S4` (placement report) vs `8.3b` (auto-vectoriser):**
   - `8.3b` emits `OXS-P0103..P0112` remarks and enforces `@expect(simd)`. Should `8.S4` land first so `8.3b` plugs into the finalized `placement.json` schema (recommended), or should `8.3a` introduce a minimal `Remark` struct in `oxiscript-abi` early?
4. **GPU OOB semantics in `#kernel` (`clamped/zero` vs `debug trap flag`):**
   - WebGPU/WGSL cannot trap mid-shader; OOB storage writes are dropped and OOB reads return 0. Should OxiScript's `#kernel` profile document clamping/zeroing as the defined GPU OOB rule (with an optional debug-mode atomic `oob_flag` buffer that reports `TrapKind::Bounds` after dispatch so dev/BAD ACTOR catches OOB bugs)? *Recommendation:* Yes—debug atomic `oob_flag` in dev builds, silent bounds clamp in release, documented explicitly in `ir.md` & `contracts.md`.
5. **Browser `wgpu` vs thin JS WebGPU bridge for `host:gpu@1`:**
   - In browser local mode (`oxiterm-web`, Phase 7.1), `oxiterm-core` + `oxiscript` already run in wasm32. Should `host:gpu@1` in the browser go through `wgpu 30` compiled to wasm32 (shares 100% Rust code with `oxiterm-gpu`, enables zero-copy `GpuSink` in the browser, but adds wasm bundle size—currently unmeasured **[U]**) or call `navigator.gpu` via thin `web-sys` bindings when Canvas2D is the UI sink? *Recommendation:* Measure `wgpu 30` (`features = ["webgpu"]`, `default-features = false`, **no `naga`** since we emit WGSL text!) wasm32 gzip size in a 1-day spike at the start of Stage 8.3c before committing!
6. **Hardware validation target for GPU benchmarks:**
   - The current dev box runs Mesa `llvmpipe` (software Vulkan) on an i5-3450. Will Stage 8.3c/8.3d hardware GPU benchmarks run on a hardware Vulkan/WebGPU machine (e.g., laptop iGPU/dGPU or Raspberry Pi 5 V3D) before closing Stage 8.3d's speedup gate?

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
  - TC39 `SIMD.js` (archived) & V8 Wasm SIMD history: https://github.com/tc39/ecmascript_simd ; https://v8.dev/blog/simd ; https://emscripten.org/docs/porting/simd.html
  - LLVM Auto-Vectorizers & VPlan: https://llvm.org/docs/Vectorizers.html ; https://llvm.org/docs/Proposals/VectorizationPlan.html
  - Maleki et al. (PACT 2011), *An Evaluation of Vectorizing Compilers*: https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers
  - TSVC2 modern compiler vectorisation study (2025): https://arxiv.org/html/2502.11906v1
  - Matt Pharr (ISPC): https://pharr.org/matt/blog/2018/04/18/ispc-origins ; https://pharr.org/matt/blog/2018/04/29/ispc-retrospective ; https://github.com/ispc/ispc/issues/2786
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
  - TensorFlow.js Wasm SIMD & WebGPU benchmarks: https://blog.tensorflow.org/2020/09/supercharging-tensorflowjs-webassembly.html ; https://web.dev/articles/webassembly-simd

