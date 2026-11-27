# OxiPerf report: 2026-11-26-i5-3450

**Suite: 8.Bench part 2 (cross-language suite), v0 subset: unfrozen dev report, gated on owner approval Q-OP-1.**

**Not publishable.** dev tier: fewer invocations than R15 (10), no controlled DVFS, no isolated core; harness output, not a frozen report Tier `dev`; harness version 0.0.2.

**OxiScript runtime mode measured: `stub`** (test-stub runtime with a bump allocator that never reuses memory; the real runtime and f64-to-text arrive with 8.M-link, re-run after that merge). Allocation-heavy rows (`tree`, `string_template`, `json_tokenize`, `map_set`) are flattered by bump allocation, and every iteration uses a fresh instance.

**Precision target not reached:** 34 of 39 cells hit the cap of 8 invocations, so the wanted 2% half-width of the interval was limited by the time budget (and by a loaded, uncontrolled machine), not by the measured noise. Read the intervals, not the medians alone.

Checksums: **all executors agree on every workload**.

## Machine

| | |
|---|---|
| CPU | Intel(R) Core(TM) i5-3450 CPU @ 3.10GHz |
| Logical CPUs / RAM | 4 / 23973 MiB |
| OS | Linux version 6.17.0-42-generic |
| Governor / turbo disabled | schedutil" / false |
| Pinned core | null |
| Uncontrolled DVFS | true |
| Load average at start / end | 4.87 / 10.9 |
| Git commit | 35725d153891abd323afa8c53b1e00c28545dcf5" (dirty: false) |
| Plan | derived per cell from a pilot (3 invocations x 15 iterations) after Kalibera & Jones: iterations balance invocation cost against variance, invocations reach a 2% half-width of the 95% interval of the mean; clamped to 3..8 invocations and 10..30 iterations (release tier would need at least 10 invocations); 3 warm-up calls, at least 10.0 ms per timed iteration; per-cell counts in manifest.json |

## Workloads

| id | origin | what | track | n | n means |
|---|---|---|---|---:|---|
| awfy_mandelbrot | awfy | AWFY Mandelbrot | base | 500 | image size (upstream verifies 500 -> 191) |
| awfy_sieve | awfy | AWFY Sieve | base | 500 | sieves of 5000 (669 primes each) |
| sort | own | merge sort | base | 60000 | i32 keys |
| map_set | own | Map/Set churn | stdlib | 20000 | inserts, lookups, removals each |
| string_template | own | string template | base | 12000 | rows |
| json_tokenize | own | JSON tokenizer | base | 6000 | records (about 80 bytes each) |
| tree | own | tree alloc/walk | base | 13 | tree depth (8 trees) |
| matmul | own | matmul f64 | base | 100 | matrix dimension |

`origin awfy` = Are We Fast Yet: the JavaScript twin is the **upstream file, vendored verbatim at a pinned commit** (`upstream/PIN`); the Rust, C and OxiScript twins are our ports of the same algorithm because no upstream port exists (so they are not independent of the OxiScript author). `origin own` = our own workload, every twin written here, **reported separately and never a headline**. `base` = identical algorithm in every language (sources in `bench/oxiperf/suites/<id>/`); `stdlib` = each language's built-in hash map/set (the C row is absent: C has none). Every executor returns a u32 checksum and all must agree.
## Tools

| Tool | Version | Flags |
|---|---|---|
| gcc | gcc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0 | -O2 -ffp-contract=off -DNDEBUG |
| node | v20.20.2 |  |
| oxiscript | 0.0.2 |  |
| rustc | rustc 1.98.0 (88d9e12ae 2026-08-18) | cargo release profile: opt-level=3, codegen-units=16, no LTO, panic=unwind |
| w2c2 | TODO: not installed on this machine, no rows are produced |  |
| wasmi | 2.0.3 |  |
| wasmtime | 51.0.0 | Engine::default() (Cranelift, opt_level speed), module compiled once per process |

## Wall time (warm, steady state)

Median over all measured iterations of all invocations, 95% hierarchical bootstrap interval of the median (10000 resamples), p95, and the ratio to `rust-native` with its interval. `verdict` follows R18: a difference counts only if the interval excludes 1.0 and the point estimate is at least 10% away. `n` is the problem size. Fuel is wasmi's deterministic metering and is **not** comparable across engines.

| workload (origin, track, n) | engine | median ms | 95% CI ms | p95 ms | vs rust-native | verdict | fuel | warm-up drift |
|---|---|---:|---|---:|---|---|---:|---|
| awfy_mandelbrot (awfy, base, 500) | oxiscript-wasmi | 1053 | 827 to 1301 | 1811 | 15.15x [11.70, 18.78] | slower | 1976420138 | slowdown+speedup+steady |
| awfy_mandelbrot (awfy, base, 500) | oxiscript-wasmtime | 86.4 | 75.0 to 109 | 208 | 1.24x [1.04, 1.54] | slower | - | slowdown+speedup+steady |
| awfy_mandelbrot (awfy, base, 500) | rust-native | 69.5 | 64.7 to 77.5 | 146 | baseline | - | - | slowdown+speedup+steady |
| awfy_mandelbrot (awfy, base, 500) | c-gcc | 75.4 | 63.5 to 136 | 261 | 1.09x [0.89, 1.98] | parity / within noise | - | slowdown+speedup+steady |
| awfy_mandelbrot (awfy, base, 500) | js-node | 71.7 | 67.4 to 99.0 | 183 | 1.03x [0.91, 1.44] | parity / within noise | - | slowdown+speedup+steady |
| awfy_sieve (awfy, base, 500) | oxiscript-wasmi | 472 | 390 to 572 | 865 | 34.48x [21.32, 49.57] | slower | 761602750 | slowdown+speedup+steady |
| awfy_sieve (awfy, base, 500) | oxiscript-wasmtime | 34.8 | 33.7 to 47.6 | 88.3 | 2.54x [1.71, 3.74] | slower | - | slowdown+speedup+steady |
| awfy_sieve (awfy, base, 500) | rust-native | 13.7 | 10.5 to 20.0 | 32.5 | baseline | - | - | slowdown+speedup+steady |
| awfy_sieve (awfy, base, 500) | c-gcc | 7.22 | 6.71 to 11.5 | 16.5 | 0.53x [0.35, 0.88] | faster | - | slowdown+speedup+steady |
| awfy_sieve (awfy, base, 500) | js-node | 31.2 | 27.0 to 40.3 | 85.9 | 2.28x [1.46, 3.45] | slower | - | slowdown+speedup+steady |
| sort (own, base, 60000) | oxiscript-wasmi | 184 | 162 to 266 | 568 | 28.84x [23.28, 40.90] | slower | 239141117 | slowdown+speedup+steady |
| sort (own, base, 60000) | oxiscript-wasmtime | 12.6 | 11.8 to 25.6 | 51.2 | 1.98x [1.66, 3.99] | slower | - | slowdown+speedup+steady |
| sort (own, base, 60000) | rust-native | 6.37 | 6.08 to 7.38 | 11.6 | baseline | - | - | slowdown+speedup+steady |
| sort (own, base, 60000) | c-gcc | 6.92 | 6.09 to 12.6 | 16.5 | 1.09x [0.87, 1.94] | parity / within noise | - | slowdown+speedup |
| sort (own, base, 60000) | js-node | 13.7 | 13.2 to 16.0 | 50.5 | 2.15x [1.84, 2.55] | slower | - | slowdown+speedup+steady |
| map_set (own, stdlib, 20000) | oxiscript-wasmi | 120 | 108 to 159 | 336 | 34.52x [21.17, 48.30] | slower | 95052681 | slowdown+speedup+steady |
| map_set (own, stdlib, 20000) | oxiscript-wasmtime | 18.3 | 9.27 to 23.9 | 54.7 | 5.26x [2.47, 7.27] | slower | - | slowdown+speedup |
| map_set (own, stdlib, 20000) | rust-native | 3.49 | 3.08 to 5.66 | 8.74 | baseline | - | - | speedup+steady |
| map_set (own, stdlib, 20000) | js-node | 8.24 | 6.22 to 13.4 | 20.2 | 2.36x [1.31, 4.10] | slower | - | slowdown+speedup+steady |
| string_template (own, base, 12000) | oxiscript-wasmi | 272 | 222 to 443 | 684 | 79.50x [45.73, 129.67] | slower | 204150668 | slowdown+speedup |
| string_template (own, base, 12000) | oxiscript-wasmtime | 18.6 | 16.5 to 24.7 | 74.7 | 5.46x [2.97, 7.24] | slower | - | slowdown+speedup+steady |
| string_template (own, base, 12000) | rust-native | 3.42 | 3.29 to 6.29 | 20.0 | baseline | - | - | slowdown+speedup+steady |
| string_template (own, base, 12000) | c-gcc | 6.31 | 3.76 to 9.02 | 12.1 | 1.85x [0.88, 2.55] | parity / within noise | - | slowdown+speedup |
| string_template (own, base, 12000) | js-node | 8.35 | 6.93 to 13.1 | 22.7 | 2.44x [1.38, 3.59] | slower | - | slowdown+speedup+steady |
| json_tokenize (own, base, 6000) | oxiscript-wasmi | 138 | 115 to 234 | 404 | 97.03x [62.83, 166.80] | slower | 119675338 | slowdown+speedup+steady |
| json_tokenize (own, base, 6000) | oxiscript-wasmtime | 7.83 | 7.05 to 12.4 | 21.2 | 5.50x [3.67, 8.39] | slower | - | slowdown+speedup+steady |
| json_tokenize (own, base, 6000) | rust-native | 1.42 | 1.36 to 2.14 | 3.02 | baseline | - | - | slowdown+speedup+steady |
| json_tokenize (own, base, 6000) | c-gcc | 2.08 | 1.38 to 2.99 | 4.87 | 1.46x [0.84, 2.12] | parity / within noise | - | slowdown+speedup+steady |
| json_tokenize (own, base, 6000) | js-node | 6.23 | 5.67 to 12.9 | 25.3 | 4.38x [2.92, 9.07] | slower | - | slowdown+speedup+steady |
| tree (own, base, 13) | oxiscript-wasmi | 117 | 99.1 to 154 | 318 | 25.38x [19.48, 35.73] | slower | 89418873 | slowdown+speedup |
| tree (own, base, 13) | oxiscript-wasmtime | 8.89 | 8.07 to 14.7 | 54.2 | 1.93x [1.55, 3.27] | slower | - | slowdown+speedup+steady |
| tree (own, base, 13) | rust-native | 4.60 | 4.06 to 5.54 | 8.49 | baseline | - | - | slowdown+speedup+steady |
| tree (own, base, 13) | c-gcc | 5.20 | 4.26 to 9.34 | 19.1 | 1.13x [0.82, 2.15] | parity / within noise | - | slowdown+speedup+steady |
| tree (own, base, 13) | js-node | 6.10 | 4.32 to 7.71 | 13.6 | 1.32x [0.91, 1.81] | parity / within noise | - | slowdown+speedup |
| matmul (own, base, 100) | oxiscript-wasmi | 130 | 111 to 206 | 455 | 80.46x [44.10, 142.74] | slower | 170694912 | slowdown+speedup+steady |
| matmul (own, base, 100) | oxiscript-wasmtime | 7.12 | 6.44 to 8.44 | 13.9 | 4.40x [2.41, 6.11] | slower | - | slowdown+speedup+steady |
| matmul (own, base, 100) | rust-native | 1.62 | 1.28 to 2.80 | 9.49 | baseline | - | - | slowdown+speedup+steady |
| matmul (own, base, 100) | c-gcc | 1.66 | 1.11 to 2.17 | 4.98 | 1.02x [0.49, 1.54] | parity / within noise | - | slowdown+speedup+steady |
| matmul (own, base, 100) | js-node | 6.43 | 4.45 to 8.73 | 17.3 | 3.97x [2.07, 6.23] | slower | - | slowdown+speedup+steady |

Geometric mean of the ratio to `rust-native` **per origin group** (`awfy` is the only candidate for a headline, `own` is a separate labelled number; both only mean something next to the table above; min and max are the extremes):

| origin | engine | benchmarks | geomean | min | max |
|---|---|---:|---:|---:|---:|
| awfy | c-gcc | 2 | 0.757x | 0.528x | 1.09x |
| awfy | js-node | 2 | 1.53x | 1.03x | 2.28x |
| awfy | oxiscript-wasmi | 2 | 22.9x | 15.2x | 34.5x |
| awfy | oxiscript-wasmtime | 2 | 1.78x | 1.24x | 2.54x |
| own | c-gcc | 5 | 1.28x | 1.02x | 1.85x |
| own | js-node | 6 | 2.57x | 1.32x | 4.38x |
| own | oxiscript-wasmi | 6 | 50.0x | 25.4x | 97.0x |
| own | oxiscript-wasmtime | 6 | 3.72x | 1.93x | 5.50x |

## Startup and compile latency

`cold process` = spawn to exit of a child that only runs the workload once (process start, engine start, load, first call). `load` = module load, validation/translation or compile, and instantiation of the first instance (Wasm engines). `first call` = the cold first execution. `compile` = source to artifact (OxiScript: in-process check, lowering, codegen and validation, median of 6 after a first call; C: gcc best of 3 with link; JS and the Rust baseline have no ahead-of-time step here).

| workload | engine | cold process ms | load ms | first call ms | compile ms (first / median) |
|---|---|---:|---:|---:|---|
| awfy_mandelbrot | oxiscript-wasmi | 1168 | 1.11 | 1159 | 1.75 / 1.34 |
| awfy_mandelbrot | oxiscript-wasmtime | 186 | 30.9 | 150 | 1.75 / 1.34 |
| awfy_mandelbrot | rust-native | 123 | 0.000 | 103 | - |
| awfy_mandelbrot | c-gcc | 132 | - | 130 | 101 |
| awfy_mandelbrot | js-node | 157 | - | 80.2 | - |
| awfy_sieve | oxiscript-wasmi | 573 | 0.834 | 566 | 1.31 / 1.11 |
| awfy_sieve | oxiscript-wasmtime | 60.7 | 17.4 | 36.8 | 1.31 / 1.11 |
| awfy_sieve | rust-native | 19.9 | 0.000 | 13.1 | - |
| awfy_sieve | c-gcc | 10.7 | - | 9.09 | 102 |
| awfy_sieve | js-node | 104 | - | 43.4 | - |
| sort | oxiscript-wasmi | 194 | 0.941 | 189 | 1.95 / 1.64 |
| sort | oxiscript-wasmtime | 52.4 | 27.3 | 14.5 | 1.95 / 1.64 |
| sort | rust-native | 9.96 | 0.000 | 6.99 | - |
| sort | c-gcc | 14.9 | - | 6.41 | 121 |
| sort | js-node | 128 | - | 44.4 | - |
| map_set | oxiscript-wasmi | 214 | 2.48 | 188 | 7.43 / 6.89 |
| map_set | oxiscript-wasmtime | 149 | 116 | 28.9 | 7.43 / 6.89 |
| map_set | rust-native | 8.37 | 0.000 | 4.07 | - |
| map_set | js-node | 88.2 | - | 24.5 | - |
| string_template | oxiscript-wasmi | 280 | 0.929 | 275 | 1.65 / 1.42 |
| string_template | oxiscript-wasmtime | 66.2 | 32.7 | 25.5 | 1.65 / 1.42 |
| string_template | rust-native | 6.46 | 0.000 | 3.37 | - |
| string_template | c-gcc | 16.7 | - | 10.0 | 114 |
| string_template | js-node | 78.0 | - | 21.6 | - |
| json_tokenize | oxiscript-wasmi | 183 | 1.55 | 175 | 2.26 / 2.02 |
| json_tokenize | oxiscript-wasmtime | 51.3 | 28.9 | 9.89 | 2.26 / 2.02 |
| json_tokenize | rust-native | 4.59 | 0.000 | 2.31 | - |
| json_tokenize | c-gcc | 5.95 | - | 2.88 | 148 |
| json_tokenize | js-node | 117 | - | 32.8 | - |
| tree | oxiscript-wasmi | 119 | 0.860 | 114 | 1.69 / 1.62 |
| tree | oxiscript-wasmtime | 34.4 | 18.9 | 9.93 | 1.69 / 1.62 |
| tree | rust-native | 8.79 | 0.000 | 4.59 | - |
| tree | c-gcc | 9.01 | - | 7.63 | 186 |
| tree | js-node | 106 | - | 29.1 | - |
| matmul | oxiscript-wasmi | 150 | 0.931 | 145 | 1.42 / 1.37 |
| matmul | oxiscript-wasmtime | 33.9 | 18.7 | 7.12 | 1.42 / 1.37 |
| matmul | rust-native | 6.87 | 0.000 | 1.93 | - |
| matmul | c-gcc | 4.62 | - | 1.66 | 123 |
| matmul | js-node | 130 | - | 29.1 | - |

## Memory and size

`peak RSS` = `VmHWM` of the child process (median of invocations; includes the engine itself, so compare Wasm engines with each other and read the others as ceilings). `pages` = Wasm linear memory in 64 KiB pages after one call (stub runtime: bump allocation, no reuse). `artifact` = `.wasm` file, C executable (unstripped) or JS source.

| workload | engine | peak RSS MiB | Wasm pages | artifact |
|---|---|---:|---:|---|
| awfy_mandelbrot | oxiscript-wasmi | 5.95 | 2 | wasm 4430 B |
| awfy_mandelbrot | oxiscript-wasmtime | 12.1 | 2 | wasm 4430 B |
| awfy_mandelbrot | rust-native | 3.81 | - |  - |
| awfy_mandelbrot | c-gcc | 8.74 | - | executable 16.0 KiB |
| awfy_mandelbrot | js-node | 43.2 | - | source 450 B |
| awfy_sieve | oxiscript-wasmi | 13.9 | 126 | wasm 4099 B |
| awfy_sieve | oxiscript-wasmtime | 19.9 | 126 | wasm 4099 B |
| awfy_sieve | rust-native | 3.80 | - |  - |
| awfy_sieve | c-gcc | 8.74 | - | executable 16.1 KiB |
| awfy_sieve | js-node | 43.2 | - | source 504 B |
| sort | oxiscript-wasmi | 7.09 | 17 | wasm 5155 B |
| sort | oxiscript-wasmtime | 13.2 | 17 | wasm 5155 B |
| sort | rust-native | 3.96 | - |  - |
| sort | c-gcc | 8.74 | - | executable 16.1 KiB |
| sort | js-node | 51.5 | - | source 842 B |
| map_set | oxiscript-wasmi | 10.1 | 63 | wasm 10.7 KiB |
| map_set | oxiscript-wasmtime | 16.2 | 63 | wasm 10.7 KiB |
| map_set | rust-native | 4.08 | - |  - |
| map_set | js-node | 57.9 | - | source 677 B |
| string_template | oxiscript-wasmi | 14.9 | 142 | wasm 5412 B |
| string_template | oxiscript-wasmtime | 21.1 | 142 | wasm 5412 B |
| string_template | rust-native | 3.83 | - |  - |
| string_template | c-gcc | 8.74 | - | executable 16.2 KiB |
| string_template | js-node | 57.9 | - | source 480 B |
| json_tokenize | oxiscript-wasmi | 8.70 | 42 | wasm 6089 B |
| json_tokenize | oxiscript-wasmtime | 14.7 | 42 | wasm 6089 B |
| json_tokenize | rust-native | 4.17 | - |  - |
| json_tokenize | c-gcc | 8.74 | - | executable 16.1 KiB |
| json_tokenize | js-node | 70.8 | - | source 1641 B |
| tree | oxiscript-wasmi | 8.17 | 33 | wasm 4047 B |
| tree | oxiscript-wasmtime | 14.1 | 33 | wasm 4047 B |
| tree | rust-native | 4.31 | - |  - |
| tree | c-gcc | 8.74 | - | executable 20.1 KiB |
| tree | js-node | 73.7 | - | source 490 B |
| matmul | oxiscript-wasmi | 6.88 | 13 | wasm 4798 B |
| matmul | oxiscript-wasmtime | 12.8 | 13 | wasm 4798 B |
| matmul | rust-native | 3.95 | - |  - |
| matmul | c-gcc | 8.74 | - | executable 16.1 KiB |
| matmul | js-node | 50.7 | - | source 604 B |

## Not measured yet (TODO, nothing here is faked)

- `oxiscript-w2c2` (wasm to C to gcc): w2c2 is not installed on this machine; no row.
- wasmi executed-op count (diagnostic mode of R11) and the `Linked` runtime mode: after 8.M-link merges, re-run and compare `runtime_mode`.
- Warm-up changepoint classification (Barrett et al.): only a first-third versus last-third drift check is done.
- R15 release sample sizes (10 invocations), controlled DVFS, isolated core, live-object count after cycle collection, PSS.
- Rust, Go, AssemblyScript, Python and Lua twins beyond the Rust baseline; browser and boards; the `peak` track.
- Per-language verification by a second reviewer (section 4.3 of the methodology); the twins here are written by one author.

See `docs/bench/oxiperf.md` for what each column means and for the claims policy (no website claims from a harness run).
