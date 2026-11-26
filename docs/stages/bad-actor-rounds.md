# BAD ACTOR rounds

Owner rule (2026-11-26, roadmap D-33/D-34): the BAD ACTOR is Claude: Opus for large stages and releases, Sonnet with Opus consultation for small changes. To save cost, several changes may be **collected into one round** and attacked by one Opus agent.

Rules:
- A change waiting for a round may be merged to `main` after its gate (`scripts/oxiscript-gate.sh merge` or the OxiUI gate) and a review, **but no release tag is made while any change in it is still waiting for its round.**
- A large stage that touches the runtime, memory, sandbox, capabilities or the compiler's safety checks gets its round before merge, unless the owner says otherwise.
- A round lists its changes (commit ranges), gets one report `docs/stages/rounds/<date>-<n>.md`, and every finding becomes a regression test.

## Waiting for a round

| Change | Merged | Why it waits | Planned round |
|---|---|---|---|
| 8.Std v0 (std library) | fd7bf41 | had a Sonnet+advisor pass; large stage, Opus pass owed | next Opus round |
| Playground "no main" notice | 0bd281a | small change, no BAD ACTOR yet | next round |
| OxiUI "How it works" diagram and statuses | 31e8075 | small change, static page | next round |
| Taller live demos (CSS) | (sites commit) | small change | next round |
| Versioning rule: std counts, `--owner-release` | d624ff5 | small change in a release script | next round |

## Rounds done

| Round | Changes | Model | Report |
|---|---|---|---|
| 8.M-link round 2 | gemini-rtlink d6b6afd | Opus | running (2026-11-26) |
