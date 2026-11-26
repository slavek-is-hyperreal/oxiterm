# Roadmaps

Two roadmaps replace every earlier plan of this repository (`docs/oxi-ui.md` as plan, `docs/HANDOFF.md` status, the status lists in `docs/IDEAS.md`, the roadmap checklists in `embedded-iot-platform.md`, `iot-screens.md` and `retro-desktop-environment.md`, and the concept documents in the repository root). They link to each other.

| File | Covers |
|---|---|
| [oxiui.md](oxiui.md) | OxiTerm / OxiUI: renderer, `oxiterm-core`, server, web and local mode, GPU sink, OxiDE, accessibility, devices and screens, apps, CI, site infrastructure, release |
| [oxiscript.md](oxiscript.md) | OxiScript: language, compiler, runtime, standard library, tooling, data parallelism (SIMD/GPU), boards, websites content and course |
| [consistency.md](consistency.md) | every inconsistency found between documents and code (resolved how, or needs the owner), every stale document with the action, doc fixes for the lead, cross-cutting items |

## How ids work

- A stage id is unique across both files. Existing plan ids are kept (`5.3`, `7.2b`, `8.S1`, `8.3c`, ...). New stages use a prefix: `U.<Name>` in the OxiUI roadmap, `8.<Name>` in the OxiScript roadmap. Renamed ids are listed in section 5 of each file as "old -> new".
- Every stage is a heading with an explicit anchor: `<a id="s-<id>"></a>`, id lowercased and "." replaced by "-". Link as `[8.S1](oxiscript.md#s-8-s1)` or `[5.3](oxiui.md#s-5-3)`.
- Principles are `oxiscript.md#p-1`..`p-11`; OxiScript owner decisions are `oxiscript.md#d-1`..; OxiUI owner decisions are `oxiui.md#o-1`..; OxiUI open questions are `oxiui.md#q-1`..; OxiScript open questions are grouped by source in `oxiscript.md` (`#q-simd`, `#q-scb`, `#q-dx`, `#q-ob`, `#q-safe`, `#q-lang`, `#q-plan`, items `Q-OB-1`, `Q-PLAN-1`, ...). A question with an owner in the other roadmap is a one-line link there.
- Each roadmap row ends with `Sources:`, ids of the reader inventories (`R1`..`R4` in `scratch/roadmap/`), so coverage can be checked with `grep -o 'R[1-4]-[0-9]*'`.
- Each shared item has exactly one owner roadmap and a link from the other.

## Shared items and their owner

| Item | Owner | Linked from |
|---|---|---|
| `<script lang="oxs">` in OxiUI pages | OxiScript: 8.S1 (language, runtime); OxiUI: U.S1 (parser and page side) | both |
| GPU compute context 8.3c, kernels 8.3d, tensor 8.3e | OxiScript | oxiui.md 5.3, U.KernelDraw, 9.x |
| GPU sink 5.2-5.7, browser GPU painter U.WebGPU, Phase 9 | OxiUI | oxiscript.md 8.3c |
| Sites infrastructure, deploy, headers, generator input | OxiUI: U.Sites | oxiscript.md 8.W* |
| Sites content, playground, course, blog | OxiScript: 8.W1-8.W3, 8.Blog | oxiui.md U.Sites |
| CI workflow, `cargo deny`, release tags | OxiUI: U.CI | oxiscript.md 8.Sec |
| Gate script and nightly fuzz of OxiScript | OxiScript | oxiui.md section 8 |
| ESP32 board, flasher | OxiScript: 8.S3, 8.S3w | oxiui.md U.Screens-3 |
| Small-memory measurements (Pi 1 B+) | OxiScript: 8.Mem | oxiui.md U.Bench, U.Damage |
| Damage tracking | OxiUI: U.Damage (conditional on 8.Mem) | oxiscript.md 8.Mem |
| `host:dom` | OxiScript: 8.Dom | oxiui.md section 1 |
| Versioning | OxiScript: oxiscript-v*; OxiUI: U.Ver, `v*` | both |
| Repository split | OxiScript: 8.Repo | oxiui.md O-13 |

## Machine-readable status

`oxiui.md` section 3 (Done) has the stage id in the first cell and the merge date plus 7-hex hash in the last cell of each row. `oxiscript.md` keeps a table with the columns `Stage | Status | Merged`. See consistency.md "Cross-cutting" for what `sites/build.mjs` has to read.
