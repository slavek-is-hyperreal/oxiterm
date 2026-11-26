# `oxiterm-gpu`

Headless `wgpu` GPU sink for OxiUI (Phase 5, Stage 5.2).

Renders an OxiUI `DisplayList` into an RGBA8 `FrameImage` (and PNG) without a window, using a custom GLES 3.0 / WebGL2-level pipeline (vertex + fragment shaders only, no compute shaders):

- **Shared `GpuContext`** (`src/context.rs`): owns `wgpu::Instance`, `wgpu::Adapter`, `wgpu::Device`, and `wgpu::Queue` separately from `GpuSink` (`Arc<GpuContext>`), following OxiUI's *"one device, several users"* rule so future OxiScript GPU kernels (Stage 8.3b), 3D rendering (Stage 9.3), and window swapchains (Stage 5.3) share one device without CPU round-trips.
- **Instanced SDF pipeline** (`src/shaders/sdf.wgsl`): solid fills, rounded-rect fills (`border-radius`), rounded-border background ring cutouts, and single / rounded / double border strokes with 1 px analytic edge anti-aliasing and crisp miter joins for sharp corners.
- **Instanced glyph-atlas pipeline** (`src/shaders/glyph.wgsl`, `src/atlas.rs`): CPU-rasterized alpha masks (`swash`, quarter-pixel horizontal subpixel phases, synthetic bold/italic, `oxiterm-renderer` `Shaper` for clusters/ligatures) packed with `etagere` into an `R8Unorm` texture (grows up to 2048×2048 and flushes + resets cleanly on overflow).
- **Hardware scissor clips**: `PushClip` / `PopClip` and `DisplayItem::Text { clip, .. }` intersect cleanly on a clip stack and drive `RenderPass::set_scissor_rect`.
- **`CellGrid` support**: renders cell-grid overlays and legacy placeholders respecting `skip` trailers and wide Unicode characters.
- **`Path` items**: deferred to Stage 5.6 (ignored in 5.2).

## Example CLI

Render any `.thtml` page to PNG (and optionally benchmark warm frames):

```bash
cargo run -p oxiterm-gpu --example render -- examples/demos/counter.thtml target/gpu-outputs/counter.png --examples examples --bench 20
```

Flags: `--cols N` (default 80), `--rows N` (default 24), `--scale S` (default 1.0), `--bench N` (warm iterations), `--examples DIR`.

## Frame Timing (Stage 5.2)

Measured on Mesa `llvmpipe (LLVM 20.1.2, 256 bits)` software Vulkan (`Vulkan/Cpu`), 80×24 cells at scale 1.0 (768×480 px), **including full GPU-to-CPU staging texture readback**:

### Release build (`--release`)

| Page | DisplayList items | Instances (SDF / glyphs) | Draw batches | Cold frame | Warm median (20 iters) |
|---|---:|---:|---:|---:|---:|
| `examples/demos/counter.thtml` | 53 | 144 / 204 | 45 | 39.1 ms | **8.9 ms/frame** (min 8.2 ms) |
| `examples/index.thtml` | 75 | 231 / 345 | 40 | 33.5 ms | **9.6 ms/frame** (min 9.0 ms) |
| `examples/demos/showcase.thtml` | 143 | 312 / 387 | 120 | 49.2 ms | **13.9 ms/frame** (min 11.7 ms) |

### Debug build (`dev` profile)

| Page | DisplayList items | Instances (SDF / glyphs) | Draw batches | Cold frame | Warm median (20 iters) |
|---|---:|---:|---:|---:|---:|
| `examples/demos/counter.thtml` | 53 | 144 / 204 | 45 | 136.4 ms | **28.6 ms/frame** (min 18.9 ms) |
| `examples/index.thtml` | 75 | 231 / 345 | 46 | 210.4 ms | **25.7 ms/frame** (min 22.9 ms) |
| `examples/demos/showcase.thtml` | 143 | 312 / 387 | 120 | 356.8 ms | **45.5 ms/frame** (min 28.2 ms) |

> Note: These timings are on software Vulkan (`llvmpipe`) and include synchronous `copy_texture_to_buffer` + `map_async` readback every frame; hardware GPUs and window presentation without readback (Stage 5.3) will be dramatically faster.

## Testing & Golden Workflow

```bash
# Unit, primitive, and CPU-vs-GPU differential tests (against oxiterm-fb)
cargo test -p oxiterm-gpu --lib --test primitives --test cpu_compare

# Tolerance-based PNG golden tests (6 example pages)
cargo test -p oxiterm-gpu --test golden

# Bless goldens after visual review (lead only)
OXI_BLESS_GPU=1 cargo test -p oxiterm-gpu --test golden
```

- When a golden PNG is missing or differs beyond tolerance (`CHANNEL_TOLERANCE = 2`, `MAX_DIFF_PERCENT = 0.25%`), the test writes `<name>.new.png` next to the golden path and fails with instructions.
- When no `wgpu` adapter is available, GPU tests skip with a printed note (developer machines without a GPU). With `OXI_GPU_REQUIRED=1` a missing adapter is a hard failure; the CI `gpu` job sets it and runs everything on Mesa lavapipe. Goldens compare only on llvmpipe LLVM 20 / Mesa 25.2 (the blessed build) and skip with a note on another llvmpipe build (`OXI_GPU_GOLDEN_ANY_LLVM=1` forces the comparison). Details: `docs/stages/5.2/ci.md`.

Local run exactly like CI (software driver only):

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan OXI_GPU_REQUIRED=1 \
  cargo test --release -p oxiterm-gpu -- --test-threads=1
```
