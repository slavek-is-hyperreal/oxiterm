# Handoff: OxiTerm / OxiUI / OxiScript, operations note

This file lets another Claude account (or a person) take over without the previous session's memory. It holds rules and operational facts only. **Status, plans, open questions and the order of work are in [docs/roadmap/](roadmap/README.md)** ([oxiui.md](roadmap/oxiui.md), [oxiscript.md](roadmap/oxiscript.md)); this file does not repeat them.

## 1. The owner and how to work with them
- **Language:** the owner writes in Polish; answer in Polish, write code, docs and commits in English.
- **Repository:** github.com/slavek-is-hyperreal/oxiterm (public), branch `main`. Every change is committed and pushed. Commit messages end with a `Co-Authored-By:` line for the model that made them.
- **Licence and business:** MIT OR Apache-2.0. Revenue from support, compliance certification (IEC 62443, EU CRA) and voluntary donations (buycoffee.to/smajcher). **Dependencies: permissive licences only** (MIT/Apache/BSD/Zlib/ISC; MPL only if recorded; fonts OFL/Apache; never GPL/AGPL/LGPL).
- **Decisiveness:** the owner prefers decisions now over long option lists, and delegates taste decisions (fonts, colours, visual calls) to the lead after screenshots. Stop only for decisions with real consequences (licence, money, scope, language semantics).
- **Inform, don't forbid:** useful trade-offs that weaken a guarantee (e.g. linking an `unsafe` Rust crate) are documented in the spec, announced at build time and forbiddable by a profile, not banned.
- **Don't claim a gap without checking the repo** (grep code and docs first).
- **Report outcomes from independent evidence:** after every push check GitHub CI (`gh run list`) and report its result with the link; a local "all green" from an agent is a claim, not proof (on 2026-11-25 CI was red for hours unnoticed, and an agent reported green while two test files failed).

## 2. How work is done
- **Lead:** the main conversation (Opus) designs, writes briefs, reviews, merges and keeps the roadmaps.
- **Implementers:** Sonnet subagents in their own git worktrees, using their `advisor` tool (Opus) before starting and before reporting. Optional since 2026-11-26: Gemini as implementer through the `gemini-oxi` MCP server (auto-policy: `cargo` allowed, writes only in the worktree, no network, no subagents); flow = Gemini writes, Sonnet with the advisor reviews, BAD ACTOR attacks, the lead merges (decision D-25 in [oxiscript.md](roadmap/oxiscript.md)).
- **BAD ACTOR gate (every stage):** a separate adversarial **Claude** agent: Opus for large stages and releases, Sonnet with Opus consultation for small changes (owner rule, roadmap D-33; Gemini never as the only security reviewer, D-31) writes failing tests and a report, never fixes code; then a fix agent fixes everything; findings stay as regression tests. Notes of each stage go to [stages/](stages/README.md).
- **Gates before every merge and push:** `scripts/oxiscript-gate.sh merge` for OxiScript work (each crate, clippy `-D warnings`, a CI-equivalent unified run, release with `OXI_FULL_FUZZ=1`, the long fuzz campaigns); nightly `.github/workflows/oxiscript-fuzz.yml` with date seeds.
- **Shared files** (root `Cargo.toml`, CI workflows, the roadmaps) are edited only by the lead. Agents never set `OXI_BLESS*` and run no git commands unless told to; the lead blesses goldens after review.
- **Usage limits** stop agents mid-work: every agent keeps a HANDOFF-style note current in its stage directory; on a stop, snapshot-commit its worktree and resume it with SendMessage.
- **Dev blog:** posts only by the owner or the lead, each signed by its real author.

## 3. Machine limits
Old 4-core box, about 12 GB free RAM: at most 3 concurrent agents, `CARGO_BUILD_JOBS=2`, per-package cargo in worktrees (a workspace-wide build in a worktree fails on the missing web bundle). `/` is nearly full: put scratch and `TMPDIR` on `/vectorlegis_ssd_pool/oxiterm/scratch`, never in `/tmp`.

## 4. Where things are
- Status, stages, owner questions, risks: [roadmap/README.md](roadmap/README.md) (index), [roadmap/oxiui.md](roadmap/oxiui.md), [roadmap/oxiscript.md](roadmap/oxiscript.md). Doc inconsistencies and what was fixed: [roadmap/consistency.md](roadmap/consistency.md).
- History of the old plan (Phase 0-7 narratives, measurements): [history/oxi-ui-plan-history.md](history/oxi-ui-plan-history.md); the original concept documents: [history/](history/).
- Protocol and architecture: [ARCHITECTURE.md](ARCHITECTURE.md). Agent guidance for OxiScript: [../AGENTS.md](../AGENTS.md).

## 5. Operational facts
- **Run locally:** `.claude/launch.json` → `oxiterm-site` (examples on web port 8087) and `oxiterm-sans` (8091). Build the two web bundles first (commands in the README; CI does the same).
- **Local mode is off by default:** `--local-mode` / `OXITERM_LOCAL_MODE=1`. Other env: `OXITERM_ALLOWED_HOSTS`, `OXITERM_ALLOW_HARDLINKS=1`, `OXITERM_LOCAL_PROXY_TIMEOUT_MS`. Grid clamped to 500×200.
- **Deploy:** the live server is `oxiterm-cli serve …`; no public server today (a VPS with port-22 SSH comes later). The websites are static (Cloudflare Workers with static assets, deployed by the lead with `npx wrangler@4.86.0 deploy --config sites/deploy/<site>.wrangler.jsonc` after `sites/build.sh`, see [../sites/DEPLOY.md](../sites/DEPLOY.md); domain `slavekm.pl` on Cloudflare).
- **Never let an agent touch `/dev/fb0`, `/dev/dri/*` or `/dev/input/*`**: they are the owner's real display.
- **Hardware:** ESP32-S3 4.3" 800×480 touch (N16R8) bought after 2026-12-10; Xtensa needs `espup` (not installed). Android SDK 34 + Java 17 at `/vectorlegis_ssd_pool/android_sdk`; no device.
- **Click-miss debugging:** three historical causes: ambiguous-width glyphs in labels, SSH emitter cursor desync, web canvas pixel↔cell mapping.

## 6. First steps for the next lead
1. Read this file, then [roadmap/README.md](roadmap/README.md) and the roadmap of the product you work on, then [../AGENTS.md](../AGENTS.md), then [../creator_persona.md](../creator_persona.md) (how to explain things to the owner and teach during the work) and [owner-questions.md](owner-questions.md) (open owner questions with suggestions, in Polish).
2. `gh run list -L 5`: is `main` green? If not, that is the first job.
3. `git worktree list`: unmerged agent worktrees under `.claude/worktrees/` may hold work (the roadmaps say which).
