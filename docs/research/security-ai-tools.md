# Commercial & Online Security AI Models and Services for OxiScript / OxiUI BAD ACTOR

> **Compiled / Checked:** 2026-11-26
> **Scope:** Evaluation of commercially available, online security-specialised AI models, SaaS scanners, and open-sourced autonomous Cyber Reasoning Systems (CRSs) that could strengthen OxiScript/OxiUI's adversarial **BAD ACTOR** process ([`docs/stages/bad-actor-rounds.md`](../stages/bad-actor-rounds.md), [`docs/research/oxiscript-known-attacks.md`](oxiscript-known-attacks.md)).
> **Hard constraints from [`docs/roadmap/oxiscript.md`](../roadmap/oxiscript.md):**
> - **D-32 (Zero additional budget):** Current budget is solely the owner's **Claude Code Pro** subscription; no money for paid APIs, SaaS subscriptions, or external auditors right now. Zero-cost and public-repo free-tier options are ranked first.
> - **D-31 / D-33 (Model governance):** Claude (Opus for large stages/releases; Sonnet with Opus consultation for small changes) is always the BAD ACTOR. Gemini or any other model/service may add independent passes, but **never** replaces Claude.
> - **D-34 (BAD ACTOR rounds):** Changes may be batched into rounds; large runtime/memory/sandbox/safety stages get an Opus round before merge, and every finding becomes a regression test.
> - **P-11 (Licensing):** Permissive licences only (MIT / Apache-2.0 / BSD) for anything embedded or linked into OxiScript/OxiUI. External services or standalone CI/CLI tools not embedded in the product may carry other licences (e.g. LGPL-2.1, AGPL-3.0, or proprietary SaaS terms).
> - **Target stack:** Rust compiler and runtime crates (`oxiscript-syntax`, `oxiscript-check`, `oxiscript-lower`, `oxiscript-wasm`, `oxiscript-rc`, `oxiscript-runtime` embedding `wasmi`), emitting Core WebAssembly (`.wasm`), plus OxiScript source (`.oxs`) and upcoming `w2c2` Wasm-to-C AOT output for the `micro` target ([D-27](../roadmap/oxiscript.md#d-27), [D-28](../roadmap/oxiscript.md#d-28)).
>
> **Verification note:** All facts and pricing figures below were checked via web search on **2026-11-26**. Where an exact pricing figure, query list, or term could not be confirmed on a primary vendor page directly (e.g. behind sales quotes or gated docs), it is tagged `[unverified]`.

---

## Executive Summary & Ranked Overview

Most commercial "AI security" products are built for **enterprise web apps (OWASP Top 10) or SOC alert triage**, not for a statically typed language compiler, reference-counting runtime, and WebAssembly sandbox written in Rust. Furthermore, empirical evaluations ([PrimeVul](https://arxiv.org/abs/2403.18624), [SEC-bench](https://arxiv.org/abs/2506.11791)) show that small fine-tuned vulnerability classifiers collapse under real-world deduplicated conditions, while autonomous agents (AIxCC CRSs, Big Sleep, Codex Security) succeed mainly when pairing **frontier reasoning models with deterministic fuzzers/sanitizers** — and at significant API compute cost.

Under **D-32 ($0 additional budget)** and **D-31/D-33 (Claude always in the loop)**, the highest-leverage additions to BAD ACTOR cost nothing on a public GitHub repository:
1. **Claude Code `/security-review`** (already included in the owner's Claude Code Pro subscription) for fast, diff-aware pre-round passes before spending Opus quota on full BAD ACTOR rounds.
2. **GitHub CodeQL for Rust (GA) + GitHub Copilot Autofix** (free on public GitHub repositories) for continuous cross-file Rust static analysis and automated patch suggestions in CI/PRs.
3. **Semgrep Community Edition (LGPL-2.1 CLI) + `cargo-fuzz` / ClusterFuzzLite** (free in GitHub Actions, paving the way for **Google OSS-Fuzz + OSS-Fuzz-Gen** once OxiScript meets critical-project eligibility) for custom project invariants (Rust `unsafe`/host-boundary/determinism checks, `.wat` rules) and continuous coverage-guided fuzzing.
4. *(Optional zero-cost supply-chain layer)* **Socket AI GitHub App** (free on public repos) to audit `Cargo.lock` / `build.rs` / `unsafe` in Rust dependencies (`wasmi`, etc.) alongside `cargo audit` and `cargo deny`.

### Ranked Summary Table (Zero-Cost / Included-in-Pro Options First)

| Rank | Name | Type | Cost / Free Tier (checked 2026-11-26) | Rust / Wasm Support | What It Adds to BAD ACTOR | Source Links |
|---|---|---|---|---|---|---|
| **1** | **Claude Code `/security-review`** (Anthropic) | Frontier LLM diff-aware security review (local CLI) | **$0 extra** (included in existing Claude Pro $20/mo subscription; GitHub Action requires paid API key) | **Rust: Strong** (semantic); **Wasm/`.oxs`/`.wat`: Strong** (understands specs, `AGENTS.md`, `oxiscript-known-attacks.md`) | Fast, low-friction diff audit before batching changes into Opus BAD ACTOR rounds; directly complies with D-32 & D-33. | [Claude Code docs](https://docs.anthropic.com/en/docs/claude-code), [claude-code-security-review](https://github.com/anthropics/claude-code-security-review) |
| **2** | **GitHub CodeQL + Copilot Autofix** | Semantic SAST + AI remediation (CI / PR) | **$0** for public GitHub repos (no paid Copilot or GHAS required); paid GHAS on private repos | **Rust: Yes (GA)** (`CWE-476`/`825`, injections, crypto, OWASP); **Wasm: No** (scans source Rust, or C from `w2c2`) | Automated cross-file dataflow SAST on every PR + AI fix diffs + custom CodeQL queries for host-boundary / `unsafe` invariants. | [GitHub CodeQL Rust GA](https://github.blog/changelog/), [CodeQL docs](https://codeql.github.com/docs/), [Copilot Autofix docs](https://docs.github.com/en/code-security/code-scanning/managing-code-scanning-alerts/responsible-use-autofix-code-scanning) |
| **3** | **Semgrep Community Edition** *(+ hosted Free Tier / Assistant)* | Pattern-based SAST CLI (LGPL-2.1) + optional hosted AI triage | **$0** for local/CI CLI (LGPL-2.1); hosted free tier up to 10 contributors (~60 AI credits/mo `[unverified]`); paid Team ~$30–$40/dev/mo per module | **Rust: Yes** (CE single-file; Pro cross-file); **Wasm: Partial** (custom rules on `.wat` text format); **`.oxs`: Regex/generic** | Cheap, fast, deterministic custom rules in CI for Stage `8.Sec` invariants (no raw ANSI echoes, no `unsafe`, checked `(ptr, len)` math, deterministic collections). | [Semgrep](https://semgrep.dev/), [Semgrep GitHub](https://github.com/semgrep/semgrep), [Semgrep Pricing](https://semgrep.dev/pricing) |
| **4** | **Google OSS-Fuzz & OSS-Fuzz-Gen** *(+ ClusterFuzzLite now)* | Free continuous fuzzing + LLM fuzz-harness generator | **$0** for accepted critical OSS projects (Google-funded compute); **ClusterFuzzLite** is $0 in GitHub Actions today | **Rust: Yes** (native `cargo-fuzz` / libFuzzer + ASan); **Wasm: Indirect** (fuzz compiler/runtime + byte-mutated `.wasm`) | Continuous 24/7 fuzzing + LLM-generated harnesses finding crashes, panics, OOMs, and `unsafe`/FFI bugs; upgrades `.github/workflows/oxiscript-fuzz.yml`. | [OSS-Fuzz](https://github.com/google/oss-fuzz), [OSS-Fuzz-Gen](https://github.com/google/oss-fuzz-gen), [Google Security Blog (2024-11)](https://security.googleblog.com/2024/11/leveling-up-fuzzing-finding-more.html) |
| **5** | **Socket AI** (Socket.dev) | AI supply-chain & dependency risk scanner | **$0** on unlimited public repos (GitHub App; OSS Business upgrade available by application); paid Team/Business ~$25–$50+/dev/mo `[unverified]` | **Rust/Cargo: Yes (GA)** (`Cargo.toml`/`Cargo.lock`, `build.rs`, hidden `unsafe`, FFI, typosquatting); **Wasm: Indirect** (npm/PyPI `.wasm` bundles) | Guards `wasmi` and future Stage `8.N` linked Rust crates against malicious `build.rs`, typosquatting, and silent `unsafe` additions. | [Socket.dev](https://socket.dev/), [Socket Pricing](https://socket.dev/pricing), [Socket Rust announcement](https://socket.dev/blog) |
| **6** | **Corgea** *(+ open-source **Sighthound**)* | AI SAST + triage + auto-fix SaaS; MIT Rust AST scanner | **$0 Free Tier** (up to 2 devs, 10 repos, 10 PR scans & 10 fixes/mo `[unverified]`); paid Growth ~$39/dev/mo. **Sighthound**: MIT open source | **Rust: Yes** (`unsafe`, FFI, `Result`/`Option`, Axum/Actix); **Wasm: No** | Optional second-opinion AI SAST on PRs; MIT-licensed **Sighthound** (written in Rust + Tree-sitter) is a permissive reference for Stage `8.S4c` bug shapes. | [Corgea](https://corgea.com/), [Corgea/Sighthound](https://github.com/Corgea/Sighthound) |
| **7** | **ZeroPath** | AI-native SAST & logic-bug scanner + MCP server | **$0 Free Tier** (basic use; free researcher access); paid starts ~$200/mo flat `[unverified]` | **Rust: Yes** (semantic LLM analysis); **Wasm: No** | Can triage/re-score CodeQL/Semgrep alerts and hunt business-logic flaws; open-source MCP server can plug into Claude workflows. | [ZeroPath](https://zeropath.com/), [ZeroPath GitHub](https://github.com/ZeroPathAI) |
| **8** | **Snyk DeepCode AI** (Snyk Code) | Hybrid SAST + AI autofix SaaS | **$0 Free Plan** (capped monthly tests) + **Secure Developer Program** (free Enterprise tier for eligible OSS by application); Team ~$25/dev/mo | **Rust: Partial/Yes** (web/async frameworks; full Rust SAST may require higher tier `[unverified]`); **Wasm: SCA only** | Broad SAST + SCA second opinion if accepted into Snyk's OSS program; weaker fit for compiler internals than CodeQL. | [Snyk Pricing](https://snyk.io/plans/), [Snyk Open Source Program](https://snyk.io/open-source/), [Snyk DeepCode AI](https://snyk.io/platform/deepcode-ai/) |
| **9** | **Aikido Security** | All-in-one AppSec (SAST/SCA/secrets) + AI AutoFix | **$0 Free Developer Plan** (up to 2 users, 10 repos, capped monthly AI credits); paid ~$315–$350+/mo | **Rust: Yes** (SCA + SAST); **Wasm: No** | Low-noise consolidated dashboard (SAST + Cargo SCA + secrets) for a small public repo, though overlaps CodeQL + Socket + `cargo-audit`. | [Aikido Security](https://www.aikido.dev/), [Aikido Pricing](https://www.aikido.dev/pricing) |
| **10** | **Mobb** | Automated vulnerability remediation layer (over SAST) | **$0 forever for public OSS repos**; paid commercial tiers | **Rust: Limited** (depends on upstream SAST + Mobb fix-engine language matrix, primarily Java/JS/TS/C#); **Wasm: No** | Little incremental value over GitHub Copilot Autofix (which is already free on public repos and supports Rust natively). | [Mobb Pricing](https://mobb.ai/pricing), [Mobb GitHub](https://github.com/mobb-dev) |
| **11** | **SonarQube Cloud** *(AI CodeFix is paid)* | Static analysis SaaS + paid AI CodeFix | **$0** on SonarQube Cloud for public OSS repos (static analysis only); **AI CodeFix requires paid Team/Enterprise** | **Rust: Yes** (native analyzer + Clippy integration); **Wasm: No** | Extra static quality/security gate on Rust crates, but AI remediation is paywalled and overlaps Clippy + CodeQL. | [SonarQube Cloud](https://www.sonarsource.com/products/sonarcloud/), [Sonar Rust](https://www.sonarsource.com/knowledge/languages/rust/) |
| **12** | **Amazon Q Developer** *(absorbed CodeGuru Security)* | IDE AI coding assistant + inline security scans | **$0 Free Tier** (AWS Builder ID, 50 agentic requests/mo); Pro $19/user/mo | **Rust: Yes** (IDE suggestions & inline scans); **Wasm: No** | Capped IDE-only scan; weak value compared to Claude Code Pro + GitHub CodeQL in CI. | [Amazon Q Developer Pricing](https://aws.amazon.com/q/developer/pricing/) |
| **13** | **Team Atlanta Atlantis** *(AIxCC 1st)* | Open-source autonomous CRS (LLM + fuzzing + symex) | **MIT software ($0)**, but requires **paid LLM API keys + heavy cloud compute** ($50k LLM + $85k cloud budget in AIxCC finals) | **Targets: C & Java** (competition scope; uses Rust/Python internally); **Wasm: No** (potential future fit for `w2c2` C output) | Architectural reference for multi-agent fuzzing/patching (permissive MIT licence); impractical to run today under D-32 ($0 API budget). | [Team-Atlanta/aixcc-afc-atlantis](https://github.com/Team-Atlanta/aixcc-afc-atlantis), [DARPA AIxCC](https://aicyberchallenge.com/) |
| **14** | **Trail of Bits Buttercup** *(AIxCC 2nd)* | Open-source autonomous CRS (static analysis + AI-guided fuzzing) | **AGPL-3.0 software ($0)**, but requires **paid LLM API keys + cluster compute**; cannot embed in OxiScript (P-11) | **Targets: C & Java** (via OSS-Fuzz/libFuzzer/Jazzer); **Rust/Wasm: Not out-of-the-box** | Demonstrates high patch accuracy (~90%) with cheaper non-reasoning models; external-only due to AGPL-3.0, blocked by D-32 API cost & C/Java focus. | [trailofbits/buttercup](https://github.com/trailofbits/buttercup), [Trail of Bits Blog](https://blog.trailofbits.com/) |
| **15** | **Other AIxCC Finalists** *(RoboDuck, Artiphishell, FuzzingBrain, Bug Buster, Lacrosse)* | Open-source autonomous CRSs | **$0 code** (Artiphishell MIT; others archived OSS `[unverified]` per repo), but require **paid LLM APIs + Kubernetes/cloud compute** | **Targets: C & Java**; **Rust/Wasm: No** | Useful research references for Stage `8.S4c` / fuzz-harness automation; unusable directly under D-32. | [Shellphish/artiphishell](https://github.com/shellphish/artiphishell), [theori-io/aixcc-afc-archive](https://github.com/theori-io/aixcc-afc-archive), [AIxCC](https://aicyberchallenge.com/) |
| **16** | **Cisco Foundation-Sec-8B** *(Base / 1.1-Instruct / Reasoning)* | Open-weight 8B cybersecurity LLM (Llama 3.1 base) | **$0 weights (Apache-2.0)** for self-hosting; Hugging Face Inference Endpoints ~$0.40+/hr GPU (no free hosted API) | **SOC/CTI/CWE-mapping focus**; not specialised for Rust compilers or Wasm | Strong at CVE->CWE triage & CTI on a small 8B budget, but far weaker than Claude Opus/Sonnet at Rust/compiler bug finding (and dev box has no GPU per roadmap `8.Bench`). | [fdtn-ai/Foundation-Sec-8B](https://huggingface.co/fdtn-ai/Foundation-Sec-8B), [Foundation-Sec-1.1-8B-Instruct](https://huggingface.co/fdtn-ai/Foundation-Sec-1.1-8B-Instruct), [Foundation-Sec-8B-Reasoning](https://huggingface.co/fdtn-ai/Foundation-Sec-8B-Reasoning) |
| **17** | **WhiteRabbitNeo / Kindo Deep Hat** | Open-weight & enterprise offensive/defensive LLM | **$0 weights** (custom Llama + WhiteRabbitNeo restricted licence); hosted via HF pay-as-you-go or Kindo Enterprise (sales quote, ~$20+/user/mo `[unverified]`) | **Pentest / red-team / IaC / web focus**; weak on Rust/Wasm semantics | Uncensored offensive framing is unnecessary for our white-box compiler BAD ACTOR (Claude already runs adversarial tests on our own code); custom licence restricts reuse. | [WhiteRabbitNeo on HF](https://huggingface.co/WhiteRabbitNeo), [Deep Hat](https://www.deephat.ai/), [Kindo](https://www.kindo.ai/) |
| **18** | **OpenAI Codex Security** *(formerly **Aardvark**)* | Autonomous repo-wide security researcher agent (GPT-5) | **Paid only**: Research preview for ChatGPT Pro ($200/mo `[unverified]`), Business, Enterprise, Edu + token/credit usage; no free tier | **Language-agnostic LLM reasoning** (supports Rust semantically; sandboxed validation); **Wasm: Indirect** | High capability (found real CVEs in Chromium/OpenSSL/GnuTLS), but excluded by D-32 ($0 additional budget). | [Introducing Aardvark](https://openai.com/index/introducing-aardvark/), [Codex Security Preview](https://openai.com/index/codex-security-now-in-research-preview/) |
| **19** | **Anthropic Claude Security / Project Glasswing (Mythos Preview)** | Managed autonomous security scanner & partner frontier model | **Enterprise / invite-only**: Claude Security beta is for Claude Enterprise; Project Glasswing (Mythos) is restricted to critical-software partners | **Language-agnostic frontier reasoning** (strong on systems/C/Rust) | Same vendor/lineage as our BAD ACTOR, but managed scanner & Mythos Preview are inaccessible on an individual Pro subscription (D-32). | [Project Glasswing](https://www.anthropic.com/news/project-glasswing), [Mythos Cybersecurity Assessment](https://www.anthropic.com/news/assessing-claude-mythos-preview-cybersecurity-capabilities) |
| **20** | **Google Sec-PaLM / Sec-Gemini / Google SecOps / Big Sleep** | Security LLMs, enterprise SIEM/SOAR, & internal 0-day agent | **Big Sleep: Not available to outsiders** (Google internal). **Sec-Gemini**: invite-only trusted-tester (free if accepted). **SecOps**: enterprise SIEM contract | **Big Sleep**: C/C++ OSS targets (internal); **SecOps/Sec-Gemini**: SOC, CTI, YARA-L, OSV triage | **Big Sleep** is world-class but unavailable to us; **SecOps** is a SOC SIEM, not a Rust compiler auditor. | [Google Project Zero Big Sleep](https://googleprojectzero.blogspot.com/), [Sec-Gemini GitHub](https://github.com/google/sec-gemini), [Google SecOps](https://cloud.google.com/security/products/security-operations) |
| **21** | **Checkmarx & Veracode AI Fix** | Enterprise AppSec suites (SAST/SCA + AI Fix) | **Enterprise quote-only**: Checkmarx ~$30k–$350k+/yr; Veracode ~$12k–$300k+/yr `[unverified]`; no free OSS tier | **Checkmarx**: Native Rust SAST; **Veracode**: Weak native Rust coverage; **Wasm: No** | Completely outside D-32 budget and tailored to large corporate AppSec portfolios. | [Checkmarx](https://checkmarx.com/), [Veracode Fix](https://www.veracode.com/products/veracode-fix) |
| **22** | **Microsoft Security Copilot** | Enterprise SOC / Defender / Azure AI assistant | **~$4/SCU/hr** (~$2,880/mo minimum for 1 SCU 24/7; or M365 E5/E7 pool); no free tier | **SOC / KQL / Defender / script de-obfuscation**; not developer Rust/Wasm SAST | Wrong domain (SOC/cloud ops) and prohibitively expensive under D-32. | [Microsoft Security Copilot Pricing](https://azure.microsoft.com/en-us/pricing/details/security-copilot/) |
| **23** | **XBOW** | Autonomous AI web penetration tester | **Enterprise quote-only** (~$50k/yr cloud marketplace anchor `[unverified]`); no free tier | **Live web apps (DAST/pentest)**; **Rust/Wasm compiler: No** | Designed for black-box web pentesting, irrelevant to OxiScript's compiler/runtime threat model (and far outside D-32). | [XBOW](https://xbow.com/) |

---

## 1. Specialised Security Models & Online Services

### 1.1 Zero-Cost & Included-in-Pro Options (Public GitHub Repo / Existing Subscription)

#### Anthropic Claude Code Security Features (`/security-review`, GitHub Action, Claude Security)
*Checked 2026-11-26.*
- **What exists:** Anthropic splits security tooling into three tiers:
  1. **Local `/security-review` slash command in Claude Code:** Included in paid Claude subscriptions (**Pro at $20/mo** or $17/mo annual, Max $100/mo, Ultra $200/mo, Team/Enterprise) ([Claude Pricing](https://claude.com/pricing), [Claude Code docs](https://docs.anthropic.com/en/docs/claude-code)). It inspects the current branch diff against the default branch using semantic reasoning to trace data flows, logic bugs, and missing checks.
  2. **`anthropics/claude-code-security-review` GitHub Action:** Open-source GitHub Action ([github.com/anthropics/claude-code-security-review](https://github.com/anthropics/claude-code-security-review)) that runs diff-aware PR security reviews and posts PR comments. **Important D-32 & security caveats:** Running it in GitHub Actions requires an **Anthropic API key** (pay-as-you-go tokens, NOT covered by the consumer Pro subscription `[unverified]` whether OAuth/Pro token is supported in CI without violating terms), and Anthropic explicitly warns the Action is **not hardened against indirect prompt injection** from untrusted external PRs.
  3. **Claude Security (managed web/repo scanner) & Project Glasswing (Claude Mythos Preview):** Anthropic's dedicated autonomous vulnerability-hunting product and unreleased Mythos Preview model are gated to **Claude Enterprise** public beta and select critical-infrastructure partners under Project Glasswing ([Project Glasswing](https://www.anthropic.com/news/project-glasswing), [Mythos Cybersecurity Assessment](https://www.anthropic.com/news/assessing-claude-mythos-preview-cybersecurity-capabilities)). A local Claude Security plugin beta exists for paid Claude Code plans `[unverified]`.
- **Relevance to OxiScript:** Local `/security-review` inside Claude Code Pro is **free at the margin** (uses existing D-32 budget) and naturally enforces D-33. It reads Rust, `.oxs`, `.wat`, `AGENTS.md`, and `oxiscript-known-attacks.md`.
- **Privacy/Terms:** Individual Claude Pro operates under consumer privacy terms (unlike Team/Enterprise zero-training defaults); because OxiScript is a public open-source repo, sending code to Anthropic is acceptable, though users should check their Claude.ai privacy toggle.

#### GitHub CodeQL (Rust GA) + GitHub Copilot Autofix
*Checked 2026-11-26.*
- **What it does:** **CodeQL** performs deep semantic, interprocedural taint and dataflow analysis over a compiled/extracted database of the codebase. **Copilot Autofix** uses an LLM with an agentic validation loop (re-running CodeQL best-effort) to explain alerts and propose drop-in PR diffs ([Copilot Autofix docs](https://docs.github.com/en/code-security/code-scanning/managing-code-scanning-alerts/responsible-use-autofix-code-scanning)).
- **Rust & Wasm support:** CodeQL support for **Rust is Generally Available (GA)** across GitHub.com and the CodeQL CLI ([CodeQL Rust queries](https://codeql.github.com/codeql-query-help/rust/)). Built-in Rust query suites cover memory/pointer-lifetime errors (`CWE-476`, `CWE-825` in `unsafe` code), injection (`CWE-020`/`079`/`089`), cryptographic misuse (`CWE-295`/`327`), and cleartext logging/storage (`CWE-312`/`532`), covering OWASP Top 10 categories except A06 (handled by Dependabot). CodeQL does **not** natively parse `.wasm` bytecode or `.oxs`, though it **does** have mature C/C++ support that can later scan `w2c2`-generated C output for the `micro` target.
- **Cost:** **$0 for public repositories** on GitHub.com — both CodeQL code scanning and Copilot Autofix are free on public repos without a paid Copilot or GitHub Advanced Security (GHAS) licence ([GitHub Code Security docs](https://docs.github.com/en/code-security/code-scanning/introduction-to-code-scanning/about-code-scanning)). Private repos require paid GHAS / GitHub Code Security.
- **Privacy & Licence:** GitHub explicitly states that code and context processed by Copilot Autofix are **not used to train LLMs**. Note on licensing (P-11): standard CodeQL queries (`github/codeql`) are MIT-licensed, while the CodeQL CLI binary is governed by GitHub's proprietary CodeQL Terms and Conditions (free for OSI-approved open-source projects). Since it runs externally in CI and is never embedded in OxiScript, this complies with P-11.

#### Semgrep Community Edition & Semgrep Assistant
*Checked 2026-11-26.*
- **What it does:** **Semgrep Community Edition (CE)** is an open-source, fast, AST-aware static analyzer (intraprocedural/single-file) ([github.com/semgrep/semgrep](https://github.com/semgrep/semgrep)). **Semgrep Assistant** (part of Semgrep's hosted AppSec Platform / Semgrep Multimodal) adds LLM triage, false-positive filtering, and auto-fix patches on top of Semgrep Pro (cross-file dataflow) ([Semgrep](https://semgrep.dev/)).
- **Rust & Wasm support:** Native **Rust** AST support is available in both CE (community rules + custom YAML rules) and Pro (cross-file dataflow). For WebAssembly, teams can write custom Semgrep rules against **WebAssembly Text Format (`.wat`)** or generic regex rules against `.oxs`.
- **Cost:** Semgrep CE CLI is **$0 forever** (runs locally or in GitHub Actions). Semgrep's hosted platform offers a **Free tier for up to 10 contributors** including base SAST/SCA and a small monthly allocation of AI Assistant credits (~60 AI credits/mo `[unverified]`); paid Team modules run ~$30–$40/contributor/month per product ([Semgrep Pricing](https://semgrep.dev/pricing)).
- **Privacy & Licence:** Local Semgrep CE sends **zero code** off the machine/CI runner; hosted Semgrep Assistant sends relevant code snippets to Semgrep/LLM subprocessors. Semgrep CE is licensed under **LGPL-2.1** — therefore **never embed or link** into OxiScript (P-11), but fully permissible as an external CI/CLI linter.

#### Socket AI (Socket.dev)
*Checked 2026-11-26.*
- **What it does:** AI-augmented software supply-chain security platform focused on detecting malicious packages, typosquatting, protestware, sneaky network/filesystem access, and risky diffs on PRs rather than first-party compiler logic bugs ([Socket.dev](https://socket.dev/)).
- **Rust & Wasm support:** **General Availability (GA) support for Rust & Cargo** (`Cargo.toml`, `Cargo.lock`). Specifically flags suspicious `build.rs` scripts (arbitrary compile-time code execution), hidden `unsafe` blocks, FFI boundaries, and suspicious crate updates on `crates.io`. Also inspects npm/PyPI packages containing bundled `.wasm` binaries.
- **Cost:** **$0 on unlimited public repositories** via the Socket GitHub App (free baseline developer limits ~1,000 scans/mo & 500 API requests `[unverified]`), plus a dedicated **Open Source Program** granting a free Business-tier upgrade (blocking policies, reachability, SBOM) to OSI-licensed public projects upon application ([Socket Pricing](https://socket.dev/pricing)). Paid tiers start ~$25–$50+/dev/month `[unverified]`.
- **Privacy/Terms:** Inspects public repository manifests and PR diffs via GitHub App; low privacy risk for public repos.

#### Corgea (+ Open-Source Rust Scanner `Sighthound`)
*Checked 2026-11-26.*
- **What it does:** AI-native SAST, triage, and automated vulnerability remediation platform; also ingests findings from CodeQL, Semgrep, and Snyk to generate fix PRs ([Corgea](https://corgea.com/)). Corgea also open-sourced **Sighthound** ([github.com/Corgea/Sighthound](https://github.com/Corgea/Sighthound)), an AST-aware static analyzer written in **Rust** using Tree-sitter.
- **Rust & Wasm support:** Corgea provides native Rust analysis (ownership/borrowing context, `unsafe` blocks, FFI boundaries, `Result`/`Option` handling, Axum/Actix/Rocket). No native `.wasm` analysis.
- **Cost:** **Free tier ($0/mo)** covers up to 2 users and 10 repositories with AI SAST, logic/auth checks, SCA, and secrets — capped at 10 PR scans and 10 auto-fixes/month `[unverified]`. Paid Growth starts ~$39/dev/mo, Scale ~$49/dev/mo.
- **Licence & Privacy:** **Sighthound** is **MIT-licensed** (P-11 compatible!). Corgea SaaS processes repo code on vendor servers (review vendor zero-retention terms before connecting private repos `[unverified]`).

#### ZeroPath
*Checked 2026-11-26.*
- **What it does:** AI-native SAST and AppSec platform built around LLM semantic analysis to uncover authentication, authorization, and business-logic flaws with lower false-positive rates, plus CVSS 4.0 re-scoring of CodeQL/Semgrep/Snyk alerts and patch generation ([ZeroPath](https://zeropath.com/)). Provides an open-source **MCP server** ([github.com/ZeroPathAI](https://github.com/ZeroPathAI)) for IDE/agent workflows.
- **Rust & Wasm support:** Supports 30+ languages including **Rust** via LLM semantic analysis. No native `.wasm` bytecode analysis.
- **Cost:** Offers a **Free tier ($0/mo)** for basic usage and free policies for independent security researchers; paid team/core plans start around **$200/month** flat `[unverified]`.
- **Privacy/Terms:** Cloud-hosted code scanning (code transmitted to ZeroPath/LLM backend `[unverified]`).

#### Snyk DeepCode AI (Snyk Code)
*Checked 2026-11-26.*
- **What it does:** Hybrid symbolic + AI SAST engine (**DeepCode AI**) integrated into Snyk Code for vulnerability detection and agentic autofix ([Snyk DeepCode AI](https://snyk.io/platform/deepcode-ai/)).
- **Rust & Wasm support:** Snyk Code supports **Rust** (focused on web apps, async frameworks, and DB/injection flows; full Rust SAST/CLI support has historically varied by tier `[unverified]`). Wasm coverage is limited to SCA on npm/PyPI/Rust dependencies.
- **Cost:** **Free plan ($0/mo)** for individuals/small evaluations with capped monthly test counts; Team starts ~$25/dev/mo ([Snyk Pricing](https://snyk.io/plans/)). Open-source maintainers can apply for the **Snyk Secure Developer Program** to receive free Enterprise-tier entitlements on non-corporate OSS repos ([Snyk Open Source](https://snyk.io/open-source/)).
- **Privacy:** Snyk states that customer source code is **never used to train** DeepCode AI models (models are trained on curated permissively licensed OSS + Snyk researcher fixes).

#### Aikido Security, Mobb, SonarQube Cloud, Amazon Q Developer
*Checked 2026-11-26.*
- **Aikido Security** ([aikido.dev/pricing](https://www.aikido.dev/pricing)): Consolidated AppSec (SAST, SCA, secrets, IaC + AI AutoFix) with a **$0 Free Developer Plan** (up to 2 users, 10 repos, capped monthly AI credits; paid ~$315–$350+/mo). Supports Rust SCA/SAST; no native Wasm.
- **Mobb** ([mobb.ai/pricing](https://mobb.ai/pricing), [github.com/mobb-dev](https://github.com/mobb-dev)): Automated vulnerability remediation layer sitting on top of CodeQL/Snyk/Checkmarx rather than a native scanner. **$0 forever for public OSS repos**, but remediation language support skews heavily toward Java, JS/TS, and C#; limited Rust coverage compared to GitHub Copilot Autofix.
- **SonarQube Cloud** ([sonarsource.com/products/sonarcloud](https://www.sonarsource.com/products/sonarcloud/)): **$0 for public OSS repos** with native Rust static analysis + Clippy integration, but its **AI CodeFix** feature is gated behind paid Team/Enterprise plans.
- **Amazon Q Developer** *(successor to CodeGuru Security)* ([aws.amazon.com/q/developer/pricing](https://aws.amazon.com/q/developer/pricing/)): **$0 perpetual Free Tier** via AWS Builder ID (capped at 50 agentic requests/mo; Pro $19/user/mo). Supports Rust inline IDE security scans, but is IDE-bound and offers less depth than CodeQL + Claude Code Pro.

---

### 1.2 Open-Weight / Self-Hostable Security Models & Hosted Endpoints

#### Cisco Foundation-Sec (`Foundation-Sec-8B`, `1.1-8B-Instruct`, `8B-Reasoning`)
*Checked 2026-11-26.*
- **What it is:** Open-weight 8-billion-parameter cybersecurity LLMs developed by Cisco's Foundation AI team via continued pre-training of **Meta Llama 3.1 8B** on ~5.1B tokens of cybersecurity corpora (CVEs, CWEs, MITRE ATT&CK, CAPEC, GHSA, NIST guides, CTI reports, cloud/K8s security docs). Available in Base, Instruct, and Reasoning (SFT + RLVR) variants on Hugging Face:
  - [huggingface.co/fdtn-ai/Foundation-Sec-8B](https://huggingface.co/fdtn-ai/Foundation-Sec-8B)
  - [huggingface.co/fdtn-ai/Foundation-Sec-1.1-8B-Instruct](https://huggingface.co/fdtn-ai/Foundation-Sec-1.1-8B-Instruct)
  - [huggingface.co/fdtn-ai/Foundation-Sec-8B-Reasoning](https://huggingface.co/fdtn-ai/Foundation-Sec-8B-Reasoning)
- **What it does well vs poorly:** Excels for its size on **SOC/CTI tasks** (CTI-MCQA, root-cause mapping, CVE-to-CWE classification, alert triage, IaC/config review), matching or beating Llama 3.1 70B / GPT-4o-mini on domain CTI benchmarks. However, an 8B parameter model is **not competitive with frontier models** (Claude Opus/Sonnet) for deep multi-file Rust compiler soundness, RC lifetime reasoning, or Wasm sandbox escape analysis.
- **Cost & Hosting:** Weights are **$0** under a permissive **Apache-2.0 licence** (P-11 compatible). Cisco does **not** operate a free public API for it; hosted inference via **Hugging Face Inference Endpoints** costs roughly **$0.40+/hour** on a mid-tier GPU (HF PRO at $9/mo includes only $2/mo compute credit), and continuous cloud hosting runs ~$1,000+/mo. Self-hosting locally via Ollama/vLLM is free if you own a GPU, but per [`docs/roadmap/oxiscript.md`](../roadmap/oxiscript.md) (`8.Bench`), the owner's dev box is an i5-3450 with `llvmpipe` (no dedicated GPU), making local 8B inference too slow for routine BAD ACTOR use.

#### WhiteRabbitNeo / Kindo Deep Hat
*Checked 2026-11-26.*
- **What it is:** A family of uncensored red-team/blue-team fine-tuned LLMs (Llama-3 / Llama-3.1 / Qwen-2.5-Coder bases, 7B–70B) trained on offensive/defensive security, pentesting, and IaC datasets ([huggingface.co/WhiteRabbitNeo](https://huggingface.co/WhiteRabbitNeo)). Commercially evolved into **Deep Hat** inside **Kindo**'s enterprise AI agent platform ([deephat.ai](https://www.deephat.ai/), [kindo.ai](https://www.kindo.ai/)).
- **Cost & Hosting:** Open weights on Hugging Face are **$0** to self-host (same GPU constraint as Foundation-Sec); hosted inference via HF is pay-as-you-go; Kindo/Deep Hat commercial hosting is sales-led enterprise (historical community/pro SaaS wrapper tiers ~$20/user/mo `[unverified]`).
- **Licence & Fit:** Released under **custom open-weight licences** combining base model terms (Llama) with WhiteRabbitNeo Extended Version use restrictions (non-OSI, **not** P-11 permissive). Its main selling point — uncensored exploit/pentest framing — solves a problem OxiScript doesn't have: Claude already writes adversarial Rust/OxiScript negative tests, fuzzers, and sandbox probes on our own repository without refusal.

---

### 1.3 Commercial / Enterprise-Only Security AI Platforms & Agents

| Service | Vendor | Primary Capability | Rust / Wasm | Pricing & Public Availability (checked 2026-11-26) | Why Out of Scope Now |
|---|---|---|---|---|---|
| **Codex Security** *(formerly **Aardvark**)* | OpenAI | Autonomous GPT-5 repo threat modelling, commit scanning, sandboxed exploit validation, Codex patch PRs ([Aardvark](https://openai.com/index/introducing-aardvark/), [Codex Security](https://openai.com/index/codex-security-now-in-research-preview/)) | Language-agnostic LLM (Rust yes; Wasm indirect) | Research preview for **ChatGPT Pro ($200/mo `[unverified]`), Business, Enterprise, Edu** + token/credit usage. No free tier. | Excluded by D-32 ($0 budget); revisit only if OpenAI launches a free OSS tier. |
| **Big Sleep** | Google Project Zero + DeepMind | Autonomous Gemini-powered 0-day memory-corruption hunter ([Project Zero](https://googleprojectzero.blogspot.com/)) | C/C++ OSS targets internally | **Not available to outsiders** (Google internal research/defense tool only). | Unavailable externally. |
| **Sec-PaLM / Sec-Gemini / Google SecOps** | Google Cloud | Sec-PaLM (legacy) -> **Sec-Gemini (v1/v3)** threat intel/forensics/OSV triage ([github.com/google/sec-gemini](https://github.com/google/sec-gemini)); **Gemini in SecOps** = SIEM/SOAR YARA-L & alert triage ([Google SecOps](https://cloud.google.com/security/products/security-operations)) | SOC telemetry / CTI / OSV; not Rust compiler SAST | Sec-Gemini: **invite-only trusted-tester** (free if accepted). SecOps: enterprise SIEM contract. | Wrong layer (SOC/CTI, not compiler/sandbox code auditing); Sec-Gemini is gated. |
| **Microsoft Security Copilot** | Microsoft | SOC incident triage, Defender for Cloud / IaC findings, KQL generation, malicious script de-obfuscation ([Pricing](https://azure.microsoft.com/en-us/pricing/details/security-copilot/)) | PowerShell/JS/Bash script analysis; **not** Rust/Wasm SAST | **~$4/SCU/hr** (1 SCU 24/7 floor ≈ **$2,880/mo**; or M365 E5/E7 allocation). No free tier. | Wrong domain (SOC) and far outside D-32. |
| **Checkmarx** *(Developer Assist & Triage Agents)* | Checkmarx | Enterprise SAST/SCA/ASPM + MCP/IDE AI hooks & auto-PR remediation ([Checkmarx](https://checkmarx.com/)) | **Rust: Yes** (native SAST); Wasm: No | Custom enterprise quote: **~$30k–$350k+/yr** `[unverified]`. No free OSS tier. | Prohibitively expensive under D-32. |
| **Veracode** *(Veracode Fix)* | Veracode | Enterprise SAST/SCA + zero-retention AI Fix ([Veracode Fix](https://www.veracode.com/products/veracode-fix)) | **Rust: Weak/gaps** (focus is Java/.NET/JS/C++); Wasm: No | App-based enterprise quote: **~$12k–$300k+/yr** `[unverified]`. No free OSS tier. | Prohibitively expensive and poor Rust fit. |
| **XBOW** | XBOW | Autonomous AI penetration testing of live web applications ([XBOW](https://xbow.com/)) | Live web apps (HTTP/DAST); **not** Rust/Wasm compiler | Quote-only / cloud marketplace (**~$50k/yr** attack-credit packages `[unverified]`). No free tier. | Wrong domain (web DAST) and far outside D-32. |

---

### 1.4 DARPA AIxCC Finalists & Autonomous Cyber Reasoning Systems (CRSs)

At **DEF CON 33 (August 2025)**, all seven finalists of **DARPA's AI Cyber Challenge (AIxCC)** open-sourced their autonomous Cyber Reasoning Systems ([aicyberchallenge.com](https://aicyberchallenge.com/)). Each CRS autonomously ingested repositories, discovered vulnerabilities (producing Proof-of-Vulnerability crashing inputs), and generated verified patches.

| Rank | Team | CRS Name | GitHub Repository | Licence | Implementation / Target Languages | Key Architectural Pattern |
|---|---|---|---|---|---|---|
| **1st ($4M)** | **Team Atlanta** *(GaTech, Samsung, KAIST, POSTECH)* | **Atlantis** | [Team-Atlanta/aixcc-afc-atlantis](https://github.com/Team-Atlanta/aixcc-afc-atlantis) | **MIT** ([LICENSE](https://github.com/Team-Atlanta/aixcc-afc-atlantis/blob/main/LICENSE)) | Built in Python/Rust/C; **targets C & Java** | Multi-agent LangGraph + LiteLLM ensemble combining directed fuzzing, symbolic execution, and static analysis. |
| **2nd ($3M)** | **Trail of Bits** | **Buttercup** | [trailofbits/buttercup](https://github.com/trailofbits/buttercup) *(finals: `afc-buttercup`)* | **AGPL-3.0** ([LICENSE](https://github.com/trailofbits/buttercup/blob/main/LICENSE)) | Built in Python; **targets C & Java** | Deterministic multi-agent LangGraph pipeline pairing Tree-sitter static analysis with OSS-Fuzz/libFuzzer/Jazzer and **non-reasoning** LLMs (Sonnet / GPT-4-mini class) for high cost efficiency. |
| **3rd ($1.5M)** | **Theori** | **RoboDuck** | [theori-io/aixcc-afc-archive](https://github.com/theori-io/aixcc-afc-archive) | Open-source archive (`[unverified]` exact SPDX) | Built in Python/Rust; **targets C & Java** | Stateful LLM-first agent orchestration with LiteLLM routing. |
| **4th** | **All You Need Is A Fuzzing Brain** | **FuzzingBrain** | AIxCC Archive (`[unverified]` direct URL) | Open-source archive `[unverified]` | Built in Python/Go; **targets C & Java** | Custom LLM-guided fuzzing orchestration. |
| **5th** | **Shellphish** | **Artiphishell** | [shellphish/artiphishell](https://github.com/shellphish/artiphishell) | **MIT** ([LICENSE](https://github.com/shellphish/artiphishell/blob/main/LICENSE)) | Built in Python; **targets C & Java** | Modular Kubernetes/`pydatatask` pipeline (grammar generation, static pre-screening, crash gen). |
| **6th** | **42-b3yond-6ug** | **Bug Buster** | AIxCC Archive (`[unverified]` direct URL) | Open-source archive `[unverified]` | Built in Python; **targets C & Java** | Multi-agent detection and patching loop. |
| **7th** | **Lacrosse** | **Lacrosse** | AIxCC Archive (`[unverified]` direct URL) | Open-source archive `[unverified]` | Built in Python; **targets C & Java** | Custom autonomous CRS framework. |

**Why AIxCC CRSs cannot be adopted directly today (despite being open source):**
1. **API & compute cost (violates D-32):** During the 143-hour finals, each CRS was budgeted **$50,000 in LLM API credits and $85,000 in cloud compute**. Even scaled down locally, Buttercup and Atlantis require pay-as-you-go LLM API keys (Anthropic/OpenAI/Gemini) and Docker/Kubernetes fuzzing infrastructure — they do not run on a consumer Claude Code Pro subscription.
2. **Target-language mismatch:** Competition targets were strictly **C** (via libFuzzer/ASan) and **Java** (via Jazzer). None support Rust `cargo-fuzz` or OxiScript `.oxs`/Wasm out of the box without non-trivial engineering. *(Note: once Stage `8.2b` emits `w2c2` C code for the `micro` target, C-focused CRSs become technically applicable to the generated C, if an API budget ever exists.)*
3. **Licensing note (P-11):** **Buttercup is AGPL-3.0** (external-tool use only; never embed code into OxiScript). **Atlantis** and **Artiphishell** are **MIT** (safe to borrow architectural ideas or helper patterns from).

---

### 1.5 Free Open-Source Fuzzing & Continuous Scanning Programs

#### Google OSS-Fuzz, OSS-Fuzz-Gen & ClusterFuzzLite
*Checked 2026-11-26.*
- **What it does:** **OSS-Fuzz** ([github.com/google/oss-fuzz](https://github.com/google/oss-fuzz)) runs continuous, distributed, coverage-guided fuzzing (24/7 on Google-funded compute) for accepted open-source projects, automatically filing minimized reproducers and deduplicated crash reports. **OSS-Fuzz-Gen** ([github.com/google/oss-fuzz-gen](https://github.com/google/oss-fuzz-gen)) uses LLMs (Gemini/OpenAI) to automatically synthesize, compile, and evaluate new fuzz harnesses to expand coverage.
- **Rust support:** Native first-class **Rust** support via `cargo-fuzz` (wrapping LLVM `libFuzzer` + AddressSanitizer) using `gcr.io/oss-fuzz-base/base-builder-rust` ([OSS-Fuzz Rust docs](https://google.github.io/oss-fuzz/getting-started/new-project-guide/rust-lang/)).
- **Eligibility & Zero-Cost Stepping Stone:** Acceptance into Google-hosted OSS-Fuzz requires an OSI-approved licence **and** demonstrated significance/wide user base in critical open-source infrastructure — an early `v0.0.x` compiler like OxiScript will likely be **deferred until it has external adoption**. However, **ClusterFuzzLite** ([google.github.io/clusterfuzzlite](https://google.github.io/clusterfuzzlite/)) is free, Apache-2.0 licensed, runs directly inside GitHub Actions on PRs and nightly schedules, and uses the **exact same `cargo-fuzz` harnesses and Dockerfile layout** as OSS-Fuzz — giving us immediate CI fuzzing today and a one-PR migration path to OSS-Fuzz later.

---

## 2. Detailed Comparison Matrix (Capabilities, Languages, Privacy & Licensing)

| Tool / System | Detect | Triage | Patch / Fix | Fuzz Harness Gen | Exploit / PoC Gen | Rust | Wasm / `.wat` | Public GitHub Repo Free? | Privacy / Code Sent to Vendor | OSS Licence (P-11 Check) |
|---|---|---|---|---|---|---|---|---|---|---|
| **Claude Code `/security-review`** | Yes | Yes | Yes | Yes (manual) | Yes (neg tests) | Strong | Strong (`.wat`/spec) | Yes (uses Pro sub) | Sent to Anthropic (consumer Pro terms unless Enterprise) | Proprietary SaaS/CLI (external tool OK) |
| **GitHub CodeQL + Copilot Autofix** | Yes | Partial | Yes | No | No | **Yes (GA)** | No (source Rust / `w2c2` C) | **Yes ($0)** | Runs on GitHub; Autofix **does not train** on code | Queries MIT; CLI proprietary (external CI OK) |
| **Semgrep CE (CLI)** | Yes | No | No | No | No | **Yes** (1-file) | Custom `.wat` rules | **Yes ($0)** | **Local/CI only** (zero code leaves runner) | **LGPL-2.1** (external CI OK; never embed) |
| **Semgrep Assistant (Cloud)** | Yes | Yes | Yes | No | No | Yes (Pro cross-file) | Custom `.wat` rules | Limited (<=10 devs, ~60 AI cr/mo `[unverified]`) | Sent to Semgrep + LLM subprocessors | Proprietary SaaS |
| **OSS-Fuzz + OSS-Fuzz-Gen** | Yes (crashes) | Dedup | No | **Yes (Gen)** | Crash inputs | **Yes** (`cargo-fuzz`) | Indirect (fuzz parser/runtime) | **Yes ($0)** if eligible (ClusterFuzzLite always $0) | Public OSS fuzzing on Google infra / GitHub Actions | **Apache-2.0** (P-11 compatible) |
| **Socket AI** | Yes (SCA) | Yes | Dependency | No | No | **Yes (GA)** (`Cargo`, `build.rs`, `unsafe`) | Indirect (npm `.wasm`) | **Yes ($0)** (+ OSS Business grant) | Public repo manifests/diffs scanned on Socket | Proprietary SaaS |
| **Corgea** *(+ Sighthound)* | Yes | Yes | Yes | No | No | **Yes** | No | Limited (2 devs, 10 PRs/mo `[unverified]`) | Sent to Corgea SaaS (Sighthound is local) | SaaS proprietary; **Sighthound is MIT** (P-11 OK) |
| **ZeroPath** | Yes | Yes | Yes | No | No | **Yes** | No | Limited free tier / researcher access | Sent to ZeroPath SaaS | SaaS proprietary; MCP server OSS `[unverified]` |
| **Snyk DeepCode AI** | Yes | Yes | Yes | No | No | Partial/Yes `[unverified]` | SCA only | Capped Free / OSS Enterprise grant | Sent to Snyk; **no training** on customer code | Proprietary SaaS |
| **Aikido Security** | Yes | Yes | Yes | No | No | Yes | No | Free Dev (2 users, 10 repos, capped AI) | Sent to Aikido SaaS | Proprietary SaaS |
| **Mobb** | No (ingests) | Yes | Yes | No | No | Limited | No | **Yes ($0)** for public OSS | Sent to Mobb SaaS | Proprietary SaaS |
| **SonarQube Cloud** | Yes | Partial | Paid only | No | No | **Yes** | No | **Yes ($0)** (no AI CodeFix) | Sent to SonarCloud | Open-core / LGPL parts; Cloud proprietary |
| **Amazon Q Developer** | Yes (IDE) | Partial | Yes (IDE) | No | No | Yes | No | Capped Free (50 reqs/mo) | Sent to AWS (Free tier telemetry opt-out check) | Proprietary SaaS |
| **Cisco Foundation-Sec-8B** | Weak (code) | **Strong (SOC/CWE)** | Weak | No | No | Weak | No | Weights free; no free hosted API | Local = private; HF Endpoint = cloud | **Apache-2.0** (P-11 compatible) |
| **WhiteRabbitNeo / Deep Hat** | Moderate | Moderate | Moderate | No | Yes (pentest) | Weak | No | Weights free; hosted paid | Local = private; Kindo = enterprise | Custom Llama + WRN restrictions (non-P-11) |
| **Team Atlanta Atlantis** | Yes | Yes | Yes | Yes | Yes (PoV) | Internal only (C/Java targets) | No | Code free; **API keys paid** | Sent to configured LLM APIs | **MIT** (P-11 compatible) |
| **Trail of Bits Buttercup** | Yes | Yes | Yes | Yes | Yes (PoV) | No (C/Java targets) | No | Code free; **API keys paid** | Sent to configured LLM APIs | **AGPL-3.0** (external only; never embed) |
| **OpenAI Codex Security** | Yes | Yes | Yes | No | Yes (sandbox) | Yes (LLM) | Indirect | **No** (Pro/Business/Enterprise) | Sent to OpenAI cloud | Proprietary SaaS |
| **Anthropic Claude Security / Mythos** | Yes | Yes | Yes | No | Yes | Yes (LLM) | Yes (LLM) | **No** (Enterprise / Glasswing partners) | Sent to Anthropic | Proprietary SaaS |
| **Google Big Sleep** | Yes | Yes | No | No | Yes | C/C++ (internal) | No | **No** (internal only) | Google internal | Proprietary internal |
| **Checkmarx / Veracode / MS Sec Copilot / XBOW** | Varies | Yes | Yes (Cx/Vc) | No | XBOW (web) | Cx yes; Vc weak; others no | No | **No** (enterprise only) | Vendor cloud | Proprietary enterprise |

---

## 3. Evidence of Effectiveness & Critical Limitations

### 3.1 Real-World Vulnerabilities & CVEs Found

Published evidence proves that **agentic systems combining frontier models with execution/fuzzing** do find real zero-days — whereas standalone static fine-tuned classifiers do not:

1. **Google Project Zero & DeepMind — Big Sleep:**
   - **Late 2024:** Autonomously discovered an exploitable stack buffer underflow in **SQLite** (pre-release) that traditional fuzzing had missed — widely cited as the first real-world memory-corruption bug found by an AI agent ([Google Project Zero](https://googleprojectzero.blogspot.com/)).
   - **Mid-2025:** Discovered **CVE-2025-6965** in SQLite (a flaw already known to threat actors and poised for exploitation), enabling preemptive patching ([Google Blog](https://blog.google/)).
   - **August 2025:** Large-scale sweeps discovered **20 new vulnerabilities** across widely deployed open-source C/C++ projects including FFmpeg, ImageMagick, and Redis.
2. **Google OSS-Fuzz & OSS-Fuzz-Gen:**
   - **OSS-Fuzz** overall has surfaced **13,000+ vulnerabilities and 50,000+ bugs** across 1,300+ open-source projects ([github.com/google/oss-fuzz](https://github.com/google/oss-fuzz)).
   - **OSS-Fuzz-Gen** (Nov 2024) used LLMs to write new fuzz targets, uncovering **26 new vulnerabilities** in real-world OSS projects, including **CVE-2024-9143** (a decades-old out-of-bounds write in OpenSSL) ([Google Online Security Blog, 2024-11](https://security.googleblog.com/2024/11/leveling-up-fuzzing-finding-more.html)).
3. **DARPA AIxCC Finals (DEF CON 33, August 2025):**
   - Across 54 million lines of C and Java open-source code, the 7 finalist CRSs discovered **54 of 63 synthetic vulnerabilities** (patching **43**) and organically discovered **18 real-world zero-day vulnerabilities** (6 in C, 12 in Java), producing automated patches for **11** of them ([DARPA](https://www.darpa.mil/), [AIxCC](https://aicyberchallenge.com/)).
   - Trail of Bits' **Buttercup** alone found **28 unique vulnerabilities**, delivered **19 verified patches** (~90% patch accuracy) across 20 CWEs, and did so using cheaper non-reasoning models paired with deterministic static analysis and fuzzing.
4. **OpenAI Aardvark -> Codex Security (Oct 2025 – Mar 2026):**
   - OpenAI reported that during private beta across 1.2 million commits, Codex Security surfaced **792 critical** and **10,561 high-severity** findings, yielding multiple CVEs across Chromium, OpenSSL/OpenSSH, GnuTLS, GnuPG, PHP, and GOGS ([Codex Security Preview](https://openai.com/index/codex-security-now-in-research-preview/)).
5. **Anthropic Project Glasswing / Claude Mythos Preview (Apr – late 2026) — and the False-Positive Reality Check:**
   - Anthropic reported that Project Glasswing partners found **10,000+ high/critical issues** in the first month (and >33,000 high/critical in later cumulative sweeps across partner repos), including named findings such as FreeBSD **CVE-2026-4747** ([Project Glasswing](https://www.anthropic.com/news/project-glasswing)).
   - **Crucial caveat (late-2026 independent critique):** Analyses by VulnCheck and open-source maintainers (e.g. `curl`) highlighted a wide gulf between raw AI-flagged volume and formally assigned, actionable CVEs: automated AI vulnerability reports sent upstream suffered high rejection and false-positive rates, imposing substantial triage overhead unless backed by a **concrete, crashing or executable reproducer** — validating OxiScript's own BAD ACTOR rule that every finding must become an executable regression test!

### 3.2 Benchmarks & Academic Evaluations

| Benchmark / Paper | Citation | What It Measures | Key Result & Takeaway for OxiScript |
|---|---|---|---|
| **PrimeVul** (Ding et al., ICSE 2025) | [`arXiv:2403.18624`](https://arxiv.org/abs/2403.18624) | Deduplicated, chronologically split C/C++ vulnerability detection with realistic false-positive penalty (*VD-Score*) | A fine-tuned 7B code LM drops from **68.26% F1** on legacy BigVul to **3.09% F1** on PrimeVul. **Lesson:** Do not trust small fine-tuned binary vulnerability detectors; older benchmark hype was driven by duplicates and data leakage. |
| **SEC-bench** & **SEC-bench Pro** | [`arXiv:2506.11791`](https://arxiv.org/abs/2506.11791), [`arXiv:2605.26548`](https://arxiv.org/abs/2605.26548) | End-to-end LLM agents (SWE-agent, OpenHands, Aider) on containerized real-world PoC generation and vulnerability patching | Top agents reach at most **~18% PoC generation** and **~34% patching** success. **Lesson:** Even strong coding agents fail most hard end-to-end security tasks without human/tool scaffolding and deterministic test harnesses. |
| **CVE-Bench** | [`arXiv:2503.17332`](https://arxiv.org/abs/2503.17332) | Autonomous AI agent exploitation (and repair variant) of real-world web CVEs in containers | Exposes **multi-file context blindness** and **poor security-tool usage** (agents misread SAST output and enter patch-verification loops). |
| **CyberSecEval 1 / 2 / 3** (Meta) | [`arXiv:2312.04724`](https://arxiv.org/abs/2312.04724), [`2404.13161`](https://arxiv.org/abs/2404.13161), [`2408.01605`](https://arxiv.org/abs/2408.01605) | Insecure code generation rate, exploitability, and prompt-injection susceptibility across LLMs | Confirms both that LLMs routinely emit subtle insecure patterns when writing code (justifying independent BAD ACTOR review of Gemini-written code per D-31!) and that CI agents are vulnerable to **prompt injection**. |

### 3.3 Why Most Commercial Security AI Misses OxiScript's Threat Model

Mapping [`docs/research/oxiscript-known-attacks.md`](oxiscript-known-attacks.md) against commercial AI scanners shows a sharp structural mismatch:

| OxiScript Attack Surface (`oxiscript-known-attacks.md`) | Caught by Commercial AI SAST (CodeQL / Semgrep / Snyk / Corgea)? | Caught by Coverage-Guided Fuzzing (`cargo-fuzz` / ClusterFuzzLite)? | Caught by Frontier-Model BAD ACTOR (Claude Opus + `/security-review`)? |
|---|---|---|---|
| **1. Type-system unsoundness** (variance, narrowing invalidation, generic constraint holes, use-before-init) | **No** (domain-specific compiler invariants) | Partial (only if differential / panic oracle exists) | **Yes** (reads spec + checker + writes counterexample `.oxs`) |
| **2. Lowering & codegen logic bugs** (side-effect modelling, bounds-check elision, Wasm `div`/`trunc` edge semantics, `-0`/NaN) | **No** | **Strong** (with differential Stub-vs-Linked & opt-vs-unopt oracles) | **Yes** (designs targeted differential probes) |
| **3. RC memory runtime** (`oxiscript-rc`: UAF, cycle collector OOM headroom, drop re-entrancy, refcount overflow) | **Partial** (CodeQL/Corgea check `unsafe` pointers & integer overflows in Rust) | **Strong** (fault-injection sweep + `stale_object` poison checks) | **Yes** (already caught BL01–BL10 in `8.M-link`!) |
| **4. `wasmi` host boundary & metering** (fuel gaps on bulk ops/host calls, `(ptr, len)` overflow, reentrancy across `memory.grow`, capability forging) | **Partial** (CodeQL/Semgrep custom rules can flag unchecked `ptr + len` or `unsafe` in `oxiscript-runtime`) | **Strong** (fuzzed `.wasm` bytes + hostile guest modules) | **Yes** (reasons across ABI contract + Rust host code) |
| **5. Parser / tooling / terminal injection** (Trojan Source bidi, deep nesting stack overflow, formatter semantic drift, ANSI/OSC escape injection in OxiTerm) | **Partial** (Semgrep custom rules catch unsanitised terminal prints & regexes) | **Very Strong** (`parse(fmt(x)) == parse(x)`, arbitrary-byte no-panic fuzzing) | **Yes** (checks spec edge cases & diagnostics paths) |
| **6. Supply chain** (`wasmi` CVEs like CVE-2025-66627, malicious `build.rs`, hidden `unsafe` in future `8.N` crates) | **Yes** (**Socket AI** + `cargo audit` + `cargo deny` + Dependabot) | No | Partial (not a live CVE database) |

**Bottom line:** Commercial SAST + Socket + Fuzzing handle **classes 4–6 and Rust memory/crash hygiene continuously in CI at $0**, freeing the scarce **Claude Code Pro Opus quota** to concentrate exclusively on **classes 1–4 (compiler soundness, lowering/codegen semantics, RC invariants, and sandbox capability boundaries)** where commercial SAST is blind.

---

## 4. Recommendations for OxiScript / OxiUI

### 4.1 Top 3 Zero-Cost Options to Adopt First

Ranked strictly by signal-to-noise ratio for a Rust/Wasm compiler under **D-32 ($0 extra budget)** and **D-31/D-33 (Claude always involved)**:

1. **Claude Code `/security-review` (Included in Claude Code Pro — $0 extra)**
   - **Why first:** Already paid for, natively understands Rust + OxiScript `.oxs` + `.wat` + `oxiscript-known-attacks.md`, and directly supports D-33/D-34.
   - **How to use it:** Run `/security-review` locally in Claude Code on every non-trivial feature branch (especially Gemini-authored branches per D-31) *before* queueing the commit into a batched Opus BAD ACTOR round. This catches shallow host-boundary, overflow, and diagnostic-sanitisation bugs early so the expensive Opus BAD ACTOR round spends its context on deep soundness, RC-elision, and sandbox-escape attacks.
   - **Do NOT** enable `anthropics/claude-code-security-review` as an unguarded GitHub Action on public PRs: it requires a paid API key (violating D-32) and is vulnerable to indirect prompt injection from untrusted external PRs.

2. **GitHub Code Scanning with CodeQL for Rust (GA) + Copilot Autofix ($0 on Public GitHub Repos)**
   - **Why second:** Zero-cost, zero-maintenance semantic SAST on every push/PR with native GA Rust support (`CWE-476`/`825`, overflows, crypto, injection) and free AI autofix diffs — plus zero LLM training on our code.
   - **Bonus future payoff:** When Stage `8.2b` (`micro` target via `w2c2`) lands, CodeQL's mature C/C++ query suite can also scan generated C firmware output for undefined behaviour and integer/buffer bugs at $0.
   - **High-leverage customization:** Write 3–5 small custom CodeQL queries for `oxiscript-runtime` and `oxiscript-rc` (e.g. flagging any `unsafe` block outside allowlisted crates, unchecked `ptr + len` arithmetic on guest linear-memory indices, or unvalidated host imports).

3. **Semgrep Community Edition (LGPL-2.1 CLI in CI) + `cargo-fuzz` / ClusterFuzzLite -> OSS-Fuzz ($0)**
   - **Why third:** Deterministic, fast, zero-cloud-dependency guardrails in GitHub Actions that directly implement Stage `8.Sec` and pave the road for Stage `8.S4c`:
     - **Semgrep CE (`semgrep --config .semgrep/`):** Commit a tiny repo-local YAML ruleset encoding our exact Stage `8.Sec` invariants (no `HashMap`/`HashSet` iteration in deterministic compiler output [BL04 / `8.Sec`], no unsanitised terminal control chars in diagnostics, no `unwrap()`/`panic!()` on untrusted guest input in `oxiscript-runtime`, optional `.wat` checks on generated Wasm). Runs locally and in CI with zero data sent to any vendor.
     - **`cargo-fuzz` + ClusterFuzzLite -> OSS-Fuzz / OSS-Fuzz-Gen:** Our repository already has `.github/workflows/oxiscript-fuzz.yml` running nightly `OXI_NIGHTLY_IGNORED=1` tests. Adding proper libFuzzer `cargo-fuzz` targets (lexer/parser/formatter round-trip, type checker, lowering/codegen + `wasmparser` validation, `oxiscript-rc` alloc/drop sequences, `oxiscript-runtime` hostile `.wasm` + fuel-sweep) under **ClusterFuzzLite** in GitHub Actions gives us continuous coverage-guided fuzzing at $0 today — and makes our project turnkey-ready to apply for **Google OSS-Fuzz** (and **OSS-Fuzz-Gen**) once OxiScript gains external adoption.

*(Optional 4th zero-cost add-on, 5-minute setup: install the **Socket AI GitHub App** on the public repository to monitor `Cargo.lock` PR diffs for malicious `build.rs`, typosquatting, and silent `unsafe` introductions in `wasmi` and future `8.N` crates — complementing `cargo audit` and `cargo deny`.)*

---

### 4.2 How Each Plugs into BAD ACTOR Rounds and CI

```text
[1. Local Dev / Every Branch]
   ├─ `scripts/oxiscript-gate.sh` (unit + corpus + `cargo audit` + `cargo deny`)
   ├─ `semgrep --config .semgrep/` (local CE rules for 8.Sec invariants; <10s, $0)
   └─ Claude Code `/security-review` on branch diff (Pro sub, $0 extra; mandatory on Gemini branches per D-31)

[2. Pull Request / Push to GitHub CI]
   ├─ GitHub CodeQL (Rust GA) + Copilot Autofix ($0 public repo)
   ├─ Semgrep CE workflow (custom Rust + `.wat` rules, $0)
   ├─ Socket AI GitHub App on `Cargo.lock` / `Cargo.toml` changes ($0 public repo)
   └─ ClusterFuzzLite short PR fuzzing (e.g. 5–10 min per touched crate, $0)

[3. Nightly CI (`.github/workflows/oxiscript-fuzz.yml`)]
   ├─ Existing `OXI_NIGHTLY_IGNORED=1` heavy fuzz tests
   └─ ClusterFuzzLite batch `cargo-fuzz` corpus growth (parser, checker, codegen, RC, runtime)
      -> Later: migrate identical harnesses to Google OSS-Fuzz + OSS-Fuzz-Gen ($0)

[4. BAD ACTOR Round (Pre-Merge for Runtime/Sandbox/Safety, Batched Pre-Release per D-34)]
   ├─ Input packet: commit ranges + CodeQL/Semgrep/fuzz findings + `oxiscript-known-attacks.md`
   ├─ Optional independent Gemini security audit pass (only if paired with Claude per D-31)
   ├─ Mandatory Claude Opus BAD ACTOR round (D-33): attacks soundness, RC elision, host boundary, fuel, reload
   └─ Output: `docs/stages/rounds/<date>-<n>.md` + executable regression tests + new Semgrep/CodeQL/`8.S4c` bug shapes
```

---

### 4.3 What NOT to Rely On

1. **Small fine-tuned vulnerability classifiers (PrimeVul lesson):** Models fine-tuned as binary "vulnerable / safe" detectors collapse to near-random performance (**3.09% F1** on PrimeVul) on real, deduplicated codebases. Do not spend setup time self-hosting 7B–8B security models (`Cisco Foundation-Sec-8B`, `WhiteRabbitNeo-7B`) expecting them to find Rust compiler or Wasm sandbox bugs — they are tuned for SOC/CTI and pentesting, and our dev hardware lacks a GPU anyway.
2. **DARPA AIxCC CRSs (`Buttercup`, `Atlantis`, `Artiphishell`) as turnkey tools today:** Despite being open-sourced, they target **C and Java** and require paid LLM API budgets + heavy container compute (violating D-32). Moreover, **Buttercup is AGPL-3.0**, so none of its code may ever be embedded into OxiScript (P-11). Treat `Atlantis` (MIT) and Trail of Bits' Buttercup write-ups strictly as **design inspiration** for Stage `8.S4c` and future automated fuzz-harness generation.
3. **Enterprise SOC/AppSec/DAST platforms (`Checkmarx`, `Veracode`, `Microsoft Security Copilot`, `Google SecOps`, `XBOW`, `OpenAI Codex Security`, `Claude Security Enterprise`):** Either prohibitively expensive under D-32, unavailable to individuals (`Big Sleep`, `Project Glasswing`), or aimed at SOC telemetry / live web pentesting rather than language/runtime security.
4. **Unguarded AI GitHub Actions on public repositories:** Never wire `claude-code-security-review` or any LLM agent to automatically trigger with secrets or write permissions on untrusted external PRs — both Anthropic and CyberSecEval highlight **indirect prompt injection** as an active attack vector.
5. **Gemini (or any single third-party AI scanner) as sole security sign-off:** Strictly forbidden by **D-31** and **D-33**, and empirically backed by late-2026 findings that unverified AI alerts carry high false-positive/false-negative rates. Deterministic verification (`#![forbid(unsafe_code)]`, `wasmparser`, Stub-vs-Linked & opt-vs-unopt differential tests, `cargo-fuzz`, `8.S4a` laws) + **Claude Opus BAD ACTOR rounds with executable regression tests** remain the irreplaceable core of OxiScript's security assurance.
