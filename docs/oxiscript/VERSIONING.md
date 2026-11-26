# OxiScript versioning

**Current status: 0.0.x — nothing is guaranteed before 1.0.0.**

Syntax, semantics, the ABI, OxiIR, the `oxi` CLI and its JSON schema may change in any release. Programs written today may need changes tomorrow; the compiler's diagnostics and fixes will say how.

## The rule (owner decision, 2026-11-25)

The version is `x.y.c`.

- The first public version is **0.0.1**.
- **`c` grows by 1 for every 5555 changed lines of compiler code.**
  - "Changed lines" = lines added + lines deleted, as counted by `git diff --shortstat` between the previous release tag and the new one.
  - The `src/` directories of the `oxiscript-*` crates count, and the standard library sources `oxiscript/std/**/*.oxs` (owner 2026-11-26: the standard library is part of what the language ships). Tests, documentation, examples and websites do not.
  - The remainder below 5555 carries over to the next release, so no change is ever lost from the count.
- **Owner override (2026-11-26):** the owner may release the next `c` before 5555 lines have accumulated, deliberately (first used for 0.0.2 with the standard library v0). Tool: `scripts/oxiscript-version.sh --owner-release "<reason>"`; the tag message carries `carry=0` and `owner-release=<reason>`, so the counted lines are spent on that release.
- `x` and `y` are raised by hand, by the owner, for milestones. `1.0.0` is the first version with stability guarantees.
- All `oxiscript-*` crates, the `oxi` CLI, the VS Code extension and the web playground always carry the same version.

## Releasing (tools and tag)

- **Tag name: `oxiscript-vX.Y.C`** (first: `oxiscript-v0.0.1`). It is OxiScript-specific so it never collides with OxiTerm's own `v*` release tags. OxiTerm and OxiUI are versioned separately.
- **The tag must be annotated**, and its message carries the leftover count on a line of its own: `carry=N`. This makes the next version computable from git alone.
- `scripts/oxiscript-version.sh` prints the next version. It finds the last `oxiscript-v*` tag, sums added + deleted lines of `oxiscript/oxiscript-*/src/**` since it (`git diff --numstat`, the same sum as `--shortstat`), adds the tag's `carry`, and raises `c` by `total / 5555`. The new carry (`total % 5555`) is printed for the next tag message. If less than one step has accumulated it prints "no release due" and exits 3. With no tag yet it prints `0.0.1` (earlier history is not counted).
- `scripts/oxiscript-version.sh --check` fails if the crates' `Cargo.toml`, the extension's `package.json` and `package-lock.json` disagree. It runs in `scripts/oxiscript-gate.sh` (all levels). The playground page reads the version from `oxiscript-cli/Cargo.toml` when the site is built, so it has no separate copy.
- Raising `x` or `y` by hand: edit all manifests to the new version (`c` = 0) and tag with that name; the carry still goes in the tag message.
- `oxi --version` prints `oxi X.Y.C` and a line saying nothing is guaranteed before 1.0.0.

## Why 5555

Code in this project is written mostly by AI agents, quickly. A per-commit or per-1000-line counter would grow by dozens a day and mean nothing. 5555 lines is roughly one large compiler stage (for example the lowering stage 8.L), so the number tells you how much compiler has been built, not how many commits happened. And the owner likes the number.
