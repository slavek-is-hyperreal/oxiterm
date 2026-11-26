# Verification ledger: `oxiscript-simd-gpu-design.md` (Gemini draft)

Verifier: Claude (Opus), 2026-11-26. Method: every sourced claim, and every unsourced claim that drives a decision, was checked against the cited source or a primary substitute. Local facts were checked against the crate sources at the versions in this worktree's `Cargo.lock`, under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`, and against the repository. Web facts were checked against the cited page, or against the primary page when the cited one was dead or empty.

Status values:
* **VERIFIED**: the source says it.
* **PARTLY**: the gist is right but a number, scope or attribution is off.
* **WRONG**: the source contradicts it.
* **UNVERIFIABLE**: the source is dead, empty or blocked, or gives no figure.
* **NRC** ("not re-checked"): a prior-art fact (mostly licences) that matches what is widely known but that I did not re-open in this pass. No decision rests on one of these.

## Counts

| | Count |
|---|---|
| Claims checked (ledger rows B-G, excluding NRC) | 100 |
| VERIFIED | 64 |
| PARTLY (incl. 4 design gaps / inconsistencies) | 14 |
| WRONG | 10 |
| UNVERIFIABLE | 12 |
| NRC (not counted above) | 13 |

Section A lists the 10 findings that change the design. Each one also appears as a row in B-G, so it is not counted twice.

## A. Corrections that change the design (read first)

| # | Report section | Gemini claim | Finding | Evidence |
|---|---|---|---|---|
| A1 | §5.2 | Negotiate compute with `Limits::defaults().using_resolution(adapter_limits)`, "clamping storage/compute limits down to adapter limits" | **WRONG, and a bug.** `using_resolution` copies only `max_texture_dimension_{1d,2d,3d}`. Requesting `defaults()` on an adapter below the WebGPU defaults (a GLES 3.1 or mobile part with 4 storage buffers) makes `request_device` fail. Use `Limits::downlevel_defaults()` (compute, 4 storage buffers, 16 352 B workgroup memory) as the floor, then `.or_worse_values_from(&adapter.limits())`, or request exactly what the kernels need. | `wgpu-types-30.0.2/src/limits.rs:753-760` (`using_resolution`), `:931` (`or_worse_values_from`), `:557-569` (`downlevel_defaults`) |
| A2 | §5.2, §6.2, §7 8.3e | `subgroups: bool // wgpu::Features::SUBGROUP`, used as an optional fast path, implicitly in the browser too | **PARTLY / WRONG for the browser.** In wgpu 30, `SUBGROUP` is a native-only feature: "it remains a native-only feature in wgpu for now" (gfx-rs/wgpu#5555). Through wgpu on WebGPU there are no subgroups. `SHADER_F16` *is* a web+native feature. | `wgpu-types-30.0.2/src/features.rs:1130-1142` (SUBGROUP), `:1690-1700` (SHADER_F16 "web and native") |
| A3 | §5.4 (2) | "Enforce scope hygiene via RAII: wrap `ErrorScopeGuard`s inside a Drop-safe helper" | **PARTLY (redundant).** In wgpu 30, `push_error_scope` already returns an `ErrorScopeGuard` that pops on `Drop` and is `!Send`/`!Sync`. B-03 is already fixed on the sink side (`with_error_scopes`, fixes.md). **The real remaining hole:** `GpuContext::device()` is `pub` and returns `&wgpu::Device`. Any Rust co-user (3D, a future `host:gpu` provider) can therefore call `on_uncaptured_error`, which replaces one device-wide handler. That is B-03's second half, and only API design closes it: no raw `device()` for co-users, scoped helpers only. A guard can also be leaked with `mem::forget`. | `wgpu-30.0.2/src/api/device.rs:542-549, 957-985`; `wgpu-30.0.2/src/backend/wgpu_core.rs:1840-1843` (handler = single `Option`), `:1845-1858` (scopes keyed by `ThreadId`); `oxiterm-gpu/src/context.rs:238` (`pub(crate) with_error_scopes`), `:295` (`pub fn device`) |
| A4 | §1.1 (5) | "wasmi lowers relaxed-SIMD ops deterministically in software" | **PARTLY. It is deterministic *within wasmi*, but not the Wasm 3.0 deterministic profile.** `f32x4_relaxed_madd` = `Float::mul_add` = std `mul_add` / `libm::fma`, i.e. **fused**, even with the `deterministic` feature. The Wasm 3.0 spec says: "In the deterministic profile, the unfused behaviour is used." The relaxed min/max/swizzle/laneselect/trunc forwardings do match the profile. Consequence: `wasm_relaxed_simd(false)` is mandatory, not merely advisable, because a program using `relaxed_madd` would differ between wasmi and a profile-conforming engine. Worth an upstream issue. | `wasmi_core-2.0.3/src/simd.rs:1399-1402, 1889-1904`; `src/value.rs:472-475, 550-553, 710-713`; the translator and executor route the opcode there unconditionally, with no `deterministic` cfg (`wasmi-2.0.3/src/engine/translator/func/simd/visit.rs:1281-1290`, `executor/handler/exec/simd.rs:463-465`); spec: https://webassembly.github.io/spec/core/exec/numerics.html (relaxed operations, `frelaxed_madd`) |
| A5 | §1.1 (4), §1.4, Exec. summary | treVM shows that "interpreted Wasm SIMD can even regress speed" on MCUs: "3 of 4 boards (RP2350: 18.6 vs 13.2)" | **WRONG inference, PARTLY wrong number.** treVM measured **Wasmtime (probably the Pulley interpreter)** built with vs without SIMD support, running **scalar CoreMark**. That is the cost of *enabling the feature*, not of SIMD code. No-SIMD scored higher on **all 4 boards**, though marginally on ESP-WROOM-32 (11.6 vs 11.7). The ESP32-S3 was not tested, and wasmi's SIMD setting is not stated. For wasmi 2.0 the CHANGELOG says: "Enabling the `simd` crate feature no longer affects memory consumption or execution performance of non-`simd` Wasm code." The ESP32-S3 "SIMD off" default therefore becomes **[O]**: justified by flash size and handler code size, unmeasured, and to be benchmarked. | https://arxiv.org/html/2604.27570v1 Table I; https://raw.githubusercontent.com/wasmi-labs/wasmi/main/CHANGELOG.md (2.0.0) |
| A6 | §4.3, §9 Q4 | "WGSL has no trap; OOB writes are dropped and OOB reads return zero" | **WRONG (too strong).** WebGPU robust access lets an OOB load return a value from *any in-bounds location* (or zero, or a partially discarded dispatch). An OOB store may land at *another in-bounds location* or be discarded. Implementations differ: naga offers `Restrict`, `ReadZeroSkipWrite` and `Unchecked` (its `Default`), and the backend picks the policy (`naga-30.0.2/src/proc/index.rs:42-134`). For `@exact` CPU/GPU parity the emitter **must** insert its own explicit guards: these become mandatory, not optional. | https://kvark.github.io/webgpu-debate/RBA.html ; https://qianqians.github.io/2021/08/06/WebGPU%202.4.%20Out-of-bounds%20access%20in%20shaders (summary of the spec text) |
| A7 | §8.2, §6.1, §8 crossover | Benchmark cells sourced to ORT Web, nuss-and-bolts.com, web.dev, wasmi-benchmarks | **UNVERIFIABLE.** The ORT Web tutorial has no performance numbers at all. nuss-and-bolts.com is a Substack landing page; the article is https://zanussbaum.github.io/blog/2024/webgpu ("1 TFLOP+"), with GPU and sizes not visible. web.dev/articles/webassembly-simd returns 404. wasmi-benchmarks publishes no numbers and has no SIMD case. These cells become **[P]**. Verified replacements: TF.js Wasm SIMD gives **1.7-4.5x** over plain Wasm (plus 1.8-2.9x from threads); MediaPipe hand tracking went from 14-15 to 38-40 FPS with SIMD; WebGPU per-dispatch API cost is **24-36 µs (Vulkan) and 32-71 µs (Metal)** natively, Chrome 33-67 µs, Safari 32 µs, **Firefox ~1040 µs**; naive single-op measurements overestimate by about 20x. | https://blog.tensorflow.org/2020/09/supercharging-tensorflowjs-webassembly.html ; https://v8.dev/features/simd ; https://arxiv.org/html/2604.02344v1 (abstract, Table 6) |
| A8 | §1.3, §1.4 | Hand-coded PIE gives "5x-44x" [S: esp-dsp] | **PARTLY.** The esp-dsp README gives no speedups. The esp-nn README gives per-kernel speedups on ESP32-S3 over ANSI C of **3.5x-14.2x** (conv 1x1 14.24x, FC 5.32x, elementwise mul 3.51x). Model-level `invoke()` gains are 42-104x (Person Detection 2300 ms to 54 ms), but those include more than SIMD. Both libraries are **Apache-2.0** (VERIFIED from the READMEs). | https://github.com/espressif/esp-nn ; https://github.com/espressif/esp-dsp |
| A9 | §5.7, §9 Q5, §4.3 | Emit WGSL, so "no naga in the WASM bundle" | **PARTLY.** True only for a **WebGPU-only** wasm32 build (`webgpu` feature; `ShaderSource::Wgsl` goes straight to `GPUShaderModuleDescriptor`). The report's own tiered design keeps a **WebGL2 fallback in the browser**. That needs the `webgl` feature, which pulls in `wgpu-core` + `wgpu-hal` (GLES) and therefore naga (WGSL in, GLSL out). Measure the bundle size for both variants. | `wgpu-30.0.2/Cargo.toml` `[features]` (`webgl = ["web","wgpu-core/webgl","dep:wgpu-hal",…]`, `webgpu = ["web","naga?/wgsl-out",…]`); `src/backend/webgpu.rs:1972-1981` |
| A10 | §4.4 IDL | `fn compile_kernel(wgsl: str, entry: str)` | **Design hole (not a fact).** A guest-supplied WGSL string bypasses the OxiScript checker: no kernel profile, no static fuel counter, no `std430` checks. The comment says "kernel_id references module's verified oxi.kernels entry", but the signature takes arbitrary text. Compile only from the module's own `oxi.kernels` section (a `kernel_index: u32`), validated by the host with naga before `create_shader_module`. | report §4.4 |

## B. Ledger: SIMD in engines (task Q1)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| B1 | `wasmi` in Cargo.lock is 2.0.3 | VERIFIED | `Cargo.lock` (`wasmi`, `wasmi_core`, `wasmi_ir` = 2.0.3) |
| B2 | simd since 0.43.0, relaxed-simd since 0.44.0 | VERIFIED | `wasmi-2.0.3/README.md:64-65`; also features.json ("Wasmi": 0.43 / 0.44) |
| B3 | One feature `simd = ["wasmi_core/simd","wasmi_ir/simd","wasmparser/simd"]`, disabled by default | VERIFIED (line number wrong) | `wasmi-2.0.3/Cargo.toml:83` (report says :69); `src/lib.rs:69` "Disabled by default" |
| B4 | The feature enables both SIMD and relaxed SIMD | VERIFIED | `wasmi-2.0.3/src/engine/config.rs:94-95` |
| B5 | `Config::wasm_simd` / `wasm_relaxed_simd` gated on `simd`+`validate`; both default on | VERIFIED | `config.rs:323-338`, `:94-95` |
| B6 | `oxiscript-runtime` enables `validate, deterministic, stable, auto-dispatch` (no `simd`) | VERIFIED | `oxiscript/oxiscript-runtime/Cargo.toml:18` |
| B7 | `V128` occupies 2 cells | VERIFIED | `wasmi-2.0.3/src/engine/translator/utils.rs:12-18` |
| B8 | wasmi 2.0: no cost on non-SIMD code | VERIFIED (was [S]) | CHANGELOG 2.0.0 quote (A5) |
| B9 | wasmi 1.x: `simd` regressed CoreMark by about 8% | PARTLY | CHANGELOG 0.43/0.44: "may introduce Wasm execution overhead, increase memory consumption"; **no 8% figure found** |
| B10 | No `core::arch` intrinsics; lane ops are plain array loops | VERIFIED | `wasmi_core-2.0.3/src/simd.rs:243-270` (`lanewise_*`); no `core::arch` in file |
| B11 | Interpreted SIMD 1.5-3.5x over interpreted scalar | UNVERIFIABLE ([P], unmeasured) | none; benchmark in 8.3a |
| B12 | wasmi about 10-30x slower than JIT [S: wasmi-benchmarks] | UNVERIFIABLE | repo publishes no numbers |
| B13 | SIMD float ops go through `wasm::f32_*` and canonicalise NaN under `deterministic` | VERIFIED (with caveat) | `simd.rs:938-955`, `value.rs:402-422, 617-648`. Caveat: `neg/abs` (sign-bit ops, `simd.rs:974-977`) are not canonicalised, which is allowed by the spec. |
| B14 | `f32x4_relaxed_min` forwards to `f32x4_min` | VERIFIED | `simd.rs:1895` |
| B15 | wasmi relaxed SIMD is "deterministic" | PARTLY | see A4 (fused madd) |
| B16 | Wasmtime: SIMD and relaxed SIMD Tier 1 | VERIFIED | https://docs.wasmtime.dev/stability-wasm-proposals.html |
| B17 | Cranelift: no 32-bit backend; Pulley elsewhere | VERIFIED (repo research) | `docs/research/oxiscript-language.md:297` citing the Wasmtime platform page |
| B18 | Cranelift performs no loop auto-vectorisation [S: cranelift.dev] | UNVERIFIABLE from the cited page | cranelift.dev says only "does not have the aggressive optimization stance … of LLVM or gcc" and avoids "advanced alias analysis". The recommendation (emit v128 ourselves) holds regardless. |
| B19 | Gap: Wasmtime has `Config::relaxed_simd_deterministic` and `cranelift_nan_canonicalization` | ADDED (VERIFIED) | https://docs.rs/wasmtime/latest/wasmtime/struct.Config.html |
| B20 | SIMD128: Chrome 91, Firefox 89, Safari 16.4 | VERIFIED | features.json; v8.dev/features/simd ("available by default from Chrome 91") |
| B21 | Relaxed SIMD in all three engines (Chrome 114, Firefox 145, Safari) | **WRONG for Safari** | features.json: Safari "Requires JavaScriptCore flag `useWebAssemblyRelaxedSIMD`"; Chrome 114, Firefox 145 VERIFIED. The repo's `oxiscript-language.md:320,341` carries the same error (it cites BCD). |
| B22 | wasm2c supports SIMD via SIMDe | VERIFIED | WABT PR "wasm2c: add SIMD support (#2119)" incorporates simd-everywhere; `third_party/simde` in wabt; features.json wasm2c simd "1.0.33" |
| B23 | w2c2 SIMD needs C99 + SIMDe; MIT | VERIFIED | https://github.com/turbolent/w2c2 README |
| B24 | Relaxed SIMD in wasm2c/w2c2 unknown [U] | VERIFIED as unknown | features.json lists no relaxedSimd for wasm2c |
| B25 | SIMDe is MIT | NRC | |
| B26 | WAMR SIMD on x86-64/AArch64 | NRC | |
| B27 | ESP32-S3 PIE: 128-bit q0-q7, `ee.*`, TIE-based | VERIFIED | https://developer.espressif.com/blog/2024/12/pie-introduction/ ; ESP32-S3 TRM |
| B28 | GCC/LLVM do not auto-vectorise into PIE; no intrinsics | PARTLY | community statement only (https://github.com/vroland/epdiy/discussions/289); no official Espressif toolchain doc found |
| B29 | SIMDe has no Xtensa PIE backend | NRC ([U] in report, correctly marked) | |
| B30 | esp-dsp / esp-nn Apache-2.0 | VERIFIED | READMEs |
| B31 | 5x-44x PIE speedup | PARTLY | A8 |
| B32 | treVM SIMD regression | WRONG inference | A5 |

## C. Ledger: language designs (task Q2)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| C1 | Rust `Simd<T, N>`, N = 1..64 powers of two, nightly | **WRONG (lane rule)**, rest VERIFIED | std docs: "`N` cannot be 0 and may be at most 64"; no power-of-two rule; still `portable_simd` #86656 |
| C2 | Rust comparisons via methods returning `Mask` (`simd_lt`) | VERIFIED | std::simd docs (`SimdPartialOrd::simd_lt`). This supports the report's `.lt()` choice. |
| C3 | C# `Vector128<T>` / `Vector<T>` | NRC | |
| C4 | Java Vector API JEP 489 | UNVERIFIABLE (openjdk.org returned 403) | |
| C5 | Zig `@Vector`: operators elementwise; `@splat`/`@reduce`/`@shuffle`/`@select` | VERIFIED (operators); `+%`/`+|` implied by "same builtin operators as their underlying base types", not named | ziglang.org docs, Vectors |
| C6 | Zig comparisons on vectors return a vector | VERIFIED. Note this is counter-evidence to the report's "overloading `<` to a mask violates intuition". It is a taste call, kept as [O]. | same |
| C7 | Mojo `SIMD[DType, size]` | NRC | |
| C8 | ISPC: Pharr "regrets lacking explicit vectors" | PARTLY | retrospective: a wish ("It would be nice to … support explicit vectors"), not a regret; [V] tag overstated |
| C9 | Halide / Futhark descriptions | NRC | |
| C10 | WGSL vec2/3/4, swizzles, `select(f, t, cond)` | VERIFIED (vector and scalar types); the argument order is NRC | WGSL TR (scalar types bool/i32/u32/f32/f16; no 8/16/64-bit ints, no f64) |
| C11 | SIMD.js abandoned; users moved to Wasm | VERIFIED | tc39/ecmascript_simd README ("removed from Stage 3 … WebAssembly … better vehicle"; implementing outside asm.js "creates a lot of complexity"); v8.dev/features/simd ("archived in favor of … WebAssembly") |
| C12 | SIMD.js bloated V8 by about 10% | UNVERIFIABLE | cited https://v8.dev/blog/simd is a 404; no source found |
| C13 | AssemblyScript untyped `v128` + `f32x4.add` intrinsics | NRC | |
| C14 | `LiteralType = STRING \| INT` already parses `Simd<f32, 4>` | PARTLY | `grammar.md:245-247` parses it, but the rule is annotated "(* discriminants only *)", so the checker must newly accept INT as a type argument of the builtin `Simd` (a checker change, not a grammar change) |
| C15 | `FieldKind::Inline{size,align,ptr_slots}` exists; `OBJECT_ALIGN = 8` | VERIFIED | `oxiscript-abi/src/layout.rs:23, 225-235` |
| C16 | `WasmTy`, `Ty`, `Op` non-exhaustive; Simd needs only a minor ABI bump | VERIFIED | `contracts.md` §3 (SIMD row) |
| C17 | ABI 1.0 bans scalar `trunc_sat`; ABI 1.1 may allow vector `trunc_sat` | VERIFIED (ban); the allowance is a proposal | `abi.md:5`. It needs an owner decision (added to open questions). |
| C18 | wasm-encoder 0.263 supports v128 | VERIFIED | `wasm-encoder-0.263.0/src/core/code.rs` (244 V128/I32x4/F32x4 matches); `oxiscript-wasm/Cargo.toml:12` |
| C19 | ir.md §6: NaN canonical at bitcast and host boundary; under `safety` also at store/min/max/conv; no FMA, no FTZ | VERIFIED | `docs/oxiscript/ir.md` §6 |
| C20 | Internal consistency: §2.2 recommends keeping trapping `+` on integer Simd; §9 Q1 recommends forbidding it | **WRONG (self-contradiction)** | report §2.2 vs §9. Corrected: one recommendation (Option A, forbid), supported by Rust `std::simd` integer ops wrapping. |
| C21 | Example code is valid OxiScript | **WRONG** | §4.4 `main(gpu: Gpu): void` uses `?` (needs a `Result` return, AGENTS.md rule 4) and an arrow closure `(result) => {...}` with no body; `Simd.splat<…>` is a static namespace call on a type, which the current stdlib does not have ("Nothing else is global"). Marked as illustrative pseudo-syntax. |

## D. Ledger: auto-vectorisation (task Q3)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| D1 | Maleki et al.: compilers vectorise 18-30% of real loops; "fail on 70-82%" | **WRONG** | Abstract: "45-71% of the loops in the synthetic benchmark … only a few loops from the real applications" (https://www.microsoft.com/en-us/research/publication/an-evaluation-of-vectorizing-compilers) |
| D2 | TSVC2 46-54% (arXiv 2502.11906) | VERIFIED | abstract: GCC 54%, ICX 50%, Clang 46% (x86) |
| D3 | LLVM LV/SLP/VPlan description | NRC (standard) | |
| D4 | ispc #2786: users complained when warnings changed between debug and release | **WRONG** | the issue is "Undefined behaviour warning that looks like a false positive?", not about debug vs release |
| D5 | OxiIR: block params, Cooper-Harvey-Kennedy dominators in verify.rs | VERIFIED | `oxiscript-ir/src/verify.rs:546` |
| D6 | Codes P0101 (assertable no), P0102 (yes), P0201 (yes), P0301 (no) | VERIFIED | `codes.txt:620-650` |
| D7 | `@expect(simd)` attaches to statements; reserved `parallel for` | VERIFIED | `grammar.md:276, 360, 376` |
| D8 | `OXS-E0110` = invalid operand types; `E0117` = invalid generic arguments | VERIFIED (codes exist). Using E0117 for a dynamic lane index is a stretch; a dedicated code is cleaner. | `codes.txt:193, 242` |
| D9 | Stage naming | **Inconsistent** | oxi-ui.md:651 names "8.3 SIMD/parallel" and "8.3b GPU kernels". The report §3.3 calls the vectoriser "8.3b", while §7 uses the task's 8.3a-8.3e. Mapping added in the report. |

## E. Ledger: GPU kernels (task Q4)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| E1 | `kernel` reserved; `FnModifiers::kernel`; `OXS-E0500` "construct forbidden by profile" | VERIFIED | `grammar.md:19,126`; `codes.txt:501` |
| E2 | `OXS-E0600` irreducible control flow; "Beyond Relooper" in `oxiscript-wasm/src/cfg.rs` | VERIFIED | `codes.txt:508`; `cfg.rs:2`, `lib.rs:8` |
| E3 | `LIMIT_LOCALS = 12_000` | VERIFIED | `oxiscript-wasm/src/lib.rs:93` |
| E4 | WebGPU `ShaderSource::Wgsl` passes the string straight through; SPIR-V / Naga sources need naga in wasm32 | VERIFIED | `wgpu-30.0.2/src/backend/webgpu.rs:1903-1991` (`spirv` → `naga::front::spv`, `naga-ir` → validate+transform, `wgsl` → `GpuShaderModuleDescriptor::new(code)`) |
| E5 | naga 30.0.2 MIT OR Apache-2.0; wgsl-in has a uniformity analysis; supports `enable f16`, `subgroups`, `packed_4x8_integer_dot_product` | VERIFIED | `naga-30.0.2/Cargo.toml:38`; `src/valid/analyzer.rs:4`; `src/front/wgsl/parse/directive/enable_extension.rs:132,140`; `language_extension.rs:17` |
| E6 | WGSL has no 8/16-bit ints and no 64-bit types; f16 behind `enable f16` | VERIFIED | WGSL TR scalar types; Chrome 120 blog (`enable f16;` required). Note: wgpu natively offers `SHADER_I16`, `SHADER_INT64`, `SHADER_F64` (features.rs:1071-1125), all native-only, so "rejected on GPU kernels" is right for portability. |
| E7 | WGSL f32: `/` at 2.5 ULP, FTZ allowed, NaN unspecified, fusion allowed "unless inhibited" | PARTLY | CTS fp_primer confirms FTZ modelling, and secondary sources confirm reassociation and fusion are permitted. I could not reach WGSL §15.7 through the fetch tool. **Correction:** WGSL offers **no** way to inhibit fusion or reassociation, so float kernels are inherently `@approx`. |
| E8 | Writable storage binding aliasing is rejected by WebGPU validation | NRC (WebGPU usage-scope rule) | |
| E9 | OOB semantics | **WRONG** | A6 |
| E10 | `HOST_PAYLOAD_MAX = 4096`; §5.2 out-buffer; §5.3 no re-entrancy; §5.4 callbacks carry at most one str/bytes in scratch | VERIFIED | `abi.md:148-158` |
| E11 | IDL syntax (`interface X "host:x" @1 { handle …; callback …; fn … -> result<…>; }`) | VERIFIED | `oxiscript-abi/idl/state.idl`; `abi.md:106-128` |
| E12 | `compile_kernel(wgsl: str…)` | Design hole | A10 |
| E13 | Prior art: CubeCL MIT/Apache; WGSL, SPIR-V, CUDA, HIP, Metal, CPU | VERIFIED | https://github.com/tracel-ai/cubecl ("dual licensed under either MIT or Apache 2.0"; CPU "lowers … to the platform's native SIMD") |
| E14 | rust-gpu: pinned nightly; "Embark archived it in Oct 2025" | PARTLY / **WRONG date** | the post is dated 2024-08-12 and describes a transition to community ownership ("Embark has decided not to transfer the existing repository"); nightly requirement VERIFIED; no archival in Oct 2025 is stated |
| E15 | Taichi Apache-2.0; Futhark ISC + WebGPU backend; Triton MIT; Slang Apache-2.0 w/ LLVM exc.; Halide MIT; Mojo licence | NRC | |
| E16 | wgpu boids / hello-compute examples, MIT OR Apache-2.0 | VERIFIED (licence) | `wgpu-30.0.2/Cargo.toml` licence |

## F. Ledger: shared device (task Q5)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| F1 | `GpuContextOptions::default()`: `Features::empty()`, webgl2 limits + `using_resolution` | VERIFIED | `oxiterm-gpu/src/context.rs:111-120, 195-199` |
| F2 | webgl2 limits: all storage/compute = 0 | VERIFIED | `limits.rs:644-661` |
| F3 | `set_device_lost_callback` + `OnceLock`; `is_device_lost` | VERIFIED (lines 213-225) | `context.rs:213-225` |
| F4 | `with_error_scopes` is `pub(crate)`, LIFO pop of Internal/OOM/Validation | VERIFIED | `context.rs:235-260` |
| F5 | Error scopes thread-local, uncaptured handler device-wide | VERIFIED | `wgpu_core.rs:1840-1858`; `device.rs:520-524` ("With the `std` feature enabled, this stack is thread-local. Without, this is global"); bad-actor.md B-03 |
| F6 | RAII guard needed | PARTLY (already in wgpu) | A3 |
| F7 | `Limits::defaults().using_resolution` clamps | **WRONG** | A1 |
| F8 | `defaults()` values (8 storage buffers, 4 storage textures, 16 KiB workgroup memory, 256 invocations, 65 535 workgroups per dimension) | VERIFIED | `limits.rs:421-458` |
| F9 | `SUBGROUP` optional feature | PARTLY (native only) | A2 |
| F10 | `TIMESTAMP_QUERY` is a WebGPU feature | VERIFIED | `features.rs:61, 1663` |
| F11 | `PollType::Wait { timeout }` exists; no effect on WebGPU | VERIFIED | `wgpu-types-30.0.2/src/lib.rs:176-197` |
| F12 | `Buffer::destroy` / `Texture::destroy` | VERIFIED | `wgpu-30.0.2/src/api/buffer.rs:334`, `texture.rs:117` |
| F13 | Sink renders `Rgba8Unorm` (`sink.rs:30`) | VERIFIED | `oxiterm-gpu/src/sink.rs:30` |
| F14 | `Rgba8Unorm` is a core WebGPU storage-texture format | NRC (correct per WebGPU spec, not re-fetched) | |
| F15 | Windows TDR 2 s default | NRC (Microsoft docs) | |
| F16 | Linux/macOS hangcheck "2-10 s" | UNVERIFIABLE (no source) | |
| F17 | WebGPU availability per browser | VERIFIED (refined) | gpuweb wiki: Chrome 113 Win/Mac/ChromeOS; Android 12+ (121); Linux Intel Gen12+ (144), NVIDIA Wayland (147); Firefox Win 141, Mac Apple Silicon 145 (macOS 26) / 147; Firefox Linux & Intel Mac Nightly only; Safari 26 |
| F18 | `HandleTable` exists in oxiscript-runtime | VERIFIED | `oxiscript-runtime/src/handle.rs:19` |
| F19 | WebGL2 browser fallback still gives no-naga bundle | PARTLY | A9 |

## G. Ledger: AI (task Q6) and plan (Q7) and benchmarks (Q8)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| G1 | `shader-f16` shipped in Chrome 120 | VERIFIED | Chrome blog "new in WebGPU 120" |
| G2 | f16 "widely shipped in Safari 26, modern Vulkan/Metal/DX12" | UNVERIFIABLE (no source) | |
| G3 | bf16 not in WGSL | VERIFIED (absent from WGSL scalar types); "provisionally reserved" UNVERIFIABLE | |
| G4 | `dot4I8Packed` exists | VERIFIED | WGSL TR §17.5.22; naga `language_extension.rs:17` |
| G5 | WebLLM / Burn / Candle / ORT Web / Transformers.js licences and descriptions | NRC | |
| G6 | ORT Web: WebGPU 5-10x faster, Wasm faster below 10 ms | **UNVERIFIABLE** (page has no numbers) | A7 |
| G7 | Naive GEMM 10-30x slower than tiled | UNVERIFIABLE (no source) | |
| G8 | 8.L done `c792b4f`; 8.M `5a5929d` | VERIFIED | `oxi-ui.md:699-700` |
| G9 | half 2.5 pulled by naga | PARTLY (version) | `Cargo.lock`: half **2.7.1**, MIT OR Apache-2.0 (`half-2.7.1/Cargo.toml`) |
| G10 | wasm-encoder / wasmparser 0.263 licence; wasmi MIT/Apache; wgpu MIT/Apache; bytemuck Zlib/Apache/MIT; pollster MIT/Apache | VERIFIED | each `Cargo.toml` `license` |
| G11 | Benchmark reference numbers in the §8.2 table | mostly UNVERIFIABLE → [P] | A7. TF.js 1.7-4.5x and per-dispatch overhead VERIFIED as replacements |
| G12 | TF.js Wasm SIMD 1.7-4.5x | VERIFIED | TF blog |
| G13 | wgpu boids "1M+ particles at 60 fps" | UNVERIFIABLE (the example page gives no such figure) | |
| G14 | "OxiScript v1 needs no GPU", Core Wasm only, wasmi as live engine | VERIFIED consistent | `oxi-ui.md:711, 757`; `abi.md:5` |
| G15 | Licence rule: no GPL/LGPL in the plan | VERIFIED: every recommended dependency is MIT/Apache/Zlib/BSD; esp-dsp/esp-nn are Apache-2.0 | |
| G16 | Multi-core layer (isolates, "CPU SIMD + multi-core first") | GAP | `oxi-ui.md:757` and `oxiscript-language.md` §E Layer 2. The report covers it only by a `rayon` row; added as an open question |

## H. Assessment of the draft

* **Accuracy:** good on local crate facts (wasmi feature gating, config toggles, slot widths, wgpu limits, error-scope internals, repo code numbers: about 90% right, with a few wrong line numbers). Weak on web facts: several misattributions (Maleki, ispc #2786, rust-gpu date, treVM inference, Safari relaxed SIMD) and one real API bug (A1).
* **Sourcing:** the [V]/[S]/[P]/[U] tagging is a good practice, but [V] was used for things not verified (treVM inference, Pharr, the RAII need). Four benchmark sources are dead or contain no numbers, yet carry precise ranges.
* **Completeness:** all 8 questions are answered and the required tables are present. Gaps: Wasmtime determinism knobs, the multi-core layer, the WebGL2-in-browser bundle cost, the guest-WGSL hole, and stage-name mapping.
* **Honesty:** it mostly marks unknowns, but it is internally contradictory once (integer Simd overflow), and it presents benchmark expectations with false precision. No fabricated local file references were found.
