# Verification ledger: `cross-language-benchmark-methodology.md` (OxiPerf, Gemini draft)

Verifier: Claude (Opus), 2026-11-26. Method: the same as `oxiscript-memory-performance-verification.md` and `oxiscript-simd-gpu-design-verification.md`. Every cited paper was checked for its bibliographic data (Crossref API for each DOI) and for the specific claim the report makes (abstract, author PDF, or a primary summary where the PDF was too large to fetch). Suite details were checked against the Are We Fast Yet sources (`smarr/are-we-fast-yet`, JavaScript port). Tool facts were checked on the projects' own pages or repositories. Repository facts were checked in this worktree and on the dev box. Beyond the claims, section H judges the methodology itself, as the task asked: is it complete, is it practical for one owner with an old 4-core PC, a Raspberry Pi 1 B+ and an ESP32-S3, and what could still be gamed.

Status values:
* **CONFIRMED**: the source says it (or, for a rule, the rule is sound and consistent with its source).
* **PARTLY**: the gist is right, but a number, attribution or scope is off, or the rule has a gap.
* **WRONG**: the source contradicts it.
* **UNVERIFIABLE**: no source given and none found, or the source is silent.
* **NRC** ("not re-checked"): matches widely known facts, no decision rests on it. Not counted.

Note: the task named `docs/bench/README.md` as context; that file does not exist in this worktree (no `docs/bench/` directory). The relevant repository context is the 8.Bench, 8.Perf, 8.Mem, 8.Lab and 8.OB stage blocks in `docs/roadmap/oxiscript.md`.

## Counts

| | Count |
|---|---|
| Claims checked (ledger rows B-G, excluding NRC) | 95 |
| CONFIRMED | 64 |
| PARTLY | 25 |
| WRONG | 3 |
| UNVERIFIABLE | 3 |
| NRC (not counted above) | 7 |

Many CONFIRMED rows in C are rules judged sound, not facts; the factual error rate is in B and D.

Section A lists the findings that change the methodology; each one is also a row in B-G and is not counted twice. Section H is the overall judgment.

## A. Corrections that change the methodology (read first)

| # | Report section | Gemini claim | Finding | Evidence |
|---|---|---|---|---|
| A1 | §5.5 8.Lab gate | Promote if one target shows a significant ≥ 5% win and "no target regresses by ≥ 5% (95% CI wholly above 1.05)" | **PARTLY, and the most gameable rule in the report.** The win test needs a tight CI, the regression test needs a CI *wholly above* 1.05, so a noisy run can never fail the regression test and an under-measured candidate is promoted by default. The gate must be a non-inferiority test on every target: block unless the **upper** bound of the 95% CI of `candidate / baseline` is below 1.02 (or another stated margin), and require the full release-tier run count for the targets that gate. | report §5.5; standard non-inferiority design |
| A2 | §4.3, §3.4, R01 | Ports written per language, audited by "a second reviewer (human or separate agent instance)" | **PARTLY: not independent.** The same vendor's agents would write the OxiScript version and every competitor port, and a second agent from the same pipeline is not an independent reviewer; this is the classic "handicapping the competitor" crime of van der Kouwe et al. Where upstream ports exist, use them **verbatim and pinned**: AWFY has JavaScript, Lua, Python, C++, Java, Ruby, Crystal, Smalltalk, SOM and SOMns ports (no C, Rust or Go); all six of P01-P06 exist in the C++, Lua, Python and JavaScript directories. **Licences (O-14):** the ports carry per-file MIT headers (© Stefan Marr), Mandelbrot and NBody derive from CLBG (Revised BSD, © Isaac Gouy), but **DeltaBlue** (and Richards) refer to Mario Wolczko's Sun-era click-through terms, kept only on an archived page; Wolczko himself notes that derived versions appeared under incompatible licences. Do not vendor DeltaBlue into this MIT/Apache repository until those terms are read and found permissive; fetch it from a pinned AWFY commit at run time instead, or ask (E13). New ports (Rust, C, Go, AssemblyScript, MicroPython) should be published for comment before the freeze and linked from the report, and the OxiScript port must follow the same "idiomatic within the shared subset" rule. | `smarr/are-we-fast-yet` `benchmarks/` listings (`C++/src`: `sieve.h`, `storage.h`, `json.h`, `mandelbrot.h`, `nbody.h`, `deltablue.cpp`), `LICENSE.md`, file headers of `sieve.js`/`json.js`/`deltablue.js`; http://www.wolczko.com/java_benchmarking.html; AWFY README rules 1-2 |
| A3 | §4.1, Exec. summary | 12 programs, 6 of them OxiScript/UI workloads chosen by us; suite-wide geomean as a summary | **PARTLY: selection bias.** P07-P12 are chosen for OxiScript's strengths and roadmap (layout, templates, sort with our fuel budget, MLP). Keep them, but **headline cross-language claims only on the AWFY subset** (P01-P06, which nobody here chose), and publish the home-workload geomean as a separate, labelled number. Size profiles, `inner_reps` and the competitor roster must be frozen before the first measured run (R21 says this for sources and flags; it must also cover sizes). | report §4.1; R21 |
| A4 | R15, R16 | Fixed run counts (10 invocations × 30 iterations; CI tier 3 × 15), citing Kalibera & Jones | **PARTLY: contradicts its own source.** Kalibera & Jones's contribution is *deriving* the repetition count at each level from a measured initial experiment ("the number of repetitions at each level of an experiment necessary and sufficient"), not a fixed count. Also, a hierarchical bootstrap whose top level has only 10 units has poor coverage. Do one dimensioning run per ⟨engine, benchmark, machine⟩, compute the counts with their formula, freeze them. Barrett et al.'s own recommendation for process executions is 10, so 10 is a floor, not a design. | Kalibera & Jones ISMM'13 author manuscript (kar.kent.ac.uk/33611), abstract and §9; Acolyer summary of Barrett et al. ("Good results should be obtained with 10 process executions") |
| A5 | R09 | Warmup window `max(50 iterations, 2 s)` + 30 measured iterations, then a Barrett-style changepoint classification | **PARTLY.** Barrett et al. classified series of **2 000 in-process iterations** over 30 process executions (3 660 executions, 7.32 M iterations in total); about 80 points cannot tell "steady" from "slowdown" or "late changepoint". For JIT rows (V8, LuaJIT, Wasmtime JIT) run a one-off long series (≥ 1 000 iterations, a few executions) to pick the warmup length per benchmark, then freeze it; report the classification from the long series, not the short one. | Barrett et al. (arXiv 1602.00602, Acolyer summary: 3 660 process executions, 7 320 000 iterations, plots to 2 000 iterations) |
| A6 | R06, R04, §4.1 P02/P11/P12 | Exact IEEE bits across languages on deterministic targets | **PARTLY: a missing rule.** GCC defaults to `-ffp-contract=fast` outside strict ISO modes, so C and w2c2-generated C may fuse `a*b+c` on any target with FMA instructions, which changes bits and breaks P-9 parity silently. All C, C++ and w2c2 builds in `base` need `-ffp-contract=off` (and never `-ffast-math`); Rust and Wasm do not contract. JavaScript has only `f64`: the `f32` workloads (P07, P11, P12) need `Math.fround` on every operation in the JS port, or they compare different arithmetic. | measured on the dev box: `double f(a,b,c){return a*b+c;}` at `-O2 -march=haswell` emits `vfmadd` with GCC 13.3 by default, none with `-ffp-contract=off` or `-std=c11`; clang 18 emits `vfmadd` by default **and** with `-std=c11`, none with `-ffp-contract=off`. (The i5-3450 itself has no FMA and ARMv6 VFPv2 has no fused multiply-add, so the risk is on FMA targets: `-march=native` on newer x86, Pi 4/5, possibly the ESP32-S3 FPU [unverified].) P-9 |
| A7 | §3.4 Lua row, Q-OP-2 | A trap is "assuming LuaJIT exists on ARMv6/ESP32-S3"; LuaJIT only on x86_64 | **WRONG for ARMv6.** LuaJIT's status page lists ARM 32-bit, "ARMv5+, ARM9E+", from v2.0. LuaJIT runs on the Pi 1 B+ and is the strongest scripting peer there; add it to the `device` roster. It does not target Xtensa (correct). | https://luajit.org/status.html (CPU Architectures table) |
| A8 | §1.2 Barrett et al. | VMs studied include "CPython" | **WRONG.** The VMs were GCC 4.9.3 (C baseline), Graal 0.18, HHVM 3.15.3, JRuby+Truffle 9.1.2.0, HotSpot 8u112b15, LuaJIT 2.0.4, PyPy 5.6.0 and V8 5.4.500.43. CPython was not studied. | https://blog.acolyer.org/2017/11/07/virtual-machine-warmup-blows-hot-and-cold/ |
| A9 | §1.2 Georges et al. | DOI `10.1145/1297846.1297881` | **WRONG.** That DOI is "The JastAdd extensible Java compiler" (OOPSLA'07 companion, pp. 773-774). Georges, Buytaert, Eeckhout is `10.1145/1297027.1297033`, OOPSLA'07 pp. 57-76. | Crossref API for both DOIs |
| A10 | §3.2, R13, Q-OP-2, §5.5 | Rows for w2c2, Wasmtime, `micro`, Node on the Pi, run under `taskset` with load < 0.2 | **PARTLY: much of the matrix cannot run yet or not as written.** (1) w2c2 AOT and `--target micro` are Stage [8.2b](../roadmap/oxiscript.md#s-8-2b) (todo) and Wasmtime is not integrated, so v0.1 today can measure only wasmi plus native C/Rust/JS/Lua/Python rows. (2) Node.js dropped ARMv6; only unofficial armv6l builds exist, with "minimal or no testing". (3) The Pi 1 is single-core: `taskset` and "quiet core" do nothing; what matters is stopping services and recording `vcgencmd get_throttled` (the owner's board is corroded, so under-voltage throttling is a real risk). (4) The dev box hosts concurrent agent sessions, so "load average < 0.2" needs a scheduled quiet window. (5) A stock Debian/Ubuntu `arm-linux-gnueabihf` toolchain targets ARMv7 and its libc will not run on ARMv6; cross-builds need the Raspberry Pi OS sysroot (or static musl builds with `-march=armv6zk -mfpu=vfp -mfloat-abi=hard`). | roadmap status table (8.2b todo); https://github.com/nodejs/unofficial-builds; owner board notes |

## B. Ledger: literature (§1)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| B1 | van der Kouwe, Heiser, Andriesse, Bos, Giuffrida, "SoK: Benchmarking Flaws in Systems Security", EuroS&P 2019 pp. 310-325, DOI 10.1109/EuroSP.2019.00031; preprint arXiv:1801.02381 | CONFIRMED | Crossref; arXiv abstract page |
| B2 | 22 flaws "in five groups" (selective benchmarking, improper handling, wrong benchmarks, improper comparison, missing information) | PARTLY | 22 confirmed (abstract). The five groups are those of Heiser's living page, which lists **18** crimes; Heiser notes that the VU Amsterdam study "added a security-specific category" |
| B3 | 50 papers, "average of 5 flaws per paper", only 1 of 50 flaw-free | PARTLY | abstract: "**tier-1** papers commit an average of five benchmarking crimes"; "only a single paper in our sample that committed no benchmarking crimes" |
| B4 | Hoefler & Belli SC15, DOI 10.1145/2807591.2807644, twelve reporting rules | CONFIRMED | Crossref (title "Scientific benchmarking of parallel computing systems", SC15); rule content not reopened |
| B5 | Fleming & Wallace CACM 29(3):218-221, DOI 10.1145/5666.5673; geometric mean for normalised ratios | CONFIRMED | Crossref (vol 29, issue 3, pp. 218-221) |
| B6 | Georges et al. DOI 10.1145/1297846.1297881 | WRONG | A9 |
| B7 | Georges et al.: separate startup from steady state, multiple VM invocations, confidence intervals | NRC | consistent with the title and widely cited; not reopened |
| B8 | Kalibera & Jones ISMM'13 pp. 63-74, DOI 10.1145/2464157.2464160 | CONFIRMED | Crossref |
| B9 | Nested variance; pooling iterations as i.i.d. gives falsely narrow intervals | CONFIRMED | ISMM'13 manuscript (multi-level experiments, "repetition is most needed" where variation is); companion report criticises analysing multi-level data "as if it came from a one-level experiment" |
| B10 | ISMM'13 "introduced hierarchical random resampling (hierarchical bootstrap)" for ratio CIs | PARTLY | ISMM'13's summarising method is parametric (analysis of variance; effect-size CI by its eq. 5) and says "we also demonstrate an alternative non-parametric method" in the technical report: Kalibera & Jones, "Quantifying performance changes with effect size confidence intervals", TR 4-12, Univ. of Kent, 2012 (arXiv 2007.10899), which describes the hierarchical bootstrap for the ratio of means. Cite the TR for the bootstrap. |
| B11 | K&J allocate the run budget to the level with most variance | CONFIRMED | ISMM'13 abstract ("number of repetitions at each level … necessary and sufficient") |
| B12 | Barrett, Bolz-Tereick, Killick, Mount, Tratt, OOPSLA'17, DOI 10.1145/3133876, arXiv:1602.00602 | CONFIRMED | Crossref (PACMPL 1, OOPSLA, pp. 1-27); arXiv |
| B13 | "at most 43.5%" of ⟨VM, benchmark⟩ pairs consistently reach peak steady state | CONFIRMED | arXiv abstract (verbatim) |
| B14 | VMs studied include CPython | WRONG | A8 |
| B15 | Changepoint analysis; many series never stabilise or slow down | CONFIRMED | arXiv abstract and Acolyer summary |
| B16 | Mytkowicz, Diwan, Hauswirth, Sweeney, ASPLOS'09, DOI 10.1145/1508244.1508275 | CONFIRMED | Crossref (pp. 265-276) |
| B17 | Environment size and link order bias can flip an O2-vs-O3 conclusion | CONFIRMED | abstract summaries: measurement bias "can lead to … an incorrect conclusion"; UNIX environment size and link order; Pentium 4, Core 2, m5, gcc and icc, SPEC CPU2006 |
| B18 | None of 133 papers (ASPLOS, PACT, PLDI, CGO) controlled for it | CONFIRMED | same |
| B19 | Remedies: causal analysis, setup randomisation | CONFIRMED | same |
| B20 | Curtsinger & Berger, STABILIZER, ASPLOS'13 pp. 219-228, DOI 10.1145/2451116.2451141 | CONFIRMED | Crossref; content summary not reopened |
| B21 | CLBG criticisms (1) unidiomatic C-transplants, (2) C libraries (PCRE, GMP), (3) concurrency, (4) uneven effort | PARTLY | opinion without a source (pages checked: how-programs-are-measured, description/pidigits). CLBG documents its own caveats: programs "crowd-sourced from a changing, self-selected group", measured on an "isolated and unloaded" host, lowest of 12 elapsed-time runs. CLBG flags library use openly: the pidigits description says "some use a third-party library" (GMP) and labels the topic "Contentious. Different libraries." Useful fact the report misses: CLBG measures on a quad-core **i5-3330**, the same Ivy Bridge generation as the owner's i5-3450, so CLBG numbers are a sanity check for the dev box. |
| B22 | Rosetta Code: author-skill confounding, toy loops | UNVERIFIABLE | opinion, no source; harmless |
| B23 | SPEC CPU2017 base: uniform flags per language, no FDO in base; peak allows per-benchmark tuning; output verification; medians | CONFIRMED | run rules 2.3.5 ("all flags must be the same within a suite and language"), 2.3.6 ("Feedback directed optimization must not be used in base"), 2.2.3; 1.2.1 (median of 3 runs, or slower of 2) |
| B24 | SPEC CPU2017 retired November 2026 in favour of SPEC CPU 2026 | CONFIRMED | spec.org CPU2017 page: retirement end of day 16 November 2026 (FTP mirror says 17 November); CPU 2026 released |
| B25 | AWFY: Marr, Daloze, Mössenböck, DLS'16, DOI 10.1145/2989225.2989232; 14 benchmarks, 5 macro (CD, DeltaBlue, Havlak, Json, Richards), 9 micro (Bounce, List, Mandelbrot, NBody, Permute, Queens, Sieve, Storage, Towers) | CONFIRMED | Crossref (pp. 120-131); AWFY README |
| B26 | AWFY: identical algorithms, idiomatic within a shared core | CONFIRMED | AWFY README rules 1 and 2 |
| B27 | Pereira et al. SLE'17 DOI 10.1145/3136014.3136031; SCP 205:102609 (2021) | CONFIRMED | Crossref (SLE pp. 256-267; SCP vol. 205) |
| B28 | Pereira: 27 languages, CLBG (and Rosetta in 2021), Intel RAPL | NRC | consistent with the papers' titles and abstracts as widely cited |
| B29 | van Kempen, Kwon, Nguyen, Berger (2024), arXiv 2410.05460 | PARTLY | authors and arXiv id confirmed; the paper is "to appear at ASE'25" (v4, revised 2025-10-03); cite it as ASE 2025 |
| B30 | van Kempen: once execution time, JIT warmup, GC, active cores, cache misses and FFI are controlled, language has no effect beyond execution time | PARTLY | conclusion confirmed ("the choice of programming language implementation has no significant impact on energy consumption beyond execution time"); the factors named in the abstract are language vs implementation, the application implementations, number of active cores, and memory activity. "JIT warmup, GC, cache misses, FFI" are Gemini's additions. |
| B31 | Browser `performance.now()` 100 µs non-isolated, 5 µs cross-origin isolated | CONFIRMED | MDN `Performance/now` |

## C. Ledger: rules R01-R21 (§2)

| ID | Rule | Status | Evidence / gap |
|---|---|---|---|
| C1 | R01/R02 mandatory `base` track, optional `peak` never mixed | CONFIRMED | mirrors SPEC base/peak (B23) and AWFY (B26) |
| C2 | R03 no FFI smuggling in `base`; stdlib containers tagged | CONFIRMED | sound |
| C3 | R04 semantic-parity disclosure; Rust `overflow-checks = true` variant; C `-ftrapv` | CONFIRMED | Rust release profile disables overflow checks by default [NRC]; add C `-fwrapv` vs `-ftrapv` note (`-ftrapv` is known to be incomplete in GCC; `-fsanitize=signed-integer-overflow` with trap is the reliable form) [NRC] |
| C4 | R05 identical inputs via documented PRNG, opaque to constant folding | CONFIRMED | sound |
| C5 | R06 digest checked on every invocation, outside the timed region | PARTLY | sound, but exact-bit float digests need A6 (`-ffp-contract=off`, `Math.fround`) |
| C6 | R07 pinned, uniform flags per ⟨language, target⟩ | CONFIRMED | SPEC 2.3.5 |
| C7 | R07/§3.4: C `-O2` in `base` labelled "native unchecked ceiling" | PARTLY | `-O2` is a reference point, not a ceiling; call the `base` row "C reference (-O2)" and the `-O3` peak row the ceiling, or use `-O3` uniformly in `base` (SPEC lets the tester choose one uniform set) |
| C8 | R08 compile, cold and steady-state reported separately | CONFIRMED | Georges et al. (B7) |
| C9 | R09 warmup window and changepoint classification | PARTLY | A5 |
| C10 | R10 RSS/PSS/USS, heap, GC/RC timing; uncollected vs collected variants | CONFIRMED | sound; `/proc/<pid>/smaps_rollup` exists since Linux 4.14 [NRC] |
| C11 | R11 fuel is a metering budget, never a speed proxy; wasmi charges per region on entry | CONFIRMED | consistent with `oxiscript-memory-performance-verification.md` A1 and roadmap 8.Perf (`docs/roadmap/oxiscript.md:339`). Gap: fuel metering itself costs wall time; report wasmi wall time with fuel **off** too, or the interpreter row includes the metering tax |
| C12 | R12 cold vs warm caches; ESP32-S3 XIP vs IRAM, SRAM vs PSRAM disclosure | CONFIRMED | sound; page-cache drop needs root |
| C13 | R13 governor `performance`, `no_turbo`, `taskset`, load < 0.2, interleaving | PARTLY | dev box: `intel_cpufreq` (intel_pstate passive) with `/sys/devices/system/cpu/intel_pstate/no_turbo` present (checked); turbo to 3.5 GHz exists. Pi 1: A10 (single core, throttling flags). Interleaving across invocations is good practice and correct. |
| C14 | R14 environment padding across invocations; record ASLR | CONFIRMED | Mytkowicz et al. setup randomisation (B19). Link-order and code-layout randomisation (Stabilizer) are not covered; say so as a residual bias |
| C15 | R15 hard-coded 10 × 30 (release), 3 × 15 (CI); ≥ 10 ms per timed iteration | PARTLY | A4 |
| C16 | R16 median, IQR, p95; hierarchical bootstrap 10 000 resamples; geomean only with breakdown | CONFIRMED | sound (B5, B10); see A4 on coverage with 10 top-level units |
| C17 | R17 no silent outlier removal; invalidate whole invocations with a reason | CONFIRMED | Hoefler & Belli spirit; sound |
| C18 | R18 "win" = digests match, warmup ok, CI excludes 1.0, effect ≥ 5% (internal) / ≥ 10% (external) | PARTLY | sound per comparison, but there is no control for multiple comparisons: 12 benchmarks × several competitors × several targets gives dozens of tests, and about 1 in 20 null comparisons excludes 1.0 by chance. Use Holm-adjusted (or simultaneous) intervals for any claim drawn from the family, or claim only the pre-registered headline comparisons |
| C19 | R19 publish the whole suite incl. losses | CONFIRMED | van der Kouwe et al. "selective benchmarking" |
| C20 | R20 raw data and one-command reproduction package | CONFIRMED | sound |
| C21 | R21 preregistration freeze, version bump on change | CONFIRMED | mirrors `oxibench-methodology.md` §7 (exists, "Freeze (preregistration)"); extend to size profiles and `inner_reps` (A3) |

## D. Ledger: tiers, hardware, competitors (§3)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| D1 | wasmi 2.0.3, `CompilationMode::Eager` | CONFIRMED | `oxiscript/oxiscript-runtime/src/runtime.rs:163`; Cargo.lock |
| D2 | wasmi: register-based tail-call interpreter, `Vec<u8>` memory | NRC | not reopened |
| D3 | w2c2 on 32-bit targets: no 4 GiB guard region, explicit bounds checks | NRC | consistent with the SIMD/GPU research |
| D4 | Wasmtime on 64-bit: guard pages, no explicit bounds checks | NRC | Wasmtime default memory configuration |
| D5 | Browser harness with COOP/COEP; wasm-in-wasm playground in a separate row | CONFIRMED | MDN (B31); sound |
| D6 | Dev box Intel Core i5-3450, 4 cores / 4 threads, llvmpipe | CONFIRMED | `lscpu` on the dev box; roadmap 8.Bench gate (`docs/roadmap/oxiscript.md:401`). The manifest example's `ram_total_mib: 16384` is illustrative; the box has about 23 GiB |
| D7 | Pi 1 B+: BCM2835, ARM1176JZF-S ARMv6Z @ 700 MHz, VFPv2, no NEON, 512 MB; target `arm-unknown-linux-gnueabihf` | CONFIRMED | `config.txt` defaults (arm 700); owner notes; Rust target name. Cross-build trap: A10 (5) |
| D8 | ESP32-S3 N16R8: dual LX7 ≤ 240 MHz, 512 KB SRAM, 16 MB flash, 8 MB octal PSRAM | CONFIRMED | roadmap 8.S3 ("ESP32-S3 4.3\" 800x480 touch board (N16R8)"); datasheet figures [NRC] |
| D9 | `esp_timer_get_time()` 1 µs, `esp_cpu_get_cycle_count()` | NRC | ESP-IDF API |
| D10 | No wasmi on MCUs (D-27) | CONFIRMED | `docs/roadmap/oxiscript.md:117` |
| D11 | TS/JS: strip types, V8 default flags, check deopts, `Int32Array`/`Float64Array` in `base` | PARTLY | sound, but conflicts with A2: the upstream AWFY JavaScript ports use plain `Array` (e.g. `sieve.js`: `new Array(5000)`, `flags.fill(true)`). Either reuse AWFY ports verbatim (then the TypedArray rule applies only to P07-P12) or rewrite them (then they are no longer AWFY's ports). Decide once, in the spec. |
| D12 | Node/V8 on the Pi 1 B+ | PARTLY | A10 (2): unofficial armv6l builds only |
| D13 | AssemblyScript `--runtime incremental` (TLSF + incremental GC) vs `stub` (never frees) | CONFIRMED | assemblyscript.org/runtime.html ("TLSF + incremental GC (default)", stub "never frees"); the name ITCMS is from the source tree, not that page |
| D14 | Run AssemblyScript and Rust `.wasm` on the same engines as OxiScript | CONFIRMED | sound; this is the cleanest comparison in the report |
| D15 | Compile w2c2 output with the same C compiler, version and flags as the C baseline | CONFIRMED | sound; isolates the sandbox tax |
| D16 | TinyGo `-gc=conservative` is the default managed GC on Wasm and ESP32-S3 | PARTLY | TinyGo target files: `xtensa.json` (inherited by `esp32s3.json`) sets `"gc": "conservative"`, but `wasm.json` sets `"gc": "precise"`. TinyGo does have ESP32-S3 targets, including `esp32s3-n16r8-devkit.json` |
| D17 | MicroPython bytecode as `base` MCU peer; `native`/`viper` in `peak` | CONFIRMED | sound |
| D18 | LuaJIT absent on ARMv6 | WRONG | A7 |
| D19 | Lua 5.4 "everywhere, including ESP32-S3" | PARTLY | PUC Lua builds anywhere with a C compiler, but there is no official ESP32-S3 firmware: someone has to build and maintain a port; cost belongs in the plan |
| D20 | V8's multi-tens-of-MB baseline RSS is a legitimate axis under D-20 | CONFIRMED | D-20 (`docs/roadmap/oxiscript.md:110`); sound |

## E. Ledger: suite (§4)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| E1 | P01-P06 are AWFY benchmarks (Mandelbrot, NBody, Sieve, Storage, Json, DeltaBlue) | CONFIRMED | AWFY README |
| E2 | P01 `max_iter = 50`, `limit = 4.0`; sizes 128/250/500 "match AWFY verify values" | PARTLY | `mandelbrot.js`: `z < 50`, `> 4.0` confirmed; AWFY verifies only sizes 500 (→ 191), 750 (→ 50) and 1 (→ 128). Sizes 128 and 250 need new goldens; the AWFY checksum is a small integer, a weak digest, so add a full-output hash |
| E3 | P02 5-body, `dt = 0.01`, energy check | PARTLY | `nbody.js`: `advance(0.01)`, verifies `-0.1690859889909308` only for 250 000 steps (and 1); 5 000 and 50 000 need new goldens |
| E4 | P03 prime counts 669 / 5 133 / 41 538 | CONFIRMED | AWFY `sieve.js` (5 000 → 669); counts for 50 000 and 500 000 recomputed |
| E5 | P04 seed 74755, depth 7, branching 4, 5 461 nodes | CONFIRMED | `storage.js` (`buildTreeDepth(7)`, `5461 === result`), `som.js` (`seed = 74755`, `(seed * 1309 + 13849) & 65535`). Depths 5 and 6 give 341 and 1 365 nodes (new goldens) |
| E6 | P05 AWFY Json over 8/64/512 KB fixtures | PARTLY | AWFY Json parses one fixed embedded document; larger fixtures are a new benchmark, not AWFY's |
| E7 | P06 DeltaBlue sizes 50/200/1 000 | NRC | not checked against the AWFY driver |
| E8 | P07-P12 tie into 8.Bench (particles, MLP), 8.Perf/BS07 (30k sort), BL06 (templates) | CONFIRMED | roadmap 8.Bench scope (four workloads GEMM, image filter, particles, MLP), 8.Perf ("sorting 30k `i32` … BS07"), BL06 |
| E9 | GEMM and image filter deferred to v0.2 | PARTLY | 8.Bench owns those workloads and its gate blocks 8.3a, 8.3b, 8.3d (`docs/roadmap/oxiscript.md:398-401`); deferring them inside OxiPerf is fine only if 8.Bench keeps them in its SIMD/GPU part |
| E10 | Size profiles put every program at 10-500 ms per iteration | PARTLY | not achievable at one fixed size across a 50-100× spread (C vs CPython, wasmi vs native); `inner_reps` fixes the floor, not the ceiling. State the per-row expected range instead |
| E11 | Exact-bit float oracles on deterministic `base` | PARTLY | A6 |
| E13 | Licences of the AWFY programs (not addressed by the report) | UNVERIFIABLE | P01-P05 ports: MIT per-file headers (© Stefan Marr) or CLBG Revised BSD: permissive. P06 DeltaBlue: Wolczko/Sun click-through terms on an archived page, not read; treat as not cleared (A2) |
| E14 | 6 of 12 programs chosen by us, suite geomean as summary | PARTLY | A3 |
| E12 | Zero-leak check `oxi_live_objects() == 0` after `oxi_deinit()` / `oxi_collect_cycles()` | CONFIRMED | `docs/oxiscript/abi.md` (`oxi_deinit()` "so leak checks can assert `oxi_live_objects() == 0`"); `oxiscript-runtime/src/runtime.rs:816-834` |

## F. Ledger: harness, gates, claims (§5, §6)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| F1 | Crate `oxiscript-bench`, binary `oxiperf` | CONFIRMED | roadmap 8.Bench ("todo (no crate)"); naming is new |
| F2 | `--unstable-opt=<name>` A/B mode for 8.Lab | CONFIRMED (planned) | roadmap 8.Lab scope (`docs/roadmap/oxiscript.md:347`); not implemented |
| F3 | 8.Lab promotion gate | PARTLY | A1 |
| F4 | Fuel-only gains flagged as metering artifacts | CONFIRMED | memory-performance verification A1 |
| F5 | `RuntimeMode::Stub` vs `Linked` comparison in 8.Perf | CONFIRMED | `oxiscript/oxiscript-wasm/src/lib.rs:145` (`enum RuntimeMode`) |
| F6 | Host collects cycles between events (`abi.md:81`) | CONFIRMED | `docs/oxiscript/abi.md` §3.1 ("`oxi_collect_cycles` is called by the host between events, never from inside a call"; line 80 in this worktree) |
| F7 | 8.Mem compares wasmi vs w2c2 on the Pi | CONFIRMED | roadmap 8.Mem scope (`docs/roadmap/oxiscript.md:476`); needs 8.2b first |
| F8 | D-20: no claim on the sites without measurements | CONFIRMED | `docs/roadmap/oxiscript.md:110` |
| F9 | Claims rules C1-C6 (frozen reports only, ranges, six-tuple disclosure, losses shown, no cross-axis/energy extrapolation, staleness) | CONFIRMED | sound; matches SPEC fair-use spirit and OxiBench §8 |
| F11 | Per-language verification gate by "a second reviewer (human or separate agent instance)" | PARTLY | A2 |
| F10 | C6 "two major/LTS releases behind → archived" | CONFIRMED | sound policy (opinion) |

## G. Ledger: OxiPerf / OxiBench separation (§7)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| G1 | OxiBench is 8.OB, blocked on the owner | CONFIRMED | `docs/roadmap/oxiscript.md:511` ("8.OB OxiBench (blocked on the owner)") |
| G2 | OxiBench role R2 "nobody grades their own work", freeze §7 | CONFIRMED | `oxibench-methodology.md` §1, §7 |
| G3 | OxiBench uses a private held-out test set with a canary | CONFIRMED | `oxibench-methodology.md` §3 ("Data sets and contamination") [content not reopened in detail] |
| G4 | OxiPerf fully public | CONFIRMED | sound: performance benchmarks need no held-out set, they need upstream ports (A2) |
| G5 | Shared stats crate `oxiscript-bench-stats` | UNVERIFIABLE | proposal; no such crate. Fine as an idea |

## H. Judgment of the methodology

**Complete?** Mostly. The report covers the known failure modes (selective reporting, warmup, nested variance, layout bias, semantic gaps, memory and size axes, preregistration) and maps each to a rule. The gaps are the ones in section A: a gameable 8.Lab gate (A1), self-written competitor ports (A2), selection bias in the headline suite (A3), fixed run counts where the cited method derives them (A4), a too-short warmup series (A5), float contraction (A6), and no multiple-comparison control (C18). Also missing: measuring wasmi with fuel off (C11), a per-row expected time range (E10), and a code-layout residual-bias statement (C14).

**Practical for one owner?** Not as written for v0.1. Rough budget for the Pi alone: 12 benchmarks × about 7 rows (C, Rust, wasmi, w2c2, CPython, Lua, LuaJIT) × 10 invocations × (warmup + 30 iterations). With iterations of 0.1-2 s (C to CPython at the `device` size) that is roughly 1-2 days of wall time per frozen run, which is acceptable overnight. The real cost is people-time, not machine time: about 12 × 9 ≈ 100 `base` ports plus `peak` ports, each with goldens for three size profiles, plus three drivers (Linux, browser, Pi over SSH, ESP32 serial). Recommended v0.1 scope:
* dev box only (x86-64), AWFY subset P01-P06 with **upstream AWFY ports** for JS, Lua, Python and C++ (as the native reference), plus OxiScript on wasmi and Rust; DeltaBlue only after its licence is cleared (E13); P07-P12 run internally for 8.Perf/8.Lab but are not published cross-language yet;
* Pi 1 B+ rows (C, wasmi, LuaJIT, Lua, CPython; w2c2 once 8.2b lands) as the 8.Mem part, under the D-20 claims rules;
* ESP32-S3 only after 8.S3 and 8.2b, with C, w2c2 and MicroPython first.

**Honest? What can still be gamed?**
1. The 8.Lab regression test (A1): fix first, it gates weekly releases.
2. Who writes the competitors (A2): use upstream ports, publish new ones for comment.
3. Which benchmarks headline (A3): AWFY subset only; home workloads labelled.
4. Choice of sizes and `inner_reps` after seeing results: freeze them (A3).
5. Many comparisons, few claims (C18): pre-register the headline comparisons.
6. `base` rules that suit OxiScript (TypedArray in JS, struct layouts specified per task): every such rule should be written in the task spec before any port exists, and checked by someone outside the OxiScript author pipeline.
7. Interpreter vs AOT framing: the report is already explicit here (§3.1); keep it.

**Recommendation for the roadmap.** Keep [8.Bench](../roadmap/oxiscript.md#s-8-bench) as defined (harness + the four SIMD/GPU workloads; it blocks 8.3a/b/d). Make the cross-language OxiPerf suite a separate part of 8.Bench (or a follow-up stage), gated on owner approval (Q-OP-1) and, for the w2c2 and `micro` rows, on [8.2b](../roadmap/oxiscript.md#s-8-2b). Adopt A1 before 8.Lab's first weekly alpha.
