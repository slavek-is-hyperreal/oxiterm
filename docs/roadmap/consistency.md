# Consistency, stale documents and doc fixes

Written 2026-11-26 against main 19cf8cd. Two halves (OxiUI by W-UI, OxiScript by W-OXS) plus cross-cutting items. Nothing here has been applied: the lead deletes or edits the old documents after review. Roadmaps: [oxiui.md](oxiui.md), [oxiscript.md](oxiscript.md), index [README.md](README.md).

## Cross-cutting

| # | Item | Resolution / action |
|---|---|---|
| X-1 | **Roadmap generator.** `sites/build.mjs` (`roadmapData`, lines 117-170) reads only `docs/oxi-ui.md`. It finds phases with `^### (Phase \d+): (.+)$` (state `done` if the heading has "✅ done", `planned` if it has "planned" or "draft"), the **wave-log table** by header (`Slice` | `Implementation`, six cells; a slice is done if the sixth cell, `Merged`, holds a 7-hex hash or a date and contains no em dash, active if it has an em dash, planned if no row starts with the slice id), the **slices table** by header (`Slice` | `Crate / area`), and the **Phase 7 table** (`Stage` | `Scope`, with a row starting `7.1`). It throws if the tables are missing. | When the lead deletes `docs/oxi-ui.md` the build fails. The generator must read `docs/roadmap/oxiui.md` section 3 (first cell = stage id, last cell = `date, hash`) and the `Stage \| Status \| Merged` table of `docs/roadmap/oxiscript.md`, and take titles/scopes from the stage headings. Do not delete `oxi-ui.md` before this is changed (stage U.Sites). Also add rows for stages that never had a wave-log row (5.2, 7.6, Run, identity, showcase, targets, 0.0.1): they are in both new files. |
| X-2 | "Decision 4" in the SIMD/GPU research (`oxiscript-simd-gpu-design.md:12` and the "[Revised ... for decision 4 transparency]" markers) means *principle* 4 of the plan (`oxi-ui.md:621`); *decision* 4 (`:715`) is "GPU in v1: no". | Resolved in the new numbering: `oxiscript.md#p-4` vs `#d-4`. Fix the research document's wording. Source: R4 incons. 1. |
| X-3 | 8.M-link has no row in the old plan although `8.Perf`, `8.Std`, `8.Mem`, 8.S1 and 8.3d depend on it. | Resolved: a stage in [oxiscript.md](oxiscript.md#s-8-m-link). Source: R2 incons. 4. |
| X-4 | Old "8.2" meant both targets (done) and AOT/profiles (todo); old "Stage 8.0" was named four times but never a row. | Resolved by W-OXS ids: 8.2a (targets, done), 8.2b (AOT), 8.2c (profiles), 8.0 (measurement spike). Source: R2 incons. 7. |
| X-5 | Pages vs Workers (UI-21). | Owner roadmap: oxiui.md U.Sites. |
| X-6 | The input document of 8.Perf, `docs/research/oxiscript-memory-performance.md`, exists only as an uncommitted file in the worktree `gemini-memres`; the plan links it and the link is dead on `main`. | Commit it from that worktree before pruning (U.Hygiene); linked by [8.Perf](oxiscript.md#s-8-perf). Source: R2 incons. 5; R4. |
| X-7 | Stage notes live in `docs/stages/<id>/`; dir names `run`, `identity`, `showcase`, `targets`, `host-export`, `0.0.1` have no plan id. | Resolved: ids 8.Run, 8.Id, `Showcase`, 8.2a, 8.HX, 8.Rel in the new files; index them in `docs/stages/README.md`. |
| X-8 | Shared GPU context: `GpuContext` today exposes `device()`/`queue()` publicly and `with_error_scopes` is `pub(crate)`; 8.3c changes this, 5.3 must not break it. | Requirement recorded in oxiui.md 5.3 and [8.3c](oxiscript.md#s-8-3c). |

## OxiUI half (W-UI)

Verification done for this file: `git merge-base --is-ancestor` on every hash cited in oxiui.md; spring order, tick rates and `position` default re-read in code on `main` at `19cf8cd`.

### A. Inconsistencies

| # | Inconsistency | Resolution | Sources |
|---|---|---|---|
| UI-1 | `spring(...)` argument order: code `oxiterm-renderer/src/parser/tcss.rs:944-951` and `tcss-reference.md:180` read `spring(stiffness, damping, mass)`; `CHANGELOG.md:64`, `IDEAS.md:60` say `(mass, stiffness, damping)`; thtml skill line 42 says `spring(m,k,c)`. | Resolved: code is right (verified). Fix the three documents (section C). | R1 incons. 1; R1-91 |
| UI-2 | `position` default: `tcss-reference.md:72` `static`; thtml skill line 37 `relative`. | Resolved: code `oxiterm-proto/src/style.rs:22-25` `#[default] Static` (verified). Fix the skill. | R1-93 |
| UI-3 | Idle tick and frame rate: code `event_loop.rs:1261-1264` is 16 ms while a transition/animation is active, 1000 ms idle (verified). `ARCHITECTURE.md:68` (5 ms idle poll, 66 ms / 15 FPS), `DEMO.md:31`/`:280`, `retro-desktop-environment.md:127` say otherwise; `CHANGELOG.md:67`, `tcss-reference.md:183`, `IDEAS.md:61` are correct. | Resolved: code wins; fix the stale ones. The idle wake-up itself is U.Idle (`next_deadline` never returns `None`). | R1-46,90; R1 incons. 4 |
| UI-4 | Mobile design: concept document 1 (User-Agent + 302 `/mobile`) vs `ARCHITECTURE.md:92`/`CHANGELOG.md` 0.4.0 (client-side < 800 px + `0x11`). | Resolved: shipped design wins; concept document is historical. | R1-07,48 |
| UI-5 | SSH auth: concept "keys only, passwords rejected" vs `OXITERM_PASSWORD`, `--no-auth`, guest web access. | Resolved: shipped behaviour wins (decision superseded, oxiui.md section 2). | R1-04,81 |
| UI-6 | A11y: concept 2 rejects the linear fallback as sole answer; the linear `--a11y` sink is what shipped for SSH; AccessKit/AT-SPI tunnel promised, `DBusBridge` has no transport. | Needs owner: Q-6, stage U.A11y. | R1-24,25,47 |
| UI-7 | Unicode width: concept 2 calls PUA/VTM modifiers "critical"; `insert_vtm_modifier` (`render/unicode.rs:74`) has no non-test caller, `OSC 1337` appears nowhere; shipped approach is glyph avoidance + width cache + linter W006. | Needs owner: Q-7, decision record in U.Term. | R1-21; R1 incons. 5 |
| UI-8 | Roadmap checklists vs code: `embedded-iot-platform.md:163` leaves `oxiterm-fb` unchecked; `IDEAS.md:11-16` statuses (fb "conceptual", OxiUI "Phases 0-2"); `retro-desktop-environment.md:76-93` lists position/z-index/drag as missing though `CHANGELOG` 0.6.0 ships them; OxiDE phases 1-5 and `iot-screens.md:41,139` "OxiUI phase 3/5" collide with the current phase numbers. | Resolved by this roadmap: those lists are replaced (stale-document table, section B); phase naming now only in oxiui.md. | R1-63,74,79,56; R1 incons. 6,14 |
| UI-9 | `oxi run`/`oxi build` "next" (`ARCHITECTURE.md:247`, `README.md:75`) vs shipped (`CHANGELOG.md:18,23`, `DEPLOY.md:93`, Run stage merged `6dda5de`). | Resolved: shipped. Fix the two docs. | R1-34,111 |
| UI-10 | Build prerequisites: `README.md:23-26` (wasm-pack before `cargo build`) vs `getting-started.md:11-16`. | Resolved: README is right; fix getting-started. | R1-80 |
| UI-11 | App Server tests: `spotify-demo.md:121` and `CHANGELOG.md:101` (tests only inside Docker) vs app-integration skill `:229` (`python -m pytest` on the host). | Resolved: Docker rule wins (also AGENTS.md exception: per-package cargo in worktrees). Fix the skill. | R1-99 |
| UI-12 | Skills vs docs, both stale in places: app-integration skill says `open_url` undocumented and omits `navigate`/`set_app_token`; thtml skill says self-closing is limited to `<img>`/`<input>`, "no `flex: 1`" (it documents `flex`), "No units anywhere" while GUI font/radius properties take `px/dp/em/%/ch/lh`. | Resolved: docs and code win; fix both skills; one consistent units statement in `tcss-reference.md:66` vs `:195`. | R1-89,94,95,100; R1 incons. 16 |
| UI-13 | `README.md:89` calls `apps/oxiterm-reader` an e-book reader; it is a web/HN/Wikipedia/RSS reader; layout table omits `oxiterm-gpu`, `reader-site`, `deploy/`, `scripts/`. | Resolved: fix README. | R1-42,103,113 |
| UI-14 | `app-server-guide.md:309-312` Axum sample uses `axum::extract::TypedHeader` (removed in axum 0.7, which it pins); five "languages" are five frameworks in three languages. | Resolved: fix the sample (axum-extra), reword. | R1-85,86 |
| UI-15 | `getting-started.md` payload example lacks `username`/`auth_method`; env table lacks `OXITERM_LOCAL_MODE`, `--local-mode`, `--a11y`; patch limits (100 keys, 256-char key, 64 KiB string, 1000-item list) and "state = keys bound on the current page" are not in the guide. | Resolved: add. | R1-81,82,83,84 |
| UI-16 | Plan status line `oxi-ui.md:7` and `HANDOFF.md:31`: 5.2, 7.6, Run, identity, showcase, targets, tag missing; Phase 7 table has no 7.6 and no status; `:606` "Nothing here is implemented"; `:463` "Closed 2026-11-24" vs `:501` "not marked done yet"; `:5` links HANDOFF dated 2026-11-25 (file says 2026-11-26). | Resolved by this roadmap (section 3); the plan file is deleted. | R2-01,159; R2 incons. 1,2 |
| UI-17 | Execution order `oxi-ui.md:130-135` vs the parallel schedule `:664-672` vs `:759`. | Needs owner: Q-1 (overtaken by events, not revoked). | R2-09; R2 incons. 3 |
| UI-18 | D3 text `oxi-ui.md:124` ("two sinks, Vello vs custom quads") vs revised D3 `:344-356` and `research/vulkan-acceleration.md`. | Resolved: revised D3 is the decision (O-5). | R2 incons. 13 |
| UI-19 | Phase 4 text: `oxi-ui.md:220` ("aarch64-linux-android not installed") and `:327` ("CI job ... not touched") vs `ci.yml:49-51` building `oxiterm-core` for Android. | Resolved: CI does it. | R2-17; R2 incons. 16 |
| UI-20 | `docs/stages/README.md` lists only 8.T, 8.G, 8.L, 8.D, 8.W, host-export; the directory also has 0.0.1, 5.2, 7.6, identity, run, showcase, targets; stage notes say "lead blesses ... pending" (5.2, blessed `326b53b`), "not tagged" (0.0.1, tag exists), "goldens not blessed" (7.6, `d52bb89`). `CHANGELOG.md:26` points to 8.P/8.R/8.M notes that were never archived. | Resolved: index fully; mark notes; 8.M findings lost (see UI-22). | R1 incons. 8; R2 incons. 15 |
| UI-21 | Pages vs Workers: `HANDOFF.md:3,45`, `sites/DEPLOY.md` (first ~60 lines), 8.W notes say Cloudflare Pages; commit `ddad8b3` and the lead's notes say Workers static assets; `_headers` rules were written for Pages semantics and are unverified under Workers. | Resolved: Workers (O-20). Fix DEPLOY.md, HANDOFF.md; verify headers in U.Sites. | R1-106; R2-127; R2 incons. 9; R1 incons. 20 |
| UI-22 | 8.S4 gate (re-find all 8.G/8.M/8.T/8.L BAD ACTOR findings) cannot be met: 8.M notes were "not archived"; evidence exists only in old worktrees. | Resolved for OxiUI half: U.Hygiene keeps those worktrees until notes are extracted. The gate itself is in the OxiScript half. | R2 incons. 8 |
| UI-23 | `iot-screens.md` sizing example 320x240 vs the board bought (800x480); `iot-screens.md` ESP32 plan predates the step API and display-list wire. | Resolved: U.Screens-3 notes it. | R1-71,72 |
| UI-24 | Concept docs' unmeasured numbers (`<1 ms` diff, RAM 2-15 MB, 18-35 MB, `< 0.5 s` boot) vs only `oxiterm-gpu/README.md` having measurements. | Needs owner: Q-15; stage U.Bench. | R1 incons. 19 |
| UI-25 | Predictive echo: concept 2 specifies a client-side layer; code keeps it server-side (`session_state.rs:41`, `event_loop.rs:1500-1515`); adaptive-on-RTT not found. | Resolved: shipped is server-side; adaptive decision in U.Term. | R1-12; R1 incons. 15 |
| UI-26 | `IDEAS.md` item 5 `:hover`: "architecture defined" vs TCSS spec having only tag/class/id selectors. | Resolved: not implemented, stage U.Hover. | R1 incons. 17 |
| UI-27 | The 5.2 research doc `vulkan-acceleration.md` section 2 facts ("reset_nodes every frame", "PushClip not emitted", "wire v1/v2") overtaken by 5.1b/5.2/6.x. | Resolved: keep as research, add a pointer. | R2 stale |

### B. Stale documents and the action

| Document / section | Problem | Action |
|---|---|---|
| `docs/oxi-ui.md` (whole plan) | Status, phase tables, wave log, parallel schedule, risks, section 2 coupling map (line numbers gone after Phase 4), 3.1 DisplayItem snippet, 4 D3, Phase 4/5/6 narrative are history | **Delete** after the lead has moved the history that is worth keeping (Stage 5.0/5.1 measurement blocks with numbers, Phase 6 stage narrative, Phase 4 step notes) into `docs/stages/`; decisions are carried in oxiui.md section 2 and oxiscript.md. The sites generator must be retargeted first (see Cross-cutting). |
| `docs/HANDOFF.md` | Done list incomplete, steps 1-3 done, Pages vs Workers, worktree note | **Rewrite** short: rules (sections 1-2), operational facts (section 5), pointer to the two roadmaps; drop status and next steps. |
| `docs/IDEAS.md` | Status column stale, spring order, no OxiScript/3D/Terminal+ entries | **Update**: statuses to "see roadmap id", fix `spring`. |
| `docs/ARCHITECTURE.md` sections 5 (tick, predictive echo), 6 (a11y), 11 ("run/build next", "linking the real rc is next") | stale statements; otherwise the best protocol reference | **Update** those sections. |
| `docs/DEMO.md` | tick rates (`:31`, `:280`), no GUI/local modes, superseded in practice by `sites/` | **Update** or fold into `getting-started.md`. |
| `docs/getting-started.md` | UI-10, UI-15 | **Update**. |
| `docs/app-server-guide.md` | UI-14, UI-15 | **Update**. |
| `docs/retro-desktop-environment.md` | section 3.1 obsolete, section 5 numbering, 5 ms poll, unmeasured table | **Rewrite** against 7.2b and U.OxiDE, then keep as the OxiDE blueprint. |
| `docs/embedded-iot-platform.md` | checklist (fb done), duplicates `oxiterm-fb`, unmeasured RAM table, no ESP32 800x480, no OxiScript/targets | **Rewrite** against OxiUI + OxiScript; keep blueprint. |
| `docs/iot-screens.md` | old phase cross-refs (`:41`, `:139`, `:207`, `:305`), predates step API | **Update** refs; keep as the OxiScreens blueprint (U.Screens). |
| `docs/tcss-reference.md` | spring is right; units statement `:66` vs `:195`; `box-shadow` silently ignored but shown in an example | **Update** wording. |
| `docs/thtml-reference.md` | no `alt` semantics for `<img>/<video>` in GUI, no `0x12` features | **Update** (small). |
| `docs/spotify-demo.md`, `examples/spotify/README.md` | the latter is 6 Polish lines | **Fold** the README into the doc, delete the README. |
| `docs/stages/README.md`, individual stage notes | UI-20 | **Update**. |
| `docs/research/vulkan-acceleration.md` | UI-27 | **Update** (pointer). |
| `sites/DEPLOY.md` | first ~60 lines describe Pages | **Rewrite** the head; keep the Workers section, headers/CSP, owner gates. |
| `README.md` | layout table, "run next", add `oxiterm-gpu` | **Update**. |
| `CHANGELOG.md` | duplicate "Unreleased" headers, no release after 0.6.0 | **Update** (U.Ver). |
| `Specyfikacja Terminal+ Amendment.md`, `Terminalowy HTML_CSS_ Nowy Mainframe.md` (repo root) | historical concept documents; mobile section, sprint list, fallback mode, Bypass Diff superseded | **Move** to `docs/history/` with a "superseded, see docs/roadmap" banner. |
| `.claude/skills/thtml-tcss-authoring`, `oxiterm-app-integration` and their `README.md` | UI-1, 2, 11, 12; lists of doc errors partly fixed | **Update**. |
| `.agents/rules/how-to-comment-rust.md` | orphaned, newlines lost | **Link** from AGENTS.md or **delete**. |
| Root-level `test_screenshots/`, `run_site.sh`, `setup_demo.sh`, `bundle_code.sh` | not covered by any inventory item | lead decides (no evidence either way). |

### C. Doc fixes for the lead (mechanical)

1. `CHANGELOG.md:64`, `docs/IDEAS.md:60`, thtml skill `:42`: `spring(stiffness, damping, mass)`.
2. thtml skill `:37`: `position` default is `static`.
3. `docs/ARCHITECTURE.md:68`, `docs/DEMO.md:31,280`, `docs/retro-desktop-environment.md:127`: 16 ms while animating, 1000 ms idle.
4. `docs/ARCHITECTURE.md:247`, `README.md:75`: `oxi run`/`oxi build` and playground Run are shipped.
5. `docs/getting-started.md:11-16`: wasm-pack step before `cargo build`; add `?gui=1`, `?local=1`, `--local-mode`, `--a11y`.
6. `sites/DEPLOY.md`: Workers static assets (not Pages); `docs/HANDOFF.md:3,45` likewise.
7. `README.md:89`: reader is a web reader; add `oxiterm-gpu` to the layout table.
8. `docs/app-server-guide.md` Axum sample; payload fields; patch limits.
9. App-integration skill: tests in Docker; `open_url`, `navigate`, `set_app_token` are documented; thtml skill: self-closing, `flex`, units.
10. `docs/stages/0.0.1/release.md` "tagged"; `docs/stages/5.2` "blessed"; `docs/stages/7.6` "blessed"; `docs/stages/README.md` full index.
11. Delete after retargeting the generator: `docs/oxi-ui.md`.
12. Prune merged worktrees (U.Hygiene) but keep `gemini-rtlink`, `gemini-std`, `gemini-memres` and the old 8.M worktree until its notes are extracted.

## OxiScript half (W-OXS)

_Pending: W-OXS writes this to scratch/roadmap/consistency-oxs.md; merged here by W-UI._
