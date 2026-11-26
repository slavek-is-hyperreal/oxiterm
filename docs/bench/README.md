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
| Harness crate `oxiscript-bench` (one command, table + JSON, machine description) | **not started** (stage 8.Bench) |
| Workloads | **not started** as a suite. Existing pieces: fuel-per-operation probes in `oxiscript-lower/tests/bad_actor_link_perf.rs` (on branch `gemini-rtlink`, merging soon; numbers in `docs/stages/8.M-link/`), OxiUI frame cost in `oxiterm-server/benches/frame_cost.rs` |
| Reference implementations in other languages | **not started** |
| Real GPU | the dev machine has an AMD Radeon R7 260X (Vulkan via RADV); used only with the owner's consent because it drives the owner's display |
| Boards | Raspberry Pi 1 B+ waiting for setup (8.Mem); ESP32-S3 after 2026-12-10 (8.S3) |
| First frozen report | none |

### OxiBench

| Part | State |
|---|---|
| Methodology v0.1 | **written**, waiting for owner approval (question Q-OB-1 in [owner-questions.md](../owner-questions.md)) |
| Private task set (outside the public repo) | **not started**; location to decide (Q-OB-2) |
| Public dev tasks `bench/dev/` | **not started** |
| Harness (roles, conditions C0-C3, k = 5 samples, bootstrap) | **not started**; needs `oxi check`/`oxi run` as tools (both exist) and MCP tools from 8.D1 for the agent-mode variant |
| Models and API keys | **open** (Q-OB-3) |
| First frozen report | none |
