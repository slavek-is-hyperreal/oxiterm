# OxiScript versioning

**Current status: 0.0.x — nothing is guaranteed before 1.0.0.**

Syntax, semantics, the ABI, OxiIR, the `oxi` CLI and its JSON schema may change in any release. Programs written today may need changes tomorrow; the compiler's diagnostics and fixes will say how.

## The rule (owner decision, 2026-11-25)

The version is `x.y.c`.

- The first public version is **0.0.1**.
- **`c` grows by 1 for every 5555 changed lines of compiler code.**
  - "Changed lines" = lines added + lines deleted, as counted by `git diff --shortstat` between the previous release tag and the new one.
  - Only the `src/` directories of the `oxiscript-*` crates count. Tests, documentation, examples and websites do not.
  - The remainder below 5555 carries over to the next release, so no change is ever lost from the count.
- `x` and `y` are raised by hand, by the owner, for milestones. `1.0.0` is the first version with stability guarantees.
- All `oxiscript-*` crates, the `oxi` CLI, the VS Code extension and the web playground always carry the same version.

## Why 5555

Code in this project is written mostly by AI agents, quickly. A per-commit or per-1000-line counter would grow by dozens a day and mean nothing. 5555 lines is roughly one large compiler stage (for example the lowering stage 8.L), so the number tells you how much compiler has been built, not how many commits happened. And the owner likes the number.
