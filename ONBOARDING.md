# Where to start

| You are | Read |
|---|---|
| Someone new to the project | [README.md](README.md) |
| An AI agent writing OxiScript (`.oxs`) | [AGENTS.md](AGENTS.md) and the skill `.claude/skills/oxiscript-authoring/` |
| An AI agent writing THTML/TCSS or an App Server | the skills `.claude/skills/thtml-tcss-authoring/` and `.claude/skills/oxiterm-app-integration/` ([index](.claude/skills/README.md)) |
| Taking over as lead | [docs/HANDOFF.md](docs/HANDOFF.md), then the roadmaps [docs/roadmap/](docs/roadmap/README.md) |
| Changing the OxiScript compiler | [docs/oxiscript/contracts.md](docs/oxiscript/contracts.md) (what each compiler stage may rely on), then the crate's own docs |

Rules that hold everywhere:
- **Code beats docs.** Where a document and the code disagree, the code is right and the document is a bug: report it, do not silently "fix" the code to match.
- **A test for every safety check must hit the failing branch**, not only the happy path. `if let Some(x) = … { assert!(…) }` without `expect` is an empty test.
- **Check your claims independently** before reporting them (run the tests yourself; after a push, look at CI).
