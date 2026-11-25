# 8.G fixes handoff
Status: started. F1..F6 + interp fuel pending.
F1 done (wrap.rs finish_results, func.rs push_canon_float_body)
F2 done (ir verify.rs find_field/lookup_fields borrowed; ir/tests/wide_struct.rs 100ms@70k)
F4 done (modlayout checked ptr_count/ids/addr, Rec.ix; test asserts 65535 ok / 65536 E0603)
F3 done (LIMIT_LOCALS 12000; measured trap >=17000; test asserting)
F6 test asserts E0603 or ok; interp fuel per block done
F5 in progress: wrap.rs check_export_boundary + CompileOptions.allow_raw_exports (corpus/golden set it); fixing catalogue cat05 etc.
F5 done; golden re-blessed (float-return wrappers). Remaining: full verify (debug, ir, abi, clippy, wasm32)
Verify (Opus takeover 2026-11-25): debug+release oxiscript-wasm green; ir/abi green; clippy green after if_not_else fix in ir/tests/wide_struct.rs; wasm32 build ok. CompileOptions used only inside oxiscript-wasm. DONE.
