# Stage records

Working notes written by the agents during each stage, kept as history: what was built, what the BAD ACTOR pass found, how each finding was fixed. They are **not** current documentation. Numbers, file names and test counts in them describe the moment they were written; the code and the docs in `docs/` win.

| Stage | Contents |
|---|---|
| [8.T](8.T/) | Type checker: implementation notes, BAD ACTOR findings (22), fixes |
| [8.G](8.G/) | Wasm code generator: implementation, BAD ACTOR (6 findings, 0 miscompilations), fixes |
| [8.L](8.L/) | Lowering (end-to-end compiler): implementation, BAD ACTOR (0 miscompilations in ~19.6k differential programs), fixes |
| [8.D](8.D/) | DevKit v0 (`oxi` CLI, VS Code extension, AGENTS.md): implementation, BAD ACTOR (17 findings), fixes + semicolon assistant |
| [8.W](8.W/) | Websites (slavekm.pl, oxiui, oxiscript) and the playground: implementation, BAD ACTOR (12 findings), fixes |
| [host-export](host-export/) | Owner decision 2026-11-26: `export` = module visibility, host entry points are `@host` |

Earlier stages (8.C, 8.P, 8.R, 8.M, 7.1, 7.2) kept their notes in their worktrees only; their results are summarised in the wave log of [../oxi-ui.md](../oxi-ui.md).

From now on agents write their notes directly to `docs/stages/<stage>/` in their worktree.
