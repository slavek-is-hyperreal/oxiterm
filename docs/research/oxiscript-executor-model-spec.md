# OxiScript executor model (stage 8.3): design specification

Status: design specification, 2026-11-27. Author: Claude Sonnet (advisor-reviewed by Opus), taking over from a Gemini run that could not start. No product code. Inputs read: `docs/roadmap/oxiscript.md` (P-4, P-5, P-9, D-4, D-27..D-31, 8.3a-e, 8.Lab, 8.Mem), `docs/research/oxiscript-simd-gpu-design.md` and its verification ledger, `docs/research/videocore-iv-acceleration.md` and its ledger, `docs/oxiscript/{ir,contracts,grammar,abi}.md`, `docs/owner-questions.md`, `AGENTS.md`.

Normative words: MUST, MUST NOT, SHOULD, MAY as in RFC 2119. Rules are numbered `R-<area>-<n>`. Codes written `OXS-E05xx` / `OXS-P03xx` are **proposed, not in `oxiscript/oxiscript-abi/registry/codes.txt`** (the registry is append-only and is not edited by this report; numbers are re-checked when each slice lands). Facts about WGSL marked `[S]` come from the SIMD ledger and were not re-read in the WGSL text by this pass (the spec page could not be fetched past section 12); they are conservative assumptions, to be confirmed by the 8.3d verification.

## Executive summary

One OxiScript source runs on several **executors** with the same observable behaviour. "Same" is defined per **accuracy class**, because no GPU and no VideoCore IV QPU is bit-compatible with the IEEE-754 reference in `ir.md` section 6:

* **Class E (exact):** integers, booleans, fixed point. Bit-identical on every executor, including which trap fires. Allowed everywhere, including `safety`.
* **Class C (in-contract):** `f32` code on a GPU executor. Not bit-identical; each operation obeys a documented deviation model (fusion, `/` and `sqrt` ULP envelope, flush-to-zero, NaN payload, sign of zero). The 8.3d gate compares with a `close` predicate (1e-5 relative) on a curated well-conditioned corpus. Forbidden for floats in `safety`.
* **Class A (`@approx`):** results specified only up to a declared bound (`@approx(ulp: 8)`, `@approx(rel_ppm: 1000)`). Needed for QPU float code, fast transcendentals (`std/math/approx`) and `host:fft`. Forbidden in `safety`.

The design in ten sentences:

1. An executor is a descriptor (id, class, residency domain, accepted kernel subset, trap model, lowering); scalar, SIMD128, WGSL-GPU are real, threads are reserved (8.4), the VideoCore IV QPU is a conditional idea gated by the 8.Mem probe.
2. Eligibility for a non-CPU executor is computed from `Op::may_trap()` and the effect set, not from a hand-written list.
3. Layer 0 core is `generate`, `map`, `zip`, `reduce`, `scan` (associative integer operators only), `filter` (CPU only), `gather`; everything else (scatter, stencil, matmul, conv, sort) is a library built on those.
4. Reduction order is a normative **canonical pairwise tree** (split at the largest power of two below the length), the same on every executor and for every class; a float `+` reduction exists only under the explicit name `reduceSum`.
5. Out-of-bounds (Q-SIMD-4, proposal): the automatic path offloads only proved-in-bounds code in every build mode; trap timing inside `Resident` operations is **deferred to the next sync point on every executor**; an explicit core `gather` carries an always-on guard feeding that deferred trap; `host:gpu` gets language-defined zero/skip guards (WebGPU does not return zero, ledger A6); the QPU accepts only host-validated affine streaming access.
6. The executor choice is a pure function of static residency types, host facts, profile, target and per-edition constants; a size threshold may only choose between executors of the same class and the same residency domain.
7. `@approx` is a function attribute with integer-valued keys (the attribute grammar has no float literal), propagates to callers (`OXS-E0502`), and is an error under `safety` (`OXS-E0501`).
8. The QPU executor is a *streaming-kernel* executor: audited memory skeletons plus a whitelisted straight-line ALU body, never guest machine code, behind a privileged helper with a fixed command set; `host:fft` (GPU_FFT, BSD-3) is a fixed capability next to it, not an executor.
9. Every executor has a differential-test obligation whose equality predicate is its class (E: bits and trap kind; C: `close`; A: the declared bound against the CPU reference, plus run-to-run bit reproducibility on one host).
10. Stages 8.3a-e are amended (section 8.4); one new conditional stage `8.3f` (QPU) follows the 8.Mem probe.

## 1. Decisions already taken (not re-decided here)

D-30 adopted the lead's recommendation for Q-SIMD-1..3 and 5..18 and recorded the float rule for non-IEEE executors. The spec conforms to them:

| Q / D | Decision in force | Where used |
|---|---|---|
| Q-SIMD-1 | integer `Simd` forbids plain `+ - *` (`+% +|`) | 3.3 operator set |
| Q-SIMD-2 | inline-element arrays at the start of 8.3d | 8.4 |
| Q-SIMD-3 | placement report (8.S4b) before 8.3b or a minimal `Remark` type early | 9 |
| Q-SIMD-12 | no runtime-inserted transfers; library `gpuMap` does `upload`+`map`+`download` visibly | R-POL-2 |
| Q-SIMD-13 | thresholds are fixed constants per language edition | R-POL-4 |
| Q-SIMD-14 | name `Resident<T>`; no manifest "may use GPU" line (host quotas only) | 2.4 |
| Q-SIMD-15 | failure of resident operations is a **trap**, handles are poisoned, recovery by re-`upload` | R-EXE-9, R-OOB-4 |
| Q-SIMD-16 | widen the proof scope; measure the eligible fraction on the 8.3d corpus before adding a flag-plus-deferred-trap channel for gathers or a scan for `filter` | R-OOB-6 (channel is specified here, enabled by that measurement) |
| Q-SIMD-17 | the executor may differ between hosts; record it in the report and run log | R-POL-5 |
| Q-SIMD-18 | the layer-0 vocabulary is fixed by a spec pass: this document, section 3 | 3 |
| D-30 floats | exact integer/fixed-point by default plus explicit `@approx` with documented bound, forbidden in `safety` | 4 |
| D-30 Q-SIMD-4 | **still open**: section 5 proposes | 5 |
| 8.Mem | VideoCore stays an idea; the probe decides whether it becomes a stage | 7 |
| D-29 / 8.Lab | a speedup never changes what a program does (results, traps, order of effects, float bits) | R-CLS-1 |

Premises from the existing reports that this spec relies on and re-states: `ir.md` section 6 (floats IEEE RNE, never fused, no FTZ on the reference; transcendentals are OxiScript library code, not instructions, so they are plain `+ - * /` sequences on every executor); the kernel profile of the SIMD report 4.2 (no managed allocation, no host calls, no recursion, no `f64/i64/u64` on GPU); `P-5` assertions are only for language-defined facts.

## 2. The executor interface

### 2.1 What an executor is

An **executor** is a way to run a *verified data-parallel region*: the body of a layer-0 combinator, a `parallel for` (frozen shape, see 2.6), a `Simd` loop, or a `kernel function`. It consumes verified OxiIR of that region; it never changes the program. Compiler-side an executor is a descriptor:

```text
Executor {
  id:        scalar | simd128 | wgsl | qpu_vc4 | threads        // closed set per edition
  class:     E | C | A            // strongest accuracy class it can give float code (integers are always E)
  domain:    host | webgpu | vc4  // residency domain its Resident<T> values live in
  host_visible: bool              // can the CPU read/write that domain in place, no transfer
  accepts(region) -> Accept | Reject(reason_code)    // pure function of the IR, see R-EXE-3
  lower(region) -> Variant        // wasm loop | v128 loop | WGSL text | QPU streaming program
  trap_model: Precise | Deferred | None
}
```

| Executor | class | domain | host_visible | trap_model | Exists when |
|---|---|---|---|---|---|
| scalar | E | host | yes | Precise | always |
| simd128 | E | host | yes | Precise (accepts only trap-free regions, so no trap occurs) | target has SIMD128 |
| wgsl | C | webgpu | no | Deferred | host has `ComputeTier::Compute` |
| qpu_vc4 | A | vc4 | only with `vcio2` | Deferred | idea, gated on 8.Mem (section 7) |
| threads | E | host | yes | Precise (first trap by sequential order) | stage 8.4 |

R-EXE-1. The set of executors and their descriptors is fixed per language edition. A new executor is an edition change plus a spec amendment. Executors MUST NOT be user-extensible.

R-EXE-2. Every region has a **scalar variant** (the CPU reference, ir.md semantics). Any other variant is an optimisation of it and MUST satisfy the accuracy class rules of section 4 against it.

R-EXE-3 (eligibility is computed, not listed). A region is eligible for an executor with `trap_model` Deferred or None only if the verified region contains **no instruction for which `Op::may_trap()` is true** and no managed allocation, RC op, host call, closure call or recursion. Concretely this excludes plain trapping integer `+ - *`, integer `/` and `%` (zero divisor, `MIN / -1`), `neg MIN`, `shl`/`shr` with an out-of-range amount, trapping `conv` (int range, float NaN/out of range), `unopt`, and every array access whose index the checker cannot prove in range (section 5). `.wrap`, `.sat` forms and proved accesses are eligible. The reason for rejection is the first excluded instruction, reported as `OXS-P0302` with its source span. Because the criterion is `may_trap()`, a new IR op is ineligible by default until its trap-freedom is declared.

R-EXE-4 (lowering contract). `lower` receives IR in the verified kernel profile and produces a variant that is a pure function of that IR and the edition constants (reproducible build output, ir.md section 6). Layout of data crossing the executor boundary is the element layout of `abi.md` section 2.3 (std430-compatible for WGSL, checked by the emitter; `vec3` and trailing padding rejected or padded explicitly).

R-EXE-5 (no new authority). An executor reads and writes only the program's own `Resident` data and parameters; it has no host calls, cannot name devices, buffers or shader text. It therefore needs no `main` parameter (AGENTS.md rule 7 is about authority). Fixed host capabilities (2.5) are different: they are `main` parameters.

R-EXE-6 (determinism per executor). On one host, one executor, the same inputs MUST give the same bits on every run, for every class. Float atomics, unordered reductions and any race MUST NOT appear in emitted code.

### 2.2 Residency: `Resident<T>`, `upload`, `download`

`Resident<T>` (Q-SIMD-14; the SIMD report 4.6.3 gives the reasoning for the name) marks data that lives in the executor domain of the host. `T` is `E[]`, `Image`, or a scalar `E` for results of reductions, with `E` an eligible element type.

R-EXE-7. The type system never depends on host availability: `Resident<T>` has one API on every host. Where the accelerator domain is absent, the value is host memory and the CPU variant runs in place (no transfer exists, none is reported).

R-EXE-8 (visible transfers, Q-SIMD-12). The runtime MUST NOT insert a transfer. `upload(xs: T[]): Result<Resident<T[]>, i32>` (fallible: quota) and `download(r, cb)` (callback form until 8.4 async, because readback is asynchronous) are the only host/accelerator crossings, each one `OXS-P0306` line. No layer-0 operation on `Resident` input returns host data: a reduction returns `Resident<scalar>`. Mixing `Resident` and host arrays in one combinator is a compile error (code to be allocated in the `OXS-E` range). A `Resident` input whose body is not eligible for the **baseline executor** (the WGSL eligibility of R-EXE-3) is a compile error, because the check is static and must not depend on the host.

R-EXE-9 (lifetimes, errors). Resident values are handles owned by the instance arena: released on drop and on hot reload. `upload` returns `Err(status)` when the domain quota is exceeded. A dispatch-cap overflow traps with kind `quota`; device loss or a failure recorded by a guard (R-OOB-3) traps with the first reported failure at the next sync point (R-OOB-4), after which all handles of that domain are poisoned (further operations on them trap again) and the program rebuilds by `upload` (Q-SIMD-15).

R-EXE-10 (domain completeness). An accelerator domain MAY be exposed as a `Resident` domain only if (a) it is `host_visible` (the CPU fallback runs in place, no transfer), or (b) its eligibility is a superset of the baseline eligibility, so that no baseline-eligible `Resident` code is left without a variant on that host. WebGPU satisfies (b) by definition (it *is* the baseline). The QPU satisfies neither unless `vcio2` is adopted (section 7.3); otherwise it is offered only through fixed capabilities.

### 2.3 Sync points

R-EXE-11. A **sync point** is: `download(...)`, an explicit `sync(r)`, and the end of the current host turn (the entry-point call returns to the host). At a sync point the runtime drains all pending resident operations of the instance, and if any recorded a failure, the instance traps with that failure (R-OOB-4). Failures are therefore never lost when a program never downloads.

### 2.4 Authority and manifest

R-EXE-12. Pure compute on an accelerator domain is limited by host quotas (domain bytes, invocations per dispatch, dispatches per turn, wall time per dispatch as fuel). The manifest records the variants a module carries (`variants wgsl`, with the ABI 1.1 `features simd128` line, SIMD report 4.6.6); a host without the Compute tier ignores them because a CPU variant always exists. QPU variants are **never** carried as machine code (R-QPU-1).

### 2.5 Fixed host capabilities

A fixed capability (`host:fft`, `host:state`, a future `host:qpu_gemm`) is an audited host-side routine with a fixed IDL. It is not an executor: it has no region to lower, no eligibility, no variant. It is a `main` parameter (authority), its calls are `OXS-P0301` lines, and its IDL marks the accuracy class of each function (`approx` in the IDL entry): calling an `approx` function triggers the `@approx` rules of 4.3. See 7.6.

### 2.6 Source forms

`kernel function` and `parallel for` are **reserved words today** (`OXS-E0900` at parse time, grammar section 5): the examples in this document that use them are frozen shapes, written in `ts` fences. Everything else below parses today (verified with `oxi check`: the only errors are unknown names such as `Resident`, `upload` and the unknown attribute `@approx`, never syntax errors).

## 3. Layer-0 vocabulary (Q-SIMD-18)

### 3.1 Principles

R-L0-1. Every layer-0 operation has a **sequential reference definition** (an OxiScript loop). A parallel executor MUST produce the reference result for class E. For float results the class rules of section 4 apply, with the reduction structure fixed by R-L0-3.

R-L0-2. Element functions are pure and total within the kernel profile; the order in which elements are computed is unobservable. If an element function can trap (CPU executors only, R-EXE-3), the trap reported MUST be that of the lowest element index, as in the sequential reference. A threads executor (8.4) MUST implement this by recording the minimum failing index.

### 3.2 The core list

| Operation | Reference semantics | Classes | CPU | GPU (v1) | QPU (idea) |
|---|---|---|---|---|---|
| `generate(n, f)` | `out[i] = f(i)` for `i` in `0..n` | element fn | yes | yes | yes (affine, streaming) |
| `map(xs, f)` | `out[i] = f(xs[i])` | element fn | yes | yes | yes |
| `zip(xs, ys, f)` | `out[i] = f(xs[i], ys[i])`; lengths must be equal or the call is an error at the call (precondition, trap `bounds` on CPU; on a `Resident` pair checked at the host before launch, so no deferred trap) | element fn | yes | yes | yes |
| `reduce` (named operators below) | canonical tree, R-L0-3 | E for integers; C/A for float `reduceSum` | yes | yes | ints and `@approx` float |
| `scanInclusive`, `scanExclusive` (named integer operators) | sequential prefix; result independent of blocking because the operator is associative | E | yes | yes (class E only) | no |
| `filter(xs, pred)` | stable compaction; output length is data-dependent | E | yes | **CPU only in v1** (`OXS-P0302` reason `filter`) | no |
| `gather(xs, idx)` | `out[i] = xs[idx[i]]` with checked-index semantics (R-OOB-3) | E for integers | yes | yes, guarded | no (v1) |

Library only (built from the core, defined in `std/*`, never in the grammar): `scatter` (conflicting writes have no cheap deterministic definition: "last writer by index order" needs a sort on a GPU), `stencil1d/2d` (a `generate` over affine, clamped or wrapped `View1D/View2D` reads), `matmul`, `conv2d`, `softmax`, `histogram`, `sort`, `image` operations, `std/tensor` kernels. They may be mapped to hand-tuned executor kernels only if they keep the library's documented order of accumulation (matmul: ascending `k` per output element, f32 accumulator, matching the usual tiled kernel; float class C on GPU, E for integers).

Not core and not parallelised: `Array.reduce`/`forEach` with an arbitrary closure (sequential left fold, TypeScript meaning). The compiler MAY run `xs.reduce((a, b) => a +% b, 0)` in parallel because it recognises an associative integer operator, so the result is the same; it MUST NOT for float `+`, whose parallel form is the different function `reduceSum`.

### 3.3 Reduction: the canonical tree

R-L0-3. Reduction over `n >= 1` elements is defined by the tree `T(lo, hi)`: `T(lo, lo+1) = xs[lo]`; otherwise let `h` be the largest power of two strictly less than `hi - lo` and `T(lo, hi) = op(T(lo, lo+h), T(lo+h, hi))`. The empty reduction is the identity of the operator (`reduce` with an identity) and is defined only for operators that have one. Every executor MUST use exactly this association, for every class and element type. Reference (compiles today):

```oxs
function sumTree(xs: i32[], lo: i32, hi: i32): i32 {
  const n = hi - lo;
  if (n === 1) {
    return xs[lo];
  }
  let half: i32 = 1;
  while (half * 2 < n) {
    half = half * 2;
  }
  return sumTree(xs, lo, lo + half) +% sumTree(xs, lo + half, hi);
}

export function checksum(xs: i32[]): i32 {
  if (xs.length === 0) {
    return 0;
  }
  return sumTree(xs, 0, xs.length);
}
```

Properties the executors rely on: sub-trees over aligned blocks of size `2^k` are complete trees, so a chunked dispatch (workgroup partials of `2^k` elements combined by the same rule) reproduces the tree; `Simd<f32,4>.sum()` (`((v0+v1)+(v2+v3))`) is the `n = 4` tree.

Named operators (the only parallel reductions in core):

* integers: `+%`, `*%`, `&`, `|`, `^`, `min`, `max` (associative and commutative, class E whatever the tree). **Not** `+|`: saturating add is not associative (`sat(sat(a+b)+c) != sat(a+sat(b+c))` for mixed signs), and plain trapping `+` is never vectorised or offloaded.
* `bool`: `&&`-reduce (`all`) and `||`-reduce (`any`), without short-circuit.
* floats: `min`, `max` (NaN-propagating as in `ir.md`; associative), and **`reduceSum`** (tree order of R-L0-3, the only float `+` reduction; plain sequential float accumulation in a `for` loop is not parallelised, `OXS-P0106`).

R-L0-4 (GPU honesty). WGSL drivers may reassociate and fuse `[S]`, so on the `wgsl` executor R-L0-3 is a **MUST on the emitted code** (explicit interleaved addressing with barriers, no `subgroupAdd`, no atomics), not a guarantee about what the driver executes. Float results from the GPU remain class C even though the structure is fixed. Only the CPU executors and the QPU (no driver compiler between us and the machine code) can guarantee the association. Cost: shared-memory tree reductions with fixed addressing are somewhat slower than free-form ones; the 8.3d benchmarks record it.

R-L0-5 (what the tree breaks). The auto-vectoriser shape S4 of the SIMD report (strided vector accumulators plus a horizontal sum) is exactly right for integers (any association) and **wrong for floats**, because its association differs from R-L0-3. For floats the vectoriser MUST either emit the canonical tree (partial sums over aligned blocks of the vector width, combined as in R-L0-3) or not vectorise (`OXS-P0106`). Float `scan` has no canonical parallel order and is not core: a float prefix sum is a sequential loop, CPU only.

### 3.4 Example (parses today; `Resident`, `upload`, `reduceSum` are the proposed names)

```oxs
import { Log } from "host:log";

// Exact class: integers only. Eligible for scalar, SIMD128, GPU, QPU.
export function mixBytes(px: u8[], k: u8): u8[] {
  return px.map((p: u8) => (p ^ k).wrapping<u8>());
}

// Frozen shape of the user-visible API (not checkable yet):
//   const r = upload(xs)?;                       // visible host -> resident transfer, OXS-P0306
//   const e = reduceSum(r.map((x: f32) => x * k)); // Resident<f32>, class C on GPU
//   download(e, (v: f32) => { log.write(2, `energy = ${v}`); });
export function main(log: Log): void {
  log.write(2, "ok");
}
```

## 4. Accuracy classes

### 4.1 Class E: exact

R-CLS-1. A region is class E when every operation in it is an integer, boolean or fixed-point operation (including `+% +| *% & | ^ << >> min max`, comparisons, `select`, integer `as` with `.wrap`/`.sat`). On every executor the result MUST be bit-identical to the scalar reference, and a trap, where the executor permits one, MUST be the same trap kind. A speedup in 8.Lab never moves a class E region to another class (D-29).

Fixed point is a library over integers (`std/fixed`, proposed), e.g. Q16.16, class E on every executor (compiles today):

```oxs
struct Q16 {
  raw: i32;
}

export function qmul(a: Q16, b: Q16): Q16 {
  const wide = (a.raw as i64) * (b.raw as i64);
  return { raw: (wide >> 16) as i32 };
}
```

(`i64` is CPU-only on GPU executors, so a GPU variant of a Q16 kernel uses a 32-bit decomposition; the library supplies it, the semantics are the same.)

### 4.2 Class C: in-contract

Class C exists for `f32` code on the `wgsl` executor (and any future IEEE-ish accelerator). It is **not** a promise about user programs: a subtraction of nearly equal values, fused or not, breaks any per-output relative bound (catastrophic cancellation). It is a **per-operation deviation model** plus a **gate tolerance**.

R-CLS-2 (deviation model). Relative to the scalar reference (IEEE-754 binary32, RNE, never fused, no FTZ), an executor of class C MAY deviate as follows, and in no other way:

1. `+ - *` are correctly rounded `[S]` except that subnormal operands or results MAY be flushed to zero (FTZ);
2. an adjacent multiply and add (after inlining) MAY be fused into one rounding; WGSL offers no way to inhibit this `[S]`, so the emitter MUST NOT promise otherwise;
3. `/` and `sqrt` MAY deviate from the correctly rounded result by at most the envelope of the edition table `class_c_envelope` (initial values from WGSL section 15.7.4, to be verified; a conservative working assumption is 2.5 ULP for `/`, 2 ULP for `sqrt` `[S]`, `[P]`); the table is part of the edition and frozen with it;
4. the payload and sign of a NaN are unspecified (as everywhere, `ir.md` section 6: NaN bits are unobservable);
5. the sign of a zero MAY differ; operations that observe it (`copysign`, `bitsOf`, `atan2`, a reciprocal of a data-dependent zero) make a region ineligible for class C with reason `sign of zero observable` (`OXS-P0302`);
6. overflow, NaN and infinity behaviour at the **boundary** (a fused operation whose unfused form overflows) is unspecified; regions are not rejected for it, the gate corpus avoids it;
7. transcendental functions are not an additional source of deviation: they are OxiScript library code (`ir.md` section 6), so on the GPU they are the same `+ - * /` sequence and fall under points 1 to 3. A hardware intrinsic substitutes for them only through `@approx` (class A).

R-CLS-3 (gate predicate, not a user property). The 8.3d differential gate compares executors on a curated corpus of **well-conditioned** programs with the predicate

```text
close(a, b) :=  both NaN
            or  a === b                                   // includes the infinities with equal sign; +0 and -0 compare equal
            or  |a - b| <= 1e-5 * max(|a|, |b|)
            or  |a| < 1.1754944e-38 and |b| < 1.1754944e-38   // both in the flush-to-zero range
```

A corpus entry declares its condition number bound (for a sum: sum of absolute values over absolute value of the result, at most 1e3) so that the tolerance is meaningful; the report states that the tolerance is a gate, not a guarantee. NaN versus a number, or an infinity of the wrong sign, is always a failure.

R-CLS-4. `safety` profile: a region with float element type MUST NOT run on a class C or A executor; with a `Resident` float input it is a compile error (`OXS-E0500`, as the SIMD report 4.5 says), with host input it runs scalar or SIMD128 (class E for float because both are IEEE-exact, bit-identical, ir.md section 6). Integer regions run anywhere.

### 4.3 Class A: `@approx`

R-CLS-5 (syntax). `@approx(ulp: N)` or `@approx(rel_ppm: N)` on a function declaration, where `N` is a positive integer literal. The attribute grammar of `grammar.md` section 4 allows only `STRING | INT | true | false | Ident` as an argument, so integer-valued keys avoid a grammar change (a float literal argument is an optional later extension). `ulp` is the maximum error in units in the last place of `f32` against the **reference result** (the result of the function body executed on the scalar executor); `rel_ppm` is the maximum relative error in parts per million. Exactly one key. Results with magnitude below `2^-126` are exempt (flush-to-zero range). Example (frozen attribute, parses today, rejected only as unknown):

```oxs
@approx(ulp: 8)
function fastInvSqrt(x: f32): f32 {
  return 1.0f32 / x;
}
```

R-CLS-6 (meaning). `@approx(b)` means: an executor MAY return any value within `b` of the reference. An executor with a hardware approximation (QPU SFU `recip`, `recipsqrt`, `exp2`, `log2`) MAY substitute it for a body that the compiler recognises as a library approximation. The declared bound MUST hold on **every** executor that may run the function. For library functions in `std/math/approx` (Quake-style `rsqrt`, Schraudolph `exp`, minimax polynomials; owner idea in the roadmap Ideas table) the bound is documented per function and **proved by an exhaustive sweep of all `2^32` `f32` inputs** (feasible for a unary function) on every executor in CI; for a user-declared bound it is a test obligation of the program (a `law` over a sample, or the differential harness).

R-CLS-7 (propagation). A function that calls an `@approx` function, a `std/math/approx` function or a host function marked `approx` in its IDL MUST itself be `@approx(...)` (`OXS-E0502`, fix: add `@approx(ulp: N)`). The compiler does not compute a composed bound; the author states it; the harness checks it. This keeps approximation visible in every signature on the path, in the spirit of the placement report.

R-CLS-8 (`safety`). Under `@@profile(safety)` (or `@profile(safety)` on a function) an `@approx` declaration and any call to an approximate function is `OXS-E0501` (fix: use the exact `std/math` function or move the code out of the `safety` unit). `@expect(exact)` (proposed) asserts that a function reaches no `@approx` and no class C/A region; it is assertable (a static fact) and is the certification evidence for "this offload is class E".

R-CLS-9 (report). Every region placed on a class A executor, and every call to an approximate function, emits `OXS-P0309` (not assertable: target fact) with the bound; the run log records the executor actually used.

R-CLS-10 (the QPU rule, D-30). An auto-path float region MAY run on the QPU only if its element function is `@approx`. A plain `f32` function is ineligible for the QPU (`OXS-P0307`, reason `float without @approx on class A executor`) and runs on the host executors.

## 5. Out-of-bounds behaviour (Q-SIMD-4, proposal)

### 5.1 Facts

* CPU: an out-of-range index traps `bounds`, precisely, at the access.
* WebGPU: a shader cannot trap. Robust access gives, for a read, any in-bounds value or zero; for a write, a discard or a store at another in-bounds location; implementations differ (naga offers `Restrict`, `ReadZeroSkipWrite`, `Unchecked`) (ledger A6). "WebGPU returns zero" is **false**; the lead's proposal (3) rested on it.
* VideoCore IV: no IOMMU. An out-of-range TMU read or VPM DMA write corrupts arbitrary physical memory, including the kernel (videocore report 3.1). There is no robust-access fallback at all.
* Prior art for a read-back failure flag: Futhark records a failure in a single 32-bit `global_failure` word in device memory, every kernel starts by checking it, the host reads it after each kernel launch and raises the error; the measured overhead of its GPU bounds checking is a geometric mean of 4% to 6% on 19 benchmarks (Henriksen, "Bounds Checking on GPU", HLPP 2020; blog "How Futhark implements bounds checking on the GPU"). This matches the lead's estimate ("a few to a dozen percent").

### 5.2 Evaluation of the lead's proposal

(1) *Offload only proved in-bounds loops.* Right, and already the v1 eligibility rule (SIMD report 4.6.2, proof scope: implicit `map`/`zip` index, dominating `i < xs.length`, affine indices bounded by a visible shape guard, `@shared` tile indices, shaped views). Keep. It costs eligibility for data-dependent gathers.

(2) *Debug mode with a flag read back by the CPU, which then traps.* Right in intent, with two defects: (a) if it changed **eligibility** between debug and release, the executor choice would depend on the build mode, violating the P-5 rule that assertable facts are identical in debug and release; (b) a read-back trap fires **later** than the CPU trap would, so ordering of host effects between access and sync differs. Both are fixed by stating the timing as a language rule (R-OOB-4) that every executor obeys, and by making the flag a *self-check of the prover* in debug rather than an eligibility change.

(3) *`host:gpu` keeps WebGPU semantics.* Amended: the language must not inherit driver-defined behaviour. In `host:gpu` kernels the emitter always inserts explicit guards with the language-defined result **read gives 0, write is skipped**, deterministic on every vendor, plus the debug flag.

Addition beyond the three options: a **core `gather` with an always-on guard** and the deferred trap, so that data-dependent reads can be offloaded explicitly and visibly.

### 5.3 Normative rules

R-OOB-1 (auto path, every build mode). The automatic path (layer-0 combinators and `parallel for` placed by the executor policy) offloads a region to a Deferred/None executor only if every array index is proved in range by the checker (R-EXE-3), **in debug and in release alike**. An unproved index gives `OXS-P0302` reason `unproven index`, the region runs on a CPU executor, and `@expect(gpu)` on it fails the build.

R-OOB-2 (debug self-check). In a debug build the WGSL emitter additionally guards every access of a proved region and records a failure through R-OOB-3. A failure of a proved access is a **compiler bug** (an unsound proof): the runtime traps with kind `bounds` and the report says `OXS-P0310` "bounds proof violated". This does not change eligibility and does not change release results.

R-OOB-3 (checked `gather` and failure records). `gather(xs, idx)` is eligible on the GPU with an explicit guard. A failing element does not write; it records `atomicMin(failure_slot, linear_index)` into one `u32` slot per operation (initial `0xFFFFFFFF`), where `linear_index` is the index in the whole operation (not within a chunk or workgroup), so a chunked dispatch keeps the **lowest** failing index. No other bookkeeping: no float atomics, one integer atomic on the failing path only.

R-OOB-4 (deferred trap, all executors). Inside an operation on `Resident` data, a failure is **recorded**, not trapped. It is raised at the next sync point (R-EXE-11) as a trap of kind `bounds`, carrying the source span of the **first failing operation in program order** and the lowest failing linear index in that operation (a table maps operation ids to spans, as in Futhark). The same rule applies to the CPU variant running over `Resident` data (host memory) so that the observable behaviour does not depend on the executor: on the CPU variant a failing element records and the remaining work of that operation may continue or stop, unobservably. An operation that consumes data written by a failed operation is skipped and its outputs are poisoned; a failed operation's output is never observable (the trap replaces any `download` callback). Quota and device-loss failures (R-EXE-9) use the same channel.

R-OOB-5 (`host:gpu`). Kernels compiled through `host:gpu` (8.3e) use the guarded semantics read 0 / write skipped, never the driver's, plus the debug flag of R-OOB-3 and R-OOB-4.

R-OOB-6 (staging, Q-SIMD-16). R-OOB-3 and R-OOB-4 define the channel so that the `Resident` timing semantics never change later. In v1 of 8.3d the channel is built, but data-dependent `gather` on the GPU executor is enabled only if the eligible-fraction measurement on the 8.3d corpus shows it is needed (the owner's Q-SIMD-16 rule); without it v1 behaves as the SIMD report 4.6.2 states and the only deferred failures are quota and device loss.

R-OOB-7 (QPU). The QPU executor accepts only unit-stride and affine access whose bounds the **host validates before launch** from the launch shape and buffer table; it has no `gather`, no debug flag (it cannot) and no guard that could recover. A kernel that cannot be reduced to that form is ineligible (`OXS-P0302`).

Example (reference semantics of checked `gather`; compiles today):

```oxs
export function gatherRef(xs: f32[], idx: i32[]): Result<f32[], i32> {
  const out: f32[] = [];
  for (const i of range(0, idx.length)) {
    const j = idx[i];
    if (j < 0 || j >= xs.length) {
      return Err(i);
    }
    out.push(xs[j]);
  }
  return Ok(out);
}
```

## 6. Executor choice policy

R-POL-1 (pure function). `executor = f(static residency types of the inputs, host facts, profile, target, edition constants)`. Host facts are: `ComputeTier`, the host's accelerator domain (`none | webgpu | vc4`), SIMD128 availability. No timing, load, history, adaptive tuning or dynamic profile may enter. Same facts, same executor, on every run (the 10,000-combination property test of the 8.3d gate).

R-POL-2 (table, Q-SIMD-12). Static types decide the domain; the runtime never inserts a transfer.

| Inputs | Host facts | Chosen executor |
|---|---|---|
| all host arrays | SIMD128 and region vectorisable and `n >= N_simd_min` | simd128 (threads after 8.4) |
| all host arrays | otherwise | scalar |
| all `Resident` | accelerator present, region eligible, profile allows | that accelerator's executor |
| all `Resident` | no accelerator, or float region under `safety` is a compile error already | CPU variant over host memory, no transfer |
| mixed | any | compile error (R-EXE-8) |
| float region | profile `safety` | scalar or simd128 only (R-CLS-4) |

R-POL-3 (same-class rule). A size threshold MAY only choose between executors of the **same accuracy class and the same residency domain**. Consequence: thresholds can pick scalar versus simd128 (both E, host) and nothing else. They can never move a computation to another class, and they can never cause a transfer. Where the SIMD report 4.6.4 uses `N_hint` and `N_small`, those remain **report hints** (`OXS-P0305`) that do not select anything.

R-POL-4 (constants, Q-SIMD-13). Thresholds are constants of the language edition, listed in the edition table and not measured per host: `N_simd_min = 8` elements (matches the shape remark of the SIMD report 3.3), `N_hint = 65536`, `N_small = 4096` (the latter two `[P]`, tuned in 8.3d and then frozen). A change is an edition change.

R-POL-5 (hosts may differ, Q-SIMD-17). Host facts differ between machines, so the executor, and for class C and A the low-order float bits, may differ between hosts. The placement report (compile phase) states the policy result for the facts known at build time; the run log states the executor actually used and every transfer. Class E results never differ. `safety` pins float code to class E executors.

R-POL-6 (`@expect`). `@expect(simd)`, `@expect(gpu)`, `@expect(qpu)` assert only **assertable** facts, as P-5 requires: that a variant of that executor exists and the region satisfies the language-defined shape (failing reason: `OXS-P0102..P0108`, `P0302`, `P0307`, `P0311`). *Where* it ran is a host fact and goes to the run log only; `@expect(gpu, on: "webgpu")` scoping works as for SIMD. `@expect(no_transfer)` asserts that the body has no `upload`/`download`. `@expect(exact)` is R-CLS-8.

Proposed codes (all `P` codes are `assertable` as marked; none exists in `codes.txt` except `P0101/0102/0201/0301`; `P0103..P0112` and `P0302..P0306` are already proposed by the SIMD report):

| Code | assertable | Meaning |
|---|---|---|
| `OXS-P0307` | yes | no QPU variant: float without `@approx` on a class A executor |
| `OXS-P0308` | no | executor not used at run time: tier, profile, device lost, helper unavailable |
| `OXS-P0309` | no | approximate placement: class A executor or approximate call, with bound |
| `OXS-P0310` | no | bounds proof violated (debug self-check, compiler bug) |
| `OXS-P0311` | yes | no QPU variant: access not affine-streaming, operator not in the QPU set |
| `OXS-E0501` | error | `@approx` or approximate call under `safety` |
| `OXS-E0502` | error | calls an approximate function but is not marked `@approx` |
| `OXS-E0503` | error | malformed `@approx` (missing or non-integer key, zero, two keys) |

Example (frozen shapes: `Resident` and `@expect(gpu)` on a function; parses today, unknown names only):

```oxs
@expect(gpu)
function scale(xs: Resident<f32[]>, k: f32): Resident<f32[]> {
  return xs.map((x: f32) => x * k);
}

export function run(xs: f32[], k: f32): f32[] {
  // Host data: layer 0 runs on simd128 or scalar, never on the GPU.
  return xs.map((x: f32) => x * k);
}
```

## 7. The VideoCore IV QPU executor (idea, gated on the 8.Mem probe)

Status: VideoCore stays an idea (D-30). This section fixes the design *if* the probe of 8.Mem passes (promotion rule in the roadmap: about 5x over the w2c2 CPU matrix multiply, display and GPU coexisting or a display-less demo accepted). Nothing here is scheduled before that.

### 7.1 Subset that is feasible

QPU facts used (videocore report and ledger): 12 QPUs, 16-way SIMD over 4 physical lanes, 24 GFLOPS FP32 peak at 250 MHz; FP32 add and mul with round-to-zero and no denormals (ledger B7: RTZ confirmed by tests, FTZ implied); 32-bit integer add, sub, bitwise, shifts, min, max on the ADD ALU; **24-bit** multiply on the MUL ALU; SFU `recip`, `recipsqrt`, `exp2`, `log2` with a 3-instruction latency; 4 KB-class VPM window, TMU reads; no atomics we would use, no 64-bit types, no division.

| Operation | QPU support | Class |
|---|---|---|
| `map`, `zip`, `generate` over `i32,u32,i16,u16,i8,u8,bool`: `+% -% & | ^ << >> min max`, compares, select | yes | E |
| integer `*%` | yes: three `mul24` pieces and adds for full 32-bit wrapping; direct `mul24` when operand ranges are proved to fit | E |
| `u8`/`i16` packed loads | yes (Regfile-A unpack) | E |
| `reduce` with integer operators in canonical tree order | yes | E |
| `f32` `+ - *`, compares, select, `reduceSum` | only inside `@approx` functions (R-CLS-10) | A |
| `rsqrt`, `recip`, `exp2`, `log2` | via `std/math/approx` mapping to the SFU | A |
| integer `/ %`, `f64`, `i64`, `u64`, `scan`, `filter`, `gather`, `@shared`, atomics, loops in the body, recursion, calls | no (v1) | n/a |

The accepted body is **straight-line SSA over lane registers** with no memory operations of its own.

### 7.2 Streaming-kernel executor (the safety argument)

R-QPU-1. A QPU variant MUST NOT be machine code supplied by a module. The module carries, in `oxi.kernels`, a *streaming kernel program* (SKP): a list of whitelisted ALU opcodes over virtual registers, plus the stream bindings. The host (not the guest) turns the SKP into QPU code by filling one of a **fixed set of audited memory skeletons** (unit-stride streaming load/store with host-computed bounds, reduction epilogue). The generated program contains no address arithmetic from the guest. A malformed SKP is refused at load time (finite opcode set, register indices in range, no unknown op).

R-QPU-2 (why). VideoCore IV has no IOMMU, and every QPU compute tool needs root or the `vcio2` kernel driver (ledger A9). Whoever can start a QPU program can overwrite any physical memory. Letting an untrusted module choose QPU machine code would void the sandbox of P-6. Under R-QPU-1 the QPU can only touch the buffers the host bound to the skeleton.

R-QPU-3 (fuel). Every launch carries the firmware's `EXECUTE_QPU` timeout, which acts as fuel: a hang ends in a trap of kind `quota` (via the deferred channel of R-OOB-4), the helper resets V3D, and all handles of the domain are poisoned.

### 7.3 Data path on the Pi 1

The Pi 1 B+ has a unified 512 MB memory, but mailbox/CMA memory is mappable only by root, and the QPU tools do not coexist with full KMS (`vc4-kms-v3d`); the display choice may decide this (ledger A1, A2, A4: py-videocore needs both `vc4-kms-v3d` and `vc4-fkms-v3d` off; VC4CL issues #60/#100; GPU_FFT's guide says it can run beside the old firmware GL; the `&v3d`-disabled overlay is **untested**). The 8.Mem probe on a spare SD card measures it. Two modes:

* **Mode H (helper).** An unprivileged OxiScript host talks to a small privileged helper (`oxi-qpud`, working name) over a Unix socket. The helper owns the buffer table (CMA allocations through `/dev/vcio` mailbox tags `ALLOCATE/LOCK_MEMORY`), the launch (`EXECUTE_QPU` with timeout), and performs R-QPU-1. Commands are fixed: `alloc(bytes)`, `free(handle)`, `write(handle, offset, bytes)`, `read(handle, offset, len)`, `launch(skeleton_id, bindings, shape)`, `fft(...)`. `upload` and `download` are **real copies across the process boundary** and are reported as transfers. Memory-safety enforcement lives in the helper: audited skeletons only, whitelisted body encoding, helper-owned handles, per-client quotas, per-launch timeout. In this mode the domain is **not** host-visible, so by R-EXE-10 it is not exposed as a `Resident` domain; it is available only to fixed capabilities (7.6).
* **Mode V (`vcio2`).** If Marcel Mueller's `vcio2` driver (unprivileged access to the VideoCore IV GPU, per the GPU_FFT page) is adopted, CMA buffers are mappable by the host process, the domain is host-visible, and R-EXE-10(a) applies: a `Resident` operation with no QPU variant runs on the CPU in place, no transfer. The process that can start QPU programs is then trusted for memory safety, so R-QPU-1 is the only barrier and the BAD ACTOR pass concentrates on it.

R-QPU-4. The QPU executor appears in the executor policy only in Mode V (or a future host-visible mode). In Mode H the QPU is reachable only through fixed capabilities.

### 7.4 Out of scope

A general OxiIR to QPU compiler backend (2-4+ months, bank-conflict and delay-slot scheduling, VPM DMA tiling, TMU gather: rejected in the videocore report); KMS coexistence engineering; Pi 2/3 specifics; VideoCore VI/VII (Pi 4/5 have `v3dv` Vulkan and are covered by `wgsl`); `f64`, `i64`; `scan`, `filter`, `gather`, shared memory, atomics; any guest-visible QPU API or QPU bytes; running the OxiScript host as root.

### 7.5 Verification weight

The helper and the SKP verifier are the highest-risk components of the whole executor model. They go on the BAD ACTOR list with: malformed SKP fuzzing, buffer-table confusion (use after free, double free, handle forgery across clients), shape and stride overflow, timeout and hang recovery, helper-socket abuse (D-33: Claude pass).

### 7.6 `host:fft` next to compiled kernels

GPU_FFT (Andrew Holme, BSD-3, QPU-based FFT of sizes 2^8..2^22 on the Pi) is a **fixed host capability**, not an executor, per the roadmap's smallest demo. It is a `main` parameter, an audited host routine behind the same helper, and its IDL entries are marked `approx`:

```text
// proposed IDL shape (not valid OxiScript; documentation only)
interface Fft @1 {
  fn plan(log2n: u32, batch: u32, inverse: bool) -> Result<handle, i32>;
  fn exec(plan: handle, re: bytes, im: bytes) -> Result<void, i32>;   // approx(rel_ppm: measured)
}
```

Consequences: callers must be `@approx` (R-CLS-7); `safety` refuses it (R-CLS-8); the call is an `OXS-P0301` host transfer per call plus `OXS-P0309`; in Mode H it does not touch `Resident`. Compiled kernels and `host:fft` share the QPUs through the helper's single FIFO job queue (deterministic order, one job at a time, per-job timeout), so a kernel cannot starve or interleave with an FFT. The demo shows `host:fft` against the w2c2 CPU FFT, both numbers in the report and in OxiUI.

## 8. Test and gate plan

### 8.1 What "identical" means

| Class | Equality predicate | Corpus | Extra obligations |
|---|---|---|---|
| E | bit-identical result and identical trap kind, across scalar, simd128, wgsl (and qpu) | all programs, 5,000+ fuzz cases | none |
| C | `close` (R-CLS-3) against the scalar reference, **and** run-to-run bit-identical on one executor and host | curated well-conditioned corpus with a declared condition bound per entry | scalar equals simd128 **bit-for-bit** (both IEEE-exact) |
| A | within the declared bound of the scalar reference on every executor that may run it, **and** run-to-run bit-identical on one executor and host | exhaustive `2^32` sweep for unary `std/math/approx` functions; fuzz for others | different executors or hosts may legitimately differ in bits |

Cross-cutting tests:

* **Policy property test:** at least 10,000 combinations of (types, tier, accelerator, SIMD, profile, target, edition) give the same executor on repeat; the same-class rule (R-POL-3) is asserted on every pair of executors a threshold can choose between.
* **Debug versus release:** eligibility and `assertable` codes identical (R-OOB-1).
* **Deferred trap suite (R-OOB-4):** the same program with an out-of-range index run on CPU-over-`Resident` and on the GPU reports the same trap kind, span and lowest index; chunked dispatch keeps the lowest index; a program that never downloads still traps at the end of the turn; poisoned handles trap on reuse.
* **Reduction tree suite:** `reduceSum` on lengths `1..300`, powers of two and neighbours, equals the reference of R-L0-3 bit-for-bit on scalar and simd128; on `wgsl` equals under `close` with the structure checked by a probe kernel that uses non-associative values (e.g. subtraction-like patterns of `f32` crafted so that every distinct association gives a distinct result, to detect a driver or emitter that reassociates).
* **Sequential-equivalence CI** (8.3b): every program run sequentially and in parallel and diffed.
* **Policy log equals source:** logged transfers equal the source's `upload`/`download` calls.

### 8.2 Per-executor gates

* **scalar:** the reference; the existing corpus and the fuzzers.
* **simd128:** bit-identical to scalar on at least 5,000 cases across wasmi SIMD, wasmi scalar and Wasmtime (existing 8.3a gate), `relaxed_simd` forced off and tested (a `relaxed_madd` module is rejected); reductions follow R-L0-3.
* **wgsl:** naga validation with 0 errors; E identical, C `close`; the BAD ACTOR list of 8.3d (infinite loop stopped by the fuel counter, quota, device loss, OOB through every guard form, `compile_kernel` only from the module's own `oxi.kernels`).
* **qpu_vc4 (idea):** differential test against the scalar executor on the real Pi 1 B+; helper BAD ACTOR (7.5); sustained GFLOPS and dispatch latency recorded; promotion rule of 8.Mem.

### 8.3 Mapping onto the stages (amendments)

| Stage | Change |
|---|---|
| **8.3a** | add the executor descriptor and `Accept/Reject` hook to the vector IR work (a Rust trait in the IR crate, no behaviour yet); `Simd.sum` is defined as the canonical tree for its width (R-L0-3); state class E in the gate text ("bit-identical" is the class E predicate) |
| **8.3b** | the vocabulary of section 3 is the "spec pass" the stage waits on (Q-SIMD-18); the vectoriser shape S4 is **split**: integer reductions keep strided accumulators, float reductions must use the canonical tree or not vectorise (`P0106`) (R-L0-5); `scanInclusive/Exclusive` for integer operators and `reduceSum` land on CPU; `filter` CPU only; reason codes `P0103..P0112` as proposed; `zip` length precondition |
| **8.3c** | expose `ComputeTier` and the **domain facts** (`host_visible`, quotas) as the host half of R-POL-1; nothing else changes |
| **8.3d** | `Resident<T>` with R-EXE-7..11 including **sync points and the deferred failure channel** (so the timing semantics never change later); eligibility from `may_trap()` (R-EXE-3); class C deviation model and `close` gate (R-CLS-2/3); the 8.3d gate text "f32 within 1e-5 relative" is restated as the curated-corpus predicate; R-L0-4 emitter rules (tree order, no `subgroupAdd`, no float atomics); policy per section 6 with edition constants; `@approx` parse and check (`OXS-E0501..E0503`) because the first user is the `safety` rule itself and because class C needs the class vocabulary; `@expect(gpu)`, `@expect(exact)`; codes `P0302..P0306`, `P0308..P0310`; the R-OOB-1/2 rules always, R-OOB-3 enabled by the Q-SIMD-16 measurement |
| **8.3e** | `host:gpu` kernels use the language-defined guards (R-OOB-5); `std/tensor` documents its accumulation order (matmul ascending `k`); `std/math/approx` is **not** here (8.Lab) |
| **8.Lab** | `std/math/approx` cards with an exhaustive-sweep test per function; SWAR lowering of `Simd` on `device`/`micro` is a lowering of the scalar executor, class E, invisible to the policy |
| **8.Mem** | QPU probe as already planned, plus: test Mode V (`vcio2`) and the `&v3d`-disabled overlay; `host:fft` as the smallest demo marked `approx` |
| **8.3f (new, conditional)** | QPU executor of section 7, only if the 8.Mem probe passes and the owner approves the helper privilege model; gate: 8.2 qpu row |

No stage is removed. The roadmap rows 8.3a-e only gain the scope items above; reason codes are allocated append-only when each slice lands.

## 9. Open questions for the owner

1. **Q-SIMD-4 (still open).** Approve the amended proposal of section 5: (1) automatic path only proved-in-bounds, in every build mode; (2) deferred-trap semantics for all `Resident` operations, a core `gather` with a guard that is **enabled only if the Q-SIMD-16 measurement asks for it**, and a debug self-check of the prover; (3) `host:gpu` with language-defined zero/skip guards rather than WebGPU's driver-defined behaviour. *Recommendation:* approve. It keeps one program from ever behaving differently on GPU and CPU except by the declared classes, and it fixes a false premise (WebGPU does not return zero).
2. **Class C name and scope.** Approve three classes E, C, A with C implicit for `f32` on GPU and not user-visible beyond `safety` (no annotation needed for C). *Recommendation:* approve; the alternative (every GPU float kernel needs `@approx`) taxes ordinary tensor code with no extra information.
3. **`@approx` argument form.** Integer keys (`ulp: 8`, `rel_ppm: 1000`) with no grammar change, versus extending `AttrArg` with float literals (`rel: 1e-3`). *Recommendation:* integer keys now, float literals as a later additive grammar change.
4. **Canonical tree for floats costs speed on the GPU** (fixed addressing, no subgroup operations, no atomics). Accept as the price of run-to-run determinism and an auditable structure? *Recommendation:* accept; measure the cost in 8.3d and report it; offer an explicit `@approx` free-form reduction only if the cost is large.
5. **QPU access mode.** Mode H (privileged helper, copies, capabilities only) or Mode V (`vcio2` driver, host-visible, `Resident` domain)? *Recommendation:* decide after the probe; design for both; start with Mode H plus `host:fft` because it needs nothing beyond a small root daemon.
6. **Trusting the SKP path.** Accept that the QPU executor ships no guest machine code and that a malformed SKP is refused. *Recommendation:* accept as a hard rule; it is the only barrier in Mode V.
7. **New conditional stage `8.3f`.** Add it to the roadmap as conditional on the 8.Mem probe. *Recommendation:* add as an idea row only; no schedule until the probe.

## 10. Prior art and references

* Futhark GPU bounds checking: T. Henriksen, "Bounds Checking on GPU", HLPP 2020, https://www.futhark-lang.org/publications/hlpp20.pdf ; blog "How Futhark implements bounds checking on the GPU", https://futhark-lang.org/blog/2020-07-13-bounds-checking.html ; "Why Futhark (sometimes) goes wrong", https://futhark-lang.org/blog/2018-12-08-why-futhark-sometimes-goes-wrong.html
* WebGPU robust buffer access and its implementation variation (ledger A6): https://kvark.github.io/webgpu-debate/RBA.html
* WGSL floating-point evaluation (accuracy, flush to zero, reassociation and fusion), section 15.7: https://www.w3.org/TR/WGSL/#floating-point-evaluation (not re-read in this pass, `[S]`)
* Wasm relaxed SIMD and the deterministic profile (ledger A4): https://webassembly.github.io/spec/core/exec/numerics.html
* GPU_FFT (Andrew Holme, BSD-3), mailbox use, `vcio2` mention: http://www.aholme.co.uk/GPU_FFT/Main.htm
* VideoCore IV QPU tooling and the KMS conflict: py-videocore https://github.com/Idein/py-videocore ; VC4CL https://github.com/doe300/VC4CL ; vc4asm https://github.com/maazl/vc4asm ; Linux `vc4` DRM validator docs https://docs.kernel.org/gpu/vc4.html ; Broadcom VideoCore IV 3D Architecture Reference Guide https://docs.broadcom.com/doc/12358545
* Repository: `docs/research/oxiscript-simd-gpu-design.md` (+ verification), `docs/research/videocore-iv-acceleration.md` (+ verification), `docs/oxiscript/ir.md` section 6, `docs/oxiscript/contracts.md` section 3, `docs/roadmap/oxiscript.md` section 5.5.
