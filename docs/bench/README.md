# Benchmarks: OxiPerf and OxiBench

Two separate benchmarks with separate names, methods and reports (owner, 2026-11-26). They share parts of the harness (pinned toolchains, machine description, JSON results, frozen reports), never their numbers.

| | **OxiPerf** (performance) | **OxiBench** (agent usability) |
|---|---|---|
| Question | How fast and how small are OxiScript programs, and how do they compare with other languages fairly? | How well do AI models (Claude, Gemini, others) write correct OxiScript, compared with TypeScript? |
| Roadmap stage | [8.Bench](../roadmap/oxiscript.md#s-8-bench) (harness), feeds [8.Perf](../roadmap/oxiscript.md#s-8-perf), [8.Lab](../roadmap/oxiscript.md#s-8-lab), [8.Mem](../roadmap/oxiscript.md#s-8-mem), 8.3a-e; OxiUI side [U.Bench](../roadmap/oxiui.md#s-u-bench) | [8.OB](../roadmap/oxiscript.md#s-8-ob) |
| Measures | fuel per operation (deterministic, wasmi), wall time (wasmi, w2c2, Wasmtime, browser, GPU), peak memory and RSS, `.wasm` and firmware size, compile latency; versus JS/TS, AssemblyScript, Rust, C, Go/TinyGo, Python/MicroPython | pass@1 on hidden tasks under conditions C0-C3 (cold, docs, docs + check loop, own runs), compile errors per 100 lines, the gap to a TypeScript twin of each task |
| Methodology | [research/cross-language-benchmark-methodology.md](../research/cross-language-benchmark-methodology.md) and its [verification](../research/cross-language-benchmark-methodology-verification.md) + [research/oxiscript-memory-performance.md](../research/oxiscript-memory-performance.md) and its verification for the memory workloads | [research/oxibench-methodology.md](../research/oxibench-methodology.md) v0.1 |
| Who may claim results | only frozen, reproducible reports with tool versions, machine and date; ranges, not single numbers | only frozen reports with version, date and model IDs; negative results published |

## State (2026-11-26)

### OxiPerf

| Part | State |
|---|---|
| Name and scope | decided (this file) |
| Methodology of fair cross-language comparison | **written and verified** (Gemini draft, Opus verification 2026-11-26: 95 claims, 3 wrong, corrections inline); waiting for owner approval (Q-OP-1..7) |
| Harness crate `oxiscript-bench` (binary `oxiperf`: one command, table + JSON, machine description) | **v0 done** (2026-11-26): wasmi, Wasmtime, Rust, C, Node; manifest, raw.jsonl, summary.json, report.md; statistics unit-tested; repetition counts derived from a pilot (Kalibera & Jones); see [oxiperf.md](oxiperf.md). Dev tier only (capped at 10 invocations, no controlled DVFS, unfrozen). Part 1 of 8.Bench (SIMD/GPU workloads, browser, GPU targets) not started |
| Workloads | **2 Are We Fast Yet** (Mandelbrot, Sieve; JavaScript = upstream verbatim, pinned; Rust/C/OxiScript = our ports) as the headline candidate group, plus **6 own** workloads reported separately (sort, Map/Set on the stdlib track, string template, JSON tokenizer, tree alloc, matmul f64). Checksums verified across all languages. Not yet: the other AWFY benchmarks (Storage, Json, NBody, ...; DeltaBlue not copied until its licence is cleared), the SIMD/GPU workloads of the harness part. Existing pieces: fuel-per-operation probes in `oxiscript-lower/tests/bad_actor_link_perf.rs` (on branch `gemini-rtlink`), OxiUI frame cost in `oxiterm-server/benches/frame_cost.rs` |
| Reference implementations in other languages | **Rust, C, JavaScript (Node)** for the 8 workloads (AWFY JS is upstream; all other code one author, no independent audit yet). Not yet Go, AssemblyScript, Python, Lua; w2c2 is not installed (TODO row) |
| Real GPU | the dev machine has an AMD Radeon R7 260X (Vulkan via RADV); used only with the owner's consent because it drives the owner's display |
| Boards | Raspberry Pi 1 B+ waiting for setup (8.Mem); ESP32-S3 after 2026-12-10 (8.S3) |
| Runtime mode measured | `stub` (bump allocator, no memory reuse); re-run after 8.M-link merges (`Linked` mode, f64 to text) |
| First frozen report | none (a dev-tier harness run exists under `bench/oxiperf/results/`; it is not publishable) |

### OxiBench

| Part | State |
|---|---|
| Methodology v0.1 | **written**, waiting for owner approval (question Q-OB-1 in [owner-questions.md](../owner-questions.md)) |
| Private task set (outside the public repo) | **not started**; location to decide (Q-OB-2) |
| Public dev tasks `bench/dev/` | **not started** |
| Harness (roles, conditions C0-C3, k = 5 samples, bootstrap) | **not started**; needs `oxi check`/`oxi run` as tools (both exist) and MCP tools from 8.D1 for the agent-mode variant |
| Models and API keys | **open** (Q-OB-3) |
| First frozen report | none |
