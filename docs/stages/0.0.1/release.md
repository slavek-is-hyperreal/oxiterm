# OxiScript 0.0.1 release preparation

Status: prepared, not tagged. The lead reviews, merges, then tags and pushes.

## What changed

* Version 0.0.1 in the ten `oxiscript/*/Cargo.toml`, `editors/vscode-oxiscript/package.json` and `package-lock.json` (top-level and `packages[""]` fields only), the extension README (`.vsix` name), `oxiscript-cli/SCHEMA.md` and the two JSON snapshots. `Cargo.lock` changes accordingly (workspace crate versions). OxiTerm crates are untouched. The `0.1.0` in templates, tests' `oxi.toml` strings and SCHEMA's project example is the user project's own version, not OxiScript's, and stays.
* `oxi --version` (clap `long_version`): `oxi 0.0.1` and `OxiScript 0.0.x: nothing is guaranteed before 1.0.0 (docs/oxiscript/VERSIONING.md)`. The JSON `version` field already came from `CARGO_PKG_VERSION`. The CLI test now asserts the exact two lines.
* Playground: a footer line "OxiScript 0.0.1 · no guarantees before 1.0" (EN, PL, LA strings `oxiscript.playground.versionNote`); `sites/build.mjs` reads the version from `oxiscript-cli/Cargo.toml`, so there is no second literal.
* `scripts/oxiscript-version.sh` (next version, `--check`), wired into `scripts/oxiscript-gate.sh` (all levels, before the crate tests). Rule, carry design and tag format are documented in `docs/oxiscript/VERSIONING.md`.
* CHANGELOG sections for OxiScript 0.0.1 and the OxiTerm/OxiUI work since 2026-11-23.

## Tag design

`oxiscript-v0.0.1`, annotated, message carries `carry=0`. Later: the script sums `git diff --numstat <tag> HEAD -- ':(glob)oxiscript/oxiscript-*/src/**'` (added + deleted, binaries skipped) plus the tag's carry, bumps `c` by `total / 5555`, and prints `total % 5555` for the next tag message. The no-tag branch and `--check` were exercised; the carry arithmetic against a real tag was not (no tags may be created here).

## CI release trigger

`.github/workflows/ci.yml` triggers on `push: branches: [main]`, pull requests, a weekly schedule and manual dispatch. There is no `tags:` filter, so pushing any tag, `oxiscript-v0.0.1` included, starts no workflow today; the `release` job (`if: startsWith(github.ref, 'refs/tags/')`) is therefore currently unreachable by tag pushes. It would only matter if someone adds `tags:` to `on.push`. Recommended for the lead when OxiTerm releases are wired: add `tags: ['v*']` to `on.push` and change the job condition to `startsWith(github.ref, 'refs/tags/v')` so `oxiscript-v*` can never publish an `oxiterm-cli` release. `ci.yml` was not edited. The other workflow, `oxiscript-fuzz.yml`, runs only on schedule and manual dispatch, so a tag push triggers nothing there either.

Note for each release: the two JSON snapshots in `oxiscript-cli/tests/snapshots/` embed the version and need updating.
