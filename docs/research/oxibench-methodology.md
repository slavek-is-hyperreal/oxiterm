# OxiBench: methodology (v0.1, draft for owner approval)

Status: written by the lead 2026-11-25 at the owner's request. **Nothing is run until the owner approves this file and the freeze in section 7 is committed.**

Question OxiBench answers: *how well do today's AI models write correct OxiScript, with and without our documentation and tool loop, compared with the same tasks in TypeScript, and which OxiScript features cause their mistakes?*

What it does **not** answer: whether OxiScript is pleasant for humans, whether it is a good language design, or how models behave in long multi-file projects. Those need other studies (8.S5 Shakedown, user feedback).

---

## 1. Principle: nobody grades their own work

The owner's rule: the agent that creates OxiBench must not be the one that executes it. We go further and separate five roles. No role may be played by the same agent instance as another, and the roles that judge correctness are not agents at all.

| Role | Who | Sees | Must not see / do |
|---|---|---|---|
| **R1 Task author** | Agent instance A (plus seed tasks written by the owner) | the spec, the task template | subject outputs, model results; must not run any subject model |
| **R2 Task auditor** | Agent instance B, a different instance with a different prompt; **a different model family when available** | tasks, reference solutions, hidden tests | subject outputs; must not edit tasks (only accept/reject with reasons; A revises) |
| **R3 Harness** | A deterministic program (not an agent), committed and hashed before the run | everything it executes | must not use any model to decide pass/fail |
| **R4 Subjects** | The models under test | only the task prompt, the documents allowed by the condition, and tool results | hidden tests, reference solutions, other subjects' outputs |
| **R5 Analyst** | A script computes all numbers. An agent instance C may write the prose of the report **only from the frozen result tables** | results | must not rerun, drop, reweight or re-grade anything |

The lead (Opus, this session) designs the methodology and integrates the results, but writes no tasks, no tests and no grading code. Its own model family is also a subject, which is a conflict of interest; sections 2.4 and 9 handle it.

## 2. Tasks

### 2.1 Task format

Each task is a directory with:
- `prompt.md`: a natural-language description, the same for every condition. It specifies behaviour, not implementation, and names the function signature to implement (e.g. `export function median(xs: [u8]): ...` or a `main(log: Log)` program).
- `ref.oxs`: an OxiScript reference solution.
- `ref.ts`: an equivalent TypeScript reference solution (the TS twin, section 4.3).
- `tests/`: **hidden** tests: input/output cases, laws with seeds, expected `host:log` output, expected traps where the spec demands them.
- `meta.toml`: category, difficulty tier (1-5), language features exercised, author, auditor verdict, and the hash of every file.

### 2.2 Coverage

At least **60 test-set tasks**, balanced across categories, at least 5 each: arithmetic and integer types, floats, strings and UTF-8, arrays and collections, structs (copy semantics), classes, unions and `switch`, `Result` and `?`, closures, generics and interfaces, laws, host capabilities, plus mixed tasks. OxiUI page tasks are added after 8.S1 as a separate category. Difficulty: tiers 1-2 at least 40% and tiers 4-5 at least 20%.

### 2.3 Quality gates for a task (checked by R2 and by the harness)

1. The reference solution passes all hidden tests (OxiScript and TS).
2. **The tests must discriminate.** The harness generates mutants of `ref.oxs` (operator swaps, off-by-one, dropped branch, wrong literal type) and the tests must kill at least **80%** of the mutants that still compile. Otherwise the task is rejected.
3. **The prompt is unambiguous.** R2 writes an independent solution from `prompt.md` alone. If R2's solution fails the tests and the failure is a prompt ambiguity rather than an R2 bug, the task goes back to R1.
4. **No leakage.** The prompt contains no solution code, and test inputs do not appear in the prompt except for at most 2 worked examples.
5. **Determinism.** Tests run under the fuel limit. Running them twice gives identical results.

### 2.4 Bias control in task creation

- The owner writes at least 10 seed tasks by hand, so the set is not purely model-shaped.
- R1 must not optimise tasks for any model. In particular it never sees model results before the freeze.
- When a second model family is available, half the tasks are authored by it, and results are reported per author family. A large gap between the two halves is reported as a threat.

## 3. Data sets and contamination

The repository is **public**, so anything committed there may end up in future training data.

| Set | Size | Where | Purpose |
|---|---|---|---|
| **Dev set** | ~20 tasks | public repo (`bench/dev/`) | improving AGENTS.md, compiler hints and fixes; may be used freely |
| **Test set** | >= 60 tasks | **not in the public repo** (owner's private storage, section 10, question 1) | only for reported numbers |

Rules:
- Docs, hints and language changes may be motivated **only by dev-set failures**. Tuning on the test set invalidates it.
- The test set is run once per toolchain release. Results are published as aggregates plus a few sample tasks retired into the dev set.
- A canary string (a random GUID) is placed in every test-set file. If a model reproduces it, the set is contaminated and is replaced.
- The test set is rotated (about a third of its tasks replaced) at least every three releases.

## 4. Conditions

All conditions use the same `prompt.md`.

| Id | Name | The subject gets |
|---|---|---|
| **C0** | Cold | One sentence: "OxiScript is a statically typed language similar to TypeScript; the compiler is `oxi`." No docs, no tools. Measures prior knowledge. |
| **C1** | Docs | C0 plus `AGENTS.md` (the version frozen with the run) in context. No tools. |
| **C2** | Docs + check loop (**primary**) | C1 plus a tool `oxi_check(source) -> JSON diagnostics` (the real `oxi check --json`), at most **3 rounds**, then a final answer. |
| **C3** | Docs + check + own run | C2 plus `oxi_run(source, stdin-like args) -> log/traps` on inputs the subject chooses (never the hidden tests), at most 3 runs. |
| **TS** | TypeScript twin | The same task in TypeScript with a `tsc --noEmit` loop of at most 3 rounds, graded by the TS twin of the hidden tests. Baseline only. |

### 4.3 Why the TS twin

Without it we cannot tell whether a failure is caused by OxiScript or by the task being hard. The headline language effect is **pass rate (OxiScript, C2) minus pass rate (TS)** per task, averaged. The TS twin uses the same hidden test cases, translated by R1 and verified by R2 (gate 2.3.1 for both languages).

## 5. Running (R3, the harness)

- **Identical interface** for every model: the same system prompt (a short fixed text, committed), the same task prompt, and tools exposed as standard function calls with identical JSON schemas.
- **Pinned versions:** the exact model ID strings, API endpoints and run dates are recorded. So are the toolchain version (commit hash of `oxi` and the checker) and the frozen `AGENTS.md` hash.
- **Sampling:** **k = 5 independent samples** per (task, model, condition) at the provider's default temperature (recorded). With 60 tasks this gives 300 samples per cell.
- **Budgets:** the same maximum output tokens, the same wall-clock timeout per sample, and the same number of tool rounds. API errors, rate limits and timeouts caused by the provider are retried up to 3 times and are **not** counted as failures; they are logged and reported.
- **Extraction:** the final answer must contain exactly one fenced `oxs` (or `ts`) block. No block, or several, counts as a failure (category "format"). The extraction rule is committed with the harness.
- **Isolation:** each sample runs in a fresh temporary directory. Hidden tests are copied in only after the subject has finished.
- **Logging:** full transcripts, tool calls, diagnostics, test outputs and timings are stored, raw and append-only.

## 6. Grading and metrics (R3 and R5 scripts only, no model judges)

### 6.1 Per-sample outcome (automatic)
- `compile_ok`: `oxi check` has no errors.
- `pass`: every hidden test passes (outputs, laws with fixed seeds, expected traps), within the fuel limit.
- Recorded but not counted as failures: live objects after `oxi_deinit` (a leak is a runtime bug, not the model's); diagnostics count per round.

### 6.2 Metrics
- **Primary:** pass@1 in C2 = the mean over tasks of (passing samples / k).
- **Secondary:**
  - pass@1 in C0, C1 and C3;
  - first-try compile rate (round 0 of C2);
  - mean check rounds until clean;
  - pass@5;
  - the TS gap (section 4.3);
  - tokens and cost per passing solution.
- **Error taxonomy, automatic:**
  - diagnostics are grouped by their stable `OXS-` code. This directly gives "TS habits that come back", e.g. `number`, `==`, exceptions, missing `;`;
  - test failures are grouped as wrong value, wrong trap, missing trap, timeout or fuel, or format.

  No model labels anything.

### 6.3 Statistics
- 95% confidence intervals by **bootstrap over tasks** (10,000 resamples; tasks are the unit, samples are nested).
- Model-to-model and condition-to-condition comparisons are **paired by task**: a paired bootstrap of the difference. A difference is called "significant" only if its interval excludes 0.
- Per-category numbers are shown with their n and are labelled exploratory. Categories with n < 5 are not claimed.
- No result is dropped. Every excluded sample (provider error after retries) is listed with its reason.

## 7. Freeze (preregistration)

Before the first subject call, one commit contains:
- this methodology, with its version;
- the hash of the test set (the content stays private);
- the harness commit;
- the list of models and conditions;
- k, the budgets and the prompts;
- the analysis script.

The owner approves this commit. Any later change creates a new methodology version, and results of different versions are never mixed in one table.

## 8. Reporting

- A report generated from the frozen tables, containing:
  - headline numbers with intervals;
  - the TS gap;
  - the top 10 error codes per model;
  - examples from the dev set only.
- The site may quote only numbers from such a report, with the version, date, model IDs and a link to the methodology.
- Negative results are published too, e.g. "C1 does not beat C0" or "TS gap of -40 points".

## 9. Threats to validity, and what we do about them

| Threat | Mitigation |
|---|---|
| The lead's model family (Claude) is also a subject; task authors are Claude instances | Owner seed tasks; the second-family author half (section 2.4); R2 from another family when possible; grading has no model in it; the conflict is disclosed in every report |
| Tasks too easy or too hard, so the measurement saturates | Tier balance; pilot on the dev set to check that pass rates are neither near 0% nor near 100% before freezing |
| Weak tests make wrong solutions pass | Mutation score >= 80% (2.3.2); laws with fixed seeds |
| Contamination of the test set | Private storage, canary, rotation |
| Overfitting docs to the benchmark | Docs tuned on the dev set only |
| Provider nondeterminism | k = 5, bootstrap, recorded temperature and dates |
| Prompt sensitivity | One fixed template, committed; a small ablation (2 paraphrases) on the dev set reported separately |
| Harness bugs | The harness is tested on the reference solutions (must pass 100%) and on mutants (must fail as expected) before every run |
| The TS twin not truly equivalent | R2 verification; tasks whose twin fails gate 2.3.1 are excluded from the TS gap |
| Results conflated with toolchain changes | Toolchain and AGENTS.md hashes are pinned per run |

## 10. Open questions for the owner

1. **Where does the private test set live?** For example a private GitHub repo, or a local directory outside the public repo with a backup.
2. **Which models, and who pays?** Claude is available here. Gemini, GPT and open models need API keys and budget. The rough cost is `models × conditions × 60 tasks × 5 samples` calls.
3. **Is C3 (own runs) in v0.1, or later?**
4. **Agent mode:** should we later add a variant where real coding agents (Claude Code, Gemini CLI, Cursor) solve the tasks in a scratch project? That measures the whole product, not only the model, but it is harder to keep fair.
5. Is the TS twin enough as a baseline, or do we also want **AssemblyScript** (the closest "TS to Wasm" language)?
