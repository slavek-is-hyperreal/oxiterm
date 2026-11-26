# Compilation targets (OxiScript 8.2, owner decision 2026-11-26)

`oxi build|run|check --target device|web|server`. Spec: `docs/oxiscript/abi.md` 3.6.

## Design
* `oxiscript-abi/src/target.rs`: `Target {Device (default), Web, Server}` and `Feature` (simd128, exceptions, tail-call, extended-const = web; memory64 = server; multi-memory removed, see fixes.md F2), `Feature::min_target`. Own enum, no wasmparser dependency (abi stays a leaf).
* Feature sets: Device = MVP + multi-value + bulk-memory + sign-extension + mutable globals (today's set). Web = Device + SIMD + EH (`exnref`, so REFERENCE_TYPES is enabled in the validator) + tail-call + extended-const. Server = Web + memory64 (declarable only; ABI 1.0 and the engine refuse it). No GC, relaxed-simd, threads, trunc_sat in any target.
* Manifest: optional `target` and `features` lines. A device module with no features prints neither (byte-identical to before). Parse: unknown target/feature, duplicates and a feature the target does not allow are errors (E0402 at load).
* Codegen (`oxiscript-wasm`, the only change there): `CompileOptions::target`, `features_for(target) -> WasmFeatures`, `validate_for`, `Cx::assemble` writes the target into the manifest. `allowed_features()`/`validate()` stay (Device). No code path emits a new feature, so all targets give the same code; only the manifest differs.
* `oxiscript-lower::Options::target` forwards it.
* Runtime: `HostOptions::max_target` (default Device); wasmi Config tail-call/extended-const follow it, everything else is explicitly off; `load::check_target` runs before `Module::new` and refuses with new code `OXS-E0411` (inserted after E0410 in sorted order) a manifest above the host or declaring a feature this wasmi build lacks (`ENGINE_FEATURES` = tail-call, extended-const; no simd/EH/memory64 in the wasmi build).
* CLI: `--target` on check/build/run; `[package] target` in oxi.toml; `check --json` has additive `"target"`; `oxi run --target T` uses a host with `max_target = T`.
* Playground: compiles for `web`, runs on a Web-configured runtime (`PLAYGROUND_TARGET`); no selector.
* Test matrix: `OXI_TARGET` is read by `oxiscript-wasm/tests/common` and `oxiscript-lower/tests/common` (compile target + host max_target); `scripts/oxiscript-gate.sh` step `targets-web` runs both crates' suites with `OXI_TARGET=web` (device = default run).

* Manifest honesty: `load::check_target` also validates the bytes (wasmparser 0.228) against Device base + declared features; code that uses more but would run on this host is `OXS-E0411` (a `device` module with `return_call` on a Web host); code the host could not run anyway stays `E0408` from the engine. Test in `targets.rs`.
* `check --json` snapshots got the additive `"target"` field.

## Verification (2026-11-26)
`scripts/oxiscript-gate.sh quick`: all steps ok (incl. new `targets-web`, clippy -D warnings) after fixing the two CLI JSON snapshots; `cargo test -p oxiscript-cli` re-run green; `cargo build -p oxiscript-playground --features run --target wasm32-unknown-unknown` ok. Worktree: `.claude/worktrees/targets`, branch `worktree-targets` (the original worktree vanished; restarted from d592156). Nothing committed.

## Tests
`oxiscript-wasm/tests/targets.rs` (same code on all targets, validator per target incl. never-allowed features, device host refuses web/server modules and accepts web on web host, declared features vs target/engine), manifest unit tests, `target.rs` unit test, `oxiscript-cli/tests/cli.rs::targets_build_run_check`.

## Not done / notes
* load.rs still rejects memory64/shared memories (ABI 1.0 rule); a Server module using memory64 additionally needs an ABI bump (decision 13) and an engine with memory64.
* Merge hint for the runtime-linking work: wasm changes are limited to lib.rs (`target` field, `features_for`, `validate_for`) and one line in cx.rs.
