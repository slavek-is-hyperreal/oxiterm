# HANDOFF host-export (owner decision 2026-11-26)
export = module visibility only; `@host export function` = Wasm export (entry module). main stays.
Note: worktree was at 47fbb07, fast-forwarded to main 750f7da (git merge --ff-only) because 8.L was missing.
Design: checker misuse errors in declare_item (E0121), sig checks in check_export_sig only for @host; lower is_export requires @host attr (+ honour @export_name).
## Milestones
- [x] spec docs (grammar, abi, contracts)
- [x] checker (decl.rs check_host_placement, is_host_fn, check_export_sig host-only; export_abi.rs rewritten; corpus 18_attributes @host, 09 restored)
- [x] lower (driver roots+is_export require @host; user @export_name honoured; tests/host_export.rs new; @host added to lower tests via script)
- [x] tests/corpus update (cli.rs extra 12_generic now passes: expectation dropped, LOG.md noted)
- [x] AGENTS.md template, sync_docs run, examples.rs E0121, SKILL.md row
- [x] verify: per-crate + clippy + unified all green (829 passed, 0 failed, 10 ignored)
