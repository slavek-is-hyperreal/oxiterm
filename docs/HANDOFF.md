# Handoff: OxiTerm / OxiUI / OxiScript — state as of 2026-11-26

This file lets another Claude account (or a person) take over without the previous session's memory. Read it top to bottom, then [oxi-ui.md](oxi-ui.md) (the plan; source of truth for phases, decisions and the wave log).

## 1. The owner and how to work with them
- **Language:** the owner writes in Polish; answer in Polish, write code, docs and commits in English.
- **Repository:** github.com/slavek-is-hyperreal/oxiterm (public), branch `main`. Every change is committed and pushed. Commit messages end with a `Co-Authored-By:` line for the model that made them.
- **Licence and business:** MIT OR Apache-2.0. Revenue from support, compliance certification (IEC 62443, EU CRA) and voluntary donations (buycoffee.to/smajcher). **Dependencies: permissive licences only** (MIT/Apache/BSD/Zlib/ISC; MPL only if recorded; fonts OFL/Apache; never GPL/AGPL/LGPL).
- **Decisiveness:** the owner prefers decisions now over long option lists, and delegates taste decisions (fonts, colours, visual calls) to the lead after screenshots. Stop only for decisions with real consequences (licence, money, scope, language semantics).
- **Inform, don't forbid:** useful trade-offs that weaken a guarantee (e.g. linking an `unsafe` Rust crate) are documented in the spec, announced at build time and forbiddable by a profile, not banned.
- **Don't claim a gap without checking the repo** (grep code and docs first).
- **Report outcomes from independent evidence:** after every push check GitHub CI (`gh run list`) and report its result with the link; a local "all green" from an agent is a claim, not proof (on 2026-11-25 CI was red for hours unnoticed, and an agent reported green while two test files failed).

## 2. How work is done
- **Lead:** the main conversation (Opus) designs, writes briefs, reviews, merges, keeps the plan.
- **Implementers:** Sonnet subagents in their own git worktrees, using their `advisor` tool (Opus) before starting and before reporting. Optional since 2026-11-26: **Gemini as implementer** through `/vectorlegis_ssd_pool/gemini-oxi` (MCP server `gemini-oxi`, auto-policy in `zadania/oxi-kod-polityka.json`: `cargo` allowed, writes only in the worktree, no network, no subagents); flow = Gemini writes → Sonnet+advisor reviews → BAD ACTOR → lead merges. A pilot on one small stage comes first.
- **BAD ACTOR gate (every stage):** a separate adversarial agent writes failing tests and a report, never fixes code; then a fix agent fixes everything; findings stay as regression tests. Notes of each stage go to [stages/](stages/).
- **Gates before every merge and push:** `scripts/oxiscript-gate.sh merge` (each crate, clippy `-D warnings`, a CI-equivalent unified run, release with `OXI_FULL_FUZZ=1`, the long fuzz campaigns). Nightly on GitHub: `.github/workflows/oxiscript-fuzz.yml` with date seeds.
- **Machine:** old 4-core box, ~12 GB free RAM → at most 3 concurrent agents, `CARGO_BUILD_JOBS=2`, per-package cargo in worktrees (a workspace-wide build in a worktree fails on the missing web bundle). Shared files (`docs/oxi-ui.md`, root `Cargo.toml`, CI workflows) are edited only by the lead.
- **Usage limits** stop agents mid-work: every agent keeps a HANDOFF file current; on a stop, snapshot-commit its worktree and resume it with SendMessage.
- **Golden files:** agents never set `OXI_BLESS*`; the lead blesses after review.
- **Dev blog:** posts only by the owner or the lead, each signed by its real author.

## 3. Status
- **Done and on `main`:** OxiUI Phases 0–6, 7.1 (WASM local mode), 7.2 (OxiDE software sink). OxiScript 8.C contracts, 8.P parser/formatter, 8.R runtime, 8.M memory, 8.T type checker, 8.G Wasm codegen, **8.L lowering: the compiler works end to end**, 8.D DevKit v0 (`oxi` CLI, VS Code extension, AGENTS.md, skill, llms.txt, semicolon assistant), 8.W websites step 1 (three static sites + browser playground; not yet deployed: the owner deploys per [../sites/DEPLOY.md](../sites/DEPLOY.md)). All hardened by BAD ACTOR.
- **Owner decisions of 2026-11-25/26 now in the code or the plan:** `export` = module visibility as in TS, host entry points are explicit `@host` (grammar 2.1); C-style `for (let …)` gives each iteration a fresh binding (as JS); Rust crates linked into the program as the main road to libraries (grammar 2.7, stage 8.N); versioning 0.0.1, `c += 1` per 5555 changed `src/` lines ([oxiscript/VERSIONING.md](oxiscript/VERSIONING.md)); sites in PL + EN, Latin later after the owner's review; mascot Oxi the axolotl, Physarum for the bug-shape engine.
- **Open items (owner):** OxiBench methodology approval and API keys ([research/oxibench-methodology.md](research/oxibench-methodology.md) §10); the home page "about" text; review of the Polish site copy (banner "draft" until then); Latin review.
- **Known gaps:** real linking with the 8.M runtime (`RuntimeMode::Import`) is not done yet (tests use the Stub runtime; `f64` to string needs it); Wasm local reuse (~6000 statements per function); closure cycles leak until the cycle collector is linked; two 8.P oracles (comment re-attachment, an E0006 no-op fix) passed at seed 1 on 2026-11-26 and need a multi-seed check before they are called fixed.

## 4. Next steps (in order)
1. Bump all OxiScript crates, the extension and the playground to **0.0.1**, add the version script, tag `oxiscript-v0.0.1` (prepared, see [stages/0.0.1/release.md](stages/0.0.1/release.md)), write CHANGELOG.
2. Wire `oxi run` / `oxi build` and the playground's Run button to `oxiscript_lower::compile_source`.
3. Gemini pilot on a small stage, then decide whether implementation moves to Gemini.
4. Link with the real 8.M runtime (decide the two open ABI questions from 8.G: heap base / static end, runtime trap cell), then **8.S1** `<script lang="oxs">` in an OxiUI page (end of the critical path).
5. In parallel: 8.S2 LSP, 8.S4 laws + report + bug shapes + sign vocabulary + interface laws, 8.N Rust crates, identity sketches, course level 0, dev blog.
6. After 2026-12-10: the ESP32-S3 4.3" touch board (8.S3).

## 5. Operational facts
- **Run locally:** `.claude/launch.json` → `oxiterm-site` (examples on web port 8087) and `oxiterm-sans` (8091). Build the two web bundles first (commands in the README; CI does the same).
- **Local mode is off by default:** `--local-mode` / `OXITERM_LOCAL_MODE=1`. Other env: `OXITERM_ALLOWED_HOSTS`, `OXITERM_ALLOW_HARDLINKS=1`, `OXITERM_LOCAL_PROXY_TIMEOUT_MS`. Grid clamped to 500×200.
- **Deploy:** the live server is `oxiterm-cli serve …`; no public server today (a VPS with port-22 SSH comes later). The websites are static (Cloudflare Pages, domain `slavekm.pl` on Cloudflare).
- **Never let an agent touch `/dev/fb0`, `/dev/dri/*` or `/dev/input/*`**: they are the owner's real display.
- **Hardware:** ESP32-S3 4.3" 800×480 touch (N16R8) bought after 2026-12-10; Xtensa needs `espup` (not installed). Android SDK 34 + Java 17 at `/vectorlegis_ssd_pool/android_sdk`; no device.
- **Click-miss debugging:** three historical causes: ambiguous-width glyphs in labels, SSH emitter cursor desync, web canvas pixel↔cell mapping.

## 6. First steps for the next lead
1. Read this file, then [oxi-ui.md](oxi-ui.md) (status, Phase 8 decisions, stages, wave log), then [../AGENTS.md](../AGENTS.md).
2. `gh run list -L 5`: is `main` green? If not, that is the first job.
3. `git worktree list`: unmerged agent worktrees under `.claude/worktrees/` may hold work.
4. Continue with section 4.
