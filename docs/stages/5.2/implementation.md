# Stage 5.2 — Headless GPU Sink (`oxiterm-gpu`)

## Status
- **Phase:** Implementation complete (pending lead review & golden blessing via `OXI_BLESS_GPU=1`)
- **Goal:** Headless `wgpu` GPU sink in `oxiterm-gpu` implementing OxiUI Phase 5.2 (DisplayList → RGBA8/PNG, shared `GpuContext`, SDF fills/borders, `etagere` glyph atlas matching `oxiterm-fb`, `CellGrid`, scissor clips, golden & CPU-comparison tests, example CLI binary, crate README with frame timings).

## Key Decisions
1. **Shared `GpuContext`** (`oxiterm-gpu/src/context.rs`): Owns `wgpu::Instance`, `wgpu::Adapter`, `wgpu::Device`, `wgpu::Queue`, and `wgpu::AdapterInfo` separately from `GpuSink`. Created headless via `GpuContext::new_headless()` (`InstanceDescriptor::new_without_display_handle_from_env()`, tries default adapter then `force_fallback_adapter: true` for Mesa lavapipe, requests `Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())` with `Features::empty()`). Returns `Err(GpuError::NoAdapter(_))` cleanly when no adapter exists. `GpuSink` accepts `Arc<GpuContext>` so future OxiScript GPU kernels (8.3b), 3D (9.3), and window swapchains (5.3) share the same device.
2. **GLES 3.0 / WebGL2-compatible shaders only** (`src/shaders/sdf.wgsl`, `src/shaders/glyph.wgsl`): Vertex + fragment shaders only; no compute shaders, no storage buffers, 3 `Float32x4` instance attributes per pipeline, 16-byte viewport uniform, `@builtin(vertex_index)` quad expansion.
3. **sRGB-space `Rgba8Unorm` target**: Renders into `TextureFormat::Rgba8Unorm` (straight sRGB bytes, `SrcAlpha / OneMinusSrcAlpha`) so alpha blending of glyphs and SDF edges matches `oxiterm-fb` and Canvas2D.
4. **Font & glyph rasterization parity with `oxiterm-fb`** (`src/atlas.rs`): Uses `oxiterm-renderer`'s bundled `NotoSansMono-Regular.ttf` / `NotoSans-Regular.ttf` (`features = ["text-shaping"]`, `default-features = false`), `swash` 0.2.10 outline rasterizer (quarter-pixel subpixel phases, synthetic bold/italic), and `Shaper` for multi-char clusters, packed with `etagere::AtlasAllocator` into an `R8Unorm` texture (1024×1024 initial, grows to 2048×2048, flushes active pass + resets cleanly on overflow, 1 px zero padding, `FilterMode::Nearest`).
5. **Draw-call batching**: Consecutive instances of the same pipeline under the same hardware scissor rect merge into one instanced `draw(0..6, range)` call; non-overlapping non-italic monospace `Text` runs batch all cell backgrounds before glyph quads (cutting draw batches by 2–3×).
6. **Zero changes to `oxiterm-renderer` or `oxiterm-proto`**: All needed `FontBundle`, `Shaper`, `DisplayList`, and `UnicodeWidthCache` APIs were already public. Zero new external dependencies beyond crates already locked in `Cargo.lock` (`swash = "0.2.10"` and workspace `oxiterm-proto`; `oxiterm-fb` as dev-dependency only). Zero `unsafe` code anywhere in `oxiterm-gpu`.
7. **Clippy note**: `cargo clippy -p oxiterm-gpu --all-targets --no-deps -- -D warnings` passes with 0 warnings under workspace `-W clippy::pedantic`. Without `--no-deps`, clippy fails in unmodified upstream crate `oxiterm-proto/src/pathsafe.rs:8,12` (`clippy::manual_let_else` under rustc 1.98.0), which is outside Stage 5.2 scope.

## What Works
- Headless `GpuContext` + `GpuSink::render`, `GpuSink::render_cells`, `GpuSink::render_into_view`
- `DisplayItem::Fill` (square + rounded SDF with clamped radius)
- `DisplayItem::Border` (single, rounded, double, CSS `radius > 0` ring cutout, ASCII / 1-cell glyph fallback)
- `DisplayItem::Text` (monospace default runs + proportional shaped runs with clusters, ligatures, combining marks, fallback faces, bold/italic/underline, clip rect)
- `DisplayItem::CellGrid` (wide chars, `skip` trailers)
- `DisplayItem::PushClip` / `DisplayItem::PopClip` (nested clip stack → hardware scissor rects, fail-closed on non-finite clip)
- Untrusted DisplayList sanitization (NaN/Inf/huge coordinates/huge glyph sizes/overflow guards)
- RGBA8 readback (`COPY_BYTES_PER_ROW_ALIGNMENT = 256` padding strip) + PNG encode/decode (`FrameImage`)
- Example CLI: `cargo run -p oxiterm-gpu --example render -- <page.thtml> <out.png>` with `--bench` timing

## Measured Numbers
### CPU (`oxiterm-fb`) vs GPU (`oxiterm-gpu`) differential comparison (`CHANNEL_TOLERANCE = 4`, limit `<= 0.5%`)
| Case | Resolution | Differing pixels | Differing % | Max ch diff |
|---|---|---:|---:|---:|
| `examples/index.thtml` | 768×480 @ 1.0x | 144 / 368,640 | 0.0391% | 28 |
| `examples/features.thtml` | 768×480 @ 1.0x | 240 / 368,640 | 0.0651% | 30 |
| `examples/demos/counter.thtml` | 768×480 @ 1.0x | 67 / 368,640 | 0.0182% | 29 |
| `examples/demos/showcase.thtml` | 768×480 @ 1.0x | 196 / 368,640 | 0.0532% | 29 |
| `examples/demos/styles.thtml` | 768×480 @ 1.0x | 96 / 368,640 | 0.0260% | 25 |
| `examples/demos/input.thtml` | 768×480 @ 1.0x | 127 / 368,640 | 0.0345% | 40 |
| `counter@1.5x` | 1152×720 @ 1.5x | 105 / 829,440 | 0.0127% | 47 |
| `counter@2.0x` | 1536×960 @ 2.0x | 151 / 1,474,560 | 0.0102% | 50 |
| `synthetic_all_primitives` | 442×400 @ 1.0x | 283 / 176,800 | 0.1601% | 113 |

### Frame timings on Mesa `llvmpipe (LLVM 20.1.2, 256 bits)` (debug build, 768×480, incl. readback)
- `examples/demos/counter.thtml`: 45 batches, cold 136.37 ms, warm median **28.60 ms** (min 18.89 ms)
- `examples/index.thtml`: 46 batches, cold 210.35 ms, warm median **25.69 ms** (min 22.94 ms)
- `examples/demos/showcase.thtml`: 120 batches, cold 356.83 ms, warm median **45.47 ms** (min 28.23 ms)

## Milestones
- [x] Read specification, `display_list.rs`, `cell_sink.rs`, `oxiterm-fb`, `vulkan-acceleration.md`
- [x] Implement `GpuContext`, `FrameImage`, `GlyphAtlas`, WGSL shaders (`sdf.wgsl`, `glyph.wgsl`), `GpuSink`, `page`
- [x] Add `examples/render.rs` CLI and `oxiterm-gpu/README.md` with frame timings
- [x] Add unit tests (`--lib`), primitive & robustness tests (`primitives`), CPU-vs-GPU differential tests (`cpu_compare`), and tolerance-based PNG golden tests (`golden` with `OXI_BLESS_GPU=1` workflow)
- [x] Verify `cargo clippy -p oxiterm-gpu --all-targets --no-deps -- -D warnings`, `oxiterm-gpu` tests, and `cargo test -p oxiterm-renderer`
- [ ] Lead inspects `oxiterm-gpu/tests/golden/*.new.png` and blesses with `OXI_BLESS_GPU=1 cargo test -p oxiterm-gpu --test golden`

## Verification Commands
```bash
cargo clippy -p oxiterm-gpu --all-targets --no-deps -- -D warnings
cargo test -p oxiterm-gpu --lib --test primitives --test cpu_compare
cargo test -p oxiterm-gpu --test golden
# Lead blessing step (after inspecting *.new.png):
# OXI_BLESS_GPU=1 cargo test -p oxiterm-gpu --test golden
cargo test -p oxiterm-renderer
cargo run -p oxiterm-gpu --example render -- examples/demos/counter.thtml target/gpu-outputs/counter.png --examples examples --bench 20
```
