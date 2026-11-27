# OxiPerf v0: the performance benchmark harness

OxiPerf measures how fast and how small OxiScript programs are, next to the same algorithms written in Rust, C and JavaScript. v0 is a **harness**: one command, honest numbers for the machine it runs on, everything recorded. It is not a frozen report, and nothing from it goes on a website (see [claims policy](#claims-policy)). The agent usability benchmark is a different thing ([OxiBench](README.md)).

Method: [cross-language-benchmark-methodology.md](../research/cross-language-benchmark-methodology.md) with its [Opus verification](../research/cross-language-benchmark-methodology-verification.md) (corrections A1-A10 followed where v0 can, see "Corrections applied"; rule numbers `R01`.. below are from it). Roadmap: stage [8.Bench](../roadmap/oxiscript.md#s-8-bench).

## Run it

```bash
export TMPDIR=/path/with/space       # C executables and smoke-test output go here
cargo run -p oxiscript-perf --release --features wasmtime -- run   # without --features wasmtime the Wasmtime rows are reported as unavailable
```

Writes `bench/oxiperf/results/<date>-<cpu>/` with:

| File | Content |
|---|---|
| `manifest.json` | machine (CPU, cores, RAM, OS, governor, turbo, ASLR, load, temperature before and after), pinned tool versions (rustc, wasmi, wasmtime, node, V8, gcc), git commit, run plan, which executors were available, `runtime_mode`, `publishable: false` and why |
| `raw.jsonl` | one line per child process: every iteration's nanoseconds (nothing is trimmed, R17), checksum, fuel, RSS, pages, plus `artifact` lines (sizes, compile latency) and `digest` lines (checksum agreement) |
| `summary.json` | computed from `raw.jsonl` only: medians, intervals, ratios, verdicts, geomeans |
| `report.md` | the tables |
| `artifacts/*.wasm` | the compiled OxiScript programs |

Counts are **derived, not fixed** (verification A4): the harness first runs a pilot of 3 invocations x 15 iterations for every cell (workload x executor), estimates the variance between invocations and within one, and the cost of an invocation, and computes iterations per invocation and number of invocations after Kalibera & Jones (iterations balance invocation cost against variance; invocations reach a 2% half-width of the 95% interval of the mean), clamped to 3..10 invocations and 10..50 iterations. The plan is written to `manifest.json` (`plan.cells`) and to `raw.jsonl` (`plan` lines) before the first measured run, and pilot runs stay in `raw.jsonl` but are not used in the statistics. `--invocations N --iters N` (or `--quick`) replaces this by a fixed plan. The clamps are a dev-tier budget; a release tier needs at least 10 invocations per cell.

Other commands: `oxiperf analyze --input DIR` rebuilds `summary.json` and `report.md` from `raw.jsonl` alone (statistics are never edited by hand). Useful flags of `run`: `--quick` (tiny sizes, one invocation; what the smoke test uses), `--workloads sort,tree`, `--engines oxiscript-wasmi,rust-native`, `--invocations N` (default 5), `--iters N` (30), `--warmup N` (3), `--min-iter-ms MS` (10), `--pin-core C` (runs every child under `taskset`), `--out DIR`. The exit code is non-zero if any checksum disagrees or any child run is invalid.

`--no-default-features` drops the Wasmtime executor and with it the long Cranelift build (the Wasmtime rows then say "unavailable"). The unit tests and the debug smoke test run without Cranelift: `cargo test -p oxiscript-perf --no-default-features`. Build with `CARGO_BUILD_JOBS=2` on a shared machine; per-package `-p` only.

## What is measured

Each (workload, executor, invocation) is a **separate child process**; executors are interleaved round-robin across invocations (R13) and each invocation gets a different, deterministic `OXIPERF_ENV_PAD` environment padding (R14). Per child: one cold call, `warmup` discarded calls, then `iters` timed iterations of `inner_reps` calls (chosen so one iteration lasts at least 10 ms, R15). Reported times are nanoseconds per single call.

| Executor | What it is |
|---|---|
| `oxiscript-wasmi` | the OxiScript `.wasm` in our runtime (`oxiscript-runtime`, wasmi 2, fuel metering, limits, validation). **Runtime mode `stub`** (see below). |
| `oxiscript-wasmtime` | the same `.wasm` in the Wasmtime crate (Cranelift, module compiled once per process, fresh instance per call, `oxi_init` outside the timer) |
| `rust-native` | the Rust twin compiled into the harness (cargo release: opt-level 3, no LTO). **The baseline of all ratios.** |
| `c-gcc` | the C reference, `gcc -O2 -ffp-contract=off` (never `-ffast-math`; a reference point, not a ceiling; absent for `map_set`: C has no standard hash map) |
| `js-node` | the JavaScript twin on Node (V8), typed arrays for typed data |
| `oxiscript-w2c2` | **TODO, no rows**: w2c2 is not installed here; it is listed in the manifest as a TODO, never faked |

Workloads (sources in `bench/oxiperf/suites/<id>/`, one file per language, the same algorithm and the same checksum), in two groups that are never mixed (verification A2, A3):

- **`origin awfy`** (the only candidate for a headline): `awfy_mandelbrot` (P01) and `awfy_sieve` (P03) of [Are We Fast Yet](https://github.com/smarr/are-we-fast-yet). The JavaScript twin is the **upstream file, vendored verbatim** in `suites/<id>/upstream/` (MIT, per-file headers kept, CLBG notice for Mandelbrot, commit pinned in `PIN`), adapted only by our `main.js` wrapper. AWFY has no Rust, C or OxiScript ports, so those three twins are **our ports** of the upstream algorithm; they are therefore not independent of the OxiScript author, and the report says so. Mandelbrot is verified against upstream's own value (size 500 gives 191, Sieve 669 primes). Not yet taken: Storage, Json, NBody, DeltaBlue (its old Sun licence terms must be cleared first, so it is **not copied**), Richards, CD, Havlak, Bounce, List, Permute, Queens, Towers.
- **`origin own`** (our workloads, all twins written here; reported separately, never a headline): `sort` (hand-written merge sort of i32), `map_set` (**stdlib track**: `std/collections` Map and Set against each language's built-in map), `string_template` (template literal formatting of integers, hashed), `json_tokenize` (build a JSON-like byte buffer, hand-written tokenizer), `tree` (binary-trees allocation churn), `matmul` (f64 dense multiply with inputs that are multiples of 1/4, so the checksum is exact in any summation order). Sizes are tuned so one wasmi iteration takes roughly 30 to 400 ms on the dev box and are part of the frozen plan (they change only with a suite version bump). Every executor returns a `u32` checksum; **every child run's checksum is validated as a non-negative `u32` integer and compared across all runs of that workload** (R01, section 4.3). Any missing, `null`, negative, fractional, string, boolean, or out-of-range (`> u32::MAX`) checksum — or any checksum disagreement or instability across iterations — marks the run invalid, sets `digest_ok` / `digests_ok` to `false`, suppresses verdicts and geomeans for that workload, and causes `oxiperf run` to exit non-zero (both during `run` and when re-analyzed from `raw.jsonl` via `analyze`). For wasmi, **fuel is also checked to be identical on every iteration** (determinism).

The OxiScript sources use `@host export function run(n: i32): u32` and an empty `main`, so a module has no imports and no capabilities; they pass `oxi check`.

## Columns of `report.md`

| Column | Meaning |
|---|---|
| median ms | median of all measured iterations of all invocations (R16) |
| 95% CI | hierarchical bootstrap of that median: resample invocations with replacement, then iterations inside each; 10 000 resamples, seeded, so reruns of `analyze` are identical |
| p95 | 95th percentile of the pooled iterations; `max` and `min` are in `summary.json` |
| vs rust-native | ratio of medians with its bootstrap interval |
| verdict | R18 for external comparisons: "slower"/"faster" only if the interval excludes 1.0 **and** the point estimate is at least 10% away; otherwise "parity / within noise" |
| origin group | the geometric mean of the ratios is computed separately for `awfy` and `own`, never over both |
| fuel | wasmi's deterministic metering. Static per control-flow region, so **never compare fuel across engines, and never call a fuel drop a speedup on its own** (R11). Its use: an exact, noise-free regression signal for one program. |
| warm-up drift | first third against last third of the iterations of each invocation (`steady` within 5%); a stand-in for the changepoint test (TODO) |
| cold process ms | spawn to exit of a child that runs the workload once: process start, engine start, load, first call |
| load ms | module load and validation/translation (wasmi), compile (Wasmtime, plus instantiation), instantiation (native: 0) |
| first call ms | the cold first execution |
| compile ms | source to artifact: OxiScript in-process (check, lowering, Wasm codegen, validation; first call and median of the rest), gcc best of 3 |
| peak RSS | `VmHWM` of the child (median over invocations). Includes the engine itself, so compare Wasm engines with each other and read Node/native as a ceiling |
| Wasm pages | linear memory in 64 KiB pages after one call |
| artifact | `.wasm` bytes, C executable bytes (unstripped), JS source bytes |

## Corrections applied from the verification

| # | Applied in v0 |
|---|---|
| A2 | AWFY JavaScript is the vendored upstream file; our ports are labelled as ours; own workloads are a separate group |
| A3 | headline candidate = AWFY group only; sizes frozen in the plan |
| A4 | repetition counts derived from a pilot (Kalibera & Jones), recorded and frozen |
| A6 | every C build uses `-ffp-contract=off`; no f32 workload exists yet, so no `Math.fround` need arises (add it with the first f32 workload) |
| A10 | only the rows that can run today: wasmi, Wasmtime, Rust, C, Node; w2c2 is an explicit TODO |
| C7 | the C row is called a reference (`-O2`), not a ceiling |
| not yet | A1 (8.Lab non-inferiority gate: belongs to `compare`, not built), A5 (long warm-up series for JIT rows: v0 has the short drift check only), C11 (wasmi wall time with fuel metering off: the runtime has no switch for it, so **the wasmi rows include the metering tax**), C18 (Holm-adjusted intervals: v0 prints per-comparison verdicts, no claim should be drawn from the family), A7 (LuaJIT, Pi rows) |

## How part 1 (SIMD/GPU) fits the same harness

Stage 8.Bench has two parts. v0 implements the harness and a subset of part 2 (the cross-language suite, an unfrozen dev report until the owner approves the methodology, Q-OP-1; the manifest and every report say so). Part 1 (GEMM, image filter, particles, MLP on wasmi/Wasmtime scalar and SIMD, browser, GPU resident and round trip) needs no new machinery: its workloads are another entry group in `workloads.rs` (a new `Origin`, its own geomean group and its own report section), its executors are new `Engine` variants (`wasmi-simd`, `wasmtime-simd` as another module compiled with the SIMD target; browser and GPU as drivers that speak the same one-JSON-line child protocol, like Node and C do today). GPU rows run only with the owner's consent (the real GPU drives the owner's display) or through lavapipe. The statistics, manifest, plan and report code is shared.

## Which runtime mode, and why it matters

v0 measures `runtime_mode = stub`: the `.wasm` embeds the test stub runtime, a bump allocator that never reuses memory. Consequences, all stated in every report: each timed call uses a **fresh instance** (created outside the timer), so allocation-heavy rows (`tree`, `string_template`, `json_tokenize`, `map_set`) are flattered by cheap bump allocation and show no free-list or reuse cost; peak pages are an upper bound of live data. f64 to text does not exist yet, so no workload formats floats. The fast allocator and f64 to text arrive with stage 8.M-link (branch `gemini-rtlink`). **Re-run after that merge**: the harness records `runtime_mode` in the manifest and in every raw and summary row; once the compiler offers the `Linked` mode, add it to `RUNTIME_MODE` handling in `driver.rs` (one value) and compare the two reports side by side.

## Honest limits of v0

- Dev tier, not the R15 release tier: 5 invocations instead of 10, no isolated core, no controlled DVFS (governor `schedutil`, turbo on, a shared machine with other work running; the manifest records governor, load average and temperature at start and end and sets `uncontrolled_dvfs`). Wall times from such a run are **never publishable** (R13).
- The twins are written by one author, not reviewed by a second one (section 4.3 of the methodology).
- Only Rust, C and JavaScript (Node) next to OxiScript; no Go, AssemblyScript, Python, Lua; no browser, no boards; only the `base` track plus one stdlib workload, no `peak` track.
- Rust is compiled with the cargo release profile (no LTO, 16 codegen units), which also builds wasmi; the numbers are not the best each tool could do.
- No live-object count after cycle collection, no PSS, no changepoint test, no wasmi executed-op counter.

## Claims policy

From the methodology (section 6), binding for this project:

1. **No performance or memory claim on a website, README, release note or blog from a harness run.** Public numbers come only from an owner-approved, frozen `release`-tier report with a freeze commit, manifest and raw data. `dev` and `ci` tier output, uncontrolled-DVFS numbers and estimates are never published.
2. Ranges and intervals, never one cherry-picked winner: the whole suite, min to max, geomean with its interval, wins and losses together.
3. Every public number carries: OxiScript version and exact engine and runtime mode, the competitor and its exact version, the track (`base` or `peak`), hardware and OS, methodology and suite version and date, a link to the frozen report and how to reproduce it.
4. Losses and ceilings are shown next to wins (interpreter against V8's JIT, wasmi and Wasmtime against native). Semantic differences (trapping integer overflow, bounds checks, reference counting) are named, not used as excuses.
5. No cross-axis claims (a speed ratio from one machine with a memory figure from another) and no energy claims from wall time.
6. Different methodology versions are never mixed in one chart; stale competitor versions are marked.

## Adding a workload

Create `bench/oxiperf/suites/<id>/` with `main.oxs` (`@host export function run(n: i32): u32`, `export function main(): void {}`; it must pass `oxi check`), `main.js` (`exports.run`), `main.rs` (`pub fn run(n: i32) -> u32`), optionally `main.c` (`uint32_t run(int32_t n)` and `#include "../../drivers/c_harness.h"` at the end); register it in `oxiscript/oxiscript-perf/src/workloads.rs` and `native.rs`. Make the checksum use only `u32` wrapping arithmetic (JS has no 64-bit integers) and never depend on hash-map iteration order or f64 summation order.
