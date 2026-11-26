# 🚀 GPU (Vulkan) Acceleration for OxiUI: Findings and Recommendation

> **Stale facts (2026-11-26):** section 2 ("reset_nodes every frame", "PushClip not emitted", "wire v1/v2") describes the code of 2026-11-22 and was overtaken by stages 5.1b, 5.2 and 6.x; stage 5.2 (headless `GpuSink`, merged `05c22ea`) implements decision D3 (revised) from this note, and the remaining GPU work is in [../roadmap/oxiui.md](../roadmap/oxiui.md) (5.3 to 5.7, U.WebGPU).
>
> **Note (2026-11-26, plan-cleanup):** this document predates the roadmaps. Where it cites `oxi-ui-plan-history.md` with line numbers or section names, it means the old OxiUI plan file as it was at commit `3017fd5` (its Phase 1-7 text is kept in `docs/history/oxi-ui-plan-history.md`; its Phase 8 text, principles `P-n` and decisions `D-n` are now in [../roadmap/oxiscript.md](../roadmap/oxiscript.md), the OxiUI half in [../roadmap/oxiui.md](../roadmap/oxiui.md)).

This document is a research note on how OxiUI should add GPU acceleration, what existing Rust and terminal-emulator solutions do, and what the literature says. It feeds decision **D3** and **Phase 5** of [oxi-ui-plan-history.md](../history/oxi-ui-plan-history.md).

> **Status:** Research note, 2026-11-22. No code was changed. All "status" claims are dated; anything not verified against a primary source is marked **[unverified]** or **[inferred]**. Benchmark numbers are quoted with their source or listed as unknown. Nothing here was measured on OxiTerm itself.
>
> **Related docs:** [oxi-ui-plan-history.md](../history/oxi-ui-plan-history.md) (plan, DisplayList, D3), [iot-screens.md](../iot-screens.md) §4 (3D tiers), [retro-desktop-environment.md](../retro-desktop-environment.md) and [embedded-iot-platform.md](../embedded-iot-platform.md) (low-end targets).

---

## 1. Executive Summary and Recommendation

**Recommendation in one paragraph.** Go through **wgpu**, not raw Vulkan (`ash`). Write the 2D pipeline for the DisplayList **yourselves**, in the way Zed, Warp and the GPU terminals do it: instanced quads with signed-distance-field (SDF) rounded rectangles and borders, plus a CPU-rasterized **glyph atlas** drawn as instanced textured quads. Only `Path` items need a real vector rasterizer; use **lyon** tessellation or **`vello_gpu`** for those. Keep **Canvas2D** as the browser baseline and add a **WebGL2** glyph-atlas painter later. For old hardware, the fallback is **not** a different GPU API but a **CPU sink** (cell/bitmap-font blit to DRM dumb buffers or `/dev/fb0`, already planned in the embedded docs).

**The five-line version**

1. **wgpu over ash.** wgpu covers Vulkan, Metal, DX12, GLES 3.0+ and WebGPU/WebGL2 with one API. Zed moved its Linux renderer from Blade to wgpu in February 2026; WezTerm ships a wgpu-based `WebGpu` front end. Raw Vulkan buys nothing OxiUI needs and loses macOS, Windows, web and GLES.
2. **Do not adopt Vello's compute renderer, and revise D3.** D3's premise ("Vello needs compute shaders, so old GPUs need a separate renderer") is out of date: `vello_gpu` (formerly `vello_hybrid`) has a wgpu backend that does not require compute shaders and a native WebGL2 backend. It is still pre-1.0 with panics on some features. Use it, at most, for `Path` and as a later "full vector" option, not as the core.
3. **The DisplayList is already the right shape for a quad/atlas renderer.** `Fill`/`Border` (with `radius`) become SDF instanced quads; `Text` and `CellGrid` become instanced atlas quads; `PushClip`/`PopClip` become scissor rects. Only `Path` is not a quad.
4. **The CPU side will dominate frame time before the GPU does.** `LayoutEngine::compute` discards the whole Taffy tree and its cache every frame (`reset_nodes()`), and GUI sessions additionally rasterize every frame through `CellSink` and zlib-compress the full DisplayList. Instrument first, then fix layout reuse, then build the GPU sink.
5. **First milestone: a headless wgpu `GpuSink` with PNG goldens on lavapipe.** Render the existing `examples/**/*.thtml` DisplayLists (`Fill`, `Border`, `Text`, `CellGrid`) to a texture, read it back, compare to committed PNGs in CI using Mesa's software Vulkan. Add the `winit` window afterwards.

**Licensing (OxiTerm is MIT OR Apache-2.0, OEM model):** the whole recommended stack (wgpu, winit, lyon, cosmic-text/swash, etagere, optional `vello_gpu`) is permissive; Slint (embedded needs GPLv3 or a paid licence) and kitty (GPL) are excluded; Skia is licence-compatible but a heavy C++ dependency. See §11.1. Licences marked *(memory)* there must be confirmed with `cargo deny`.

Old boards below the wgpu floor (Raspberry Pi Zero through Pi 3, GLES 2.0 class) stay on the CPU/framebuffer path. That is consistent with the existing embedded docs.

---

## 2. How OxiUI Produces Frames Today

Read from the code on 2026-11-22.

```
THTML + TCSS ─► parser ─► DOM arena + cascade
                               │
                    LayoutEngine::compute()          (Taffy 0.6.x, f32)
                    └─ reset_nodes(): new Taffy tree EVERY call
                               │
                    build_display_list()  ──►  DisplayList (flat, paint-ordered, f32 cell units)
                               │
          ┌────────────────────┼─────────────────────────────┐
          ▼                    ▼                             ▼
     CellSink              wire v1/v2                    (future) GpuSink
  DisplayList → CellBuffer  zlib(DisplayList)            winit + wgpu
  → DiffEngine → ANSI/WS    → oxiterm-web/paint.rs plan()
                              → Canvas2D PaintOps
```

Facts that matter for GPU work:

| Fact | Where | Consequence |
|---|---|---|
| The DisplayList is a flat list of `Fill`, `Border`, `Path`, `Text`, `CellGrid`, `Legacy`, `PushClip`, `PopClip`. Coordinates are `f32` cell units; `radius` and stroke widths are in cells along x. | `oxiterm-renderer/src/render/display_list.rs` | A GPU sink multiplies by the cell size in pixels, exactly as `paint.rs` does with `Metrics { cw, ch, dpr }`. |
| `Text` carries already-positioned glyphs (`ch`, `x`, `y`, `advance`), one `Glyph` per character, with whole-cell offsets inside a run (monospace until Phase 6). | `display_list.rs` | Text is a cell-grid-like instance list: one atlas quad per glyph. No shaping at paint time. |
| `CellGrid` is used for `img`/`video`/`diagram` (painted by legacy cell code) and for overlays (focus ring, echo, scroll bar). `skip` marks the trailing half of a wide glyph. | `display_list.rs`, `renderer.rs::capture_grid` | Maps directly to the GPU-terminal model (one instance per cell). Images/video as cell grids are a poor fit for a pixel GPU; they should become real image items in a later phase. |
| `PushClip`/`PopClip` are reserved and not emitted yet. | `display_list.rs` | Needs to be emitted before a GPU sink can clip correctly; maps to scissor rects. |
| In GUI sessions `render_frame_mode` still paints the list into a `CellBuffer` through `CellSink` (and runs the legacy painters) on every frame, then the server zlib-compresses the full list. | `renderer.rs::render_frame_mode`, oxi-ui-plan-history.md Phase 3 | A fixed CPU cost per frame that no GPU sink removes. |
| `LayoutEngine::compute` calls `self.reset_nodes()` first, so the Taffy tree is rebuilt every frame. The code comment says this guarantees correctness for `bind-show` toggles. The `doc.dirty_nodes` loop afterwards is therefore redundant work on a fresh tree. | `oxiterm-renderer/src/layout/engine.rs:204-210` | Taffy's per-node cache is thrown away every frame (see §3.4 and §12). |
| The web painter (`paint.rs::plan`) is **pure** (no canvas, no browser): DisplayList + `Metrics` → abstract `PaintOp`s in CSS pixels (border snapping, radius clamping, hidpi line alignment). A thin executor maps ops 1:1 to Canvas2D. Text is one `Text { ch, ... }` op, i.e. one `fillText` per glyph. | `oxiterm-web/src/paint.rs` | This is a reusable pixel-space layer. A native GPU sink could consume the same `PaintOp`s (or share `plan()` via a common crate) so snapping and radius rules do not diverge between web and native. |
| `resvg`/`tiny-skia` are already dependencies of `oxiterm-renderer` (used for SVG diagrams). | `oxiterm-renderer/Cargo.toml` | A CPU vector rasterizer is already in the tree, but retro-desktop-environment.md calls it too heavy for 400 MHz–1 GHz targets. |

### 2.1 Where GPU acceleration can apply

| Area | What the GPU can do | What it cannot fix | Section |
|---|---|---|---|
| (a) Rasterizing the DisplayList | Instanced SDF quads for fills/borders; scissor clips; tessellated or compute-rasterized paths | Nothing about layout or list construction | §5, §6 |
| (b) Text/glyphs | Glyph-atlas instanced quads (cell-grid style); optional SDF/MSDF/Slug for scalable text | Shaping, font fallback, atlas fill (CPU rasterization of each new glyph) | §5, §7 |
| (c) Compositing/damage | Retained textures per layer; redraw only on change; WebRender-style tile cache if it ever matters | Wire cost of full lists (damage matters more for the wire and software sinks) | §7.3, §12 |
| (d) Layout / CPU side | **Not GPU work.** Incremental Taffy reuse; avoiding the redundant `CellBuffer` pass in GUI sessions | This is the likely dominant cost | §3.4, §12 |
| (e) Server-side GPU (OxiScreens) | Headless wgpu rendering of `<scene>` tier 1; hardware JPEG/H.264 encode | The monolithic DisplayList wire path stays CPU/zlib | §9 |
| (f) Browser | Canvas2D now; WebGL2 glyph-atlas painter next; WebGPU later | Browser GPU availability on Linux/Android | §10 |

---

## 3. Rust GPU 2D Stacks

### 3.1 wgpu

- Cross-platform graphics API implementing the WebGPU model. Backend table from its README (checked 2026-11-22): Vulkan first class on Windows/Linux/Android; Metal first class on macOS/iOS; DX12 first class on Windows; **OpenGL/GLES downlevel/best effort: GL 3.3+ on Windows, GL ES 3.0+ on Linux/Android**; WebGL2 downlevel on wasm; WebGPU first class on wasm. MSRV 1.87 for users.
- Release status: v30.0.0 on 2026-07-01, v30.0.1 on 22 Aug, v30.0.2 on 12 Nov (GitHub releases page; the year is inferred from the page, **[partly inferred]**). Major versions arrive every few months with breaking changes, so pin carefully and budget for upgrades.
- The **floor** that matters for OxiUI: Vulkan 1.x, DX12, Metal, or GLES 3.0. Hardware below that (GLES 2.0 only) cannot run wgpu at all.
- Headless use is routine: render to a texture with `RENDER_ATTACHMENT | COPY_SRC`, `copy_texture_to_buffer` (rows padded to 256 bytes), map and read (pattern seen in egui-wgpu's capture code and several crates). Mesa's lavapipe (CPU Vulkan) is used as a software adapter in wgpu's own CI table (wgpu 0.20 docs). **[Source for the CI claim is an older docs page; verify against current wgpu CI.]**
- Cost to measure: wasm binary size of wgpu (known to be significant) **[unquantified here]**, and resident memory with a Vulkan driver loaded versus the 15–25 MB RSS target in oxi-ui-plan-history.md §1. Both are **unknown** and listed in §13.

### 3.2 Vello family (Linebender)

Per the Vello README (2026-11-22) there are now three renderers:

| Crate | What it is | Requirement / status |
|---|---|---|
| `vello_cpu` | CPU, SIMD, multithreaded | "Most mature" of the three. MSRV 1.89 per README. |
| `vello_gpu` (renamed from `vello_hybrid`; docs.rs: vello_gpu 0.4.0, vello_hybrid 0.2.0 still listed) | **Sparse strips**: path flattening, tiling, strip generation on the CPU; fine rasterization and compositing on the GPU. wgpu backend **without compute shaders**; native **WebGL2** backend (`webgl` feature). | "Slightly less mature" than `vello_cpu`. Trails the compute renderer on dynamic, vector-heavy workloads; good on images, gradients, filters. Documented gaps: mask layers, complex filter graphs and some non-isolated blend modes **panic**; some failures panic instead of returning errors; glyph caching is experimental; wgpu backend not fully optimized. Depends on wgpu >= 30. |
| `vello` (under `research/`) | The original **compute-centric** renderer (prefix-sum pipeline, GPU-side everything) | Experimental; needs compute-capable GPUs; no WebGL. |

Implication for D3: the claim "Vello needs compute shaders" applies only to the research renderer. `vello_gpu` removes that blocker, but it is pre-1.0, has panicking gaps, and targets general vector scenes with clips, gradients and blends. OxiUI's frame is mostly rectangles and glyphs, so Vello's generality is paid for without being needed. Text goes through the `glifo` crate, shared by both sparse-strips renderers (per README).

Design lineage and rationale: Levien's "Potato" hybrid CPU/GPU design and the 2025 RustWeek talk "Faster, easier 2D vector rendering"; ETH Zürich master's thesis on sparse strips (2025). I did **not** retrieve Levien's own rationale text in this session **[unverified in his words]**; the Linebender newsletter summary says the hybrid mode "works well on downlevel GPUs without strong support for compute shaders" and is more memory efficient.

Slint documents its Vello renderer as experimental, requiring a GPU with compute-shader support, never selected automatically, and not suited to low-end hardware (Slint backends page).

### 3.3 Other stacks

| Stack | How it draws | Notes for OxiUI |
|---|---|---|
| **tiny-skia** (CPU) | Rust port of a Skia subset; CPU only. README: on x86-64 roughly 20–100% slower than Skia, on ARM roughly 100–300% slower; **no text rendering**; GPU out of scope. | Good deterministic software reference and already a dependency (via `resvg`). Too heavy for 400 MHz-class retro targets per the existing doc. |
| **lyon** | Path tessellation to vertex/index buffers; "designed for GPU APIs"; no renderer and no built-in anti-aliasing (use MSAA or a fringe) | Right tool for `Path` fills/strokes if `vello_gpu` is not adopted. Small. |
| **glyphon** | wgpu text renderer: cosmic-text for shaping/rasterization, etagere atlas, LRU eviction, renders in an existing pass | Ready-made text path; but it targets proportional text, and OxiUI's monospace cell model may be simpler to do directly (§7). |
| **cosmic-text / swash / etagere** | Shaping, glyph rasterization, atlas packing | Building blocks named in oxi-ui-plan-history.md Phase 5/6. Not individually verified in this session **[not verified this session]**. |
| **femtovg** | OpenGL canvas (NanoVG-style); Slint offers it as a renderer | Per Slint docs: requires OpenGL; text/path quality "sometimes suboptimal". Not individually verified **[beyond the Slint docs]**. |
| **skia-safe** (Skia bindings) | Full Skia on GL/Vulkan/Metal/D3D | Per Slint: needs a GPU API and has a large disk footprint. Conflicts with the "lightweight" goal. **[Not otherwise verified this session.]** |
| **iced** | Two renderers: `iced_wgpu` and `iced_tiny_skia`; tries wgpu first and falls back to tiny-skia (secondary guide, iced 0.14) | The same "GPU first, software second" pattern OxiUI wants. |
| **egui / epaint** | CPU tessellation into textured triangle meshes plus a font atlas, drawn by `egui_wgpu` | Not verified this session **[from prior knowledge, not a fetched source]**. |
| **Slint** | Software (no GPU, `no_std`, microcontrollers), FemtoVG, Skia, experimental Vello; LinuxKMS backend needs no windowing system | Closest architecture to OxiUI's "GPU or software sink" split. |
| **Zed GPUI** | Per Zed's blog: a few primitives (rectangles, shadows, glyphs, icons, images), each with its own shader. Rounded rects use an **SDF in the fragment shader**; glyphs rasterized on the CPU as alpha-only and cached in a GPU atlas (up to 16 subpixel variants, packed with etagere); instanced draw calls. Linux backend moved from Blade to **wgpu** in Feb 2026 (commit "gpui: Remove blade, reimplement linux renderer with wgpu (#46758)"); one report cites CPU draw time falling from about 301 µs to about 236 µs median (secondary reports, differing slightly). Zed has reportedly paused independent GPUI development in 2026. | The reference design for OxiUI's quad/atlas renderer. |
| **Warp** | Per Warp's "How Warp Works" post: only rectangles, images and glyphs are needed, with a shader per primitive; average screen redraw 1.9 ms over one week (Warp's own number, Metal, ~2021–22). A third-party April 2026 write-up claims Warp now uses wgpu **[unverified on Warp's site]**. | Same conclusion as Zed. |

### 3.4 What this means for the CPU side (preview of §12)

Every stack above assumes the UI layer hands the renderer a cheap, mostly unchanged scene. OxiUI currently hands it a freshly rebuilt one.

---

## 4. Terminal Emulators with GPU Rendering

OxiUI's TUI heritage is a cell grid, and `CellGrid` plus `Text` runs are cell-structured, so this is the most directly applicable prior art.

| Terminal | GPU approach (what the sources show) | Notes |
|---|---|---|
| **Alacritty** | OpenGL. Two text renderers: a GLSL 3 path (OpenGL 3.3) and a GLES2 path that exists because Alacritty previously needed GL 3.3+; GLES2 is used only when necessary, is slower, and splits subpixel text across three passes (source: renderer/text modules of alacritty 0.17.0 on docs.rs). Glyphs are packed in an RGBA atlas texture row by row; a new atlas is created when full. Damage tracking: per-line column ranges merged into a frame damage tracker before GL calls (a secondary blog overview; **verify in `display/damage.rs`**). | The clearest precedent for **GL 3.3 plus a GLES2 fallback** on old hardware. |
| **Kitty** | Keeps a cache of each rendered glyph in video RAM; screen updates typically send a few bytes to the GPU (kitty docs). Throughput table (**MB/s of terminal output parsed, not latency**; Linux/X11, Ryzen 7 PRO 5850U, kitty 0.33): kitty average 134.55, alacritty 0.13.1 54.05, gnome-terminal 61.83. The page notes rendering is suppressed in that benchmark. Latency: the docs cite third-party typometer and hardware measurements as best in class; I did not retrieve those measurements. | Kitty's own numbers; treat with care. |
| **WezTerm** | `front_end` options: `OpenGL` (default), `WebGpu` (since 20221119), `Software`; auto-selects `Software` in a Windows Remote Desktop session (WezTerm docs). | Shows the value of keeping a software sink alongside the GPU ones. |
| **Ghostty** | Shared core ("libghostty"); macOS app Swift/AppKit, Linux app Zig/GTK4 (Ghostty docs, which say nothing about the GPU API). From **mirrors and a Rust port (sugarloaf) only**: a Metal renderer with a flat per-cell background buffer, per-row text-cell lists, a vertex shader converting grid coordinates to pixels, and several in-flight buffer slots so the CPU can prepare the next frame while the GPU draws. | **[Ghostty GPU details are second-hand.]** |
| **Warp** | See §3.3: rectangles/glyphs/images, Metal at first, avg 1.9 ms redraw (Warp's own figure). | |
| **beamterm** (Rust, WebGL2) | A WebGL2 terminal renderer with a cell grid and atlas | Prior art for a browser-side cell/glyph painter in Rust (found in search results; not examined in depth **[not verified]**). |

**Latency results.** I could not retrieve primary numbers for terminal latency (Dan Luu's term-latency page did not load usable results; secondary posts disagree on kitty versus Alacritty ranking depending on platform and configuration). **No latency figure is quoted here.** For OxiUI the relevant point is qualitative: GPU terminals keep a persistent glyph cache and a per-cell instance buffer, so a keystroke costs a few bytes of buffer update, not a repaint.

**Common design across all of them:** (1) glyph atlas filled lazily on the CPU; (2) one instance per cell (background color, foreground color, atlas coordinates); (3) draw in two instanced passes (backgrounds, then glyphs); (4) update only changed rows. OxiUI's `CellGrid`, `Text` and `Fill` items map onto this with no new concepts.

---

## 5. Mapping the DisplayList onto a GPU Pipeline

Proposed `GpuSink` pipelines (all in wgpu, WGSL):

| DisplayItem | GPU representation | Notes |
|---|---|---|
| `Fill { rect, radius, bg }` | One instance of an **SDF rounded-rect quad** (rect, radius, color). Fragment shader computes signed distance, with a ~1 px analytic edge fade. | Same as Zed's rectangle shader. `radius` is in cells along x; multiply by cell width and clamp to half the smaller side (as `paint.rs` already does). |
| `Border { rect, radius, chars, fg, bg }` | SDF ring (outer minus inner distance) with stroke width from the border set; `chars` selects single/double/heavy/rounded line style. Double borders need two rings (as `paint.rs` draws two paths). | `paint.rs` snaps lines to device pixels at hidpi; port that rule (or share `plan()`). ASCII and 1-cell borders fall back to glyphs in `paint.rs`. |
| `Text { origin, glyphs, clip, fg, mods }` | One instanced **atlas quad per glyph**: position = origin + glyph offset times cell size; atlas rect from a `(char, bold, italic, size)` key; tint = `fg`. Underline is an extra thin rect. `clip` becomes a scissor or a per-instance clip rect. | Monospace until Phase 6, so glyph positions are exact cell multiples. Wide glyphs: `advance` 2. |
| `CellGrid { rect, cells }` | One instance per non-`skip` cell: bg quad plus glyph quad. | Exactly the terminal model. |
| `Path { cmds, fill, stroke }` | **lyon** tessellation to triangles (fill + stroke), or `vello_gpu` scene. Anti-aliasing via MSAA or a fringe. | The only item that is not a quad. Cache tessellations by node/path hash; icons are the main users. |
| `PushClip`/`PopClip` | Scissor rect stack (axis-aligned). Rounded clips need a stencil or SDF mask (`paint.rs` clips a ring background to the rounded silhouette, so this case exists). | The builder must start emitting these (today they are reserved). |
| `Legacy` / image / video | Not needed in GUI mode: `render_frame` already replaces `Legacy` with `CellGrid`. Real image/video items (textures, external video frames) are a later step. | `vello_gpu` has a `wgpu_external_texture` feature for video frames (docs). |

Required small tables and rules:

- **AnsiColor → RGBA.** `paint.rs` uses `get_color_str` for CSS strings; a GPU sink needs the same mapping to linear or sRGB `[f32; 4]` (palette 0–15, 6×6×6 cube, 24 grays, truecolor). Share one function.
- **Colorspace:** decide once whether blending is done in sRGB (matches Canvas2D and terminals) or linear (more correct for antialiased text). wgpu v30 added a surface color-space setting.
- **Draw order:** the list is strictly paint-ordered and later items overwrite. A simple sink batches consecutive items of the same kind into one instanced draw call and breaks batches on kind change or clip change. Typical UI frames will produce tens of draw calls, not thousands.

Draw-call and instance counts will be small (a few thousand instances for a dense screen). **A GPU is not needed to draw this fast; it is needed to draw it at high DPI, with animation, without a CPU raster pass.** That framing matters for §12.

---

## 6. GPU Vector Graphics: Literature and Algorithms

These matter for `Path`, scalable text and "full vector" UIs, not for the rectangle/glyph core.

| Work | Idea | Status / notes for OxiUI |
|---|---|---|
| **Loop and Blinn, "Resolution Independent Curve Rendering using Programmable Graphics Hardware" (2005)** | Render quadratic Bézier fills by triangles plus a per-pixel implicit test in the fragment shader | The patent reportedly expired on 2026-03-25 (secondary report, via Hackaday coverage of Slug) **[verify]**. Simple, but overlap and anti-aliasing handling is awkward. |
| **Kilgard and Bolz, "GPU-accelerated path rendering" (SIGGRAPH Asia 2012)** and NV_path_rendering | Stencil-then-cover with vendor extension | NVIDIA-specific in practice; not portable to wgpu. |
| **Ganacim, Lima, de Figueiredo, Nehab, "Massively-parallel vector graphics" (ACM TOG 33(6), 2014)** | Per-pixel parallel rendering using an adaptive "shortcut tree" acceleration structure | Source code was published by the authors. Research baseline; not a drop-in for wgpu. |
| **Pathfinder (Patrick Walton / Servo)** | GPU font/vector rasterizer on GL 3.0+/GLES 3.0+/WebGL2/Metal | README says "under heavy development"; the repository index shows last push in March 2025 **[a snapshot; may be stale]**. Not recommended as a dependency. |
| **Slug (Eric Lengyel)** | Render glyph outlines directly from Bézier data in the fragment shader; per-band curve lists in a texture; vertex-shader dilation; **no glyph atlas**. Patent dedicated to the public domain on **2026-03-17** (Lengyel's post "A Decade of Slug"); reference shaders are MIT-licensed on GitHub. The commercial library remains proprietary. | A strong candidate for **resolution-independent text in Phase 6** (zoom, 3D-projected labels in iot-screens.md §4.2). Not needed for monospace at fixed size. |
| **Valve, "Improved Alpha-Tested Magnification for Vector Textures and Special Effects" (SIGGRAPH 2007, Chris Green)** | Single-channel SDF in a texture; alpha test at 0.5 for sharp edges under magnification | Cheap scalable text and icons; rounds sharp corners. |
| **Chlumský, multi-channel SDF (MSDF); thesis CTU 2015; Chlumský, Sloup, Šimeček, CGF 2018, "Improved Corners with Multi-Channel Signed Distance Fields"** | Three-channel distance fields that preserve corners; `msdfgen` | Good for an icon/glyph atlas that must scale. A paper's intro notes a working multi-channel construction had not been published until November 2014 **[as summarized by search]**. |
| **Vello compute pipeline (Levien)** | Flatten, bin, prefix-sum-based tile assignment and rasterization in compute shaders | Experimental; needs compute; see §3.2. |
| **Sparse strips (Levien, 2024 sketch; ETH thesis 2025)** | CPU preprocessing of paths into strips, GPU/SIMD fine rasterization | Basis for `vello_cpu` and `vello_gpu`. |
| **Skia Spinel/Forge (Graphite/Vulkan compute)** | Compute-based path renderers inside Skia | **Not researched in this session [not verified].** |

**Takeaway.** For OxiUI the literature says: SDF is the right tool for rounded rectangles and borders (and for cheap, scalable icons); an atlas is the right tool for fixed-size monospace glyphs; Slug-class or MSDF only becomes relevant with proportional/scalable text (Phase 6) and perspective text (3D labels).

---

## 7. Text and Damage

### 7.1 Glyph atlas (recommended for Phases 5–6)

- CPU-rasterize each `(char, style, pixel size, subpixel bucket)` once with `swash`/`cosmic-text`/`fontdue`, store alpha only, pack with `etagere`, tint at draw time (Zed's approach; Alacritty's and kitty's too). Build it lazily, so the memory cost is bounded by glyphs actually in use (matches oxi-ui-plan-history.md Phase 5: "glyph atlas built only from the glyphs in use").
- Monospace cell metrics make atlas lookups trivial and positions exact.
- Wide glyphs (CJK/emoji): `advance 2`, `skip` trailer already in the data model.
- Color emoji need an RGBA atlas page (Alacritty uses RGBA atlases).
- Atlas pressure: LRU eviction (glyphon does this).

### 7.2 Scalable text alternatives

- **SDF/MSDF atlas** (§6): one atlas serves all sizes, cheap, slightly soft at small sizes.
- **Slug** (§6): no atlas, exact at any scale, now patent-free; more shader complexity.
- Recommendation: not before Phase 6.

### 7.3 Compositing and damage tracking

- **Terminals** (Alacritty): per-line damage, redraw only damaged lines when the platform allows partial presents.
- **WebRender (Firefox):** picture caching in tiles, dirty rects inside tiles, scissored re-raster; display items are interned and hashed with coordinates relativized so scrolling does not invalidate caches (Mozilla bug tracker and source comments; no single design document retrieved **[partial]**).
- **Chromium:** tiled raster and compositor layers. **Not researched in this session [not verified].**
- For OxiUI, a GPU sink that redraws a whole frame in well under a millisecond **does not need tile caches**. Damage tracking pays off for: (1) the **wire** (oxi-ui-plan-history.md Phase 3 defers "rect diffing between lists"), (2) **software/framebuffer sinks** and **e-ink** (only changed regions are written), (3) power use on battery panels. Retain this as a *list diff* feature, not a GPU feature.
- The simplest GPU-side win: **render only when something changed** (the server already sends nothing for empty diffs, `FrameSink::send_frame` returns `Ok(false)`).

---

## 8. Old and Low-End GPU Support

| Hardware class | API reality (as sourced) | OxiUI path |
|---|---|---|
| Desktop/laptop with Vulkan 1.1+ / DX12 / Metal | Fully supported by wgpu | wgpu `GpuSink` |
| Raspberry Pi 4 / 5 (V3D 4.2 / 7.1) | Mesa v3dv is "notably used on the Raspberry Pi 4 and 5"; Vulkan 1.0 conformance announced Nov 2020, 1.1 and 1.2 later (Raspberry Pi announcements), Vulkan 1.3 conformance in Mesa 24.3 (Phoronix/LinuxToday). Mesa's current v3d page says V3D 3.3 and 4.1 support was dropped. | wgpu `GpuSink` should work; **verify compute and storage-buffer limits on device** before committing to anything beyond quads. Instanced quads need no compute. |
| Raspberry Pi Zero–3 (VideoCore IV, VC4) | Mesa's VC4 docs: GLES 2.0 (nearly conformant) plus OpenGL 2.1; kernel docs call the 3D engine OpenGL ES 2.0-compatible. **No Vulkan driver is mentioned for VC4** (so "no Vulkan on Pi 3" is **[inferred from absence]**). | Below wgpu's GLES 3.0 floor: **CPU/framebuffer sink only** (bitmap-font blitter to DRM dumb buffers or `/dev/fb0`, as in embedded-iot-platform.md §3.1). |
| Mali Valhall (G310/G610 and newer) | Mesa PanVK: Vulkan 1.1 conformance on Mali-G610 (Mesa 25.0.2), 1.2 about six weeks later (May 2025); Vulkan 1.4 support in Mesa 25.2, formal conformance not confirmed. Older Mali (Midgard/Bifrost) via Panfrost is OpenGL/GLES, not Vulkan **[not verified here]**. | wgpu via Vulkan on newer Mali; GLES 3.0 path on older ones **[unverified]**. |
| Adreno | Mesa Turnip: Vulkan 1.3 for the 600 series; reported officially 1.3 conformant on A6xx (Collabora, Sept 2026) and 1.4 conformant on 700 series. | wgpu via Vulkan. |
| Old desktop GPUs (e.g. GT 220, named in oxi-ui-plan-history.md D3) | **Open question.** Whether any supported driver exposes Vulkan, and whether the legacy NVIDIA driver provides a usable GLES 3.0 context through EGL for wgpu's GL backend, was **not checked**. Do not assume either. | Possibly GL 3.3 via a hand-written GL renderer (Alacritty-style) or the software sink. Decide only after testing on real hardware. |
| No GPU / VM / remote desktop | Mesa **lavapipe** is a software Vulkan implementation (CPU) usable as a wgpu adapter; WezTerm falls back to `Software` in RDP sessions. | lavapipe for CI goldens and for "GPU path on a GPU-less machine"; the CPU sink for retro boards. |

**Compute shaders** are available on Vulkan 1.0+ and GLES 3.1+ but **not** in WebGL2 or GLES 3.0. wgpu's GLES backend therefore cannot rely on compute. An instanced-quad/SDF/atlas pipeline uses only vertex and fragment shaders and so works at the wgpu floor. That is the practical reason to **write the core pipeline yourselves** instead of depending on a compute-based renderer.

**Software fallback options**

| Option | Cost | Fit |
|---|---|---|
| CPU cell/bitmap-font blitter (existing plan) | Tiny; 8×16 bitmap font; embedded doc claims <1% of a 400 MHz ARM core (its own estimate) | The only viable option on VC4-class and microcontroller-class targets |
| `tiny-skia` | Rust, no text; ARM 100–300% slower than Skia (README) | Reference renderer for tests; **too heavy for the retro targets** per retro-desktop-environment.md §145 |
| `vello_cpu` | SIMD, multithreaded, "most mature" Vello renderer | Reasonable for mid-range CPUs; heavier than the blitter; not evaluated for 400 MHz targets |
| lavapipe | Needs Mesa and a decent CPU | CI and developer machines, not retro boards |

---

## 9. Server-Side GPU for OxiScreens

Relevant to iot-screens.md §4 (3D tiers) and the `<scene>` element. This is independent of the UI rasterizer.

| Topic | Findings | Notes |
|---|---|---|
| Headless render | wgpu can render offscreen without a window (texture target + `copy_texture_to_buffer` + map; §3.1). On a server without a GPU, lavapipe works as the adapter. | A single shared `Device` with per-screen render targets; render **only on change**, as iot-screens.md §4.4 already says. |
| Tier 1 pixels | Render the `<scene>` rectangle, read back, encode JPEG (screens) or H.264 (browsers). | Readback latency is the cost; double-buffer readbacks. |
| NVENC | Consumer GeForce concurrent-session cap: **sources disagree.** Tom's Hardware and Guru3D report an increase from 3 to 5 simultaneous encodes; Wikipedia says 12 (marked as possibly self-contradictory); professional and datacenter cards are uncapped. **No 2026 primary confirmation was found; check NVIDIA's Video Encode and Decode GPU Support Matrix.** | For a home server serving a dozen panels this cap may matter. |
| VA-API (Intel/AMD) | Not researched in this session **[not verified]**. | Typical choice for Intel iGPUs on home servers. |
| ARM / Raspberry Pi | **Raspberry Pi 5 has no hardware H.264 encoder**; it relies on software libx264 (Raspberry Pi white paper: 1080p30 real-time in software, cannot go below ~240 kb/s, lower quality than Pi 4 hardware). Pi 4 had a hardware H.264 encoder (the white paper uses it as the baseline). V4L2 M2M details were **not verified**. | For the 320×240 JPEG screens in iot-screens.md, software JPEG encoding at ~1–2 ms per frame (that doc's own estimate) is likely sufficient. |
| ESP32-P4 | iot-screens.md §4.3 notes a hardware JPEG codec. | Client-side; no server action. |
| Multi-client | One scene render per distinct (scene, resolution, camera) tuple; share across screens that show the same view. | Tier 2 (projected vectors) avoids pixels entirely and is the cheapest; the same DisplayList `Path` items serve it. |

Order of effort: software JPEG first (cheap, no dependency), GPU only when the `<scene>` renderer exists and measurements show a need.

---

## 10. Browser (WebGPU, WebGL2, wgpu on the Web)

**WebGPU status**, from the gpuweb Implementation Status wiki (last edited **2026-10-02**) and the web.dev announcement (2025-11-25):

| Browser | Status |
|---|---|
| Chrome/Edge | Windows, macOS, ChromeOS from v113; Android from v121 on Android 12+ (ARM/Qualcomm/Intel; Imagination GPUs from v139 on Android 16+); **Linux only Intel Gen12+ (v144) and NVIDIA on Wayland with driver 535.183.01+ (v147)**, other hardware behind flags |
| Firefox | Windows from v141; macOS 26+ from v145 and all macOS from v147; **Linux: default in Nightly only** (Mozilla "expects to ship in 2026"); **Android: disabled by default** |
| Safari | Version 26 on macOS, iOS, iPadOS, visionOS |

Chrome also shipped a WebGPU **compatibility mode** aimed at OpenGL ES 3.1-class devices (Chrome 146, Feb 2026; Khronos SIGGRAPH 2026 slides say it shipped for Chrome on Android GLES 3.1). This is browser-side; it does not change what wgpu targets.

Consequences:
- **WebGPU is not a baseline for the OxiScreens use case** (old tablets, Pi kiosks, Linux browsers). Keep **Canvas2D** as the baseline.
- **WebGL2** is available essentially everywhere and has no compute. A WebGL2 painter doing instanced SDF quads plus a glyph atlas matches the native pipeline and works on old tablets. Prior art: the Rust `beamterm` WebGL2 terminal renderer. `vello_gpu` also has a native WebGL2 backend.
- **Likely Canvas2D hotspot:** `paint.rs` emits one `PaintOp::Text` (one `fillText`) per glyph. Batching to runs, or an offscreen glyph cache with `drawImage`, is a cheap step before WebGL. **[This is an inference from reading `paint.rs`; not profiled.]**
- **wgpu on the web:** wgpu compiles to wasm for both WebGPU and WebGL2 backends. This is attractive for sharing one pipeline between native and web (matches the Phase 7 local-mode WASM plan), but the wasm binary size cost is **unquantified here** and must be measured against the thin-client goals.
- **Sharing code without wgpu:** moving `oxiterm-web/src/paint.rs::plan()` (pure, DisplayList + metrics → pixel-space `PaintOp`s) into a crate shared by the web and native painters would keep border snapping, radius clamping and hidpi alignment identical across Canvas2D, WebGL2 and wgpu.

---

## 11. Comparison of Approaches

| Approach | Needs | Performance (what is known) | Complexity | Fit for OxiUI | Licence (permissive?) |
|---|---|---|---|---|---|
| **A. wgpu + custom instanced quads/SDF/glyph atlas** (Zed/Warp/terminal model) | Vulkan 1.x / Metal / DX12 / GLES 3.0 / WebGL2 / WebGPU | Zed and Warp publish qualitative or own-hardware numbers only (Warp avg redraw 1.9 ms, its own report); no OxiUI-specific number exists | Medium: ~3 small pipelines, atlas, scissor stack | **Best**: matches the DisplayList; works at wgpu's floor; no compute | wgpu MIT OR Apache-2.0; naga, winit (Apache-2.0), lyon, etagere, swash/cosmic-text all permissive |
| **B. wgpu + `vello_gpu` for everything** | wgpu >= 30; WebGL2 or wgpu | Good on images/gradients; trails the compute renderer on vector-heavy scenes; wgpu path not fully optimized (docs) | Low code, high dependency risk: pre-1.0, panics on some features | Good for `Path`; wasteful for rectangles/glyphs | Apache-2.0 OR MIT (vello_cpu verified locally; vello_gpu same family, not opened) |
| **C. wgpu + Vello compute (`vello`)** | Compute shaders; no WebGL | Best on complex vector scenes (qualitative) | Medium | Poor: experimental, excludes old GPUs and WebGL2 (the D3 concern) | Apache-2.0 OR MIT (same family) |
| **D. Raw Vulkan via `ash`** | Vulkan only | Highest control | **High**: you own memory, sync, swapchain, per-platform glue | Poor: loses macOS/Windows/web/GLES, no gain for this workload | ash MIT OR Apache-2.0 (from memory) |
| **E. Skia (`skia-safe`)** | GL/Vulkan/Metal/D3D | Mature, fast | Low code, **heavy** binary | Contradicts the "lightweight" goal | skia-safe MIT, wraps Skia (BSD-3-Clause) -- from memory; C++ build |
| **F. femtovg / lyon + hand-written GL** | OpenGL / GLES 3.0 | Adequate | Medium | Useful for a GL 3.3 sink if wgpu proves too big (Alacritty precedent) | lyon MIT OR Apache-2.0; femtovg license not verified |
| **G. CPU sink: bitmap blitter to DRM/fbdev** | Nothing | Embedded doc estimates <1% of one 400 MHz core (its own estimate; unmeasured) | Low (already planned) | **Required** for VC4-class boards | Own code; bitmap fonts: see 11.1 |
| **H. CPU `tiny-skia` / `vello_cpu`** | SIMD CPU | tiny-skia 20–100% slower than Skia on x86, 100–300% on ARM (README) | Low | Test reference and mid-range software fallback; not for retro boards | tiny-skia BSD-3-Clause (verified locally); vello_cpu Apache-2.0 OR MIT |
| **I. Canvas2D / WebGL2 painter (browser)** | Browser | Unmeasured | Low / Medium | Canvas2D baseline now; WebGL2 next; WebGPU later | Own code; browser-provided |

### 11.1 Licences and dependency weight (added on request: MIT OR Apache-2.0 project, OEM business model)

Policy applied: permissive (MIT / Apache-2.0 / BSD / Zlib) preferred; MPL-2.0 acceptable but flagged; GPL / AGPL / "royalty-free but not for embedded" excluded from the recommendation. **Verification level:** entries marked *(verified)* were read from `Cargo.toml` of crates present in the local cargo registry on 2026-11-22; entries marked *(memory)* are from prior knowledge and **must be confirmed with `cargo deny check licenses` and an SBOM export before adoption**. I did not run `cargo tree`, so transitive crate counts are **not measured**; weights below are qualitative.

| Candidate | Licence | Class | Weight / native deps |
|---|---|---|---|
| wgpu (+ naga, wgpu-hal) | MIT OR Apache-2.0 *(memory)* | Permissive | Large transitive tree (naga, gpu-allocator/ash on Vulkan, `windows`/metal bindings per platform, khronos-egl/glow for GL). Pure Rust; no C++ build. Loads the system's Vulkan/GL driver at runtime. |
| winit | Apache-2.0 *(memory)* | Permissive | Moderate; platform bindings. |
| lyon / lyon_tessellation | MIT OR Apache-2.0 *(memory)* | Permissive | Small, pure Rust. |
| cosmic-text | MIT OR Apache-2.0 *(memory)* | Permissive | Moderate (swash, skrifa/read-fonts, rustybuzz, fontdb, unicode crates). Pure Rust. |
| swash, skrifa, read-fonts | skrifa and read-fonts MIT OR Apache-2.0 *(verified)*; swash MIT OR Apache-2.0 *(memory)* | Permissive | Small to moderate, pure Rust. |
| rustybuzz (HarfBuzz port) | MIT *(verified)* | Permissive | Pure Rust port; avoids linking C++ HarfBuzz (itself MIT, *memory*). |
| etagere | MIT OR Apache-2.0 *(memory)* | Permissive | Tiny. |
| glyphon | MIT OR Apache-2.0 OR Zlib *(memory)* | Permissive | Thin layer over cosmic-text + wgpu + etagere. |
| fontdue | MIT OR Apache-2.0 *(memory)* | Permissive | Small, pure Rust, no shaping. |
| vello_cpu / vello_gpu / vello (compute) | Apache-2.0 OR MIT *(vello_cpu verified; others same family, memory)* | Permissive | `vello_gpu` pulls wgpu >= 30, `vello_common`, `glifo`, skrifa; moderate to large, pure Rust. |
| tiny-skia | BSD-3-Clause *(verified)* | Permissive | Small, pure Rust. Already in the tree via resvg. |
| resvg | Apache-2.0 OR MIT *(verified)* | Permissive | Already a dependency. |
| skia-safe (Skia) | MIT bindings, Skia BSD-3-Clause *(memory)* | Permissive | **Heavy: C++ Skia build or prebuilt binaries**, large size. Not recommended on weight, not on licence. |
| femtovg | not verified | unknown | Check before considering. |
| iced / egui | MIT; MIT OR Apache-2.0 *(memory)* | Permissive | Whole toolkits; only their renderer designs are of interest. |
| Slint | **GPL-3.0-only OR Slint Royalty-free OR Slint commercial**; the Royalty-free licence does **not** cover embedded systems; embedded needs GPLv3 or a paid commercial licence (Espressif component licence listing and Slint FAQ, via search) | **Excluded** | Not usable as a dependency for the OEM model. Only its design (software + GPU renderers, LinuxKMS) is referenced. |
| Zed GPUI crate | Apache-2.0 *(memory)*; the Zed editor itself is GPL/AGPL-family *(memory)* | Study only | Do not vendor Zed code; reimplement the (generic) technique. |
| Alacritty | Apache-2.0 *(memory)* | Study | Reading its renderer is fine; copying needs Apache notices. |
| kitty | GPL-3.0 *(memory)* | **Excluded** | Do not copy code. Ideas and published benchmarks only. |
| WezTerm / Ghostty | MIT *(memory)* | Study | OK to reference; keep attribution if code is copied. |
| Slug reference shaders | MIT, patent dedicated to public domain 2026-03-17 (Lengyel post); the commercial Slug library is proprietary | Permissive (shaders only) | Usable in Phase 6 only; do not use the proprietary library. |
| Mesa lavapipe / drivers | MIT *(memory)* | Permissive | Test/CI and system dependency only, not linked into the product. |
| Loop-Blinn technique | Patent reportedly expired 2026-03-25 (secondary) | n/a | Verify before relying on it. |
| Pathfinder | MIT OR Apache-2.0 *(memory)* | Permissive | Maintenance status unclear; not recommended. |

**Fonts bundled in `FontBundle` / bitmap blitter.** Licences from memory; **confirm per file**:
- JetBrains Mono, Noto families, Terminus: SIL OFL 1.1. Bundling is allowed, but the font may not be sold by itself and a modified font may not reuse the Reserved Font Name; ship the licence text.
- DejaVu / Bitstream Vera: permissive Bitstream Vera licence (free to bundle and modify; rename derivatives).
- Spleen: BSD-2-Clause.
- GNU Unifont: GPL-2.0-or-later **with font exception** plus OFL for part of it; the exception covers embedding in documents, but legal review is advised before bundling for OEM firmware. Prefer OFL fonts for the embedded blitter.
- Any font embedded into a binary needs its licence text in the third-party notices.

**Consequences for the recommendation.** Every crate the recommended stack needs (wgpu, winit, lyon, cosmic-text or swash/skrifa, etagere, optionally `vello_gpu`) is permissive. The two things not to depend on are Slint (embedded terms) and anything GPL (kitty). Skia is licence-compatible but excluded on weight (C++). Using libraries instead of writing code is realistic here: the **only hand-written GPU code needed is the small SDF-quad and atlas-instancing shaders**; text shaping/rasterization (cosmic-text/swash or glyphon), atlas packing (etagere), tessellation (lyon) and windowing (winit) should all be reused. Add `cargo deny` with an allow-list (MIT, Apache-2.0, BSD-2/3, Zlib, ISC, Unicode-3.0, optionally MPL-2.0 flagged) to CI before adding wgpu.

---

## 12. The CPU-Side Bottleneck: Order of Attack

The GPU only draws what the CPU hands it. In OxiUI today the per-frame CPU work is large relative to what a GPU would save.

**Observed in code (not measured):**

1. `LayoutEngine::compute` calls `reset_nodes()` at the start of every call, which replaces the Taffy tree and clears the node map and `last_layout`. All Taffy per-node caches are lost, even though `doc.dirty_nodes` exists and is iterated afterwards.
2. Taffy 0.6.3 (locked in `Cargo.lock`) has a per-node `Cache` and `TaffyTree::mark_dirty` (verified by grepping the vendored sources at `~/.cargo/registry/src/*/taffy-0.6.3/src/tree/{cache,taffy_tree}.rs`). Incremental layout is therefore supported by the engine; whether OxiUI can use it depends on handling `bind-show` toggles and `Display::None` correctly (the stated reason for the reset).
3. `build_display_list` rebuilds the whole list every frame and, in GUI sessions, `render_frame_mode` also runs `CellSink::paint` into a `CellBuffer` and the legacy painters (img, video, diagram) every frame, then the server zlib-compresses the entire list for the wire (oxi-ui-plan-history.md Phase 3a).
4. The web client rebuilds all `PaintOp`s and issues one `fillText` per glyph per frame.

**Recommended order**

| Step | What | Why first | Expected effect |
|---|---|---|---|
| 0 | **Instrument.** Per-stage timings (layout, list build, CellSink pass, legacy painters, compress, send; client: plan, paint) in a bench over `examples/**/*.thtml`. | Everything below is a guess until measured. No OxiUI frame-time number exists today. | Unknown |
| 1 | **Stop doing redundant CPU work in GUI sessions:** skip the `CellBuffer` pass when no cell client needs it; cache `Legacy` content by node and content hash. | Cheap, no design risk, and the wire/GPU sink needs it anyway | Unknown; measure |
| 2 | **Incremental layout:** keep the Taffy tree across frames, mark nodes dirty from `doc.dirty_nodes`, handle `bind-show` by style change (Display none/flex) instead of rebuilding. | Layout cost scales with document size every frame; this also helps TUI sessions | Unknown; measure |
| 3 | **List diff / damage** for the wire and software sinks | Only if step 0 shows wire or client paint dominates | Unknown |
| 4 | **GPU sink** (this document) | Valuable for high DPI, sub-cell animation and no CPU raster, but cannot reduce 1–3 | Unknown |

Honest framing: the GPU sink is likely **not** the first-order frame-time win for typical OxiUI screens (a few hundred items). It is the right foundation for the native window, hidpi smoothness, animation and the 3D tiers. Steps 1–2 help every backend including SSH.

---

## 13. Risks and Open Questions

| Risk / question | Notes | How to resolve |
|---|---|---|
| **RSS with a GPU driver loaded** | oxi-ui-plan-history.md targets <15–25 MB. A Vulkan or GL driver stack plus wgpu is likely to exceed that; the amount is **unknown** | Measure a minimal winit + wgpu window early; document in Phase 5 "done when" |
| **wasm size of wgpu** | Unquantified here | Build a minimal wgpu wasm and compare with the current thin client |
| **wgpu churn** | Majors v29 (Mar 2026) and v30 (Jul 2026) with breaking changes; patch releases since | Pin; isolate wgpu behind `GpuSink` |
| **`vello_gpu` maturity** | Panics on mask layers, complex filters, some blends; experimental glyph caching; renamed from `vello_hybrid`; needs wgpu >= 30 | Use only for `Path`, behind a feature flag, or skip in favour of lyon |
| **Old desktop GPUs (GT 220)** | Vulkan/GLES 3.0 availability unknown | Test on real hardware before promising support |
| **Pi 4/5 compute and limits** | Vulkan versions are known, wgpu limits on v3dv are not | Run a wgpu adapter limits dump on a Pi 4 and Pi 5 |
| **Mali/Adreno on older SoCs** | Only recent PanVK and Turnip status verified | Test targeted panels |
| **Color management and blending** | sRGB vs linear changes how text looks versus Canvas2D | Define it in the sink semantics, add goldens |
| **Hit-testing and a11y** | Unchanged: they use node ids on DisplayList items, not GPU state | Keep node ids on every instance for picking |
| **Image/video in GUI mode** | Currently `CellGrid`; looks wrong on a pixel GPU | Add real image/video items before the native window ships |
| **Naming collision** | A search for "oxiui render wgpu" returned crates named `oxiui-render-wgpu` on crates.io, apparently another project | Check crate names before publishing |
| **NVENC session cap** | Sources disagree (3 vs 5 vs 12) | Read NVIDIA's support matrix |
| **Chromium raster/compositing, Skia Graphite/Spinel, VA-API, V4L2 M2M details** | Not researched in this session | Follow up only if relevant |

---

## 14. Concrete First Milestone and Revised D3

### 14.1 Proposed revision of D3

> **D3 (revised):** The GPU rasterizer for Phase 5 is a custom wgpu pipeline (instanced SDF rounded-rect quads, instanced glyph-atlas quads, scissor clips), using only vertex and fragment shaders so it works down to GLES 3.0 / WebGL2. `Path` items use `lyon` (or `vello_gpu` behind a feature). Vello's compute renderer is not used. For hardware below the wgpu floor, the CPU/framebuffer sink from the embedded docs is the fallback; it is not a separate "old GPU" renderer. A hand-written GL 3.3 sink (Alacritty-style) is only added if testing shows wgpu's GL backend is inadequate on a specific retro target.

### 14.2 Milestone M1 (small, fits the golden-test culture)

1. New crate `oxiterm-gpu` (or `oxiterm-gui` skeleton) with a **headless** `GpuSink` on wgpu: no window.
2. Support `Fill` (with radius), `Border` (single/double/rounded), `Text` (atlas with the bundled monospace font), `CellGrid`, and scissor clips. Skip `Path` initially.
3. Add the AnsiColor→RGBA table, shared with `paint.rs`.
4. Render the existing `examples/**/*.thtml` DisplayLists at the two golden sizes to a texture, read back, and compare with **PNG goldens** (tolerance-based, with an `OXI_BLESS=1` workflow like `golden_cells.rs`).
5. Run these tests in CI on **lavapipe** (Mesa software Vulkan); skip when no adapter exists.
6. Emit `PushClip`/`PopClip` from the builder only as needed; keep TUI goldens byte-identical.
7. In parallel and independent: **Step 0 instrumentation** from §12.

Then, in order: `winit` window and input (Phase 5), `Path` via lyon, hover/`:hover`, RSS measurement, the Pi 4/5 test, a WebGL2 painter reusing `plan()`.

### 14.3 What not to do first

- Do not write a raw Vulkan (`ash`) backend.
- Do not start from Vello compute.
- Do not build 3D tiers on the GPU before the `<scene>` element exists.
- Do not build tile caches or retained GPU layers before measurements justify them.

---

## 15. Sources

Accessed 2026-11-22 unless noted. "Secondary" marks news or blog coverage; "mirror" marks non-canonical copies.

**GPU stacks**
- Vello repository README: https://github.com/linebender/vello
- `vello_gpu` docs (0.4.0, rename from `vello_hybrid`, wgpu >= 30, WebGL2, limitations): https://docs.rs/vello_gpu
- `vello_hybrid` docs (0.2.0, feature flags, limitations): https://docs.rs/vello_hybrid
- Linebender newsletters on sparse strips (Mar/May 2025): https://linebender.org/blog/tmil-15/ and https://linebender.org/blog/tmil-17/
- ETH thesis, "High-performance 2D graphics rendering on the CPU using sparse strips" (2025): https://ethz.ch/content/dam/ethz/special-interest/infk/inst-pls/plf-dam/documents/StudentProjects/MasterTheses/2025-Laurenz-Thesis.pdf
- wgpu README (backend tiers, MSRV): https://github.com/gfx-rs/wgpu (raw: https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/README.md)
- wgpu releases (v30.0.0 2026-07-01, v30.0.2 Nov): https://github.com/gfx-rs/wgpu/releases
- tiny-skia README: https://github.com/linebender/tiny-skia
- lyon: https://github.com/nical/lyon and https://docs.rs/lyon
- glyphon: https://docs.rs/glyphon
- iced renderers (wgpu / tiny-skia fallback; secondary guide): https://jl710.github.io/iced-guide/render_backend.html and https://docs.rs/iced_renderer/0.14.0/src/iced_renderer/lib.rs.html
- Slint backends and renderers: https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/
- Zed, "Leveraging Rust and the GPU to render user interfaces at 120 FPS": https://zed.dev/blog/videogame
- Zed GPUI wgpu migration commit (Feb 2026): https://git.secluded.site/zed/commit/af8ea0d6c26192c45f44f473c0d4a7d6f72ed018 (secondary summary: https://news.hada.io/topic?id=26669)
- Warp, "How Warp Works": https://www.warp.dev/blog/how-warp-works
- Warp, "How to draw styled rectangles using the GPU and Metal": https://www.warp.dev/blog/how-to-draw-styled-rectangles-using-the-gpu-and-metal

**Terminals**
- Alacritty renderer sources (0.17.0, GLSL3/GLES2, atlas): https://docs.rs/crate/alacritty/0.17.0/source/src/renderer/text/mod.rs , https://docs.rs/crate/alacritty/0.17.0/source/src/renderer/text/gles2.rs
- kitty performance page: https://sw.kovidgoyal.net/kitty/performance/
- WezTerm `front_end`: https://github.com/wezterm/wezterm/blob/main/docs/config/lua/config/front_end.md
- Ghostty about page (no GPU detail): https://ghostty.org/docs/about ; Ghostty renderer details only via mirrors and the Rust port sugarloaf (https://docs.rs/crate/sugarloaf/0.4.1/source/src/grid/metal.rs) **[second-hand]**
- beamterm (WebGL2 terminal, Rust): https://docs.rs/crate/beamterm-renderer/0.10.0/source/README.md **[not examined in depth]**
- Dan Luu, "Terminal latency": https://danluu.com/term-latency/ **[numbers not retrieved; none quoted]**

**Papers and talks**
- Valve, Green, "Improved Alpha-Tested Magnification..." (SIGGRAPH 2007): https://cdn.akamai.steamstatic.com/apps/valve/2007/SIGGRAPH2007_AlphaTestedMagnification.pdf
- Chlumský, msdfgen and thesis: https://github.com/Chlumsky/msdfgen , https://dspace.cvut.cz/bitstream/handle/10467/62770/F8-DP-2015-Chlumsky-Viktor-thesis.pdf ; CGF 2018: https://diglib.eg.org/handle/10.1111/cgf13265
- Ganacim et al., "Massively-parallel vector graphics" (2014): https://history.siggraph.org/?p=209956 , https://preprint.impa.br/visualizarArquivo/6602/2425
- Kilgard and Bolz, GPU path rendering (2012), NVIDIA: https://developer.nvidia.com/gpu-accelerated-path-rendering
- Slug, "A Decade of Slug" (patent dedicated 2026-03-17): https://terathon.com/blog/decade-slug.html ; coverage: https://hackaday.com/2026/03/20/slug-algorithm-for-on-gpu-rendering-of-fonts-with-bezier-curves-now-in-public-domain (Loop–Blinn expiry date from this secondary source)
- Pathfinder: https://github.com/servo/pathfinder **[status from an index snapshot; may be stale]**
- WebRender picture caching/dirty rects (Mozilla bugs, source): https://bugzilla.mozilla.org/show_bug.cgi?id=1580063 , https://searchfox.org/firefox-main/source/gfx/wr/webrender/src/tile_cache/mod.rs

**Hardware and drivers**
- Mesa v3d (Pi 4/5): https://docs.mesa3d.org/drivers/v3d.html ; VC4 (Pi 0–3): https://docs.mesa3d.org/drivers/vc4.html
- Raspberry Pi Vulkan conformance: https://www.raspberrypi.com/news/vulkan-update-version-1-2-conformance-for-raspberry-pi-4/ ; Vulkan 1.3 in Mesa 24.3: https://www.phoronix.com/news/Mesa-24.3-V3DV-Vulkan-1.3
- PanVK: https://www.collabora.com/news-and-blog/news-and-events/panvk-reaches-vulkan-12-conformance-on-mali-g610.html and https://www.collabora.com/news-and-blog/news-and-events/panvk-now-supports-vulkan-1.4.html ; Turnip A6xx: Collabora blog, Sept 2026 (via search summary; URL not opened) **[unverified]**
- Raspberry Pi 5 H.264 encode white paper: https://pip-assets.raspberrypi.com/categories/685-app-notes-guides-whitepapers/documents/RP-010033-WP-1-H.264%20encoding%20performance%20on%20Raspberry%20Pi%205_series%20computers.pdf
- NVENC session limits (sources disagree): https://www.tomshardware.com/news/nvidia-increases-concurrent-nvenc-sessions-on-consumer-gpus , https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/nvenc-application-note/index.html

**Browser**
- gpuweb Implementation Status (last edited 2026-10-02): https://github.com/gpuweb/gpuweb/wiki/Implementation-Status
- web.dev, "WebGPU is now supported in major browsers" (2025-11-25): https://web.dev/blog/webgpu-supported-major-browsers
- Chrome 146 WebGPU compatibility mode: https://developer.chrome.com/blog/new-in-webgpu-146

**Local sources**
- `docs/history/oxi-ui-plan-history.md`, `docs/iot-screens.md`, `docs/retro-desktop-environment.md`, `docs/embedded-iot-platform.md`
- `oxiterm-renderer/src/render/display_list.rs`, `layout/engine.rs`, `render/renderer.rs`, `render/cell_sink.rs`; `oxiterm-web/src/paint.rs`; `Cargo.lock` (taffy 0.6.3) and `~/.cargo/registry/src/*/taffy-0.6.3/`

**Not verified this session:** egui/epaint internals, skia-safe, femtovg beyond Slint's description, cosmic-text/swash/etagere individually, Chromium's raster pipeline, Skia Spinel/Forge, VA-API and V4L2 M2M encoders, Pi 3 Vulkan (inferred absent), GT 220 driver support, Mesa lavapipe's current Vulkan version (its docs page returned 404), RSS and wasm-size figures, any OxiUI frame-time number.
