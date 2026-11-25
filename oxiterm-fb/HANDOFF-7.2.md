# HANDOFF 7.2 (oxiterm-fb BAD ACTOR fixes) — keep current

## Status: all code fixes DONE; only the final 200k fuzz confirmation is outstanding.

### Fixed (proof)
- `cargo test -p oxiterm-fb`: bad_actor 21/21 (incl. the 2 formerly `#[ignore]`d: `shaped_huge_size_px_oom`, `surface_for_huge_viewport_must_be_none`; ignore attrs removed, no assertion changed), primitives 19/19, lib 6, golden 2 pass + 2 fail ONLY "missing golden" (by design, lead blesses).
- Decision 1 (sanitise): raster.rs: non-finite -> skip item/glyph; edges clamped to +-LIM=1e5 px (const-asserted > MAX_DIM+glyph); stroke <=1e4 px; shaped px in [0.5,512] (glyphs::size_ok) else run skipped; line grouping `j=i+1`; stroke radius re-clamped after snap (fuzz KNOWN=1 crash seed 15755400384260045945).
- Decision 2: glyph-style border uses i64 visible-range loops (`glyph_frame`).
- Decision 3: IRect is i64; grid `saturating_mul`; pixel.rs checked adds, transp validated, `convert_image` bounds-checks rgba (`hostile_layouts_never_panic`).
- Decision 4: surface.rs MAX_PIXELS 64M, MAX_DIM 32768 (also from_png); page.rs MAX_CELLS 1<<20; glyph cache byte budget 64 MiB (clear-all eviction), cluster text <=64 chars.
- Decision 5: `device::DeviceOptIn(())` token from `require_opt_in`; `fbdev::present`/`drm::present` take `&DeviceOptIn`; DRM: SIGINT/SIGTERM flag handler during hold, restores originally attached connectors, disables CRTC if it had no mode.
- Decision 6: clip mask built once per clip (`ClipEntry.mask`, lazily); fill_irect memcpy rows. Slow seed 11400714819323199497 was the 100000x100000-cell glyph border (50k+ per-cell iterations) -> fixed by visible-range loops.
- Scratch PNGs byte-identical to pre-hardening (`scratchpad/fb/pre-hardening`).

### Fuzz results (release + overflow checks + debug assertions, binary target/release/deps/bad_actor_fuzz-0d8d78784c00fba2)
- KNOWN=0 200000 cases (before fill_irect/radius tweaks): crashes=0, slow=3 (seed ...239539 ~1.7s, ...295693 2.1s load blip).
- KNOWN=1 40000 cases (seed 3, final code): crashes=0 slow=0.
- Residual "slow": seed 11400714819323239539 at scale 16 (9.2 Mpx surface): one huge-area AA path fill ~0.7s in a debug-assertions build; linear in pixels, bounded by the 64 Mpx cap. Not a hang.

### Doing now
Re-running the final 200k KNOWN=0 run (interrupted once).

### Verify commands
- `CARGO_BUILD_JOBS=2 cargo test -p oxiterm-fb --no-fail-fast`
- `CARGO_BUILD_JOBS=2 cargo clippy -p oxiterm-fb --features device` (only repo-wide pedantic warnings)
- fuzz: `CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo test -p oxiterm-fb --release --test bad_actor_fuzz --no-run`, then `FUZZ_CASES=200000 FUZZ_SEED=1 FUZZ_KNOWN=0 <binary> fuzz --ignored --nocapture` (~65 min; FUZZ_START=n resumes at case n).

### Notes
- Harness edits (diagnostics only): FUZZ_START, FUZZ_SCALE/FUZZ_KNOWN in dump_seed, timing prints. No test corrected/weakened.
- Bless goldens: `OXI_BLESS_FB=1 cargo test -p oxiterm-fb --test golden`.
