# Synchronous callbacks across the OxiScript sandbox boundary (input for 8.N / ABI 1.1)

> **Superseded in part (2026-11-26):** the premise of this report, native bindings that need sibling Wasm instances and ABI 1.1, was overtaken by decision D-14 (Rust libraries are linked statically into the program, stage 8.N; [../roadmap/oxiscript.md](../roadmap/oxiscript.md#d-14)). The report stays as the reference for kind-D native bindings (`sync callback`, see the ideas section of the roadmap). Its reference numbers 28 and 29 are unused.
>
> **Note (2026-11-26, plan-cleanup):** this document predates the roadmaps. Where it cites `oxi-ui-plan-history.md` with line numbers or section names, it means the old OxiUI plan file as it was at commit `3017fd5` (its Phase 1-7 text is kept in `docs/history/oxi-ui-plan-history.md`; its Phase 8 text, principles `P-n` and decisions `D-n` are now in [../roadmap/oxiscript.md](../roadmap/oxiscript.md), the OxiUI half in [../roadmap/oxiui.md](../roadmap/oxiui.md)).

Compiled 2026-11-25 for slice 8.N (native Rust bindings, `oxi-bindgen`). It looks at the `sync callback` design in `docs/history/oxi-ui-plan-history.md` (row 8.N, ABI 1.1 proposal (b)) against the literature and production systems. Authoritative local context: `docs/oxiscript/abi.md` sections 2-5, `docs/oxiscript/contracts.md` (convention 10), `docs/oxiscript/ir.md` section 4.

**Verification legend** (same as `oxiscript-known-attacks.md`):
- **V**: the primary source was read in this session (spec text, paper text or official docs page).
- **S**: confirmed from search-result snippets of the named source; the page was not read in full.
- **R**: recalled from prior knowledge and not re-checked this session. Check before relying on it.

Reference numbers `[n]` point to section 6.

---

## 1. Executive summary

**Three safety properties need to be kept apart.** "Mathematically safe" means nothing until we say which one we mean:

1. **P1, host safety (sandbox integrity).** Guest code cannot corrupt the host or reach anything outside its capabilities. Every option below has P1 through Wasm's isolation of linear memory and control flow; Iris-Wasm proves robust safety for Wasm 1.0 on top of WasmCert [21][22]. A sync callback adds no new P1 risk as long as the host never holds a Rust borrow of guest memory across the callback. That is a real trap in wasmi/wasmtime, because a callback can run `memory.grow` and the host-side backing buffer may move.
2. **P2, guest-internal memory safety.** No object reachable from a live outer frame (including borrows held by frames further up the stack, `ir.md` section 4 "Unprotected borrows") is freed, mutated or seen half-updated while the callback runs. **This is the property re-entrancy threatens.** The sibling instance buys it *by construction*, because memories are disjoint.
3. **P3, determinism and semantic transparency.** The result and the fuel use are the same on every host and target, and the callback cannot tell which mechanism ran it. Neither design gives P3 by itself. Fuel spent inside a Rust `sort_by` depends on how many comparator calls the Rust sort makes and in what order, and that changes between Rust releases (Rust 1.81 replaced the sort implementations, and they may now *panic* on a non-total order [30]).

**Main findings**

* **The Component Model no longer forbids re-entrance.** The `may_enter` flag/trap was **removed on 2026-08-28** (component-model PR #705, "not load-bearing") [2]. The concurrency explainer says there is "no reliable way to eagerly distinguish" recursive from non-recursive re-entrance "without introducing significant false positives and negatives", and that a host "must be careful to only perform recursive reentrance on components when explicitly supported by the component's documented API" [3]. **That is an argument for our explicit opt-in `sync callback` IDL kind**, not against it. What survives is `may_leave`: core code may not call imports while `realloc` or post-return runs [1]. That maps exactly onto our rule that a sync callback gets no capabilities. The `num_lends`/`num_borrows` counters [1][3] protect only *directly lent handles*, not the object graph reachable from them. That is the same gap for which `oxi-ui-plan-history.md` already rejected naive re-entry.
* **The callback design matters less than how often callbacks run.** No production system makes fine-grained host-to-guest callbacks cheap across a real isolation boundary. Each one batches the work, inverts control, or makes the inner calls coarse:
  * Erlang NIFs must return within about 1 ms and reschedule with `enif_schedule_nif` [12].
  * Lua resumes yields across C frames with continuation functions [13].
  * V8 fast API calls may not run JS at all [15].
  * eBPF `bpf_loop` callbacks are verifier-checked, non-recursive and bounded, with return values restricted to 0/1 [16].
  * RLBox requires callbacks to be registered and unregistered with RAII lifetimes, and their arguments are tainted [7][8].

  In wasmi (an interpreter) one host-to-guest call probably costs about as much as a few dozen interpreted instructions [R, unmeasured]. According to search summaries of the wasmi v0.32 blog, its register engine favours compute-intense over call-intense workloads [S, 27]. So `sort_by` on 10k elements through native Rust plus about 130k callbacks is probably **no faster than a sort written in OxiScript std**, and it is non-deterministic in fuel. **This must be measured (8.N gate), not assumed.**
* **Option (d), inversion, removes most sync callbacks** and keeps ABI 1.0's no-re-entrancy rule intact on wasmi, WAMR, wasm2c and the browser:
  * Use key extraction (`sort_by_cached_key` style): n crossings instead of n log n. Better still, the host sorts an index permutation and the data never crosses.
  * Use pull and iterator APIs: `regex` returns match spans or captures in batches and the guest builds the output.
  * Use event-batch visitors.

  `oxi-bindgen` should generate these shapes by default.

**Recommendation** ("right + fast + provably safe")

1. **Tier 0, no callback.**
   * `map`/`filter`/`fold`/`sort` over OxiScript data live in OxiScript std, inside the sandbox, fully metered and deterministic.
   * Bind a Rust crate only for work where the host does far more than one crossing's worth of work per call.
2. **Tier 1, inversion.** An `iter`/batch IDL form and an index-permutation sort protocol. ABI 1.0 semantics, zero re-entrancy, no new proof obligation. This is the default output of `oxi-bindgen` for closure-taking Rust APIs.
3. **Tier 2, true `sync callback`, only for irreducible cases** (third-party visitor APIs, `replace_all` with a closure when inversion is impossible). Keep the declared contract, the "no capabilities" rule (`may_leave = false`), purity and immutable captures, panic-to-trap, and a poison flag. Change the execution model in two steps:
   * **Ship (a′) first: a sibling instance pinned for the whole outer import.** Captures are copied once per import, not once per callback call. Arguments are indices or scalars wherever possible. Each callback call carries no copying, only its scalar arguments. The proof rests on memory disjointness, which the engine provides for free.
   * **Evaluate (b), frozen-region re-entry into the same instance,** as the fast path once 8.M/8.G can emit it:
     * Callback-mode code allocates only from a fresh bump arena above a base address `B`.
     * `retain`/`release` on objects below `B` become no-ops (they behave like sticky objects).
     * `setfield`/`array.set`/`array.push`/`global.set` to targets below `B` trap.
     * The deferred-destruction queue and the cycle collector's candidate buffer are paused.
     * `oxi_is_unique` returns false for any object below `B`, so copy-on-write always clones instead of mutating a captured struct in place.
     * The result is deep-copied out into the normal heap. That copy takes real `retain`s on any frozen objects it references. Then the arena is reset. Adopting the arena pages without the copy is **unsound**: a reference from the arena into the frozen region was retained as a no-op.

     This is Vale's immutable region borrowing [44] plus Lean/Swift borrowed parameters [45], with a dynamic guard. Its invariant, "while frozen, no write and no RC change below `B`", is a *syntactic property of the emitted Wasm*, so a post-codegen checker in the style of VeriWasm [24] can verify it. It costs one compare per store in callback-mode code, and there is no copying of captures or of arguments passed by reference. Switch from (a′) to (b) only if the gate benchmarks show the copies dominate, and only after BAD ACTOR has attacked the guard.
4. **Mechanised proof.** Mechanising the *frame-preservation lemma* for (b) over OxiIR is realistic in Lean 4 or Coq, at the cost of a few person-months of an experienced prover user. RustBelt-scale or Iris-Wasm-scale verification of the whole pipeline is not realistic for us. Use Iris-Wasm, WasmCert and VeriWasm as the *trusted base* and argument template, not as something we re-prove. A small model of the protocol (state machine plus invariant) checked in TLA+ or Lean, together with property tests and fuzzing against the Rust implementation, gives most of the assurance for a fraction of the cost (section 3.2.4).
5. **C++.** Do not bind native C++ into the server process with the same trust level as Rust. The safe path is to compile the C++ library to Wasm (wasi-sdk/Emscripten). Then either run it as its own sandboxed instance with copy-in/copy-out (RLBox-style tainting at the boundary, as Firefox does with wasm2c [7][9]), or link it inside the guest module, so callbacks become ordinary calls and the only damage a bug can do is to that instance. Native C++ bindings via cxx/autocxx are acceptable only for audited libraries, with `noexcept` boundaries and process isolation as a fallback (section 3.4).

---

## 2. Comparison of options

Latency figures are order-of-magnitude reference points, not wasmi measurements; see section 3.2.1 for the sources and the cost formula. "Crossing" means one host-to-guest call plus return. n = 10k elements, C ≈ 130k comparator calls.

| Option | Guarantee for P2 (outer borrows) | How enforced | Latency per crossing | Copy cost | Complexity | Browser (local mode) | ESP32 / wasm2c | Fit with RC + borrows |
|---|---|---|---|---|---|---|---|---|
| **(a) sibling instance, copy per call** (current text of 8.N) | By construction: disjoint linear memory | Engine isolation; contract validation on both directions; no imports linked in sibling | Base call + per-argument lowering into sibling (allocations in sibling = more guest calls) | O(size of arguments) per call, plus captures per call if not pinned; for sort on records ≈ 2 C element copies | Medium: instance pool, reset, sibling init entry (must not run `main`), copy/validate generator | Works (a second `WebAssembly.Instance`); memory per sibling | wasm2c: second instance = second memory struct, fine; RAM per sibling is the problem on 8 MB PSRAM | Clean: nothing in the caller's heap is touched; borrows trivially preserved |
| **(a′) sibling pinned per import + index arguments** (recommended first) | Same as (a) | Same as (a) | Base call only (scalar or index arguments) | Captures and input data copied **once per outer import**; per call only scalars | Medium (same machinery as (a)) | Works | Works; one pinned sibling per active import | Clean |
| **(b) same instance, frozen region + arena** | Invariant "no write and no RC change below `B`", checked dynamically by store barriers and RC no-ops; statically by 8.T purity | Codegen guard (trusted) + post-codegen Wasm checker + 8.T purity + RC-balance checker | Base call only; +1 compare per store in callback code | None on entry; result deep-copied out (arena adoption is unsound, see 3.2.2) | High: second codegen mode for pure closures, arena allocator, pausing the deferred queue and candidate buffer, nesting depth | Works (plain Wasm re-entry via JS import) | Works if the engine allows re-entry (wasm2c: plain C call, yes); no extra instance RAM | Good, but it touches the RC runtime; sticky-like treatment of old objects must be proven |
| **(c) static purity + no-free proof only, re-enter same instance** | Only as strong as the checker; one checker bug becomes a UAF | 8.T effect system + RC-balance checker; no dynamic guard | Base call | None | Medium (checker work), low runtime | Works | Works | The `oxi-ui-plan-history.md` objection applies: retains in the importing frame do not protect borrows further up; it needs a whole-program "no release of pre-existing objects" proof, which is exactly what (b) checks dynamically |
| **(d) inversion: iterators, batches, index-permutation protocol, resumable host functions** | Trivial: no re-entrancy (ABI 1.0 rule kept) | ABI 1.0 rule 5.3; no new invariant | 1 import call per *batch* (or per step for stepwise protocols) | Batch payload only; sort by keys: n keys once | Low to medium per API (bindgen templates); stepwise sort needs a host coroutine or a vendored resumable algorithm | Works | Works (best fit: no nested stacks) | Clean |
| **(e) Rust library compiled to wasm32 and linked inside the module** | P2 depends on the Rust code being safe and the callback being pure (Rust may hold `&` into OxiScript objects) | rustc type system inside the sandbox; sandbox for P1 | An ordinary `call_indirect` (ns) | None | High: wasm32 build, shared allocator or separate memory, ABI glue; library runs *interpreted* in wasmi, roughly an order of magnitude or more slower than native [R] | Works (best fit for local mode) | Works if the crate builds for wasm32 / no_std; code size | Good if the library treats OxiScript objects as opaque handles |
| **(f) shared-nothing message passing (isolates, actors)** | By construction | Separate instances, copies | Message plus scheduling: far too slow for comparators | Full copy per message | Medium (reserved `isolate` design) | Works | Works | Clean; right tool for coarse work, wrong for `sort_by` |
| **(g) stack-switching / async lift (Component Model 0.3 style)** | Run-to-completion per instance; still needs (a) or (b) for P2 | Engine feature | Base call + switch | Depends | High; needs non-Core features excluded by ABI 1.0 | Not in Core Wasm MVP | wasm2c: no | Out of scope for ABI 1.x |

---

## 3. Findings per question

### 3.1 How others solve safe synchronous re-entrancy and callbacks

Each entry covers the mechanism, what it guarantees, and the lesson for us.

**WebAssembly Component Model (canonical ABI)** [1][2][3][4] (V)
* *Re-entrance.* Older drafts trapped on recursive entry through `may_enter`. PR #705 (merged 2026-08-28) removed `may_enter` from `canon lift`, `resource.drop` and `subtask.cancel`, because the trap "doesn't actually appear to be load-bearing" once other kinds of re-entrance exist [2]. Concurrency.md "Reentrance": a callee may re-enter the caller's instance during any import. Recursive re-entrance is hazardous, and there is "no reliable way to eagerly distinguish" it from benign re-entrance "without introducing significant false positives and negatives". So hosts should only re-enter "when explicitly supported by the component's documented API" [3]. Component Invariant #2 (run to completion) is kept only for the async callback ABI, by backpressure [4].
* *`may_leave`* is still present. It is cleared while the runtime calls the guest's `realloc` and `post-return`, and nearly every built-in does `trap_if(not inst.may_leave)` [1]. Guest code that the host calls *for the host's purposes* therefore cannot call out. This is the exact analogue of "a sync callback gets no capabilities".
* *Borrows.* A handle lent to a callee increments `num_lends` and dropping it while lent traps. Each `borrow` received increments the task's `num_borrows`, and returning with outstanding borrows traps [1][3]. These counters are dynamic and protect **handles**, not memory reachable from them.
* *Lockdown.* Any trap locks down the whole store, and no re-entry is possible afterwards (Invariant #1) [4]. This matches our "after a trap the instance is discarded" rule (`abi.md` 3.4).
* *Copying and validation.* Values are lifted and lowered between linear memories by copying (`realloc` in the destination). Lifting validates every value: a `char` outside `[0, 0x110000)` or a surrogate traps, as do a variant discriminant out of range, a misaligned or out-of-bounds pointer, and a string longer than `MAX_STRING_BYTE_LENGTH` [1]. This is the template for our contract check.
* *Lesson.* Shared-nothing plus copying is the standard's answer for P2 *between components*. *Inside* a component, re-entrance is the toolchain's problem. Our explicit `sync callback` kind with a documented contract is exactly what the explainer recommends.

**Wasmtime / Wasmer / wasmi host calls**
* Wasmtime: host functions receive a `Caller` that acts as `AsContextMut` and can look up exports [V, 5]. Calling guest functions from a host function is the standard way bindings call `realloc`, so core-level re-entry is allowed [R]. Reported overheads [S, 6]:
  * typed calls about 20 ns floor in 2021, with "nop" calls measured at about 43 ns (Rust API) and about 51 ns (C API);
  * a 2025 user saw about 2 µs per import through the C API; the maintainers attributed it to the API path and pointed to `_unchecked` variants and a PR claiming about 10 ns.
* wasmi: `Caller` gives store access [V, 26]. Host functions can call back into Wasm through `AsContextMut` [R]. wasmi also has **resumable calls**: `call_resumable` returns a `ResumableCall` after a host trap or out-of-fuel, so the host can resume the guest later [V, 26]. That is a stackless way to invert control (option d). The v0.32 register engine favours compute-intense over call-intense workloads, and v0.28 made Wasm calls faster at the cost of host-call performance [S, 27]. **No published wasmi host-call figure was found**; the 8.N gate has to measure it.

**RLBox (Firefox)** [7][8][9] (V paper, S blog)
* Everything coming out of the sandbox is `tainted<T>` and has to be copied and verified before use. Callbacks must be registered explicitly with `sandbox_callback()`, enforced by compile errors, and their parameters are tainted.
* Static allow-listing alone "is insufficient": the sandbox can invoke callbacks "at unexpected times". So RLBox supports `unregister()` and RAII registration that unregisters "after completion of an invoked libjpeg function".
* *Callback-state exchange attacks*: the sandbox swaps the state pointer handed to a callback. RLBox counters them by raising a compile error when renderer pointers leak into the sandbox and keeping callback state in thread-local storage. `setjmp`/`longjmp` across the boundary needs special handling.
* Production backend: C/C++ compiled to Wasm, then translated back to C by wasm2c. In Firefox 95 it covered Graphite, Hunspell and Ogg, with Expat and Woff2 following in 96 [S, 9].
* *Lesson:* a sync callback should be valid only for the dynamic extent of one import. Its arguments are untrusted and validated, it gets no host pointers, and it has a defined error path.

**NaCl / SFI and zero-cost transitions** [10][24] (V abstract)
* Classic SFI uses heavyweight trampolines that save, clear and restore registers and switch stacks, and they dominate the cost when crossings are frequent. Mozilla could not ship font sandboxing with them.
* Kolosick et al. show that Wasm-structured code satisfying the *zero-cost conditions* (type-directed CFI, well-bracketed control flow, local state initialised before use) makes a transition safe as a plain function call. They prove this and check binaries with VeriZero.
* *Lesson:* in an interpreter the transition is already as cheap as it can be. What remains is the guest-runtime invariant P2, which is a *language-level* zero-cost condition. Option (b) is the analogue: a syntactic condition on callback-mode code that makes the cheap transition safe.

**CHERI / CHERIoT compartments** [11] (S)
* The switcher clears unused argument registers, narrows stack bounds, and zeroes the part of the stack it passes to the callee, using a high-water mark to avoid re-zeroing. Callees declare their stack needs and the call fails if not enough stack is available.
* `__cheri_callback` exists for callbacks across compartments. Formal verification of the switcher is "early stage".
* *Lesson:* even with hardware capabilities, callbacks are ordinary cross-compartment calls with the same cost, plus explicit stack hygiene.

**seL4 IPC** [S, 25]
* Fastpath one-way IPC costs about 300 cycles (Haswell, 2013 figures). A 2024 mailing-list report gives under 900 cycles for a round trip on x86 with MCS.
* Callbacks do not exist: a server is just another thread that receives messages. *Lesson:* shared-nothing message passing is the proven model, at the cost of hundreds of cycles per message. That is roughly the same order as our wasmi call estimate and far above a native closure.

**Erlang NIFs** [12] (V)
* A NIF should return "within 1 millisecond". Longer work uses yielding NIFs (`enif_schedule_nif`), threaded NIFs or dirty schedulers. "A native function that crashes will crash the whole VM."
* The only documented route from a NIF back into Erlang is messaging (`enif_send`). A synchronous call into Erlang code from a NIF is not offered [R].
* *Lesson:* even BEAM, built for safety, does not do synchronous re-entry from native code. It inverts with schedule-and-resume.

**Lua C API** [13] (V)
* Lua implements yields and errors with `longjmp`, so a C function cannot be resumed after a yield across its frame. `lua_callk`, `lua_pcallk` and `lua_yieldk` take a continuation `k` that finishes the C function's work after a resume. The continuation is only used "in case of errors or resuming after a yield".
* *Lesson:* this is the classic stackless-resumable host function. It is exactly the shape our option (d) needs if a host algorithm has to "call" the guest without nesting, and wasmi's `ResumableCall` gives the engine side of it.

**JNI** [R]
* `JNIEnv*` is valid only on its thread. Local references live until the native method returns and are freed by `PushLocalFrame`/`PopLocalFrame`. Java callbacks from native code (`Call<Type>Method`) are fully re-entrant, and pending exceptions must be checked after each call.
* *Lesson:* scoped handle frames are the dynamic version of our "borrow valid for the call". JNI gives no protection against Java code mutating objects the native side holds.

**PyO3** [14] (V)
* `Bound<'py, T>` is "equivalent to the Rust tuple `(Python<'py>, Py<T>)`". Holding a `Python<'py>` token is the static proof that the thread is attached to the interpreter. `Py<T>` is detached and has to re-check attachment at run time.
* Rust closures handed to Python are ordinary Python callables. Python can re-enter Rust and Rust can call Python freely. Safety rests on the GIL plus `&self` borrows of `#[pyclass]` objects, and those are checked *dynamically*: a borrow already taken fails with a `PyBorrowMutError`-style error [R].
* *Lesson for `oxi-bindgen`:* a token type (`Guest<'g>`) that only exists while the guest is not running would let Rust's borrow checker reject host code that holds guest-memory slices across a callback (P1 hazard from section 1).

**V8 isolates and embedder callbacks** [15] (V fast-API)
* Fast API calls "should not allocate on the JS heap" and "should not trigger JS execution". A fallback to the slow path handles everything else.
* Normal (slow) callbacks may re-enter JS freely, under `HandleScope`/`Isolate::Scope` discipline [R].
* *Lesson:* V8 itself splits "no re-entry" (fast) from "re-entry allowed" (slow), the same split as our `fn` versus `sync callback`.

**Deno ops** [R]
* Ops are Rust functions behind V8 fast calls where possible. Async ops return promises instead of calling back into JS synchronously, so they invert through the event loop.

**eBPF verifier and helpers** [16] (S)
* `bpf_loop(nr, cb, ctx, flags)`, from 5.17, runs a *verifier-checked* callback up to 2^23 times. The callback may only return 0 or 1, recursion is rejected, and nesting is bounded by the stack-depth limit.
* Since 6.4, open-coded iterators replace many callback helpers.
* *Lesson:* the strongest production example of statically verified synchronous callbacks. Verification is cheap because the callback language is restricted (bounded, no recursion, typed returns), which is what 8.T "pure + immutable captures" aims for. The kernel also moved *away* from callbacks toward iterators: inversion again.

**Solidity / EVM** [17][18][19] (V/S)
* Re-entrancy is "not only an effect of Ether transfer but of any function call on another contract".
* The *Checks-Effects-Interactions* pattern makes external calls the last step, after all state updates. OpenZeppelin's `ReentrancyGuard` adds a `nonReentrant` status flag, now also in a transient-storage (EIP-1153) variant.
* Grossman et al. (POPL 2018) formalise **Effectively Callback Free (ECF)** objects: every execution with callbacks is equivalent to some execution without them. They show ECF can be checked online, and that nearly all real contracts were ECF except the DAO and similar exploits [19].
* *Lesson:* ECF is a precise statement of what P3 asks of a sync callback. Purity plus immutable captures makes every callback trivially ECF, because it commutes with everything.

**Move resources** [20] (S)
* Linear resource types give conservation: a resource is never copied or dropped implicitly. Move has *no dynamic dispatch*, "and thus also does not have re-entrancy". Robust Safety for Move ties the DAO attack to a dynamic dispatch made while an invariant was broken.
* *Lesson:* removing dynamic dispatch removes re-entrancy. Our sync callbacks are dynamic dispatch by definition, so the guarantee has to come from purity, not from structure.

**Capability-safe languages (E, Joe-E, Wyvern)** [R; 40][41]
* E's *vats* run event loops with eventual sends (`<-`) and deliberately avoid synchronous re-entry between vats ("plan interference" in Miller's thesis).
* Joe-E restricts Java to a capability-safe subset and includes a verifier for *functional purity* (Finifter, Mettler, Sastry, Wagner, "Verifiable functional purity in Java", CCS 2008): a method whose parameters are all immutable and which has no ambient authority is pure.
* Wyvern adds capability-based module systems and effect annotations on resources.
* *Lesson:* Joe-E's purity rule (immutable arguments plus no capabilities means deterministic and side-effect free) is exactly our 8.T rule. It is *checkable by types*, with no whole-program analysis.

**Effect handlers / algebraic effects** [R; Perceus V-abstract 43]
* With effect handlers, a library written against an effect `Compare` can be run by a handler that suspends to the guest. This is inversion without rewriting the library, but it needs first-class continuations, which Core Wasm lacks (stack-switching is a separate proposal).
* Koka combines typed effect handlers with precise RC (Perceus, PLDI 2021, which proves RC soundness and garbage-freedom in a linear resource calculus) [43]. The effect system there *is* the purity check: a function with an empty effect row (`total`) cannot observe or mutate state.

**Linear/affine types and regions**
* *Cyclone* [R, 47] used lexical regions and region subtyping to prevent dangling pointers. A function can be polymorphic over the regions it reads.
* *Rust* [R]: a shared borrow `&T` freezes the referent for the borrow's lifetime. RustBelt (Iris, Coq) proves the type system and selected `unsafe` libraries sound [R, 42].
* *Vale* [S, 44]: "pure functions can temporarily treat multiple regions as one combined immutable region", so inputs marked temporarily immutable need no RC or generation checks while the function runs. Marked "planned features" in the guide, so it is a design, not a deployed system.
* *Lean 4 / Swift borrowed references* [S, 45]: a borrowed reference "does not actually keep the referenced value alive, but instead asserts that the value is kept alive by another, owned reference". Borrow annotations are inferred, and the idea comes from Swift.
* *RichWasm* [S, 23]: linear capabilities as ownership tokens for read or read-write access to shared Wasm memory, with type safety proved in Coq. It shows ownership across a Wasm boundary can be *typed*, but it needs a richer IR than Core Wasm.
* *Lesson:* (b) is the region pattern applied to a callback: everything allocated before entry is one immutable region for the callback's dynamic extent.

**Ownership across FFI (binding generators)**
* *cxx* [V, 31][32][33]: a "regime of commonality" checked statically on both sides, "no copying, no serialization, no memory allocation, no runtime checks". An `extern "C++"` function returning `Result` has C++ exceptions caught and turned into `cxx::Exception`; without `Result`, an exception calls `std::terminate`. A panic in `extern "Rust"` aborts the process. `rust::Fn` passes only Rust function pointers to C++, and there is no C++-to-Rust function pointer yet.
* *autocxx* [R]: generates cxx bindings from C++ headers via bindgen and inherits cxx's model. Templates are handled only when instantiated concretely.
* *uniffi* [V, 34]: callback interfaces (foreign code implementing a Rust trait) are "soft deprecated" in favour of foreign traits. They must be safe to call from several threads, which Rust cannot enforce on the foreign side.
* *Diplomat* [S, 35]: callbacks exist (`DiplomatCallback`, `!Send`, with an optional destructor), but its source notes "we do not support borrowing across callbacks". Lifetimes must be spelled out for borrowing methods.
* *wasm-bindgen* [V, 36]: stack closures `&dyn Fn` passed to JS "become invalidated" when the imported function returns, and later calls throw. `Closure<T>` heap closures live as long as the Rust owner and are invalidated on drop. That is the same split as our `callback` (registered, host-owned) versus `sync callback` (valid for one import).
* *wit-bindgen* [R]: generates canonical-ABI glue. Callbacks are expressed as exported functions or resources with methods, never as closure values across the boundary.
* *Lesson:* no mainstream generator lets a callback *borrow* across the boundary. All of them copy, or restrict callbacks to the dynamic extent of a call.

### 3.2 Design space for OxiScript

#### 3.2.1 Cost model (to be filled by the gate benchmarks)

For one outer import that triggers `C` callbacks:

```
T = T_import + C * (T_cross + T_args + T_check) + T_setup
T_cross  = host->guest call + return in the engine     (wasmtime typed: ~10-50 ns [S,6]; wasmi: unknown, measure)
T_args   = lowering of arguments into the callee memory (sibling: alloc + copy + header; same instance by index: 0)
T_check  = contract validation of arguments and results  (scalars: a compare; strings: UTF-8 scan; records: per field)
T_setup  = sibling acquire + capture copy + reset         (a: per call; a': once per import; b: arena open/close)
```

Reference points: native Rust `sort_by` on 10k `u64` keys takes on the order of 100 µs [R]. With `C ≈ n log2 n ≈ 133k` and a guessed `T_cross = 100 ns` in wasmi [R, to be measured], the crossings alone cost about 13 ms, roughly 100x the native sort. A sort written in OxiScript and interpreted by wasmi, with a comparator of a few dozen instructions, lands in the same range [R]. So for comparators **the callback design cannot be "fast" in absolute terms in an interpreter**. It can only avoid being needlessly slow: no per-call copies, no per-call setup. Under wasmtime or AOT (wasm2c) `T_cross` falls to about 10-50 ns, and the copy terms then dominate in (a), which is why (a′) or (b) matter.

Cheap wins regardless of option:
1. **Pin the sibling per import (a′).** Captures are copied once, the sibling is reset once.
2. **Index protocol.** For `sort_by(xs, cmp)` the host sorts a `Vec<u32>` permutation `0..n` and calls `cmp(i, j)`. The guest comparator reads its own `xs[i]`, `xs[j]` (the guest closure captures `xs` immutably), and the guest applies the permutation afterwards. With (a′) the sibling receives `xs` once. With (b) nothing is copied at all.
3. **Key extraction.** `sort_by_key(xs, key)` calls the guest n times (not n log n), then sorts natively on the extracted keys (scalars or strings copied once). This is Rust's `sort_by_cached_key` idea.
4. **Batching.** `filter`/`map` with a guest predicate should run in the guest, since there is nothing native to gain. `regex.replace_all(text, f)` becomes `regex.captures_all(text) -> [Match]` followed by a guest loop that builds the output: zero callbacks.

#### 3.2.2 The options in detail

**(a) Sibling instance with copy (current 8.N text).**
* *Good:* P2 by construction, because the caller's memory is unreachable. The contract check at both edges catches corrupt data from the host or the sibling. It reuses Component Model thinking (shared-nothing, lift/lower validation [1]).
* *Problems found:*
  1. **Sibling initialisation.** `oxi_init` runs module initialisers *and* `main` with capabilities (`abi.md` 3.1). A sibling needs a new entry (`oxi_init_pure`) that runs only initialisers which are themselves pure, or 8.T must forbid callbacks from reading module-level `let`s, which reset in the sibling. Otherwise the callback silently sees different state. That is a P3 (semantic) failure, not a P2 one, and the sibling's dynamic enforcement does *not* catch it.
  2. **Copying captures is semantic only if they are deeply immutable.** A captured class instance (reference semantics) copied into the sibling loses identity (`===` on classes is identity, `contracts.md` 2 point 4). 8.T must reject identity comparison on captured class values, or restrict captures to structs, strings, scalars and arrays of those.
  3. **Per-call allocation in the sibling** means extra host-to-sibling calls (`oxi_alloc`, `oxi_str_new`) unless the host writes into a sibling scratch region and the trampoline builds the objects. Reuse the existing scratch protocol (`abi.md` 5.4) and its `HOST_PAYLOAD_MAX`.
  4. **RAM.** Each pooled sibling costs initial memory (static data plus minimum heap). On ESP32-S3 with 8 MB PSRAM a pool of 1 per *nesting level* (depth limit 1 or 2) is acceptable. A pool per concurrent import is not.
* *Reset:* a fresh instantiation, or a memory snapshot restore (copy-on-write snapshot of the post-init memory; wasmtime's pooling allocator does this with memory images [R]). In wasmi, re-instantiating a small module is the safe default; measure it.

**(a′) Sibling pinned per outer import.** Same P2 guarantee, with setup amortised. Recommended as the first shipped form. Caveat: the pinned sibling keeps its heap and module state from one callback call to the next within one import. If the purity check is wrong (for example a write to a module-level `let`, or a mutated captured class), call i can influence call i+1. That breaks P3, not P2. BAD ACTOR item 9 targets it, and a debug mode can reset the sibling after every call to expose such dependence differentially.

**(b) Same-instance re-entry under a frozen region.** The precise design:
* On entering a sync callback, the host sets `oxi_frozen_base = heap_top` (an exported mutable global) and `cb_depth += 1`. The allocator switches to a bump arena starting at the base. Free lists are not used, so a new object can never land in a recycled hole below the base.
* Code emitted for a closure whose type is `pure fn` (8.T effect) is compiled in *callback mode*:
  * `retain`/`release` of a pointer `< base` are no-ops: the object is alive because some outer frame owns it, and the frozen region behaves like sticky objects;
  * `setfield`/`array.set`/`array.push`/`global.set` with a target `< base` (or any `global.set`) trap;
  * `oxi_free_bytes`/`oxi_array_grow` on blocks `< base` trap;
  * host imports trap (the `may_leave` analogue; a sync callback has no capability handles anyway).
* The deferred destruction queue and the cycle collector's candidate buffer are paused while frozen. This is needed because a bounded `oxi_release` step could otherwise finish freeing an outer object queued earlier. That would be harmless in itself, since it is already dead, but it complicates the invariant.
* **Copy-on-write in frozen mode.** A captured struct may have rc 1, and the usual lowering (`is_unique` ? mutate in place : `clone`) would then write into outer state or hit the store guard. So in frozen mode `oxi_is_unique(p)` returns false for `p < base`, which forces a clone into the arena. The 8.M elision rule that turns a `clone` into in-place mutation when `is_unique` is proven is disabled in `#callback_mode` code for any value that may be older than the base.
* On exit, the result (scalar, or an object graph in the arena) is **deep-copied into the normal heap**. The copy takes a real `retain` for every reference into the frozen region. Then the arena is reset and `cb_depth -= 1`. **Adopting the arena pages as ordinary heap is unsound**: an arena object pointing to an old object was created with a no-op `retain`, so freeing it later would release the old object once too often, which is an underflow and a use-after-free. Adoption is sound only after a pointer-map walk that replays the missing `retain`s, and that costs the same as the deep copy.
* **Invariant (frame preservation).** For every frame F on the stack at entry, every object o reachable from F's live values (owned or borrowed): `rc(o)`, `meta(o)` and the payload bytes of o are unchanged at exit and at every point during the callback, and o is not freed. Proof sketch: all writes below the base are either impossible (they trap) or no-ops (RC). Allocation never returns an address below the base. The runtime support code never frees an object below the base while frozen (the queue is paused). So the memory below the base is bit-identical across the call.
* *Static side:* 8.T purity (no capability use, no mutable-global reads, immutable captures) makes the dynamic traps unreachable for well-typed code. The guards only fire on checker or codegen bugs. Defence in depth matches the BAD ACTOR philosophy.
* *Verification:* the guard is a syntactic property of the emitted Wasm of callback-mode functions: every store is dominated by a `ptr >= base` check, and every RC call goes through the frozen-aware runtime entry. A small post-codegen verifier can check this, the way VeriWasm checks SFI guards in machine code [24]. VeriZero is the precedent for "condition + checker + soundness proof" [10].
* *Cost:* one load of the base (a global) plus compare and branch per store and per RC operation in callback code. No copies of captures or arguments. The result copy is proportional to the result.
* *Risks:* a second codegen mode (callback mode versus normal) for every closure that might be used as a `pure fn`. Mitigation: only closures whose *static type* is `pure fn`, so 8.T knows at the closure literal. Nesting: a callback that calls `sort_by` again needs nested arenas (`cb_depth`, a base stack). Scratch: the callback trampoline must not use the scratch region while an outer import still holds scratch data. Rule: at every import call site the guest has already copied everything it needs out of scratch, so scratch is dead, and 8.L/8.G must guarantee that (BAD ACTOR item).

**(c) Static proof only, no guard.** It needs a whole-program fact ("this callback releases no pre-existing object to zero"). That fact does not follow from local RC balance: a balanced callback can still `release` an object it loaded through a field after retaining it. That is fine by itself, but nothing local stops it from freeing an object reachable from an outer frame whose only owner is a field the callback overwrote. Purity forbids the overwrite, so (c) is sound *iff* 8.T purity and the RC checker are both correct. The `oxi-ui-plan-history.md` rejection of re-entry stands for (c) in isolation. (b) is (c) plus a cheap dynamic check that makes checker bugs fail safe.

**(d) Inversion.** No new invariant. Variants:
* *Batch or iterator* APIs (`find_iter` to `[Match]`, visitor to event list, chunked).
* *Index protocol plus key extraction* for sorts.
* *Stepwise resumable host*: the guest calls `sorter.step(answer) -> Request{Compare(i,j) | Done}`. The host keeps the algorithm state in a handle. That needs either a resumable algorithm (a vendored merge sort written as a state machine, simple and deterministic, with a fixed comparison sequence, which fixes P3) or a host-side stackful coroutine (`corosensei` and similar; not portable to wasm32 browser hosts [R]).
* *wasmi `ResumableCall`* [26] could even let a *native* algorithm suspend the guest without nesting, but that is wasmi-only, so not portable.
* *Cost:* 1 import per step, in the guest-to-host direction, which in wasmtime is the cheaper direction [S, 6]. The stepwise form costs the same number of crossings as callbacks but has zero re-entrancy and a deterministic comparison sequence when the algorithm is vendored.

**(e) Rust library inside the sandbox.**
* Callbacks become `call_indirect`. Rust's own type system gives P2 for the library's data.
* The library must treat OxiScript objects as opaque (indices or handles), otherwise a Rust `&[T]` into OxiScript memory is invalidated if the callback mutates it (purity prevents that).
* Static linking needs a shared allocator (`__rust_alloc` routed to `oxi_alloc_bytes`) and agreement on the stack region. The alternative is a separate module with its own memory and copies, which is (a) again.
* The library runs interpreted under wasmi, so for compute-heavy crates (regex, image) it is slower than native by roughly an order of magnitude or more [R]. It is the only option that behaves identically in browser local mode and on ESP32 and is fully fuel-metered.
* `oxi-ui-plan-history.md` already schedules it as "later, separate stage".

**(f) Shared-nothing messages.** The reserved `isolate` design: correct for coarse work, too slow for per-element callbacks.

**(g) Better ideas found.**
* *Effect-typed library facade.* `oxi-bindgen` exposes a Rust API taking `impl FnMut(A) -> B` as either Tier 0 (guest std), Tier 1 (generated inversion), or Tier 2 (`sync callback`) by attribute. The default is chosen by rules (closure called O(n) times with cheap work → guest; closure feeding an algorithm with native-heavy work → inversion; irreducible → sync).
* *Deterministic fuel for native work.* Charge a declared fuel cost per native call (an amount in the binding contract) plus the callback's own metered fuel. To make P3 hold across Rust versions, vendor or pin the algorithm (or use the stepwise protocol), so the comparison sequence is part of the contract and not a property of whatever `std` sort the host was built with.

#### 3.2.3 Host-side hazards valid for every option
* **Guest memory slices across a callback (P1).** Host code that holds `&[u8]` into guest memory (for example the borrowed `str` argument of the outer import) must drop it before calling the guest, because `memory.grow` inside the callback may move the backing buffer [R]. In Rust this is enforced if bindgen passes guest memory only through a token borrowed from the store (PyO3-style token, section 3.1). Generated glue should copy outer `str`/`bytes` arguments into host memory first.
* **Panics.** Catching a host panic and turning it into a trap needs `catch_unwind`, which **does nothing under `panic = "abort"`**. Abort is the usual setting on wasm32 (browser local mode) and common on embedded targets [R]. There a panic kills the whole host process (server) or reboots the device (ESP32). Requirements:
  1. the bindings audit every closure-taking API for documented panics (Rust 1.81 sorts may panic on a non-total order [30]);
  2. prefer vendored, panic-free algorithms for callback-driven APIs (merge sort that tolerates inconsistent comparators);
  3. the server builds with `panic = "unwind"` and uses `catch_unwind` at every binding entry;
  4. availability per target (the `native` manifest mark) refuses panic-capable callback bindings on abort-only targets.
* **Errors inside a Rust closure.** A callback trap inside `sort_by(|a, b| ...)` has no error channel (`Ordering` only). Use a **poison flag**: on the first trap or contract violation, set the flag, return `Ordering::Equal` for all further calls without entering the guest, and check the flag when the library returns, converting it into a trap of the caller. An unwinding panic is the alternative where unwinding exists. The poison flag works everywhere.
* **Fuel.** Callback fuel is drawn from the caller's budget (same instance in (b); transferred in (a)). Exhausting it is a trap with the poison-flag path.
* **Depth.** Bound nesting (a sync callback calling a binding that calls a sync callback) to a small constant. In (a) this sizes the pool. In (b) it sizes the base stack.

#### 3.2.4 What must be proven, and is mechanisation realistic?

Proof obligations for the recommended tiers:
1. **(Tier 1) No re-entrancy.** Already an ABI rule. Proof = the host binding glue never holds a guest `Func` while inside an import. Enforced structurally if the generated code for inversion APIs never receives a guest function.
2. **(a/a′) Isolation.** Follows from Wasm instance isolation: two instances with separate memories, no shared tables or globals, no imports linked in the sibling. The formal basis is WasmCert plus Iris-Wasm robust safety [21][22]. What we must prove ourselves is only that the **contract check is total**: every byte written into the sibling or read back is validated, mirroring `lift`/`lower` [1]. Differential fuzzing against a reference model of the contract suffices.
3. **(b) Frame preservation**, as stated in 3.2.2, decomposed into:
   * (i) *allocator lemma*: while frozen, `alloc` returns addresses `>= base` and never writes `< base`;
   * (ii) *RC lemma*: frozen-mode `retain`/`release` do not write `< base`;
   * (iii) *codegen lemma*: every store in callback-mode code is guarded;
   * (iv) *runtime lemma*: no runtime export called while frozen writes `< base` (queue paused);
   * (v) *import lemma*: no host import runs while frozen;
   * (vi) *COW lemma*: `oxi_is_unique` is false below `base` while frozen, and no in-place-mutation elision applies to possibly-old values;
   * (vii) *exit lemma*: the result copy-out raises the count of every frozen object referenced from the arena by exactly the number of copied references, and no arena address survives the reset.

   (i), (ii), (iv), (vi) and (vii) are about a few hundred lines of runtime code. (iii) is checkable per module by a verifier. (v) is a host flag.
4. **Purity and ECF (P3).** For a pure callback with immutable captures, any interleaving of callback calls with the host algorithm is equivalent to a callback-free execution (ECF [19]). This follows from the callback being a function of its inputs. The 8.T purity rule is the Joe-E rule (immutable inputs plus no authority means pure) [R].

Mechanisation, realistically:
* **Cheap and worth it:** a Lean 4 or TLA+ model of the frozen-region protocol (allocator plus RC plus barriers as a state machine), with the invariant model-checked or proved. One or two person-weeks [R estimate]. Pair it with a Rust property test that checks memory below the base is bit-identical across every callback in a fuzzing harness. That is the "as close to mathematically safe as possible" sweet spot.
* **Medium:** a verified post-codegen guard checker (iii), proved sound against a small Wasm semantics. A VeriWasm/VeriZero-style project [10][24]. Some person-months.
* **Not realistic for us now:** full Iris-style proof of the OxiScript type system plus RC runtime (RustBelt [R], Perceus' calculus [43] and RichWasm [23] show it takes a research group). Rely on Iris-Wasm and WasmCert for the Wasm layer and keep our own proofs at the protocol level.

### 3.3 Contracts at the boundary

* **Canonical ABI lift/lower** [1] (V): the reference for *first-order* contract checking. Each value is validated on lift (ranges, discriminants, UTF-8 and encodings, alignment, bounds, length limits) and copied into the destination via its `realloc`. Cost is linear in the value size. It guarantees *well-formedness*, not semantic properties (sortedness, consistency of a comparator).
* **Interface types → WIT** [R]: the earlier Interface Types proposal evolved into the Component Model. Same guarantees.
* **Racket contracts and blame** [37][38] (S): higher-order contracts *wrap* function values, so a callback contract is checked on each call, and blame names the party that broke it (Findler & Felleisen, ICFP 2002). Correct blame for dependent contracts was refined at POPL 2011. *Lesson:* our sync-callback contract is a higher-order contract. On a mismatch, the trap reason should carry blame (host, guest or binding) with a reason code, so BAD ACTOR findings and user diagnostics point at the right side.
* **Gradual typing boundaries** [39] (S): sound gradual typing in Typed Racket showed slowdowns of over 100x when wrapped higher-order values cross typed/untyped boundaries frequently (Takikawa et al., POPL 2016). JITs reduce but do not remove this (Bauman et al., OOPSLA 2017). *Lesson:* the *frequency* of boundary checks on callbacks is the cost driver. Check first-order data once per import (a′), check per callback call only scalars and results, and avoid wrapping in the hot path.
* **Session types** [R, 46]: Honda (CONCUR 1993) and Honda, Vasconcelos, Kubo (ESOP 1998) typed the *protocol* of a channel. The stepwise inversion protocol of (d) (`step(answer) -> Request`) is a two-party session. A typed handle state machine in the IDL (`handle Sorter` with states) would let 8.T reject protocol misuse statically. The host still validates dynamically, because the guest is untrusted.
* **Typed holes** [R]: a program-construction concept (Hazel). Not relevant to boundary validation beyond error recovery in the editor.
* **What contracts cannot guarantee:**
  * Semantic laws of a callback, such as a comparator being a total order, cannot be checked per call. They can be checked by sampling, as Rust 1.81's sort detects violations "in many cases, not guaranteed" [30], or tested with OxiScript `law`s (`contracts.md` 3 "Contracts / laws / SMT").
  * Resource bounds are enforced by fuel and size limits, not by the contract.
  * Effects are enforced by 8.T plus the runtime flag, not by data validation.

### 3.4 C++ bindings

| Tool | Model | What breaks for us |
|---|---|---|
| **cxx** [31][32][33] (V) | Safe subset both ways, static checks, zero-copy | Exceptions: only caught when the signature returns `Result`, otherwise `std::terminate`. Panics across → abort. No C++→Rust function pointers yet. Templates only via concrete instantiation. The host C++ is not memory-safe: a C++ bug is a host compromise. |
| **autocxx** [R] | bindgen + cxx, generated from headers | Same as cxx, plus incomplete coverage of overloads and templates; `unsafe` at every call by default. |
| **bindgen** [R] | Raw `extern "C"` from headers | Everything `unsafe`; no RAII, no exceptions (UB if one unwinds into Rust), C++ ABI instability across compilers and standard-library versions. |
| **SWIG** [R] | Multi-language wrapper generator | Generated glue for Python, Java and others, not Rust-native. Exceptions via `%exception` typemaps. Same host unsafety. |
| **pybind11 / nanobind** [R] | C++ templates generating Python bindings | Translate exceptions, handle overloads and templates through C++ itself, keep objects alive (`keep_alive`). Excellent ergonomics, but C++ to Python only; nothing reusable for a Wasm guest except the design (exception translation, lifetime policies). |
| **Diplomat** [35] (S) | Rust-defined API exposed *to* C/C++/JS/Dart etc. | The opposite direction (Rust library to C++ callers). Useful if OxiTerm wants to export OxiScript-facing APIs to C++ hosts, not for binding C++ libraries. |

**What breaks in general:**
* *Exceptions*: must be caught at a `noexcept` C shim and turned into status codes, otherwise UB or abort.
* *Templates and overloading*: need a concrete facade, the same as Rust generics in 8.N.
* *RAII/destructors*: handles must own C++ objects with an explicit drop, and the generational handle table already gives use-after-free detection on the guest side.
* *Lifetimes*: C++ references and iterators have no checked lifetimes, so any borrow into C++ memory that outlives a call is unsound.
* *ABI instability*: libstdc++ versus libc++, and compiler flags.
* *No memory safety on the host side at all*: a C++ library bound natively is in the TCB, unlike Rust, where the compiler gives most of the guarantee.

**How native C/C++ is sandboxed in practice:**
* **RLBox + wasm2c** [7][8][9]: compile the C/C++ library to Wasm (wasi-sdk/clang), translate to C with wasm2c, link into the host, and wrap every interaction in tainted types. Firefox ships this for Graphite, Hunspell, Ogg, Expat and Woff2 [S, 9]. Sync callbacks *into* the host are allowed, and that host code must handle tainted arguments.
* **Compile the C++ library to Wasm as an OxiScript-side module**: wasi-sdk or Emscripten, with exceptions disabled or using the Wasm EH proposal. Wasm EH is not allowed by ABI 1.0, so build with `-fno-exceptions` or Emscripten's JS-based EH, which is not available in wasmi [R].
* **Process isolation**: a separate process plus IPC. That is the seL4/Chrome model: robust and slow (μs per call), and impossible on ESP32.
* **CHERI**: hardware capabilities give fine-grained memory safety for C/C++ (CHERIoT on microcontrollers, Morello/CheriBSD). Not available on our targets.

**Recommended path and safety level:**
1. *Preferred*: compile C/C++ to Wasm and run it as a **separate pinned instance** (option (a′) with the C++ module as the sibling) behind an `oxi-bindgen`-generated IDL facade. P1 holds by Wasm isolation, so a C++ bug corrupts only its own instance (RLBox's guarantee). Callbacks from the C++ instance into OxiScript go through the same `sync callback` contract. Works on server, browser and ESP32 (via wasm2c) alike.
2. *Fast but trusted*: cxx/autocxx native bindings for a small set of audited, widely fuzzed libraries, with a `noexcept` shim and the manifest marked `native-unsafe` (a distinct, louder mark than Rust `native`). Server only.
3. *Never*: raw bindgen or SWIG bindings exposed to scripts without a facade.

---

## 4. Concrete proposal for 8.N

### 4.1 IDL changes (ABI 1.1, all additive)

```
interface Sort "host:sort" @1 {
    // Tier 1: inversion, ABI 1.0 semantics (no re-entrancy)
    fn sort_keys_u64(keys: list<u64>) -> list<u32>;          // returns a permutation
    fn sort_keys_str(keys: list<str>) -> list<u32>;
    handle Sorter;                                            // stepwise protocol
    fn sorter_new(n: u32) -> Sorter;
    fn sorter_step(s: Sorter, answer: i8) -> step<u32>;       // Compare(i,j) | Done(perm)
    // Tier 2: true sync callback, opt-in
    sync callback Cmp(i: u32, j: u32) -> i8
        contract { result in [-1, 1]; fuel <= 10_000; pure; }
    fn sort_by_index(n: u32, cmp: Cmp) -> list<u32> len n
        native_effects { panics: never; calls: <= 2 * n * log2(n) + n; }
}
```

* `sync callback` is a new kind. It is a parameter type only (like `callback`), but **borrowed for the duration of the import**: not consumed, not retained by the host, invalid afterwards (the wasm-bindgen stack-closure rule [36]). The IR view is a borrowed `pure fn(...)`. It differs from today's consumed `fn` (`abi.md` 5.1).
* A `contract { ... }` block: parameter and result types (scalars, `str`/`bytes` with explicit `max_len`, records and lists from proposal (a) of 8.N with `max_items`), value ranges, a per-call fuel cap, `pure` (mandatory in ABI 1.1), and optional `max_calls` as a function of the outer arguments (an eBPF-style bound [16]).
* `native_effects { ... }` on the binding: declared and trusted, listed in the placement report. `panics: never|caught|abort` decides per-target availability (3.2.3).
* Trampoline export `scb:<module>@<version>.<Interface>.<Callback>(closure, flattened params) -> result` (the new prefix `scb:` is reserved like `cb:`).
* Records and lists in the IDL follow the canonical ABI's layout and validation rules [1], so a WIT adapter stays a generator.
* **No out-buffer retry for callback-taking functions.** `abi.md` 5.2 allows a `str`/`bytes`/list result to return status 1 ("buffer too small"), after which the guest calls again. For `sort_by_index` that would repeat all ~130k callbacks: pure, so still correct, but double the cost and the fuel. Rule: a function with a `sync callback` parameter must either have a result size the guest can compute *before* the call (declared in the IDL, for example `-> list<u32> len n`, so the guest allocates exactly n×4 bytes and status 1 cannot occur), or return a handle from which the result is read afterwards. The IDL parser rejects any other shape.

### 4.2 What 8.T checks
1. A closure passed as a `sync callback` has a static type `pure fn`:
   * no capability values reachable;
   * no `global.set`, no `global.get` of mutable module state, no reads of module-level `let` (they are different in a sibling);
   * no `setfield`/`array.set`/`array.push` on values not allocated inside the closure;
   * no calls except to `pure` functions, and only to imports with an empty effect set (none in 1.1).
2. Captures are deeply immutable values: scalars, `str`, struct values (COW snapshot), arrays and records of those, and other `pure fn` closures. **No class instances**, because identity is lost in a sibling copy (3.2.2 (a) problem 2).
3. The closure value cannot escape: it is used only as the argument (borrowed), never stored.
4. Effect summaries (`#effects`, `contracts.md` 8.T "Owes") record `pure`. 8.L lowers pure closures with the `#callback_mode` attribute so 8.G can emit the frozen-region guards if (b) is enabled.
5. Lint: a `sync callback` passed to a binding with `calls ~ n log n` gets a `P` note suggesting key extraction or the guest std sort (placement report).

### 4.3 What the runtime (8.R) enforces
* **Both modes:**
  * a function taking a `sync callback` never returns status 1; the host checks the declared result size against the guest buffer *before* running the algorithm and traps on a mismatch, so no callback ever runs twice because of a retry;
  * the contract check on every value crossing (copy plus validate, like canonical `lift`);
  * `in_sync_cb` flag: any host import attempted while it is set traps (`may_leave` [1]);
  * depth limit (≤ 2);
  * fuel cap per call and a total drawn from the caller;
  * poison flag on the first failure, after which the library sees `Equal` or neutral values and the outer import traps on return;
  * `catch_unwind` at every binding entry (server, unwind builds); on abort-only targets bindings with `panics: caught` are refused at link time;
  * the host never holds guest-memory slices across a callback (generated glue copies outer `str`/`bytes` arguments first; token type in the bindgen API).
* **(a′) sibling mode (ship first):**
  * sibling pool per module, size = depth limit;
  * sibling created via `oxi_init_pure` (a new runtime export that runs only pure initialisers);
  * captures copied once per outer import through the sibling's scratch area, or through a guest-allocated buffer when larger (proposal (c) of 8.N);
  * the sibling is reset after the outer import (re-instantiate or memory-image restore);
  * no imports are linked into the sibling at all, so the `in_sync_cb` flag is belt and braces.
* **(b) frozen mode (behind a feature flag, after the gate):**
  * `oxi_frozen_base` stack;
  * arena allocator;
  * frozen-aware `oxi_retain`/`oxi_release`/`oxi_free_bytes`/`oxi_array_grow`;
  * deferred queue and candidate buffer paused;
  * `oxi_is_unique` false below the base;
  * result deep-copied out with real retains into the frozen region (no arena adoption), then arena reset;
  * a post-codegen verifier that checks the guards in every `#callback_mode` function before instantiation.

### 4.4 What BAD ACTOR attacks (in addition to the row in `oxi-ui-plan-history.md`)
1. *Outer-borrow UAF.* The callback tries, directly or via a checker bug (use an IR-level mutant that skips 8.T), to release, overwrite or `push` an object borrowed by an outer frame two levels up. Expected: trap (b) or no effect on the caller (a).
2. *Contract bypass:*
   * result out of range;
   * a string with invalid UTF-8 or an overlong length;
   * a record with a wrong discriminant;
   * `max_len`+1;
   * a negative or huge index;
   * an index that reads outside the guest array (the guest bounds check must trap).
3. *Captured-state mutation:* the callback mutates a COW struct capture, including one with rc 1 (must clone inside the callback and stay invisible outside; in frozen mode this relies on `oxi_is_unique` being false below the base and on the disabled in-place elision), reads a module-level `let` (8.T reject), or compares class identity (8.T reject).
4. *Inconsistent comparator* (random, non-transitive, NaN-based) against every sort binding: no panic reaches the host. The result is a permutation and is deterministic for a given input under the vendored algorithm.
5. *Re-entrancy escalation:* nested `sort_by` inside a comparator up to and beyond the depth limit; host import attempts from the callback; registering a `callback` (async) from inside a sync callback.
6. *Fuel games:* exhaust fuel mid-sort (poison path; the caller traps once; no leak: `oxi_live_objects` balanced after `oxi_deinit`); compare the fuel consumed across two host builds (P3).
7. *Memory growth:* the callback grows memory by many pages; the host must not read stale slices (an ASAN or Miri run of the binding glue).
8. *Scratch clobber:* the callback writes scratch while the outer import's data is still in scratch (must be impossible by the 8.L "scratch dead at import" rule).
9. *Sibling divergence (a):* a differential test that the same callback returns the same result run in place (a reference interpreter) and in the sibling, for all captures types.
10. *Panic on abort targets:* a link-time test that a `panics: caught` binding is refused for `wasm32` and ESP32 profiles.

### 4.5 Benchmarks for the gate
Measure on the server (x86-64), browser local mode (V8), and ESP32-S3 (wasm2c or WAMR, once the board exists):
1. **Raw crossing:** wasmi host→guest call with 2 `i32` arguments and an `i32` result; guest→host import call; the same in wasmtime as reference. Report ns per call (the missing number in this report).
2. **Sort 10k / 100k:**
   * (i) OxiScript std sort in the guest;
   * (ii) Tier 1 `sort_keys_*` (keys extracted in the guest);
   * (iii) Tier 1 stepwise `Sorter`;
   * (iv) Tier 2 `sort_by_index` in (a′);
   * (v) the same in (b) when available;
   * (vi) native Rust as the floor.

   Report time, fuel and crossings. Decision rule: ship (iv) only where it beats (i), and adopt (b) only if (v) beats (iv) by more than 2x on realistic comparators.
3. **regex `replace_all`:** closure version (Tier 2) versus `captures_all` plus a guest loop (Tier 1), on 1 MB of text with 10k matches.
4. **Sibling overhead:** acquire plus reset time and RAM per sibling, on the server and on ESP32.
5. **Determinism:** fuel totals equal across two Rust toolchain versions for vendored algorithms (expected equal) and for std `sort_by` (expected to differ, which documents why vendoring is needed).

---

## 5. Open questions for the owner

1. **Is Tier 0/1 first acceptable?** That means guest-std `sort`/`map`/`filter`, inversion APIs by default, and `sync callback` only for irreducible cases. It changes the 8.N emphasis from "callbacks everywhere" to "callbacks as a last resort".
2. **(a′) now, (b) later behind the gate?** Or (a′) only, accepting the copy cost permanently for a simpler proof?
3. **Vendored algorithms:** may bindings that take callbacks ship *our* deterministic algorithm (merge sort with a fixed comparison sequence) instead of calling Rust `std` directly? This is required for P3 (fuel determinism across hosts) and for panic-freedom on abort targets.
4. **Panic policy:** server built with `panic = "unwind"` and `catch_unwind`; abort-only targets refuse `panics: caught` bindings. Agreed?
5. **Captures:** forbid class instances in sync-callback captures (identity), or allow them read-only in (b) only, which would make (b) the only mode for such code?
6. **Mechanisation budget:** a Lean 4/TLA+ protocol model plus fuzzing (weeks) now. A verified guard checker for (b) (months) only if (b) ships. Agreed?
7. **C++:** accept "C/C++ only via Wasm (separate pinned instance)" as the rule, with native cxx bindings reserved for a short audited list (`native-unsafe` mark)?

---

## 6. References

1. WebAssembly Component Model, *Canonical ABI* (`design/mvp/CanonicalABI.md`, main, read 2026-11-25): `ComponentInstance.may_leave`, `num_lends`, `borrow_scope`, lifting validation. https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md (V)
2. component-model PR #705, "CABI: remove the may_enter flag/trap", merged 2026-08-28. https://github.com/WebAssembly/component-model/pull/705 (V)
3. Component Model *Concurrency Explainer*, sections "Reentrance", "Borrows", "Backpressure". https://github.com/WebAssembly/component-model/blob/main/design/mvp/Concurrency.md (V)
4. Component Model `Text.md`, "Component Invariants" (lockdown, run-to-completion). https://github.com/WebAssembly/component-model/blob/main/design/mvp/Text.md (V)
5. Wasmtime API, `Caller`. https://docs.wasmtime.dev/api/wasmtime/struct.Caller.html (V)
6. Bytecode Alliance Zulip archive: wasmtime issue #3319 "Optimize Func::call and its C API" (https://bytecodealliance.github.io/zulip-archive/stream/225357-git-wasmtime/topic/wasmtime.20.2F.20issue.20.233319.20Optimize.20.60Func.3A.3Acall.60.20and.20its.20C.20API.html) and "High Overhead for Wasm Import Call Despite AOT Compilation" (https://bytecodealliance.github.io/zulip-archive/stream/206238-general/topic/High.20Overhead.20for.20Wasm.20Import.20Call.20Despite.20AOT.20Compilation.html); benchmark source https://docs.rs/crate/wasmtime-cli/latest/source/benches/call.rs (S)
7. S. Narayan et al., "Retrofitting Fine Grain Isolation in the Firefox Renderer", USENIX Security 2020. https://arxiv.org/abs/2003.00572 (V, callback section read) and https://www.usenix.org/conference/usenixsecurity20/presentation/narayan
8. RLBox documentation. https://rlbox.dev/ (V)
9. Mozilla, "WebAssembly and Back Again: Fine-Grained Sandboxing in Firefox 95", 2021. https://hacks.mozilla.org/2021/12/webassembly-and-back-again-fine-grained-sandboxing-in-firefox-95/ (S)
10. M. Kolosick et al., "Isolation without Taxation: Near-Zero-Cost Transitions for WebAssembly and SFI", POPL 2022. https://arxiv.org/abs/2105.00033 (V abstract and introduction)
11. CHERIoT: "Avoiding stack overflows on CHERIoT" (https://cheriot.org/rtos/stack/programming/2024/05/01/stack-usage.html) and *The RTOS Core* (https://cheriot.org/book/top-core_rtos-top.html) (S)
12. Erlang/OTP, `erl_nif` manual. https://www.erlang.org/doc/apps/erts/erl_nif.html (V)
13. Lua 5.4 Reference Manual, section 4.5 "Handling Yields in C". https://www.lua.org/manual/5.4/manual.html (V)
14. PyO3 user guide, "Python object types". https://pyo3.rs/main/types (V)
15. V8 `include/v8-fast-api-calls.h`. https://chromium.googlesource.com/v8/v8/+/refs/heads/main/include/v8-fast-api-calls.h (V)
16. eBPF docs: `bpf_loop` (https://docs.ebpf.io/linux/helper-function/bpf_loop/), loops (https://docs.ebpf.io/linux/concepts/loops/); LWN (https://lwn.net/Articles/877062/) (S)
17. Solidity docs, "Security Considerations" (re-entrancy; Checks-Effects-Interactions). https://docs.soliditylang.org/en/latest/security-considerations.html (V)
18. OpenZeppelin Contracts 5.x, `ReentrancyGuard` / `ReentrancyGuardTransient`. https://docs.openzeppelin.com/contracts/5.x/api/utils (S)
19. S. Grossman et al., "Online Detection of Effectively Callback Free Objects with Applications to Smart Contracts", POPL 2018. https://arxiv.org/abs/1801.04032 (S)
20. S. Blackshear et al., "Resources: A Safe Language Abstraction for Money" (https://arxiv.org/abs/2004.05106); "Robust Safety for Move" (https://arxiv.org/abs/2110.05043) (S)
21. X. Rao et al., "Iris-Wasm: Robust and Modular Verification of WebAssembly Programs", PLDI 2023. https://www.cl.cam.ac.uk/~jp622/iris-wasm.pdf (S)
22. C. Watt et al., "Two Mechanisations of WebAssembly 1.0", FM 2021; C. Watt, "Mechanising and Verifying the WebAssembly Specification", CPP 2018 (WasmCert). https://github.com/WasmCert (R)
23. M. Fitzgibbons et al., "RichWasm: Bringing Safe, Fine-Grained, Shared-Memory Interoperability Down to WebAssembly", PLDI 2024. https://arxiv.org/abs/2401.08287 (S)
24. E. Johnson et al., "Доверя́й, но проверя́й: SFI safety for native-compiled Wasm" (VeriWasm), NDSS 2021. https://github.com/PLSysSec/veriwasm (S)
25. seL4 performance page (https://sel4.systems/performance.html); K. Elphinstone, G. Heiser, "From L3 to seL4: What Have We Learnt in 20 Years of L4 Microkernels?", SOSP 2013 (https://www.cs.yale.edu/flint/cs428/doc/L3toseL4.pdf) (S)
26. wasmi API docs (crate root, `Caller`, `TypedFunc::call_resumable`, `ResumableCall`). https://docs.rs/wasmi/latest/wasmi/ (V)
27. wasmi v0.32 blog (https://wasmi-labs.github.io/blog/posts/wasmi-v0.32), releases/CHANGELOG (https://github.com/wasmi-labs/wasmi/releases) (S)
28. (number unused)
29. (number unused)
30. Rust 1.81.0 release announcement (new sort implementations may panic on non-total orders). https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/ (S); see also https://github.com/rust-lang/rust/issues/130122
31. cxx overview. https://cxx.rs/ (V)
32. cxx, `Result` / exceptions. https://cxx.rs/binding/result.html (V)
33. cxx, `rust::Fn`. https://cxx.rs/binding/fn.html (V)
34. UniFFI, callback interfaces. https://mozilla.github.io/uniffi-rs/latest/types/callback_interfaces.html (V)
35. Diplomat: `DiplomatCallback` (https://doc.servo.org/diplomat_runtime/struct.DiplomatCallback.html), lifetimes (https://rust-diplomat.github.io/book/lifetimes.html), `diplomat_core` methods source (https://docs.rs/crate/diplomat_core/0.15.0/source/src/hir/methods.rs) (S)
36. wasm-bindgen guide, "Passing Rust Closures to Imported JavaScript Functions". https://rustwasm.github.io/docs/wasm-bindgen/reference/passing-rust-closures-to-js.html (V)
37. R. B. Findler, M. Felleisen, "Contracts for Higher-Order Functions", ICFP 2002. https://users.cs.northwestern.edu/~robby/pubs/papers/ho-contracts-techreport.pdf (S)
38. C. Dimoulas et al., "Correct Blame for Contracts", POPL 2011. https://www.cs.northwestern.edu/~robby/pubs/papers/popl2011-dfff.pdf (S)
39. A. Takikawa et al., "Is Sound Gradual Typing Dead?", POPL 2016 (https://popl16.sigplan.org/details/POPL-2016-papers/19/Is-Sound-Gradual-Typing-Dead-); S. Bauman et al., "Sound Gradual Typing: Only Mostly Dead", OOPSLA 2017 (S)
40. M. Finifter, A. Mettler, N. Sastry, D. Wagner, "Verifiable Functional Purity in Java", CCS 2008 (Joe-E) (R)
41. M. S. Miller, *Robust Composition* (E, vats, plan interference), PhD thesis, JHU 2006 (R)
42. R. Jung et al., "RustBelt: Securing the Foundations of the Rust Programming Language", POPL 2018 (R)
43. A. Reinking, N. Xie, L. de Moura, D. Leijen, "Perceus: Garbage Free Reference Counting with Reuse", PLDI 2021. https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse-2/ (S)
44. E. Ovadia, Vale regions: "Zero-cost borrowing with regions" overview (https://vale.dev/blog/zero-cost-borrowing-regions-overview); guide (https://vale.dev/guide/regions) (S; marked "planned" by the author)
45. S. Ullrich, L. de Moura, "Counting Immutable Beans: Reference Counting Optimized for Purely Functional Programming", IFL 2019. https://arxiv.org/abs/1908.05647 (S)
46. K. Honda, "Types for Dyadic Interaction", CONCUR 1993; K. Honda, V. Vasconcelos, M. Kubo, "Language Primitives and Type Discipline for Structured Communication-Based Programming", ESOP 1998 (R)
47. D. Grossman et al., "Region-Based Memory Management in Cyclone", PLDI 2002 (R)

Not verified in this session and marked R in the text: JNI local-reference rules, Deno ops internals, wit-bindgen callback shape, autocxx/SWIG/pybind11/nanobind details, wasmtime memory-image pooling, `corosensei` portability, the default `panic` strategy on ESP-IDF Rust, and all latency estimates for wasmi.
