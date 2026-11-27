# OxiScript async specification (stage 8.4a)

Status: design specification for owner review, 2026-11-27. Stage [8.4a](../roadmap/oxiscript.md#s-8-4a). No product code; the paper lowerings in section 6.4 were parsed and verified with the existing `oxiscript-ir` parser and verifier (unchanged crate, scratch harness outside the repository).
Author: Claude Sonnet with Opus advisor review (takes over a task a Gemini agent could not start).

**Verification legend** (as in `oxiscript-sync-callbacks.md`): **V** primary source read in this session; **S** seen in search results or a failed/partial fetch; **R** recalled from prior knowledge, not re-checked this session; **P** my estimate, to be measured; **O** opinion. Repository facts were read from the files named.

Normative language: rules are numbered `A-n`. "Must", "must not" bind implementations; the rest is rationale.

---

## 1. Executive summary

**The question.** How do OxiScript programs wait (timers, host events, network and file results from capabilities, later isolates and GPU read-back) without breaking P-6 (capabilities), P-7 (Core Wasm only, no proposals wasmi lacks), P-8 (hot reload by instance swap), P-9 (determinism), D-1 (reference counting) and the `micro` size budget, and without forcing a TypeScript developer to learn a new vocabulary.

**Recommendation in ten lines.**

1. Keep the host ABI exactly as it is: the host never re-enters a running guest and delivers events only between guest turns (`abi.md` 5.3). Everything below is language and runtime-support code *on top of* callbacks; no import or export signature changes, only one new constant (`MAX_POLLS_PER_TURN`) and a tick contract.
2. Add TypeScript-looking `async function` / `await` (the keywords are already reserved, `FnModifiers::is_async` is already in the AST). The declared return type is the plain result type (`async function f(): Result<i32, string>`), not `Promise<T>`.
3. Compile async functions in the compiler to **heap state-machine frames**: a `start` function that allocates the frame and a `poll` function that is a `switch` on a state number. No stack switching, no Wasm proposal, no new typedesc kind; a frame is an ordinary IR `struct` and `poll` an ordinary function (verified in section 6.4).
4. **Colouring is explicit but cheap:** the `async` keyword is required, the checker still computes the effect summary (profiles, report) and every "await outside async" error carries a machine-applicable fix. This deliberately replaces the roadmap's word "inferred effect" (open question Q-ASYNC-1).
5. **No floating promises.** Calling an async function without `await` yields a move-only `Pending<T>` that can only be awaited or handed to `scope.spawn`. `Task<T>` handles are move-only too.
6. **Structured concurrency:** every task belongs to a scope; the instance is the root scope; leaving a scope cancels and joins children that were not awaited (reported by the checker); an unhandled child failure is impossible because errors are `Result` values.
7. **Cancellation is cooperative, happens only at `await` points, and is not an error value:** a cancelled task is never resumed, its `defer` blocks run innermost first, then its frames are released by reference counting. Timeouts are a library function built from a scope, a `Oneshot` and a timer capability.
8. **Scheduling is single-threaded, FIFO, and bounded per turn** by a fixed poll budget; time and I/O arrive only as host deliveries, so a recorded delivery log replays a program exactly (P-9).
9. **Hot reload cancels every task of the old instance** (no frame migration); durable work lives in host state and is restarted idempotently by the new `main`. The reload is still atomic accept-or-reject.
10. Threads and isolates (8.4), GPU read-back (8.3) and OxiUI handlers (8.S1) all reuse the one mechanism: a completion resolves a `Oneshot` and wakes a task id. Not in v1: `Promise` objects, `for await`/generators, `Promise.race` over arbitrary values, async iterators in std, async `drop`, shared-memory concurrency, top-level await.

**Open questions for the owner** are in section 13 (five, each with a recommendation).

---

## 2. What the repository fixes today (constraints, all read in this session)

| Fact | Source |
|---|---|
| `async await spawn isolate yield` are reserved words (`OXS-E0900`); `FnModifiers::is_async` and an `async function` slot exist in the AST; `defer`, `scope`, `Task`, `Scope` are **not** reserved | `grammar.md` 2.1 and 5; `FnDecl` grammar |
| No exceptions: `Result<T, E>`, `e?` returns the same `E`; panics are traps that poison the instance | `AGENTS.md` rule 4; `abi.md` 3.4 |
| Capabilities only through `main`; host calls are synchronous imports returning `Result<T, i32>` | `AGENTS.md` rule 7; `abi.md` 5 |
| **No re-entrancy:** while an import runs the host must not call into the instance; host-to-guest callbacks happen only when the guest is not running; releases are queued to a quiescent point | `abi.md` 5.3 |
| Callbacks: `cb:<module>@<ver>.<Interface>.<Callback>(closure, ...)`, closure = `{table_index, reserved, captures}`, at most one `str`/`bytes` parameter of at most 4096 bytes via the scratch region | `abi.md` 5.4, 2.x |
| `oxi_tick(now_ms) -> i64` exists: today a stub that calls a user `tick` function if present and otherwise returns -1; the host owns the clock | `abi.md` 3; `oxiscript-wasm/src/wrap.rs` (the tick wrapper) |
| Fuel is reset at every guest entry (init, callback, export, tick, release); callbacks per `pump` are bounded (`max_events_per_pump`); events raised while pumping wait for the next pump | `oxiscript-runtime/src/limits.rs`, `runtime.rs` (`pump`) |
| After a trap the instance is only discarded; recovery = instance swap | `abi.md` 3.4 |
| Typedesc kinds: 1 struct, 2 union variant, 3 closure, 4 string, 5 array; flag `acyclic`; pointer map per type | `abi.md` 2.1 |
| IR: monomorphised, SSA with block parameters, explicit retain/release, no raw pointers in untrusted IR, interfaces as values are gated (no dynamic dispatch yet), `Terminator` is `#[non_exhaustive]` and `Terminator::Await` is only *planned* in `ir.md` section 7 (the roadmap sentence "the IR keeps its `Terminator::Await` slot" means exactly this: no variant exists in the code) | `ir.md` 1, 3, 7; grep of `oxiscript-ir/src` |
| Wasm features: Core Wasm only (MVP + multi-value + bulk-memory + sign-extension); no GC/threads/EH/reference-types; `micro` = w2c2 native firmware, no fuel, compiler budget checks | `abi.md` 3.6; D-18, D-27, D-28 |
| Hot reload: new instance, atomic swap, module-local state resets, host-managed `state` survives; old instance's registered callbacks are dropped by the host | `abi.md` 4; P-8, D-9 |
| 8.3 designs GPU read-back as `request_download(..., cb: OnReadbackReady)` and says "`await` sugar once 8.4 async lands" | `oxiscript-simd-gpu-design.md` 4.4 |

---

## 3. Prior art and lessons

### 3.1 Table

| System | What it does | Lesson for OxiScript | Source |
|---|---|---|---|
| **JS promises, `async/await`** | Promise = eager, shared, multi-consumer value; `await` suspends the function; the event loop runs microtasks | What TS developers expect: `async function`, `await`, sequential-looking code. Costs we avoid: floating promises, unhandled rejections (we have no exceptions), cancellation bolted on later (`AbortSignal`), `Promise<Result<..>>` noise | MDN [R] [1][2][3] |
| **"What color is your function"** | Async/sync split splits the whole ecosystem into two worlds; sync cannot call async | Colouring is real. Mitigations that fit: few entry points that bridge (`main`, `@host async`, `scope`), async closures accepted where a callback is expected, one `async` keyword with a fix-it, no second parallel std | Nystrom 2015 [R] [4] |
| **Rust futures** | Compiler turns `async fn` into a state-machine type; executor polls; `Pin` for self-referential frames; cancellation = drop | The state machine is the right lowering on Core Wasm. Rust's pain points are mostly absent here: OxiScript has no borrows across suspension (every value in a frame is owned and reference counted), so no `Pin`, no lifetime puzzles, no `Send` bounds in one isolate. Keep: drop-based cancellation, must-use. Avoid: executor choice as an ecosystem split, cancel-safety folklore (our rule: cancellation only at `await`, defers run) | Async book, Pin, tokio `select!` [R] [5][6][7] |
| **Kotlin coroutines** | `suspend` functions compiled to state machines (CPS); structured concurrency (`coroutineScope`); cancellation is cooperative, thrown as `CancellationException`, checked at suspension points; `withTimeout` | The closest match to our recommendation: scopes, cooperative cancellation at suspension points, timeouts as library. We differ on the cancellation signal: no exceptions, so cancelled code is simply not resumed and `defer` replaces `finally` | Kotlin docs [R] [8][9] |
| **Swift** | `async/await`, structured concurrency (`async let`, task groups), actors; cancellation is a flag checked cooperatively; Swift 6 strict-concurrency migration was painful | Decide the model before the ecosystem exists (the reason 8.4a exists). Cancellation as cooperative flag is weaker than ours (we need no polling at all). Actors map to isolates (8.4), not to this spec | SE-0304 [R] [10]; migration anecdotes via `oxiscript-language.md` section on Swift [S] |
| **Go goroutines** | Cheap stackful threads, channels, `context.Context` for cancellation | Needs growable stacks / a scheduler with stack switching: not available on Core Wasm (P-7). Takeaway: explicit cancellation tokens work but need discipline; our scopes make it structural | Go blog [R] [11] |
| **Zig** | `async/await` removed in 0.11 (self-hosted compiler could not bootstrap with it), being redesigned around an `Io` interface passed explicitly | Async is cross-cutting: it must be decided before the compiler/IR hardens. The `Io`-as-parameter idea is exactly our capabilities-through-`main` | LWN, zig commit [S] [12][13] |
| **Erlang / Elixir** | Lightweight processes with private heaps, message passing, supervision; preemptive per-process scheduling by reduction count | Isolates (8.4) are this model; reduction counting = our fuel/poll budget. Not for this spec: processes need a preemptive VM scheduler | Erlang docs [R] [14] |
| **Lua coroutines** | Stackful, asymmetric, `yield` across Lua frames; C frames need continuations | Generators (option d) are small but stackful variants need a runtime stack we do not have; stackless generators equal our state machines | Lua manual [R] [15] |
| **Effect handlers (OCaml 5, Koka)** | One-shot continuations as a general mechanism for async, generators, schedulers | The most general answer, but needs runtime stack management; Core Wasm cannot do it without stack switching. Keep as the long-term vocabulary for the compiler pass (our state machine *is* a one-shot continuation, selective CPS), not as a surface feature | OCaml manual, Koka book [R] [16][17] |
| **Wasm stack switching** | Continuations (`cont`, `suspend`, `resume`, `switch`), one-shot, built on function references + EH | Proposal is **Phase 3** and our P-7 forbids relying on it: wasmi's feature table lists tail calls, multi-memory, memory64 as supported and threads, exception handling, GC as tracking issues only, stack switching not listed; w2c2 documents Wasm 2.0 + threads and does not mention exceptions, tail calls or stack switching. Revisit as an optional backend optimisation later; the language semantics must not depend on it | [V] [18][19][20][21] |
| **JSPI** | Browser engines let Wasm call imports that return Promises and suspend the whole Wasm stack | Phase 5 per the proposals page [V] but browser-only; useful at most for the `host:dom` bridge (8.Dom), never for language semantics. The page fetch failed, details S | [V/S] [19][22] |
| **WASI 0.3 / Component Model async** | `stream<T>` and `future<T>` with native async in the canonical ABI, released 2026-06-11 per the repository research | A future, optional adapter target for async IDL functions (P-6 says the Component Model can be an adapter later). Our host-op shape (start + completion, cancel) maps onto it | `oxiscript-language.md` (S) [23] |
| **Trio nurseries** | Structured concurrency: children cannot outlive the block | Origin of the "children cannot outlive the scope" rule, adapted here to cancel-then-join (A-13) | Smith 2018 [R] [24] |

### 3.2 Lessons distilled

1. **Decide early, keep the ecosystem single-coloured.** Zig, Swift, Kotlin/Native and Rust all paid for late concurrency. One std, one `async`.
2. **State machines are the only portable lowering** on Core Wasm without proposals; Kotlin and Rust prove they perform.
3. **Structured concurrency removes the hardest bugs** (leaks, floating tasks, lost errors) and fits a no-exceptions language: every error is a returned value, so an orphan task could only lose it.
4. **Cancellation is the design centre**, not an afterthought: point (where), signal (how observed), cleanup (what runs), reload (what happens to in-flight work).
5. **Value semantics make frames simple:** OxiScript has no references into other objects that outlive a statement, so a frame owns copies or counted references and never self-references; no `Pin`.
6. **Determinism comes from the driver, not the language:** a single-threaded FIFO scheduler plus host deliveries as the only source of time and I/O.

---

## 4. Options on Core Wasm without stack switching

Criteria: TS readability, determinism, fuel, RC and cycles, hot reload, cancellation, `Result`/`?`, capabilities, `micro` size.

| | (a) `async/await` state machines | (b) callbacks / events only (today) | (c) explicit tasks + structured concurrency (no sugar) | (d) generator coroutines (`yield`) |
|---|---|---|---|---|
| **TS readability** | Best: same keywords and shape | Poor for sequences: "callback pyramid"; every step is a closure and a `Result` plumbing | Medium: `scope.spawn(...)`, `join`, but sequencing still callbacks | Unfamiliar (`function*`), reserved `yield` |
| **Determinism** | Same as the driver: FIFO, host deliveries | Same | Same | Same |
| **Fuel** | Per poll; one turn = one guest entry; budget on polls | Per callback | Per poll | Per resume |
| **RC / memory** | Frame = counted heap object; captured state visible and bounded; cycles via waker ids (section 6) | Closures capture; cycles through host registrations (the host holds the closure) | Same as (a) | Same as (a) |
| **Hot reload** | Cancel all tasks (frames are code-version-specific) | Host drops callbacks (already specified) | Cancel | Cancel |
| **Cancellation** | Natural at `await`; needs `defer` for cleanup | Each API needs its own cancel handle; easy to forget | Natural | Caller simply stops resuming |
| **`Result` and `?`** | `await f()?` works inside async functions | `?` unusable across callback boundary (callback returns `void`) | `?` inside spawned bodies only | `?` inside generator body only |
| **Capabilities** | Awaiting a host op = callback + `Oneshot` (no new authority) | Native | Same as (a) | Same |
| **`micro` code size** | Frame type + `poll` switch per async function; ~30-60 B per await site, scheduler ~1-1.5 KB [P] | Smallest | Scheduler + task cells [P] | Like (a) without scheduler |
| **Verdict** | **Surface of choice** | **Stays the host interface** (an implementation layer under (a)) | **Kept as the library layer** `Scope`/`Task` (needed by (a) for concurrency and by 8.4) | **Deferred**: shares the (a) lowering; add later as `Stream<T>`/`for await` once (a) is stable |

Rejected as the foundation: **engine-specific suspension** (Wasmtime async fibers or any engine's resumable calls) because P-7 requires the same semantics on wasmi, w2c2 and browsers; **stackful threads** (Go-style) for the same reason; **effect handlers as a surface feature** (cost/complexity, no Core Wasm lowering without stack switching).

**Conclusion.** (a) + (c) + (b): the compiler lowers async code to state machines over a small scheduler library (c) that is driven by callbacks and ticks (b). This is the recommendation of `oxiscript-language.md` F.5 made concrete.

---

## 5. Recommended surface

All examples are **proposed syntax** (not accepted by today's checker). Capability interfaces other than `host:log` and `host:state` are proposed IDL; their text is given where used.

### 5.1 Declarations and the colour rule

```ts
// proposed syntax
async function fetchTitle(http: Http, url: string): Result<string, i32> {
  const body = await http.get(url)?;
  return Ok(titleOf(body));
}
```

* **A-1.** `async function f(P): R` declares a function that may suspend. `R` is the *result type*; the call's type is `Pending<R>` (A-8). `Promise` does not exist. Rationale: `Promise<Result<T, E>>` doubles the wrapper, the promise features (eager, shared, multiple consumers) are not offered, and `?` needs `R` to be the `Result`.
* **A-2.** `async` methods are allowed on `class` and `struct`, in interface declarations (satisfaction is structural and includes the `async` modifier) and as `async (P) => R` arrow functions. An async function type is written `async (P) => R`. Dynamic dispatch through interfaces follows 8.L's gate for interfaces as values; until then async interface methods are usable as generic bounds only.
* **A-3.** `await e` is allowed only inside an async function or async arrow; elsewhere it is `OXS-E-AWAIT-OUTSIDE-ASYNC` with a `machine-applicable` fix that adds `async` to the enclosing function when that function is not exported through `@host`/`main` with an incompatible signature, and otherwise a note pointing to `scope`/`@host async`. (Reason-code numbers are allocated append-only when each slice lands; symbolic names here.)
* **A-4.** The checker computes an **async effect summary** per function (may suspend: yes/no, transitively), stored beside the alloc/panic/loop summaries (`8.2c`), used by the `no_async` profile restriction and the placement report. An async function that never awaits is allowed but reported (`OXS-W-ASYNC-NO-AWAIT`, fix: drop `async`).
* **A-5.** `async` is *explicit* in v1; Q-ASYNC-1 asks the owner whether to infer it instead.
* **A-6.** Sync code calls async code in exactly three ways: (i) `main` or a `@host` export declared `async` (A-30, A-31), (ii) `scope(...)` inside an async function, (iii) an async closure passed where a `void` callback is expected (A-32). There is no blocking wait, ever (a blocking wait would break A-23 and the host loop).

### 5.2 `await`, `?` and what an async call is

* **A-7.** `await` is a prefix operator with the precedence of unary operators. **`await e?`** means `(await e)?` for any operand `e` (a call, or a `Task` variable as in `await ta?`): a trailing `?` (no space, per the whitespace rule of `AGENTS.md` item 9) applies to the awaited result. This is unambiguous because neither `Pending<T>` nor `Task<T>` is ever a `Result`, so `?` on them would be a type error anyway. `(await f())?` stays legal; `await (f()?)` is the explicit other reading.
* **A-8.** A call to an async function does **not** run it. It evaluates the arguments (in order, per the written evaluation order), allocates the callee's frame (`start`, section 6) and yields a value of type `Pending<R>`. `await` starts and drives it to completion *in the awaiting task*; `scope.spawn` gives it its own task.
* **A-9.** `Pending<R>` and `Task<R>` are **single-consumer** (move-only) types. The checker enforces, by simple dataflow over the function body: a value must be consumed exactly once on every path (`await`, `spawn` for `Pending`; `await` or `cancel()` for `Task`); it may be passed as an argument to an async function's `Pending` parameter, but may not be stored in a field, an array, a module-level `let`, or captured by a non-async closure. Violations: `OXS-E-PENDING-UNUSED`, `-REUSED`, `-ESCAPES`. A `Task` still live at scope exit is cancelled and reported (A-13, A-15).
* **A-10.** There are no top-level awaits. `main` may be `async` (A-30).
* **A-11.** Inside an async function, ordinary sequential semantics hold: statements run in order, a suspension happens only at an `await`, the evaluation order of `await e` is: evaluate `e`, then suspend as needed, then resume with the value.

```ts
// proposed syntax: a loop with a timer between steps; ordinary control flow, Result and ?
async function poll(timer: Timer, log: Log, http: Http): Result<i32, i32> {
  let seen: i32 = 0;
  for (const attempt of range(0, 5)) {
    const n = await http.count("/jobs")?;
    if (n > 0) { return Ok(n); }
    seen = seen + 1;
    await sleep(timer, 250u32)?;
  }
  log.write(3, `gave up after ${seen} polls`);
  return Err(-1);
}
```

### 5.3 Scopes and tasks (structured concurrency)

```ts
// proposed syntax
async function loadBoth(http: Http, a: string, b: string): Result<string, i32> {
  return await scope(async (s: Scope): Result<string, i32> => {
    const ta = s.spawn(fetchTitle(http, a));   // a Pending, consumed by spawn: starts later, in this scope
    const tb = s.spawn(fetchTitle(http, b));
    const x = await ta?;                        // Task<Result<string, i32>> awaited, then `?`
    const y = await tb?;                        // if x was an Err we returned: tb is cancelled by the scope
    return Ok(`${x} / ${y}`);
  });
}
```

* **A-12.** `scope(body)` is an intrinsic of `std/async`: `body` is an async arrow taking a `Scope`; the call is an async call (it is awaited). The compiler **inlines a scope whose argument is a literal async arrow** into the caller's frame (no closure allocation); a non-literal `body` is an ordinary async closure call.
* **A-13.** `s.spawn(p)` takes a `Pending<R>` (A-8, A-9), enqueues a new task owned by `s`, and returns `Task<R>`. `spawn` never runs user code: the child first runs when the spawning task suspends or ends its turn (A-24). **Scope exit rule:** whenever a scope exits (falls off its end, any `return`, including the early return of `?`), every child task that is still unconsumed is **cancelled** (defers run, innermost first, section 8) and joined before the scope returns. Waiting for a child is always an explicit `await`; abandoning it is always an explicit `t.cancel()`. Because `Task` is single-consumer (A-9), the checker knows at each exit point which handles are live and reports each as `OXS-W-TASK-UNCONSUMED` (an error under `safety`), so a silent drop is visible. Rationale: errors are values, so the scope cannot tell a failing exit from a successful one; cancel-by-default keeps exit latency bounded and makes the abandon visible, where join-by-default (Trio) would make an early `?` wait for slow siblings.
* **A-14.** A `Task<R>` can be cancelled by its owner only: `t.cancel()` consumes the handle (A-9), so a cancelled task can never be awaited. The only other cancellation source is the cancellation of an enclosing scope or task, and then the awaiter itself is cancelled (it is in the same subtree), so it never observes a missing result.
* **A-15.** A `Task` handle that is live at a scope exit is reported (A-13). A task whose `R` contains a `Result` and whose handle is cancelled is not reported (the abandon is explicit).
* **A-16.** A failing child never cancels its siblings automatically (there is no exception to propagate); library helpers (`all`, `withTimeout`) implement policies with `cancel`.
* **A-17.** The **root scope** of an instance owns every task started by `@host async` exports, async callbacks (A-32) and an `async main`. It is cancelled when the instance is deinitialised, reloaded or trapped (section 9).

Library surface of `std/async` (v1; all async, none blocks):

```ts
// proposed syntax: signatures only
export class Oneshot<T> {
  constructor();
  resolve(value: T): bool;        // false if already resolved or the waiter was cancelled
  async wait(): T;                // single waiter; waits until resolved
}
export async function sleep(timer: Timer, ms: u32): Result<void, i32>;
export async function yieldNow(): void;       // end this turn, run others first
export type Timed<T> = { kind: "done"; value: T } | { kind: "timedOut" };
export async function withTimeout<T>(timer: Timer, ms: u32, work: Pending<T>): Timed<T>;
export class Latest { constructor(); replace<T>(s: Scope, work: Pending<T>): void; } // cancels the previous task of the slot
```

`withTimeout` is ordinary library code (no compiler magic), shown here because it exercises every rule:

```ts
// proposed syntax
async function runThen<T>(work: Pending<T>, done: Oneshot<bool>): T {
  const v = await work;
  done.resolve(true);
  return v;
}
async function alarm(timer: Timer, ms: u32, done: Oneshot<bool>): void {
  await sleep(timer, ms);        // a failed sleep just ends the alarm
  done.resolve(false);
}

export async function withTimeout<T>(timer: Timer, ms: u32, work: Pending<T>): Timed<T> {
  return await scope(async (s: Scope): Timed<T> => {
    const done = new Oneshot<bool>();
    const job = s.spawn(runThen(work, done));
    const clock = s.spawn(alarm(timer, ms, done));
    if (await done.wait()) {
      clock.cancel();                                     // the alarm's host timer is cancelled by its defer (A-21)
      return { kind: "done", value: await job };
    }
    job.cancel(); await clock;                                       // work and everything it spawned is dropped, defers run
    return { kind: "timedOut" };
  });
}
```

### 5.4 Host operations: awaiting what a capability does

* **A-18.** A host operation that completes later is, at the ABI, exactly what exists today: a *start* import that takes a `callback` and returns a handle (`State.watch`, the proposed `request_download`), a *completion callback* delivered between turns, and (for results over 4096 bytes) a *pull* import using the out-buffer protocol (`abi.md` 5.2). **No new ABI.**
* **A-19.** The IDL gains `async fn` as a *declaration sugar*, processed by the IDL front end (`oxiscript-abi/idl.rs`) and `oxi-bindgen`: `async fn get(url: str) -> result<bytes>;` stands for the start import `get_begin(url, cb: OnDone_get) -> result<Op>`, the completion callback `OnDone_get(op: Op, status: i32, len: u32)`, `take_get(op: Op) -> result<bytes>` and `cancel_get(op: Op) -> result<void>`, plus a generated OxiScript wrapper (below). The host implementer writes only the begin/cancel/take functions; the contract is: **the completion callback fires exactly once or never (after a successful cancel); a late completion after cancel must be dropped by the host** (handles are generational and validated on every call, `abi.md` 5).

```
// proposed IDL
interface Http "host:http" @1 {
    handle Op;
    async fn get(url: str) -> result<bytes>;      // sugar, see A-19
}
interface Timer "host:timer" @1 {
    handle Id;
    callback OnFire(id: Id);
    fn after(ms: u32, cb: OnFire) -> result<Id>;
    fn cancel(id: Id) -> result<void>;
}
```

* **A-20.** The generated wrapper is ordinary `std/async`-based OxiScript:

```ts
// proposed syntax: what the generator writes for `async fn get` (bytes result elided as `u8[]`)
async function http_get(h: Http, url: string): Result<u8[], i32> {
  const done = new Oneshot<i32>();                       // status code
  const op = h.get_begin(url, (op, status, len) => { done.resolve(status); })?;
  defer { h.cancel_get(op); }                            // cancel path and normal path; the host ignores a finished op
  const status = await done.wait();
  if (status !== 0) { return Err(status); }
  return h.take_get(op);
}
```

* **A-21.** `defer { ... }` (A-27) is how cancellation reaches the host: a cancelled task awaiting `http_get` runs its `defer`, which cancels the host operation. This keeps cancellation a *library* contract and requires no ABI support.
* **A-22.** Timers are a capability (`host:timer`), not ambient: P-6 holds (a program without `Timer` cannot sleep). The host implements deadlines with its own clock (OxiUI's `next_deadline`); the guest `oxi_tick(now_ms)` remains for animation callbacks and as the driver of the run queue (A-25).

### 5.5 `defer`

* **A-27.** `defer { stmts }` registers a block that runs, innermost-last-registered first, when the enclosing *function* exits (by `return`, by falling off the end, by `?`) **or when the task is cancelled while suspended inside that function**. A `defer` body is synchronous: it may not `await` or `spawn` (`OXS-E-DEFER-SUSPENDS`), may call host imports, and must not `return`.
* **A-28.** `defer` is useful outside async code (it replaces `finally` in a language without exceptions) but is introduced by this spec with its first use. It must be added to the reserved list now as a contextual statement keyword (`defer` followed by `{`) so existing identifiers named `defer` keep working only until the slice lands: **reserve `defer` in 8.Fix (E0900) before the first public release** (one line, no breakage, nothing is public yet).
* **A-29.** `defer` does **not** run on reload or trap (section 9): the instance is discarded and the host reclaims its registrations. Programs must not rely on `defer` for anything the host cannot reclaim itself.

### 5.6 Entry points

* **A-30.** `export async function main(log: Log, ...): void` is allowed; `oxi_init` creates the root task and polls it once (the synchronous prefix runs inside `oxi_init`, like JS).
* **A-31.** `@host export async function onSave(): void` is allowed. The generated export has the ordinary ABI-flattened signature of the sync form (result `void`): it allocates the frame, creates a root-scope task and polls it once, then returns to the host. `@host` rules (numbers, `bool`, enums, at most one `string` or `u8[]`) apply to the *parameters*; the result is always `void`.
* **A-32.** An async arrow `async () => R` may be passed where a `fn(...)` with result `void` is expected (every IDL callback today): the compiler wraps it so that each invocation allocates the frame, creates a root-scope task and polls it once; the callback itself returns `void` immediately. A non-`void` callback result is rejected for async arrows.

```ts
// proposed syntax: an OxiUI page script (8.S1); `Ui` and `host:ui` are proposed
import { Ui } from "host:ui";
import { Http } from "host:http";
import { Timer } from "host:timer";
import { State } from "host:state";
import { Log } from "host:log";

export function main(ui: Ui, http: Http, timer: Timer, state: State, log: Log): void {
  const latest = new Latest();
  ui.onClick("refresh", async () => {
    // runs as a root-scope task: polled once now, continues on completions
    const shown = state.open("status");
    switch (shown.kind) {
      case "err":
        log.write(4, `state failed ${shown.error}`);
        return;
      case "ok":
        state.write_str(shown.value, "loading...");
        const r = await withTimeout(timer, 3000u32, http_get(http, "/api/status"));
        switch (r.kind) {
          case "done":
            // r.value is Result<u8[], i32>
            state.write_str(shown.value, "updated");
          case "timedOut":
            state.write_str(shown.value, "timed out");
        }
    }
  });
}
```
(Two clicks in a row start two tasks; use `Latest.replace` to cancel the earlier one. Whether a page-level default policy exists is part of 8.S1, section 10.3.)

### 5.7 Deliberately not in v1

* `Promise<T>` objects, `.then`, shared/multi-consumer futures, `Promise.all/race/any` over arbitrary values (use scopes and `withTimeout`; `all` over a `Task[]` is a std function).
* `for await`, generators and `yield`, async iterators, `Stream<T>` (stays reserved; shares the lowering, section 12 stage A5).
* Unbounded detached tasks (`spawn` without a scope). Everything is owned by the root scope or a scope.
* Awaiting inside `defer`, inside `kernel`/`parallel for` bodies, inside laws, inside module initialisers (`let` at top level) and inside constructors.
* Async `drop`/destructors (classes have none today), exceptions across `await`.
* Blocking waits, `thread::sleep`-like calls, ambient timers (`setTimeout`).
* Preemption within a task (a task yields only at `await`; a CPU-bound loop is stopped by fuel or, on `micro`, by the compiler's budget checks, not rescheduled).
* Async recursion is allowed (frames are heap allocated) but forbidden under `realtime`/`safety` (`no_async` or `no_recursion`).

---

## 6. Lowering to OxiIR and Core Wasm

### 6.1 Principle: existing IR, existing ABI

**A-33.** The async transform is a **front-end lowering step** in `oxiscript-lower` (it works on the checked AST/CFG before the IR is built), not a new IR feature. It emits ordinary verified OxiIR: structs, closures, `switch`, calls. `Terminator::Await` stays a *reserved, unneeded* slot (it would only be added, `#[non_exhaustive]`, if a later optimisation pass wants to run over the unsplit CFG). The three paper lowerings in 6.4 were parsed and verified with the unchanged `parse_module` and `verify` of `oxiscript-ir`, which settles the 8.4a gate sentence "without changing existing IR".

**A-34.** No import or export signature changes in stages A1-A4; the only ABI additions are the constant `MAX_POLLS_PER_TURN` and the tick contract of A-25. Frames use typedesc kind 1 (struct), type erasure uses kind 3 (closure). The scheduler is runtime-support code (the `oxiscript-wasm` runtime module, 8.M family) with module-level state (`@slots`, `@gens`, the ready ring, `@cur_id`, `@cur_gen`), so it resets with the instance (P-8). In this document the word **scheduler** or **driver** means this guest-side task scheduler; it is unrelated to the "executor" of 8.3d (a data-parallel backend policy).

### 6.2 Frames, `start`, `poll`

* **A-35.** For each async function `f` the compiler generates a struct `F_f { state: i32, <owned parameters>, <locals live across an await>, <awaited child frame: ref F_g?>, <result slot>, defers: i32 }`, a function `f__start(own params) -> ref F_f` (allocate, store the owned arguments, `state = 0`) and `f__poll(ref F_f) -> i32` (0 = pending, 1 = done). Parameters are **owned by the frame** (IR arguments are borrowed by default and a borrow cannot outlive the caller returning "pending").
* **A-36.** `f__poll` begins with `switch state` and a mandatory `default` that is `trap unreachable` (polling a finished or cancelled frame is a compiler bug, never a user-visible state). States: `0` initial, `1..k` one per await site, then done, then cancelled. A resume re-evaluates the awaited condition (a wake is a hint, not a promise: spurious wakes are harmless).
* **A-37.** After a resume, **every value that was live across the await is reloaded from the frame**; no borrow is kept across a suspension (the "unprotected borrow" rule of `ir.md` section 4 forbids it anyway). Only values live across an await become frame fields; everything else stays SSA.
* **A-38.** `await g(args)` of an async callee: `g__start` allocates the child frame into a field of the parent, then the parent calls `g__poll` directly (a static call). If it returns done in the same poll, execution continues *without suspending*; otherwise the parent stores its state and returns pending. On completion the parent moves the result out of the child frame and sets the child field to `null` (the child frame is released immediately; the chain holds no back-references).
* **A-39.** Type erasure exists **only through closures** (IR has no dynamic dispatch): a spawned task is a `fn() -> i32` closure whose environment holds `ref F_f` and the task's result cell. Awaiting such a task, or a `Oneshot`, polls a `done` flag in the cell and parks.
* **A-40. No back-links, hence no RC cycles from the machinery.** A suspended task is identified to its waker by a **generational id** `(id: u32, gen: u32)` stored as two integers in the cell it waits on; the table `@slots` holds the only owning reference to a spawned task's closure. Waking is `sched_wake(id, gen)`: ignored if the generation is stale (cancelled or finished task), otherwise the id is appended to the ready ring (idempotent through a queued flag). Frames form a tree by ownership (parent to child frame, scope to child ids), so the transform creates no reference cycle; cycles can still be built by user data (class graphs) and are handled by the existing cycle collector. Frames are therefore **not** flagged `acyclic` by default (the flag may be set when the compiler proves a frame holds only scalars and strings).
* **A-41.** Cancellation support is generated per async function: `f__cancel(ref F_f)` first calls the `g__cancel` of the directly awaited child frame (a static call; the chain unwinds leaf first), then runs the `defer` blocks registered at the current state (a bitmask `defers` in the frame, set when a `defer` statement executes and cleared when it runs), cancels open scopes (children first, innermost first) and marks the state cancelled. The scheduler holds a second closure `fn() -> void` per task for it, calls it, then clears the slot (the frame is released by reference counting and its pointer map frees the owned children).

### 6.3 Driver, fuel, scheduling

* **A-23.** Completions and ticks arrive **only between guest turns** (`abi.md` 5.3). The scheduler never runs inside a host import; it runs only from the epilogue of a guest entry.
* **A-24.** One run queue per instance, FIFO. A task becomes ready by `wake`, by `spawn` (the child is enqueued; it first runs after the spawner suspends or ends its turn, never inside `spawn`) or by `yieldNow()`. A turn polls ready tasks in queue order. A host-op completion is **never** delivered synchronously inside the call that started the operation (no "Zalgo"): the host queues it for a later pump, on every executor including a CPU fallback (A-72).
* **A-25.** Every guest entry that can make a task ready (`oxi_init`, a `cb:` trampoline, a `@host` export, `oxi_tick`) ends with `sched_drain()`, which polls at most `MAX_POLLS_PER_TURN` tasks (an ABI constant the host cannot change; proposed 64 [P]). If the run queue is not empty afterwards, `oxi_tick(now_ms)` returns `now_ms` (the "next deadline is now" answer already in the contract), so a host that calls `oxi_tick` after each event batch (OxiUI's step loop does) continues the work. **8.S1 must assert that call**: today `Runtime::pump` and `tick` are separate public calls (`oxiscript-runtime/src/runtime.rs`), and `oxi_tick` is a stub that calls a user `tick` function or returns -1.
* **A-26.** Fuel is the existing per-entry fuel (`fuel_per_call`, reset at every entry), so a turn is bounded by `MAX_POLLS_PER_TURN` times the cost of one poll; a poll that runs out of fuel traps and poisons the instance exactly like any callback does. On `micro` (no fuel, D-27) the compiler's budget checks count poll iterations and loop back-edges against the same constant (8.2b). Async adds no new trap kind.
* **A-42.** Limits (host knobs, defaults proposed [P]): `max_tasks` (1024) and `max_pending_ops`. Exceeding `max_tasks` in `spawn` traps like an allocation failure (`out_of_memory`).
* **A-43.** The await chain is polled recursively (parent polls child); depth is bounded by the existing `max_recursion_depth`. A wake re-polls the task's root and the chain below it: O(depth) per wake, depth is small in practice. A trampolined variant (resume the leaf, then its parent) would need parent back-references and is deliberately not taken in v1 (cycles).

### 6.4 Three paper lowerings (verified)

The IR below was written by hand following A-35..A-41, parsed with `oxiscript_ir::parse_module` and checked with `oxiscript_ir::verify` (a scratch harness outside the repository): all three print `OK`. The verifier does structural checks only, not the RC-balance checker of 8.M; RC balance of frames is a gate of stage A3. Names use `__` because `$` is not an identifier character in the IR text. Runtime-support functions have `trap unreachable` bodies where marked.

**Example 1: await chain with `?`.**

```ts
// proposed syntax: the source
async function inner(o: Once): Result<i32, i32> { await o.wait(); return Ok(7); }
async function outer(o: Once): Result<i32, i32> {
  const v = await inner(o)?;
  return Ok(v + 1);
}
```

```oxir
oxiir 1
module "async_ex1"

type Once = struct { done: bool, wid: u32, wgen: u32 }
type ResI = union { Ok { v: i32 }, Err { e: i32 } }
type Inner = struct { state: i32, o: ref Once, res: ref ResI? }
type Outer = struct { state: i32, o: ref Once, child: ref Inner?, res: ref ResI? }

global @cur_id: u32 = 0
global @cur_gen: u32 = 0

func @inner__start(%o: own ref Once) -> ref Inner {
entry:
  %f: ref Inner = alloc Inner
  %z: i32 = const 0
  setfield.init Inner.state %f, %z
  setfield.init Inner.o %f, %o
  ret %f
}

func @inner__poll(%f: ref Inner) -> i32 {
entry:
  %st: i32 = getfield Inner.state %f
  switch %st, [0: s0(), 1: ready()], default bad()
s0:
  %o: ref Once = getfield Inner.o %f
  %d: bool = getfield Once.done %o
  br_if %d, ready(), park()
park:
  %o2: ref Once = getfield Inner.o %f
  %id: u32 = global.get @cur_id
  %gen: u32 = global.get @cur_gen
  setfield Once.wid %o2, %id
  setfield Once.wgen %o2, %gen
  %one: i32 = const 1
  setfield Inner.state %f, %one
  %p: i32 = const 0
  ret %p
ready:
  %seven: i32 = const 7
  %r: ref ResI = alloc ResI::Ok
  setfield.init ResI::Ok.v %r, %seven
  %ro: ref ResI? = opt %r
  setfield Inner.res %f, %ro
  %two: i32 = const 2
  setfield Inner.state %f, %two
  %done: i32 = const 1
  ret %done
bad:
  trap unreachable
}

func @outer__start(%o: own ref Once) -> ref Outer {
entry:
  %f: ref Outer = alloc Outer
  %z: i32 = const 0
  setfield.init Outer.state %f, %z
  setfield.init Outer.o %f, %o
  ret %f
}

func @outer__poll(%f: ref Outer) -> i32 {
entry:
  %st: i32 = getfield Outer.state %f
  switch %st, [0: s0(), 1: s1()], default bad()
s0:
  %o: ref Once = getfield Outer.o %f
  retain %o
  %c: ref Inner = call @inner__start(%o)
  %co: ref Inner? = opt %c
  setfield Outer.child %f, %co
  %one: i32 = const 1
  setfield Outer.state %f, %one
  br s1()
s1:
  %cc: ref Inner? = getfield Outer.child %f
  %ci: ref Inner = unopt %cc
  %r: i32 = call @inner__poll(%ci)
  %zero: i32 = const 0
  %pend: bool = eq %r, %zero
  br_if %pend, pending(), childdone()
pending:
  %p: i32 = const 0
  ret %p
childdone:
  %cc2: ref Inner? = getfield Outer.child %f
  %ci2: ref Inner = unopt %cc2
  %res: ref ResI? = getfield Inner.res %ci2
  %resv: ref ResI = unopt %res
  %t: u32 = tag %resv
  %okt: u32 = const 0
  %isok: bool = eq %t, %okt
  br_if %isok, ok(), err()
ok:
  %v: i32 = getfield ResI::Ok.v %resv
  %one2: i32 = const 1
  %v1: i32 = add %v, %one2
  %r2: ref ResI = alloc ResI::Ok
  setfield.init ResI::Ok.v %r2, %v1
  br fin(%r2)
err:
  %e: i32 = getfield ResI::Err.e %resv
  %r3: ref ResI = alloc ResI::Err
  setfield.init ResI::Err.e %r3, %e
  br fin(%r3)
fin(%out: ref ResI):
  %oo: ref ResI? = opt %out
  setfield Outer.res %f, %oo
  %nul: ref Inner? = null
  setfield Outer.child %f, %nul
  %two: i32 = const 2
  setfield Outer.state %f, %two
  %d: i32 = const 1
  ret %d
bad:
  trap unreachable
}
```

What it shows: `start`/`poll` pairs, `switch` on the state with a trapping default, parking by storing the waker id, the direct `poll` of the child with the synchronous-completion fast path (`s0` falls into `s1` without returning), `?` as a `tag` test whose `Err` arm rebuilds the outer `Err` (same `E`, no conversion) and a `fin` block that clears the child field.

**Example 2: a scope with two spawned children and two awaits.**

```ts
// proposed syntax: the source (the scope is inlined into the frame of `both`, A-12)
async function waitThen(o: Once, k: i32): i32 { await o.wait(); return k; }
async function both(a: Once, b: Once): i32 {
  return await scope(async (s: Scope): i32 => {
    const ta = s.spawn(waitThen(a, 1));
    const tb = s.spawn(waitThen(b, 2));
    return (await ta) + (await tb);
  });
}
```

```oxir
oxiir 1
module "async_ex2"

type Once = struct { done: bool, wid: u32, wgen: u32 }
; Task<i32> handle: shared between the running task and its handle holder
type Cell = struct { done: bool, val: i32, wid: u32, wgen: u32, tid: u32, tgen: u32 }
; frame of: async function waitThen(o: Once, k: i32): i32 { await o.wait(); return k; }
type W = struct { state: i32, o: ref Once, k: i32 }
; the type-erased scheduler entry for a spawned W: an ordinary closure env
type WThunk = closure { fr: ref W, cell: ref Cell }
; frame of: async function both(a: Once, b: Once): i32 (scope inlined)
type Both = struct { state: i32, a: ref Once, b: ref Once, ta: ref Cell?, tb: ref Cell?, res: i32 }

global @cur_id: u32 = 0
global @cur_gen: u32 = 0

; ---- runtime support (bodies elided in this paper lowering) ----
func @sched_enqueue(%poll: own fn() -> i32, %cell: ref Cell) -> void {
entry:
  trap unreachable
}
func @sched_wake(%id: u32, %gen: u32) -> void {
entry:
  trap unreachable
}

func @w__poll(%f: ref W) -> i32 {
entry:
  %st: i32 = getfield W.state %f
  switch %st, [0: chk(), 1: chk()], default bad()
chk:
  %o: ref Once = getfield W.o %f
  %d: bool = getfield Once.done %o
  br_if %d, fin(), park()
park:
  %id: u32 = global.get @cur_id
  %gen: u32 = global.get @cur_gen
  setfield Once.wid %o, %id
  setfield Once.wgen %o, %gen
  %one: i32 = const 1
  setfield W.state %f, %one
  %p: i32 = const 0
  ret %p
fin:
  %two: i32 = const 2
  setfield W.state %f, %two
  %r: i32 = const 1
  ret %r
bad:
  trap unreachable
}

; scheduler entry for a spawned task: polls the frame, publishes the result, wakes the joiner
func @w__thunk(%env: ref WThunk) -> i32 {
entry:
  %f: ref W = getfield WThunk.fr %env
  %c: ref Cell = getfield WThunk.cell %env
  %r: i32 = call @w__poll(%f)
  %zero: i32 = const 0
  %pend: bool = eq %r, %zero
  br_if %pend, pending(), done()
pending:
  %p: i32 = const 0
  ret %p
done:
  %k: i32 = getfield W.k %f
  setfield Cell.val %c, %k
  %t: bool = const true
  setfield Cell.done %c, %t
  %wid: u32 = getfield Cell.wid %c
  %wgen: u32 = getfield Cell.wgen %c
  call @sched_wake(%wid, %wgen)
  %d: i32 = const 1
  ret %d
bad:
  trap unreachable
}

; s.spawn(waitThen(o, k)): allocate the frame now (arguments evaluated now), run it later
func @spawn_w(%o: ref Once, %k: i32) -> ref Cell {
entry:
  %f: ref W = alloc W
  %z: i32 = const 0
  setfield.init W.state %f, %z
  retain %o
  setfield.init W.o %f, %o
  setfield.init W.k %f, %k
  %c: ref Cell = alloc Cell
  retain %c
  %th: fn() -> i32 = closure.new WThunk @w__thunk(%f, %c)
  call @sched_enqueue(%th, %c)
  ret %c
}

func @both__poll(%f: ref Both) -> i32 {
entry:
  %st: i32 = getfield Both.state %f
  switch %st, [0: s0(), 1: wa(), 2: wb()], default bad()
s0:
  %a: ref Once = getfield Both.a %f
  %b: ref Once = getfield Both.b %f
  %one: i32 = const 1
  %two: i32 = const 2
  %ca: ref Cell = call @spawn_w(%a, %one)
  %cb: ref Cell = call @spawn_w(%b, %two)
  %cao: ref Cell? = opt %ca
  %cbo: ref Cell? = opt %cb
  setfield Both.ta %f, %cao
  setfield Both.tb %f, %cbo
  br wa()
wa:
  %tao: ref Cell? = getfield Both.ta %f
  %ta: ref Cell = unopt %tao
  %da: bool = getfield Cell.done %ta
  br_if %da, wb(), parka()
parka:
  %id: u32 = global.get @cur_id
  %gen: u32 = global.get @cur_gen
  setfield Cell.wid %ta, %id
  setfield Cell.wgen %ta, %gen
  %s1: i32 = const 1
  setfield Both.state %f, %s1
  %p: i32 = const 0
  ret %p
wb:
  %tbo: ref Cell? = getfield Both.tb %f
  %tb: ref Cell = unopt %tbo
  %db: bool = getfield Cell.done %tb
  br_if %db, fin(), parkb()
parkb:
  %id2: u32 = global.get @cur_id
  %gen2: u32 = global.get @cur_gen
  setfield Cell.wid %tb, %id2
  setfield Cell.wgen %tb, %gen2
  %s2: i32 = const 2
  setfield Both.state %f, %s2
  %p2: i32 = const 0
  ret %p2
fin:
  ; (await ta) + (await tb): every value is reloaded from the frame after a resume
  %tao2: ref Cell? = getfield Both.ta %f
  %ta2: ref Cell = unopt %tao2
  %va: i32 = getfield Cell.val %ta2
  %vb: i32 = getfield Cell.val %tb
  %sum: i32 = add %va, %vb
  setfield Both.res %f, %sum
  %s3: i32 = const 3
  setfield Both.state %f, %s3
  %d: i32 = const 1
  ret %d
bad:
  trap unreachable
}
```

What it shows: `spawn` evaluates arguments and allocates the frame now (`@spawn_w`), wraps it in a closure `WThunk` (the type-erased scheduler entry that polls, publishes the result into the `Cell` and wakes the joiner) and enqueues it; the parent awaits a `Cell` exactly like a `Oneshot`; after the resume every value is reloaded from the frame (`fin` reloads `ta`). Elided: the `@sched_enqueue` and `@sched_wake` bodies, the scope's child-id list used by cancellation, and the scope-exit cancel of unconsumed children (none here: both are awaited).

**Example 3: `defer` and cancellation.**

```ts
// proposed syntax: the source
let open: i32 = 0;
async function guarded(o: Once): i32 {
  open = open + 1;
  defer { open = open - 1; }
  await o.wait();
  return 1;
}
```

```oxir
oxiir 1
module "async_ex3"

type Once = struct { done: bool, wid: u32, wgen: u32 }
; async function guarded(o: Once): i32 { open += 1; defer { open -= 1; } await o.wait(); return 1; }
type G = struct { state: i32, o: ref Once, defers: i32, res: i32 }
type GCancel = closure { fr: ref G }

global @open: i32 = 0
global @cur_id: u32 = 0
global @cur_gen: u32 = 0

; the body of the `defer` block, compiled once, called from every exit path
func @g__defer0(%f: ref G) -> void {
entry:
  %n: i32 = global.get @open
  %one: i32 = const 1
  %m: i32 = sub %n, %one
  global.set @open, %m
  %z: i32 = const 0
  setfield G.defers %f, %z
  ret
}

func @g__poll(%f: ref G) -> i32 {
entry:
  %st: i32 = getfield G.state %f
  switch %st, [0: s0(), 1: chk()], default bad()
s0:
  %n: i32 = global.get @open
  %one: i32 = const 1
  %n1: i32 = add %n, %one
  global.set @open, %n1
  setfield G.defers %f, %one
  br chk()
chk:
  %o: ref Once = getfield G.o %f
  %d: bool = getfield Once.done %o
  br_if %d, fin(), park()
park:
  %id: u32 = global.get @cur_id
  %gen: u32 = global.get @cur_gen
  setfield Once.wid %o, %id
  setfield Once.wgen %o, %gen
  %s1: i32 = const 1
  setfield G.state %f, %s1
  %p: i32 = const 0
  ret %p
fin:
  ; normal exit: defers run before the frame reports completion
  call @g__defer0(%f)
  %r: i32 = const 1
  setfield G.res %f, %r
  %s2: i32 = const 2
  setfield G.state %f, %s2
  ret %r
bad:
  trap unreachable
}

; generated per async function: runs the defers that are registered at the current state;
; no user code other than defer bodies runs on cancel; the scheduler then drops the frame
func @g__cancel(%env: ref GCancel) -> void {
entry:
  %f: ref G = getfield GCancel.fr %env
  %mask: i32 = getfield G.defers %f
  %z: i32 = const 0
  %none: bool = eq %mask, %z
  br_if %none, out(), run()
run:
  call @g__defer0(%f)
  br out()
out:
  %s: i32 = const 3
  setfield G.state %f, %s
  ret
}

func @g__cancel_closure(%f: ref G) -> fn() -> void {
entry:
  %c: fn() -> void = closure.new GCancel @g__cancel(%f)
  ret %c
}
```

What it shows: the `defer` body is compiled once (`@g__defer0`) and called from the normal exit path and from the generated `@g__cancel`, guarded by the `defers` bitmask so it runs exactly once; the frame's state becomes "cancelled" (3) and the scheduler drops the slot. The timeout of section 5.3 is built on this: `withTimeout` spawns the work and an alarm and `cancel()`s the loser; the alarm's `sleep` wrapper has a `defer` that cancels the host timer, so the host timer is cancelled by the same mechanism. The walk down the awaited chain (`@g__cancel` calling the child's cancel first) is elided in this single-frame example.

### 6.5 Cost estimates [P] (to be measured in A1 and A3)

* Frame: 4 bytes of state + 4 bytes `defers` + parameters + live locals + the 8-byte RC header, one allocation per call that is awaited (a later optimisation can elide it for callees that cannot suspend on the taken path).
* Code: one `poll` per async function, about 30-60 bytes of Wasm per await site, a scheduler of roughly 1-1.5 KB. On `micro` these are the whole price; no stack switching and no extra engine feature, so w2c2 translates `poll` to one C function with a `br_table`.
* Time: one `switch` per poll and one closure call per spawned task per turn; await chains cost O(depth) per wake.

---

## 7. Determinism (P-9)

* **A-44.** Inside one instance there is no thread, no preemption and no source of time or randomness other than host deliveries: a program's behaviour is a function of the **delivery log** (sequence of `{event kind, arguments, now_ms}` at guest entries) and the module.
* **A-45.** The scheduler is deterministic by construction: FIFO ready ring, ids from a deterministic free list, no hash-order iteration, no address-dependent decision. Fuel use per entry is identical for identical logs on wasmi and (as budget-check counts) on w2c2 and the browser.
* **A-46.** What is *not* deterministic is the order in which a real host's operations complete. The host contract: it assigns each delivery a position in the log and delivers in log order; a test host replays a recorded log exactly. This is the "record/replay" gate of A4.
* **A-47.** `await` on a `Oneshot` or `Task` that is already complete proceeds without suspending (guest-internal, deterministic); `await` on a host operation always suspends at least one turn (A-24). Two executors that complete work at different speeds (CPU fallback and GPU) therefore produce the same task interleaving.
* **A-48.** Timers: deadlines are measured by the host clock; the order of due timers is `(deadline, creation id)`. Tests use a virtual clock.

---

## 8. Cancellation and timeouts

* **A-49.** **Cancellation points** are exactly the suspended `await` sites (and the not-yet-started state). An instance is single-threaded, so the cancelled task is always suspended or unstarted when cancelled; cancellation never interrupts straight-line code.
* **A-50.** **Signal:** none. A cancelled task is not resumed; user code observes nothing, so cancellation needs no error variant in `Result<T, E>` (the mixed-`E` problem that "no automatic error conversion" would create disappears) and no polling. The only user code that runs is `defer` blocks.
* **A-51.** **Order:** cancelling a task cancels, innermost first, the directly awaited child frame chain (leaf first, so a wrapper such as `http_get` or `sleep` runs its `defer`), then its open scopes' children (recursively, reverse spawn order), then runs its own registered `defer`s in reverse registration order, then releases its frame. Cancelling a finished, already cancelled or stale id is a no-op.
* **A-52.** A `defer` body that traps poisons the instance like any trap; a `defer` body cannot cancel or spawn.
* **A-53.** **Host operations in flight** are cancelled by the wrapper's `defer` (A-20, A-21). A host that completes the operation anyway delivers a callback that resolves an orphaned `Oneshot`; `resolve` returns `false` (the waiter is gone) and is otherwise harmless. Side effects the host already performed are not undone: idempotency is the capability author's contract (documented per IDL function).
* **A-54.** `Task.cancel()` consumes the handle (A-14). `Scope` offers `cancelAll()`.
* **A-55.** **Timeouts** are `withTimeout` and `sleep` over the `Timer` capability, composable (nested timeouts cancel from the inside out). A timeout is "stop waiting and drop", never "interrupt a computation": a CPU-bound task is bounded by fuel, not by timeouts.
* **A-56.** A task cannot shield itself from cancellation in v1 (no `shield`/`uncancellable`); anything that must complete (a commit) belongs in the host or in a state-capability operation.

---

## 9. Hot reload with tasks in flight (P-8, D-9)

* **A-57.** A reload builds a new instance and swaps at a quiescent point between turns, as today.
* **A-58.** The new instance must finish `oxi_init` (including the synchronous prefix of an `async main`) before the swap; if that fails, the old instance keeps running with all its tasks untouched (atomic accept-or-reject, R3-26 in the roadmap).
* **A-59.** At the swap all tasks of the old instance are **cancelled without running `defer`** and the instance is discarded; pending host operations and timers of the old instance are cancelled by the host as part of dropping its registrations (the existing rule for callbacks, `abi.md` 4, extended to operations). Operation ids are generational per instance, so a late completion for the old instance is dropped by the host.
* **A-60.** Frames are never migrated or patched: layout and state numbers are specific to one compiled version (the same reason module-local state resets; Dart's rule). Work that must survive a reload is recorded in the host `state` capability and **restarted idempotently** by the new `main`:

```ts
// proposed syntax: the shape of an idempotent restart (state.open and the slot API are the real host:state)
export function main(http: Http, state: State, log: Log): void {
  const job = state.open("upload.job");          // Result<Slot, i32>
  switch (job.kind) {
    case "err":
      log.write(4, `state failed ${job.error}`);
    case "ok":
      // read the recorded offset with state.get_i64(job.value) and start the async upload again from there
      log.write(2, "resuming upload");
  }
}
```

* **A-61.** A reload requested during a turn is queued until the turn ends (BAD ACTOR gate item of 8.S1).
* **A-62.** Drain-style reloads (let in-flight tasks finish on the old instance) are deliberately not offered in v1.
* **A-63.** Orderly shutdown (`oxi_deinit` called by the host, page closed) cancels the root scope **with** `defer`s under normal fuel; a reload swap and a trap do not (A-29). A program may rely on `defer` only for resources the guest itself must release at an orderly exit.

---

## 10. Interaction with other stages

### 10.1 Threads and isolates (8.4)

* **A-64.** A *task* is cooperative concurrency inside one instance (one heap, one thread). An *isolate* (8.4) is a separate instance with a private heap and its own scheduler; tasks never migrate between isolates and no memory is shared, so **no data race exists by construction** and `Send`-like bounds are not needed inside an isolate. What may cross an isolate boundary (move or copy of acyclic data, no closures, no class identity) is 8.4's decision, not this spec's.
* **A-65.** Isolate operations are *host operations in the sense of A-18*: `ch.recv()` and `iso.join()` are async functions generated from channel and isolate capabilities (a `host:isolate` interface, to be proposed by 8.4), completing through the same callback and `Oneshot` path. An isolate handle is owned by a scope; leaving the scope cancels the isolate (a cooperative request, then the host's hard stop after a deadline) and joins it.
* **A-66.** Data-parallel combinators, `parallel for` and `kernel` bodies have the semantics of sequential code (P-4): they never contain `await` (`OXS-E-AWAIT-IN-PARALLEL`), and a call to one inside an async function is a plain synchronous call that returns when the work is done; other tasks of the instance do not run in between even if worker threads execute the region. Its duration is bounded by the turn's fuel (or by the policy 8.4 defines).
* **A-67.** **Conformance mode:** the same program must give the same results with isolates on worker threads and with `--sched=det`, a host mode that runs all isolates on one thread in a fixed round-robin driven by the delivery log. It is needed for P-4's "parallel equals sequential" gate; it is an implementation detail of the 8.4 host, stated here so the async side is not blocked by it. A `realtime` or `safety` module on a single core uses exactly this mode.
* **A-68.** The browser worker-instance backend gives each worker its own scheduler; messages arrive as ordinary deliveries.

### 10.2 GPU executors (8.3)

* **A-69.** `Resident<T>` read-back is an **awaitable**, not a language special case: `std/resident` offers `async function download<T>(r: Resident<T>): Result<T[], i32>`; `upload` stays synchronous and visible. The IDL of the SIMD/GPU design (`request_download(buf, off, len, cb) -> Fence`, `OnReadbackReady(fence, status, byte_len)`, `read_download_chunk`) **is already the begin/completion/take shape of A-19**, so 8.3e's callback API stays valid unchanged and becomes `async fn download` in the IDL when A4 lands; `await buf.download()` in the 8.3 design text is then literally the wrapper of A-20.
* **A-70.** The only suspension point of a GPU pipeline is the download (and any explicit `yieldNow`); each awaited download appears as a transfer line in the placement report (P-5).
* **A-71.** Cancelling a task awaiting a download cancels the fence through the wrapper's `defer`; device buffers are released by the handle release path (the `oxi_release` queue), not by the task, so cancellation never leaks or double-frees a `Resident`. (8.3d must give `Resident` handles a release path on RC drop; classes have no destructors today.)
* **A-72.** **No behavioural difference between backends:** the CPU fallback of `download` completes through the callback in a *later turn* like the GPU path (A-24, A-47), so a program has the same task interleaving on CPU and GPU executors; numeric results follow the contracts of the SIMD/GPU report (exact for integers, in-contract for floats) and are unaffected by awaiting.

### 10.3 OxiUI pages (8.S1)

* **A-73.** An event handler is an `@host export async function` (A-31) or an async arrow passed to a callback parameter (A-32). The host delivers one event per guest entry; the handler's synchronous prefix runs inside that entry (state writes before the first `await` are visible when the entry returns), then the task continues on later deliveries. Handlers of different events (or the same event twice) can therefore **interleave across turns**; `Latest.replace` gives the usual "cancel the previous request" policy. No default exclusion is imposed by the language.
* **A-74.** Awaiting never blocks rendering: a turn is one bounded guest entry (A-25, A-26); the frame loop is the host's.
* **A-75.** Server-driven and browser-local (WASM local mode) pages use the same mechanism; only the host's delivery path differs. Page unmount or session close is `oxi_deinit` (A-63); a reload is A-57..A-62.
* **A-76.** State written after an `await` goes through the `state` capability (handles validated per call), so a stale task of a discarded instance cannot write: its frame is gone (A-59) and its handles are generational.
* **A-77.** What 8.S1 must provide for this spec: the host calls `oxi_tick` after each event batch (A-25), delivers operation completions in log order (A-46), cancels operations and timers of a discarded instance (A-59) and queues reloads to quiescent points (A-61).

---

## 11. Profiles, `micro`, tooling

* **A-78.** `no_async` (8.2c vocabulary) forbids any function with the async effect (A-4); the effect summary is its single source. `realtime` allows async only for non-recursive async call graphs with `bounded_alloc` frames (frame sizes are static) and a bounded number of spawn sites; `safety` additionally makes `OXS-W-TASK-UNCONSUMED` an error and requires `max_tasks` to be a manifest constant.
* **A-79.** The placement report (`placement.json`) lists per async function: await sites, frame size in bytes, values live across awaits, spawn sites, scope nesting, and one `P` note when a frame exceeds a threshold or captures large arrays (so the RC/memory cost of captured state is visible). These are heuristics (`assertable no`) except `@expect(no_async)`.
* **A-80.** `oxi check --fix` adds a missing `async` (A-3); for `OXS-W-TASK-UNCONSUMED` it offers `await` or `.cancel()` as non-machine-applicable suggestions.
* **A-81.** `llms.txt`, `AGENTS.md` and the authoring skill get a short async section when A2 lands (the "what differs from TypeScript" list gains: `async` returns the plain type, `Pending` and `Task` are single-consumer, no `Promise`, cancellation = not resumed + `defer`). Those examples must compile (`cargo test -p oxiscript-cli` compiles every ```oxs block).

---

## 12. Staged plan with gates

Every stage gate also includes: a BAD ACTOR round (D-33, D-34 batching allowed) and a leak check (`oxi_live_objects() == 0` after `oxi_deinit`).

| Stage | Content | Gate | Depends on |
|---|---|---|---|
| **A0** (this document) | Spec, three verified paper lowerings | Owner reviews; answers Q-ASYNC-1..5 | none |
| **8.Fix item** | Reserve `defer` (E0900); check whether a reserved word such as `spawn` is accepted as a member name after `.` (fallback method name `launch`) | Existing tests pass | none |
| **A1** Scheduler and `Oneshot` library, no new syntax | Runtime-support scheduler (slots, generations, ready ring, `sched_drain`, `oxi_tick` returns `now_ms` while work remains), `Oneshot` in `std/async` in today's OxiScript with a callback-level API only (`resolve`, a done flag, the park id; generic classes exist, see `std/collections`); `wait()` arrives with A3, proposed `host:timer` IDL with a virtual-clock test host, hand-written task closures | Deterministic replay of a recorded delivery log gives identical output and fuel; stale-id wake and cancel-while-queued tests; scheduler size on `micro` measured against 6.5; tick contract written down for 8.S1 | 8.M-link, 8.Std |
| **A2** Front end | Un-reserve and parse `async`/`await`/`defer`; async effect summary; `Pending`/`Task` single-consumer dataflow; reason codes (append-only) with fix-its; the `await e?` rule; docs and skill examples | Checker corpus with an `oxs-err` example per new code; the syntax fuzzer extended; every doc block compiles | A0 |
| **A3** Lowering | `async-lower` in `oxiscript-lower`: frames, `start`/`poll`/`cancel`, `defer`, inlined scopes, wake ids; RC-balance checker accepts frames; the three examples of 6.4 produce IR equal up to names to the paper versions | Differential wasmi vs w2c2 (after 8.2b) vs browser on a task-graph corpus; **model-based fuzz test**: random task graphs with spawn, cancel, yield and timeouts against a small Rust reference scheduler, identical observable log; BAD ACTOR on cancel inside `defer`, stale wake, cancel during a turn, frame release order, reload during a turn | A1, A2 |
| **A4** Host async IDL | `async fn` in the IDL front end and `oxi-bindgen`; generated wrappers; `host:timer`, a test `host:http`; delivery-log record and replay in `oxi run` and the test host | Live run and replay are identical; host-contract tests (completion exactly once or never, late completion dropped, generational ids); the 8.S1 demo uses it | A3, 8.S1 host interface |
| **A5** Streams (optional) | `Stream<T>`, `for await`, generators sharing the A3 lowering | Its own spec first | A3 |
| **8.4 / 8.3e integration** | Isolate channel and join as host operations (A-65); `std/resident` `download` as `async fn` (A-69); `--sched=det` | P-4 gate: parallel equals sequential on the corpus, including task interleavings | A4, 8.4, 8.3e |

Ordering note: **8.S1 can ship with callbacks plus A1** (no async syntax) if A2 and A3 are late. Code written as callbacks over the callback-level `Oneshot` is easy to port to the later syntax: `wait()` is the same object with a suspending method, and the wrapper of A-20 is the same library.

---

## 13. Open questions for the owner (five)

1. **Q-ASYNC-1: Is `async` written or inferred?** The roadmap text says "inferred effect". *Recommendation: written (`async function`), with the effect summary computed anyway and fix-its.* A TS developer reads it without learning anything; exported signatures stay explicit (as `AGENTS.md` already demands for types); inference makes adding one `await` silently change the type of every caller, including `@host` boundaries and interfaces; an agent can rely on the keyword. Cost: the keyword on every async function (colouring remains, mitigated by A-6).
2. **Q-ASYNC-2: Return type: `async function f(): R` (plain) or `Promise<R>`?** *Recommendation: plain `R`.* `Promise<Result<T, E>>` is noise and a promise implies semantics we do not give (shared, eager, `.then`). The unawaited call has type `Pending<R>`. If you want the maximal TS look, `Promise<R>` could be a spelling of the declared type without changing anything else; it is a pure surface choice.
3. **Q-ASYNC-3: Cancellation: not resumed plus `defer` (recommended), or an observable cancel signal?** *Recommendation: not resumed.* No error variant leaks into every `E`, no polling, no "swallowed cancel" bug, the simplest lowering (drop the frame). Cost: a task cannot return a partial result on cancel (it must write progress to state or a `Oneshot` before awaiting).
4. **Q-ASYNC-4: Introduce `defer` (also for sync code) with this work?** *Recommendation: yes, and reserve the word now* (8.Fix; nothing is public yet). It is the only cleanup mechanism a no-exception language with cancellation can have; without it every capability wrapper needs its own cancel protocol.
5. **Q-ASYNC-5: Reload with tasks in flight: cancel all without `defer` (recommended) or offer a drain mode?** *Recommendation: cancel all, no defers, the host reclaims registrations, no drain in v1.* It matches P-8 and Dart's rule (reload resets), keeps the swap atomic and bounded and avoids running old code after the swap. Programs that need continuity persist a job record in `state` and restart idempotently (A-60).

Decided here and easy to reverse (not asked): poll budget 64, `max_tasks` 1024, member name `spawn` vs `launch`, the name `Latest`.

---

## 14. References

Verification: [V] read this session, [S] snippet or failed fetch, [R] recalled and not re-checked (check details before relying on them).

1. MDN, `async function`: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/async_function [R]
2. MDN, `Promise`: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Promise [R]
3. MDN, `AbortSignal`: https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal [R]
4. Bob Nystrom, "What Color is Your Function?" (2015): https://journal.stuffwithstuff.com/2015/02/01/what-color-is-your-function/ [R]
5. The Rust Async Book: https://rust-lang.github.io/async-book/ [R]
6. `std::pin`: https://doc.rust-lang.org/std/pin/index.html [R]
7. Tokio tutorial, `select!` and cancellation by drop: https://tokio.rs/tokio/tutorial/select [R]
8. Kotlin coroutines basics and structured concurrency: https://kotlinlang.org/docs/coroutines-basics.html [R]
9. Kotlin, cancellation and timeouts: https://kotlinlang.org/docs/cancellation-and-timeouts.html [R]
10. Swift evolution SE-0304, structured concurrency: https://github.com/swiftlang/swift-evolution/blob/main/proposals/0304-structured-concurrency.md [R]
11. Go blog, "Go Concurrency Patterns: Context": https://go.dev/blog/context [R]
12. LWN, Zig async redesign: https://lwn.net/Articles/1049006/ [S] (cited in `oxiscript-language.md`)
13. Zig commit removing async: https://git.jakstys.lt/motiejus/zig/src/commit/9f0b9b8da1a111d16eb8d1254212ff98a8b4be08 [S] (cited in `oxiscript-language.md`)
14. Erlang reference manual, processes: https://www.erlang.org/doc/system/ref_man_processes.html [R]
15. Lua 5.4 manual, coroutines: https://www.lua.org/manual/5.4/manual.html#2.6 [R]
16. OCaml manual, effect handlers: https://ocaml.org/manual/5.3/effects.html [R]
17. Koka book: https://koka-lang.github.io/koka/doc/book.html [R]
18. WebAssembly stack switching explainer (continuations, `suspend`/`resume`/`switch`, one-shot): https://github.com/WebAssembly/stack-switching/blob/main/proposals/stack-switching/Explainer.md [V]
19. WebAssembly proposals list (stack switching Phase 3, JSPI Phase 5, threads Phase 4): https://github.com/WebAssembly/proposals [V]
20. wasmi feature table (tail calls, multi-memory, memory64 supported; threads, exception handling, GC tracking only; stack switching not listed; built-in fuel metering): https://github.com/wasmi-labs/wasmi [V]
21. w2c2 (MIT; Wasm 2.0 and threads documented; exceptions, tail calls, stack switching not mentioned): https://github.com/turbolent/w2c2 [V]
22. JSPI overview: https://github.com/WebAssembly/js-promise-integration/blob/main/proposals/js-promise-integration/Overview.md [S] (fetch returned 503)
23. WASI 0.3 announcement (native async, `stream<T>` and `future<T>`): https://bytecodealliance.org/articles/WASI-0.3 [S] (via `oxiscript-language.md`)
24. Nathaniel J. Smith, "Notes on structured concurrency, or: Go statement considered harmful": https://vorpus.org/blog/notes-on-structured-concurrency-or-go-statement-considered-harmful/ [R]
25. Repository: `docs/roadmap/oxiscript.md` (P-6..P-9, D-1, D-17, D-27, D-28, 8.4a, 8.4, 8.S1, 8.Dom), `docs/oxiscript/grammar.md` (2.1, 5), `abi.md` (2, 3, 4, 5.3, 5.4), `ir.md` (1, 3, 4, 7), `docs/research/oxiscript-language.md` (F.5, risk table), `docs/research/oxiscript-sync-callbacks.md`, `docs/research/oxiscript-simd-gpu-design.md` (4.4), `oxiscript/oxiscript-runtime/src/{runtime,limits}.rs`, `oxiscript/oxiscript-wasm/src/wrap.rs`.
