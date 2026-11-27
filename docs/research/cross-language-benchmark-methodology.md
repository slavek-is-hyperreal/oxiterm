# OxiPerf: Methodology for Fair Cross-Language Performance Comparisons (v0.1, Draft)

**Status:** Draft research specification for owner approval (feeds Stage `8.Bench` `oxiscript-perf`, Stage `8.Perf`, Stage `8.Mem`, and Stage `8.Lab` in [../roadmap/oxiscript.md](../roadmap/oxiscript.md)).  
**Scope:** Performance benchmarking of **OxiScript** (`wasmi` interpreter, `w2c2` + C AOT, Wasmtime AOT, and browser Wasm engines) against other languages and runtimes (TypeScript/JavaScript on V8 and in browsers, AssemblyScript, Rust native and Rust → Wasm, C, Go/TinyGo, Python/MicroPython, Lua/LuaJIT).  
**Naming boundary:** **OxiPerf** is the *performance* benchmark; **[OxiBench](oxibench-methodology.md)** is the *AI-agent usability* benchmark. The two share methodological discipline and selected harness utilities, never metrics or reports (Section 7).

> **Corrected by Claude (Opus), 2026-11-26:** verified claim by claim and judged as a methodology; the ledger and the judgment are in [cross-language-benchmark-methodology-verification.md](cross-language-benchmark-methodology-verification.md) (95 claims: 64 confirmed, 25 partly, 3 wrong, 3 unverifiable). The five changes that matter most: fix the 8.Lab regression gate (A1), use upstream AWFY ports instead of self-written competitor code (A2), headline only the AWFY subset (A3), derive run counts from a dimensioning run as Kalibera & Jones do (A4), and forbid FP contraction in C/w2c2 builds (A6).


---

## Executive Summary

**OxiPerf** is OxiScript's methodology and harness (Stage `8.Bench`, crate `oxiscript-perf` in [../roadmap/oxiscript.md](../roadmap/oxiscript.md#s-8-bench)) for measuring OxiScript across its own execution tiers (`wasmi` 2.0 interpreter, `w2c2` + C compiler AOT, Wasmtime AOT, and browser Wasm engines) and comparing it honestly against external languages and runtimes (TypeScript/JavaScript on V8 and browsers, AssemblyScript, Rust native and Rust → Wasm, C, Go/TinyGo, Python/MicroPython, and Lua/LuaJIT). It serves four roadmap consumers: **8.Perf** (RC elision and allocator fast paths), **8.Mem** (the [D-20](../roadmap/oxiscript.md#d-20) "megabytes, not gigabytes" claim on Raspberry Pi 1 Model B+ and x86), **8.Lab** ([D-29](../roadmap/oxiscript.md#d-29) weekly Speedup Lab promotion gates), and **8.3a–e** (SIMD/GPU acceleration).

Cross-language benchmarks routinely fail peer scrutiny because of hidden measurement bias, unreached JIT steady states, algorithmic divergence disguised as language differences, and cherry-picked reporting (van der Kouwe et al. found an average of 5 benchmarking flaws across 50 top-tier systems papers, with only 1 of 50 flaw-free; Barrett et al. found at most 43.5% of ⟨VM, benchmark⟩ pairs reliably reach a peak steady state). OxiPerf neutralises these failures through six non-negotiable pillars:

> **Corrected by Claude (Opus), 2026-11-26:** van der Kouwe et al.'s "average of 5" applies to **tier-1** papers; the abstract: "only a single paper in our sample" had no crimes (ledger B3).


1. **Dual-track fairness (SPEC Base/Peak + AWFY discipline):** Every benchmark runs in an **Identical-Algorithm (`base`) track**—same algorithm, same data structures, same arithmetic semantics where possible, same inputs, mandatory output verification, uniform compiler flags—and optionally in a clearly separated **Idiomatic/Optimised (`peak`) track**. No cross-language claim may mix tracks or pit an expert-tuned OxiScript program against a naive competitor.
2. **Multi-axis reporting, never wall-time-only:** Every run records **startup/cold time** separately from **steady-state time**, alongside **memory footprint** (RSS, PSS, peak linear-memory pages, peak heap/live bytes, MCU SRAM/PSRAM/Flash), **code size** (`.wasm` raw/gzip, stripped native binary, flash delta), **compile latency**, and **correctness checksums**. On `wasmi`, **fuel** is reported strictly as a deterministic metering budget and internal regression signal—**never** as a cross-language speed proxy—because `wasmi` 2.0 charges static region totals on entry rather than executed instructions ([oxiscript-memory-performance-verification.md](oxiscript-memory-performance-verification.md), Finding A1).
3. **Statistical rigour with nested variance:** Following Kalibera & Jones (2013) and Georges et al. (2007), OxiPerf executes multiple independent process/VM invocations (with randomised environment padding per Mytkowicz et al. 2009) and multiple iterations per invocation, tests warmup stability via changepoint/drift diagnostics (Barrett et al. 2017), and computes **medians, IQR/p95, and 95% hierarchical bootstrap confidence intervals for ratios**—never flat-pooled t-tests or arithmetic means of normalized ratios. A "win" requires both statistical separation (95% CI excludes `1.0`) and a practical effect-size threshold (`>= 5%` on wall time; `>= 10%` on cross-language headline claims).
4. **Honest hardware & runtime stratification:** OxiScript's `wasmi` interpreter is never silently presented as AOT, and `w2c2` AOT on `micro` (ESP32-S3, [D-27](../roadmap/oxiscript.md#d-27)/[D-28](../roadmap/oxiscript.md#d-28)) or `device` (Raspberry Pi 1 Model B+) is compared against native ceilings (C, Rust) and board peers (MicroPython, Lua, TinyGo) with explicit disclosure of OxiScript's mandatory safety invariants (trapping integer overflow, bounds checks, reference counting, deterministic floats per [P-9](../roadmap/oxiscript.md#p-9)).
5. **Preregistered, frozen reproducibility packages:** Borrowed directly from [OxiBench](oxibench-methodology.md), every public report is tied to a **freeze commit** pinning the methodology version, benchmark sources, input seeds, compiler/runtime versions and flags, hardware/OS manifest, raw JSON measurements, and analysis scripts. Negative results and native ceilings are always published.
6. **Strict claims policy & OxiPerf/OxiBench separation:** OxiScript websites may quote **only** frozen OxiPerf reports, using **ranges and confidence intervals** (never a single cherry-picked multiplier), naming exact competitor versions, flags, track, and hardware. **OxiPerf** measures runtime/compiler performance; **OxiBench** measures AI-agent usability. They share preregistration discipline and selected harness primitives, never leaderboards or reports.

---

## 1. Literature Review and Known Benchmarking Pitfalls

Cross-language performance evaluation has a 40-year literature documenting how well-meaning engineers publish false conclusions. Every rule in Section 2 traces directly to a documented failure mode below.

### 1.1 Benchmarking Crimes and Flaw Taxonomies
- **van der Kouwe, Heiser, Andriesse, Bos, and Giuffrida (2019), *"SoK: Benchmarking Flaws in Systems Security"*** (IEEE EuroS&P 2019, pp. 310–325, DOI: [10.1109/EuroSP.2019.00031](https://doi.org/10.1109/EuroSP.2019.00031); earlier arXiv preprint *"Benchmarking Crimes: An Emerging Threat in Systems Security"*, [arXiv:1801.02381](https://arxiv.org/abs/1801.02381); building on Gernot Heiser's living checklist [Systems Benchmarking Crimes](https://gernot-heiser.org/benchmarking-crimes.html)):
  - Catalogues **22 recurring benchmarking flaws** in five groups: (1) *selective benchmarking* (subsetting suites or inputs to hide overheads), (2) *improper handling of results* (microbenchmarks-only, arithmetic mean of ratios, ignoring latency/tail, no statistical significance), (3) *wrong benchmarks* (unrealistic workloads, calibrating on the evaluation set), (4) *improper comparison* (weak/stale baselines, self-comparison, handicapping competitors), and (5) *missing information* (omitting hardware, flags, sub-benchmark breakdowns, or absolute numbers).

> **Corrected by Claude (Opus), 2026-11-26:** 22 flaws is right; the five-group list is Gernot Heiser's page (18 crimes); the SoK adds a security-specific category (ledger B2).

  - Double-independent audit of **50 tier-1 defense papers** found an **average of 5 flaws per paper**, with **only 1 of 50** flaw-free.
  - *OxiPerf mitigation:* Rules R01–R04 (fixed suite, no post-hoc subsetting), R16–R18 (absolute + relative numbers, geometric mean with per-benchmark breakdown, statistical significance + effect-size thresholds), R19–R21 (mandatory negative results, full disclosure, preregistration freeze).
- **Hoefler and Belli (2015), *"Scientific Benchmarking of Parallel Computing Systems: Twelve Ways to Tell the Masses when Reporting Performance Results"*** (SC15, DOI: [10.1145/2807591.2807644](https://doi.org/10.1145/2807591.2807644)):
  - Formalises 12 reporting rules: report measurement counts and full environment, publish non-parametric summaries (median + percentiles/CIs) because system timings are skewed and heavy-tailed, never silently drop outliers, and match summary statistics to the distribution rather than assuming normality.
- **Fleming and Wallace (1986), *"How Not to Lie with Statistics: The Correct Way to Summarize Benchmark Results"*** (CACM 29(3):218–221, DOI: [10.1145/5666.5673](https://doi.org/10.1145/5666.5673)):
  - Proves that the **arithmetic mean of normalized ratios** is mathematically inconsistent: changing which system is chosen as the `1.0×` baseline can invert the ranking. Only the **geometric mean** preserves ranking invariance for normalized ratios (while noting that total time across a weighted workload uses arithmetic sums of raw times, not ratios).

### 1.2 Statistical Rigour, Hierarchical Variance, and Warmup Pathology
- **Georges, Buytaert, and Eeckhout (2007), *"Statistically Rigorous Java Performance Evaluation"*** (ACM OOPSLA '07, DOI: [10.1145/1297846.1297881](https://doi.org/10.1145/1297846.1297881)):

> **Corrected by Claude (Opus), 2026-11-26:** the DOI is wrong: `10.1145/1297846.1297881` is "The JastAdd extensible Java compiler". Georges et al. is [10.1145/1297027.1297033](https://doi.org/10.1145/1297027.1297033), OOPSLA'07 pp. 57-76 (ledger A9).

  - Demonstrated that JIT compilation, timer-driven profiling, GC heuristics, and OS scheduling introduce severe non-determinism in managed VMs. Ad-hoc practices (single runs, min/max cherry-picking, or iterating only inside a single VM invocation) yield **contradictory conclusions**.
  - Mandated separating **startup performance** (cold invocation including VM boot, loading, and initial compilation) from **steady-state performance**, executing **multiple independent VM invocations**, and reporting **confidence intervals** rather than point estimates.
- **Kalibera and Jones (2013), *"Rigorous Benchmarking in Reasonable Time"*** (ACM ISMM '13, pp. 63–74, DOI: [10.1145/2464157.2464160](https://doi.org/10.1145/2464157.2464160)):
  - Showed that benchmark observations have **nested (hierarchical) variance**: across builds/binaries, across fresh VM invocations, and across iterations within an invocation. Pooling iterations across invocations as if they were i.i.d. produces falsely narrow confidence intervals and high false-positive rates.
  - Introduced **hierarchical random resampling (hierarchical bootstrap)** to compute non-parametric confidence intervals for the **ratio of means** (or medians), and an analytical method to allocate run budget where variance is highest (more invocations when between-invocation variance dominates, fewer wasted inner iterations).

> **Corrected by Claude (Opus), 2026-11-26:** the ISMM'13 paper's summarising method is parametric (analysis of variance, effect-size CI); the hierarchical bootstrap for the ratio is in the companion report, Kalibera & Jones, "Quantifying performance changes with effect size confidence intervals", TR 4-12, Univ. of Kent, 2012 ([arXiv:2007.10899](https://arxiv.org/abs/2007.10899)) (ledger B10).

- **Barrett, Bolz-Tereick, Killick, Mount, and Tratt (2017), *"Virtual Machine Warmup Blows Hot and Cold"*** (PACMPL / OOPSLA '17, DOI: [10.1145/3133876](https://doi.org/10.1145/3133876), [arXiv:1602.00602](https://arxiv.org/abs/1602.00602)):
  - Applied automated **changepoint analysis** to long-running microbenchmarks across widely used VMs (HotSpot, V8, PyPy, LuaJIT, Truffle, CPython, etc.) on controlled hardware.

> **Corrected by Claude (Opus), 2026-11-26:** CPython was **not** studied. The systems were GCC 4.9.3, Graal 0.18, HHVM 3.15.3, JRuby+Truffle 9.1.2.0, HotSpot 8u112b15, LuaJIT 2.0.4, PyPy 5.6.0 and V8 5.4.500.43, with series of 2 000 in-process iterations (ledger A8, A5).

  - Refuted the textbook assumption that JIT benchmarks reliably warm up to a flat peak plateau: **at most 43.5%** of ⟨VM, benchmark⟩ pairs consistently reached a steady state of peak performance; many never stabilized, oscillated across deoptimizations, or slowed down over time.
  - *OxiPerf mitigation:* Never blindly discard a fixed `N` warmup iterations and declare "steady state". Record every iteration trace, run automated warmup/drift classification (stable steady state, non-converging/oscillating, or slowing), and report **both** fixed-window steady state and warmup status (Rule R09, R20).

### 1.3 Measurement Bias and Memory-Layout Sensitivity
- **Mytkowicz, Diwan, Hauswirth, and Sweeney (2009), *"Producing Wrong Data Without Doing Anything Obviously Wrong!"*** (ACM ASPLOS '09, DOI: [10.1145/1508244.1508275](https://doi.org/10.1145/1508244.1508275)):
  - Proved that tiny, seemingly irrelevant experimental parameters—specifically **Unix environment size** (shifting initial stack alignment) and **object link order** (shifting code/data cache-set and branch-predictor alignment)—cause performance swings large enough to **flip the sign of a compiler optimization** (`-O3` appearing slower than `-O2`, or vice versa). None of 133 surveyed papers across ASPLOS, PACT, PLDI, and CGO controlled for measurement bias.
  - Proposed **causal analysis** and **setup randomization** (deliberately varying environment padding and layout across invocations so systematic layout bias converts into measurable variance).
- **Curtsinger and Berger (2013), *"STABILIZER: Statistically Sound Performance Evaluation"*** (ACM ASPLOS '13, pp. 219–228, DOI: [10.1145/2451116.2451141](https://doi.org/10.1145/2451116.2451141)):
  - Extended Mytkowicz et al. by randomizing function, stack, and heap layouts dynamically at runtime, eliminating layout-induced bias and showing that claimed speedups smaller than layout noise are statistically indistinguishable from zero.
  - *OxiPerf mitigation:* Mandatory environment-size randomization across invocations (`OXIPERF_ENV_PAD` of randomized length) on Linux hosts, plus explicit effect-size floors so micro-architectural layout jitter is never published as a speedup (Rules R14, R18).

### 1.4 Existing Cross-Language Suites, Standards, and Energy Critiques
- **The Computer Language Benchmarks Game (CLBG)** ([benchmarksgame-team.pages.debian.net/benchmarksgame](https://benchmarksgame-team.pages.debian.net/benchmarksgame/)):

> **Corrected by Claude (Opus), 2026-11-26:** these criticisms are opinion without a source; CLBG documents its own caveats (crowd-sourced, self-selected contributors; isolated unloaded host; lowest of 12 runs), and library use is open per benchmark. CLBG measures on an i5-3330, the same generation as the owner's i5-3450, which makes it a useful sanity check (ledger B21).

  - Useful as a compiler stress test, but widely criticised as a cross-language ranking because: (1) community submissions evolve into heavily **unidiomatic C-transplants** using `unsafe`, manual pools, inline SIMD intrinsics, or custom allocators; (2) interpreted languages often delegate core work to **C libraries** (PCRE, GMP, OpenSSL), measuring FFI bindings rather than the language; (3) concurrency models diverge arbitrarily across submissions; and (4) entries suffer uneven author optimization effort across languages.
- **Rosetta Code and Chrestomathy-Style Comparisons** ([rosettacode.org](https://rosettacode.org/)):
  - Snippet collections written by different volunteer authors for didactic clarity rather than performance parity suffer extreme **author-skill and idiom confounding** (e.g., comparing an expert's hand-tuned C loop against a casual naive Python/JS snippet, or tiny `<50`-line toy loops that fit entirely in L1 cache and trigger dead-code elimination).
- **SPEC CPU 2017 Run and Reporting Rules** ([spec.org/cpu2017/Docs/runrules.html](https://www.spec.org/cpu2017/Docs/runrules.html); formally retired in November 2026 in favour of SPEC CPU 2026, with rules preserved as industry canon):
  - Sets the gold standard for publication integrity: mandatory **Base** track (uniform flags across the whole suite, no per-benchmark tuning, no FDO/PGO) separated from optional **Peak** track (per-benchmark tuning permitted); mandatory **output verification** against reference checksums; **full disclosure** of hardware, OS, compiler versions, and `.cfg`/flag definitions; automated `sysinfo` capture; reporting **medians** across independent iterations.
- **Are We Fast Yet (AWFY)** — Marr, Daloze, and Mössenböck (2016), *"Cross-Language Compiler Benchmarking: Are We Fast Yet?"* (ACM DLS '16, DOI: [10.1145/2989225.2989232](https://doi.org/10.1145/2989225.2989232); suite at [github.com/smarr/are-we-fast-yet](https://github.com/smarr/are-we-fast-yet); runner [github.com/smarr/ReBench](https://github.com/smarr/ReBench)):
  - Explicitly designed to fix CLBG's apples-to-oranges flaw by constraining **14 benchmarks** (5 macro: `CD`, `DeltaBlue`, `Havlak`, `Json`, `Richards`; 9 micro: `Bounce`, `List`, `Mandelbrot`, `NBody`, `Permute`, `Queens`, `Sieve`, `Storage`, `Towers`) to a **shared core of language abstractions** (objects, closures, arrays, basic arithmetic) and enforcing **identical algorithms** across implementations while remaining idiomatic within that shared subset.
  - *OxiPerf adoption:* OxiPerf adopts AWFY's identical-algorithm discipline for its `base` track and selects 6 well-characterized AWFY programs alongside 6 OxiScript/UI-relevant workloads (Section 4).
- **Energy Efficiency Studies and Critiques** — Pereira et al. (2017), *"Energy Efficiency across Programming Languages: How Do Energy, Time, and Memory Relate?"* (ACM SLE '17, DOI: [10.1145/3136014.3136031](https://doi.org/10.1145/3136014.3136031)) and Pereira et al. (2021), *"Ranking programming languages by energy efficiency"* (Science of Computer Programming 205:102609, DOI: [10.1016/j.scico.2021.102609](https://doi.org/10.1016/j.scico.2021.102609)), critiqued by **van Kempen, Kwon, Nguyen, and Berger (2024)**, *"It's Not Easy Being Green: On the Energy Efficiency of Programming Languages"* ([arXiv:2410.05460](https://arxiv.org/abs/2410.05460), DOI: [10.48550/arXiv.2410.05460](https://doi.org/10.48550/arXiv.2410.05460)):
  - Pereira et al. ranked 27 languages using CLBG/Rosetta snippets and Intel x86 RAPL counters, inheriting CLBG's micro-benchmark bias, conflating language specifications with specific runtimes (e.g., CPython vs. PyPy, treating TypeScript as a runtime distinct from V8), and relying on Intel-only RAPL estimates that omit peripherals and non-x86 architectures.
  - van Kempen et al. (2024) showed via causal analysis that once execution time, JIT warmup, GC, active core count, cache misses, and native FFI libraries are controlled, **language choice has no statistically significant intrinsic effect on energy beyond execution time** ($E = P \times t$).

> **Corrected by Claude (Opus), 2026-11-26:** the paper is to appear at ASE 2025 (arXiv v4, 2025-10-03). Its abstract names language vs implementation, application implementations, number of active cores and memory activity as confounders; "JIT warmup, GC, cache misses, FFI" are additions of this draft (ledger B29, B30).

  - *OxiPerf takeaway:* OxiPerf **never** claims "OxiScript is greener than language X" by extrapolating from wall time or x86 RAPL alone. If battery/power measurements are ever added on ESP32-S3 or Raspberry Pi, they must be measured directly at the power rail (e.g., INA219/USB power meter) alongside active-vs-sleep duty cycle, and attributed to the *runtime/implementation*, never the abstract language.

---

## 2. Fairness and Statistical Rules (Numbered Checklist)

Every OxiPerf run used for internal gating (`8.Perf`, `8.Lab`, `8.Mem`) or external publication must satisfy the applicable rules below. Violations invalidate a report.

### Track & Implementation Fairness
- **R01 — Mandatory Identical-Algorithm (`base`) Track:** Every cross-language benchmark must implement the exact same algorithm, asymptotic complexity, data layout category (e.g., array of structs vs. struct of arrays specified in the task spec), and problem parameters across all languages, following AWFY discipline. The `base` track is always mandatory and is the primary comparison table.
- **R02 — Optional Idiomatic/Optimised (`peak`) Track, Strictly Separated:** If a language has a strongly idiomatic alternative (e.g., Rust iterators/slices, JS `TypedArray` vs. boxed array, C manual arena, OxiScript `Simd<T,N>` or `rust:` crate), it may be entered in a separate `peak` track. `base` and `peak` results are **never** plotted on the same axis without explicit track labels, and a `peak` entry in OxiScript is never compared against a `base` entry in another language.
- **R03 — No Unlisted Shortcuts or Native FFI Smuggling in `base`:** In `base`, programs may not call out to native C/C++/Rust FFI libraries (`ctypes`, NumPy, `rust:<crate>`, Node native addons, regex engines backed by PCRE) to perform the benchmarked kernel. Standard-library data structures (`Map`, `Set`, `sort`) are allowed *only* on workloads explicitly designated to test standard libraries (Section 4), and any native/C backing must be explicitly tagged in the result table.
- **R04 — Semantic Parity Disclosure:** OxiScript enforces trapping integer overflow, array bounds checks, non-null soundness, and deterministic IEEE-754 floats ([P-9](../roadmap/oxiscript.md#p-9)). Where a competitor omits these checks by default (e.g., C `-O3` UB on signed overflow, Rust `--release` wrapping integer arithmetic, JS `f64` numbers instead of `i32`), OxiPerf **records the semantic delta explicitly in the table footnote** and, for Rust/C ceiling runs, optionally runs a paired checked variant (e.g., Rust `overflow-checks = true`, C `-ftrapv` or explicit bounds checks) so readers can separate compiler quality from safety-check cost.

> **Corrected by Claude (Opus), 2026-11-26:** for exact-bit float parity (R06) every C, C++ and w2c2 build must use `-ffp-contract=off` (GCC defaults to `fast` in GNU dialects and fuses `a*b+c` where FMA exists), never `-ffast-math`; JS ports of `f32` workloads (P07, P11, P12) need `Math.fround` on every operation (ledger A6).

- **R05 — Identical Inputs & Deterministic Seeds:** Every language receives byte-identical input files or generates identical inputs from the same documented PRNG (e.g., splitmix64/xorshift32 with a fixed seed specified in the benchmark's `spec.toml`). Inputs must not be hard-coded constants that allow constant folding at compile time; inputs are loaded from an external buffer/file or passed across an opaque boundary (e.g., CLI arg / host import / `black_box` barrier).
- **R06 — Mandatory Correctness Verification on Every Run:** Every benchmark computes a deterministic digest (e.g., FNV-1a 64-bit hash or exact expected scalar/string checksum) over its full output and returns/prints it **outside** the timed region (or uses it as a data-dependent sink preventing dead-code elimination). The harness checks this digest against the golden reference on **every invocation**; any mismatch invalidates the entire measurement and halts the run.

### Toolchain, Configuration & Execution Hygiene
- **R07 — Pinned, Uniform Flags and Full Disclosure:** Every compiler, interpreter, runtime, libc, and OS kernel version is pinned (exact version string + git hash for OxiScript and `w2c2`) and emitted into the run manifest. Following SPEC Base rules, all `base` benchmarks for a given ⟨language, target⟩ pair must share **one uniform flag set** (e.g., `clang -O2`, `rustc -C opt-level=3`, `oxi build --target server`). Per-benchmark flags are permitted only in `peak` and must be disclosed verbatim.

> **Corrected by Claude (Opus), 2026-11-26:** `-O2` is a reference, not a ceiling: label the C `base` row "C reference (-O2)" and keep "ceiling" for the `-O3` peak row, or use `-O3` uniformly (ledger C7).

- **R08 — Separate Startup (Cold) and Steady-State (Warm) Metrics:** Never conflate cold start with steady state (Georges et al. 2007). Every benchmark reports:
  1. **Compile/build time** (source → executable/Wasm artifact, offline or on-the-fly);
  2. **Cold invocation time** (process/VM boot + module load/instantiate + first execution of the workload);
  3. **Steady-state iteration time** (in-process repeated execution after warmup, with warmup status classified).
- **R09 — Warmup Verification, Not Blind Discard:** For JIT engines (V8, SpiderMonkey, JSC, Wasmtime JIT, LuaJIT, PyPy), execute a minimum warmup window (default: at least `max(50 iterations, 2 s)` or benchmark-specific count) **and** record the full per-iteration time series. Run an automated stability check (sliding-window drift + changepoint test inspired by Barrett et al. 2017): tag every ⟨runtime, benchmark⟩ series as `steady`, `slowdown`, `oscillating`, or `no_steady_state`. Non-steady series are **flagged visibly** in tables and excluded from unqualified "steady-state speedup" headlines.

> **Corrected by Claude (Opus), 2026-11-26:** about 80 points cannot be classified the way Barrett et al. did with 2 000-iteration series. For JIT rows run one long series (≥ 1 000 iterations, a few executions) per benchmark to choose and freeze the warmup length; report the classification from that series (ledger A5).

- **R10 — Mandatory Memory & GC/RC Reporting:** Execution time is meaningless if one runtime cheats by leaking memory (`--runtime stub`, `-gc=leaking`) or hoarding 10× RAM to avoid GC cycles. Every run records:
  - Peak OS **RSS** (and Linux **PSS/USS** where measurable via `/proc/<pid>/smaps_rollup`),
  - Runtime heap metrics where exposed (OxiScript peak Wasm linear-memory pages × 64 KiB, `oxi_live_objects()` / `oxi_live_bytes()` after `oxi_collect_cycles()` to verify zero leaks; V8 `heapUsed`/`rss`; MCU static `.bss`/`.data` + heap high-water mark),
  - Whether GC/cycle collection ran during timed iterations or between iterations (OxiScript collects cycles between host events per `abi.md:81`; for fair comparison, allocation-heavy benchmarks include both an **uncollected single-call** measurement and a **multi-event with full cycle collection / GC** measurement so reclamation cost is never hidden).
- **R11 — Determinism (`wasmi` Fuel) vs. Wall Time:** Per Verification Finding A1 ([oxiscript-memory-performance-verification.md](oxiscript-memory-performance-verification.md)), `wasmi` 2.0 charges fuel **statically per control-flow region on entry** (function entry, `loop`, `if` then-arm, `else`; `block`/`br_if` opens no region). Consequently:
  - Fuel is a **100% deterministic resource-metering metric** ideal for CI regression alerts and sandbox budgeting,
  - Fuel **must never** be used to compare OxiScript against another language or even to declare an OxiScript optimisation a "speedup" on its own (since reshaping `br_if` into `if` cuts fuel without reducing real work),
  - Every `wasmi` benchmark reports **three** numbers: `fuel`, `wall_ns`, and (in diagnostic mode) `executed_ops` (if instrumented) or AOT `w2c2`/`wasmtime` `wall_ns`.

> **Corrected by Claude (Opus), 2026-11-26:** also report wasmi wall time with fuel metering **off**; otherwise the interpreter row includes the metering tax (ledger C11).

- **R12 — Cold vs. Warm Hardware Caches Explicitly Controlled:** Distinguish cold-cache (first run after binary load / flash XIP cold on ESP32-S3 / Linux page-cache drop for startup benchmarks) from warm-cache steady-state runs. On ESP32-S3, explicitly record whether code runs from Flash XIP cache (`QIO`/`OPI`, cache size) or internal SRAM (`IRAM_ATTR`), and whether heap resides in internal 512 KB SRAM or external Octal PSRAM.
- **R13 — CPU Frequency Scaling, Thermal Stability, and Core Pinning:** On Linux hosts (`x86_64` dev box, Raspberry Pi 1):
  - Lock CPU governor to `performance` (`scaling_governor=performance`) and disable Intel Turbo Boost (`/sys/devices/system/cpu/intel_pstate/no_turbo=1`) during wall-time runs; record actual frequency and CPU temperature before and after the run. If root access is unavailable (e.g., shared CI runner), record `uncontrolled_dvfs = true` and **forbid** publishing those wall-time numbers externally.
  - Pin single-threaded benchmarks to a fixed isolated/quiet core (`taskset -c <core>`), and verify host load average `< 0.2` before starting.

> **Corrected by Claude (Opus), 2026-11-26:** the Pi 1 is single-core, so `taskset` does nothing there; stop services and record `vcgencmd get_throttled` before and after (the owner's board is corroded, under-voltage throttling is a real risk). The dev box runs concurrent agent sessions, so "load < 0.2" needs a scheduled quiet window (ledger A10, C13).

  - Interleave languages/configurations across invocations (round-robin or randomised order: `A, B, C, A, B, C...`) rather than running all `A` then all `B`, so thermal drift or background cron activity cannot systematically favour one language.
- **R14 — Mitigating Layout Bias (Mytkowicz et al. 2009):** Across independent process invocations on Linux, vary an unused environment variable `OXIPERF_ENV_PAD` across a deterministic pseudo-random set of lengths (e.g., 0 to 4096 bytes) so accidental stack-alignment anomalies convert from hidden bias into observable variance captured by confidence intervals. Record ASLR status (`/proc/sys/kernel/randomize_va_space`).

> **Corrected by Claude (Opus), 2026-11-26:** environment padding covers stack alignment only; link-order and code-layout bias (what Stabilizer randomises) remain and should be named as residual bias (ledger C14).


### Sample Size, Statistics, Outliers & Decision Thresholds
- **R15 — Hierarchical Sample Sizes (Minimum Run Counts):**

> **Corrected by Claude (Opus), 2026-11-26:** fixed counts contradict the cited method: Kalibera & Jones derive the repetitions at each level from a measured dimensioning run. Do one per ⟨engine, benchmark, machine⟩, compute the counts, freeze them; treat 10 invocations (Barrett et al.'s recommendation) as a floor. A bootstrap with 10 top-level units has poor coverage (ledger A4).

  - **Release / Publishable Report (Linux x86_64 / RPi):** At least **10 independent process/VM invocations** (`I >= 10`) × at least **30 measured steady-state iterations** per invocation (`N >= 30`, after warmup), with each single timed iteration lasting at least **10 ms** (via an explicit outer repeat count `inner_reps` chosen per benchmark so timer quantization is `< 0.1%`, and `< 1%` even under browser 100 µs coarsened timers). For startup benchmarks, at least **30 independent cold process invocations**.
  - **Microcontroller (ESP32-S3):** At least **5 cold resets/boots** (for startup/cold-cache) × **30 measured iterations** using `esp_timer_get_time()` or `esp_cpu_get_cycle_count()`, with FreeRTOS task pinning and Wi-Fi/BLE disabled during compute measurements (and separately enabled if measuring connected-device jitter).
  - **PR / CI Regression Check (`8.Lab` / `8.Perf` fast gate):** At least **3 invocations × 15 iterations** for wall time (flagged `ci_tier`, never publishable), plus exact deterministic `wasmi` fuel (1 invocation is exact).
- **R16 — Non-Parametric & Hierarchical Statistics:**
  - Report **median** as the primary location estimator, plus **IQR (p25–p75)**, **p95**, **min**, and **max** (and arithmetic mean + CV for comparability with legacy suites).
  - Compute **95% confidence intervals** for single runtimes and for **cross-language / before-after ratios** via **hierarchical bootstrap** (Kalibera & Jones 2013; **10,000 resamples**: resample invocations with replacement, then resample steady-state iterations within each chosen invocation).
  - For suite-wide summaries across benchmarks, report the **geometric mean of ratios** (Fleming & Wallace 1986) **only alongside the full per-benchmark breakdown and min–max range**—never as a lone headline number.
- **R17 — Strict Outlier Policy (Never Silently Drop):** Raw measurements are **never** deleted or trimmed silently. GC pauses, OS preemption spikes, and JIT deoptimizations are *real* runtime behaviour, not "measurement noise".
  - Report **all** measured post-warmup iterations in the raw JSON and in `p95`/`max`.
  - If an external system fault occurs (e.g., thermal throttle flag tripped, OOM kill, SSH disconnect, host load spike `> 1.0`), the **entire invocation** is marked `invalid` with an automated reason code and logged in the manifest—never cherry-picked iteration-by-iteration.
- **R18 — What Counts as a "Win" (Significance + Effect Size):** A comparison (`A` vs `B`) may be called faster/slower **only** if all four conditions hold:

> **Corrected by Claude (Opus), 2026-11-26:** add multiple-comparison control: with dozens of ⟨benchmark, competitor, target⟩ comparisons, about 1 in 20 null comparisons excludes 1.0 by chance. Pre-register the headline comparisons and use Holm-adjusted or simultaneous intervals for anything claimed from the family (ledger C18).

  1. Correctness digests match 100%;
  2. Warmup check did not flag severe `no_steady_state` without explicit caveat;
  3. The 95% hierarchical bootstrap CI for `ratio = A / B` **excludes `1.0`**;
  4. The point estimate exceeds the **Minimum Practical Effect Size (MPES)**: **`>= 5%`** (`ratio <= 0.95` or `>= 1.05`) for internal compiler/Speedup-Lab changes (`8.Perf`, `8.Lab`), and **`>= 10%`** (`ratio <= 0.90` or `>= 1.10`) for external cross-language claims (buffer against residual layout/compiler-flag variance). Smaller differences are reported as **"parity / within noise"** (`~1.0×`).

### Transparency, Negative Results & Reproducibility
- **R19 — Mandatory Publication of Negative Results and Ceilings:** Every frozen report must publish the **entire** configured suite—including benchmarks where OxiScript loses badly (e.g., `wasmi` interpreter vs. V8 TurboFan or native C, or cyclic-GC stress cases). Dropping a losing benchmark after seeing results is a benchmarking crime (van der Kouwe et al. 2019).
- **R20 — Raw Data & One-Command Reproducibility Package:** Every published report ships a versioned archive (committed or linked release asset) containing: (a) frozen benchmark source code for all languages, (b) exact input generator/checksums, (c) complete `manifest.json` (hardware, OS, kernel, CPU governor, compiler/runtime versions and flags, git SHA), (d) unaggregated per-invocation, per-iteration `raw.jsonl`, (e) deterministic analysis script that regenerates every table and CI directly from `raw.jsonl`, and (f) a Dockerfile / `nix` flake / setup script reproducing the Linux x86_64 environment.
- **R21 — Preregistration Freeze Before External Runs:** Mirroring [OxiBench §7](oxibench-methodology.md#7-freeze-preregistration), before executing a publishable cross-language run, a **freeze commit** locks the methodology version, benchmark sources, competitor list, compiler flags, input sizes, and analysis script. Any change after the freeze increments the methodology/suite version; results from different versions are **never** mixed in one table.

---

## 3. OxiScript-Specific Measurement Axes and Competitor Protocols

OxiScript is neither a single-engine VM nor a bare-metal-only compiler: one `.oxs` program compiles to Core WebAssembly and runs across four distinct execution environments ([../roadmap/oxiscript.md](../roadmap/oxiscript.md#p-7), [D-27](../roadmap/oxiscript.md#d-27), [D-28](../roadmap/oxiscript.md#d-28)). A fair benchmark must never blur these tiers.

### 3.1 Execution Tiers (Never Conflate Interpreter, AOT, and Browser Engines)

| OxiScript Execution Tier | Target Flag | Where Used | Runtime Properties Affecting Measurement | Fair Comparison Framing |
|---|---|---|---|---|
| **`wasmi` 2.0.3 interpreter** (`CompilationMode::Eager`) | `device` / `server` / `web` | Live reload on servers, OxiUI local dev, browser simulator (wasm-in-wasm) | Register-based tail-call interpreter; deterministic region-based fuel metering ([Finding A1](oxiscript-memory-performance-verification.md)); no JIT warmup; `Vec<u8>` linear memory (`memory.grow` may copy). | Compare against **interpreters** (CPython, Lua 5.4, V8 `--jitless` diagnostic) for interpreter peers, AND against **JIT/AOT** (V8 default, Wasmtime, native C/Rust) to honestly report the interpreter tax. Never imply `wasmi` is release AOT speed. |
| **`w2c2` + C compiler AOT** (GCC / Clang, `-O2` base) | `micro` (mandatory, [D-27](../roadmap/oxiscript.md#d-27)), `device` (candidate, [8.Mem](../roadmap/oxiscript.md#s-8-mem)) | ESP32-S3 firmware, Raspberry Pi 1 B+, safety route | Native machine code with no JIT warmup; **no `wasmi` fuel** (uses compiler-inserted budget checks at loop back-edges/calls, [8.2b](../roadmap/oxiscript.md#s-8-2b)); on 32-bit ARMv6/Xtensa lacks 4 GiB `mmap` guard pages so linear-memory loads/stores are explicitly bounds-checked. | Compare against **native ceilings** (C, Rust, TinyGo) and **MCU peers** (MicroPython, Lua 5.4). Report with and without budget checks if isolating sandbox cost. |
| **Wasmtime / Cranelift AOT** (precompiled `.cwasm` & JIT) | `server` | 64-bit servers and desktops | Native 64-bit AOT/JIT with 4 GiB virtual-memory guard pages (zero explicit bounds-check branches); deterministic `simd128` (no relaxed SIMD, NaN canonicalisation per [8.3a](../roadmap/oxiscript.md#s-8-3a)). | Compare against **server JIT/AOT runtimes** (Node.js/V8, Go, Rust→Wasm on Wasmtime, AssemblyScript on Wasmtime) and native Rust/C ceilings. |
| **Browser native Wasm** (V8 Liftoff+TurboFan, SpiderMonkey Ion, JSC OMG) | `web` | Production OxiScript on web pages (`<script lang="oxs">`) | Two-tier Wasm JIT (fast streaming baseline + optimising backend); timer resolution clamped to **100 µs** without cross-origin isolation vs. **5 µs** with `COOP`/`COEP` headers ([MDN `performance.now()`](https://developer.mozilla.org/en-US/docs/Web/API/Performance/now), [web.dev COOP/COEP](https://web.dev/articles/coop-coep)). | Always serve the browser harness with `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` (`crossOriginIsolated === true`). Compare against **Browser JS** and **Rust/AssemblyScript Wasm in the same browser**. Keep **playground wasm-in-wasm** (`wasmi` inside browser Wasm) in a separate row—never mix it with native browser Wasm. |

### 3.2 Hardware Target Matrix

OxiPerf stratifies measurements across three physical hardware classes tied directly to the roadmap:

1. **Desktop / Server Reference (`server` / `web`):** Currently the project dev box (**Intel Core i5-3450**, 4c/4t x86_64, Linux; software `llvmpipe` GPU until real GPU hardware is selected per [Q-SIMD-6](../roadmap/oxiscript.md#q-simd)). Must lock `scaling_governor=performance`, disable Turbo (`no_turbo=1`), pin to one core (`taskset`), and record exact kernel/microcode.
2. **Small Linux Board (`device`, Stage `8.Mem`, [D-20](../roadmap/oxiscript.md#d-20) / [D-28](../roadmap/oxiscript.md#d-28)):** Owner's **Raspberry Pi 1 Model B+** (Broadcom BCM2835, single-core ARM1176JZF-S ARMv6Z @ 700 MHz, VFPv2, **no NEON SIMD**, 512 MB LPDDR2, target `arm-unknown-linux-gnueabihf`).
   - *Specific measurement rules:* Single-core hardware means background SSH/systemd daemons directly perturb timings—stop non-essential services, record `/proc/loadavg` and SoC temperature (`vcgencmd measure_temp`) before/after, and disable HDMI/desktop compositor during headless compute runs (enable separately for Stage `8.Mem` framebuffer UI benchmarks). Always test **both** `wasmi` and `w2c2` + GCC on identical workloads to empirically settle the `device` engine choice ([D-28](../roadmap/oxiscript.md#d-28)).
3. **Microcontroller Class (`micro`, Stage `8.S3`, [D-27](../roadmap/oxiscript.md#d-27) / [D-28](../roadmap/oxiscript.md#d-28)):** Planned **ESP32-S3 N16R8** board (dual-core Xtensa LX7 @ up to 240 MHz, 512 KB internal SRAM, 16 MB SPI Flash with XIP instruction/data cache, 8 MB Octal PSRAM; [Espressif ESP32-S3 Datasheet](https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf)).
   - *Specific measurement rules:* Run `w2c2` + ESP-IDF GCC native firmware only (`wasmi` is not shipped on MCUs per [D-27](../roadmap/oxiscript.md#d-27)). Measure wall time via `esp_timer_get_time()` (1 µs resolution; [ESP-IDF High Resolution Timer](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/system/esp_timer.html)) and cycles via `esp_cpu_get_cycle_count()`. Pin benchmark FreeRTOS task to Core 1 with Wi-Fi/BLE off (separate "radio active" profile if testing connected jitter). Disclose Flash mode (`QIO`/`OPI`), XIP cache size (32 KB / 64 KB), `IRAM_ATTR` placement (forbidden in `base` unless applied uniformly), and whether the heap sits in internal SRAM vs. Octal PSRAM (PSRAM misses are orders of magnitude slower than SRAM).

### 3.3 Non-Time Axes (Memory, Code Size, Compile & Startup Latency)

Because OxiScript's core positioning is **"GUI and scripting for devices with megabytes, not gigabytes, of memory"** ([D-20](../roadmap/oxiscript.md#d-20)) and fast developer iteration ([8.S2](../roadmap/oxiscript.md#s-8-s2), [8.S3](../roadmap/oxiscript.md#s-8-s3)), time alone is insufficient. Every publishable OxiPerf run reports four orthogonal axes:

| Axis | Linux (`server` / `device`) Metrics | Browser (`web`) Metrics | Microcontroller (`micro`, ESP32-S3) Metrics |
|---|---|---|---|
| **1. Execution Time** | Cold invocation (ms); steady-state median/IQR/p95 (µs or ms); `wasmi` deterministic fuel (separately from wall time). | Cold load+first-run (ms); steady-state median/p95 with `crossOriginIsolated === true`. | Cold-cache first run (XIP miss); warm-cache steady-state median/p95 (µs & CPU cycles). |
| **2. Memory Footprint** | Peak **RSS** (`ru_maxrss`), steady-state **PSS & USS** (`/proc/<pid>/smaps_rollup`), peak OxiScript linear-memory pages (`pages × 64 KiB`), post-`oxi_collect_cycles()` `live_objects`/`live_bytes` (leak check = 0), GC/CC headroom delta. | `WebAssembly.Memory.buffer.byteLength` peak; `performance.measureUserAgentSpecificMemory()` (COOP/COEP, Chrome; labelled browser-specific). | Static `.data` + `.bss` (`idf.py size`); runtime heap high-water mark (`heap_caps_get_minimum_free_size` for `MALLOC_CAP_INTERNAL` SRAM and `MALLOC_CAP_SPIRAM` PSRAM separately); stack high-water mark (`uxTaskGetStackHighWaterMark`). |
| **3. Code & Artifact Size** | `.wasm` raw & `gzip -9` / `brotli -11` bytes (split: user code vs. `oxiscript-rc` vs. `std/*`); stripped native ELF size (`w2c2`+GCC, Rust, C, Go/TinyGo). | Total wire transfer size (JS glue + `.wasm`, raw & compressed) for first load. | Flash `.text` + `.rodata` delta (`idf.py size-components`) compared to empty ESP-IDF firmware skeleton. |
| **4. Compile & Startup Latency** | OxiScript compile time (`parse + check + lower + wasm`, ms & LOC/s); `wasmi` eager translation time; Wasmtime AOT compile vs. `.cwasm` load; `w2c2`+GCC build time. | `WebAssembly.compileStreaming` + `instantiate` latency (ms). | Total incremental build + `w2c2` + GCC + link + flash time (tracks [8.S3](../roadmap/oxiscript.md#s-8-s3) edit-to-visible target). |

### 3.4 Fair Comparison Protocols by Competitor Language Family

| Competitor | Engines / Versions to Pin | Fairness Traps to Avoid | Mandatory OxiPerf Protocol |
|---|---|---|---|
| **TypeScript / JavaScript** | Node.js LTS (V8 version pinned); Chrome/Firefox/Safari versions on `web`. (Deno/Bun optional secondary rows.) | (1) Treating TS as a separate runtime from JS (Pereira et al. flaw)—TS compiles to JS with zero runtime semantics. (2) Crippling V8 with `--jitless` or `--no-opt` in headline tables. (3) Measuring cold Ignition/Sparkplug runs as "JS speed" without letting Maglev/TurboFan warm up ([V8 Maglev blog](https://v8.dev/blog/maglev)). (4) Accidental megamorphic object shapes or boxed `Array<number>` vs. `Int32Array`/`Float64Array`. | Strip TS types ahead of time (`tsc` / `esbuild` time reported in Compile axis, not Run axis). Run V8 with **default flags**; verify warmup stability (R09) and inspect `--trace-deopt` in a validation pass so the JS port has no accidental deopt bugs. Use `Int32Array`/`Float64Array` in `base` for numeric array workloads matching OxiScript's packed `i32[]`/`f64[]` (and document JS's `f64` scalar semantics vs. OxiScript `i32`). Report V8 RSS alongside speed—V8's multi-tens-of-MB baseline RSS is a legitimate axis difference under [D-20](../roadmap/oxiscript.md#d-20). |
| **AssemblyScript** | Exact `asc` version + Binaryen version; `--optimize` (`-O3` / `-Oz` disclosed). | (1) Comparing `--runtime stub` (leaking bump allocator) against OxiScript `RuntimeMode::Linked` (full RC + cycle collector). (2) Running AssemblyScript on V8 while running OxiScript on `wasmi`. | In `base`, use `--runtime incremental` (TLSF + ITCMS tracing GC; [AssemblyScript Runtime Docs](https://www.assemblyscript.org/runtime.html)) for managed-memory parity with OxiScript Linked. `--runtime stub` may be compared **only** against OxiScript `RuntimeMode::Stub`. Execute both `.wasm` artifacts on the **exact same Wasm engine** (`wasmi`, Wasmtime, `w2c2`, and browser Wasm). |
| **Rust (Native & → Wasm)** | Exact `rustc`/LLVM version; `wasm32-unknown-unknown` and host triple; explicit `opt-level`, `lto`, `codegen-units`, allocator (`dlmalloc`/`talc`/`rlsf`). | (1) Default `--release` turns **off** integer overflow checks, whereas OxiScript traps on overflow ([P-9](../roadmap/oxiscript.md#p-9)). (2) Using `unsafe { get_unchecked }` in `base`. | Rust serves as both the **native ceiling** and the **Wasm/LLVM codegen ceiling**. In `base`: no `unsafe`, uniform `--release` (`opt-level = 3`, `lto = "thin"`). On arithmetic-heavy workloads, run **both** standard `--release` (ceiling) and `--release` with `overflow-checks = true` (semantic-parity reference). For `wasm32`, run Rust's `.wasm` on the exact same engines (`wasmi`, Wasmtime, `w2c2`, browser) as OxiScript. |
| **C (Native Ceiling)** | Exact `gcc` and/or `clang` version; `-O2` (`base`) and `-O3` (`peak` ceiling). | Comparing C's unchecked pointer arithmetic and UB-on-overflow against OxiScript's bounds-checked, overflow-trapping, RC-managed code without disclosing the safety gap. | C is explicitly labelled **"Native Unchecked Ceiling (no bounds/overflow/RC)"**. Crucially, compile OxiScript's `w2c2`-generated C with the **exact same C compiler, version, libc, and `-O2` flags** as the hand-written C baseline—this isolates the exact tax of Wasm linear-memory bounds checks + OxiScript RC + budget checks. |
| **Go & TinyGo** | Exact `go` version (Linux `server`/`device`) and `tinygo` version (`wasm` / `esp32s3`). | Comparing `tinygo -gc=leaking` (leaks all heap) against OxiScript Linked RC, or conflating multi-MB standard Go Wasm binaries with TinyGo. | Use standard Go on Linux (`device`/`server`), reporting full RSS and stripped binary size. Use TinyGo with `-gc=conservative` (default managed GC; [TinyGo Heap Allocation](https://tinygo.org/docs/concepts/compiler-internals/heap-allocation/)) in `base` on Wasm and ESP32-S3; report `-gc=leaking` only in a labelled leaking/stub row. |
| **Python & MicroPython** | Exact CPython 3.x (and optional PyPy 3.x) on Linux; exact MicroPython firmware version on ESP32-S3. | (1) Calling NumPy/C-extensions in `base` (measuring C/BLAS instead of Python). (2) Comparing CPython interpreter against OxiScript AOT while hiding PyPy, or vice versa. | Pure Python in `base` (R03); NumPy/C-backed libraries allowed only in `peak` with explicit `C-FFI` badge. On ESP32-S3, MicroPython bytecode is the primary `base` MCU scripting peer; `@micropython.native` / `@micropython.viper` go in `peak` (disclosing viper's dropped safety/dynamic checks). |
| **Lua & LuaJIT** | Exact Lua 5.4.x (`device`, `server`, `micro`) and LuaJIT 2.1 (`server`, x86_64). | Claiming Lua is slow by testing only PUC-Rio Lua 5.4 on x86_64 without noting LuaJIT, or assuming LuaJIT exists on ARMv6/ESP32-S3. | Report PUC-Rio Lua 5.4 everywhere (including ESP32-S3 and RPi 1—a direct embedded peer for `wasmi` and `w2c2`), and LuaJIT 2.1 on x86_64 with standard warmup verification (R09). Explicitly note platform availability (LuaJIT does not target Xtensa ESP32-S3). |

> **Corrected by Claude (Opus), 2026-11-26:** TS/JS row: the upstream AWFY JavaScript ports use plain `Array` (e.g. `sieve.js`), so "reuse AWFY ports" and "`Int32Array` in `base`" are mutually exclusive; decide once (ledger D11). Node.js on the Pi 1 exists only as unofficial armv6l builds with "minimal or no testing" (D12). Go/TinyGo row: TinyGo's Wasm default GC is `precise` (`wasm.json`), Xtensa/ESP32-S3 is `conservative`; TinyGo has an `esp32s3-n16r8-devkit` target (D16). Lua row: **LuaJIT supports 32-bit ARM from ARMv5**, so it runs on the Pi 1 B+ and belongs in the `device` roster; it does not target Xtensa (ledger A7). Lua 5.4 on the ESP32-S3 needs a self-built port (D19).


---

## 4. OxiPerf Concrete Benchmark Suite Proposal (v0.1)

To avoid the "toy problem" trap of Rosetta Code and the bloat of unmaintainable 50-program suites, **OxiPerf v0.1** starts with **12 self-contained programs**: **6 adapted from Are We Fast Yet (AWFY)** ([Marr et al. 2016](https://doi.org/10.1145/2989225.2989232), [github.com/smarr/are-we-fast-yet](https://github.com/smarr/are-we-fast-yet)) for academic comparability and compiler coverage, plus **6 UI- and OxiScript-specific workloads** covering page layout, string/template rendering, deterministic sorting, hash collections, 2D physics, and neural-network inference (directly unifying the `8.Bench`, `8.Perf`, `8.Mem`, and `8.Lab` pipelines in [../roadmap/oxiscript.md](../roadmap/oxiscript.md)).

Each program ships with **three problem-size profiles** (`micro` for ESP32-S3, `device` for RPi 1 B+, `server` for x86_64/browser) so every benchmark runs in the 10–500 ms per-iteration sweet spot without OOMing 512 KB SRAM or taking hours on ARMv6. Cross-hardware ratios are computed only within the same size profile.

> **Corrected by Claude (Opus), 2026-11-26:** 10-500 ms per iteration cannot hold for every row at one fixed size (C vs CPython or wasmi differ 50-100×); state the expected range per row. Freeze sizes and `inner_reps` before the first measured run (ledger E10, A3).


### 4.1 OxiPerf v0.1 Core Suite (12 Programs)

| ID | Name | Origin | Primary Bottlenecks Exercised | Identical-Algorithm (`base`) Rules | Size Profiles (`micro` / `device` / `server`) | Correctness Oracle |
|---|---|---|---|---|---|---|
| **P01** | **`mandelbrot`** | AWFY / CLBG | `f64` scalar arithmetic, tight nested loops, bit-packing, zero heap allocation. | Exact AWFY `Mandelbrot` escape-time loop (`max_iter = 50`, `limit = 4.0`, packed 8-bit rows); no SIMD in `base` (SIMD goes to `peak` / 8.3a). | `128×128` / `250×250` / `500×500` px | Exact bit-packed checksum (matches AWFY verify values). |
| **P02** | **`nbody`** | AWFY / CLBG | `f64` arithmetic + `sqrt`, struct/object field loads & stores, inner $O(n^2)$ loop, zero alloc in hot loop. | Exact AWFY 5-body solar system symplectic Euler integrator (`dt = 0.01`); explicit array of 5 `Body` objects/structs; no fast-math reassociation. | `5,000` / `50,000` / `250,000` steps | Final system energy rounded/compared within `1e-12` (exact IEEE-754 bits on deterministic targets). |
| **P03** | **`sieve`** | AWFY | `i32` array indexing, bounds-check overhead, predictable branches, cache locality. | Exact AWFY Sieve of Eratosthenes over boolean/`u8` array of size $N$ (no wheel factorization, no bitset packing in `base`). | `N = 5,000` / `50,000` / `500,000` | Exact prime count (`669` / `5,133` / `41,538`). |
| **P04** | **`storage`** | AWFY | Pure allocation throughput, recursive tree-of-arrays construction, RC/GC pressure & reclamation. | Exact AWFY `Storage` (deterministic LCG seed `74755`, depth 7, branching 4 arrays); includes **both** peak memory and full reclamation (`oxi_collect_cycles` / GC) accounting. | `depth = 5` / `6` / `7` | Exact total allocated cell count (`5461` at depth 7). |
| **P05** | **`json_parse`** | AWFY (`Json`) | Byte/UTF-8 scanning, recursive-descent parsing, string slicing, discriminated-union / AST node allocation, RC/GC churn. | Exact AWFY recursive-descent JSON parser (derived from `minimal-json`) over a fixed deterministic JSON fixture; **no `JSON.parse` / `serde_json` native calls in `base`** (native parsers allowed only in `peak` tagged `stdlib-native`). | `8 KB` / `64 KB` / `512 KB` JSON fixture | Deterministic AST walk checksum (node count + FNV-1a of keys/values). |
| **P06** | **`deltablue`** | AWFY | Polymorphic/class dispatch, object graph traversal, small-object churn, cyclic/bi-directional references (stress-tests OxiScript's Bacon-Rajan cycle collector & `#acyclic` analysis). | Exact AWFY `DeltaBlue` incremental constraint solver (`chainTest` + `projectionTest`); preserves cyclic `Variable ↔ Constraint` links. | `n = 50` / `200` / `1,000` variables | Exact final variable sum & plan length. |
| **P07** | **`page_tree_layout`** | OxiUI / OxiScript | Tree construction, two-pass flex/box layout (measure + arrange), COW `struct` updates (`Rect`, `Style`), recursive traversal, dirty-flag diffing. | Fixed deterministic UI tree (depth 6, mixed row/column containers + leaf text boxes); two-pass layout computing `(x, y, w, h)` in `f32`/`i32` cells + dirty-subtree update pass (10% nodes mutated per tick × 20 ticks). Directly models OxiUI `<script lang="oxs">` ([8.S1](../roadmap/oxiscript.md#s-8-s1), [8.Mem](../roadmap/oxiscript.md#s-8-mem)). | `200` / `1,000` / `5,000` nodes | FNV-1a checksum of all final `(x, y, w, h)` bounds. |
| **P08** | **`string_template`** | OxiScript (`8.Perf` B6) | UTF-8 string building, integer/float formatting (`itoa`/`ftoa`), template interpolation, short-lived string alloc/free churn. | Render $K$ UI rows of the form `` `row #${i}: ${label} (${x}, ${y}) delta=${d}` `` into a single output buffer/array and hash it; identical integer/float formatting precision (2 decimals). Directly tracks BL06's `template string` bottleneck ([oxiscript-memory-performance-verification.md](oxiscript-memory-performance-verification.md) A4). | `K = 250` / `2,000` / `10,000` rows | Exact byte length + FNV-1a 64-bit hash of rendered UTF-8 bytes. |
| **P09** | **`merge_sort_det`** | OxiScript (`8.Std` / `8.Perf` BS07) | Generic comparison callback/closure overhead, array copying, deterministic stable sorting. | Top-down or bottom-up **stable merge sort** over $N$ `i32` keys (and a second variant over `{ key: i32; id: i32 }` structs to verify stability) generated by fixed xorshift32 seed. In `base`, every language implements the **exact same merge sort** (no stdlib Timsort/pdqsort); stdlib `sort` runs in `peak`. Tracks BS07's 30k-element fuel budget ([../roadmap/oxiscript.md](../roadmap/oxiscript.md#s-8-perf)). | `N = 2,000` / `10,000` / `30,000` items | Sortedness check + stability checksum over `(key, id)` pairs. |
| **P10** | **`map_set_churn`** | OxiScript (`8.Std` / `8.Perf` B7) | Hash table insert/lookup/delete, string hashing, dynamic bucket resizing, insertion-order preservation. | Two sub-modes: (`base-algo`) identical open-addressing linear-probing hash map over `i32` and short `string` keys written in each language; (`base-stdlib`) each language's built-in insertion-ordered `Map`/`Set` (tagged `stdlib`) performing $M$ deterministic insert/lookup/update/delete ops. | `M = 1,000` / `10,000` / `50,000` ops | Final map size + ordered iteration FNV-1a digest. |
| **P11** | **`physics_2d_step`** | OxiScript (`8.Bench` particles) | `f32` vector math, AoS vs. SoA memory bandwidth, spatial-hash or $O(n^2)$ pairwise collision impulse, integer-to-float conversions. | $P$ 2D particles in a bounded box updated for $S$ ticks with semi-implicit Euler integration, wall bounce, and pairwise elastic collision using a uniform grid cell hash. Maps directly to `8.Bench`'s `particles` workload; scalar in `base`, `Simd<f32, 4>` / GPU in `peak` ([8.3a–d](../roadmap/oxiscript.md#s-8-3a)). | `P = 128, S = 50` / `P = 512, S = 100` / `P = 2,048, S = 100` | Checksum of quantized final positions & total kinetic energy (within `1e-5` on GPU/f32-relaxed `peak`; exact bits on deterministic `base`). |
| **P12** | **`tiny_mlp_layer`** | OxiScript (`8.Bench` MLP / `8.W3` L5) | Dense `f32` matrix-vector/matrix-matrix multiply-accumulate, ReLU activation, softmax (`exp` determinism per [8.Std](../roadmap/oxiscript.md#s-8-std)), contiguous array indexing. | 2-layer MLP forward pass (`784 → 128 → 10` for `device`/`server`; `64 → 32 → 10` for `micro` SRAM) over deterministic pseudo-random weights and batch of $B$ inputs. Directly connects to `8.Bench` `MLP` and Course Level 5 MNIST ([8.3e](../roadmap/oxiscript.md#s-8-3e)); scalar loops in `base`, SIMD128 / WebGPU / BLAS only in `peak`. | `64→32→10, B=8` / `784→128→10, B=16` / `784→128→10, B=64` | Argmax vector + sum of logits within `1e-5` relative error (exact bit hash on CPU deterministic `base`). |

> **Corrected by Claude (Opus), 2026-11-26:** suite table: AWFY golden values exist only for its own sizes (Mandelbrot 500 → 191, NBody 250 000 steps → -0.1690859889909308, Storage depth 7 → 5461, Sieve 5000 → 669); every other profile needs new goldens (Storage depth 5 → 341, depth 6 → 1 365). AWFY Json parses one fixed embedded document, so 8/64/512 KB fixtures are a new benchmark. Headline cross-language claims only on P01-P06; P07-P12 are our own choice and must be labelled "home workloads" with a separate geomean (ledger A3, E2-E6).


### 4.2 Planned Roadmap Extensions (Not Blocking v0.1)

To preserve full continuity with `docs/roadmap/oxiscript.md` without overloading v0.1:
- **v0.2 Data-Parallel Extension (Stage `8.3a–d`):** Adds `gemm_f32` (tiled dense matrix multiply) and `image_filter_3x3` (RGBA8 convolution) from `8.Bench`'s original four SIMD/GPU workloads, measuring both **GPU-resident** and **CPU↔GPU round-trip** latency plus `placement.json` `@expect` verification ([P-5](../roadmap/oxiscript.md#p-5)).

> **Corrected by Claude (Opus), 2026-11-26:** roadmap 8.Bench owns GEMM and the image filter and its gate blocks 8.3a, 8.3b and 8.3d; deferring them is fine only if 8.Bench keeps them in its SIMD/GPU part (ledger E9).

- **v0.2 Embedded UI Frame Extension (Stage `8.Mem` / `8.S1`):** Adds end-to-end OxiUI `<script lang="oxs">` frame-time & RSS benchmarks (full redraw vs. damage-rect diff on Raspberry Pi 1 framebuffer and browser local mode).
- **Internal-Only RC Micro-Suite (`8.Perf`):** Keeps the existing OxiScript-internal micro-probes (`B1–B9` from [oxiscript-memory-performance.md §6.1](oxiscript-memory-performance.md#61-proposed-microbenchmark--macrobenchmark-suite-oxiscript-perf--oxiscript-rc--oxiscript-wasm), e.g., `new class`, `[1,2,3]`, cycle headroom at 90% heap) inside `oxiscript-perf --suite internal-rc`; these drive `8.Perf` engineering and are **never** marketed as cross-language benchmarks.

### 4.3 Per-Language Implementation Verification Gate

Before any language's implementation of `P01..P12` enters a frozen OxiPerf release:
1. **Spec conformance review:** A second reviewer (human or separate agent instance, mirroring [OxiBench Role R2](oxibench-methodology.md#1-principle-nobody-grades-their-own-work)) audits the `base` implementation against `spec.toml` to verify identical algorithm, no native FFI smuggling, no missing bounds/work, and idiomatic quality within the shared subset (no deliberately crippled competitor code—a classic benchmarking crime!).

> **Corrected by Claude (Opus), 2026-11-26:** a second agent from the same pipeline is not independent. Use upstream AWFY ports verbatim where they exist (JS, Lua, Python, C++, Java, Ruby, ...; there are none for C, Rust or Go), publish new ports for comment before the freeze, and hold the OxiScript port to the same "idiomatic within the shared subset" rule. Licences: the AWFY ports carry MIT headers and Mandelbrot/NBody are CLBG Revised BSD, but DeltaBlue refers to Sun-era click-through terms: do not vendor it until cleared (ledger A2, E13).

2. **Cross-language oracle parity:** Every language must reproduce the exact golden checksum on `micro`, `device`, and `server` inputs.
3. **Zero-leak verification:** Managed runtimes verify bounded steady-state memory across 100 iterations; OxiScript explicitly asserts `oxi_live_objects() == 0` after `oxi_deinit()` / `oxi_collect_cycles()`.

---

## 5. Harness Design, Reproducibility Package, and `8.Perf`/`8.Lab` Integration

The harness lives in the Stage `8.Bench` crate (`oxiscript/oxiscript-perf`, binary `oxiperf`) with benchmark sources under `bench/oxiperf/`. It follows three engineering principles: **one-command execution**, **automated environment capture**, and **strict separation of execution from statistical analysis** (no manual spreadsheet editing).

### 5.1 One-Command CLI Interface

```text
# Run full cross-language suite for all locally available runtimes & targets
cargo run -p oxiscript-perf --bin oxiperf -- run \
  --suite v0.1 \
  --track base \
  --size server \
  --tier release \
  --targets auto \
  --out results/oxiperf-v0.1-2026-12-01-x86

# Run A/B Speedup Lab or 8.Perf comparison (paired interleaved invocations)
cargo run -p oxiscript-perf --bin oxiperf -- compare \
  --baseline main \
  --candidate "--unstable-opt=swar-utf8" \
  --targets wasmi,w2c2,wasmtime \
  --tier release

# Regenerate statistics, warmup diagnostics, and Markdown/CSV tables from raw data only
cargo run -p oxiscript-perf --bin oxiperf -- analyze \
  --input results/oxiperf-v0.1-2026-12-01-x86 \
  --out reports/oxiperf-v0.1-2026-12-01-x86.md
```

### 5.2 Repository and Package Layout

```text
bench/oxiperf/
  VERSION                     # "0.1.0" (monotonic; bumped on any spec/source/input change)
  freeze.toml                 # Pinned methodology version, suite list, input hashes, competitor flags
  suites/
    p01_mandelbrot/
      spec.toml               # Algorithm rules, size profiles (micro/device/server), golden digests
      base/                   # Identical-algorithm implementations
        main.oxs
        main.ts               # Compiled to JS; no runtime TS overhead
        main.as.ts            # AssemblyScript
        main.rs
        main.c
        main.go
        main.py
        main.lua
      peak/                   # Optional idiomatic/optimised implementations (strictly separated)
        ...
    ... (p02 .. p12)
  drivers/
    linux_cli/                # Spawns processes with env-pad randomization, taskset, /proc RSS/PSS
    browser_harness/          # Local HTTP server injecting COOP/COEP + headless Playwright runner
    rpi_ssh/                  # Cross-compiles armv6 artifacts, rsyncs to RPi 1, collects vcgencmd
    esp32_serial/             # Flashes ESP32-S3 via ESP-IDF, parses framed JSON over USB serial
```

### 5.3 Recorded Machine & Run Manifest (`manifest.json`)

Every run writes an immutable `manifest.json` before measurements begin (analogous to SPEC's `sysinfo` and [OxiBench §5](oxibench-methodology.md#5-running-r3-the-harness)):

```json
{
  "schema_version": 1,
  "methodology_version": "oxiperf-v0.1",
  "suite_version": "0.1.0",
  "freeze_commit": "<git-sha>",
  "oxiscript_version": "0.0.2",
  "run_id": "2026-12-01T10:00:00Z-x86-release",
  "tier": "release",
  "track": "base",
  "size_profile": "server",
  "host": {
    "arch": "x86_64",
    "cpu_model": "Intel(R) Core(TM) i5-3450 CPU @ 3.10GHz",
    "cores_physical": 4,
    "pinned_core": 2,
    "governor": "performance",
    "turbo_disabled": true,
    "uncontrolled_dvfs": false,
    "aslr_mode": 2,
    "ram_total_mib": 16384,
    "os": "Linux 6.x ...",
    "temp_c_start": 42.0,
    "temp_c_end": 45.5,
    "loadavg_start": 0.04
  },
  "toolchains": {
    "oxiscript": { "version": "0.0.2", "git_sha": "...", "flags": "--target server" },
    "wasmi": { "crate_version": "2.0.3", "mode": "Eager", "fuel": true },
    "w2c2": { "git_sha": "...", "cc": "gcc 13.3.0", "cflags": "-O2 -DNDEBUG" },
    "wasmtime": { "crate_version": "...", "flags": "cranelift-opt-speed" },
    "node": { "version": "v22.x.x", "v8": "12.x", "flags": "" },
    "rustc": { "version": "1.98.0", "flags": "-C opt-level=3 -C lto=thin" },
    "c": { "cc": "gcc 13.3.0", "flags": "-O2 -DNDEBUG" },
    "assemblyscript": { "version": "0.27.x", "flags": "--runtime incremental -O3" },
    "cpython": { "version": "3.12.x", "flags": "" },
    "lua": { "version": "5.4.x", "flags": "" }
  }
}
```

### 5.4 Raw Measurement and Summary Schemas

- **`raw.jsonl`** (append-only, one JSON object per invocation): records `benchmark`, `language`, `engine`, `track`, `invocation_idx`, `env_pad_bytes`, `valid` (bool + `invalid_reason` if faulted), `digest_ok`, `compile_ms`, `cold_ms`, `warmup_iters`, `inner_reps`, `iter_ns` (full array of **every** iteration including warmup, enabling Barrett et al. changepoint classification), `wasmi_fuel` (if `wasmi`), `peak_rss_kib`, `steady_pss_kib`, `peak_wasm_pages`, `live_objects_after_collect`, `artifact_bytes_raw`, `artifact_bytes_gzip`, and MCU SRAM/PSRAM/Flash counters when applicable.
- **`summary.json`** (computed purely by `oxiperf analyze` from `raw.jsonl`): per `(benchmark, language, engine, track)` median, IQR, p95, min, max, arithmetic mean, warmup status (`steady` | `slowdown` | `oscillating` | `no_steady_state`), memory/size/compile medians, and pairwise **95% hierarchical bootstrap CIs** (10,000 resamples) for every ratio vs. configured baselines (`c-base`, `rust-base`, `node-v8`, `wasmi-stub`), plus suite geometric mean with bootstrap CI.
- **`report.md`**: auto-generated Markdown report containing the machine summary, toolchain table, per-benchmark wall-time + CI table, startup/compile table, memory/code-size table, warmup anomaly flags, and explicit negative-result callouts.

### 5.5 Integration with `8.Perf`, `8.Mem`, and `8.Lab`

- **Stage `8.Perf` (RC elision & allocator fast path):** Runs `oxiperf --suite internal-rc` plus `P04`–`P10` across `wasmi` (reporting **both** fuel and wall time, per [Finding A1](oxiscript-memory-performance-verification.md)), `w2c2`+GCC, and `RuntimeMode::Stub` vs. `Linked`, verifying zero leaks and identical differential outputs between unoptimised and optimised OxiIR.
- **Stage `8.Mem` (Raspberry Pi 1 B+ positioning):** Uses `oxiperf` over `rpi_ssh` to compare OxiScript `wasmi` vs. `w2c2`+GCC and competitor baselines on `size = device`, capturing RSS/PSS/binary size/startup/frame time for the [D-20](../roadmap/oxiscript.md#d-20) gate table.

> **Corrected by Claude (Opus), 2026-11-26:** the w2c2 rows need Stage 8.2b first; cross-builds for the Pi need the Raspberry Pi OS sysroot (a stock Debian/Ubuntu `armhf` sysroot is ARMv7) or static musl builds with `-march=armv6zk -mfpu=vfp -mfloat-abi=hard` (ledger A10).

- **Stage `8.Lab` (Weekly Speedup Lab, [D-29](../roadmap/oxiscript.md#d-29)):** Every candidate `--unstable-opt=<name>` runs `oxiperf compare` in interleaved A/B mode across `wasmi`, `w2c2`, and `wasmtime`. Automated promotion gate checks: (1) 100% identical outputs/traps/float bits across the corpus and fuzzers, (2) at least one target achieves a statistically significant `>= 5%` wall-time win (95% CI excludes `1.0`), and (3) no target regresses by `>= 5%` (95% CI wholly above `1.05`). Fuel-only gains without wall-time improvement are flagged as metering artifacts, not speedups.

> **Corrected by Claude (Opus), 2026-11-26:** the regression test is asymmetric: a noisy run never has its CI "wholly above 1.05", so an under-measured candidate is promoted by default. Use non-inferiority on every target: block unless the **upper** bound of the 95% CI of `candidate / baseline` is below 1.02 (or another stated margin), with release-tier run counts on the gating targets (ledger A1).


---

## 6. Website and Public Claims Policy

Decision [D-20](../roadmap/oxiscript.md#d-20) establishes an explicit rule for OxiScript's websites (`oxiscript.slavekm.pl`, README, release notes, dev blog): **no performance or memory claim appears without measurements.** Borrowing the publication discipline of SPEC Full Disclosure and [OxiBench §8](oxibench-methodology.md#8-reporting), every public claim derived from OxiPerf must obey six rules:

1. **C1 — Frozen, Reproducible Reports Only:** Public prose, charts, and landing-page copy may quote numbers **only** from an owner-approved, frozen `release`-tier OxiPerf report (`oxiperf-vX.Y`) backed by a public freeze commit, `manifest.json`, and raw `raw.jsonl` archive. Never publish `ci`-tier numbers, uncontrolled-DVFS cloud numbers, scratch experiments, or `[estimate]`/`[P]` roadmap projections as measured results.
2. **C2 — Ranges and Intervals, Never Single Cherry-Picked Numbers:** Public summaries must state the **observed range across the full suite** (min–max) and/or the **geometric mean with its 95% bootstrap CI**—never one cherry-picked winning benchmark.
   - *Forbidden:* *"OxiScript is 15× faster than Python"* or *"OxiScript runs at near-C speed."*
   - *Required:* *"Across the 12 OxiPerf v0.1 identical-algorithm (`base`) benchmarks on Raspberry Pi 1 Model B+, OxiScript 0.1.0 (`w2c2` + GCC 13.3 `-O2`) ran at **1.4×–3.1×** the time of hand-written C (`gcc 13.3 -O2`, geomean 2.0×, 95% CI [1.8×, 2.2×]) while using **2.1–4.8 MB RSS** (report `oxiperf-v0.1-2026-12-rpi1`)."* *(Illustrative format only; numbers are placeholders.)*
3. **C3 — Mandatory Six-Tuple Disclosure:** Every public chart, table, or headline sentence must display (directly or via an immediately adjacent footnote/badge):
   1. OxiScript version **and exact execution engine** (`wasmi`, `w2c2`+C, Wasmtime, or browser Wasm);
   2. Competitor **runtime/compiler name and exact version** (e.g., `Node.js 22.11 / V8 12.4`, `CPython 3.12.7`, `Rust 1.98.0`, `MicroPython 1.24`);
   3. Track (`base` identical-algorithm vs. `peak` idiomatic/optimised);
   4. Hardware class and OS (e.g., `ESP32-S3 N16R8`, `Raspberry Pi 1 B+`, `i5-3450 Linux`);
   5. Methodology & suite version (`OxiPerf v0.1`) and run date;
   6. Direct link to the frozen report and reproduction instructions.
4. **C4 — Mandatory Disclosure of Ceilings, Trade-Offs, and Losses:** Any page showing where OxiScript wins (e.g., RSS vs. V8/Electron, or `w2c2` AOT speed vs. MicroPython/CPython) must also show the **full suite** including losses and ceilings (e.g., `wasmi` interpreter slowdown vs. V8 JIT, and `w2c2` gap vs. unchecked C/Rust). Semantic differences (trapping integer overflow, bounds checks, RC + cycle collection) are noted plainly rather than used as excuses.
5. **C5 — No Cross-Axis or Energy Extrapolation:** Never pair a speed ratio measured on x86 Wasmtime with a memory figure measured on ESP32-S3 `w2c2` in the same claim. Never extrapolate energy/battery claims from wall time or x86 RAPL (van Kempen et al. 2024); energy claims require direct rail measurements on the target device.
6. **C6 — Version Pinning and Staleness Hygiene:** Numbers from different OxiPerf methodology versions are never mixed in one chart. Published website claims carry an explicit date/version badge; any headline comparison whose competitor runtime is more than two major/LTS releases behind current upstream is visually marked `archived` until re-measured.

---

## 7. Relationship Between OxiPerf and OxiBench

OxiScript maintains **two distinct, strictly non-overlapping benchmark programs** with clear names so performance and AI-usability claims are never conflated:

| Dimension | **OxiPerf** (this document) | **[OxiBench](oxibench-methodology.md)** |
|---|---|---|
| **Roadmap stage** | `8.Bench` (`oxiscript-perf`), feeding `8.Perf`, `8.Mem`, `8.Lab`, `8.3a–e` | `8.OB` (blocked on owner approval & API keys, [Q-OB-1](../roadmap/oxiscript.md#q-ob)) |
| **Question answered** | *How fast, small, and memory-light are OxiScript programs across `wasmi`, `w2c2`, Wasmtime, and browsers compared to other languages/runtimes?* | *How accurately do AI models write correct OxiScript (with/without docs and compiler loop) compared to TypeScript, and which language features cause mistakes?* |
| **Subjects under test** | Compilers, Wasm engines, allocators, hardware targets (`micro`/`device`/`web`/`server`). | Frontier and open-weight LLM APIs (isolated from the repo). |
| **Primary metrics** | Wall time (cold & steady state), `wasmi` fuel (budget only), RSS/PSS/heap/SRAM/Flash, code size, compile latency. | `pass@1` in condition C2, TS twin gap, `OXS-*` diagnostic taxonomy, tokens/cost per pass. |
| **Visibility of benchmark set** | **100% public** (open source in `bench/oxiperf/` for independent third-party reproduction). | **Split:** public dev set (`bench/dev/`) + **private held-out test set** outside the repo with canary GUID to prevent LLM training contamination. |
| **Report series** | `reports/oxiperf-vX.Y-...` | `reports/oxibench-vX.Y-...` |

### What They Share vs. Keep Separate
- **Shared methodological discipline and tooling primitives:**
  1. **Preregistration freeze commit** before any publishable run; version bumps whenever methodology or benchmarks change; never mix versions in one table.
  2. **Role separation for fairness:** independent implementation/task auditing (Role R2) so nobody grades or self-optimises their own benchmark unfairly; deterministic harness (R3); script-only statistics (R5).
  3. **Shared Rust statistics & reporting utilities** (optional internal library crate `oxiscript-perf-stats`): 10,000-resample bootstrap engine (flat task-level for OxiBench, hierarchical invocation×iteration for OxiPerf), JSON-manifest validator, Markdown table generator, and website frozen-report quote checker.
  4. **Identical publication ethics:** publish negative results, disclose conflicts/versions, quote only frozen reports on the websites.
- **Strictly separate:** Benchmark directories (`bench/oxiperf/` vs. `bench/oxibench/` + private external store), version counters (`oxiperf-v0.1` vs. `oxibench-v0.1`), execution schedules (OxiPerf runs locally/on boards per milestone or weekly in `8.Lab`; OxiBench runs once per toolchain release against paid LLM APIs), and published reports/pages (never combined into a composite score).

---

## 8. Open Questions for the Owner

Following the roadmap convention ([../roadmap/oxiscript.md §7](../roadmap/oxiscript.md#7-open-questions-for-the-owner)), these decisions lock OxiPerf v0.1 before the first freeze commit:

1. **`Q-OP-1` (Methodology approval & freeze gate):** Approve this methodology (v0.1) as the governing spec for Stage `8.Bench` (`oxiscript-perf`), requiring a freeze commit before any cross-language numbers are published on the sites.
2. **`Q-OP-2` (Mandatory v0.1 competitor roster):** Confirm the minimum set of comparison languages required for the first frozen OxiPerf v0.1 release vs. deferred to v0.2:

> **Corrected by Claude (Opus), 2026-11-26:** recommended v0.1 scope: dev box only, AWFY subset P01-P06 with upstream ports (JS, Lua, Python, C++ as native reference) plus OxiScript on wasmi and Rust; the Pi 1 rows (C, wasmi, LuaJIT, Lua, CPython; w2c2 after 8.2b) as the 8.Mem part; ESP32-S3 after 8.S3 and 8.2b with C, w2c2 and MicroPython first (verification section H).

   - *Proposed v0.1 Linux (`server`/`device`):* OxiScript (`wasmi`, `w2c2`+GCC, Wasmtime), TypeScript/JS (Node.js/V8), AssemblyScript (`--runtime incremental`), Rust (native + `wasm32`), C (`gcc -O2`), CPython 3.x, Lua 5.4.
   - *Proposed v0.1 Browser (`web`):* OxiScript native Wasm (plus separate playground `wasmi`-in-Wasm row), Browser JS, Rust `wasm32`, AssemblyScript.
   - *Proposed v0.1 MCU (`micro`, after `8.S3` hardware arrives):* OxiScript (`w2c2` + ESP-IDF GCC), C (ESP-IDF GCC), MicroPython, Lua 5.4, TinyGo (`-gc=conservative`).
   - *Deferred to v0.2 unless requested now:* Go (full Linux), PyPy, LuaJIT, Deno/Bun.
3. **`Q-OP-3` (Unifying `8.Bench` SIMD/GPU workloads with OxiPerf):** Confirm merging `8.Bench`'s existing `particles` and `MLP` workloads into `P11` (`physics_2d_step`) and `P12` (`tiny_mlp_layer`) in v0.1, while keeping `GEMM` and `image_filter_3x3` as the v0.2 Data-Parallel Extension once Stage `8.3a` begins.
4. **`Q-OP-4` (DVFS & quiet-machine setup on dev box and Raspberry Pi 1 B+):** Confirm whether benchmark runs on the `i5-3450` dev box and Raspberry Pi 1 Model B+ may temporarily set `scaling_governor=performance`, disable Turbo Boost, and stop non-essential services (via a documented wrapper script) so wall-time runs qualify as `release`-tier publishable data.
5. **`Q-OP-5` (Browser engine coverage in v0.1):** Is automated headless Chromium (via Playwright/CDP against a local COOP/COEP server) sufficient for the first frozen `web` report, with Firefox (SpiderMonkey) and WebKit (JSC) added in v0.2?
6. **`Q-OP-6` (Analysis implementation language):** Confirm implementing `oxiperf analyze` (hierarchical bootstrap, Barrett changepoint/drift check, Markdown generator) in pure Rust inside `oxiscript-perf` (zero Python/R runtime dependencies, deterministic compiled binary, shared with OxiBench stats).
7. **`Q-OP-7` (Speedup Lab `8.Lab` public reporting tier):** For the weekly `oxiscript-vX.Y.C-speedup.N` alphas ([D-29](../roadmap/oxiscript.md#d-29)), should the optional public Speedup Lab table require full `release`-tier runs (10×30 invocations on quiet hardware), or may weekly alphas publish `ci`-tier numbers with an explicit `"alpha/unfrozen"` banner while reserving `release`-tier for promoted defaults?
