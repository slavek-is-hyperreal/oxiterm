# HANDOFF 8.D BAD ACTOR fixes (update after every fix)

> **Status (2026-11-26):** complete and merged (`148b8e8`; DevKit v0 of stage 8.D0). The unchecked boxes below were never ticked in these working notes: the items were finished (see `oxiscript/oxiscript-cli/tests/agent_test/LOG.md`, `oxiscript/oxiscript-cli/SCHEMA.md`, `editors/vscode-oxiscript`), and the final report was delivered.

Plan: I/O layer (F1 F5 F8 F9 F15) -> F7 -> notes (F13 F6 F15) -> F16 -> F2 -> F6 -> F17 -> F10 -> F4 -> extension (F3 F11 F12).

## Done
(none yet)
- [x] F1 (project.rs FsLoader::resolve + canonical-in-root check; reasons shown in E0302 message, no new code), F5/F8/F9/F15 (project.rs read_regular/write_atomic/walk: no symlink follow, regular files only, caps 8MiB/1MiB/64KiB, atomic write, per-file diagnostics E0014/E0012), F7 (HashMap index, one CheckOptions per root borrowed), F13 (note), F16 (find_root stops at .git/$HOME), F17 (clap conflicts_with), F6 (in-memory rounds, windowed progress when capped, notes + JSON `fix`), F2 (src/needs.rs, E0400/E0402), F10 (fmtcmd::format_preserving), F4 (docs route; test corrected).
- [ ] extension F3 F11 F12, SCHEMA.md notes/fix, verification
- [x] extension F3 (scope machine + untrustedWorkspaces limited + resolveExe from absolute PATH entries), F11 (parseReport validation + try/catch in check), F12 (queue, 3 concurrent). SCHEMA.md updated (notes, fix).
- Test corrections: tests/bad_actor_8d.rs fmt_inserts_semicolon_as_docs_claim (pinned the false docs claim; now asserts fmt refuses + points at check --fix, check --fix inserts `;`, AGENTS.md claim gone); tests/cli.rs project_idl_is_resolved... (manifest of the last case now lists needs host:log@1, required by F2).
- No registry codes added. Verified: cargo test -p oxiscript-cli, clippy -D warnings, npm test 25/25, npm run package, sync_docs idempotent.
- [x] advisor follow-ups (F11 stale diag clear, ReadError kinds, needs dup E0400, exhausted flag, CRLF template literal verified). Next: semicolon assistant (owner item).

## Semicolon assistant (owner item, done)
- CLI: `oxi check --fix-only semicolons [--stdin]` (check.rs FixMode::Semicolons: only E0004 single `;` insertions; stdin prints fixed source). hints.rs `continuation_hazard`: E0004 on a line starting with `( [ ` + - * . ...` (the `a\n[1]` = `a[1]` case) becomes maybe-incorrect + advice, for `--fix` and the extension alike.
- Diagnostics were already one-per-statement at the end of the statement (no cascade seen in ~20 TS-style shapes), so (1) needed no change besides the hazard rule.
- Extension: setting `oxiscript.semicolons.fixOnSave` (default true), onWillSave -> precise `;` insert edits (convert.ts semicolonInsertions verifies the output is the input plus `;` only), quick fix "Add missing semicolons in this file".
- Tests: oxiscript-cli/tests/semicolons.rs (4), extension.mock.test.js (2), convert.test.js (1). Docs: AGENTS.md template, extension README, SCHEMA.md.
- For 8.P: the parser/checker could report E0004 with the continuation-hazard knowledge itself (or a dedicated code for "ASI hazard: line starts with ( [ `"), so the CLI-side hints rule can go; and the error range for E0004 could carry the previous-line position for the `a\n[1]` case.
