# Consistency, stale documents and doc fixes

Written 2026-11-26 against main 19cf8cd. Two halves (OxiUI by W-UI, OxiScript by W-OXS) plus cross-cutting items. Nothing here has been applied: the lead deletes or edits the old documents after review. Roadmaps: [oxiui.md](oxiui.md), [oxiscript.md](oxiscript.md), index [README.md](README.md).

## Cross-cutting

| # | Item | Resolution / action |
|---|---|---|
| X-1 | **Roadmap generator.** `sites/build.mjs` (`roadmapData`, lines 117-170) reads only `docs/oxi-ui.md`. It finds phases with `^### (Phase \d+): (.+)$` (state `done` if the heading has "✅ done", `planned` if it has "planned" or "draft"), the **wave-log table** by header (`Slice` | `Implementation`, six cells; a slice is done if the sixth cell, `Merged`, holds a 7-hex hash or a date and contains no em dash, active if it has an em dash, planned if no row starts with the slice id), the **slices table** by header (`Slice` | `Crate / area`), and the **Phase 7 table** (`Stage` | `Scope`, with a row starting `7.1`). It throws if the tables are missing. | When the lead deletes `docs/oxi-ui.md` the build fails. Both roadmaps expose the same table `Stage | Status | Merged` (stage id in the first cell; `Merged` is `date, hash` or an em dash), so one parser handles both: `oxiui.md` section 3 (Done rows) plus its section 5 status table, and the corresponding tables in `oxiscript.md`. Titles and scopes come from the stage headings (`#### <id> <title>`). Do not delete `oxi-ui.md` before this is changed (stage U.Sites). Also add rows for stages that never had a wave-log row (5.2, 7.6, Run, identity, showcase, targets, 0.0.1): they are in both new files. |
| X-2 | "Decision 4" in the SIMD/GPU research (`oxiscript-simd-gpu-design.md:12` and the "[Revised ... for decision 4 transparency]" markers) means *principle* 4 of the plan (`oxi-ui.md:621`); *decision* 4 (`:715`) is "GPU in v1: no". | Resolved in the new numbering: `oxiscript.md#p-4` vs `#d-4`. Fix the research document's wording. Source: R4 incons. 1. |
| X-3 | 8.M-link has no row in the old plan although `8.Perf`, `8.Std`, `8.Mem`, 8.S1 and 8.3d depend on it. | Resolved: a stage in [oxiscript.md](oxiscript.md#s-8-m-link). Source: R2 incons. 4. |
| X-4 | Old "8.2" meant both targets (done) and AOT/profiles (todo); old "Stage 8.0" was named four times but never a row. | Resolved by W-OXS ids: 8.2a (targets, done), 8.2b (AOT), 8.2c (profiles), 8.0 (measurement spike). Source: R2 incons. 7. |
| X-5 | Pages vs Workers (UI-21). | Owner roadmap: oxiui.md U.Sites. |
| X-6 | The input document of 8.Perf, `docs/research/oxiscript-memory-performance.md`, exists only as an uncommitted file in the worktree `gemini-memres`; the plan links it and the link is dead on `main`. | Commit it from that worktree before pruning (U.Hygiene); linked by [8.Perf](oxiscript.md#s-8-perf). Source: R2 incons. 5; R4. |
| X-7 | Stage notes live in `docs/stages/<id>/`; dir names `run`, `identity`, `showcase`, `targets`, `host-export`, `0.0.1` have no plan id. | Resolved: ids 8.Run, 8.Id, `Showcase`, 8.2a, 8.HX, 8.Rel in the new files; index them in `docs/stages/README.md`. |
| X-8 | Shared GPU context: `GpuContext` today exposes `device()`/`queue()` publicly and `with_error_scopes` is `pub(crate)`; 8.3c changes this, 5.3 must not break it. | Requirement recorded in oxiui.md 5.3 and [8.3c](oxiscript.md#s-8-3c). |
| X-9 | R2 said the 8.C/8.P/8.R/8.M/7.1/7.2 stage notes exist only in old worktrees. W-OXS checked: for the OxiScript stages they exist nowhere (not in main history, not in any worktree). | The 8.S4 gate is amended in oxiscript.md (Q-PLAN-1); U.Hygiene need not archive those notes, but must still not delete `gemini-memres` before its research file is merged. |
| X-10 | The memres research document is now committed on branch `gemini-memres` (`e4b9f7d`) but is not on `main`. | Merge after verification; until then links to it are dead. |

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
| UI-22 | 8.S4 gate (re-find all 8.G/8.M/8.T/8.L BAD ACTOR findings) cannot be met: 8.M notes were "not archived"; evidence exists only in old worktrees. | Notes do not exist in any worktree (X-9); gate amended in the OxiScript half. | R2 incons. 8 |
| UI-23 | `iot-screens.md` sizing example 320x240 vs the board bought (800x480); `iot-screens.md` ESP32 plan predates the step API and display-list wire. | Resolved: U.Screens-3 notes it. | R1-71,72 |
| UI-24 | Concept docs' unmeasured numbers (`<1 ms` diff, RAM 2-15 MB, 18-35 MB, `< 0.5 s` boot) vs only `oxiterm-gpu/README.md` having measurements. | Needs owner: Q-15; stage U.Bench. | R1 incons. 19 |
| UI-25 | Predictive echo: concept 2 specifies a client-side layer; code keeps it server-side (`session_state.rs:41`, `event_loop.rs:1500-1515`); adaptive-on-RTT not found. | Resolved: shipped is server-side; adaptive decision in U.Term. | R1-12; R1 incons. 15 |
| UI-26 | `IDEAS.md` item 5 `:hover`: "architecture defined" vs TCSS spec having only tag/class/id selectors. | Resolved: not implemented, stage U.Hover. | R1 incons. 17 |
| UI-27 | The 5.2 research doc `vulkan-acceleration.md` section 2 facts ("reset_nodes every frame", "PushClip not emitted", "wire v1/v2") overtaken by 5.1b/5.2/6.x. | Resolved: keep as research, add a pointer. | R2 stale |

### B. Stale documents and the action

| Document / section | Problem | Action |
|---|---|---|
| `docs/oxi-ui.md` (whole plan) | Status, phase tables, wave log, parallel schedule, risks, section 2 coupling map (line numbers gone after Phase 4), 3.1 DisplayItem snippet, 4 D3, Phase 4/5/6 narrative are history | **Delete** after the lead has moved the history that is worth keeping (Stage 5.0/5.1 measurement blocks with numbers, Phase 6 stage narrative, Phase 4 step notes) into `docs/stages/`; decisions are carried in oxiui.md section 2 and oxiscript.md. The sites generator must be retargeted first (see Cross-cutting). |
| `docs/HANDOFF.md` | Done list incomplete, steps 1-3 done, Pages vs Workers, worktree note (see X-9) | **Rewrite** short: rules (sections 1-2), operational facts (section 5), pointer to the two roadmaps; drop status and next steps. |
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
12. Prune merged worktrees: executed by U.Hygiene using the list in the OxiScript half C.9; keep `gemini-rtlink`, `gemini-std`, `gemini-memres` until merged (X-9, X-10).

## OxiScript half

Written by W-OXS, 2026-11-26, against `main` at 19cf8cd. Source ids `[Rn-xx]` refer to the reader inventories. Each inconsistency says how it is resolved in [oxiscript.md](oxiscript.md) or that the owner must decide. "Lead" means the lead agent.

### A. Inconsistencies

#### A.1 Numbering and naming

| # | Inconsistency | Resolution | Source |
|---|---|---|---|
| A1 | "Decision 4" names two things: the SIMD/GPU research and its revision markers cite "decision 4 transparency" (which is Principle 4 of Phase 8), while Decision 4 of the old list is "GPU in v1: no". | One scheme in oxiscript.md: principles `P-1..P-11`, decisions `D-1..D-13`, later decisions `D-14+`. Mapping: "decision 4 (transparency)" -> `P-4`; "decision 4 (GPU in v1)" -> `D-4`. The SIMD/GPU report text should be edited to say "principle 4" (lines 12 and the revision markers). | R4 inconsistency 1 |
| A2 | Old plan row "8.2" is one stage for AOT, profiles and targets; the targets stage used the name "8.2" for itself and only that part is merged. | Split: `8.2a` targets (done), `8.2b` AOT, `8.2c` profiles. Old name -> new name table in oxiscript.md section 5. | R2 inconsistency 7, R4-51 |
| A3 | "Stage 8.0" (research spike) is named four times in the old plan but is not a row; "8.1" never existed; decision 1 says Stage 8.0 measures RC overhead with elision, 8.Perf also claims it. | `8.0` kept as the measurement stage; the "RC overhead with elision" measurement moved to `8.Perf`. | R2-84, R4-62, R2 inconsistency 19 |
| A4 | 8.M-link is a dependency of 8.Perf, 8.Std, 8.Mem, 8.S1 in practice but has no row; 8.S1 lists only "8.L, 8.R, 7.1"; 8.N depends on "8.M". | `8.M-link` is a stage; it is a dependency of 8.S1, 8.Std, 8.Perf, 8.N, 8.Mem, 8.3d. | R2 inconsistency 4 |
| A5 | 8.3b meant "GPU kernels" in `docs/stages/5.2` and in the SIMD report scope text (lines 15, 23, 253); the plan renamed 8.3b to the auto-vectoriser and GPU to 8.3c-8.3e. | oxiscript.md uses only the new names. The report lines and the 5.2 notes are stale (see B). | R2 inconsistency 7, R4 inconsistency 5 |
| A6 | Reuse table says `ena` for 8.T; 8.T did not use it (no inference variables); salsa "evaluate first" never done. | Table kept in oxiscript.md section 10 with the `ena` row corrected; salsa decided in 8.S2. | R2-104, R2 inconsistency 6 |
| A7 | Stage directories `run`, `identity`, `host-export`, `targets`, `0.0.1`, `showcase` have no plan id. | Ids `8.Run`, `8.Id`, `8.HX`, `8.2a`, `8.Rel`; showcase is OxiUI. | R2 inconsistency 7 |

#### A.2 Spec against spec

| # | Inconsistency | Resolution | Source |
|---|---|---|---|
| A8 | `grammar.md` gives `OXS-E0100` for `===` on a struct/union/array; `contracts.md`, `llms.txt` and the registry say `E0110`. | Registry wins: `E0110`. Fix in 8.Spec. | R3 inc. 1 |
| A9 | `grammar.md` gives `E0105` for `?.` on a non-nullable operand; registry and `contracts.md`: `E0127`. | Registry wins. 8.Spec. | R3 inc. 2 |
| A10 | `grammar.md` says "`OXS-W0001` family" for a no-effect statement; registry: `W0001` unused binding, `W0004` no effect. | Registry wins. 8.Spec. | R3 inc. 3 |
| A11 | `grammar.md` gives `E0100` for "must return on every path"; registry: `E0114`. | Registry wins. 8.Spec. | R3 inc. 4 |
| A12 | `contracts.md` decision 2 says switch expressions use `=>` arms; `grammar.md` says arms use `:` and `=>` is `E0001`. | Grammar wins (it is what the parser implements). 8.Spec. | R3 inc. 5 |
| A13 | `abi.md` lists exceptions (exnref) as a `web`/`server` target feature; `contracts.md` calls EH an "opt-in ABI minor bump". | Owner decision D-24: opt-in ABI minor. One sentence in `abi.md` whether `oxi_abi_version` changes: 8.Spec decides, 8.6 implements. | R3 inc. 6 |
| A14 | `grammar.md` section 6 numbering skips rows 14 and 15 while other documents cite rows by number. | Do not renumber; add a note "rows 14 and 15 were removed". 8.Spec. | R3 inc. 7, R3-24 |
| A15 | `readonly`, `private`, `static` have no stated semantics on struct copies. | Owner question `Q-LANG-3`; 8.Fix decides and documents. | R3 inc. 8, R3-210 |
| A16 | `contracts.md` section 2 convention list is numbered 1-9, 11, 12, 10. | Renumber in the rewrite of `contracts.md` (history). | R3 stale list |
| A17 | Research (`oxiscript-language.md`) recommends `unknown` and `int`/`float` aliases; `grammar.md` reserves `unknown` (`E0900`) and has no aliases; no rejection recorded. | Question `Q-LANG-5`. | R3 inc. 18, 19 |
| A18 | Principle 4 says "SPIR-V/MSL via naga" and "threads"; the verified SIMD/GPU report emits WGSL text only, naga as validator only, and puts threads in 8.4. Principle 7 says "wasmi or WAMR", D-3 says wasmi. Principle 10 says "query-based front end", no Salsa exists. | Owner texts carried unchanged; conflicts annotated under the principles in oxiscript.md; open as `Q-SIMD-5`, `Q-ENG-1` and the 8.S2 decision. | R4 inc. 9, R2 inc. 11 |
| A19 | `sync-callbacks.md` frames 8.N as native bindings needing sibling instances and ABI 1.1; the plan and `grammar.md` 2.7 (later) link crates statically (D-14). | Document partly superseded: kept as the reference for kind-D native bindings (idea, section 5.7). Mark it (see B). | R3 inc. 16 |
| A20 | Dev-experience research and `llms.txt` claim fix coverage (">50% machine-applicable"); `.length` and `as` fixes are `maybe-incorrect`. | 8.D1 gate measures the share. | R3-235 |
| A21 | `oxibench-methodology.md` uses `[u8]` in a signature; the language type is `u8[]`. | Doc fix (C list). | R3 inc. 17 |

#### A.3 Spec against code and tooling

| # | Inconsistency | Resolution | Source |
|---|---|---|---|
| A22 | `templates/common/AGENTS.md:185` and `SKILL.md:23` say `oxi run`/`oxi build` are not available; the CLI, `SCHEMA.md`, `LOG.md` and the about string say they exist; root `AGENTS.md` omits the sentence. Every `oxi new` project ships the wrong guide. | Doc fix (C list), then `sync_docs.py`. 8.Spec. | R3 inc. 9, R1 inc. 7 |
| A23 | `llms.txt` lists `OXS-E0500` (no code emits it) and omits `E0400`/`E0402`. | 8.Spec: generate by a stated rule; `E0500` listed when 8.2c emits it. | R3 inc. 10 |
| A24 | `ir.md` documents `#effects(alloc, panic, loops, recursion, ...)`; the checker computes only `uses_host`, `reads/writes_globals`, `mutates_heap`, `calls_unknown`. Profiles and `@expect(no_alloc)` have nothing to read. | Doc says which are computed now; computing the rest is part of 8.2c. | R3 inc. 11 |
| A25 | `abi.md`, `grammar.md`, `llms.txt` describe profile restrictions as checked; the checker only validates the `@profile` argument. `@expect` is registered but nothing evaluates it. | 8.2c (profiles) and 8.S4b (report, `@expect`). Docs say "not yet enforced" until then. | R3 inc. 12, 13 |
| A26 | `${f64}` accepted by the checker, traps under the Stub runtime (`oxi_str_from_f64` exists in `oxiscript-rc`). | Resolved by 8.M-link. Docs (`ir.md:102`, `SCHEMA.md:109`, playground) updated after merge. | R3 inc. 14 |
| A27 | Known-attacks recommends bidi controls be an error; the lexer emits only warning `W0003` inside literals. | `Q-LANG-4`; 8.Sec. | R3 inc. 15 |
| A28 | The SIMD report says wasmi `simd` is not enabled (line 64, ledger B6); main enables `simd` and `memory64` (targets merge). Report also lists 8.2 targets and 8.M linking as planned. | Report stale (see B). Whether `relaxed_simd(false)` is forced and tested is an 8.3a gate item. | R4 inc. 4, R4-07 |
| A29 | Plan row 8.S4 gate requires re-finding every BAD ACTOR finding of 8.G, 8.M, 8.T and 8.L on their original repros; no stage notes exist for 8.C, 8.P, 8.R, 8.M anywhere (not in `main` history, not in any worktree: checked). R2 said they live in worktrees; they do not. | Gate amended in oxiscript.md 8.S4c; confirmation `Q-PLAN-1`. Before the lead prunes worktrees: nothing to archive for these stages. | R2 inc. 8, R2 stale |
| A30 | Plan links `research/oxiscript-memory-performance.md` (dead on main); and "targets fixes.md F2" (the file is `docs/stages/targets/fixes.md`, exists). | The memres document is committed on `gemini-memres` (e4b9f7d); it must be merged to main after Opus verification. The F2 link is fine. | R2 inc. 21, R2 inc. 5 |
| A31 | `docs/stages/targets` notes: no code path emits a new Wasm feature on any target, yet the plan says SIMD/EH "arrive as first web/server features". | Consistent: first user is 8.3a. Recorded in 8.2a remaining items. | R2-100 |
| A32 | Two versions of "where the version is stated": `SCHEMA.md` example shows `"version": "0.0.1"` while the patch grows with code. | Harmless; the example ages. Note in 8.Spec release item. | R3 inc. 22 |
| A33 | Three copies of agent guidance (`AGENTS.md`, `templates/common/AGENTS.md`, `llms.txt`) are synced by `sync_docs.py` for the shared block only; the skill is hand-maintained with its own code table. | 8.Spec: sync the skill table or generate it. | R3 inc. 23 |
| A34 | The 8.Std branch base predates the targets merge; merging naively deletes `oxiscript-wasm/tests/targets.rs` and a gate step. | 8.Std gate: rebase first. | R2 inc. 17 |
| A35 | Bounds: syntax `DEFAULT_MAX_DEPTH` 56 against checker `MAX_EXPR_DEPTH` 120. | Different layers; not an error. Documented in `contracts.md` rewrite. | R3-54 |

#### A.4 Needs the owner

`Q-LANG-1..5` (whitespace `?`, grammar section 7 and contracts section 4 sign-offs, `readonly` semantics, bidi error, `int`/`float` aliases and `unknown`); `Q-PLAN-1` (8.S4c gate); `Q-ENG-1` (ESP32 release engine); `Q-SIMD-5` (SPIR-V/MSL tail of principle 4); `Q-ABI-1` (confirm the heap-base and trap-channel decisions the rtlink code took); `Q-PROC-1` (Gemini as default implementer). Full list with sources: oxiscript.md section 7.

### B. Stale documents and sections, with action

Action words: rewrite, update, delete (after the roadmaps are accepted), mark-superseded (add a banner, keep), keep.

| Document | Stale part | Action |
|---|---|---|
| `docs/oxi-ui.md` Phase 8 section (lines 605-738) and the whole old plan | Status line, "Nothing here is implemented", rows marked "(now)", missing 8.M-link, parallel schedule, wave log, reuse table, risks | delete after review: principles, decisions, rows, reuse table, wave log and risks are carried over into oxiscript.md sections 2, 5, 8, 10, 11. Note: `sites/build.mjs stateOf` reads the wave-log table of this file; see the cross-cutting section. |
| `docs/HANDOFF.md` | Done-list incomplete (misses 5.2, 7.6, Run, identity, showcase, targets, 0.0.1); next steps 1-3 already done; "link real 8.M runtime" in progress; "Cloudflare Pages" | delete or rewrite as a 15-line operations note (rules, machine limits, owner open items); the plan content now lives in the roadmaps. |
| `docs/stages/README.md` | Lists only 8.T, 8.G, 8.L, 8.D, 8.W, host-export; the directory also has 0.0.1, 5.2, 7.6, identity, run, showcase, targets | update (index of all directories, with the roadmap ids) |
| `docs/stages/0.0.1/release.md` | "prepared, not tagged" (tag `oxiscript-v0.0.1` exists); CI tag trigger finding | update |
| `docs/stages/5.2/*`, `7.6/*` | "lead blesses goldens pending" (done); name "8.3b kernels" | update (small) |
| `docs/stages/8.D/*`, `8.T/*`, `8.G/*` | Unchecked boxes, "final report only remains" | update (small) |
| `docs/oxiscript/grammar.md` | Header "contract of slice 8.C (2026-11-24)"; section 7 without sign-offs; wrong codes (A8-A11); numbering gap | update (8.Spec) |
| `docs/oxiscript/contracts.md` | Sections 4-5 describe the 8.C state; `=>` arms; list numbering | rewrite sections 4-5 as history (8.Spec) |
| `docs/oxiscript/ir.md` | line 102 `from_f64` status; section 7 duplicates contracts 3; effects vocabulary | update (8.Spec after 8.M-link) |
| `docs/oxiscript/abi.md` | line 5 feature list redundancy; `result<str>` cap; EH statement; heap base and trap channel | update |
| `docs/oxiscript/llms.txt` | code list incomplete | update (generated) |
| `docs/oxiscript/VERSIONING.md` | none | keep |
| `oxiscript/oxiscript-cli/templates/common/AGENTS.md`, `.claude/skills/oxiscript-authoring/SKILL.md` | "`oxi run`/`oxi build` not available"; skill table drift | update, then run `tools/sync_docs.py` |
| `oxiscript/oxiscript-cli/SCHEMA.md` | `${f64}` limitation after 8.M-link; version example | update later |
| `editors/vscode-oxiscript/README.md` | none now ("No LSP yet" true) | keep |
| `tests/agent_test/LOG.md` | dated record; not a clean-room run | keep (re-run in 8.D1) |
| `docs/research/oxiscript-language.md` | Phased roadmap and estimates superseded; architecture table names `oxiscript-front`; Safari relaxed-SIMD claim lines 320, 341; numbering of questions; many `[S]/[U]` claims unverified | mark-superseded (roadmap and estimates); fix lines 320, 341 |
| `docs/research/oxiscript-known-attacks.md` | Every row `S`/`R`; per-slice lists predate real crates; no status of regression tests | keep as catalogue; mark with a status column (8.Spec/8.Sec) |
| `docs/research/oxiscript-dev-experience.md` | Status column ("planned", "8.P merged"), "run/build wired when merged" | update status; keep |
| `docs/research/oxiscript-sync-callbacks.md` | Premise overtaken by D-14; reference numbers 28/29 unused | mark-superseded (keep for kind-D native bindings) |
| `docs/research/oxibench-methodology.md` | Draft v0.1 blocked on owner; `[u8]`; `oxi run` now exists so C3 feasible | update (8.OB) |
| `docs/research/oxiscript-simd-gpu-design.md` | Old stage names (lines 3, 8, 10, 15, 23, 253), "decision 4" wording (line 12 and markers), wasmi `simd` "not enabled" (line 64), 8.2 targets and 8.M linking shown as planned (lines 728-738) | update (small edits) |
| `docs/research/oxiscript-simd-gpu-design-verification.md` | ledger item B6 outdated (runtime now enables simd) | update one row |
| `docs/research/oxiscript-memory-performance.md` (on `gemini-memres`) | Unverified Gemini draft | keep; verify, then merge to main |

### C. Doc fixes for the lead (mechanical, safe to apply now)

1. `docs/oxiscript/grammar.md`: line 325 `OXS-E0100` -> `OXS-E0110`; line 327 `OXS-E0105` -> `OXS-E0127`; line 272 "`OXS-W0001` family" -> `OXS-W0004`; line 274 `OXS-E0100` -> `OXS-E0114`; add "rows 14 and 15 were removed" to section 6.
2. `docs/oxiscript/contracts.md` line 80 (decision 2): "`=>` arms" -> "`:` arms".
3. `oxiscript/oxiscript-cli/templates/common/AGENTS.md:185` and `.claude/skills/oxiscript-authoring/SKILL.md:23`: delete "`oxi run` and `oxi build` are not available...", then `python3 -I oxiscript/oxiscript-cli/tools/sync_docs.py`.
4. `docs/research/oxibench-methodology.md:30`: `[u8]` -> `u8[]`.
5. `docs/research/oxiscript-simd-gpu-design.md`: "decision 4" -> "principle 4" (line 12 and revision markers); stage names at lines 3, 8, 10, 15, 23, 253.
6. `docs/research/oxiscript-language.md:320,341`: remove "relaxed SIMD in all three engines" / Safari flag claim.
7. `docs/stages/0.0.1/release.md`: "not tagged" -> tagged `oxiscript-v0.0.1`.
8. Merge `gemini-memres` research doc after verification so that the link in the plan (and in oxiscript.md 8.Perf) is not dead.
9. Worktree cleanup (checked with `git rev-list --count main..HEAD` and `git status --porcelain` per worktree, 2026-11-26 evening): all 19 `agent-*` worktrees plus `gemini-simd`, `gemini-5.2`, `integ-sites` and `targets` have 0 commits ahead of `main` and are clean, except `agent-a736d876` (untracked `.wrangler/`). Four are locked by an agent session (`agent-a402933a`, `agent-a5c23966`, `agent-a736d876`, `agent-ad24f942`); `agent-aaafc3f9` is unlocked and merged but absent from the earlier inventory, so confirm it is not a live session before pruning. Do NOT prune `gemini-rtlink` (3 commits ahead, 24 uncommitted files), `gemini-std` (2 ahead, 12 uncommitted files), `gemini-memres` (1 ahead, 1 uncommitted file) or `roadmaps`. The Gemini BAD ACTOR scratch directories under `/vectorlegis_ssd_pool/oxiterm/scratch/` (`badactor-rtlink`, `ba-std`) contain only test temp dirs, no stage notes.
10. `oxiscript/oxiscript-cli/SCHEMA.md` and the playground text about `${f64}` after 8.M-link merges.
