# Verification ledger: `videocore-iv-acceleration.md` (Gemini draft)

Verifier: Claude (Opus), 2026-11-26. Method: the same as `oxiscript-memory-performance-verification.md` and `oxiscript-simd-gpu-design-verification.md`. Every number, licence and decision-driving claim was checked against the cited source, or against a primary substitute when the cited page was dead, generic or silent. Licences were read from the `LICENSE`/`COPYING` files of the project repositories (GitHub API, 2026-11-26), not from summaries. Hardware facts were checked against the Broadcom *VideoCore IV 3D Architecture Reference Guide* (`VideoCoreIV-AG100-R`, downloaded from docs.broadcom.com/doc/12358545, text-searched), the Raspberry Pi `config.txt` documentation, the Linux `vc4` sources and documentation, and the Mesa driver pages. Repository facts were checked in this worktree. The owner's board notes (`/home/slavekm/Dokumenty/Uruchomienie DSI_CSI Raspberry Pi B+.md`, which Gemini could not read) were read and are used in section A and F.

Status values:
* **CONFIRMED**: the source says it.
* **PARTLY**: the gist is right, but a number, scope, clock or attribution is off.
* **WRONG**: the source contradicts it.
* **UNVERIFIABLE**: the cited source is generic, empty or silent, and no primary substitute was found.
* **NRC** ("not re-checked"): matches widely known facts, no decision rests on it, not opened in this pass. Not counted.

## Counts

| | Count |
|---|---|
| Claims checked (ledger rows B-F, excluding NRC) | 86 |
| CONFIRMED | 54 |
| PARTLY | 20 |
| WRONG | 5 |
| UNVERIFIABLE | 7 |
| NRC (not counted above) | 4 |

Section A lists the findings that change a recommendation. Each one also appears as a row in B-F and is not counted twice.

## A. Corrections that change the recommendation (read first)

| # | Report section | Gemini claim | Finding | Evidence |
|---|---|---|---|---|
| A1 | header, §6 | The owner's notes could not be read, so the display and camera configuration is an open question; frame sizes use 800×480 | **Superseded by the notes.** The board is a Pi 1 B+ V1.2 (2014, BCM2835, Samsung K4P4G324EQ 512 MB LPDDR2). **No display or camera is bought yet.** Candidates in the notes: the original 7" Touch Display (800×480) and Touch Display 2 (5"/7", **720×1280**, officially supported from Pi 1 B+). The 10.1" Touch Display 2 needs 4 DSI lanes and does not work. Custom DSI panels and Touch Display 2 need full KMS (`dtoverlay=vc4-kms-v3d`); the notes recommend Raspberry Pi OS Lite 32-bit (Bookworm/Bullseye) with `vc4-kms-v3d`, `display_auto_detect=1`, `camera_auto_detect=1`. The board shows **oxidation on GPIO, HDMI and A/V**: clean and check 5 V / 3.3 V before anything else. Native builds are slow (libcamera about 30 h), so cross-compile. | owner notes, sections "Identyfikacja sprzętu", "Dobór pasujących komponentów", "Ocena stanu fizycznego" |
| A2 | §1, §5.3 risk 1 | DSI display and mailbox QPU cannot coexist; disabling KMS "typically breaks modern KMS DSI panels" | **PARTLY, and the display choice decides it.** The original 7" Touch Display was driven by the closed firmware before KMS existed (owner notes: detection over I2C under firmware control), so with **legacy firmware display + `/dev/fb0`** (which `oxiterm-fb` already supports) the V3D is free for mailbox QPU work. Touch Display 2 and custom panels need full KMS, and under full KMS the mailbox tools do not work (A4). Gemini's `&v3d`-disabled experiment is **plausible**: in `vc4_drv.c` the V3D is an optional component (`get_param` returns `-ENODEV` when `!vc4->v3d`, the component match loop only adds devices that exist). It is still untested on this board. **Owner decision: if a QPU demo with the AI shown on DSI matters, buy the original 7" display, not Touch Display 2.** | owner notes; `torvalds/linux` `drivers/gpu/drm/vc4/vc4_drv.c` (`vc4_match_add_drivers`, `if (!vc4->v3d)`) |
| A3 | Exec. summary Q5, §4.4, §5.3 risk 5 | VideoCore IV is "discontinued", "dead-end", "obsolete Pi 0-3 silicon" | **WRONG.** "Raspberry Pi 1 Model B+ will remain in production until at least January 2030" (official product page); the Pi Zero (same BCM2835) has the same date. The ROI argument against a general QPU compiler backend still holds (few users, quirky ISA, no MMU), but not because the silicon is gone. | https://www.raspberrypi.com/products/raspberry-pi-1-model-b-plus/ |
| A4 | §3.1 (4), §3.2 table | Every QPU project "conflicts with `vc4-kms-v3d`"; fkms is the historic workaround; error "Unable to enable V3D/QPUs"; source: `github.com/raspberrypi/linux/issues` (generic) and the VC4CL README | **PARTLY: true, but the cited sources do not say it.** Primary evidence: the py-videocore README says it "does not work with the CPU-side OpenGL graphics stack" and tells users to comment out **both** `vc4-kms-v3d` **and** `vc4-fkms-v3d` (so fkms is not a workaround for py-videocore). VC4CL author (issues #60, #100): "VC4CL is not compatible with the 'full kms' OpenGL driver"; with `REGISTER_POKE_KERNELS` enabled they "won't work together". The VC4CL README itself says nothing about KMS. For GPU_FFT no primary statement was found; its real error text is "Unable to enable V3D. Please check your firmware is up to date." GPU_FFT's guide also says it **runs concurrently with OpenGL on Pi 1** when its L2-cache define is 0, which refers to the old firmware GL stack, not Mesa vc4. | https://github.com/Idein/py-videocore (README); https://github.com/doe300/VC4CL/issues/60 and /100; `raspberrypi/userland` `hello_fft/hello_fft.c:71`, `gpu_fft.txt` ("With Open GL on Pi 1") |
| A5 | §4.1 note, Exec. summary Q4 | Pi Zero runs "the same V3D/QPU @ 250 MHz", so "QPU compute numbers transfer directly" to Pi 1 B+ | **WRONG.** Defaults per `config.txt`: Pi Zero `v3d_freq` 300 MHz, `sdram_freq` 450; Pi 1 `v3d_freq` 250, `sdram_freq` 400. Pi 1 B+ QPU work is therefore about 17% slower in compute and about 11% lower in memory clock than Pi Zero figures. Idein also writes that its earlier figures were measured overclocked at 400 MHz and later ones at the 300 MHz default. Every Pi Zero number in the report has to be scaled down for this board, and every un-clocked number (py-videocore's 8 GFLOPS) is of unknown clock. | https://www.raspberrypi.com/documentation/computers/config_txt.html (per-model defaults table); Idein IIJlab seminar 2019-10-15, slide "行列乗算のみGPU化" note |
| A6 | §1, Exec. summary | ARM1176 "0.1-0.3 GFLOPS" so the V3D is "a ~100× theoretical compute asymmetry" | **WRONG as "theoretical".** Idein gives ARM1176 **peak** as 2 GFLOPS at 1 GHz (so about 1.4 at 700 MHz) against 28.8 GPU (Pi Zero), "about 14.4×". For Pi 1 B+: 24 / 1.4 ≈ **17×** peak. The 0.1-0.3 figure is an unmeasured sustained estimate; a sustained-vs-peak ratio is not "theoretical". Measure in 8.Mem. | Idein IIJlab seminar slides (BCM2835 table: "CPU peak performance 2Gflops", "GPU peak performance 28.8Gflops", "GPUの方が約14.4倍") |
| A7 | §3.2 (Idein, Pete Warden), §4.1 | MobileNet v1/v2 at 95-120 ms on Pi Zero, "~49× faster than unoptimized ARM11"; CPU MobileNet "~2-6 s" [Idein/SO reports, py-videocore link]; Pete Warden 20 s to 3 s | **PARTLY.** CONFIRMED: MobileNet V2 1.0 224×224 **95 ms** on Pi 0 (88 ms Pi 3), V1 **120 ms** (115 ms Pi 3) (Idein DLLab deck); 8.5 fps on Pi 0 including display; converted model "mathematically equivalent", FP32, no quantization (IIJ slides). **But** these used Idein's **private** libraries (qmkl README: "our private libraries, which are highly optimized for neural networks"), so they are not reproducible from open code. "49×" and the CPU "2-6 s" are **UNVERIFIABLE**: the cited py-videocore README has no MobileNet number and the cited Speaker Deck link is the site root. Pete Warden: DeepBeliefSDK README says "around five seconds" on a **stock** Pi and "three seconds with a boosted GPU clock"; pi-gemm's 500 ms vs 8 000 ms (ATLAS) was "on my overclocked Pi". | https://speakerdeck.com/nineties/ideinfalseshao-jie-at-dllab-tui-lun-naito ; https://seminar-materials.iijlab.net/iijlab-seminar/iijlab-seminar-20191015.pdf ; `Idein/qmkl` README; `jetpacapp/DeepBeliefSDK` README; `jetpacapp/pi-gemm` README |
| A8 | §3.2 qmkl | qmkl licence "verify before any use [unverified — mixed/GPL-adjacent tooling history]" | **WRONG.** `Idein/qmkl/LICENSE`: "Modified (3-Clause) BSD License", © 2016-2017 Idein Inc. It is usable under O-14. Its build needs `py-videocore` (MIT), `qasm2`, `qbin2hex`, `mailbox`, `librpimemmgr` (licences of these four not checked) and kernel ≥ 4.9.79 with the legacy mailbox stack. | https://github.com/Idein/qmkl (LICENSE, README) |
| A9 | §4.5 option 1, §5.2 | `host:qpu` prototype with 1-2 hand-tuned new QPU kernels in "~1-2 weeks" | **PARTLY (estimate, optimistic) and a missing design point.** Writing and verifying new QPU assembly (bank conflicts, delay slots, VPM/TMU DMA) is the expensive part. The smallest demo **reuses audited BSD-3 kernels** that already run on BCM2835: GPU_FFT (Andrew Holme, BSD-3) or pi-gemm (Jetpac, BSD-3). Also missing: every one of these tools needs **root** (GPU_FFT guide: "programs which use the mailbox must run as root"; pi-gemm "as 'su'"). Running the OxiScript host as root contradicts the sandbox story; put the QPU calls in a tiny privileged helper process with a fixed command set, or evaluate Marcel Müller's `vcio2` driver ("access the Raspberry Pi VideoCore IV GPU without root privileges", per the GPU_FFT page). GPU_FFT also `dlopen`s `libbcm_host.so` from the legacy userland (repository archived); whether the owner's OS still ships it must be checked in Phase 0. | GPU_FFT page (http://www.aholme.co.uk/GPU_FFT/Main.htm); `hello_fft/gpu_fft_base.c:69` (`dlopen("libbcm_host.so")`); `raspberrypi/userland` archived = true |
| A10 | §4.2 | "int8 gives zero ALU throughput gain over fp32" | **PARTLY.** The MUL ALU has 8-bit vector ops (`v8muld`, `v8adds`, `v8subs`, `v8min`, `v8max`) that work on 4 bytes per 32-bit lane, but `v8muld` treats bytes as fixed-point [0, 1.0] colour values and there is no wide accumulator, so it is useless for int8 dot products. The practical conclusion (compute in FP32, store weights as FP16/U8 and unpack in regfile A) stands. | AG100 p.17 ("8-bit vector adds/subs/min/max and multiply (where the 8-bit vector elements are treated as being in the range [0.0, 1.0])"), op table (`V8muld 3`) |

## B. Ledger: hardware (§1.1)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| B1 | BCM2835, ARM1176JZF-S ARMv6 @ 700 MHz, VFPv2, no NEON, 512 MB LPDDR2 @ 400 MHz | CONFIRMED | `config.txt` defaults (Pi 1: arm 700, sdram 400); owner notes (K4P4G324EQ, 512 MB) |
| B2 | 12 QPUs in 3 slices × 4, 250 MHz stock on Pi 1 | CONFIRMED | py-videocore6 README peak formula ("3 [slice] x 4 [qpu/slice]"); `config.txt` (`v3d_freq` 250 for Pi 1) |
| B3 | 16-way SIMD built from 4 physical lanes over 4 cycles; dual-issue ADD and MUL ALU | CONFIRMED | AG100 p.17: "Internally the QPU is a 4-way SIMD processor multiplexed to 16-ways …"; "dual-issue" |
| B4 | 24 GFLOPS @ 250 MHz, 28.8 @ 300 MHz | CONFIRMED | `Idein/py-videocore6` README (formula for VideoCore IV at 250 and 300 MHz) |
| B5 | ARM1176 0.1-0.3 GFLOPS; "~100× theoretical asymmetry" | WRONG | A6 |
| B6 | Overclockable to 300-400+ MHz | CONFIRMED | Idein slides (earlier measurements "overclocked (400MHz)") |
| B7 | FP32 only; FTZ, incomplete NaN/Inf, round-to-zero; source AG100 and VC4CL `clinfo` | PARTLY | AG100 contains no IEEE/denormal/rounding text (full-text search). VC4CL `src/vc4cl_config.h`: "according to tests, values are always rounded to zero (i.e. for fmul, fadd and fsub operations)", `FLOATING_POINT_CONFIG = CL_FP_ROUND_TO_ZERO` (no `CL_FP_DENORM`, no `CL_FP_INF_NAN`). RTZ confirmed by test; FTZ and NaN behaviour implied by the missing flags, not documented. P-9 conclusion holds. |
| B8 | 32-bit int add/bitwise/shift on ADD ALU; `mul24` on MUL ALU; 8-bit saturating ops | CONFIRMED | AG100 op tables (`mul24 2 24 bit multiply`, `v8adds`, `v8subs`) |
| B9 | No native int8×4 dot product | CONFIRMED (see A10 for the throughput nuance) | AG100 op tables |
| B10 | Regfile-A (and r4) unpack: 16a/16b FP16→FP32, 8a-8d u8→[0,1] float or u8→u32 | CONFIRMED | AG100 unpack table (`16a32` … `8d32`), "accumulator r4 has a more limited unpack unit" |
| B11 | SFU recip/recipsqrt/exp2/log2 per slice; r4 hazard on the next two instructions | CONFIRMED | AG100 p.17 ("separate, shared block in each slice"); restrictions list ("After an SFU lookup instruction, accumulator r4 must not be read in the following two instructions") |
| B12 | Two 32-entry 16-wide register files plus r0-r5; 4 KB registers per QPU | CONFIRMED | AG100: "64 locations, 32 of these are backed by the physical registers"; four general + two special accumulators; 2×32×16×4 B = 4 KB |
| B13 | VPM "4 KB total (64 rows × 16 × 32-bit) shared across all QPUs" | PARTLY | AG100: "The minimum VPM size is 8Kbytes … Fully configured systems may have up to 16Kbytes"; 64×16 words (4 KB) is the maximum **QPU general-purpose window**, not the VPM size |
| B14 | 2 TMUs per slice with L2 cache; GPGPU uses TMU for gathers, VPM DMA for writes | PARTLY | AG100: "one or two" TMUs per slice (configuration-dependent); GPU_FFT page: TMU "accepts a vector of 16 potentially scattered addresses", writes through VPM/VDW |
| B15 | Three branch delay slots | CONFIRMED | AG100: "three 'delay slot' instructions following a branch instruction are always executed" |
| B16 | No IOMMU/MMU; V3D accesses physical memory directly | CONFIRMED | docs.kernel.org/gpu/vc4.html: "Since the VC4 has no IOMMU between it and system memory …"; VC4CL README "Security Considerations" |

## C. Ledger: APIs and other blocks (§1.2, §1.3, §2)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| C1 | Mesa `vc4`: GLES 2.0, "nearly conformant" | CONFIRMED | https://docs.mesa3d.org/drivers/vc4.html |
| C2 | OpenGL 2.1 partial; missing 32-bit index buffers, occlusion queries | PARTLY | Mesa page: GL 2.1 "mostly correct"; 4-byte indices are **truncated to 2 bytes** (wrong above 65535), no occlusion queries, no line/point polygon modes. No "software workaround". |
| C3 | No GLES 3.x | CONFIRMED | Mesa vc4 page lists only GLES 2.0 / GL 2.1 |
| C4 | `v3d`/`v3dv` target VideoCore VI/VII (Pi 4/5), not VC IV | CONFIRMED | https://docs.mesa3d.org/drivers/v3d.html ("V3D 4.2 (Raspberry Pi 4), and V3D 7.1 (Raspberry Pi 5)", "conformant GLES3.1"); Vulkan version not on that page |
| C5 | Kernel `vc4` refuses arbitrary compute because of no IOMMU; validator checks graphics submissions | CONFIRMED | kernel docs: compute-style job interface "isn't supported by this driver"; "The shader validator walks over a shader's BO, ensuring that its accesses are appropriately bounded" |
| C6 | OpenCL only via VC4CL, OpenCL 1.2 Embedded Profile | CONFIRMED | VC4CL README |
| C7 | VC4CL runs "as root" via mailbox/register access | PARTLY | VC4CL README: "needs to be either `root` … or be in the `video` group", depending on the selected interfaces (mailbox, register poking, VCHI; memory via VCSM-CMA, VCSM or mailbox) |
| C8 | VPU: dual-core 250 MHz with 16×16 integer vector unit, runs ThreadX `start.elf` | NRC | hermanhermitage/videocoreiv (not reopened) |
| C9 | HVS: planes, scaling, colour conversion, compositing; feeds HDMI/DSI/DPI | CONFIRMED | kernel docs: HVS "does translation, scaling, colorspace conversion, and compositing"; "Each DRM plane is a layer of pixels being scanned out by the HVS" |
| C10 | ISP and H.264 codec via "mainline V4L2" (`bcm2835-isp`, `bcm2835-codec`) | PARTLY | Both are Raspberry Pi kernel drivers on top of the firmware VCHIQ/MMAL services (staging/downstream), not mainline; the CSI receiver `bcm2835-unicam` is the open one (owner notes; LWN 966543). They do not depend on V3D, so they coexist with mailbox QPU use. |
| C11 | H.264 1080p30 decode/encode | NRC | widely documented |
| C12 | `oxiterm-gpu` default requests `Limits::downlevel_webgl2_defaults()` (`context.rs:93-95,195-199`) | CONFIRMED | `oxiterm-gpu/src/context.rs:93-97` (doc comment), `:197-199` |
| C13 | wgpu GL backend needs GLES 3.0 / GL 3.3, so no adapter on vc4 | CONFIRMED | `wgpu-hal-30.0.2/src/gles/adapter.rs:254-266` (rejects `es_ver < (3, 0)` and `full_ver < (3, 3)`) |
| C14 | lavapipe/llvmpipe on ARMv6 "sub-1 FPS", tens of MB | UNVERIFIABLE | marked `[unverified]` by Gemini; no source; plausible |
| C15 | Path A "already exists in 7.2": dumb buffers + HVS planes in `drm.rs` | PARTLY | `oxiterm-fb/src/device/drm.rs:331-372`: one dumb buffer, `convert_image` to XRGB8888, `SETCRTC` on the primary plane. No plane API (no overlay planes, no atomic, no page flip) is used today; "HVS planes" is a future step (§2.3 item 4 says so correctly). |
| C16 | `Rasterizer::render` clears the whole surface and repaints (`raster.rs:85-97`) | CONFIRMED | `oxiterm-fb/src/raster.rs:84-97` |
| C17 | 800×480×4 = 1.54 MB; 1080p 8.29 MB; one 20 px line touches 64 KB | CONFIRMED (arithmetic) | 800×480×4 = 1 536 000; 1920×1080×4 = 8 294 400; 800×20×4 = 64 000. Add Touch Display 2: 720×1280×4 = 3.69 MB (A1). |
| C18 | LPDDR2 400 MHz ≈ 3.2 GB/s theoretical | CONFIRMED (arithmetic) | 400 MHz DDR × 32-bit = 3.2 GB/s; usable 1.0-1.8 GB/s is marked `[unverified]` |
| C19 | DRM `FB_DAMAGE_CLIPS` cuts panel transfers only on command-mode/SPI panels | NRC | kernel KMS helper docs (not reopened) |
| C20 | tiny-skia "100-300% slower than Skia on ARM" | PARTLY | tiny-skia README says exactly that, but "ARM" there is AArch64 with NEON ("We support ARM AArch64 NEON"); on ARMv6 without NEON tiny-skia runs its scalar fallback, so expect worse. The advice (keep Pi 1 themes square, cache rounded panels) is right. |
| C21 | Mesa/EGL adds 15-30 MB RSS | UNVERIFIABLE | marked `[unverified]` by Gemini; measure in 8.Mem |
| C22 | A GLES 2.0 sink "cannot coexist with mailbox QPU AI because Mesa vc4 actively uses V3D" | CONFIRMED | A4 (VC4CL #60: Mesa full-KMS driver and VC4CL "stomping on each other") |

## D. Ledger: prior art (§3.2), numbers and licences

| ID | Claim | Status | Evidence |
|---|---|---|---|
| D1 | GPU_FFT licence BSD-3-Clause | CONFIRMED | `raspberrypi/userland/host_applications/linux/apps/hello_pi/hello_fft/gpu_fft.c` header: "Copyright (c) 2015, Andrew Holme. All rights reserved." + BSD-3 text |
| D2 | `raspberrypi/userland` archived | CONFIRMED | GitHub API `archived: true` |
| D3 | GPU_FFT "~10× faster than Pi 1 ARM11 FFTW": 256-pt 0.033 vs 0.092 ms; 4096-pt 0.25 vs 3.0; 65 536-pt 7.0 vs 83; 4M-pt 786 ms | PARTLY (every number exact; "~10×" overstated for small sizes) | `gpu_fft.txt` (release 3.0) table "on a Pi 1 with L2 cache enabled", ms, 2 s.f.: exact match. The ratio is **2.8×** at 256 points (batch 1) and 8-12× for 4K-4M points; the 2014 release-1.0 blog table even shows 256-pt batch-1 at 112 µs vs FFTW 92 µs (slower than the CPU, because of the ~100 µs mailbox call). Short single transforms gain little. |
| D4 | GPU_FFT ~1 ppm RMS error, uses 8 QPUs | CONFIRMED | `gpu_fft.txt` (0.33-1.5 ppm; "FFTW is still one order of magnitude more accurate"); GPU_FFT page ("BCM2835 V3D has 12 QPUs; but GPU_FFT only uses 8") |
| D5 | GPU_FFT needs root, uses mailbox `/dev/vcio` + V3D registers | CONFIRMED | GPU_FFT page ("programs which use the mailbox must run as root"; register poking for short jobs) |
| D6 | GPU_FFT conflicts with `vc4-kms-v3d` | UNVERIFIABLE | A4 (no primary statement found; inferred from the shared mailbox mechanism) |
| D7 | py-videocore licence MIT | CONFIRMED | `Idein/py-videocore` (= `nineties/py-videocore`) LICENSE, MIT |
| D8 | py-videocore "~8 GFLOPS SGEMM (1/3 of peak, memory-bound)" | PARTLY | README "Records: Achieved 8GFlops with sgemm." Board and clock not stated (see A5: Idein measured overclocked at 400 MHz early on). "1/3 of peak, memory-bound" is Gemini's inference. |
| D9 | py-videocore conflicts with KMS, needs root | CONFIRMED | README (comment out `vc4-kms-v3d` and `vc4-fkms-v3d`; requires `rpi-vcsm`) |
| D10 | py-videocore6: VC VI (Pi 4 only), 3.6-3.9 GFLOPS SGEMM of 32 peak @ 500 MHz; py-videocore7 21.3 of 76.8 | CONFIRMED | README: "QPU: 0.5546 sec, 3.878 Gflop/s" (1024³); py-videocore7 "21.3827 Gflop/s", peak 76.8 @ 800 MHz |
| D11 | py-videocore6 via the `v3d` DRM driver (`/dev/dri/card0`), safe because VC VI has an MMU | CONFIRMED | README ("communicates with the V3D hardware through `/dev/dri/card0`") |
| D12 | py-videocore6 licence GPL-2.0, reference only | CONFIRMED | GitHub licence `GPL-2.0` (py-videocore7 likewise) |
| D13 | vc4asm licence GPL-3.0, reference only | CONFIRMED | `maazl/vc4asm` `LICENSE.txt` ("version 3 of the License, or (at your option) any later version"), `COPYING.txt` GPLv3 |
| D14 | qmkl licence unverified / GPL-adjacent | WRONG | A8 (BSD-3) |
| D15 | qmkl6 (Pi 4) BSD-3-Clause | CONFIRMED | GitHub licence `BSD-3-Clause` |
| D16 | Actcast compiler proprietary | CONFIRMED | qmkl README ("our private libraries") |
| D17 | Idein MobileNet v1/v2 FP32 95-120 ms (8-10 fps) on Pi Zero | CONFIRMED | A7 |
| D18 | "~49× faster than unoptimized ARM11"; CPU MobileNet 2-6 s | UNVERIFIABLE | A7 (cited links contain no such figure) |
| D19 | Idein via legacy mailbox stack, root, conflicts with KMS | PARTLY | qmkl needs `mailbox`, `librpimemmgr`, kernel ≥ 4.9.79 (legacy stack); Actcast's own OS configuration was not checked |
| D20 | DeepBeliefSDK / pi-gemm licence BSD-3 | CONFIRMED | `jetpacapp/DeepBeliefSDK/LICENSE`, `jetpacapp/pi-gemm/LICENSE` (BSD-3, © 2014 Jetpac); pi-gemm README "All code is under the BSD three-clause license" |
| D21 | Pete Warden: ~20 s CPU → 6 s → 3 s per image | PARTLY | blog: "almost twenty seconds even with optimized assembler", VPM version ~6 s, TMU version "just three"; DeepBeliefSDK README: about **5 s stock**, 3 s "with a boosted GPU clock rate" |
| D22 | pi-gemm GEMM ~8 000 ms ATLAS → ~500 ms on overclocked Pi 1 QPU | CONFIRMED | pi-gemm README ("500ms on my overclocked Pi, rather than 8,000 ms using the official Atlas library") |
| D23 | Pete Warden "~16× over ATLAS" (§4.1) | PARTLY | 8 000 / 500 = 16× but overclocked; stock clock not stated |
| D24 | Mesa vc4 QPU compiler MIT, usable as reference | NRC | Mesa vc4 files carry MIT headers (not reopened) |
| D25 | QPULib MIT, "explicitly archived/unmaintained" | PARTLY | `mn416/QPULib` LICENSE: MIT **except** `Lib/VideoCore/Mailbox.cpp/.h` (Broadcom BSD-3, header "Copyright (c) 2012, Broadcom Europe Ltd.") and two images. Not archived on GitHub; README: "an experimental library, no longer under development". |
| D26 | V3DLib MIT, VC4 + VC6 | PARTLY | `wimrijnders/V3DLib` LICENSE: MIT except the mailbox files, `assign.h`, `catch.hpp`, everything under `mesa/`, and a copy of the AG100 PDF. All exceptions are permissive or documentation, but the vendored Mesa subtree needs its own SBOM line. |
| D27 | VC4CL / VC4C / VC4CLStdLib MIT | CONFIRMED | GitHub licence `MIT` on all three |
| D28 | VC4CL "strongly outperforms PoCL CPU on float16 ALU benchmarks" | UNVERIFIABLE | not in the README; no source given |
| D29 | VC4CL VideoCore IV only (Pi 0-3), not Pi 4 | CONFIRMED | README: "VC4CL will NOT work with Raspberry Pi 4" |
| D30 | VC4CL conflicts with full KMS in register-poke mode | CONFIRMED | A4 (issues #60, #100) |
| D31 | VC4CL "legacy / minimal maintenance" | CONFIRMED | last push 2022-09-14 (VC4C 2025-11-08) |

## E. Ledger: AI on the QPUs (§4)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| E1 | QPU sustained "~4-8 GFLOPS" on well-tiled SGEMM | PARTLY | 8 GFLOPS is py-videocore's unclocked record (D8); Idein reports "about 40% of theoretical peak" on DNN work (Nikkei xTECH, via search summary, not opened) |
| E2 | Latency table for MLP, LeNet, KWS, MobileNet-tiny, 1M-param LM | UNVERIFIABLE | all cells marked `[estimate]`; no measurement exists. Keep as [estimate]; the Pi Zero → Pi 1 clock difference (A5) also applies. |
| E3 | MobileNet 1.0 224 row: "proven by Idein in FP32 on Pi Zero" | CONFIRMED | A7 (95 / 120 ms) |
| E4 | Compute in FP32, store FP16/U8 and unpack for free | CONFIRMED | AG100 unpack table; A10 for the int8 nuance |
| E5 | int8 has zero ALU throughput gain | PARTLY | A10 |
| E6 | CMA 64-128 MB under `vc4-kms-v3d`; static `gpu_mem` under legacy | PARTLY | overlay README offers `cma-64`/`cma-96`/`cma-128` on 512 MB boards (larger values "needs 1GB"), and the firmware refuses the overlay on boards with less than 512 MB. The default value on a 512 MB Pi was not checked. |
| E7 | Memory budget numbers (Linux 40-80 MB, headroom 250-350 MB) | UNVERIFIABLE | marked `[unverified]`; measure in 8.Mem Phase 0 |
| E8 | QPU offload cannot be a silent 8.Lab speedup because of non-IEEE floats | CONFIRMED | D-29 rule "a speedup never changes what a program does" (`docs/roadmap/oxiscript.md:348`) plus B7 |
| E10 | Pi Zero QPU numbers "transfer directly" to the Pi 1 B+ | WRONG | A5 (v3d 300 vs 250 MHz, sdram 450 vs 400 MHz) |
| E9 | Pattern mirrors ESP32-S3 PIE via esp-nn in `oxiscript-simd-gpu-design.md` §1.3/§6.3 | CONFIRMED | that report (esp-nn Apache-2.0 verified in its verification ledger, A8 there) |

## F. Ledger: recommended path and risks (§5, §6)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| F1 | Never touch `/dev/fb0`, `/dev/dri/*`, `/dev/vcio`, `config.txt` without the owner (O-16) | CONFIRMED | `docs/roadmap/oxiui.md:46` (O-16 lists fb0, dri, input; vcio and config.txt are a sensible extension) |
| F2 | Phase 0/1/2 measurement plan | CONFIRMED (sound) | add `vcgencmd get_throttled` (the corroded board may under-volt, A1) and a check for `libbcm_host.so` (A9) |
| F3 | Cross-compile with `-C target-cpu=arm1176jzf-s` / `-mcpu=arm1176jzf-s -mfpu=vfp -mfloat-abi=hard` | CONFIRMED | correct flags for the ARMv6 hard-float ABI; note that a stock Debian/Ubuntu `arm-linux-gnueabihf` sysroot is ARMv7 and its libc will not run on ARMv6 (use the Raspberry Pi OS sysroot or a static/musl build) |
| F4 | `&v3d { status = "disabled"; }` keeps HVS/DSI on vc4 | PARTLY | A2 (driver code supports a missing V3D; board test still needed; Gemini correctly marked it `[unverified]`) |
| F5 | Risk 5: "dead-end hardware" | WRONG | A3 |
| F6 | Risk 4: forbid `host:qpu` in the safety profile, classify `@approx` | CONFIRMED (consistent with P-9 and the SIMD/GPU report's `@approx` tier) | `oxiscript-simd-gpu-design.md` §4.5 |
| F7 | Effort: option 1 "1-2 weeks", option 2 "3-5 days", option 3 "2-4+ months" | PARTLY | A9 (estimates; option 1 assumes new kernels; reusing GPU_FFT/pi-gemm is smaller) |
| F8 | Owner notes "outside allowed paths" | WRONG (now) | A1 |
| F9 | `oxiterm-fb` stays the Pi 1 path (O-5) | CONFIRMED | `docs/roadmap/oxiui.md:35` (O-5: "hardware below the wgpu floor (Raspberry Pi 0-3) uses the CPU/framebuffer sink") |
| F10 | Mailbox tags `MBOX_TAG_ENABLE_QPU`, `ALLOCATE/LOCK_MEMORY`, `EXECUTE_QPU` | CONFIRMED | GPU_FFT page (`qpu_enable()`, `execute_qpu()` through the mailbox; ~100 µs per call) |
| F11 | Direct register dispatch via `V3D_SRQPC` / `V3D_SRQUA` | CONFIRMED | `gpu_fft_base.c` (`base->peri[V3D_SRQCS]` …), AG100 register map |

## G. Recommendation for the roadmap (verifier's view)

* **Keep VideoCore QPU compute an idea, not a stage.** Add one consent-gated item to [8.Mem](../roadmap/oxiscript.md#s-8-mem): a QPU probe on a **spare SD card** (never the owner's working one): run `hello_fft` and pi-gemm, record sustained GFLOPS, mailbox/register dispatch latency, and whether the `&v3d`-disabled overlay keeps KMS working. Promote to a stage only if (1) the probe shows at least about 5× over the w2c2 CPU SGEMM on the same board, and (2) either display and QPU coexist (original 7" display on the legacy firmware path, or the `&v3d` trick under KMS) or the owner accepts a headless/SSH demo.
* **Smallest worthwhile demo:** do not write QPU assembly. Wrap an existing BSD-3 kernel in a tiny privileged helper and expose it as a fixed host capability: GPU_FFT as `host:fft` driving a live spectrum view in OxiUI (audio or synthetic signal; GPU_FFT's measured 8-12× at 4K+ points is real and documented), or pi-gemm for a batched MNIST MLP step. Mark the result `@approx` and show the CPU w2c2 number next to it. This is a few days of integration, not 1-2 weeks of kernel work.
* **OxiUI on the Pi 1:** the report's recommendation stands (CPU sink, U.Idle, U.Damage if 8.Mem shows the need, double-buffered dumb buffers + page flip, render directly in XRGB8888). Frame sizes depend on the display the owner buys (A1).
* **Owner questions to add:** which display (original 7" 800×480 keeps the QPU option on the legacy path; Touch Display 2 720×1280 needs KMS); is a headless QPU demo acceptable.
