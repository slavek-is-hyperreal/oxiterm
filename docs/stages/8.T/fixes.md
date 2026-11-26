# HANDOFF 8.T BAD ACTOR fixes
ALL DONE. bad_actor_8t (42 tests), catalogue, whole suite green debug+release; clippy clean; wasm builds; speed ~107k lines/s.
Test corrections: limits.rs deep_generics_are_bounded (polymorphic recursion now E0113), nested reversed const chain -> lenient (forward const reads are E0123).
Not done: static constructor (parser drops `static`), excluded-case warning, non-constant case labels (grammar allows), this-closure semantics (documented).
Docs: docs/oxiscript/contracts.md 8.T "Soundness rules added by the BAD ACTOR pass".
