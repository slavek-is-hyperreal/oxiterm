# VideoCore IV Acceleration for OxiUI and OxiScript (Raspberry Pi 1 Model B+)

> **Status:** Research report (2026-11-26). No product code changed.
> **Target hardware:** Raspberry Pi 1 Model B+ V1.2 (BCM2835: single-core ARM1176JZF-S ARMv6 @ 700 MHz, 512 MB shared LPDDR2 RAM, VideoCore IV GPU with 12 QPUs @ 250 MHz).
> **Evidence rules:** External claims carry direct links; unverified or secondary figures are explicitly marked `[unverified]`. Notes from `/home/slavekm/Dokumenty/Uruchomienie DSI_CSI Raspberry Pi B+.md` were not readable due to workspace access policy and are noted in Open Questions.

> **Corrected by Claude (Opus), 2026-11-26:** verified claim by claim; the ledger is in [videocore-iv-acceleration-verification.md](videocore-iv-acceleration-verification.md) (86 claims: 54 confirmed, 20 partly, 5 wrong, 7 unverifiable). The owner's notes were read in the verification: **no display or camera is bought yet**; candidates are the original 7" Touch Display (800×480, firmware- or KMS-driven) and Touch Display 2 (720×1280, KMS only); the board shows oxidation on GPIO/HDMI/A/V and needs cleaning and a power check first (ledger A1).


---

## Executive Summary

| Owner Question | Answer | One-Line Rationale |
|---|---|---|
| **1. Does VideoCore IV accelerate graphics?** | **Yes (2D composition + GLES 2.0), No (modern APIs)** | Hardware has 12 QPUs (24 GFLOPS FP32 peak @ 250 MHz) exposed as OpenGL ES 2.0 / OpenGL 2.1 via Mesa `vc4`, plus a hardware 2D compositor/scaler (HVS). **No GLES 3.0, no Vulkan, no compute shaders, no mainline OpenCL.** |
| **2. Can our existing `oxiterm-gpu` (`wgpu`) run on it?** | **No** | `oxiterm-gpu` (`wgpu 30.0.2`, `Limits::downlevel_webgl2_defaults()`) requires at least OpenGL ES 3.0 / GL 3.3 / Vulkan. VideoCore IV hardware tops out at GLES 2.0. |
| **3. Can OxiUI use VideoCore IV hardware at all?** | **Yes — via HVS DRM/KMS scanout (not 3D shaders)** | Best path on Pi 1 B+ is CPU rendering via `oxiterm-fb` + damage tracking (`U.Damage`) + `vc4` DRM/KMS double-buffered dumb buffers and HVS hardware planes/scaling. A bespoke GLES 2.0 sink is possible but not worth the complexity unless `8.Mem` proves CPU rasterization too slow. |
| **4. Can we run AI (small neural networks) on the QPUs?** | **Yes — technically proven, with major caveats** | Prior art (Idein, Pete Warden, `GPU_FFT`, `VC4CL`) achieves **~8 GFLOPS SGEMM** and **~95–120 ms/frame MobileNet v1/v2 (224×224, FP32)** on the same BCM2835 silicon (~10×–49× over the 700 MHz ARM1176 CPU, which has no NEON). |
| **5. Should we build a general OxiScript QPU compiler backend?** | **No — use a tiny host-kernel shim (`host:qpu`) for a demo instead** | Direct QPU compute requires **root**, has **no IOMMU** (a bug corrupts kernel RAM), **conflicts with `vc4-kms-v3d`** (needed by modern DSI displays), violates strict `P-9` float determinism (FTZ/non-IEEE rounding), and targets discontinued Pi 0–3 silicon. An isolated, host-owned SGEMM/GEMV capability (`host:qpu`) gives the full "AI on a 2014 Pi GPU in OxiScript" demo in ~1–2 weeks without polluting OxiScript's core compiler. |

> **Corrected by Claude (Opus), 2026-11-26:** three things in this table are off. (1) The ARM1176 **peak** is about 1.4 GFLOPS at 700 MHz (Idein: 2 GFLOPS at 1 GHz), so the peak asymmetry is about **17×**, not ~100× (A6). (2) Idein's MobileNet numbers (95 ms V2, 120 ms V1) are confirmed, but on a **Pi Zero with V3D at 300 MHz** (Pi 1 B+: 250 MHz) and with Idein's **private** libraries; "~49×" has no source (A5, A7). (3) The Pi 1 B+ is **not discontinued**: in production until at least January 2030 (A3).


**Bottom line:** VideoCore IV is a genuine 24 GFLOPS SIMD co-processor bolted next to a slow scalar 0.1–0.3 GFLOPS ARMv6 CPU. For **OxiUI**, stay on the CPU software sink (`oxiterm-fb`) backed by `vc4` DRM/KMS + HVS planes and add `U.Damage` if `8.Mem` shows full redraws are too slow (confirming owner decision [O-5](../roadmap/oxiui.md#o-5)). For **AI in OxiScript**, VideoCore IV *can* run real small neural networks (MLPs, tiny CNNs, keyword spotting, MobileNet-tiny, tiny LMs) dramatically faster than the ARM11 CPU, but only via a privileged, `@approx`, pre-assembled host capability (`host:qpu`)—never as an untrusted guest shader or a core 8.3d compiler target.

> **Corrected by Claude (Opus), 2026-11-26:** the KMS conflict is real for py-videocore and VC4CL (their own README and issues). Touch Display 2 needs full KMS; the original 7" display was historically firmware-driven, so a legacy-firmware `/dev/fb0` setup *might* leave V3D free, but Bookworm replaced the firmware framebuffer with KMS, so this must be tested on a spare SD card (A2, A4).


---

## 1. What VideoCore IV Offers (BCM2835 Hardware & Driver Stack)

The Raspberry Pi 1 Model B+ V1.2 uses the Broadcom **BCM2835** SoC: a single-core **ARM1176JZF-S** (ARMv6, 700 MHz, **VFPv2 scalar FPU, no NEON SIMD**) sharing 512 MB of 400 MHz LPDDR2 RAM with a **VideoCore IV** multimedia/GPU subsystem ([Raspberry Pi BCM2835 docs](https://www.raspberrypi.com/documentation/computers/processors.html#bcm2835); [Broadcom VideoCore IV 3D Architecture Reference Guide `VideoCoreIV-AG100-R`](https://docs.broadcom.com/doc/12358545)). Because the ARM1176 core achieves only roughly **0.1–0.3 GFLOPS** in scalar single-precision benchmarks (`[unverified]` exact Linpack on B+ @ 700 MHz; measure in `8.Mem`), the VideoCore IV 3D block represents a ~100× theoretical compute asymmetry on this board.

> **Corrected by Claude (Opus), 2026-11-26:** "~100× theoretical" is wrong: peak against peak is about 17× (Idein's BCM2835 table); 0.1-0.3 GFLOPS is an unmeasured sustained estimate (ledger A6).


### 1.1 The 3D Block (V3D) and QPUs

Per Broadcom's `VideoCoreIV-AG100-R` ([Broadcom AG100-R](https://docs.broadcom.com/doc/12358545)) and community ISA documentation ([`maazl/vc4asm` notes](https://github.com/maazl/vc4asm)):

* **QPU topology & clock:** **12 QPUs** (Quad Processing Units) organized into **3 slices × 4 QPUs/slice**, running at **250 MHz** stock on BCM2835 (overclockable to 300–400+ MHz in firmware config).
* **SIMD execution model:** Each QPU is a **16-way SIMD** vector processor built from **4 physical 32-bit lanes multiplexed over 4 clock cycles** (so one 16-element vector instruction completes every 4 cycles). Each QPU has **two dual-issue vector ALUs**—one ADD ALU and one MUL ALU—able to issue concurrently on the same 64-bit instruction word.
* **Peak theoretical GFLOPS:**
  $$12\text{ QPUs} \times 4\text{ physical lanes} \times 2\text{ FLOPs/cycle (ADD + MUL)} \times 250\text{ MHz} = \mathbf{24.0\text{ GFLOPS (FP32)}}$$
  (Up to 28.8 GFLOPS at 300 MHz; [Idein `py-videocore6` README comparison](https://github.com/Idein/py-videocore6)).
* **Numeric precision & ALU quirks (critical for OxiScript `P-9`):**
  * **FP32 only (no FP64):** Single-precision 32-bit float on both ADD and MUL ALUs. **Not fully IEEE-754 compliant**: denormals/subnormals are flushed to zero (FTZ), NaN/Inf propagation is incomplete, and ALU rounding defaults to simplified/round-to-zero behavior rather than strict IEEE round-to-nearest-even across all paths ([Broadcom AG100-R](https://docs.broadcom.com/doc/12358545); [VC4CL `clinfo` capabilities report](https://github.com/doe300/VC4CL)).

> **Corrected by Claude (Opus), 2026-11-26:** the AG100 guide says nothing about IEEE compliance, denormals or rounding. Round-to-zero is confirmed by VC4CL's tests (`vc4cl_config.h`: `CL_FP_ROUND_TO_ZERO`, no `CL_FP_DENORM`, no `CL_FP_INF_NAN`); FTZ and NaN behaviour are implied by the missing flags, not documented (ledger B7). The P-9 conclusion stands.

  * **Integers:** 32-bit integer add/sub/bitwise/shifts on the ADD ALU, **24-bit integer multiply (`mul24`)** on the MUL ALU (no single-cycle 32×32→32 int multiply), and 8-bit saturating add/sub/mul helpers. **No native `int8×4` dot-product instruction.**
  * **Hardware pack/unpack on Regfile-A (and `r4`):** Free on-the-fly unpacking of 16-bit halves (`16a`/`16b`: **FP16→FP32** for float ops, signed `i16→i32` for int ops) and 8-bit bytes (`8a`–`8d`: unsigned `u8→[0.0, 1.0]` FP32 or `u8→u32`) as operands enter the ALU ([Broadcom AG100-R](https://docs.broadcom.com/doc/12358545)).
  * **Special Function Unit (SFU):** Shared per-slice `recip` ($1/x$), `recipsqrt` ($1/\sqrt{x}$), `exp2`, `log2` with a fixed 3-instruction pipeline latency into accumulator `r4` (hazard if `r4` is touched on instructions $N+1, N+2$; approximate graphics precision `[unverified exact ULP bound]`).
* **On-chip memory & DMA:**
  * **Registers:** Two 32-entry 16-wide register files per QPU (`Regfile-A` and `Regfile-B`) plus 6 accumulators (`r0`–`r5`) — 4 KB of registers per QPU (48 KB across 12 QPUs).
  * **VPM (Vertex Pipe Memory):** **4 KB total** (64 rows × 16 columns × 32-bit) shared across all QPUs, backed by VCD/VDW DMA engines for 2D block transfers to/from main RAM.

> **Corrected by Claude (Opus), 2026-11-26:** AG100: the VPM is at least 8 KB and up to 16 KB; 64×16×32-bit (4 KB) is the maximum QPU general-purpose window (ledger B13).

  * **TMU (Texture and Memory Lookup Unit):** 2 TMUs per slice with L2 texture cache; GPGPU kernels abuse TMU general-memory lookups for cached gather reads while using VPM DMA for coalesced writes ([Broadcom AG100-R](https://docs.broadcom.com/doc/12358545); [Pete Warden 2014 write-up](https://petewarden.com/2014/08/07/how-to-optimize-raspberry-pi-code-using-its-gpu/)).
* **No IOMMU / MMU:** The V3D block accesses physical bus memory directly with **zero hardware memory protection** ([Linux kernel `vc4` DRM docs](https://docs.kernel.org/gpu/vc4.html)).

### 1.2 Open Graphics & Compute APIs on VideoCore IV

| API / Feature | Status on VideoCore IV (BCM2835) | Source / Notes |
|---|---|---|
| **OpenGL ES 2.0** | **Supported** (open-source Mesa `vc4` Gallium driver + kernel `vc4-kms-v3d`) | Nearly conformant; primary hardware-accelerated API ([Mesa `vc4` docs](https://docs.mesa3d.org/drivers/vc4.html)). |
| **OpenGL 2.1** | **Partial** (Mesa `vc4`) | Missing hardware features (e.g. 32-bit index buffers, occlusion queries) require software workarounds ([Mesa `vc4` docs](https://docs.mesa3d.org/drivers/vc4.html)). |
| **OpenGL ES 3.0 / 3.1** | **No** | Hardware lacks silicon support (no MRTs, no 3D textures, no UBOs/SSBOs, no compute pipeline). |
| **Vulkan (`v3dv`)** | **No** | **Confirmed:** Mesa's `v3d` (GLES 3.1) and `v3dv` (Vulkan 1.0–1.3) drivers target **VideoCore VI / VII** (Raspberry Pi 4 and Pi 5, V3D 4.2/7.1 with hardware MMU), **not VideoCore IV** ([Mesa `v3d` docs](https://docs.mesa3d.org/drivers/v3d.html); [Mesa `vc4` docs](https://docs.mesa3d.org/drivers/vc4.html)). |
| **Compute Shaders (GL/VK)** | **No** | Linux `vc4` DRM explicitly rejects arbitrary compute shaders because VideoCore IV has no IOMMU; `vc4_validate_shaders.c` validates only graphics command lists (`DRM_IOCTL_VC4_SUBMIT_CL`) ([Linux kernel `vc4` docs](https://docs.kernel.org/gpu/vc4.html)). |
| **OpenCL** | **No official / mainline support** | Only via third-party [`doe300/VC4CL`](https://github.com/doe300/VC4CL) (OpenCL 1.2 Embedded Profile), bypassing Mesa via mailbox/register access as root (Section 3). |

> **Corrected by Claude (Opus), 2026-11-26:** OpenGL 2.1: Mesa truncates 4-byte index buffers to 2 bytes (wrong above 65535) rather than working around it; occlusion queries and line/point polygon modes are missing (ledger C2).


### 1.3 Other VideoCore IV Hardware Blocks and What Each Means for Us

| Hardware Block | Capabilities on BCM2835 | Linux Interface | Relevance to OxiUI / OxiScript |
|---|---|---|---|
| **VPU** (Video Processing Unit) | Dual-core 250 MHz scalar + 16×16 integer vector processor; boots the SoC and runs Broadcom's closed ThreadX RTOS firmware blob (`start.elf`) ([`hermanhermitage/videocoreiv`](https://github.com/hermanhermitage/videocoreiv)). | Closed firmware; ARM communicates via Mailbox (`/dev/vcio`). Reverse-engineered GCC/binutils (`vc4-toolchain`) exist for bare metal only. | **Not usable.** Taking over or injecting code into the VPU conflicts with the running firmware managing clocks, power, camera, and display. |
| **HVS** (Hardware Video Scaler) | Dedicated 2D display composition engine: fetches multiple memory layers (**DRM planes**), performs on-the-fly **scaling, alpha blending, cropping, and YUV↔RGB conversion**, and feeds Pixel Valves (HDMI, DSI, DPI) ([Linux kernel `vc4` docs](https://docs.kernel.org/gpu/vc4.html)). | Mainline DRM/KMS (`vc4-kms-v3d`, `/dev/dri/card0`). | **High value for OxiUI / OxiDE.** Free hardware compositing of UI planes, hardware cursor, hardware upscaling of lower-res UI buffers, and zero-copy camera overlay planes. |
| **ISP** (Image Signal Processor) | Dedicated CSI-2 camera pipeline: debayering, auto-exposure/white-balance, color conversion, and **hardware downscaling** into multiple output streams simultaneously. | Mainline V4L2 (`bcm2835-isp`) + `libcamera` ([Raspberry Pi camera/V4L2 docs](https://www.raspberrypi.com/documentation/computers/camera_software.html)). | **High value for camera-AI demos.** Can deliver pre-scaled e.g. 96×96 or 224×224 RGB frames directly into CMA memory for an AI model without burning ARM11 CPU cycles on resize/demosaic. |
| **H.264 Codec** | Hardware 1080p30 H.264 video decode and encode ([Raspberry Pi BCM2835 specs](https://www.raspberrypi.com/documentation/computers/processors.html#bcm2835)). | V4L2 M2M (`bcm2835-codec`). | **Low value for UI/AI.** Useful only if OxiUI plays back or streams H.264 video (`9.2` Tier-1 `<scene>` video client or camera streaming); cannot accelerate DisplayList rendering or neural nets. |

> **Corrected by Claude (Opus), 2026-11-26:** ISP and H.264 rows: `bcm2835-isp` and `bcm2835-codec` are Raspberry Pi kernel drivers over the firmware VCHIQ/MMAL services, not mainline; they do not use V3D (ledger C10).


---

## 2. OxiUI Graphics Acceleration on Raspberry Pi 1 B+

### 2.1 Can Our Existing `oxiterm-gpu` (`wgpu`) Run on It?

**No.** Direct inspection of `oxiterm-gpu/src/context.rs:93-95,195-199` confirms that `GpuContextOptions::default()` requests `wgpu::Limits::downlevel_webgl2_defaults()`, targeting `wgpu 30.0.2`'s minimum downlevel floor: **OpenGL ES 3.0 / WebGL2 / Desktop OpenGL 3.3 / Vulkan 1.0+** ([`wgpu` README / docs](https://docs.rs/wgpu/latest/wgpu/)). Because Mesa `vc4` only exposes **OpenGL ES 2.0** (and partial OpenGL 2.1), `wgpu::Instance::request_adapter` cannot create a hardware adapter on VideoCore IV.

Falling back to software Vulkan (`lavapipe`) or software GL (`llvmpipe`) on a 700 MHz single-core ARMv6 without NEON is also unviable: LLVM JIT compilation consumes tens of megabytes of RAM (out of 512 MB shared) and renders at sub-1 FPS (`[unverified]` exact FPS; `vulkan-acceleration.md` §8 explicitly restricts `lavapipe` to CI and desktop hosts). This directly confirms owner decision **[O-5](../roadmap/oxiui.md#o-5)**: hardware below the `wgpu` floor (Pi 0–3) stays on the CPU/framebuffer sink (`oxiterm-fb`).

### 2.2 Minimum Paths to Display OxiUI on Pi 1 B+

| Path | How It Works | Pros | Cons | Verdict |
|---|---|---|---|---|
| **A. `oxiterm-fb` on `vc4` DRM/KMS + HVS** *(already exists in Stage 7.2)* | CPU rasterizes `DisplayList` into RGBA/XRGB buffer; `vc4` DRM/KMS scans out via dumb buffers + HVS planes (`oxiterm-fb/src/device/drm.rs`). | Already built and tested; zero Mesa/EGL RSS overhead (~2–15 MB target); works natively on DSI/HDMI under `vc4-kms-v3d`; deterministic pixels. | Full-screen CPU redraw + `tiny-skia` on ARMv6 without NEON can be slow at high resolutions unless paired with damage tracking and fast-path blitting. | **Recommended primary path.** |
| **B. New dedicated GLES 2.0 sink (`glow` + `khronos-egl` + `gbm`)** | Hand-written GLSL ES 1.00 vertex/fragment shaders (SDF rects + glyph atlas quads, Alacritty-style), rendered straight onto GBM/DRM buffers via Mesa `vc4`. | Hardware-accelerated full-screen fills, alpha blending, smooth scrolling, and rounded borders at 60 FPS even at 1080p. | Separate shader pipeline to maintain (GLSL ES 1.00 lacks UBOs, integer bit-ops, and core instancing); pulls Mesa/EGL into RSS (~15–30 MB `[unverified]`); **cannot coexist with mailbox QPU AI** because Mesa `vc4` actively uses V3D. | **Do not build unless `8.Mem` proves Path A + `U.Damage` misses frame-time targets.** |

> **Corrected by Claude (Opus), 2026-11-26:** path A: `drm.rs` today allocates one dumb buffer and calls `SETCRTC` on the primary plane; no overlay plane, atomic or page-flip API is used yet (ledger C15). Frame sizes depend on the display bought: Touch Display 2 is 720×1280 (3.69 MB per XRGB frame).


### 2.3 What Gives the Best UI Performance for OxiUI on This Board?

Reading `oxiterm-fb/src/raster.rs:85-97` and `oxiterm-fb/src/device/drm.rs:331-372` reveals four concrete bottlenecks in the current Stage 7.2 implementation—and the exact optimizations that maximize Pi 1 B+ UI speed without writing a GLES 2.0 engine:

1. **Render only on change (`U.Idle`):** Today `oxiterm-fb` is a one-shot/full-redraw sink and the event loop wakes every 16–1000 ms (`oxiui.md` `U.Idle`). For HMIs, dashboards, and TUI-derived pages, 95%+ of frames have zero visual change; sleeping until an input/state patch arrives cuts idle CPU to ~0%.
2. **Damage tracking (`U.Damage`) — highest CPU win:** Currently `Rasterizer::render()` clears the entire `Surface` (`surface.clear(DEFAULT_BG)`) and repaints every item, and `drm.rs` converts the entire RGBA `Surface` into XRGB8888 (`pixel::convert_image`). On BCM2835's 400 MHz LPDDR2 (~3.2 GB/s theoretical, much less per single ARM11 core with 16 KB L1 / 128 KB L2 cache), touching 800×480×4 B = **1.54 MB** (or 1920×1080×4 B = **8.29 MB**) twice per frame is expensive. Diffing consecutive `DisplayList`s into dirty rectangles so `oxiterm-fb` only rasterizes and blits changed rows/boxes shrinks per-keystroke work by **10×–50×** (for example, updating one 20 px text line on 800×480 touches 64 KB instead of 1.54 MB).
   * *Note on DRM `FB_DAMAGE_CLIPS` / `DRM_IOCTL_MODE_DIRTYFB`:* On standard video-mode HDMI/DSI panels driven by `vc4` HVS, the hardware continuously scans out the whole buffer anyway, so the main win is saving CPU raster + RAM write bandwidth. On command-mode DSI/SPI/e-ink displays, attaching the atomic `FB_DAMAGE_CLIPS` property also cuts panel bus transfers ([Linux DRM damage docs](https://docs.kernel.org/gpu/drm-kms-helpers.html)).
3. **Double-buffered DRM dumb buffers + `PAGE_FLIP` (replacing single-buffer `SETCRTC`):** `oxiterm-fb/src/device/drm.rs` currently allocates one dumb buffer and calls `SETCRTC`. For `7.2b` (`oxide run`), allocate **two** dumb buffers (front/back) and swap with `DRM_IOCTL_MODE_PAGE_FLIP` (synchronizing damage to the back buffer or flipping full dirty frames) for tear-free updates, and render directly into `XRGB8888` (or `RGB565` on 16-bit panels) to eliminate the separate RGBA→XRGB full-frame copy.
4. **HVS DRM planes (hardware composition & scaling) + lightweight raster fast path:**
   * `vc4` exposes multiple hardware overlay planes through the HVS ([Linux kernel `vc4` docs](https://docs.kernel.org/gpu/vc4.html)). Putting a static UI layer on the primary plane, a hardware cursor or moving overlay on a second plane, and (for camera-AI demos) a zero-copy `bcm2835-isp` DMA-BUF preview on an underlay/overlay plane lets the HVS blend and scale them at 60 FPS with **0% ARM11 CPU**.
   * `oxiterm-fb/src/raster.rs` already uses exact integer blits for square fills and cached `swash` alpha glyphs, reserving `tiny-skia` for rounded corners, borders, and `Path` items. Because `tiny-skia` is 100–300% slower than Skia on ARM ([`tiny-skia` README](https://github.com/RazrFalcon/tiny-skia), cited in `vulkan-acceleration.md` §3.3) and ARMv6 lacks NEON, keeping `OxiDE` themes on Pi 1 mostly square/integer-filled (or caching static rounded panels) avoids `tiny-skia`'s per-frame overhead.

> **Corrected by Claude (Opus), 2026-11-26:** the tiny-skia README's "100-300% slower on ARM" refers to AArch64 with NEON; on ARMv6 tiny-skia runs scalar, so expect worse (ledger C20).


---

## 3. General-Purpose Compute on VideoCore IV QPUs: Prior Art

### 3.1 How QPU Compute Works — and Why It Conflicts with `vc4-kms-v3d`

Because VideoCore IV has **no IOMMU/MMU**, the mainline Linux kernel `vc4` DRM driver (`vc4-kms-v3d`) **deliberately refuses** arbitrary compute shader submissions: its in-kernel validator (`vc4_validate_shaders.c`) only accepts immutable graphics shaders submitted through `DRM_IOCTL_VC4_SUBMIT_CL` and rejects unconstrained VPM DMA writes or arbitrary TMU memory accesses that would allow any unprivileged process to overwrite kernel RAM ([Linux kernel `vc4` DRM docs](https://docs.kernel.org/gpu/vc4.html)).

Consequently, **every real-world GPGPU project on VideoCore IV bypasses Mesa and Linux DRM**:

1. **Memory allocation & power control via Mailbox (`/dev/vcio`):** User-space sends VideoCore property mailbox tags (`MBOX_TAG_ENABLE_QPU`, `MBOX_TAG_ALLOCATE_MEMORY`, `MBOX_TAG_LOCK_MEMORY`) through `/dev/vcio` (or legacy `/dev/mem` + `mbox`) to power on the V3D block and allocate physically contiguous, bus-addressed memory buffers ([Andrew Holme `GPU_FFT`](http://www.aholme.co.uk/GPU_FFT/Main.htm); [`doe300/VC4CL`](https://github.com/doe300/VC4CL)).
2. **Job dispatch:** Either via the firmware mailbox (`MBOX_TAG_EXECUTE_QPU`) or by directly memory-mapping `/dev/mem` to poke the V3D QPU scheduler registers (`V3D_SRQPC` / `V3D_SRQUA`).
3. **Root privilege required:** Direct physical memory mapping (`/dev/mem`) and unguarded QPU execution require **`root`** (or `CAP_SYS_RAWIO` / custom device permissions), because a single out-of-bounds VPM DMA write corrupts arbitrary physical RAM.
4. **Hard conflict with `vc4-kms-v3d` (Full KMS):** When `dtoverlay=vc4-kms-v3d` is active in `/boot/firmware/config.txt` (the default on modern Raspberry Pi OS Bookworm+), the Linux kernel `vc4` DRM driver claims exclusive ownership of the V3D block and its power domain. Attempting mailbox QPU enable/execution simultaneously fails (`Unable to enable V3D/QPUs`) or causes kernel hangs/panics ([`GPU_FFT` / Raspberry Pi KMS conflict reports](https://github.com/raspberrypi/linux/issues); [`doe300/VC4CL` KMS notes](https://github.com/doe300/VC4CL)). Historically users resolved this by commenting out `dtoverlay=vc4-kms-v3d` (reverting to legacy firmware display / headless mode) or using deprecated `vc4-fkms-v3d`—**which directly clashes with modern KMS DSI panel drivers** (see Section 5.3).

> **Corrected by Claude (Opus), 2026-11-26:** the cited sources (a generic issues page, the VC4CL README) do not say this. Primary evidence: py-videocore's README ("does not work with the CPU-side OpenGL graphics stack"; comment out **both** `vc4-kms-v3d` and `vc4-fkms-v3d`, so fkms is no workaround there) and VC4CL issues #60 and #100 ("not compatible with the 'full kms' OpenGL driver"). The real GPU_FFT error text is "Unable to enable V3D. Please check your firmware is up to date." GPU_FFT's guide says it runs alongside the old firmware OpenGL on Pi 1. The `vc4` kernel driver treats V3D as an optional component, so the `&v3d`-disabled experiment is plausible but untested (ledger A2, A4).


### 3.2 Prior-Art Projects Comparison

Per OxiUI/OxiScript's permissive-only dependency rule ([O-14](../roadmap/oxiui.md#o-14), [P-11](../roadmap/oxiscript.md#p-11)), **only MIT / Apache-2.0 / BSD / Zlib projects** may be reused as code; GPL projects below are marked **reference only**.

| Project | Target Hardware | Achieved Numbers / Results | QPU Access, Root & KMS Conflict | Licence & Usability for Us | Maintenance Status |
|---|---|---|---|---|---|
| **Andrew Holme's `GPU_FFT` (`hello_fft`)** ([site](http://www.aholme.co.uk/GPU_FFT/Main.htm), [RPi firmware repo](https://github.com/raspberrypi/userland/tree/master/host_applications/linux/apps/hello_pi/hello_fft)) | VideoCore IV (Pi 0–3) | **~10× faster than Pi 1 ARM11 FFTW**: 256-pt `0.033 ms` vs `0.092 ms`; 4096-pt `0.25 ms` vs `3.0 ms`; 65,536-pt `7.0 ms` vs `83 ms`; 4M-pt `786 ms` (1D & 2D FFTs, ~1 ppm RMS error using 8 QPUs). | Mailbox `/dev/vcio` + V3D regs; requires **root**; **conflicts with `vc4-kms-v3d`**. | **BSD-3-Clause** — **Usable** (note: third-party wrappers are sometimes GPLv2; use upstream only). | Mature / frozen (upstream `raspberrypi/userland` archived). |
| **`py-videocore`** ([`nineties/py-videocore`](https://github.com/nineties/py-videocore), [`Idein/py-videocore`](https://github.com/Idein/py-videocore)) | VideoCore IV (Pi 0–3) | Python internal DSL QPU assembler; achieved **~8 GFLOPS SGEMM** on VideoCore IV (1/3 of 24 GFLOPS theoretical peak, memory-bound). | Mailbox `/dev/vcio`; requires **root**; **conflicts with `vc4-kms-v3d`**. | **MIT** — **Usable** (excellent clean ISA encoding reference). | Legacy / largely unmaintained (superseded by `py-videocore6`/`7`). |
| **`py-videocore6`** ([`Idein/py-videocore6`](https://github.com/Idein/py-videocore6)) | **VideoCore VI (Pi 4 only — NOT Pi 1!)** | ~**3.6–3.9 GFLOPS** SGEMM example out of 32 GFLOPS peak @ 500 MHz (`py-videocore7` on Pi 5 hits ~21.3 / 76.8 GFLOPS). | Linux **`v3d` DRM driver** (`/dev/dri/card0` CSD ioctl — safe because VC6 has an MMU!). | **GPL-2.0** — **Reference only (NOT usable in our repo)**; also wrong GPU generation. | Maintained/mature for Pi 4. |
| **`vc4asm`** ([`maazl/vc4asm`](https://github.com/maazl/vc4asm)) | VideoCore IV (Pi 0–3) | Full macro assembler & disassembler with pipeline-hazard / constraint checking; outputs binary, ELF, or C headers. | Offline assembler (output run via mailbox/V3D). | **GPL-3.0** — **Reference only (NOT usable as a linked dependency or vendored code)**. | Legacy / mature. |
| **Idein NN stack (`qmkl`, `nnoir`, Actcast)** ([`Idein/qmkl`](https://github.com/Idein/qmkl), [Actcast](https://actcast.io/), [ASCII.jp / Idein talks](https://speakerdeck.com/)) | VideoCore IV (Pi Zero / Pi 1–3) | Proved **MobileNet v1 / v2 (224×224, FP32)** in **~95–120 ms/frame (~8–10 FPS)** on Pi Zero (~**49× faster** than unoptimized ARM11 CPU) with mathematically equivalent FP32 execution; `qmkl` provides QPU `cblas_sgemm`. | Mailbox `/dev/vcio` (legacy firmware stack); **root** / container privilege; **conflicts with `vc4-kms-v3d`**. | `qmkl`: **verify `LICENSE` file before any use** `[unverified — mixed/GPL-adjacent tooling history]`; `qmkl6` (Pi 4) is BSD-3-Clause; Actcast compiler is **proprietary**. Treat as **reference / proof of feasibility**. | Commercial Actcast stack active; OSS `qmkl` legacy. |
| **Pete Warden's `DeepBeliefSDK` / `pi-gemm` (2014)** ([`jetpacapp/DeepBeliefSDK`](https://github.com/jetpacapp/DeepBeliefSDK), [blog](https://petewarden.com/2014/08/07/how-to-optimize-raspberry-pi-code-using-its-gpu/)) | VideoCore IV (Pi 1) | Cut CNN image classification on Pi 1 from **~20 s** (ARM CPU) → `6 s` → **3 s/frame** via QPU `pi-gemm` + TMU read pipelining; GEMM test dropped from ~8,000 ms (ATLAS CPU) to ~500 ms (overclocked Pi 1 QPU). | Mailbox + custom `qpu-asm` + VPM/TMU; requires **root**; **conflicts with `vc4-kms-v3d`**. | **BSD-3-Clause** (`pi-gemm` & SDK QPU code) — **Usable** as reference/code study. | Historical / archived (2014–2015). |
| **Mesa `vc4` QPU compiler** ([Mesa `vc4` docs](https://docs.mesa3d.org/drivers/vc4.html), `src/gallium/drivers/vc4/vc4_qir*`, `vc4_qpu*`) | VideoCore IV (Pi 0–3) | Production NIR → QIR → QPU shader compiler + register allocator + instruction scheduler for OpenGL ES 2.0 graphics. | Kernel `DRM_IOCTL_VC4_SUBMIT_CL` + `vc4_validate_shaders.c` (unprivileged, safe, **graphics-only — no compute**). | **MIT** — **Usable** as permissive ISA/scheduling reference. | Actively maintained in Mesa. |
| **`QPULib`** ([`mn416/QPULib`](https://github.com/mn416/QPULib)) & fork **`V3DLib`** ([`wimrijnders/V3DLib`](https://github.com/wimrijnders/V3DLib)) | VideoCore IV (`QPULib`); VC4 + VC6 (`V3DLib`) | C++ embedded DSL compiling vector kernels to QPU machine code at runtime (heat-map / convolution / math benchmarks; bounded by RAM bandwidth). | Mailbox `/dev/vcio` + V3D regs (VC4); requires **root**; **conflicts with `vc4-kms-v3d`**. | **MIT** — **Usable** (cleanest C++ reference for QPU codegen & mailbox runner). | `QPULib` explicitly **archived/unmaintained**; `V3DLib` legacy/low activity. |
| **`VC4CL` / `VC4C` / `VC4CLStdLib`** ([`doe300/VC4CL`](https://github.com/doe300/VC4CL), [`doe300/VC4C`](https://github.com/doe300/VC4C)) | VideoCore IV (Pi 0–3 only) | Full **OpenCL 1.2 Embedded Profile** runtime + LLVM/SPIR-V→QPU compiler; strongly outperforms PoCL CPU on `float16` ALU benchmarks; real apps bottlenecked by shared LPDDR2 bandwidth. | Mailbox/V3D register pokes (`REGISTER_POKE_KERNELS`) + CMA/VCSM; requires **root**; **conflicts with Full KMS (`vc4-kms-v3d`)** in default register-poke mode. | **MIT** (`VC4CL`, `VC4C`, `VC4CLStdLib`) — **Usable** permissively, though heavy C++/LLVM dependency. | Legacy / minimal maintenance. |

> **Corrected by Claude (Opus), 2026-11-26:** GPU_FFT: every number matches `gpu_fft.txt` (release 3.0, Pi 1, ms), but "~10×" holds from 1K points up; at 256 points, batch 1, it is 2.8× (ledger D3). The KMS conflict for GPU_FFT has no primary source (D6). GPU_FFT `dlopen`s `libbcm_host.so` from the archived legacy userland: check that the installed OS ships it.

> **Corrected by Claude (Opus), 2026-11-26:** py-videocore: README "Achieved 8GFlops with sgemm", board and clock not stated (Idein measured overclocked at 400 MHz early on); "1/3 of peak, memory-bound" is an inference (ledger D8).

> **Corrected by Claude (Opus), 2026-11-26:** Idein: `Idein/qmkl` is **BSD-3-Clause** (LICENSE: "Modified (3-Clause) BSD License"), usable under O-14 (ledger A8). The MobileNet results (95 ms V2, 120 ms V1 on Pi 0; FP32, "mathematically equivalent") are confirmed by the DLLab deck (https://speakerdeck.com/nineties/ideinfalseshao-jie-at-dllab-tui-lun-naito) and the IIJlab 2019 slides (https://seminar-materials.iijlab.net/iijlab-seminar/iijlab-seminar-20191015.pdf), but used Idein's private libraries; "~49×" is unsourced (A7).

> **Corrected by Claude (Opus), 2026-11-26:** Pete Warden: DeepBeliefSDK README says about **5 s on a stock Pi**, 3 s only "with a boosted GPU clock rate"; the 500 ms vs 8 000 ms GEMM was "on my overclocked Pi". Licences BSD-3 confirmed (ledger D20-D22).

> **Corrected by Claude (Opus), 2026-11-26:** QPULib is MIT except `Lib/VideoCore/Mailbox.cpp/.h` (Broadcom BSD-3); it is not archived on GitHub, the README says "no longer under development". V3DLib's MIT also excludes its mailbox files, `catch.hpp` and a vendored `mesa/` subtree (ledger D25, D26).

> **Corrected by Claude (Opus), 2026-11-26:** VC4CL: the PoCL comparison has no source (D28); VC4CL needs root **or** the `video` group depending on the chosen interfaces (C7).


---

## 4. Running AI (Small Neural Networks) on VideoCore IV

### 4.1 Realistic Small Models and Latency Expectations on Pi 1 Model B+

On the Pi 1 B+, the single-core **700 MHz ARM1176** has no NEON (~0.1–0.3 GFLOPS scalar VFPv2), while the **12 QPUs @ 250 MHz** deliver **24 GFLOPS peak** and **~4–8 GFLOPS sustained** on well-tiled FP32 SGEMM (`py-videocore` achieved 8 GFLOPS; Pete Warden achieved ~16× over ATLAS). Shared **400 MHz LPDDR2** has a **3.2 GB/s theoretical ceiling** (real usable ~1.0–1.8 GB/s `[unverified]`), which bounds batch-1 GEMV/tiny-LM workloads while leaving compute-heavy conv/batched-MLP layers strongly QPU-accelerated.

| Model Class | Size (Params / RAM) | Compute per Call | ARM1176 @ 700 MHz (Scalar VFPv2) | VideoCore IV 12 QPUs @ 250 MHz | Bottleneck & Verdict |
|---|---|---|---|---|---|
| **Tiny MLP (MNIST 784→128→10, Course Level 5)** | ~102K params (~0.4 MB FP32) | ~204K FLOPs (batch 1); ~13M FLOPs (batch 64 fwd+bwd) | Batch-1 infer: **~1–3 ms** `[estimate]`; 1 epoch (60k imgs) train: **~20–60 s** `[estimate]` | Batch-1 infer: **~0.2–0.5 ms** (dispatch overhead dominates); batched train step (64): **~5–15 ms** → **full epoch in ~2–8 s** `[estimate]` | **Great showcase** for live training & batched inference; single-sample inference is already fast on CPU. |
| **Small CNN (LeNet-5 / CIFAR-10 / gesture 64×64)** | ~50K–300K params (~0.2–1.2 MB) | ~1–5M MACs (~2–10M FLOPs) | **~30–150 ms** `[estimate]` | **~1–5 ms** (100+ FPS compute-side) `[estimate]` | **Ideal sweet spot** for real-time camera/touch demos (e.g. CSI camera + ISP 64×64 downscale). |
| **Keyword Spotting (DS-CNN-S / 1D-CNN on MFCCs)** | ~25K–100K params (< 1 MB) | ~3–7M MACs (~6–14M FLOPs) | **~40–200 ms** `[estimate]` | **~2–10 ms** `[estimate]` | **Effortlessly real-time** (standard KWS runs every 50–100 ms); easily fits in RAM. |
| **MobileNet-tiny (V1 0.25× @ 96×96 or 128×128)** | ~0.47M params (~1.9 MB FP32) | ~4–14M MACs (~8–28M FLOPs) | **~100–400 ms** `[estimate]` | **~10–30 ms (~30+ FPS)** `[estimate]` | **Very practical** for real-time vision (face/object/person detection) if depthwise conv is fused or tuned. |
| **MobileNet v1 / v2 1.0× (224×224)** | ~3.5–4.2M params (~14–17 MB FP32) | ~300–569M MACs (~0.6–1.14 GFLOPs) | **~2–6 s** (~0.17–0.4 FPS on Pi Zero/Pi 1; [Idein/SO reports](https://github.com/Idein/py-videocore)) | **~95–120 ms (~8–10 FPS)** — **proven by Idein in FP32 on Pi Zero** ([Idein slides/ASCII.jp](https://actcast.io/)) | **Upper bound of real-time vision** on BCM2835; requires heavily tuned QPU kernels (Idein-grade engineering). |
| **Tiny Language Model (1M-param Transformer, Course Level 5)** | 1M params (4 MB FP32 / 2 MB FP16) | ~2M FLOPs/token (batch-1 GEMV) | **~10–30 ms/tok** (~30–100 tok/s? Likely **~5–20 tok/s** with cache misses `[estimate]`) | **~4–15 ms/tok (~65–250 tok/s ceiling; ~20–60 tok/s realistic)** `[estimate]` | **Memory-bandwidth bound** (4 MB / 1 GB/s = 4 ms floor). Packing weights as **FP16** doubles speed! |

*(Note: Pi Zero runs the same BCM2835 V3D/QPU @ 250 MHz as Pi 1 B+, with ARM11 at 1 GHz vs Pi 1 B+'s 700 MHz—so QPU compute numbers transfer directly while Pi 1 B+ CPU-side overhead is ~1.4× slower than Pi Zero.)*

> **Corrected by Claude (Opus), 2026-11-26:** **wrong**: by default Pi Zero runs V3D at 300 MHz and SDRAM at 450 MHz, Pi 1 B+ at 250 and 400 (`config.txt` defaults). Pi Zero QPU numbers are about 17% too optimistic for this board (ledger A5). Every row of the table above is an `[estimate]` (E2).


### 4.2 INT8 vs FP32 on VideoCore IV QPUs (Crucial Hardware Insight)

Unlike modern NPUs or the ESP32-S3's Xtensa PIE (`ee.vmulas.s8.accx` in `oxiscript-simd-gpu-design.md` §1.3), **VideoCore IV QPUs do NOT have an `int8` dot-product or high-throughput `int8` MAC unit**:
* Integer multiplication is **24-bit (`mul24`)** at the same 16-way issue rate as **FP32 `fmul`**, so computing in `int8` gives **zero ALU throughput gain** over `fp32`—and adds requantization overhead.

> **Corrected by Claude (Opus), 2026-11-26:** the MUL ALU does have 8-bit vector ops on 4 bytes per lane (`v8muld` etc.), but they treat bytes as [0, 1.0] colour values with no wide accumulator, so they do not help int8 dot products; the FP32-compute conclusion stands (ledger A10).

* **Best strategy on VideoCore IV:**
  1. **Compute and accumulate in FP32** (gets full 24 GFLOPS dual-issue `fmul` + `fadd` throughput and zero quantization accuracy drop—exactly why Idein runs MobileNet in FP32!).
  2. **Optionally store weights in FP16 (2× smaller) or U8/INT8 (4× smaller)** for bandwidth-bound layers (GEMV / Tiny LM / large 1×1 convs), and use the QPU **Regfile-A hardware unpack (`16a`/`16b` FP16→FP32 or `8a`–`8d` U8→FP32)** to decompress weights into FP32 registers at zero extra ALU cost ([Broadcom AG100-R](https://docs.broadcom.com/doc/12358545)).

### 4.3 Memory Budget on 512 MB Shared LPDDR2

All subsystems share one 512 MB pool:
* Linux kernel + userspace services (headless / OxiDE console): ~40–80 MB `[unverified — measure on board]`.
* GPU memory split: under `vc4-kms-v3d`, **CMA** dynamically reserves typically **64–128 MB** (shared with kernel when idle, allocated for DRM framebuffers and `bcm2835-isp` camera buffers); under legacy firmware (`vc4-kms-v3d` disabled for mailbox QPU), static `gpu_mem=64` or `128` carves RAM permanently away from Linux.
* Display double-buffering (e.g. 800×480×4 B × 2): **~3.1 MB** (or ~16.6 MB at 1080p).
* OxiUI + OxiScript native process (`w2c2`, target [O-6](../roadmap/oxiui.md#o-6)): **~10–25 MB RSS**.
* **Remaining headroom for AI models & scratchpads:** **~250–350 MB**—vastly more than needed for MLPs (< 1 MB), KWS (< 2 MB), MobileNetV1/V2 (~20–40 MB weights + activations), or 1M–10M-param tiny LMs (~5–40 MB). The tight constraint is **never** fitting a small model in Rust/OxiScript; it is avoiding Python/PyTorch/TensorFlow runtimes (~150–300 MB RSS overhead) and avoiding swapping to SD card.

### 4.4 How This Plugs Into Our OxiScript Roadmap (`8.3a–e`, `8.Lab`, `D-20`, `D-27`, `D-28`)

Our roadmap cleanly separates targets (`micro` < `device` < `web` < `server`, [D-28](../roadmap/oxiscript.md#d-28)), places Pi 1 B+ in `--target device` ([D-20](../roadmap/oxiscript.md#d-20), [8.Mem](../roadmap/oxiscript.md#s-8-mem)), and builds GPU compute around **OxiIR → WGSL** on `wgpu` ([8.3c–e](../roadmap/oxiscript.md#s-8-3c), `oxiscript-simd-gpu-design.md`). Where does VideoCore IV fit?

1. **Do NOT add an OxiIR → VideoCore IV QPU backend to Stage `8.3d`.** VideoCore IV is a discontinued, quirky VLIW-like architecture (dual A/B register-file bank conflicts, 3-cycle SFU/branch delay slots, 4 KB shared VPM 2D DMA, no IOMMU). Writing and BAD-ACTOR-verifying a general compiler for it would consume months for zero benefit on Pi 4/5, desktops, browsers, or MCUs.
2. **Plug in as a tiny host capability (`host:qpu` / `host:ml`) under `std/tensor` (Stage `8.3e`) — mirroring ESP32-S3 PIE (`esp-nn`).**
   `oxiscript-simd-gpu-design.md` §1.3 & §6.3 already established the exact pattern for proprietary low-end accelerators: keep `std/tensor` written once in OxiScript (`Resident<f32[]>` + 7 kernel families), and let `--target micro` delegate `matmul`/`conv2d` to Apache-2.0 `esp-nn` (Xtensa PIE) while reporting `placed: host:dsp (esp32-s3 PIE)`. The exact same hook lets `--target device` on BCM2835 optionally delegate `sgemm` / `gemv_f16` / `relu` to a tiny, self-contained permissive Rust crate (`oxiscript-qpu-vc4`, MIT/Apache) that ships **2–3 pre-assembled, audited QPU binary kernels** and dispatches them via `/dev/vcio`, reporting `placed: host:qpu (vc4, approx)`.
3. **Fit with `8.Lab` ([D-29](../roadmap/oxiscript.md#d-29)):** `8.Lab`'s strict rule is *"a speedup never changes what a program does"* (including float bits, `P-9`). Because VideoCore IV QPU floats are non-IEEE (`FTZ`, simplified rounding), QPU offload **cannot** be a silent `8.Lab` optimization flag. It belongs either as an explicit `@approx` opt-in capability (`host:qpu`) or as a standalone `8.Lab` *research experiment / blog showcase* clearly flagged as `@approx`.

### 4.5 Honest Effort Estimate

| Scope | What It Involves | Effort Estimate | Verdict |
|---|---|---|---|
| **Option 1: Minimal `host:qpu` SGEMM/GEMV prototype (Recommended)** | Small permissive Rust crate (`oxiscript-qpu-vc4`) implementing `/dev/vcio` mailbox calls + 1–2 hand-tuned pre-assembled QPU kernels (FP32 SGEMM + optional FP16-unpacked GEMV, studying BSD-3 `pi-gemm` / `GPU_FFT` and MIT `py-videocore`), wired into OxiScript's `host:qpu` IDL + differential CPU oracle test. | **~1–2 weeks** (plus real-board testing turns with the owner) | **Worth doing** as a showcase demo & `std/tensor` portability proof. |
| **Option 2: Wrap `doe300/VC4CL` (MIT) via FFI** | Build `VC4C` + `VC4CL` (C++/LLVM) on/for Pi 1 and call OpenCL `clEnqueueNDRangeKernel` from a Rust `host:qpu` shim. | **~3–5 days** integration, but **high operational friction** (heavy C++/LLVM build, legacy maintenance, still requires root + no `vc4-kms-v3d`). | **Only useful for quick one-off benchmarking**, not for shipping a lightweight binary. |
| **Option 3: Full OxiIR → QPU compiler backend (`oxiscript-vc4`)** | Register-bank A/B allocator, delay-slot & SFU scheduler, VPM 2D DMA tiler, TMU gather lowering, BAD ACTOR fuzzing. | **2–4+ months** | **Reject** — terrible ROI on obsolete Pi 0–3 silicon. |

> **Corrected by Claude (Opus), 2026-11-26:** option 1 is an estimate that assumes new hand-written kernels. The smallest demo reuses audited BSD-3 kernels that already run on BCM2835 (GPU_FFT, pi-gemm). All of them need root: keep the OxiScript host unprivileged and put the QPU calls in a tiny privileged helper with a fixed command set, or evaluate the `vcio2` driver (ledger A9).


---

## 5. Recommended Path

### 5.1 (a) What to Measure First on the Real Board (Owner Consent / SSH, per `O-16` & `8.Mem`)

Never touch `/dev/fb0`, `/dev/dri/*`, `/dev/vcio`, or `/boot/firmware/config.txt` without explicit owner approval ([O-16](../roadmap/oxiui.md#o-16)). Run measurements in two safe phases over SSH:

1. **Phase 0 — Read-only system & graphics inventory (zero risk):**
   * OS, kernel, CPU, RAM, GPU/CMA split: `uname -a`, `cat /etc/os-release`, `cat /boot/firmware/config.txt` (or `/boot/config.txt`), `free -h`, `grep -E 'MemTotal|MemAvailable|Cma' /proc/meminfo`, `vcgencmd get_mem arm && vcgencmd get_mem gpu`.
   * Device nodes & active drivers: `ls -l /dev/dri /dev/fb* /dev/vcio /dev/video*`, `dmesg | grep -iE 'vc4|v3d|dsi|isp|cma'`, and (if `libdrm-tests` installed) `modetest -M vc4` to record the exact DSI connector resolution, pixel format, and available HVS DRM planes.
2. **Phase 1 — `8.Mem` CPU baselines (user-space only, headless, zero display impact):**
   * Cross-compile (`arm-unknown-linux-gnueabihf`, `-C target-cpu=arm1176jzf-s`) and run on Pi 1 B+:
     * `oxiterm-fb` headless PNG raster benchmark for 3–5 OxiUI pages at the DSI panel's native resolution: measure **full redraw ms**, **per-component breakdown** (`tiny-skia` vs integer fill vs `swash` glyphs), and **peak RSS**. This directly decides whether `U.Damage` is triggered (`oxiui.md` `U.Damage`).
     * OxiScript `wasmi` vs `w2c2 + gcc -O3 -mcpu=arm1176jzf-s -mfpu=vfp -mfloat-abi=hard` on a small MLP/256×256 SGEMM benchmark: record **ARM11 real GFLOPS**, **ms/inference**, **binary size**, and **RSS**.
3. **Phase 2 — QPU probe (only with owner's explicit boot-config consent):**
   * First probe `/dev/vcio` `MBOX_TAG_ENABLE_QPU` under the current `config.txt` (expected to fail if `vc4-kms-v3d` owns V3D).
   * If blocked, test in an isolated reboot (or spare SD card) with either (i) `dtoverlay=vc4-kms-v3d` commented out (headless SSH) or (ii) a test DT overlay disabling only `&v3d { status = "disabled"; }` while keeping HVS/DSI under `vc4` (`[unverified]` — verify whether the Pi kernel cleanly probes KMS-only `vc4` without V3D!).

> **Corrected by Claude (Opus), 2026-11-26:** use a spare SD card. Also record `vcgencmd get_throttled` (the board is corroded, under-voltage throttling is likely) and check that `libbcm_host.so` exists before running `hello_fft` (ledger F2, A9).

   * Run Andrew Holme's `hello_fft` (BSD-3) and a 256×256 / 512×512 SGEMM probe to record **true sustained GFLOPS** and **dispatch latency (µs)** on this exact Pi 1 B+.

### 5.2 (b) A Minimal Demo Worth Showing ("AI on a 2014 Raspberry Pi GPU, Written in OxiScript")

**Demo concept: "Live Handwritten Digit / Tiny-Vision Classifier in OxiScript on a 2014 Pi 1 B+ (< 20 MB RAM)"**

* **Program:** One unmodified OxiScript program (targeting `--target device`, compiled via `w2c2` native AOT) running a small **MNIST MLP (784→128→10, live mini-batch training + inference)** or a **tiny 64×64 CNN** (optionally fed by the CSI camera via `bcm2835-isp` downscaling if KMS/DSI coexistence works, or from bundled/streamed test frames over SSH/OxiUI).
* **Live A/B toggle inside OxiUI** (works identically over SSH TUI and OxiDE framebuffer):
  * **Mode 1 (`CPU — ARM1176 700 MHz w2c2`):** shows live latency (e.g. ~500–2000 ms per training batch or CNN frame, ~0.2 GFLOPS), placement badge `scalar (w2c2)`.
  * **Mode 2 (`GPU — VideoCore IV 12 QPUs via host:qpu`):** flips to the pre-assembled QPU SGEMM kernel; latency drops **~10×–30×** (~20–80 ms), showing live GFLOPS, **< 20 MB RSS**, and placement badge `host:qpu (vc4, @approx)`.
* **Why this demo punches far above its weight:**
  * Directly validates [D-20](../roadmap/oxiscript.md#d-20) (*"GUI and scripting for megabyte-class devices, measured on real Pi 1 B+ hardware"*).
  * Reuses the exact `std/tensor` + Course Level 5 MNIST story ([8.3e](../roadmap/oxiscript.md#s-8-3e)) without adding a single ML concept or VC4 quirk to OxiScript's core language.
  * Makes a standout, independently verifiable dev-blog post (`8.Blog`) with real hardware measurements.

### 5.3 (c) Risks and Mitigations

| Risk | Severity | Details | Mitigation |
|---|---|---|---|
| **1. `vc4-kms-v3d` vs Mailbox QPU driver conflict (DSI breakage)** | **High** | Modern Raspberry Pi OS DSI display drivers require `vc4-kms-v3d`, which claims V3D exclusively and blocks `/dev/vcio` QPU compute. Disabling `vc4-kms-v3d` typically breaks modern KMS DSI panels. | Test in Phase 2 whether disabling *only* the `&v3d` DT node lets HVS/DSI stay on `vc4` KMS while freeing V3D for mailbox QPU (`[unverified]`). Fallback: run the QPU AI demo over SSH TUI / legacy HDMI fbdev, or keep Pi 1 B+ DSI strictly on `oxiterm-fb` + CPU. |
| **2. Root privileges & zero IOMMU memory protection** | **High** | VideoCore IV has no MMU; any buggy QPU DMA write overwrites arbitrary physical kernel RAM. Violates OxiScript's sandbox guarantee if exposed carelessly ([P-6](../roadmap/oxiscript.md#p-6), [O-8](../roadmap/oxiui.md#o-8)). | Never allow guest OxiScript code to pass QPU bytecode or raw physical addresses. Expose **only** fixed, host-owned, bounds-checked kernels (`sgemm`, `gemv`, `relu`) through an explicit `host:qpu` capability; host allocates CMA/mailbox buffers, validates matrix dimensions before launch, and emits a loud build notice per [O-15](../roadmap/oxiui.md#o-15) ("Inform, don't forbid"). |
| **3. System stability & GPU hangs** | **Medium** | Bare-metal QPU execution has no OS TDR watchdog or preemption; a hung QPU kernel wedges V3D until reboot. | Static, finite, host-audited loop bounds inside the 2–3 pre-assembled kernels; chunk large GEMMs into bounded tiles; add a host-side timeout with circuit-breaker fallback to CPU `w2c2`. |
| **4. Float determinism vs OxiScript `P-9`** | **High (spec compliance)** | VideoCore IV QPUs flush denormals to zero, lack full IEEE NaN/Inf handling, and use simplified/RTZ rounding on ALU paths—**incompatible with strict bit-identical `P-9` float semantics**. | Classify `host:qpu` strictly as **`@approx`** (matching WGSL GPU kernels in `oxiscript-simd-gpu-design.md` §4.5: differential test within `1e-4`–`1e-5` relative tolerance against CPU reference), **forbid in `safety` profile (`OXS-E0500`)**, never enable silently in `8.Lab`, and log `approx` in the placement report (`P-5`). |
| **5. Dead-end hardware / maintenance drag** | **Medium** | VideoCore IV is 2012–2016 silicon (Pi 0–3); Pi 4/5 use VideoCore VI/VII (`v3dv` Vulkan → already covered by `wgpu`!). | Keep `oxiscript-qpu-vc4` strictly isolated as an optional experimental crate/capability outside `oxiscript-*` core crates; zero core compiler complexity. |

> **Corrected by Claude (Opus), 2026-11-26:** risk 5: Pi 1 B+ and Pi Zero stay in production until at least January 2030; the maintenance argument against a general compiler backend stands, the "dead-end" argument does not (ledger A3). Risk 1: see A2 (display-dependent).


---

## 6. Open Questions for the Owner

*(Note: `/home/slavekm/Dokumenty/Uruchomienie DSI_CSI Raspberry Pi B+.md` was outside this workspace's allowed read paths, so the board-specific questions below need the owner's input.)*

> **Corrected by Claude (Opus), 2026-11-26:** answers from the notes: no display or camera bought yet; candidate displays are the original 7" (800×480) and Touch Display 2 (720×1280, KMS only; the 10.1" model does not work on B+); the notes recommend Raspberry Pi OS Lite 32-bit with `vc4-kms-v3d`. New question: if the QPU demo must show its output on DSI, test first (spare SD card) whether the original 7" display runs on a legacy-firmware setup with mailbox QPU access, or whether the `&v3d`-disabled overlay works under KMS; only then choose the display (ledger A2).


1. **Current Pi 1 B+ software & display/camera config:** Which OS release is installed (Raspberry Pi OS Bullseye / Bookworm / Trixie, 32-bit `armhf`)? Does the DSI display require `dtoverlay=vc4-kms-v3d`, what is its exact resolution/pixel format (e.g., 800×480 DSI?), and what is the current `gpu_mem` / CMA allocation alongside the CSI camera?
2. **Simultaneous DSI + QPU vs headless QPU demo:** If `vc4-kms-v3d` cannot decouple `&v3d` from `vc4_dsi`, is an SSH-TUI / alternate-boot QPU benchmark acceptable, or do you specifically want the AI output painted live on the DSI screen at the same time?
3. **Target role for VideoCore IV QPU:** Do you want to pursue **Option 1** (a ~1–2 week isolated `host:qpu` SGEMM/GEMV prototype + "AI on a 2014 Pi GPU in OxiScript" demo after `8.Mem` / `8.2b`), or keep Pi 1 B+ strictly CPU-only (`oxiterm-fb` + `w2c2`) and save GPU compute exclusively for `wgpu`/WebGPU/Pi 4–5?
4. **SSH & measurement window for `8.Mem`:** Are you ready to authorize a dedicated non-root SSH measurement pass (Phase 0 + Phase 1: read-only hardware inventory + headless `oxiterm-fb` and `w2c2` CPU benchmarks) on the Pi 1 B+ before touching any boot config or QPU device nodes?
