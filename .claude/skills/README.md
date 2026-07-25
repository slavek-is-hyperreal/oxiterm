# Agent skills for OxiTerm

Skills in this directory are loaded automatically by Claude Code when working in
this repository. They exist because OxiTerm's two authoring languages resemble
HTML and CSS closely enough that web intuition produces documents which parse
cleanly, render wrong, and emit no error.

| Skill | Use it for |
|---|---|
| `thtml-tcss-authoring` | Any `.thtml` file, any `<style>` block, any layout that renders wrong. |
| `oxiterm-app-integration` | Anything crossing the UI ↔ App Server boundary: custom `event-htmx` actions, `/events` handlers, state patches, OAuth flows. |

Both were derived by reading `oxiterm-renderer/src/parser/`,
`oxiterm-server/src/dispatcher.rs`, and `oxiterm-server/src/session.rs` — not by
summarising `docs/`. Where a skill and a document in `docs/` disagree, the skill
is the one checked against source. Known documentation errors are listed in
`thtml-tcss-authoring/SKILL.md` §2.2 and `oxiterm-app-integration/SKILL.md` §2.1.

## Layout linter

`oxiterm check` validates parse structure only. It passes files whose styling
does nothing, because the TCSS parser discards unrecognised properties silently.
To cover that gap:

```bash
python3 .claude/skills/thtml-tcss-authoring/scripts/lint_layout.py examples/
```

| Code | Meaning |
|---|---|
| E001 | unknown TCSS property or tag |
| E002 | non-integer value for an integer property (parses to `0`) |
| E003 / E004 | bordered element shorter/narrower than its own border |
| W005 | rigid container smaller than the sum of its children |
| W006 | ambiguous-width symbol in a clickable label (hit box may shift) |
| I007 | ambiguous-width letter in a clickable label (informational) |

Exit status is non-zero when any E-level finding is present, so it can gate a
commit hook or CI job.
