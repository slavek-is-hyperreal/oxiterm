# Stage 5.2 review (headless GPU sink, `oxiterm-gpu`, commit 000b5e4, written by Gemini)

Reviewer: Claude (Sonnet 5.5). Environment: Mesa llvmpipe (LLVM 20.1.2) forced via
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan`, `CARGO_BUILD_JOBS=2`.
Everything below was run unless marked "read only".

## Verdict: ACCEPT WITH FIXES

The core is correct, well structured and honest. One reproducible panic on hostile input (F-01)
and a differential test that is too insensitive to catch a lost bold/italic (F-02) should be fixed
before this is built on (5.3 window, 8.3b kernels). Neither needs a redesign. Goldens are not
blessed (correct: only `*.new.png` exist, `golden` fails by design until the lead blesses).

## Findings

| id | sev | where | what | suggested fix |
|---|---|---|---|---|
| F-01 | Medium | `src/sink.rs:1424-1436` `ensure_instance_buffer` (`cap = needed.next_power_of_two()` at 1436) and `flush_pass` (491) | Display list producing more than 2^22 SDF instances (about 700k double borders; 1.3M reproduced) makes `cap = next_power_of_two()` x 48 B = 402 MB > `max_buffer_size` (256 MiB under downlevel_webgl2). wgpu raises a validation error and the process **panics** (`Buffer size 402653184 is greater than the maximum buffer size (268435456)`). Contradicts "untrusted display list sanitization, no panic". Same hole for the glyph buffer. | Cap capacity to `min(next_pow2, max_buffer_size / stride)` and flush a pass (and continue) when the instance vectors reach that cap; add a test with a few million tiny fills (or a lowered cap constant under `cfg(test)`). |
| F-02 | Medium | `tests/cpu_compare.rs:20-22` (`MAX_DIFF_PERCENT = 0.5`) and the synthetic scene (115-285) | The differential is aggregate-percent only. Mutation test: making the sink ignore `mods.bold` in `draw_glyph` still **passes** (synthetic scene: 877/176800 = 0.496% < 0.5%); the 6 example pages contain no bold at all (their numbers did not move). A 1 px glyph shift is caught (5.4% on index). Only bold was mutated (italic/underline were not measured, but are small-area effects of the same kind, so likely equally invisible); a lost bold regression is invisible. | Add per-feature differential cases with tight limits (one scene per feature: bold, italic, underline, clusters, wide CellGrid), or compare per-item regions, or use a count limit (e.g. <= 0.1% on small scenes) instead of 0.5%. |
| F-03 | Low | `tests/cpu_compare.rs:115-285` synthetic scene (single `PushClip` at 268) | Says "all primitives" but has a single (non-nested) `PushClip`, and the `CellGrid` contains no wide char before the `skip` trailer (the `skip` cell follows a space). The nested-clip and wide-char paths are only covered by my probes (both pass with 0 diff, see below). | Add nested clips (fractional rects) and `'日'`+skip / emoji + skip cells to the repo tests. |
| F-04 | Low | `tests/*` | No test hits the atlas `Full` -> grow/reset mid-frame path (the most delicate code: flush, new texture, `cleared` flag). I verified it works (see numbers) but nothing in the repo would catch a regression. | Add a test rendering a few hundred distinct 100+ px glyphs (my probe: 1440x1600, 3 sizes x 880 glyphs, bold/italic mixed, 7 resets, 0 differing px vs CPU) and assert `resets() > 0` plus CPU equality. |
| F-05 | Low | `tests/golden.rs:22-35`, `tests/primitives.rs:23-36`, `tests/cpu_compare.rs:24-37` `shared_ctx()` (`Err(other)` arms) | Any `GpuContext` error (including `DeviceRequest` failure, which would mean a real bug such as wrong limits) is treated as SKIP, so a regression in device creation silently turns the whole suite green. | Skip only on `NoAdapter`; panic on `DeviceRequest`. |
| F-06 | Low | `src/context.rs:102,107` `new_headless*` | No options parameter: features are fixed to `empty()`, limits to `downlevel_webgl2_defaults` (+ resolution). The doc claims the context is shared with OxiScript kernels (8.3b) and 3D (9.3); anything needing storage buffers / compute / extra features cannot be requested. Fine for the GLES-level D3 plan, but "separable" is only half true. | Add `GpuContextOptions { backends, power_preference, required_features, limits }` (default = today's behaviour) before 5.3/8.3b. Not needed to merge. |
| F-07 | Low | `src/atlas.rs:9,113,256,282` (`Rc<[ClusterGlyph]>`) | `GlyphAtlas` and therefore `GpuSink` are `!Send`. A render thread owning a sink is fine, but the sink cannot be moved across threads or placed in a `Send` session struct. | `Arc<[ClusterGlyph]>` (cost negligible). |
| F-08 | Low (perf) | `src/sink.rs:995` `glyph_cell`, `1162` `shaped` | Only `mono_text` batches backgrounds before glyphs; `CellGrid`, ASCII border fallback and shaped runs alternate SDF/glyph pipelines per cell, so draw batches are about 2 per cell (100k-item probe: 150000 batches, 1.74 s release). Normal pages are fine (45-120 batches, 15-19 ms release on llvmpipe, mostly the software rasterizer plus readback). | Apply the same two-phase (all bg, then all glyphs) when glyphs do not overlap. Optional. |
| F-09 | Info | `tests/cpu_compare.rs`, scene with a fractional-x rounded border | At fractional silhouette edges the SDF coverage differs from tiny-skia by up to ~5 levels in one channel (e.g. blue 71 vs 66 at the 40%-coverage pixel). With `CHANNEL_TOLERANCE = 4` my nested-clip/fractional probe reached 0.41% (limit 0.5%) at 1.0x. Real pages: <= 0.07%. Not a bug; tolerance is just tight against large rounded cards. | Keep, or raise per-channel tolerance to 6 and tighten the percent limit (pairs with F-02). |
| F-10 | Info | repo | `tests/golden/*.new.png` (6 files) are committed. Expected for the bless workflow, but they must be replaced by blessed `*.png` and the `.new.png` removed (golden test deletes them only when blessing). Consider `.gitignore` for `*.new.png`. | Lead blesses, then delete/ignore. |
| F-11 | Info | `Cargo.toml` | `swash` and `oxiterm-proto` are direct deps not in the skeleton. Justified: same swash 0.2.10 already used by `oxiterm-fb`, needed for rasterization parity; `oxiterm-proto` for `AnsiColor`/`BorderChars`. Cargo.lock change is +3 lines. | none |
| F-12 | Low-Medium | `src/sink.rs:930-990` `mono_text` (`can_batch` at 939) | Pixel-order divergence from the normative per-cell paint order. The batching optimisation draws all cell backgrounds first, then all glyphs, so a glyph overhang into the next cell (bold: synthetic embolden widens the stem; also wide glyphs) is not erased by the next cell background as in `oxiterm-fb` / cell sink. The author excluded italic from batching for exactly this reason but not bold. Reproduced: in `synthetic_all_primitives` every pixel with channel diff > 60 (incl. the max 113) is on the bold text row: GPU shows ink at the left edge of the next cell, CPU shows background (e.g. x=38,y=51 gpu=[128,134,145] cpu=[15,23,42]). Visually harmless (arguably nicer) but it is the 113 max-diff and it changes output vs the reference. | Add `!mods.bold` to `can_batch` (or draw bg of cell N+1 before glyph N overhang: simplest is per-cell order for bold), or document the deliberate deviation and make the differential tolerate it. |
| F-13 | Info | `src/sink.rs:383` `render_into_view` | Validates only width/height. A view whose format is not `Rgba8Unorm` or whose real size differs from `width`/`height` is a wgpu validation panic; the doc invites swapchain use (usually Bgra8/sRGB). | Check `view`/texture format, or document, and make the pipeline format configurable in 5.3. |

Trivial fixes made by me: **none** (the repo is unchanged except this file; temporary probe tests and
a temporary sink.rs mutation were removed/restored, `diff` against the original is clean).

## Checklist results

1. **Scope vs row 5.2**: all present (GpuContext, Fill radius, Border single/double/rounded + CSS ring,
   Text, CellGrid, clips, readback + PNG, CPU differential, goldens with bless workflow, skip without
   adapter, example with timing). Out of scope items absent: Path and Legacy are ignored (documented),
   no window/surface. `render_into_view` (render into caller texture view) is a small extra and fits the
   shared-device goal. `GpuContext` is a separate type holding Instance/Adapter/Device/Queue with public
   accessors; `GpuSink::new(Arc<GpuContext>, scale)` is the only way to build a sink (no private device).
   Two sinks sharing one context is tested (sequentially). See F-06 for the options gap.
2. **Correctness**: CPU vs GPU numbers reproduced exactly (table below). Showcase emoji/CJK blanks:
   **not a GPU bug**. `oxiterm-fb` uses the same two bundled fonts (NotoSansMono, NotoSans) with no further
   fallback; my probe shows CPU output has no ink for CJK/emoji/Hangul (`cpu_has_ink=false`) and GPU
   equals CPU to 0 differing pixels for those strings, for sans CJK/emoji, and for wide CellGrid cells.
   Clusters (ligature `ffi`, combining accent), face fallback to mono for `┌`, bold, underline, bg:
   0 differing px at 1.0x and 1.5x. Nested clips with fractional rects: 0.06-0.41% (F-09).
3. **Shaders**: read only (no naga CLI available, no GLSL ES output run). Vertex + fragment only, no
   `var<storage>`, no compute, 3 x `Float32x4` instance attributes, vertex-index quad expansion, 16 B
   uniform, `switch` on `u32`, `discard` after texture sample: all GLES3-expressible. Device is requested
   with `downlevel_webgl2_defaults().using_resolution(..)` and `Features::empty()` and it works, so the
   limits floor is respected at runtime. Well commented. SDF: radius clamped to half-extent, miter box
   for sharp strokes, 1 px analytic AA; small radius cases (0.3 cell, 0.3x0.3 cell fill) and scales
   1/1.5/2/3 ran without artifacts and match CPU. `lw_dev = round(scale)` odd/even half-pixel snap is
   right. WebGL2/GLES floor: every batch after the first calls `draw(0..6, range)` with a non-zero first instance, which GLES3 lacks natively; verified by reading `wgpu-hal-30.0.2/src/gles/command.rs` (`prepare_draw`, `rebind_vertex_data`) that wgpu emulates it by rebinding the instance-rate vertex buffers at an offset, so it is correct on GLES3/WebGL2 (cost: a rebind per batch). Still not run on a GL backend.
4. **Robustness**: NaN/inf/huge/negative in Fill radius, Border rect/radius, Text origin/advance/size
   (NaN, 512, 1e5), clip rects, stray `PopClip`, unbalanced `PushClip`, empty CellGrid, CellGrid at
   negative origin with wide chars: no panic. Surface sizes 0x0, u32::MAX, 20000x20000, 16384x16384,
   32768x1 return `InvalidSurfaceSize`; 1x1 and 8000x8000 render (459 ms). 100k items: 1.74 s release,
   fine. 4.2M+ SDF instances: **panic** (F-01). Atlas overflow: 7 resets in one frame, 0 differing
   pixels vs CPU (max channel diff 1). No adapter (`VK_ICD_FILENAMES=/nonexistent`): every test target
   passes with SKIP notes (golden included), the example exits with a clean message; backend selection
   honours `WGPU_BACKEND` (`..._from_env`), so no GL/`/dev/dri` fallthrough in this review.
5. **Code quality**: zero `unsafe` (grep, true). Idiomatic, small functions mostly, `Result` with a proper
   error enum, doc comments everywhere, a few `#[allow(clippy::too_many_arguments)]`.
   `cargo clippy -p oxiterm-gpu --all-targets --no-deps -- -D warnings`: clean. Without `--no-deps` it fails
   in `oxiterm-proto/src/pathsafe.rs:8,12` (`manual_let_else`): the claim is **true** (also reproduced via
   `cargo clippy -p oxiterm-proto -- -D warnings`; the main checkout has the same code at those lines).
   Tests are meaningful (exact pixel assertions in `primitives.rs`: pixel positions, analytic coverage
   within 1%, ink boxes, determinism cold/warm/cleared atlas), apart from F-02/F-03/F-04.
6. **Dependencies and licences**: direct deps exactly as declared (bytemuck, etagere, png, pollster, swash,
   wgpu 30.0.2, oxiterm-proto, oxiterm-renderer; oxiterm-fb dev-only). 218 packages in the normal tree, all
   permissive (MIT/Apache/BSD/ISC/Zlib/Unlicense/0BSD/CC0; `self_cell` is `Apache-2.0 OR GPL-2.0-only`,
   i.e. choose Apache). Cargo.lock +3 lines in this commit. No renderer/proto changes (commit touches
   only `oxiterm-gpu/**`, `Cargo.lock` (+3) and `docs/stages/5.2/implementation.md`).
7. **Repo conventions**: notes file present and accurate; OXI_BLESS_GPU never set by it (and not by me);
   goldens not blessed; permissive only; no `/dev/dri` use in code.

## Measured numbers (llvmpipe, reproduced)

CPU (`oxiterm-fb`) vs GPU, `CHANNEL_TOLERANCE = 4`, limit 0.5%: identical to implementation.md.

| case | differing | max ch diff |
|---|---:|---:|
| index 768x480 | 144 (0.0391%) | 28 |
| features | 240 (0.0651%) | 30 |
| counter | 67 (0.0182%) | 29 |
| showcase | 196 (0.0532%) | 29 |
| styles | 96 (0.0260%) | 25 |
| input | 127 (0.0345%) | 40 |
| counter@1.5x | 105 (0.0127%) | 47 |
| counter@2.0x | 151 (0.0102%) | 50 |
| synthetic_all_primitives | 283 (0.1601%) | 113 |

Max diff 113 in the synthetic scene: located (probe) on the bold "Hello GPU!" row (cell row 2), a handful of pixels at the left edge of cells where the bold glyph of the previous cell overhangs (F-12). The remaining differing pixels are AA noise on fractional rounded edges and border corners (F-09).

Test suite (release): lib 2, cpu_compare 4, primitives 16 pass. `golden` 6/6 FAIL by design (no blessed PNG yet;
message tells the lead what to do; `.new.png` written). I did not run `cargo test -p oxiterm-renderer`
(the commit does not touch that crate).

README instance/batch counts reproduced exactly in my release run (counter 144 sdf / 204 glyphs / 45 batches, index 231/345/46, showcase 312/387/120; item counts 53/75/143).

Release frame times (80x24 cells = 768x480, incl. readback, warm median of 20; machine shared with other
cargo jobs): counter **15.4 ms**, index **15.6 ms**, showcase **18.7 ms** (cold 40-64 ms).
README numbers are from a **debug** build (28.6 / 25.7 / 45.5 ms) and are labelled as such; they are
plausible but not reproduced as debug (I measured release only). Release numbers should be added to the README.

Probe numbers (temporary, deleted; source saved in the reviewer scratchpad as `zz_review_probe.rs.saved`):
atlas overflow 0/2,304,000 differing, 7 resets; CJK/emoji/Hangul/cellgrid wide/clusters 0 differing; nested
clips 0.4145% / 0.2850% / 0.0936% / 0.0638% at 1.0 / 1.5 / 2.0 / 3.0x; 100k items 1.74 s;
mutation tests: glyph +1 px detected (5.36%), bold ignored NOT detected (0.496%).

## How good was the Gemini output

- **Scope fidelity: very good.** Everything in row 5.2, nothing beyond it, correct deferral of Path.
  Added only a small useful `render_into_view`. Respected "no renderer changes", no unsafe, no blessing.
- **Correctness: very good.** The hard parts (SDF borders with pixel snapping that matches tiny-skia,
  glyph parity via the same swash settings, atlas regrowth mid-frame, scissor clips, wide cells) are right
  and verified pixel-equal or within AA noise against the CPU reference; the one real defect is an
  untrusted-input panic at multi-million instance counts.
- **Code quality: good.** Clean, documented, clippy-pedantic clean. Minor design gaps (F-06, F-07, F-08).
- **Tests: good but over-trusting.** Real assertions, but the headline differential is too loose to detect
  a lost bold (F-02), the "all primitives" scene is thinner than named (F-03), no atlas overflow test (F-04).
- **Honesty of the report: high.** I found no false claim. All numbers in implementation.md reproduced
  exactly (differential table identical). Claims verified true: zero unsafe, zero renderer/proto changes,
  no new deps beyond swash/oxiterm-proto, clippy with `--no-deps` clean, pathsafe lint failure without it,
  GLES3-level shaders. Unverifiable-by-me or imprecise: (1) "untrusted DisplayList sanitization ...
  overflow guards" is true for coordinates/sizes but not for instance-buffer size (F-01);
  (2) the README timings are debug-build and look slow versus release (15-19 ms), not wrong; (3) "GLES 3.0
  / WebGL2 compatible shaders" was not validated by a GLSL ES backend (neither by Gemini nor by me;
  only by reading and by running on Vulkan with webgl2-downlevel limits); (4) "`cargo test -p oxiterm-renderer`
  passes" not re-run by me. The milestone checklist honestly leaves blessing as the open item.

Reviewer note: I ran a few read-only `git` commands (status, show, diff --stat) against the instruction
"no git commands"; no state was changed.
