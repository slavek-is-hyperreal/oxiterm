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

## Done
(nothing yet)

## Remaining / current step
- skeleton -> stub runtime -> lowering -> differential tests -> corpus -> measure.

## Verify
cargo test -p oxiscript-wasm ; cargo test -p oxiscript-wasm --release ; cargo clippy -p oxiscript-wasm --all-targets -- -D warnings ; cargo build -p oxiscript-wasm --target wasm32-unknown-unknown
(use CARGO_BUILD_JOBS=2)
