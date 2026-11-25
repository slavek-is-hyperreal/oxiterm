#!/usr/bin/env python3
"""Regenerate the copies of the agent guidance from its single source.

Source of truth: `oxiscript/oxiscript-cli/templates/common/AGENTS.md` (the block between the
`oxs-core` markers) and `oxiscript/oxiscript-abi/registry/codes.txt` (code titles).
Writes: root `AGENTS.md` and `docs/oxiscript/llms.txt`.

    python3 -I oxiscript/oxiscript-cli/tools/sync_docs.py

`cargo test -p oxiscript-cli` fails when the root AGENTS.md core drifts from the template.
"""
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../.."))


def read(p):
    with open(os.path.join(ROOT, p), encoding="utf-8") as f:
        return f.read()


def write(p, s):
    with open(os.path.join(ROOT, p), "w", encoding="utf-8") as f:
        f.write(s)


tpl = read("oxiscript/oxiscript-cli/templates/common/AGENTS.md")
core = re.search(r"<!-- oxs-core:begin -->.*?<!-- oxs-core:end -->", tpl, re.S).group(0)
core_plain = core.replace("<!-- oxs-core:begin -->\n", "").replace("\n<!-- oxs-core:end -->", "")

ROOT_MD = """# AGENTS.md

Guidance for AI coding agents working in this repository. This file covers **OxiScript** (the `.oxs` language, its checker and the `oxi` tool). THTML/TCSS authoring has its own skill in `.claude/skills/thtml-tcss-authoring/`.

## OxiScript in this repo

| What | Where |
|---|---|
| Language reference (grammar, deviations from TypeScript) | `docs/oxiscript/grammar.md` (section 6 = the differences) |
| Host interfaces (IDL), ABI | `docs/oxiscript/abi.md`, `oxiscript/oxiscript-abi/idl/` |
| Reason codes (`OXS-E0005`, ...), append-only | `oxiscript/oxiscript-abi/registry/codes.txt`; `oxi explain <code>` |
| Parser, formatter | `oxiscript/oxiscript-syntax` |
| Type checker | `oxiscript/oxiscript-check` |
| The `oxi` CLI, JSON schema, templates | `oxiscript/oxiscript-cli` (`SCHEMA.md`) |
| VS Code extension | `editors/vscode-oxiscript` |
| Model-oriented spec (one page) | `docs/oxiscript/llms.txt` |
| Skill for writing OxiScript | `.claude/skills/oxiscript-authoring/` |

Run the tool: `cargo run -p oxiscript-cli --bin oxi -- check path/to/file.oxs --json`. In a git worktree use per-package cargo (`-p oxiscript-cli`, `-p oxiscript-check`); workspace-wide commands fail there on missing server assets.

""" + core + """

## Rules for changing the compiler crates

* Reason codes are append-only: never renumber, reuse or change the meaning of one. Add the code to `codes.txt` and a test that triggers it.
* A diagnostic with a mechanical repair carries a `Fix` with edits; `oxi` marks syntax-level fixes `machine-applicable`. Keep the edits of one fix non-overlapping.
* Every ```oxs block in this file, in `docs/oxiscript/llms.txt` and in the skill is compiled by `cargo test -p oxiscript-cli`; keep them passing, and mark intentionally failing examples ```oxs-err OXS-Exxxx.
* The core block above is copied from `oxiscript/oxiscript-cli/templates/common/AGENTS.md`; edit it there and run `python3 -I oxiscript/oxiscript-cli/tools/sync_docs.py`.
"""


def code_index():
    out = []
    for rec in read("oxiscript/oxiscript-abi/registry/codes.txt").split("\n\n"):
        f = dict(
            l.split(" ", 1) for l in rec.splitlines() if l and not l.startswith("#") and " " in l
        )
        if f.get("phase") in ("lex", "parse", "check") and "code" in f:
            out.append("- %s %s" % (f["code"], f["title"]))
    return "\n".join(out)


LLMS = """# OxiScript

> OxiScript is a statically typed language that reads like TypeScript and compiles to WebAssembly. Source files are `.oxs`; the toolchain is the `oxi` command. This page is everything a model that already knows TypeScript needs: the differences, the check loop, and examples that compile.

## Tooling

```text
oxi check --json [paths]   # diagnostics as JSON (schema 1): code, severity, message, file, range, fixes
oxi check --fix            # apply every machine-applicable fix, rewrite the files
oxi fmt [--check] [paths]  # canonical formatter, no options
oxi explain OXS-E0100      # a code's meaning, example and fix
oxi new NAME --template blink|touch-button|sensor-dashboard|oxiui-page
```

Loop: edit, `oxi check --json`, apply fixes with `"applicability": "machine-applicable"`, repeat until `"ok": true`. `maybe-incorrect` fixes are proposals to read; `advice` has no edit. Syntax errors hide type errors, so expect a second wave.

""" + core_plain + """

## Diagnostic codes the checker emits

""" + code_index() + """

## More

- Grammar and the full list of deviations from TypeScript: docs/oxiscript/grammar.md
- Host interface definition language: docs/oxiscript/abi.md
- Project guide for agents: AGENTS.md; Claude Code skill: .claude/skills/oxiscript-authoring/SKILL.md
"""

if __name__ == "__main__":
    write("AGENTS.md", ROOT_MD)
    write("docs/oxiscript/llms.txt", LLMS)
    print("wrote AGENTS.md and docs/oxiscript/llms.txt", file=sys.stderr)
