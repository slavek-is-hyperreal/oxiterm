# OxiScript developer experience: what the persona loves, misses, and what we should build

> **Status (2026-11-26):** the `Status` column of the tables below is partly stale; current status is in [../roadmap/oxiscript.md](../roadmap/oxiscript.md). Shipped since this was written: the checker (8.T), parser and formatter (8.P), lowering and codegen, `oxi run` and `oxi build`, the DevKit v0 (8.D0). Still planned: LSP (8.S2), laws (8.S4a), placement report (8.S4b), Rust crates (8.N), board (8.S3).
>
> **Note (2026-11-26, plan-cleanup):** this document predates the roadmaps. Where it cites `oxi-ui-plan-history.md` with line numbers or section names, it means the old OxiUI plan file as it was at commit `3017fd5` (its Phase 1-7 text is kept in `docs/history/oxi-ui-plan-history.md`; its Phase 8 text, principles `P-n` and decisions `D-n` are now in [../roadmap/oxiscript.md](../roadmap/oxiscript.md), the OxiUI half in [../roadmap/oxiui.md](../roadmap/oxiui.md)).

Date: 2026-11-25. Author: research agent (developer experience). Scope: web-researched, not implemented.

**Legend.** `[V]` = a primary page was fetched and read. `[S]` = taken from a search-result summary only (secondary or snippet). `[M]` = from the author's background knowledge, not re-checked in this session (treat as unverified; canonical URL given where I am confident of it). `[I]` = internal repo document. `[O]` = opinion/inference. Percentages and counts are quoted as the source gave them; several secondary sources disagree and that is flagged.

Internal references: `docs/history/oxi-ui-plan-history.md` Phase 8 [I], `docs/oxiscript/grammar.md` [I] (section numbers are cited as "G §n"; "G §6 #n" = row n of "Deviations from TypeScript"), `docs/research/oxiscript-language.md` [I].

---

## 0. Executive summary (read this if nothing else)

1. The persona's biggest unmet needs are not language features. They are (a) feedback speed (save -> see/hear it on the device in under 2 s), (b) zero config (no tsconfig/eslint/prettier/bundler sprawl), (c) errors that say what to do, (d) a simulator so the toy works before the board arrives, and (e) agent-friendliness (the persona's agent writes most of the code).
2. TypeScript 7.0 (native/Go port, stable 2026-07-08) is already ~8-12x faster on full builds vs the 6.0 baseline [S], so "tsc is slow" is no longer a differentiator. Our fast-feedback story must be measured against TS7, not TS5.
3. The embedded-TypeScript precedent (DeviceScript) was archived by its owner on 2025-12-08 with no stated reason [V]. Our own research already reads this as "lab project without an owner" [I]. Lesson: ship the loop (editor -> device) with an owner, not just a compiler.
4. Agents are now a first-class user. Concrete patterns exist: gopls ships an MCP server whose instructions say "After every code modification, you MUST call the `go_diagnostics` tool" [V], Svelte ships an MCP server with an `svelte-autofixer` meant to be looped until clean plus an AGENTS.md snippet [S], Biome has a JSON reporter [S]. The Stack Overflow 2025 survey says 66% of developers fight "almost right" AI output [S]. A language whose compiler returns structured diagnostics with machine-applicable fixes and stable codes attacks exactly that.
5. Top proposals (section 6): `oxi` single binary (check/fmt/run/test/flash/mcp), VS Code extension + LSP (8.S2), browser/CLI simulator, MCP server + AGENTS.md, `create-oxi`, law test runner, prebuilt Rust-crate catalogue. First-15-minutes walkthrough in section 6.9.

---

## 1. What TS developers and similar communities love

### 1.1 TypeScript / JS ecosystem
- **TypeScript has won mainstream adoption.** State of JS 2025: about 40% write exclusively TypeScript [S, aggregator]; the closing essay frames "TypeScript has won" [S, ithome.com.tw]. Sample sizes are reported inconsistently (12k vs 25k) [S]. Sources: [InfoQ](https://www.infoq.com/news/2026/03/state-of-js-survey-2025), [2025.stateofjs.com](https://2025.stateofjs.com/en-US/usage/).
- **Tools that are loved are fast, single-binary, near-zero-config:** Vite is the most loved library in State of JS 2025, within ~2 points of overtaking webpack in usage with a ~78-point satisfaction advantage [S]; Vitest rising while Jest satisfaction declines [S] ([strapi summary](https://strapi.io/blog/state-of-javascript-2025-key-takeaways), [heise](https://heise.de/-11171168)). Biome replaces ESLint+Prettier in one binary (claims range 25x-100x faster; benchmarks are third-party and inconsistent [S]) ([flaviocopes](https://flaviocopes.com/biome/), [betterstack](https://betterstack.com/community/guides/scaling-nodejs/biome-eslint/)).
- **Spirit:** "it just runs after install", watch mode that does not lie, one tool per job with sane defaults. [O]

### 1.2 Rust
- Rust Survey 2025: users want even more Rust; most-wanted features are generic const expressions and improved trait methods; let-chains and async closures (stabilised 2025) are "very popular and often used"; the Zed editor jumped notably, and editors with agent support are eroding VS Code/IntelliJ share [V: [blog.rust-lang.org](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results/), 7,156 responses].
- Loved: error messages that explain with a stable code and `rustc --explain` [M, https://doc.rust-lang.org/error_codes/error-index.html], `cargo` as the one entry point [M], rust-analyzer, `rustfmt` as non-negotiable [M].

### 1.3 Gleam, Zig, Go, Deno/Bun (spirit)
- Gleam: Stack Overflow 2025 admiration 70% (vs Rust 72%, Elixir 66%, Zig 64%), attributed to small language and minimal syntax [S, [byteiota](https://byteiota.com/gleam-hits-70-developer-admiration-stack-overflow-survey/)]. The Gleam team's 2024 survey focus was the language server and called it "a tremendous success" [S, [gleam.run](https://gleam.run/news/developer-survey-2024-results)]. One binary does build/format/LSP/test [M].
- Go: gofmt-ends-arguments, `go test` built in, fast compile [M]. Deno: `deno lint` and `deno fmt` ship in the binary with "no packages to install, no config files required" [V: [docs.deno.com](https://docs.deno.com/runtime/fundamentals/linting_and_formatting/)]; `deno check`/`deno test` zero-config and a lint JSON option were not confirmed there [unverified]. Bun [M].
- Elm/Roc style: compiler messages written as prose with a suggested next step [M].

### 1.4 What these have in common (design rules we should copy)
1. One executable, one command vocabulary. 2. Formatter has no options. 3. Check is fast enough to run on every keystroke. 4. Errors carry stable codes, locations, and a next step. 5. Tests and docs are built in. 6. The editor experience ships with the language, not as a community afterthought.

---

## 2. What they miss or waited too long for

### 2.1 TypeScript
Most-upvoted *open* TypeScript issues (GitHub reaction sort; reaction counts were not shown by the fetch, the numbers below are comment-style counts shown on the page and must not be read as upvotes) [V: [issues sorted by +1](https://github.com/microsoft/TypeScript/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)]:
- #19573 `@ts-ignore` for block scope and imports; #32063 `import ConstJson from './config.json' as const`; **#12936 Exact Types**; #16607 compiler plugins; #14833 "type system is Turing complete" (not a wish); **#4881 class decorator mutation**; #14419 custom transformers plugin support; #1213 classes parametric in parametric classes; #27379 tsconfig with input files; #26242 partial type argument inference.
- Reading: top wishes are about **soundness/precision** (exact types, inference), **extensibility** (plugins) and **config friction** (tsconfig vs CLI args). Several of these are things OxiScript changes by construction (no tsconfig, nominal structs, closed languages) [O].

Recurring complaints [M unless stated; each needs a citation pass before use in public text]:
- No runtime type safety: types erased; `as`, `any`, index access and `JSON.parse` lie. OxiScript: G §6 #3, #12.
- One `number`: no integer type, no 64-bit ints without BigInt. OxiScript: G §6 #1, #17.
- Exceptions untyped: `catch (e: unknown)`; no Result idiom in the language. OxiScript: G §6 #6.
- `enum` oddities, decorators saga (experimental decorators vs TC39 standard decorators in TS 5.0), pattern matching still not in JS (TC39 proposal stage unchecked).
- Config sprawl: tsconfig + eslint + prettier + bundler + test runner; ESM/CJS interop; source-map debugging; flaky hot reload.
- Speed: addressed by TypeScript 7.0 (native `tsgo`, stable 2026-07-08; the 10x claim is Microsoft's, independent coverage reports 8x-12x full-build) [S: [devblog](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/), [InfoWorld](https://www.infoworld.com/article/4196378/go-based-typescript-7-0-arrives.html), [The Register](https://www.theregister.com/devops/speedier-type-checks-in-typescript-70-as-first-stable-go-release-ships/5268828)]. Implication: our edge is not "faster than tsc" any more.
- Embedded/hardware story: essentially absent from TS itself; see section 4.
- Dates: State of JS 2025 write-ups list poor date handling among gripes [S: [daily.dev](https://app.daily.dev/posts/javascript-survey-reveals-gripes-against-date-handling-webpack-and-next-js---and-that-typescript-h-ymgpw8afr)]. Relevant: `jiff` is on our 8.N list.
- State of JS 2025 open-ended pain points: complexity 435 mentions, performance 239, state management 217, choice overload 215 (3,748 respondents; the tally is about React, not TS) [S]. "Choice overload" is the config-sprawl complaint.

### 2.2 Rust
- Compile times and storage usage remain a top non-trivial problem; debugging slipped from 2nd to 4th; concern that the language keeps getting more complex [V: Rust survey above]. Async pain [M].
- Lesson: complexity concern is the cost of a feature-rich type system; OxiScript should keep the MVP small (G §5 freezes shapes instead of adding features) [I].

### 2.3 AssemblyScript (closest "TS-like to Wasm" precedent)
- Still has no closures; waiting for Wasm function-references + GC so captured environments can be collected; (third-party compiler rasc calls it upstream's oldest open request [S]; not confirmed by AS). Status page [S]: [status](https://assemblyscript.org/status.html). Issue #173 (2018, closed by its reporter) records the maintainer saying closures "can't capture yet" because they need GC support to attach the outer scope [V: [#173](https://github.com/AssemblyScript/assemblyscript/issues/173)]; #798 is the later tracking request [S]. Our research already records "no closures, no iterators, limited unions, own GC runtime" [I: oxiscript-language.md line 82] and wasmi lacking GC/EH/threads [I: line 38].
- Why people leave (from the above): a "TypeScript-looking" language that cannot run ordinary TS idioms (closures, union narrowing) breaks the persona's promise of *reading* it. We lower closures ourselves (RC + `call_indirect`) [I]. Keep that a launch-blocker.

### 2.4 Go, Kotlin, Swift, Zig, MoonBit, Roc
- Go added generics in 1.18, which the release notes call "major - but fully backward-compatible" changes [V: [go1.18](https://go.dev/doc/go1.18)]; that it took ~13 years is [M]. Kotlin: K2 (Kotlin 2.0, 2024-05-21) was marketed as up to 2x faster compilation, i.e. compile speed was the pain to fix [S: [JetBrains blog](https://blog.jetbrains.com/kotlin/2024/05/celebrating-kotlin-2-0-fast-smart-and-multiplatform/), [benchmarks](https://blog.jetbrains.com/kotlin/2024/04/k2-compiler-performance-benchmarks-and-how-to-measure-them-on-your-projects/)]. Swift: "unable to type-check this expression in reasonable time" is a years-long complaint (Swift 4.2 to 6, often SwiftUI/result builders) whose message can mask the real error; one 2024 post reports 42 s on 12 lines [S: [Apple forums](https://developer.apple.com/forums/thread/771265), [Daniel Hooper](https://danielchasehooper.com/posts/why-swift-is-slow/)]. Lesson for 8.T: bidirectional local inference must have a hard budget and an error that names the offending sub-expression, never a timeout message [O]. None researched in depth this session; one citation each is still outstanding. The common thread: the pain that stays longest is the one that needs a language-level change after launch. OxiScript's "frozen reserved words" approach (G §1 reserved list) addresses this for `async`, `match`, `trait`, etc. [I].

---

## 3. What AI coding agents need from a language and toolchain

Evidence base:
- **Developers do not trust the output.** Stack Overflow 2025: 66% cite "almost right, but not quite"; 46% distrust accuracy (31% in 2024); 87% worry about agent accuracy; 75% still ask a human when unsure [S: [stackoverflow.blog 2025/07/29](https://stackoverflow.blog/2025/07/29/developers-remain-willing-but-reluctant-to-use-ai-the-2025-developer-survey-results-are-here/), [VentureBeat](https://venturebeat.com/ai/stack-overflow-data-reveals-the-hidden-productivity-tax-of-almost-right-ai-code)]. Another outlet gives 33% trust and Stack Overflow's blog 29% (different framings; unresolved) [S].
- **Agents need a closed loop of: fast structured check -> fix -> re-check -> test.** gopls' own MCP instructions: after every code change call `go_diagnostics` with the edited paths, fix and re-run, run tests only when clean; tools named on the instructions page (master): `go_workspace, go_vulncheck, go_search, go_file_context, go_package_api, go_symbol_references, go_diagnostics` (the v0.20.0 list from search differs slightly; the set evolves; marked experimental [S]) [V for the rule and master tool list: [instructions.md](https://go.googlesource.com/tools.git/+/refs/heads/master/gopls/internal/mcp/instructions.md)]. Note a separate go.dev blog URL I tried returned 404 [V], so cite the googlesource file.
- **Svelte:** MCP server (`npx -y @sveltejs/mcp`, remote `https://mcp.svelte.dev/mcp`) with `list-sections`, `get-documentation`, and `svelte-autofixer` (static analysis of generated code, intended to be run in an agent loop until clean); the same autofixer as a CLI; recommended snippet for AGENTS.md/CLAUDE.md/GEMINI.md; `llms.txt` was the earlier mechanism (added Dec 2024) [S: [svelte.dev/docs/ai/mcp](https://svelte.dev/docs/ai/mcp), [tools](https://svelte.dev/docs/ai/tools)].
- **Biome:** `--reporter=json` (since 1.7.0), safe fixes with `--write` [S: [betterstack](https://betterstack.com/community/guides/scaling-nodejs/biome-eslint/)].
- Rust/cargo: `--message-format=json` emits `compiler-message` objects embedding rustc's JSON diagnostics [V: [cargo external tools](https://doc.rust-lang.org/cargo/reference/external-tools.html)]; the page does not mention suggestion applicability (`MachineApplicable`), that is [M] from the rustc JSON docs; stable error codes with `--explain` [M].  rust-analyzer as the reference LSP [M]. Agents.md convention [M, https://agents.md] and llms.txt [M, https://llmstxt.org].
- Rust survey: editors with agentic support are rising and eroding VS Code/IntelliJ [V]. So LSP-only is not enough: the agent calls CLI/MCP, not the editor.

Requirements distilled [O, grounded in the above]:
1. `oxi check --json`: stable `OXS-Exxxx` codes (we already have them: G §6 mentions OXS-E0005, E0900) [I], byte spans, a one-line cause, and **machine-applicable fixes** (applicability levels as rustc does [M]).
2. `oxi explain OXS-E0042` prints a short, example-driven page; same text via MCP.
3. Deterministic formatter with no options (8.P) so agent diffs are minimal.
4. Spec small enough for context: a single `llms.txt`/`AGENTS.md` of < ~6k tokens plus the grammar.md deviations table; this is the TS-vs-OxiScript delta, which is all a model needs since it already knows TS [O].
5. Test runner with JSON/JUnit output and **counterexample shrinking** printed as a reproducible one-liner.
6. Sandboxed run: capabilities mean `oxi run` cannot touch files/network unless granted (Principle 6) [I]; an agent can run generated code safely; fuel limits stop infinite loops.
7. Placement report and bug shapes as *structured* output: agents can read "OXS-P0003 not vectorised: reason X" and act.
8. Measurable: 8.S5 already plans "compile errors per 100 lines on the agent's first try" [I]; make it a CI metric.

---

## 4. Hardware and embedded developer experience for web developers

| Tool | Why people like it | Why it hurts / ended | Lesson |
|---|---|---|---|
| CircuitPython | Board appears as a USB drive `CIRCUITPY`; saving `code.py` restarts the program; no software installs besides a text editor; REPL is available; safe mode recovers a bad board [S: [Adafruit CIRCUITPY drive](https://learn.adafruit.com/adafruit-metro-rp2350/the-circuitpy-drive), [docs](https://docs.circuitpython.org/en/9.2.x/README.html)] | REPL is a fresh VM (cannot inspect `code.py` state); auto-reload is disabled while REPL is active [S] | The magic is "save = it runs". Anything slower than that feels broken. Copy the *loop*, add state preservation across reload (our instance swap, Principle 8) [I]. |
| DeviceScript (Microsoft Research) | "TypeScript for tiny IoT devices"; custom VM bytecode; VS Code + debugger folders; ESP32/RP2040 [V: [repo](https://github.com/microsoft/devicescript)] | **Repository archived by owner 2025-12-08, no reason stated** [V]. (I did not find an official explanation; do not infer one.) | Research-lab projects stall without an owner [I: oxiscript-language.md line 549]. Needs a named maintainer and a path from toy to product. |
| Wokwi | Browser/VS Code ESP32 simulator; GDB debugging with breakpoints; Wi-Fi simulation; virtual logic analyser; `wokwi-cli` runs a simulation in CI with timeout, expected-serial-text checks, serial logs and screenshots; ESP-IDF pytest-embedded integration [S: [Espressif docs](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/third-party-tools/wokwi.html)] | Espressif itself warns simulation may differ from hardware [S]; commercial service/token for CI [S] | A simulator plus CI assertions makes "no hardware yet" workable. We can build the weaker but local and free version: our core already runs in wasm32 (7.1) [I]. |
| Toit / Jaguar | `jag` CLI updates and restarts ESP32 apps over WiFi on save, README claims <2 s; VS Code extension + language server [I: oxiscript-language.md line 534, V there] | LGPL VM; own non-JS language; small ecosystem [I] | Reference for live-reload latency target; cannot be a dependency (licence). |
| Moddable / XS | Desktop simulator, `xsbug` source-level debugger, full ES2024 [I: line 532] | Heavy toolchain setup; GPL/LGPL mix [I] | Debugger + simulator expectations. |
| Jaculus, Espruino, Low.js | Jaculus demonstrates demand for JS/TS on ESP32; Espruino Web IDE runs in the browser with no install, works offline after load, uses Web Bluetooth/Web Serial (Chrome/Edge/Opera) [V: [espruino.com/Web+IDE](https://www.espruino.com/Web+IDE)]; on-device REPL [M] | Jaculus GPL; Low.js (`neonious/lowjs`): a Node.js port for ESP32-WROVER and Linux; core under a permissive licence but **the ESP32 adaptation is not open source**; 432 commits, no releases, no archived flag, last-activity date not visible [V: [neonious/lowjs](https://github.com/neonious/lowjs)]. Lesson: proprietary device layer limits community uptake [O] | JS-on-MCU keeps getting re-invented: the persona exists. |
| Arduino IDE 2 / PlatformIO | Huge library and board catalogue; one-click upload [M] | C++ errors, library manager version conflicts, slow compile/upload loop [M, unchecked] | The catalogue is the moat; our Rust-crate prebuilt set (8.N) is the substitute. |
| MicroPython | REPL-first, ESP32 supported, huge tutorial base [M] | Interpreter speed; no types | Python proves REPL + USB workflow is what hobbyists want; types are optional pain for them, essential for us. |

Not done: PlatformIO pain data, Arduino IDE 2 changelog, MicroPython WebREPL/mpremote, JetBrains Developer Ecosystem survey, Zig/MoonBit tooling.

---

## 5. What already distinguishes OxiScript for this persona (and where we are worse than TS)

### 5.1 Pain -> OxiScript feature (honest)
| Pain | OxiScript answer | Grammar/plan ref | Status |
|---|---|---|---|
| Types lie at runtime (`as`, `any`, `JSON.parse`) | no `any`, checked downcast, no unchecked casts | G §6 #3, #12 | shipped (8.T) |
| One `number` type, silent overflow | `i32/u8/f64...` with suffixes; overflow traps | G §6 #1, #17 | shipped (8.T/8.L; see the roadmap) |
| Untyped exceptions | `Result<T,E>` + `?`; traps for bugs | G §6 #6 | shipped (8.T/8.L; see the roadmap) |
| `undefined` vs `null` | one absence value | G §6 #4 | shipped (8.T/8.L; see the roadmap) |
| Switch fall-through, non-exhaustive switch | exhaustive, no fall-through | G §6 #7 | shipped (8.T/8.L; see the roadmap) |
| `arr[i]` out of range gives `undefined` | traps `bounds` | G §6 #19 | shipped (8.T/8.L; see the roadmap) |
| Config sprawl | single toolchain, manifest, built-in formatter/LSP | oxi-ui-plan-history.md Principle 10 | shipped (`oxi` CLI with fmt, 8.P `13b5987`, 8.D0); LSP is 8.S2 |
| Hidden slowness | placement report + `@expect` | Principle 5, 8.S4 | planned |
| Hardware access from JS | capabilities; wasm; ESP32 via wasm->C | Principles 6, 7, 8.S3 | blocked on board (after 2026-12-10) |
| Missing libraries | `rust:` linked crates | G §2.7, 8.N | planned |
| Tests that find real bugs | `law` property tests | G §2.6, 8.S4 | planned |
| Edit-run loop | hot reload by instance swap | Principle 8, 8.S1/8.S2 | planned |

### 5.2 Where we are worse than TypeScript (do not hide these in the docs)
- **No npm.** A TS developer reaches for a package for everything. The manifest is `package.json`-like but "without promising that npm packages run" [I]. Mitigation is 8.N (Rust crates) and 8.S5 ports, i.e. a smaller, different catalogue.
- **No `extends`** (G §6 #11) and **no anonymous object literals** (G §6 #18): common TS idioms need rewriting. **No `any`/`typeof`/`instanceof`** (G §6 #12). `x as T` changes meaning (G §6 #3).
- **Mandatory `;`** (G §6 #13) is friction for people who write semicolon-free TS; an AI agent or formatter hides it. Cheap fix: the LSP offers "insert missing `;`" as a quickfix and `oxi fmt` repairs it.
- **`str.length` / `str[i]` semantics** differ (UTF-8) (G §6 #20): a classic silent port bug; needs a dedicated diagnostic with a fix, not just a rename.
- **Young tooling:** no debugger, no source maps story, LSP not yet built (8.S2). TS has a decade of editor polish and TS7 is now fast.
- **No runtime like a browser/Node ecosystem:** no DOM, no `fetch`; all via capabilities (more explicit but more to learn).
- **Heavy features of the language promise** (laws, placement, shapes, profiles) can overwhelm a toy builder: they must be invisible until opted in. [O]

---

## 6. Proposal: tools and automations to build

Effort: S = days, M = weeks, L = months (single agent-team estimates, [O]). "Reuse" lists permissive OSS only; each needs the usual adoption check (licence, size, wasm32 build).

### 6.1 Priority table
| # | Tool | User value | Effort | Depends on | Reuse |
|---|---|---|---|---|---|
| 1 | `oxi` single CLI: `check --json`, `fmt`, `run`, `test`, `explain`, `new`, `flash`, `mcp`, `doctor` | One entry point, scriptable by agents | M | 8.P, 8.T, 8.L | `clap`, `ariadne`/`codespan-reporting` |
| 2 | JSON diagnostics with stable codes and machine-applicable fixes + `oxi explain` | Agents and editors share one truth | S-M | 8.T (diagnostics exist) | LSP `Diagnostic`/`CodeAction` shapes; rustc applicability levels as model [M] |
| 3 | VS Code extension (TextMate grammar, semantic tokens, LSP client, commands) | "Feels like TS" on day one | M | 8.S2 | `lsp-server`, `lsp-types`, `vscode-languageclient`; TextMate from existing TS grammar patterns (check licence) |
| 4 | LSP features: diagnostics, hover, go-to-def, rename, inlay hints (placement report, inferred integer types), code actions (insert `;`, `==`->`===`, `.length`->`.byteLength`), law CodeLens ("run law", pass/fail count) | The editor teaches the deltas | L | 8.T, 8.S2, 8.S4 | `salsa` evaluate |
| 5 | Live reload to simulator and device (`oxi dev`) | Save -> run under 1-2 s, state kept | M (sim) / L (device) | 8.R, 8.S1, 8.S3 | instance swap exists in 8.R; Toit/Jaguar latency as target (reference only, LGPL) |
| 6 | Browser simulator (core in wasm32; OxiUI canvas + virtual GPIO/sensors panel) | Toy works before the board arrives | M | 7.1 (done), 8.S1 | existing wasm32 core; Wokwi as UX reference (not code) |
| 7 | MCP server `oxi mcp`: tools `check`, `format`, `compile`, `run` (sandboxed, fuel-limited), `explain_code`, `apply_fix`, `report` (placement), `test_laws`, `docs_search` | Closes the agent loop | M | 1, 2, 8.S4 | `rmcp` (official Rust MCP SDK, tokio) [V: [rust-sdk](https://github.com/modelcontextprotocol/rust-sdk); licence not stated on the page, check LICENSE]; pattern from gopls/Svelte [S] |
| 8 | AGENTS.md + skills + `llms.txt` shipped by `create-oxi` and at the docs root | Agents write idiomatic OxiScript first try | S | grammar.md | llmstxt.org / agents.md formats [M] |
| 9 | `create-oxi` scaffolder (`npm create oxi`/`cargo`/`curl`) with templates: blink, touch-button, sensor dashboard, OxiUI page | Working project in one command | S | 1 | `cargo-generate` style [M] |
| 10 | Law test runner `oxi test` (property tests, shrinking, seeds, JSON/JUnit) | Tests are the first-class citizen | M | 8.S4 | `proptest`/`quickcheck` ideas for shrinking [M] |
| 11 | Rust-crate catalogue (`oxi add rust:regex`) with prebuilt wasm bindings, a build notice, and an `oxi doctor` for crate eligibility | Libraries without writing glue | L | 8.N | `wasm-ld`/`walrus`, `syn`/`quote` |
| 12 | Debugger: source maps via DWARF-in-Wasm or custom line tables; DAP adapter stepping in wasmi; "log on save" fallback first | Persona expects breakpoints | L | 8.R, 8.G | `wasmi` has no debugger [M, unchecked]; Wasm DWARF + `gimli` [M] |
| 13 | CI recipes: GitHub Action running `oxi check/test` + simulator screenshot assertions | Quality for toys that grow | S | 1, 6 | `wokwi-cli` pattern [S] |
| 14 | "Port from TS" assistant: `oxi migrate file.ts` (static report of deltas: numbers, `as`, exceptions) and agent prompt | Lowers switching cost from the biggest lock-in (existing TS) | M | 8.S5 | `swc`/`oxc` parser (Apache/MIT) to read TS [M, check] |
| 15 | Language issue logger (`oxi report-friction`) feeding 8.S5 | Real user friction becomes ranked proposals | S | 8.S5 | — |

### 6.2 Constraints for the VS Code extension (spell out)
- **Syntax:** TextMate grammar for instant colouring + LSP semantic tokens for correct integer-suffix, struct vs class, `law` contextual keyword, `?` whitespace rule (G §1.5).
- **Inline placement report:** inlay hints per function/loop with reason code (`OXS-P####`); toggle; shows `@expect` failures as errors.
- **Law results:** CodeLens above each `law` with last run result and shrunk counterexample; "run on save" with a time budget.
- **Live reload:** status bar item `oxi: live (sim) / live (board)`; failures keep the old instance running (Principle 8) and show the error inline.
- **Debugger story (honest):** v0 is `print`/trace panel + structured logs + `oxi run --trace`; v1 DAP over wasmi with line tables. Do not promise breakpoints on device until 8.S3 exists.
- **Devices:** a "Flash" command wrapping `oxi flash`; port auto-detect (WebSerial in browser) [M].

### 6.3 Agent integration detail
- MCP tool names mirror CLI subcommands; every result is JSON with `{ok, diagnostics[], fixes[], report?}`.
- `apply_fix` takes a diagnostic id and returns a patch, never writes silently; agent decides.
- `run` is capability-scoped by default (no file, network, clock) and fuel-limited; returns stdout, trap reason, fuel used.
- Instruction file mirrors the gopls rule: "after every edit, call `check`; run `test` only when clean" [S].
- Ship `AGENTS.md` with: grammar delta table (G §6), numeric rules, the `Result`/`?` idiom, capability model, forbidden words.

### 6.4 Rust crates for 8.N: ranking for the persona
**Hard filter (unchecked for every row below; do the check in 8.N):** crate must build for `wasm32-unknown-unknown` **without** `wasm-bindgen`, `js-sys`, `web-sys`, or `getrandom`'s JS backend (the linked module runs in wasmi with no JS host); no threads; no `std::fs`/sockets/OS clocks (these become capabilities, G §2.7) [I]. Likely troublemakers: `rand` and `uuid` (need a randomness capability shim), `chrono` default features (clock), `reqwest`-style network crates. Embedded sensor drivers written against `embedded-hal` traits need a capability shim (I2C/SPI/GPIO traits mapped onto OxiScript capabilities), so they are not drop-in. Licence check per crate (all below are believed MIT/Apache-2.0 or dual, **[M] unverified**, GPL excluded).

Ranked list (value for JS/TS dev + kid + ESP32 toys):
1. `serde_json` (JSON is the lingua franca; JSON.parse replacement with typed errors) [in 8.N gate list]
2. `jiff` (dates/time zones, the complaint about JS dates) [in 8.N gate list; clock = capability]
3. `regex` [in gate list]
4. `glam` (2D/3D maths for games and graphics) [in gate list]
5. `miniz_oxide` (zlib/deflate) [in gate list]
6. `image` (decode PNG/JPEG/GIF for UI assets; large, check size on ESP32) [in gate list]
7. `base64`, `hex` (tiny, ubiquitous)
8. `fastrand` or `oorandom` (small RNG without getrandom; a better first choice than `rand`) [M]
9. `unicode-segmentation` (string iteration by grapheme: answers G §6 #20) [M]
10. `smallvec`/`heapless` (no-alloc containers, useful for the `realtime` profile) [M]
11. `csv` (data logging) [M]
12. `toml` / `serde` ecosystem (config) [M]
13. `semver`, `url` (parsing) [M]
14. `rapier2d` (game physics; heavy, may exceed ESP32; browser/server first) [M]
15. `palette` or `colorsys` (colour maths for LEDs and UIs) [M]
16. `fontdue` or `ab_glyph` (text rasterisation; relevant to OxiUI) [M]
17. `qrcode` (toy-friendly, visible result) [M]
18. `sha2`, `blake3`, `hmac` (hashing; needs no OS entropy) [M]
19. `nom`/`winnow` (parser combinators, protocol parsing) [M]
20. `embedded-graphics` + display drivers, and sensor drivers (BME280, MPU6050) via `embedded-hal` shim (ESP32 route, after 8.S3) [M]

Also in 8.S5, prefer porting tiny libraries (e.g. `base64`, `hex`, colour maths) to OxiScript so effects/laws/bug shapes cover them (G §2.7 table) [I].

### 6.5 Build order suggestion
1. With 8.L done: `oxi check --json`, `oxi fmt`, `oxi run`, `oxi explain`, AGENTS.md/llms.txt (items 1, 2, 8). Tiny cost, immediate agent value.
2. With 8.S1/8.S2: VS Code extension v0, `oxi dev` against the simulator, MCP v0 (check/format/run/explain), `create-oxi`.
3. With 8.S4: law runner, placement-report hints, MCP `report`/`test_laws`.
4. With 8.N: crate catalogue + `oxi doctor`.
5. With hardware (after 2026-12-10): `oxi flash`, device live reload, board templates.
6. Later: debugger, `oxi migrate`.

### 6.6 What to measure (so the DX does not rot)
- Edit -> visible effect latency in sim (target < 300 ms) and on device (target < 2 s, Toit's claim as reference [I]).
- Time from `create-oxi` to first running toy (target < 5 min on a clean machine).
- Agent first-try compile errors per 100 lines (8.S5 metric) [I], and "errors fixed by applying the suggested fix without edits".
- Diagnostics with a machine-applicable fix: share of all diagnostics (target > 50% for the common-delta set: `==`, `;`, `.length`, `as`).

### 6.7 What not to build (yet)
- A package registry (use git URLs + `rust:` first). A Jupyter-style notebook. A WebSerial IDE of our own beyond the simulator. Do not clone Wokwi's component library.

### 6.8 Licensing caveat
Everything above assumes permissive licences only (oxi-ui-plan-history.md Principle 11) [I]; Toit, XS and Jaculus are references, not dependencies.

### 6.9 First 15 minutes: the ideal walkthrough
| Min | What the persona does | What happens |
|---|---|---|
| 0:00 | Opens a terminal, runs one install line | `oxi` binary (no Node, no Rust required), `oxi doctor` green |
| 1:00 | `oxi new blinky --template touch-button` | Project with `main.oxs`, `oxi.toml`, `AGENTS.md`, VS Code recommendations; no tsconfig/eslint/prettier |
| 2:00 | `code .` | Extension auto-suggested; colours, hover show `i32`; no config prompt |
| 3:00 | Edit button color; save | Browser simulator updates in < 300 ms, state kept |
| 5:00 | Mistypes `a == b`, writes `x: number` | Squiggles with `OXS-E0005`/numeric type quickfix "use f64 or i32"; one click fixes |
| 7:00 | Asks agent to add a counter that saves to flash | Agent reads AGENTS.md, calls MCP `check`, fixes `Result` handling, loops until clean |
| 9:00 | Adds `law counter_never_negative` | CodeLens runs 1,000 cases; shrunk counterexample appears if it fails |
| 11:00 | Hover over a loop | Inlay: "scalar: `OXS-P0xxx` reason"; suggests `@expect(simd)` only if applicable |
| 12:00 | Plugs ESP32 (after 2026-12-10 hardware) | `oxi flash` auto-detects the port, flashes, prints "running"; save -> device updates in < 2 s |
| 14:00 | `oxi add rust:jiff` | Notice lists crate, version, `unsafe` yes/no, dropped guarantees; clock handed in as a capability |
| 15:00 | Shows the kid | Toy works; commit; CI action already scaffolded |

---

## 7. Open questions for the owner
1. Is a free, local browser simulator (virtual GPIO/touch/sensors) acceptable as the default instead of integrating Wokwi? (Wokwi CI use needs a token [S].)
2. Do we ship the MCP server inside `oxi` (one binary) or as a separate package? Is a hosted remote MCP endpoint wanted (as Svelte has) [S]?
3. Debugger: accept "logs + traces" until after 8.S3, or fund DAP earlier?
4. Which editors beyond VS Code at launch (Zed is rising per Rust survey [V]; Neovim/Helix get LSP for free)?
5. Semicolons: stay mandatory (G §6 #13) given persona friction, with a fix-on-save formatter as mitigation?
6. Is the package story "git URLs + `rust:` + `std-preview`" enough for 1.0, or do we need a registry earlier?
7. Which prebuilt Rust bindings must be size-checked against ESP32 flash first (`image`, `regex` are large)? Should the catalogue publish a size/feature table?
8. Telemetry: opt-in friction reporting (`oxi report-friction`) acceptable?
9. Name/owner for the DX program: DeviceScript's archive suggests an explicit long-term maintainer matters [V/I].
10. Should `oxi migrate` (TS delta report) be an early marketing feature given the persona already has TS code?

---

## Sources (fetched or surfaced this session)
- [State of JS 2025 coverage, InfoQ](https://www.infoq.com/news/2026/03/state-of-js-survey-2025) [S]; [2025.stateofjs.com](https://2025.stateofjs.com/en-US/usage/) [S]; [strapi key takeaways](https://strapi.io/blog/state-of-javascript-2025-key-takeaways) [S]; [heise](https://heise.de/-11171168) [S]
- [Rust Survey 2025 results](https://blog.rust-lang.org/2026/03/02/2025-State-Of-Rust-Survey-results/) [V]
- [TypeScript issues by reactions](https://github.com/microsoft/TypeScript/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc) [V]; [TS 7.0 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/) [S]; [InfoWorld](https://www.infoworld.com/article/4196378/go-based-typescript-7-0-arrives.html) [S]; [The Register](https://www.theregister.com/devops/speedier-type-checks-in-typescript-70-as-first-stable-go-release-ships/5268828) [S]
- [DeviceScript repo (archived)](https://github.com/microsoft/devicescript) [V]
- [AssemblyScript status](https://assemblyscript.org/status.html) [S]; [issue 798](https://redirect.github.com/AssemblyScript/assemblyscript/issues/798) [S]
- [Wokwi in ESP-IDF docs](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/third-party-tools/wokwi.html) [S]
- [CircuitPython CIRCUITPY drive](https://learn.adafruit.com/adafruit-metro-rp2350/the-circuitpy-drive) [S]; [CircuitPython docs](https://docs.circuitpython.org/en/9.2.x/README.html) [S]
- [gopls MCP instructions](https://go.googlesource.com/tools.git/+/refs/heads/master/gopls/internal/mcp/instructions.md) [S]
- [Svelte AI docs: MCP](https://svelte.dev/docs/ai/mcp) [S]; [tools](https://svelte.dev/docs/ai/tools) [S]
- [Stack Overflow 2025 survey blog](https://stackoverflow.blog/2025/07/29/developers-remain-willing-but-reluctant-to-use-ai-the-2025-developer-survey-results-are-here/) [S]; [VentureBeat](https://venturebeat.com/ai/stack-overflow-data-reveals-the-hidden-productivity-tax-of-almost-right-ai-code) [S]
- [Gleam 2024 survey results](https://gleam.run/news/developer-survey-2024-results) [S]; [byteiota Gleam 70%](https://byteiota.com/gleam-hits-70-developer-admiration-stack-overflow-survey/) [S]
- [Biome guides](https://betterstack.com/community/guides/scaling-nodejs/biome-eslint/) [S]; [flaviocopes](https://flaviocopes.com/biome/) [S]
- Internal: `docs/history/oxi-ui-plan-history.md` Phase 8, `docs/oxiscript/grammar.md`, `docs/research/oxiscript-language.md` [I]

## Known gaps in this report
Zig/Roc/MoonBit/Bun claims are [M] and need citations (Go 1.18 and Deno lint/fmt now [V]); TS issue reaction counts not captured; Espruino, Low.js, PlatformIO, Arduino IDE 2 not researched; Rust crate licences and wasm32 eligibility not checked; `rmcp` licence not read; cargo suggestion-applicability fields not confirmed from the rustc JSON doc.
