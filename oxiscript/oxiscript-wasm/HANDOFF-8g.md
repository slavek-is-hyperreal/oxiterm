# HANDOFF 8.G (oxiscript-wasm codegen)

Status: STARTED (design done, writing code). Update after every milestone.

## Decisions
- deps: wasm-encoder 0.263, wasmparser 0.263 (normal); wasmi 2.0.3, wasmprinter, wat, oxiscript-runtime (dev).
- `compile(&Module,&CompileOptions) -> Result<Vec<u8>,Diagnostics>`; `RuntimeMode::{Stub, Import}`. Stub = embedded test runtime (bump allocator + counts + UTF-8 validation) so output loads in oxiscript-runtime.
- CFG: Beyond-Relooper stackifier (RPO + dominators), irreducible -> diagnostic. Arms flattened (`if {then} end; else-code`) to keep nesting low.
- one wasm local per SSA value; block params = locals; branch args via wasm stack (parallel move).
- traps inline: `i32.const kind; global.set oxi_trap_kind; unreachable` (no call to oxi_trap).
- out-buffer retry loop is IR-level (8.L); codegen only flattens str/[u8] -> (ptr,len).
- needs: `CompileOptions.needs` (ordered), default derived from imports; `main` params must equal needs count.
- new registry codes appended to abi codes.txt (E0600.. codegen) -- see report.

## Lead note from 8.M (applied to design)
- compiler materialises globals `oxi_trap_kind`, `oxi_scratch_cap` (4096); compiler-raised traps are inline `global.set $oxi_trap_kind; unreachable`.
- generated `oxi_init` calls runtime `oxi_rt_init(typedesc_base, typedesc_count)` FIRST. Import mode: imported from rt module; Stub mode: internal no-op (not exported: oxiscript-runtime rejects reserved names outside the ABI table).
- extension exports (oxi_gc_*, oxi_rt_*) are runtime-side; codegen only needs oxi_rt_init. oxi_alloc_bytes(0) may return 0: codegen never dereferences array data before bounds checks.
- Possible conflict: oxi_scratch_ptr is a runtime-exported static; Import mode does not define it.

## Done
- src/: body, cfg, cx, func, helpers, modlayout, stubrt, wrap, lib -- COMPILES; corpus (all 7 oxir programs) compiles+validates (tests/corpus.rs).
- registry codes E0600-E0604, I0002 appended to oxiscript-abi/registry/codes.txt (abi tests green).
- Next: tests/diff.rs (reference interpreter + random IR generator vs oxiscript-runtime), tests/programs_abi.rs (40 programs, runtime loading with host/callbacks), golden dumps, measure example, clippy, wasm32 build, cleanup warnings.

## Lead note 2 (pointer width)
- src/target.rs holds PTR_VT/PTR_BYTES; vt() uses PTR_VT. Many `Mem::I32`/`i32c` address sites (elem_addr, ArrayPush, wrappers, stubrt) are still wasm32-specific; a memory64 port must parametrise them. Reported as partial.
- tests/common/interp.rs (reference interpreter) written; next tests/diff.rs generator.

## Progress 2
- tests/diff.rs: differential random IR (reference interp tests/common/interp.rs, generator tests/common/gen.rs): release 3000 modules / 72000 cases agree incl. trap kinds (debug 150 modules; OXI_FULL_FUZZ=1 full).
- tests/programs.rs p01-p15 scalar programs pass. Bugs found+fixed: u32 mul trap extra local.get; sat sub u64 select order.
- Next: managed programs p16+ (struct/cow/union/array/string/closure/opt/globals/wrappers/host/callbacks/irreducible/chain/tick/safety/import-mode/determinism), goldens, measure example, clippy, wasm32, warning cleanup, final advisor.

## Progress 3 (near final)
- tests: corpus (7 programs compile+validate+load), programs.rs p01-p15, programs2.rs p16-p42 (managed, host/callback/outbuffer, irreducible, config errors, import mode, determinism, features), golden (4), diff (random). clippy -D warnings clean, wasm32 build OK, example `measure` done.
- Remaining: final advisor review, report. Not done: tag-proof elision of variant bad_cast checks, local reuse, memory64 parametrisation (see lead note 2).

## Remaining / current step
- skeleton -> stub runtime -> lowering -> differential tests -> corpus -> measure.

## Verify
cargo test -p oxiscript-wasm ; cargo test -p oxiscript-wasm --release ; cargo clippy -p oxiscript-wasm --all-targets -- -D warnings ; cargo build -p oxiscript-wasm --target wasm32-unknown-unknown
(use CARGO_BUILD_JOBS=2)
