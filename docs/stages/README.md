# Stage records

Working notes written by the agents during each stage, kept as history: what was built, what the BAD ACTOR pass found, how each finding was fixed. They are **not** current documentation. Numbers, file names and test counts in them describe the moment they were written; the code and the docs in `docs/` win. The current plans are the roadmaps: [OxiUI](../roadmap/oxiui.md), [OxiScript](../roadmap/oxiscript.md).

| Directory | Roadmap id | Contents |
|---|---|---|
| [0.0.1](0.0.1/) | [8.Rel](../roadmap/oxiscript.md#s-8-rel) | OxiScript 0.0.1 release (versions, version script, tag `oxiscript-v0.0.1`, CHANGELOG) |
| [5.2](5.2/) | [5.2](../roadmap/oxiui.md#s-5-2) | Headless GPU sink on wgpu (`oxiterm-gpu`): implementation, review, BAD ACTOR, fixes, CI |
| [7.6](7.6/) | [7.6](../roadmap/oxiui.md) | Media and diagram parity across modes: implementation, BAD ACTOR, fixes |
| [8.T](8.T/) | [8.T](../roadmap/oxiscript.md#s-8-t) | Type checker: implementation notes, BAD ACTOR findings (22), fixes |
| [8.G](8.G/) | [8.G](../roadmap/oxiscript.md#s-8-g) | Wasm code generator: implementation, BAD ACTOR (6 findings, 0 miscompilations), fixes |
| [8.L](8.L/) | [8.L](../roadmap/oxiscript.md#s-8-l) | Lowering (end-to-end compiler): implementation, BAD ACTOR (0 miscompilations in ~19.6k differential programs), fixes |
| [8.D](8.D/) | [8.D0](../roadmap/oxiscript.md#s-8-d0) | DevKit v0 (`oxi` CLI, VS Code extension, AGENTS.md): implementation, BAD ACTOR (17 findings), fixes + semicolon assistant |
| [8.W](8.W/) | [8.W1](../roadmap/oxiscript.md#s-8-w1) | Websites (slavekm.pl, oxiui, oxiscript) and the playground: implementation, BAD ACTOR (12 findings), fixes |
| [run](run/) | [8.Run](../roadmap/oxiscript.md#s-8-run) | `oxi run` / `oxi build` and the playground Run button: implementation, BAD ACTOR, fixes |
| [identity](identity/) | [8.Id](../roadmap/oxiscript.md#s-8-id) | Site identity (palette, mark, mascot) |
| [host-export](host-export/) | [8.HX](../roadmap/oxiscript.md#s-8-hx) | Owner decision 2026-11-26: `export` = module visibility, host entry points are `@host` |
| [targets](targets/) | [8.2a](../roadmap/oxiscript.md#s-8-2a) | Compilation targets `device` / `web` / `server`: implementation, BAD ACTOR, fixes |
| [8.Std](8.Std/) | [8.Std](../roadmap/oxiscript.md#s-8-std) | v0 standard library (`std/math`, `std/string`, `std/array`, `std/result`, `std/collections`): implementation, BAD ACTOR, fixes |
| [showcase](showcase/) | Showcase (done, see [oxiui.md section 3](../roadmap/oxiui.md#3-done-verified)) | OxiUI versions of the three sites: implementation, BAD ACTOR, fixes |

Stages 8.C, 8.P, 8.R and 8.M (OxiScript) and 7.1 and 7.2 (OxiUI) have no stage directory: their notes were never archived, and for the four OxiScript stages they exist in no worktree either. Their results are in the wave log (section 11 of [oxiscript.md](../roadmap/oxiscript.md)); the OxiUI Phase 0-7 narratives are in [../history/oxi-ui-plan-history.md](../history/oxi-ui-plan-history.md).

Agents write their notes directly to `docs/stages/<stage>/` in their worktree.
