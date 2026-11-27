# OxiScript ABI

Status: contract of slice 8.C (2026-11-25), ABI version **1.0**. Rust constants and parsers: `oxiscript/oxiscript-abi` (`layout.rs`, `exports.rs`, `idl.rs`, `manifest.rs`, `trap.rs`, `codes.rs`, `registry/codes.txt`, `idl/*.idl`). This document is the human spec; where the two disagree the Rust constants win and this file is a bug.

Scope: how OxiScript values sit in Wasm linear memory, what a conforming `.wasm` module exports and imports, how a host declares capabilities (IDL), and how diagnostics are identified. By default (target `device`, section 3.6) everything is **Core WebAssembly** (MVP + multi-value + bulk-memory + sign-extension + exported mutable globals, needed for `oxi_trap_kind`; non-trapping float-to-int `trunc_sat` is **not** allowed, so codegen must emit trapping or explicit-saturating sequences): no GC, threads, exception-handling or reference-types proposals. Additional opt-in target features (`web`, `server`) are specified in section 3.6. The host enforces exactly this set (plus whatever the *targets* `web` and `server` add, section 3.6, when it is configured for them) and also refuses a start section, imported memories/tables/globals/tags (`OXS-E0405`), a second memory, memory64, table64 and shared memories (8.R, `OXS-E0408`).

One module = compiler output **statically linked** with the runtime-support code (slice 8.M). "Runtime exports" below are functions of that one module.

---

## 1. Memory

* wasm32, one linear memory exported as `memory`, little endian. Initial size = static data + stack + a minimum heap; no maximum declared (the host/engine limits growth; growth failure is `out_of_memory`).
* Address `0` is null. Bytes `[0, 1024)` are never allocated or mapped. Static data (string constants, type descriptors, immortal objects) follows, then the (reserved) shadow stack region, then the heap. All allocations are 8-aligned.
* The runtime also owns a **scratch region** (>= 4096 bytes, 8-aligned, never moves) exported as `oxi_scratch_ptr`/`oxi_scratch_cap`: the one region the host may write without having been handed an address, and only while the guest is not running (before it calls a guest entry point, 5.4). Apart from the scratch region the host writes into guest memory **only** into out-buffers the guest explicitly passed it for that call (5.2); any other write is a host bug and the sandbox must not allow it.

## 2. Object layout

Every managed object is `[header: 8 bytes][payload]`. **An object pointer points at the payload**; the header is at `ptr - 8`. Total object size is a multiple of 8.

```
ptr-8  u32 rc      reference count (non-atomic; one instance = one thread)
ptr-4  u32 meta    bits 0..23 type id | bits 24..26 colour | bit 27 buffered | bits 28..31 reserved (0)
ptr+0  payload
```

* **rc**: values `>= 0x8000_0000` are *sticky*: `retain`/`release` leave them alone and the object is never freed. Static objects (string constants) are created sticky; a count that would overflow becomes sticky (a deterministic leak, never a wraparound). Null (`0`) is accepted by `retain`/`release` and ignored.
* **colour** (cycle collector, Bacon-Rajan): 0 black, 1 gray, 2 white, 3 purple (candidate root), 4 green (cannot be part of a cycle: never buffered or traced). `buffered` marks membership in the candidate-roots buffer. New objects get the colour from their type descriptor (`green` if the type is `acyclic`, else `black`).
* **type id**: 24 bits. `0` invalid, `1` string, `2..15` reserved for built-ins, user types from `16`. The id indexes the **type descriptor table**.

### 2.1 Type descriptors

The exported immutable globals `oxi_typedesc_base` (address) and `oxi_typedesc_count` locate a table of 16-byte entries indexed by type id (entry 0 is zero):

```
word0  size_or_elem   record / closure / union variant: payload size (rounded to 8)
                      array: element size in bytes; string: 0
word1  kind (bits 0..7) | flags (8..15) | ptr_count (16..31)
word2  ptr_map_addr   address of ptr_count u32 offsets; 0 when none
word3  reserved (0)
```
Kinds: 1 struct, 2 union variant, 3 closure, 4 string, 5 array. Flag bit 0: `acyclic`. The **pointer map** lists the payload offsets of managed-reference slots; for arrays it lists the slots *inside one element* (so arrays of inline records, and later device buffers and SIMD element types, need no new mechanism). The runtime uses the descriptor to release children, trace for cycles, size allocations and `oxi_clone`.

### 2.2 Value representation

| Language type | Representation |
|---|---|
| `bool`, `i8..i32`, `u8..u32` | Wasm `i32`, always canonical (sign/zero-extended); in memory 1/2/4 bytes |
| `i64`, `u64` | `i64`; 8 bytes |
| `f32`, `f64` | `f32`, `f64`; 4/8 bytes; NaN rules in `ir.md` section 6 |
| `enum` (C-like) | its integer representation type |
| `string`, arrays, structs, classes, unions, closures | `i32` object pointer (managed); `T \| null` of these: same, null = 0 |
| `T \| null` for scalar `T` | boxed: a generated one-field struct (null = 0); a later scalar-replacement pass may unbox |
| `Result<T,E>` | union, tag 0 `ok`, tag 1 `err` |

### 2.3 Payloads

Field layout is **declaration order, natural alignment** (`layout::layout_fields(prefix, fields)`): offsets start at `prefix` and each field is aligned to its own size (1/2/4/8; managed refs 4), the payload size is rounded up to 8. This one function is used by the compiler and the runtime tests.

* **Struct / class**: prefix 0, fields in order.
* **String** (immutable): `u32 byte_len`, `u32 reserved (0)`, then the UTF-8 bytes, then one `0` byte not counted in `byte_len` (valid UTF-8 by construction; validated at the host boundary). Data pointer for `(ptr, len)` flattening: `ptr + 8`.
* **Array**: `u32 len`, `u32 cap`, `u32 data` (payload 12 bytes). `data` points to a separate header-less 8-aligned block of `cap * elem_size` bytes from `oxi_alloc_bytes`, owned by the array. Elements are stored back to back at natural size (scalars, 4-byte references, or reserved inline records). `len <= 2^31 - 1`; indexes are `i32` and bounds-checked.
* **Closure**: `u32 table_index` (index into the module's function table), `u32 reserved`, captures from offset 8 (record layout with prefix 8). The function at `table_index` takes the closure object itself as its first parameter (the environment) followed by the declared parameters. A plain function used as a value is a closure with no captures.
* **Union variant**: `u32 tag` at offset 0 (the variant's declaration index; **the TypeScript-style discriminant field `kind` is not stored**, it is derived from the tag: reading `s.kind` yields the literal belonging to the tag, see `grammar.md` 2.5), fields from offset 4 (record layout, prefix 4; an `f64` field lands at offset 8). The **header type id names the variant** (each variant has its own descriptor), so freeing and tracing need no tag dispatch; the static type of a union value is the union.

## 3. Module conventions

### 3.1 Exports (fixed table: `exports::EXPORTS`)

Compiler-provided:
* `memory`
* `oxi_abi_version` (global i32): `major << 16 | minor`. A host accepts the same major and a module minor `<=` its own.
* `oxi_trap_kind` (mutable global i32): code of the last trap, 0 if none (see 3.4).
* `oxi_typedesc_base`, `oxi_typedesc_count` (globals i32).
* `oxi_init(cap_0, ..., cap_n-1)`: exactly once per instance. Calls `oxi_rt_init(oxi_typedesc_base, oxi_typedesc_count, heap_base)` first (passing the 8-aligned end of static data so the runtime's allocator starts after all static data), then runs module initialisers (module-level `let`), then calls the exported source function `main`, passing the same capability handles as its arguments in the same order (the language defines `main`'s parameters as exactly the capability types, one `needs` line each; see `grammar.md` 2.1). A module without `main` has no `needs` and `oxi_init` takes no parameters. No results.
* `oxi_deinit()`: drops module-level state so leak checks can assert `oxi_live_objects() == 0`.
* `oxi_tick(now_ms: i64) -> i64`: run due timers/animation callbacks; returns the next deadline in milliseconds or `-1` (matches OxiUI's `next_deadline` step API). The host owns the clock (no ambient time).
* `cb:<module>@<version>.<Interface>.<Callback>(closure, flattened params...) -> flattened result`: one trampoline per IDL `callback` the module can receive (5.4).
* User `@host export function`s of the entry module (plain `export` is module visibility only and exports nothing to the host) under their own names (or `@export_name`), with ABI-flattened signatures per 5.1. Names starting with `oxi_`, `__oxi_` or `cb:`, the name `memory` and the linker's layout symbols `__heap_base` / `__data_end` are reserved (the IR verifier rejects them outside the trusted runtime module), and exported names must be unique. **The host-to-guest data rule applies to user exports too:** an exported function receives `str`/`bytes` parameters through the same scratch delivery, so it has at most one `str`/`bytes` parameter of at most `HOST_PAYLOAD_MAX` bytes; the checker (8.T) rejects `@host` functions violating it.

Runtime-provided (names and signatures are `exports::EXPORTS`; semantics in its `doc` fields): `oxi_rt_init(typedesc_base, typedesc_count, heap_base)`, `oxi_alloc(type_id)`, `oxi_alloc_bytes(size)`, `oxi_free_bytes(ptr, size)`, `oxi_retain`, `oxi_release`, `oxi_clone`, `oxi_is_unique`, `oxi_collect_cycles`, `oxi_live_bytes`, `oxi_live_objects`, `oxi_str_new/concat/eq/cmp/slice/from_i64/from_f64`, `oxi_array_new`, `oxi_array_grow`, `oxi_trap`, `oxi_scratch_ptr`, `oxi_scratch_cap`.
* **`oxi_rt_init`**: validates and installs the type descriptor table and sets the allocator's starting heap address (`heap_base`, the 8-aligned end of static data). In linked modules, `oxi_rt_init` refuses any `heap_base < static_end` (including `0`, the runtime's own `__heap_base`, or any address inside program static data), any misaligned address, or any address beyond current linear memory size by trapping with runtime-extension code `64` (`T_BAD_TYPEDESC`). Calling `oxi_rt_init` again with valid arguments is a harmless no-op once the heap has been used; before the first allocation the latest valid call wins, so an early call by a host cannot make the program run with the host's table (the program's own `oxi_init` installs its own). In linked modules the allocator's floor is patched at link time to the end of the program's static data, so even an allocation made before `oxi_init` (a misbehaving host) lands above static data.
* **Memory reclamation & OOM recovery**: `oxi_release` at count 0 frees leaf/small objects directly and drains deeper graphs iteratively with a bounded amount of work per call (deferred destruction queue) so a long list drop cannot stall a frame. Before trapping `out_of_memory`, the runtime drains the deferred destruction queue and runs a *bounded* cycle collection (`Rt::oom_collect`: `oxi_gc_step`-style steps of 2,048 work units, growing 4x up to 8,192 per step, stopping as soon as the requested size (at least 64 KiB) has been returned to the heap, with at most 8,192 work units per allocation attempt, retried with a doubled target while it keeps freeing memory) and then retries the allocation. A work unit is one object or one pointer-slot visit of the collector, so an attempt costs at most about 8,192 x 561 = 4.6M fuel with the release runtime (8,192 x 1,042 = 8.5M with the debug runtime), always under the default `fuel_per_call` of 10M, and typically far less. A garbage subgraph larger than the attempt budget is **not** reclaimed in-call: the allocation traps `out_of_memory` (a clean, recoverable-by-restart trap) instead of running the call out of fuel half-way through a trial (`fixes_link2::bounded_oom_recovery_ends_in_out_of_memory_not_out_of_fuel`: 652,000 live purple nodes at 245 pages, then an allocating call ends in `out_of_memory` after 6.6M fuel, its own allocation work included). The host's incremental housekeeping between calls (section 4) and `oxi_collect_cycles` (unbounded, used by `deinit` and explicit `collect_cycles()`) reclaim such graphs.
* **Small-object pool**: blocks of 8..=256 bytes come from per-size-class slabs (one 4,096-byte, 4,096-aligned `dlmalloc` block each, 24-byte header: free list, `used` count, links of the class's partial-slab list). A slab that becomes completely free is returned to `dlmalloc` (one empty slab per class is kept as a reserve), so memory freed in one size class is reusable by every other size and by large blocks. The debug runtime additionally quarantines freed small blocks (FIFO, 64 entries) so a stale pointer cannot alias a new object of the same size.
* **Fuel layout rule (runtime authors)**: wasmi charges the fuel of every instruction of a function body up front on entry, up to the next `loop`/`if`/`else`, whichever branch then runs. Hot runtime entry points (`oxi_release`, `oxi_retain`, `oxi_alloc`) are therefore thin dispatchers and each path (unique record, shared object, string, array, destruction queue) is its own small non-inlined function; code a path never executes must not sit in the entry function. Measured on `bad_actor_link2`: moving the record/string/array freeing code out of the `rc > 1` path of `oxi_release` halved the fuel of every shared release.
* **Optional collector exports** (`exports::COLLECT_STEP`, `exports::COLLECT_STAT`; not part of the fixed table, absent from the Stub runtime): `oxi_collect_step(units) -> bits` is the runtime's bounded incremental collection step (`oxi_gc_step`: bit 0 = more work, bit 1 = candidates remain that exceeded this step's budget; `units == 0` does nothing) and `oxi_collect_stat(key)` its counters (0 pending destruction queue length, 1 candidates, 5 collection wanted). They have type `(i32) -> i32`; the host loader accepts them and `Runtime::call_export` refuses them like every `oxi_` name.
* **Float-to-text formatting**: `oxi_str_from_f64` formats IEEE-754 `f64` values and the compiler-internal helper `oxi_str_from_f32` formats IEEE-754 `f32` values as shortest round-trip decimal text for each type's own precision (matching Rust's `core::fmt::Debug` with a trailing `.0` stripped for integral values: `1.0` -> `"1"`, `0.0` -> `"0"`, `-0.0` -> `"-0"`, `0.1` -> `"0.1"`, `0.1f32` -> `"0.1"`, `16777216.0f32` -> `"16777216"`, `3.4028235e38f32` -> `"3.4028235e38"`, `1e21` -> `"1e21"`), `"NaN"` for any NaN (payload/sign unobservable), and `"Infinity"` / `"-Infinity"` for infinities.

### 3.1.1 Static linking contract (program + runtime)

When the compiler links the program module with the runtime-support Wasm module (`oxiscript-rc`) into one Core Wasm module:
* The runtime module's own stack, static data, BSS and scratch buffer (`oxi_scratch_ptr`) occupy the low region `[0, runtime_heap_base)`. The compiler places the program's static data (string constants, pointer maps, type descriptor table) starting at the 16-aligned end of the runtime's static region and passes the 8-aligned end (`static_end`) as `heap_base` to `oxi_rt_init`. The linker wraps `oxi_rt_init` with a guard that traps (`64`) on any `heap_base < static_end`.
* The runtime module imports one internal function `("oxi:rt", "oxi_set_trap")(kind: i32)` which the linker resolves to a compiler-generated helper that executes `global.set $oxi_trap_kind`. No `oxi:rt` imports remain in the final linked module, and `oxi_trap_kind` is the single trap channel read by the host for both compiler-raised and runtime-raised traps.
* **Default fuel budget (`Limits::default().fuel_per_call = 10_000_000`)**: with the linked runtime's size-class small-object pool (`8..=256` bytes), 10M fuel per guest entry buys roughly 15,000 small class/struct allocations and releases (~620–640 fuel/iter), ~5,000 integer template interpolations (~2,000 fuel/iter), ~8,400 four-element array pushes (~1,190 fuel/iter), or ~200,000 empty loop iterations (~49 fuel/iter). The budget resets on every guest entry (`oxi_init`, `call_export`, `oxi_tick`, callback).

### 3.2 Imports

All imports are host capability functions (5) and nothing else: the runtime support is internal. The import module name is `host:<name>@<version>` (for example `host:log@1`); the import field is the IDL function name. A module that imports anything not listed in its manifest, or a version the host does not provide, is rejected at link time (`OXS-E0400`). There is no WASI, no `env`, no clock, no randomness: those are capabilities if a host offers them.

### 3.3 The manifest (custom section `oxi.manifest`)

Text, read before instantiation (`manifest::Manifest`): `abi 1.0`, `edition 2026`, `profile default|realtime|safety|kernel` (accepted in the manifest; profile restrictions are not yet enforced by the checker until stage 8.2c), `target device|web|server` (optional, absent = `device`) and `features <name> ...` (optional: the Wasm features beyond the Device set the module needs, each allowed by its target), and one `needs host:state@1 State` line per required capability, which fixes the order of `oxi_init`'s parameters. Unknown directives are an error, so an old host refuses a newer module instead of misrunning it. Further rules: the printer omits `target`/`features` for a device module with no features, so the manifest of such a module is byte-identical to the one before targets existed; `features` is sorted and without duplicates; a feature its target does not allow is a manifest error (`OXS-E0402`); numbers are canonical decimals (no `+`, no leading zeros); `profile` is one of `default|realtime|safety|kernel`; a module needs at most one version of each capability module and one interface name per capability; `needs` lines are limited to `MAX_NEEDS = 32` (the parameter count of `oxi_init`); module names are `host:<identifier>` with a non-empty identifier.

### 3.4 Traps

A trap executes Wasm `unreachable` after `oxi_trap(kind)` stored the code in `oxi_trap_kind` (codes in `trap::TrapKind`: 1 unreachable, 2 overflow, 3 div_by_zero, 4 bounds, 5 bad_cast, 6 null_deref, 7 out_of_memory, 8 assert, 9 bad_utf8, 10 panic). After a trap the instance must not be called again (only discarded): instance swap on reload and a fresh `oxi_init` are the recovery path. Fuel exhaustion and memory limit are engine traps (the host knows them); they leave `oxi_trap_kind` at 0.

### 3.5 Versioning

* **ABI version** (`oxi_abi_version`): minor bumps only add (new exports, new descriptor kinds, new flag bits, new reserved-word uses); anything that changes the meaning of an existing layout or signature is a major bump.
* **Capability versions** are part of the import module name; a host can offer `host:state@1` and `host:state@2` side by side.
* **Text/IR versions** are separate (`oxiir 1`, `oxi-manifest 1`).
* **Reason codes** are never reused (section 6).

### 3.6 Compilation targets and host `max_target`

A **target** is a build setting, not a language construct and not a profile: it selects which Wasm features the code generator *may* emit and the host *must* provide. The language and its semantics are identical on every target and no code path emits a new feature, so all targets produce the same code; only the manifest differs (`target`/`features` lines, 3.3). Spec of the sets: `oxiscript_abi::target`.

| target | features beyond the Device base set |
|---|---|
| `device` (default) | none: MVP + multi-value + bulk-memory + sign-extension + mutable globals |
| `web` | `simd128`, `exceptions` (exnref), `tail-call`, `extended-const` |
| `server` | `web` plus `memory64` |

Targets are ordered `device < web < server`; each set contains the previous one. (Per owner decisions D-27/D-28, stage 8.2b will split a lower `micro` target for microcontrollers, where output always compiles to native firmware through w2c2 + the board C compiler rather than an on-device wasmi interpreter; `device` covers small Linux machines and the default Core Wasm baseline.) GC, relaxed-SIMD, threads, `trunc_sat`, wide-arithmetic, custom page sizes and reference-types instructions are in **no** target. `multi-memory` is in no target either: ABI 1.0 has exactly one linear memory (section 1), so the feature was removed until an ABI bump (BAD ACTOR F2); `features multi-memory` is an unknown feature (`OXS-E0402`). `memory64` is *declarable* on `server`, but ABI 1.0 requires a 32-bit memory and the engine has it off, so a module using it is refused (`OXS-E0411` when it declares it, `OXS-E0408` when it does not): it needs an ABI bump too. Adding Wasm exception handling (`exceptions` / `exnref`, stage 8.6, decision D-24) is an **opt-in ABI minor bump** (`oxi_abi_version` minor increments for modules that declare `features exceptions`; EH-free modules remain ABI 1.0).

**Host side.** `HostOptions::max_target` (default `device`) is the largest target the host runs. A host *provides* a feature when `max_target` allows it **and** its engine has it (`load::ENGINE_FEATURES` = `tail-call`, `extended-const`: the wasmi build has no exception handling, and SIMD/memory64 are switched off). `Host::with_options` switches every other proposal off explicitly (`wasm_simd`, `wasm_relaxed_simd`, `wasm_memory64`, `wasm_multi_memory`, reference types, ...), whatever the wasmi Cargo features or their defaults are after feature unification.

**`OXS-E0411` (module needs more than this host provides).** Raised before the engine sees the module when the manifest names a target above `max_target`, declares a feature that `max_target` does not allow or the engine lacks, or declares features but the code uses more than it declared (the manifest lies: a `device` module containing `return_call` is refused even by a `web` host).

**The fail-closed rule.** The bytes of every module are validated (all proposals known to the validator) against *Device base + declared features*, whatever the manifest says (a missing or unreadable manifest counts as Device with no features; `check` then reports it as `E0402`). A module that needs anything else is refused by the host itself, `E0411` if the host could run it with its full set, otherwise `E0408`; it is never passed on with "the engine will refuse it". Before and independently of the engine the host also scans the module: a non-function import (memory, table, global, tag: `E0405`), memory64, table64 or shared memories (`E0408`). The manifest section is size-checked (`MAX_MANIFEST_BYTES`, 4096) before it is parsed, and manifest tokens are separated by ASCII space or tab only.

## 4. Instance lifecycle and reload

`instantiate` -> `oxi_init(caps...)` -> events (`cb:` callbacks, user exports, `oxi_tick`) -> `oxi_deinit`. **Hot reload** builds a *new* instance and swaps atomically: module-local state (module-level `let`, closures, caches) resets; state that must survive lives in the host's `state` capability (schema-versioned, see the IDL example). The old instance's callbacks registered with the host are dropped by the host when the old instance is discarded.

**Host-driven cycle collection policy (incremental, never charged to the guest call).** After a successful `call_export`, `tick` or non-empty `pump` turn, `oxiscript-runtime` looks at the collector's counters through `oxi_collect_stat` (one tiny call) and collects when the candidate buffer has reached the collector threshold (1,024 candidates), when linear memory grew by more than 4 pages since the last pass, or under memory pressure (>= 50% of `max_memory_pages`) with candidates or pending destruction work. The work is done with `oxi_collect_step(units)`, **never** with the unbounded `oxi_collect_cycles`, and never with `oxi_live_objects`/`oxi_live_bytes` (which drain without bound). Rules:
* **Budgeted by work, not by live objects.** A step is bounded in collector work units (object and edge visits), so a 130,000-edge array costs what it really costs. The host plans `GC_FUEL_PER_UNIT = 700` fuel per unit (measured worst case of the release runtime 560, of the debug runtime 1,042; `fixes_link2::collector_step_fuel_per_unit_stays_under_the_backstop`) and gives each step a fuel limit of 4x the plan plus a fixed 10,000 as a backstop only: the unit bound, not fuel, ends a step, so a step is never cut off in the middle of a trial (which would leave the collector inconsistent). A debug-runtime pass can overshoot its nominal budget by about 1.5x; a release-runtime pass cannot.
* **Per-turn housekeeping budget** `pump_fuel.max(fuel_per_call)` (30M by default), separate from the guest's `fuel_per_call` (`last_fuel_used` is untouched). A turn uses far less in practice (it stops when the collector is idle). When the budget runs out mid-collection the pass stops cleanly and resumes on the next turn (the collector keeps its sweep position).
* **Escalation.** Steps start at 4,096 units. A step that burned most of its budget without shrinking the candidate buffer met a candidate whose reachable subgraph is bigger than the step; the host retries with 4x bigger steps up to `budget / GC_FUEL_PER_UNIT` units. If even a full-budget step cannot take the candidates (a huge live graph kept purple), the host **backs off**: it leaves them alone for 64 turns, doubling to at most 1,024 turns while passes stay unproductive, unless memory pressure or growth asks again. Such candidates are reclaimed by out-of-memory recovery (bounded, see 3.1), `collect_cycles()` or `deinit()`.
* `Runtime::collect_step(units)` exposes one step to embedders and `Runtime::collector_stats()` the accounting (steps, fuel, worst fuel per unit).
This keeps long-running instances from leaking cyclic garbage to `out_of_memory` for any live-set size (the previous `live_objects * 1000 <= budget` guard switched collection off for good above 30,000 live objects and could turn a successful call into `OutOfFuel`). `deinit()` runs `oxi_deinit()`, then `oxi_collect_cycles()`, then `oxi_live_objects()`.

## 5. Host interface definition language (IDL)

A capability is an **interface**; scripts see it as an opaque type (`import { Log } from "host:log"`) and receive an instance as a parameter of their entry point (**capabilities are values, there is no ambient authority**). The IDL is a small text format (`idl.rs` has the parser and the EBNF in its module docs):

```
// idl/log.idl
interface Log "host:log" @1 {
    fn write(level: u8, msg: str);
}
// idl/state.idl
interface State "host:state" @1 {
    handle Slot;
    callback OnChange(slot: Slot, key: str);
    fn open(key: str) -> result<Slot>;
    fn get_i64(slot: Slot) -> result<i64>;
    fn set_i64(slot: Slot, value: i64) -> result<void>;
    fn read_str(slot: Slot) -> result<str>;
    fn write_str(slot: Slot, value: str) -> result<void>;
    fn watch(slot: Slot, cb: OnChange);
}
```

Types: `bool i8 u8 i16 u16 i32 u32 i64 u64 f32 f64 str bytes`, `handle` types (host-managed opaque ids, generational, validated by the host on every call), and `callback` types (parameters only). `result<T>` is the fallible return form.

### 5.1 Mapping to plain Wasm imports

Import module `host:<name>@<version>`, field = function name. **Every function receives the instance's capability handle as an implicit first `i32` parameter** (`self`): the host validates it on each call, so a capability cannot be forged and revocation is a host decision.

| IDL | Wasm parameter(s) |
|---|---|
| `bool`, `i8..i32`, `u8..u32` | `i32` (canonical) |
| `i64`/`u64`, `f32`, `f64` | `i64`, `f32`, `f64` |
| `str`, `bytes` | `(ptr: i32, len: i32)`, **borrowed for the duration of the call**; `str` is UTF-8 |
| handle `H` | `i32` |
| callback `C` | `i32` closure pointer, ownership passes to the host (it holds one reference until it drops the registration) |

Results: none = `()`; `-> T` (primitive or handle) = `(T)`; `result<void>` = `(i32 status)`; `result<T>` = `(i32 status, T)` using Core Wasm **multi-value**; `result<str|bytes>` = `(i32 status, i32 len)` plus two **extra trailing parameters** `(out_ptr: i32, out_cap: i32)`. A plain `-> str` is not allowed. Status codes: `0` ok, `1` buffer too small, `2` denied (bad/revoked capability or handle), `16+` host-defined; a failed call returns a zero value, except that for `result<str|bytes>` status `1` carries the required length in `len` (5.2). The language lowers `result<T>` to the language-level `Result<T, i32>` (the `err` payload is the status); status `1` is consumed by the generated retry loop and never reaches the program.

**Functions without a status channel** (`-> void`, `-> T`) cannot report denial or revocation: if the capability handle or a handle argument is invalid, the host **traps the instance** (an engine trap; `oxi_trap_kind` stays 0). Interfaces whose operations can legitimately fail or be revoked therefore declare them `result<...>`.

**IR view of an import.** `oxiscript-ir` declares imports with the *typed boundary* so untrusted IR never handles raw addresses: `str` is `str` (borrowed, flattened by codegen to `(ptr, len)`), `bytes` and the out-buffer are `[u8]` (borrowed, flattened to `(ptr, len)`, where `len` is the array length = `out_cap`), a callback is `fn(...) -> R` and is **consumed** by the call (the host keeps the reference), handles and numbers are scalars, and results are flat scalars (multi-value). After an out-buffer call the guest builds the string with `str.from_utf8 buf, len`.

### 5.2 Out-buffer protocol for returned strings and bytes

The guest allocates a `[u8]` buffer, passes its `(out_ptr, out_cap)`, the host writes up to `out_cap` bytes into that buffer **and nowhere else** and returns `len` (status `0`). If `len > out_cap` the host returns status `1` and the required `len` **without side effects**; the guest retries with a larger buffer. A host function returning `str`/`bytes` must therefore be idempotent up to its commit point (it may be called twice). Returned `result<str|bytes>` payloads are capped by the host's configured payload limit (`Limits::max_host_payload`, default 64 KiB in `oxiscript-runtime`; contrast with host-to-guest bulk parameters in section 5.4, which pass through the scratch region and are capped at `HOST_PAYLOAD_MAX = 4096`).

### 5.3 No re-entrancy (MVP rule)

While a host import runs the host **must not call into the instance** (no exports, not even `oxi_alloc`). All data flows through the arguments, the results and the out-buffers above; host-to-guest *calls* (callbacks) happen only when the guest is not running. This keeps wasmi/WAMR/wasm2c identical, removes a class of sandbox and aliasing bugs, and means the guest allocator is never entered from the host side. Consequently the host cannot call `oxi_release` on a callback closure from inside an import; it queues the release and performs it at its next quiescent point (between events).

### 5.4 Callbacks (host to guest)

For each IDL `callback C(params)` the module exports `cb:<module>@<version>.<Interface>.<C>(closure: i32, flattened params) -> flattened result` (for example `cb:host:state@1.State.OnChange`; module and version are part of the name so two interfaces or two versions of one capability never collide). The result is none or a primitive. **Delivery of bulk data:** a callback has **at most one** `str`/`bytes` parameter (the IDL parser rejects more); it arrives as `(ptr, len)` into the **scratch region**: the host writes the bytes at `oxi_scratch_ptr` before calling (`len <= HOST_PAYLOAD_MAX = 4096 <= oxi_scratch_cap`); the guest copies if it keeps them. A host that has more data must not deliver it (it fails or splits the originating operation itself); there is no chunking protocol in ABI 1.0. The `cb:` trampoline is generated by the code generator: it builds a `str`/`[u8]` from the scratch bytes (`oxi_str_new`) and `call_indirect`s through the closure's table index.

**Passing a callback to the host.** The IR passes an owned closure as a `fn(...)` argument of the import; that argument is consumed and the Wasm argument is the closure pointer (an `i32` the module itself never forges: it only ever comes from a consumed `fn` value). The host later drops its registration by queueing `oxi_release(closure)` for its next quiescent point (5.3); the balance rule is: one `fn` argument consumed = one reference owned by the host = one `oxi_release` by the host. The closure was passed to the host by a previous call (`watch(slot, cb)`); the trampoline performs the `call_indirect` through the closure's table index.

## 6. Reason codes

Every diagnostic, placement report entry and law outcome has a stable code `OXS-<class><4 digits>`: `E` error, `W` warning, `P` placement/performance note, `L` law outcome, `I` internal error (class/severity pairing is enforced). The registry is the text file `oxiscript-abi/registry/codes.txt` (parsed by `codes::parse_registry`; format and number ranges in `codes.rs` docs):

```
code OXS-E0101
severity error
phase check
title Switch is not exhaustive
message `switch` over `{ty}` does not handle: {missing}
fix Add the missing cases or a `default`.
```

Rules: codes are **append-only** (never renumbered, never reused, never changing meaning); messages use `{name}` placeholders filled by the emitter (`codes::render`); each slice adds codes in its own range; `P` codes carry `assertable yes|no` (yes = a semantic fact `@expect` may assert and that only gets stronger within an edition; no = a heuristic choice that is reported but never assertable). The placement report (`placement.json`, slice 8.S4) and the LSP use exactly these ids, so a report from one compiler version diffs cleanly against another. The registry is data, so `llms.txt`/model-oriented docs can be generated from it.
