# Stage 5.2: GPU tests in CI (Mesa lavapipe)

## What runs

`.github/workflows/ci.yml`, job `gpu` (parallel to `build`, skipped on the weekly schedule):

1. `apt-get install mesa-vulkan-drivers libvulkan1`, find `/usr/share/vulkan/icd.d/lvp_icd*.json` by glob (fails loudly if absent), export it as `VK_ICD_FILENAMES` and `VK_DRIVER_FILES`; print `dpkg -l mesa-vulkan-drivers 'libllvm*'` so the runner's versions are in the log.
2. `cargo test --release -p oxiterm-gpu -- --test-threads=1 --nocapture` with `WGPU_BACKEND=vulkan` and `OXI_GPU_REQUIRED=1`.
3. Advisory step (`continue-on-error`): goldens again with `OXI_GPU_GOLDEN_ANY_LLVM=1`, so a Mesa/LLVM drift shows the real pixel diff without failing the job.
4. `upload-artifact` of `oxiterm-gpu/tests/golden/*.new.png` (`if: always()`, ignore if none).

No wasm-pack step: `cargo tree` shows `oxiterm-gpu` does not depend on `oxiterm-server`/`oxiterm-web`. `oxiterm-fb` tests already run in `build` (workspace), so they are not repeated.

## Skip vs require

`oxiterm-gpu/tests/common/mod.rs` is the single place. Every skip site (the three `shared_ctx()`, the three standalone tests in `primitives.rs`, `bad_actor_52.rs::ctx()`) goes through `ctx_or_skip`:

- `GpuError::NoAdapter`: print `SKIP`, pass (default); panic with a clear message if `OXI_GPU_REQUIRED` is `1`/`true`.
- any other error: always a panic. (`bad_actor_52` used to skip on every error; fixed.)

Not changed: `src/sink.rs` lib unit test `test_ctx()` still skips on NoAdapter. Harmless in CI: the integration binaries fail under REQUIRED.

## Goldens across llvmpipe versions

Blessed on `llvmpipe (LLVM 20.1.2, 256 bits)`, `driver_info = "Mesa 25.2.8-0ubuntu0.24.04.5 (LLVM 20.1.2)"` (Ubuntu 24.04). The local packages carry the `0ubuntu0.24.04.x` suffix (noble-updates), and `ubuntu-latest` is 24.04 as of this writing (GitHub migrates it to the next LTS some months after release; if that happens Mesa/LLVM change and the goldens flip to the loud skip by design; pin `runs-on: ubuntu-24.04` for a stable golden identity), so after `apt-get update` the runner most likely installs the same Mesa 25.2.x / LLVM 20. This is a hypothesis; the first CI run confirms it (dpkg line plus the `golden tests adapter:` line in the log).

Decision, implemented as the pure function `golden_gate` in `tests/golden.rs` (unit-tested there):

- not llvmpipe: Skip by default, Fail when REQUIRED.
- llvmpipe with LLVM major 20 and Mesa 25.2 (patch bumps ignored): run the PNG comparison, tolerance unchanged (2 per channel, 0.25% of pixels).
- any other llvmpipe build: SKIP with a loud note (also appended to `$GITHUB_STEP_SUMMARY`), even when REQUIRED, because pixel output of another LLVM is not what was blessed. The advisory CI step still compares and uploads `.new.png`, so the lead can re-bless on the runner's version (or widen the gate) after looking.
- The CPU-vs-GPU differential tests (`cpu_compare`, `primitives`, `bad_actor_52`) are untouched, driver independent and always hard.

## Local verification

Software driver only; `WGPU_BACKEND=vulkan`; release.

| Case | Result |
|---|---|
| `VK_ICD_FILENAMES=lvp_icd.json OXI_GPU_REQUIRED=1`, whole crate | all pass: lib 5, bad_actor_52 21 (5 ignored), cpu_compare 7, golden 11 (6 goldens compared on llvmpipe LLVM 20.1.2), primitives 22, doc 1 |
| `VK_ICD_FILENAMES=/nonexistent`, REQUIRED unset | all binaries pass with SKIP notes (as before) |
| `VK_ICD_FILENAMES=/nonexistent OXI_GPU_REQUIRED=1` | hard failures: bad_actor_52 21, cpu_compare 6, golden 6 (gate-independent unit tests pass), primitives 22 fail with "OXI_GPU_REQUIRED is set, so skipping is not allowed..." |
| non-llvmpipe adapter | not runnable locally (real GPU off limits); covered by `gate_tests` unit tests |

Not verifiable here: `apt-get install`, the runner's actual Mesa/LLVM and adapter string, whether goldens run or skip there, wall time, artifact upload. `ci.yml` parses with PyYAML (`actionlint`/`yamllint` not installed).

## Things to expect on the first run

- Timing-sensitive tests that now run on a 4-core runner (local box: 4 cores too): `bad_actor_52` `ratio < 8.0` (4x glyphs vs 1x time, around line 672) and the RSS bounds (`< 20/64/16 MB`, lines 472-520). Not loosened (outside scope); if one is red on the first run it is likely a runner effect, not this change.
- The artifact upload uses `if: always()` (superset of "on failure") because the advisory golden step is `continue-on-error`.
- `release` has `needs: [build, audit]` and does not include `gpu`; a tag can ship with GPU tests red. Lead's call.
- Look for in the log: the `dpkg -l` line, `golden tests adapter: name=... driver_info=...`, a "goldens SKIPPED" line in the step summary, the advisory step status, job wall time. Goldens are not LFS (no `.gitattributes`), `*.new.png` is gitignored.
