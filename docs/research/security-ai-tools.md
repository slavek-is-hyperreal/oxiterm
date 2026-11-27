# Online AI models and services for vulnerability finding: what could help our BAD ACTOR

> **Status:** completed by Claude (Sonnet 5.5, with Opus advisor) on **2026-11-27**, taking over a partial Gemini draft (commit 58498d2). Every fact below was re-checked against its source on **2026-11-27** unless it carries another date. Anything I could not confirm from a primary page is marked `[unverified]`. Section 6 lists what was removed or corrected relative to the Gemini draft.
> **Scope:** commercially available online models/services (plus open-source systems) for vulnerability finding, CVE knowledge, exploit reasoning, fuzzing and security code review, judged for OxiScript/OxiUI: a Rust code base (compiler, RC runtime, `wasmi` host) that emits WebAssembly.
> **Constraints (read in `docs/roadmap/oxiscript.md`, 2026-11-27):**
> [D-32](../roadmap/oxiscript.md#d-32): no budget for external auditors; safety work uses what the Claude Code Pro subscription allows plus tools with permissive licences. Zero-cost options rank first.
> [D-31](../roadmap/oxiscript.md#d-31): Gemini may do security audits but never as the only model; each is paired with an independent Claude review.
> [D-33](../roadmap/oxiscript.md#d-33)/[D-34](../roadmap/oxiscript.md#d-34): BAD ACTOR is Claude (Opus for large stages/releases, Sonnet + `advisor` for small); other models add passes, never replace it; changes are batched into rounds ([`docs/stages/bad-actor-rounds.md`](../stages/bad-actor-rounds.md)).
> [P-11](../roadmap/oxiscript.md#p-11): permissive licences only for anything embedded or linked. External CI services and standalone tools may have other terms.
> **Facts about our repo** (checked locally 2026-11-27): public at `github.com/slavek-is-hyperreal/oxiterm`, licence Apache-2.0 (GitHub API); CI has `cargo audit` ([`ci.yml`](../../.github/workflows/ci.yml)) and a nightly gate ([`oxiscript-fuzz.yml`](../../.github/workflows/oxiscript-fuzz.yml)); no CodeQL, Semgrep, `cargo-fuzz`/libFuzzer or `cargo deny` is wired up yet (`cargo deny` is only planned in the roadmap).

---

## 1. Executive summary and ranked table

**Main finding.** Almost every "AI security" product targets enterprise web apps or SOC work. The tools with real, independent signal for a Rust/Wasm compiler are old-fashioned: semantic static analysis (CodeQL), coverage-guided fuzzing, and deterministic rules. LLM agents find real bugs when paired with execution (fuzzers, sanitizers, a runnable reproducer) and, even then, maintainers report high triage cost (section 4). Small fine-tuned vulnerability classifiers do not generalise (PrimeVul). No service I found is both free for us and specialised for compilers or Wasm.

**Independence caveat.** `/security-review` and any other Claude pass share the BAD ACTOR's model family, so they add convenience, not an independent opinion. The independent signal at zero cost comes from CodeQL, fuzzing and Semgrep/Opengrep rules, plus optionally a non-Claude model (Gemini under D-31, or an OpenAI/Google programme if the owner is accepted).

### Ranked table (zero-cost first)

| Rank | Name | Type | Cost / free tier | Rust / Wasm | What it adds to our BAD ACTOR | Sources |
|---|---|---|---|---|---|---|
| 1 | **GitHub CodeQL (Rust, GA) + Copilot Autofix** | Semantic SAST + AI fix suggestions in PRs | **$0** on public repos: CodeQL CLI licence allows analysis of open-source code hosted on GitHub.com including CI; Autofix is free on public repos, no Copilot subscription | Rust GA since 2025-10-14 (editions 2021/2024, no nightly features). No Wasm/`.oxs` | Independent, non-Claude static analysis of `unsafe`, overflows, injection, on every PR; custom queries possible | [GA changelog](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available), [CodeQL licence](https://github.com/github/codeql-cli-binaries/blob/main/LICENSE.md), [Autofix free on public repos](https://github.blog/changelog/2024-09-17-now-available-for-free-on-all-public-repositories-copilot-autofix-for-codeql-code-scanning-alerts), [supported languages](https://codeql.github.com/docs/codeql-overview/supported-languages-and-frameworks/) |
| 2 | **`cargo-fuzz` + ClusterFuzzLite** (later OSS-Fuzz / OSS-Fuzz-Gen) | Coverage-guided fuzzing in GitHub Actions; LLM harness generation in OSS-Fuzz-Gen | **$0** (Apache-2.0; runs on GitHub runners). Google-hosted OSS-Fuzz needs "significant user base and/or critical to global IT infrastructure" | Rust listed as supported by ClusterFuzzLite. Wasm only indirectly (fuzz the parser, checker, codegen, `wasmi` host with hostile modules) | Best fit for classes 2-5 of our attack list; crashes come with reproducers, so no false-positive triage. Needs harnesses written (none exist today) | [ClusterFuzzLite](https://google.github.io/clusterfuzzlite/), [OSS-Fuzz eligibility](https://google.github.io/oss-fuzz/getting-started/accepting-new-projects/), [oss-fuzz repo](https://github.com/google/oss-fuzz), [oss-fuzz-gen](https://github.com/google/oss-fuzz-gen) |
| 3 | **Semgrep Community Edition or Opengrep** with repo-local rules | Deterministic pattern/taint SAST, runs locally | **$0**, both engines LGPL-2.1 (external tool, never embedded: P-11). Semgrep's *hosted* free tier: max 10 contributors, 60 AI credits | Semgrep lists Rust as GA with cross-function dataflow (not cross-file); the page does not split CE vs Pro and says 40+ Rust rules are Pro. No Wasm. Rust coverage of Opengrep `[unverified]` | Encodes our own invariants (illustrative examples, not taken from the roadmap: no `unwrap` on guest input, checked `ptr+len` arithmetic, no unordered-map iteration in emitted output) as fast CI rules; no code leaves the runner | [Semgrep languages](https://docs.semgrep.dev/supported-languages), [Semgrep pricing](https://semgrep.dev/pricing), [Semgrep LGPL-2.1](https://github.com/semgrep/semgrep/blob/develop/LICENSE), [Opengrep](https://github.com/opengrep/opengrep), [Opengrep vs Semgrep](https://semgrep.dev/docs/faq/comparisons/opengrep) |
| 4 | **Claude Code `/security-review`** | LLM review of the branch diff vs origin's default branch | Included in the owner's subscription (no extra cost); CI use needs API key or subscription OAuth token (see 2.1) | Language-agnostic; reads Rust and our specs | Cheap pre-round pass on every branch, especially Gemini-authored ones (D-31). **Not independent** of the BAD ACTOR | [Commands reference](https://code.claude.com/docs/en/commands), [Security docs](https://code.claude.com/docs/en/security) |
| 5 | **Socket** (free plan) | Supply-chain scanner (malicious/risky dependencies) | **$0** plan: 1 dev seat, unlimited public and private repos, 1,000 scans/mo; "always free for open-source" | Rust/Cargo: crates.io only; GA claim seen only via aggregator copies `[unverified on socket.dev]` | Watches `wasmi` and future crates for typosquats/suspicious updates, next to `cargo audit` | [Pricing](https://socket.dev/pricing), [Rust beta post](https://socket.dev/blog/rust-support-now-in-beta) |
| 6 | **Owner-applied OSS programmes** (Claude for Open Source; Codex for OSS incl. conditional Codex Security) | Free subscription / security agent for maintainers | $0 if accepted; uncertain for a young project | Language-agnostic LLM | A strong non-Gemini second opinion if granted; do not plan on it | [Claude page](https://claude.com/open-source-max), [OpenAI page](https://developers.openai.com/codex/community/codex-for-oss) |
| 7-10 | Corgea free, Snyk free/OSS programme, Aikido free, Mobb free (public repos), SonarQube Cloud free OSS plan | SaaS SAST with caps | $0 with caps (2.4) | Rust support unverified or partial per vendor | Overlap CodeQL; low priority | see 2.4 |
| not usable | Claude Security, Codex Security (paid), Glasswing/Mythos, Google CodeMender/Flash Cyber, Big Sleep, Security Copilot, Checkmarx, Veracode, XBOW, ZeroPath (no free tier), AIxCC systems (need paid LLM keys; C/Java only) | | paid, invite-only, or enterprise | | | sections 2.5-2.7 |

---

## 2. Services and models, one by one

### 2.1 Anthropic

- **`/security-review` (Claude Code).** "Analyze the changes on your current branch for security vulnerabilities. Reviews the diff between your branch and origin's default branch... Needs an `origin` remote" ([commands reference](https://code.claude.com/docs/en/commands), 2026-11-27). Customisable by copying `security-review.md` into `.claude/commands/` ([action README](https://github.com/anthropics/claude-code-security-review)). Detection and explanation only; no fuzzing; no execution unless the session runs tests.
- **Is it included in Pro?** The docs list it as a built-in command and do not gate it by plan. The consumer terms cover Free, Pro, Max and Ultra ([security docs](https://code.claude.com/docs/en/security)). It spends your normal subscription usage; I found no per-plan statement either way, so treat "no extra charge" as inferred, not quoted `[unverified wording]`.
- **`anthropics/claude-code-security-review` GitHub Action.** MIT. The README example passes an Anthropic **API key** (`claude-api-key`); it says the action "is not hardened against prompt injection attacks and should only be used to review trusted PRs" and recommends "Require approval for all external contributors" ([README](https://github.com/anthropics/claude-code-security-review), 2026-11-27). The repo was last pushed 2026-02-11 (GitHub API), so it looks unmaintained.
- **Subscription in CI (corrects the draft).** The general `anthropics/claude-code-action` supports a `claude_code_oauth_token` (from `claude setup-token`) for Pro/Max/Ultra/Team/Enterprise, and "runs use your Claude subscription instead of API billing"; the docs advise API keys for secrets shared across repositories, and note that fork PRs get no secrets on public repos ([GitHub Actions docs](https://code.claude.com/docs/en/github-actions), 2026-11-27). So CI use *can* stay inside the subscription, but it competes with interactive usage and ties the token to the owner's account. Whether this fits the consumer terms for automated use is `[unverified]`; read the Consumer Terms before doing it.
- **Data terms (consumer plans).** Since 2025-08-28 consumers choose whether chats and Claude Code sessions may be used for model improvement; allowed means 5-year retention, not allowed means the 30-day period ([Anthropic update](https://www.anthropic.com/news/updates-to-our-consumer-terms)). Our repo is public, so the exposure is low; check the toggle at claude.ai/settings/privacy.
- **Claude Security** (managed scanner, Opus 4.7): public beta for **Enterprise**; Team and Max "coming soon"; no price on the page ([announcement, 2026-04-30](https://claude.com/blog/claude-security-public-beta)). Not available to us.
- **Project Glasswing / Claude Mythos Preview** (announced 2026-04-07): launch partners plus 40+ organisations; maintainers can apply via Claude for Open Source; up to $100M usage credits committed ([Anthropic](https://www.anthropic.com/news/project-glasswing)). The page says "thousands of high-severity vulnerabilities" and gives no exact total.
- **Claude for Open Source.** Six months free; eligibility is any one of: 500+ dependent repos / 100+ dependent packages / 200k monthly downloads; listed committer on a recognised foundation or language project; 100+ merged PRs elsewhere in 12 months; a repo with 20+ external contributors; OpenSSF criticality score >= 0.4. "No lower threshold... you can still apply and explain" ([page](https://claude.com/open-source-max), 2026-11-27). The page says the benefit is "Claude Ultra", the terms page found by search says Max 20x: discrepancy, `[unverified which]`. OxiScript is unlikely to meet the numeric criteria now.

### 2.2 GitHub: CodeQL and Copilot Autofix

- **What it does.** CodeQL builds a database and runs interprocedural dataflow queries; Autofix proposes diffs for a subset of default and security-extended queries, including Rust ([responsible-use doc](https://docs.github.com/en/code-security/code-scanning/managing-code-scanning-alerts/responsible-use-autofix-code-scanning)).
- **Rust.** Public preview 2025-07-01, GA 2025-10-14 with CodeQL CLI 2.23.3; covers all OWASP Top 10 categories except A06 ([changelog](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available)). Supported: Rust editions 2021 and 2024, no nightly-only features; Wasm not mentioned ([supported languages](https://codeql.github.com/docs/codeql-overview/supported-languages-and-frameworks/)). CodeQL does not analyse `.wasm` or `.oxs`; `w2c2`-generated C (stage 8.2b) could be scanned by the C/C++ extractor `[our inference]`.
- **Cost.** Autofix: "free on all public repositories", "do not need a Copilot subscription" ([changelog 2024-09-17](https://github.blog/changelog/2024-09-17-now-available-for-free-on-all-public-repositories-copilot-autofix-for-codeql-code-scanning-alerts), [docs](https://docs.github.com/en/code-security/code-scanning/managing-code-scanning-alerts/responsible-use-autofix-code-scanning)). Code scanning on private repos needs GitHub Code Security. The newer "agentic autofix" is paid (Enterprise Cloud). Do not confuse the two.
- **Licence/terms.** The CodeQL CLI is proprietary: free for "open source code" and for generating databases for CI "only if the codebase is hosted and maintained on GitHub.com"; no use on private repos without a paid GHAS licence ([licence](https://github.com/github/codeql-cli-binaries/blob/main/LICENSE.md)). We qualify while the repo stays public on GitHub.com. The `github/codeql` queries' licence (MIT) I did not re-verify `[unverified]`.
- **Privacy.** Autofix data is "not employed for LLM training purposes" (same responsible-use doc).

### 2.3 Fuzzing: ClusterFuzzLite, OSS-Fuzz, OSS-Fuzz-Gen

- **ClusterFuzzLite** (Apache-2.0 per GitHub API): code-change fuzzing on PRs, batch fuzzing, coverage reports; supports GitHub Actions, GitLab, Cloud Build, Prow; Rust is among the listed languages; the page gives no price ([site](https://google.github.io/clusterfuzzlite/)). Uses the same harness layout as OSS-Fuzz. Cost on GitHub-hosted runners follows GitHub Actions minutes (free for public repos per GitHub's billing docs, not re-fetched `[unverified]`).
- **OSS-Fuzz**: Apache-2.0; eligibility is one sentence: open source and "a significant user base and/or be critical to the global IT infrastructure" ([page](https://google.github.io/oss-fuzz/getting-started/accepting-new-projects/)). A new compiler will probably be deferred. Totals: "over 13,000 vulnerabilities and 50,000 bugs across 1,000 projects" as of May 2025 ([README](https://github.com/google/oss-fuzz)); the draft's "1,300+" was wrong.
- **OSS-Fuzz-Gen** (Apache-2.0): LLM-written fuzz targets; Google reported 26 new vulnerabilities including CVE-2024-9143 in OpenSSL (out-of-bounds write, CVSS 4.3, reported 2024-09-16, fixed 2024-10-16) and extra coverage in 272 C/C++ projects ([blog 2024-11-20](https://security.googleblog.com/2024/11/leveling-up-fuzzing-finding-more.html), figures read through [The Hacker News](https://thehackernews.com/2024/11/googles-ai-powered-oss-fuzz-tool-finds.html) and search summaries, 2026-11-27). Evidence is for C/C++ targets; Rust effect `[unverified]`.

### 2.4 Other SaaS SAST / supply chain (all checked on pricing pages 2026-11-27)

| Service | What it does | Free tier / price | Rust | Note |
|---|---|---|---|---|
| **Semgrep hosted** | SAST, SCA, Assistant (AI triage) | Free: 10 contributors max, 60 AI credits, 10 private repos; Teams from $30/contributor/mo ([pricing](https://semgrep.dev/pricing)) | GA, cross-function only ([docs](https://docs.semgrep.dev/supported-languages)) | Hosted Assistant sends snippets to vendor/LLM `[unverified details]` |
| **Socket** | Dependency risk, AI analysis | Free plan above; Team $25, Business $50/dev/mo ([pricing](https://socket.dev/pricing)) | crates.io only | Free OSS Business upgrade on request |
| **Corgea** | AI SAST, triage, fixes; free MIT scanner Sighthound | Free: 2 members, 10 repos, 10 PR scans/mo; Growth $39, Scale $49/dev/mo ([pricing](https://corgea.com/pricing)) | Pricing page names no languages. **Sighthound has no Rust support** (its README table lists Python, JS/TS, Java, PHP, C#, Go, Ruby, ObjectScript, HTML, SQL; "not supported: C/C++") ([README](https://github.com/Corgea/Sighthound)) | Draft wrongly called it a Rust-bug-shape reference; it is MIT and written in Rust, but scans other languages |
| **ZeroPath** | AI SAST | **No free tier** on the pricing page; Team from $1,000/mo plus $60/dev; free access for independent researchers on application ([pricing](https://zeropath.com/pricing)) | not listed | MCP server is real: [ZeroPathAI/zeropath-mcp-server](https://github.com/ZeroPathAI/zeropath-mcp-server), MIT, needs a ZeroPath account |
| **Snyk Code** | SAST + AI fix | Free: 100 Snyk Code tests/mo; Team from $25/mo; no-cost route for OSS maintainers via application ([plans](https://snyk.io/plans/)) | `[unverified]` (supported-languages page fetched without the per-language table) | |
| **Aikido** | SAST/SCA/secrets | Free: 2 users, 10 repos, 10 AI AutoFixes/mo, AI SAST; paid plans start at 300/mo (currency not shown on the page text I read) ([pricing](https://www.aikido.dev/pricing)) | `[unverified]` (docs URL 404) | No OSS programme on the page |
| **Mobb** | Fix layer on top of other scanners | Free "$0 forever" for public repos; IDE auto-fix $20, Team $40 per dev/mo; Enterprise "let's talk" ([pricing](https://mobb.ai/pricing)) | page lists no languages | Overlaps Autofix |
| **SonarQube Cloud** | Static analysis | Free OSS plan for public projects, announced 2026-01-27, AI CodeFix not mentioned for it ([community post](https://community.sonarsource.com/t/introducing-the-new-free-oss-plan-in-sonarqube-cloud/176923)); Rust support announced 2025-04-22 (date from a search summary, no primary link read), plan coverage `[unverified]` | `[unverified]` on free | AI CodeFix is Team/Enterprise per search summary `[unverified]` |
| **Amazon Q Developer** | IDE/CLI assistant; AWS points former CodeGuru Security users here for code review (SAST, secrets, SCA) | Free: 50 agentic requests/mo; Pro $19/user/mo; the pricing page states no security-scan limits, so whether scans are in the free tier is `[unverified]` ([pricing](https://aws.amazon.com/q/developer/pricing/)) | `[unverified]` | **CodeGuru Security was discontinued on 2025-11-20** ([AWS end-of-support](https://docs.aws.amazon.com/it_it/codeguru/latest/security-ug/end-of-support.html), via search summary) |

### 2.5 Paid, invite-only or enterprise (not usable under D-32)

- **OpenAI Codex Security** (formerly Aardvark, research preview 2026-03-06): ChatGPT Pro/Enterprise/Business/Edu, free for the first month of the preview; in a 30-day beta window it scanned 1.2M+ commits, reporting 792 critical and 10,561 high findings, 14 CVEs assigned ([OpenAI](https://openai.com/index/codex-security-now-in-research-preview/)). Those are vendor figures. **Codex for OSS** gives six months of ChatGPT Pro and reviews Codex Security access case by case for core maintainers with write access ([OpenAI](https://developers.openai.com/codex/community/codex-for-oss)).
- **Google**: Big Sleep appears to be Google-internal (no public access found; `[unverified]`, no primary statement read); it found a stack buffer underflow in SQLite (2024-10) and CVE-2025-6965 (disclosed 2025-07-15, fixed in SQLite 3.50.2) ([SecurityWeek](https://www.securityweek.com/google-says-ai-agent-thwarted-exploitation-of-critical-vulnerability/)); in 2025-08 Google reported 20 flaws in open-source projects such as FFmpeg and ImageMagick, details withheld under the 90-day policy ([TechCrunch](https://techcrunch.com/2025/08/04/google-says-its-ai-based-bug-hunter-found-20-security-vulnerabilities/)). Sec-Gemini v1 (2025-04) was an experimental model for select organisations by request form ([BetaNews](https://betanews.com/2025/04/04/google-launches-sec-gemini-v1-ai-model-to-improve-cybersecurity-defense)); I found no 2026 update. **CodeMender** is a preview for selected enterprise customers; **Gemini 3.5 Flash Cyber** (announced 2026-07-21) is limited to governments and trusted partners through CodeMender, no price or standalone API; Google's V8 comparison (55 vs 47 vs 36 confirmed issues) is unverified vendor data ([TechRepublic](https://techrepublic.com/article/news-google-flash-cyber-codemender/)).
- **Microsoft Security Copilot**: SOC product. Provisioned capacity is $4 per SCU-hour (minimum 1 SCU) and overage $6, seen in Microsoft's worked examples; E5/E7 customers get an SCU allotment ([search summary of Microsoft pages](https://learn.microsoft.com/security-copilot/security-compute-units-capacity)). The pricing page URL in the draft returned 404. The draft's "$2,880/mo" is arithmetic on that rate for one SCU 24/7 (about $2,880 at 720 h); keep as our calculation. No free tier found.
- **Google Security Operations (Sec-PaLM lineage, Gemini in SecOps)**: SIEM/SOAR platform with a Gemini assistant for search, case summaries and playbooks, sold to enterprises by quote. Wrong layer for us (SOC telemetry, not code audit). The product page fetch returned no body on 2026-11-27, so this description is `[unverified]`: [product page](https://cloud.google.com/security/products/security-operations). Sec-PaLM itself was superseded by Sec-Gemini per the draft; not re-checked.
- **Checkmarx and Veracode AI features**: both are enterprise AppSec suites (SAST/SCA with AI-assisted remediation: Checkmarx "Developer Assist"/triage agents, Veracode Fix); I did not re-verify feature lists or Rust support (the draft said Checkmarx has native Rust SAST and Veracode weak Rust: `[unverified]`). Product pages: [Checkmarx](https://checkmarx.com/), [Veracode Fix](https://www.veracode.com/products/veracode-fix).
- **Checkmarx, Veracode, XBOW (prices)**: no public price lists; third-party sites disagree (for example $15K-$100K+/yr Veracode, XBOW from $6,000 per test) and are `[unverified]` ([summary](https://www.pixee.ai/blog/checkmarx-vs-veracode)). The draft's price ranges were unsourced and are removed.

### 2.6 Open-weight models

- **Cisco Foundation-Sec-8B** family: Hugging Face metadata shows the base model as Apache-2.0 with Llama-3.1-8B as base; the 1.1-Instruct and Reasoning cards say "License: other" and point to a NOTICE that names the **Llama 3.1 Community License** (for Llama material) and Apache-2.0 (for Cisco's changes) ([card](https://huggingface.co/fdtn-ai/Foundation-Sec-1.1-8B-Instruct), [NOTICE](https://huggingface.co/fdtn-ai/Foundation-Sec-1.1-8B-Instruct/blob/main/NOTICE.md)). So "Apache-2.0, P-11 compatible" in the draft is **not safe**; the Llama licence is not permissive. Cisco's own benchmarks: CTI-RCM (CVE to CWE mapping) 0.694 vs GPT-4o-mini 0.655; CTI-MCQA 0.644 vs 0.672; intended use is SOC/threat-intel, not code audit ([card](https://huggingface.co/fdtn-ai/Foundation-Sec-1.1-8B-Instruct)). No free hosted API found. Our dev box has no GPU (roadmap 8.Bench).
- **WhiteRabbitNeo**: the 33B v1.5 card is under the DeepSeek licence, an 8B variant under the Llama 3.1 licence (Hugging Face API metadata, 2026-11-27); neither is OSI-permissive. Hosted access via Kindo/Deep Hat is `[unverified]` (not re-checked; the draft's "$20+/user/mo" removed). Pentest framing; no reason to prefer it over Claude for our white-box review.

### 2.7 DARPA AIxCC systems (open source)

DARPA's results page: in the finals teams found 54 and patched 43 of 63 synthetic challenges, found 18 real vulnerabilities (6 C, 12 Java) and patched 11; prizes $4M / $3M / $1.5M; each team received $50,000 in LLM credits (part of a $350,000 donation per vendor); all seven systems to be released under OSI-approved licences ([DARPA](https://www.darpa.mil/news/2025/aixcc-results)). The draft's "$85,000 cloud budget" is not on that page and was removed. Targets were C and Java; Rust is not mentioned in the Trail of Bits finals write-up ([tale of the tape](https://blog.trailofbits.com/2025/08/07/aixcc-finals-tale-of-the-tape/)).

| System | Licence (GitHub API / LICENSE file, 2026-11-27) | Notes |
|---|---|---|
| Trail of Bits **Buttercup** (2nd) | AGPL-3.0 | 28 vulnerabilities found and 19 patches in the finals (48 challenges, 23 repos), spend about $39.6k ($21.1k LLM, $18.5k compute) ([post](https://blog.trailofbits.com/2025/08/09/trail-of-bits-buttercup-wins-2nd-place-in-aixcc-challenge/)); local run needs 8 cores, 16 GB RAM, 100 GB disk, **paid LLM keys**, C and Java OSS-Fuzz-compatible repos ([README](https://github.com/trailofbits/buttercup)). The ">90% accuracy" is the vendor's claim without breakdown |
| Team Atlanta **Atlantis** (1st) | **GPL-3.0** with some MIT/Apache/OpenMDW components ([LICENSE](https://github.com/Team-Atlanta/aixcc-afc-atlantis/blob/main/LICENSE)) | Draft said MIT: wrong |
| Shellphish **Artiphishell** | MIT (GitHub API) | Python, Kubernetes pipeline |
| Theori **RoboDuck** (archive repo) | AGPL-3.0, archived (GitHub API) | Draft said "unverified SPDX" |
| Others (FuzzingBrain, Bug Buster, Lacrosse) | not checked `[unverified]` | |

All need paid LLM keys and heavy compute, target C/Java, and are therefore out under D-32. Use only as design reading; do not copy GPL/AGPL code (P-11).

---

## 3. Terms for sending code (summary)

| Option | Where code goes | Note |
|---|---|---|
| CodeQL / Autofix | GitHub (our host already) | Autofix not used for LLM training (doc above) |
| ClusterFuzzLite, Semgrep CE/Opengrep | Stays on the CI runner | No vendor LLM |
| `/security-review`, Claude in CI | Anthropic | Consumer training toggle and 5-year/30-day retention (2.1) |
| Semgrep hosted Assistant, Socket, Corgea, Snyk, Aikido, Mobb, Sonar | Vendor cloud | Public repo, so low sensitivity; check each ToS before connecting |
| Codex Security, Claude Security, CodeMender | Vendor cloud | Out of budget anyway |

OxiScript is public and Apache-2.0, so confidentiality matters little; what matters is not putting secrets in prompts and not giving AI workflows write tokens on untrusted PRs.

---

## 4. Evidence of effectiveness

| Source | What it shows | Caveat |
|---|---|---|
| **PrimeVul**, Ding et al., ICSE 2025, [arXiv:2403.18624](https://arxiv.org/abs/2403.18624) | A 7B model scored 68.26% F1 on BigVul but 3.09% F1 on PrimeVul (abstract) | Supports "do not trust small fine-tuned detectors" (the draft's number and conclusion check out) |
| **SEC-bench**, Lee et al., [arXiv:2506.11791](https://arxiv.org/abs/2506.11791) | Top agents: up to 18.0% PoC generation, up to 34.0% patching | |
| **SEC-bench Pro**, [arXiv:2605.26548](https://arxiv.org/abs/2605.26548) (submitted 2026-05-26, revised 2026-07-20) | 344 instances in V8, SpiderMonkey and the Linux kernel; Codex with GPT-5.5 solved 58%; Claude Code with Opus 4.6 often timed out; GLM-5 13 of 344 | The draft cited the ID with no content; it exists |
| **CVE-Bench**, Zhu et al., [arXiv:2503.17332](https://arxiv.org/abs/2503.17332) | Agents exploit real **web application** CVEs; best framework resolves up to 13% | Web exploitation, not code review or compilers |
| **CyberSecEval 1/2/3**, [2312.04724](https://arxiv.org/abs/2312.04724), [2404.13161](https://arxiv.org/abs/2404.13161), [2408.01605](https://arxiv.org/abs/2408.01605) | Insecure-code suggestion rates, prompt-injection susceptibility, exploit generation, offensive capability | Measures model risk, not detector quality |
| **AIxCC finals**, [DARPA](https://www.darpa.mil/news/2025/aixcc-results) | 54/63 synthetic found, 43 patched, 18 real vulnerabilities, 11 patched | C and Java only; cost about $152 per task on average (DARPA) |
| **OSS-Fuzz-Gen**, [Google 2024-11](https://security.googleblog.com/2024/11/leveling-up-fuzzing-finding-more.html) | 26 new vulnerabilities from AI-written targets | C/C++ |
| **Big Sleep**, [SecurityWeek](https://www.securityweek.com/google-says-ai-agent-thwarted-exploitation-of-critical-vulnerability/), [TechCrunch](https://techcrunch.com/2025/08/04/google-says-its-ai-based-bug-hunter-found-20-security-vulnerabilities/) | CVE-2025-6965 in SQLite; 20 reported flaws | Not available to us; C/C++ |
| **Codex Security**, [OpenAI](https://openai.com/index/codex-security-now-in-research-preview/) | 792 critical, 10,561 high findings over 1.2M commits; 14 CVEs; vendor-reported noise reductions | Vendor figures; the appendix has mismatched CVE links per the fetch summary |
| **Mythos on curl**, [Stenberg 2026-05-11](https://daniel.haxx.se/blog/2026/05/11/mythos-finds-a-curl-vulnerability/) | Five "confirmed vulnerabilities" reported; one real (low severity), three false positives, one plain bug; about twenty other bugs mostly valid; zero memory-safety issues; Stenberg calls the hype "primarily marketing" yet still urges scanning with AI analysers | Independent maintainer view. The draft's "VulnCheck critique" was uncited and I could not verify it: removed |

**What this means for us.** AI findings are only as good as the reproducer attached. This matches our rule that every finding becomes an executable regression test (bad-actor-rounds.md). Fuzzers produce reproducers by construction; LLM reviews do not.

### Fit against our attack list (`docs/research/oxiscript-known-attacks.md`, classes 1-5 in section headings)

| Class | CodeQL / Semgrep | Fuzzing | Claude BAD ACTOR |
|---|---|---|---|
| 1 TypeScript-style type-system unsoundness (checker) | Weak (domain-specific) | Only with a differential or panic oracle | Main tool |
| 2 Engine-style bugs in lowering/codegen | Weak | Strong with differential oracles (our own tests, Stub vs Linked) | Strong |
| 3 Wasm runtime advisories (`wasmi` host, metering, memory) | Partial: unsafe, unchecked `ptr+len` rules | Strong (hostile `.wasm`, fuel sweeps) | Strong |
| 4 Parser/tooling (nesting, fmt round trip, escape injection) | Partial (custom rules) | Very strong | Good |
| 5 Supply chain (future) | `cargo audit` today, Socket optional | n/a | Weak (no live CVE database) |

(This is our own judgement, not a published result; the section numbers refer to the headings of the known-attacks file.)

---

## 5. Recommendation

### 5.1 Try first, at zero cost, in this order

1. **CodeQL for Rust with Autofix on the public repo.** Add the GitHub default setup for Rust (or an advanced workflow). Cost: none. Then, after the first results, add a few repo-local queries for the runtime crates. It is the only non-LLM semantic analysis that is free and Rust-native. Run it on PRs and `main`; triage alerts in the next BAD ACTOR round. Watch for the licence condition: public repo on GitHub.com only.
2. **`cargo-fuzz` harnesses + ClusterFuzzLite.** Harness candidates: lexer/parser/format round trip, checker, codegen plus `wasmparser` validation, RC alloc/drop sequences, `wasmi` host with hostile modules. Start with PR-mode short runs and nightly batch runs next to the existing nightly gate. Apply to OSS-Fuzz only when eligibility is plausible. This needs engineering time, not money, and each crash is a ready regression test.
3. **Semgrep CE or Opengrep with a small `.semgrep/` ruleset** for our invariants. Pick one engine (both LGPL-2.1, run as external CLI). Prefer a pinned release and our own rules; avoid depending on Semgrep's upstream rules repository, whose licence changed in 2024-25 (Semgrep Rules License v1.0 restricts use in competing products; the CE engine stays LGPL-2.1; see [InfoQ](https://www.infoq.com/news/2025/02/semgrep-forked-opengrep), 2026-11-27).

Also keep `/security-review` as the cheap pre-round Claude pass (it is the BAD ACTOR's own family, so never count it as the independent check), and add `cargo deny` (already planned in the roadmap). Optional: Socket free plan on the repo. Owner action, zero cost: apply to Claude for Open Source and Codex for OSS; treat acceptance as a bonus.

### 5.2 How they plug into BAD ACTOR rounds (D-31/D-33/D-34)

1. **Per branch:** `scripts/oxiscript-gate.sh` as today; `/security-review` on the diff (mandatory for Gemini-authored branches).
2. **Per PR/push (CI):** CodeQL; Semgrep/Opengrep rules; ClusterFuzzLite short run. Their alerts are inputs, not verdicts.
3. **Nightly:** existing `oxiscript-fuzz.yml` gate plus ClusterFuzzLite batch fuzzing; new crashes become issues with reproducers.
4. **Round packet for the Opus BAD ACTOR:** commit ranges, open CodeQL/Semgrep alerts, new fuzz crashes and corpus stats, plus the attack list. The Opus agent concentrates on class 1 (type-system soundness) and on semantic and exploitability checks for classes 2-3, which tools only partly cover; it re-checks tool alerts and writes the regression tests. Optional Gemini pass only alongside it.
5. **Output:** `docs/stages/rounds/<date>-<n>.md`, regression tests, and any new rule/query/harness for the tools above, so each round improves the deterministic layer.

### 5.3 Do not rely on

- Small fine-tuned vulnerability classifiers (PrimeVul; Foundation-Sec is SOC/CTI-oriented and its licence includes Llama terms).
- AIxCC systems as turnkey tools (paid LLM keys, C/Java, GPL/AGPL licences for the top two).
- AI GitHub Actions on public PRs with secrets or write permission (README warning above; fork PRs get no secrets anyway).
- Vendor statistics (Codex Security, Mythos, Flash Cyber) as evidence for our code; the curl case shows a gap between "confirmed vulnerabilities" claimed and accepted.
- Any single AI scanner, Gemini included, as security sign-off (D-31/D-33).

---

## 6. Changes from the Gemini draft (58498d2)

| Draft claim | Verdict | Source |
|---|---|---|
| Atlantis licence MIT | **Wrong**: GPL-3.0 with third-party components | [LICENSE](https://github.com/Team-Atlanta/aixcc-afc-atlantis/blob/main/LICENSE) |
| Theori repo "unverified SPDX" | AGPL-3.0, archived | GitHub API |
| Foundation-Sec "Apache-2.0, P-11 compatible"; HF Endpoint $0.40/hr, PRO $9 with $2 credit | Licence partly Llama 3.1 Community; prices unsourced, removed | model card, NOTICE |
| Sighthound as Rust-aware scanner/reference for Rust bug shapes | **Wrong**: no Rust support | [README](https://github.com/Corgea/Sighthound) |
| ZeroPath "free tier, ~$200/mo" | **Wrong**: no free tier; Team $1,000/mo + $60/dev | [pricing](https://zeropath.com/pricing) |
| ZeroPath MCP server `[unverified]` | Exists, MIT | [repo](https://github.com/ZeroPathAI/zeropath-mcp-server) |
| AIxCC "$85k cloud budget per team" | Not on DARPA page; removed. $50k LLM credits confirmed | [DARPA](https://www.darpa.mil/news/2025/aixcc-results) |
| OSS-Fuzz "1,300+ projects" | README says 1,000 projects (as of May 2025) | [README](https://github.com/google/oss-fuzz) |
| `claude-code-security-review` Action "needs API key... NOT covered by Pro" | Partly wrong: the repo's example uses an API key, but `claude-code-action` supports a subscription OAuth token; repo looks unmaintained | [Actions docs](https://code.claude.com/docs/en/github-actions), repo |
| Claude Security Enterprise/"local plugin beta `[unverified]`" | Enterprise public beta, Team/Max "coming soon"; plugin claim dropped | [announcement](https://claude.com/blog/claude-security-public-beta) |
| Glasswing "10,000+ first month, >33,000 later, CVE-2026-4747" | Not on the cited page (which says "thousands"); removed | [Glasswing](https://www.anthropic.com/news/project-glasswing) |
| "VulnCheck and curl maintainers' late-2026 critique" (uncited) | Replaced with the checkable curl/Mythos post; VulnCheck figures unverified | [curl blog](https://daniel.haxx.se/blog/2026/05/11/mythos-finds-a-curl-vulnerability/) |
| CVE-Bench "exposes multi-file context blindness... repair variant" | Not in the abstract; CVE-Bench is web-exploitation (up to 13%); removed | [arXiv](https://arxiv.org/abs/2503.17332) |
| SEC-bench Pro ID | ID is real; draft attached no content | [arXiv](https://arxiv.org/abs/2605.26548) |
| Big Sleep "mid-2025 CVE-2025-6965", "20 vulns" | Confirmed (20 flaws, details withheld) | SecurityWeek, TechCrunch |
| Codex Security numbers | Confirmed against OpenAI page; "$200/mo Pro" unsourced, removed; "Chromium, OpenSSL/OpenSSH, PHP" CVE list not confirmed | [OpenAI](https://openai.com/index/codex-security-now-in-research-preview/) |
| Socket "free ~1,000 scans, ~$25-$50" | Confirmed (Team $25, Business $50); Rust GA only via aggregator copy | [pricing](https://socket.dev/pricing) |
| Semgrep "Rust: CE single-file; Pro cross-file" | Docs: Rust GA with cross-function dataflow, no cross-file; CE/Pro split not stated | [docs](https://docs.semgrep.dev/supported-languages) |
| Semgrep Team "$30-$40" | $30/contributor/mo; free tier confirmed with 60 credits | [pricing](https://semgrep.dev/pricing) |
| Mobb "limited Rust, Java/JS focus" | Page lists no languages; claim removed. Free for public repos confirmed | [pricing](https://mobb.ai/pricing) |
| Amazon Q "CodeGuru absorbed"; Checkmarx/Veracode/XBOW/Kindo price ranges | Unsourced; removed or marked third-party `[unverified]` | sections 2.4-2.6 |
| Microsoft pricing URL | 404; rate seen only in worked examples | section 2.5 |
| Repo facts: `scripts/oxiscript-gate.sh` runs `cargo audit` and `cargo deny` | **Wrong**: gate has neither; `cargo audit` is in `ci.yml`; no `cargo deny` yet | local grep |
| Bare-homepage citations (`github.blog/changelog/`, `blog.google/`, `darpa.mil`, project-zero root, `socket.dev/blog`) | Replaced by article URLs or dropped | throughout |
| Check date 2026-11-26 | Re-dated 2026-11-27 | |
| Confirmed unchanged | PrimeVul numbers, SEC-bench 18%/34%, CodeQL Rust GA 2025-10-14, Autofix free on public repos, Buttercup AGPL-3.0, Artiphishell MIT, Buttercup 28/19 and $50k credits, AIxCC 54/43/18/11, OSS-Fuzz-Gen 26 + CVE-2024-9143 | sources above |

## 7. Open points for the owner

- Whether to use the subscription OAuth token in CI (technically supported; consumer-terms fit unchecked).
- Whether to apply to Claude for Open Source and Codex for OSS now or after the project has traction.
- Which of Semgrep CE and Opengrep to adopt (Opengrep's Rust coverage was not verified here).
