# BAD ACTOR rounds

Owner rule (2026-11-26, roadmap D-33/D-34): the BAD ACTOR is Claude: Opus for large stages and releases, Sonnet with Opus consultation for small changes. To save cost, several changes may be **collected into one round** and attacked by one Opus agent.

Rules:
- A change waiting for a round may be merged to `main` after its gate (`scripts/oxiscript-gate.sh merge` or the OxiUI gate) and a review, **but no release tag is made while any change in it is still waiting for its round.**
- A large stage that touches the runtime, memory, sandbox, capabilities or the compiler's safety checks gets its round before merge, unless the owner says otherwise.
- A round lists its changes (commit ranges), gets one report `docs/stages/rounds/<date>-<n>.md`, and every finding becomes a regression test.

## Waiting for a round

| Change | Merged | Why it waits | Planned round |
|---|---|---|---|
| (nothing waiting) | | | |

Everything listed here before 2026-11-27 was covered by round 2026-11-27-1 (below).

## Rounds done

| Round | Changes | Model | Report |
|---|---|---|---|
| 8.M-link round 2 | gemini-rtlink d6b6afd | Opus | docs/stages/8.M-link/bad-actor-2.md (fixed in dab81b0, merged a17d1ef) |
| 2026-11-27-1 | 8.Spec+8.Sec+8.Fix, 8.Std v0 (Opus pass), OxiPerf v0, 0bd281a, d624ff5, 31e8075, demo CSS | Opus | [rounds/2026-11-27-1.md](rounds/2026-11-27-1.md): 0 high, 3 medium (R1-03, R1-04, R1-40), 10 low; fixes on branches gemini-fixspecsec 3de5e9e, ba-round1 a0ed1ab |
| 2026-11-27-2 | sites-deploy workflow c137360, fixes of round 2026-11-27-1 | Sonnet + Opus advisor | [rounds/2026-11-27-2.md](rounds/2026-11-27-2.md): 1 medium (R2-10), 4 low, 1 info; all fixed in 4ae4707 |
