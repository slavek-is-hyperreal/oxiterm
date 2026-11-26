# `oxi` command reference: JSON schema and exit codes

## Exit codes (every command)

| Code | Meaning |
|---|---|
| 0 | success (`check`: no errors, warnings allowed; `fmt --check`: nothing would change) |
| 1 | findings: errors reported by `check`, a syntax error in `fmt`, or `fmt --check` would change a file |
| 2 | usage, I/O or configuration error (missing path, unreadable or broken `oxi.toml` / `idl/*.idl`, unknown code, bad `new` arguments) |
| 3 | reserved (it once meant "not merged yet"); never reused |
| 4 | `run` only: the program started and stopped on a trap (integer overflow, bounds, division by zero, out of fuel, stack overflow, ...) |

## `oxi check [paths] [--json] [--fix] [--stdin --stdin-path P]`

`paths` are files or directories (default `.`; directories are searched for `*.oxs`, skipping hidden directories, `target` and `node_modules`). Each file is checked as an entry module together with its imports. The project root is the nearest ancestor holding `oxi.toml`; `host:` imports resolve against the built-in `host:log@1` and `host:state@1` plus `<root>/idl/*.idl` (a project IDL replaces a built-in one with the same module; two project files may not declare the same module). Relative imports (`"./util"`) and package names (`"pkg/mod"`, resolved from the root) map to `<root>/<name>.oxs`.

`--stdin` reads one source from stdin; `--stdin-path` names it (project lookup, messages, `file` in the output). `--fix` applies every `machine-applicable` fix to the files, re-checks, and repeats (at most 10 rounds), then reports what remains. Edits that overlap another accepted edit are skipped for that round. Convergence guard: a round must strictly reduce the number of syntax diagnostics (`OXS-E0001..E0099`); otherwise it is undone and `--fix` stops, so a file is never left worse than it was.

### `--json` document (schema 1)

Keys are emitted in alphabetical order; the shape is stable within schema 1 (fields may be added, never removed or retyped).

```json
{
  "schema": 1,
  "tool": "oxi",
  "version": "0.1.0",
  "ok": false,
  "fatal": null,
  "files": ["main.oxs"],
  "summary": { "errors": 1, "warnings": 0 },
  "diagnostics": [
    {
      "code": "OXS-E0005",
      "severity": "error",
      "message": "`==` does not exist in OxiScript; use `===`",
      "file": "main.oxs",
      "range": {
        "start": { "byte": 56, "line": 3, "col": 12, "utf16": 12 },
        "end":   { "byte": 58, "line": 3, "col": 14, "utf16": 14 }
      },
      "fixes": [
        {
          "title": "use `===`",
          "applicability": "machine-applicable",
          "edits": [
            { "file": "main.oxs", "new_text": "===",
              "range": { "start": { "...": 0 }, "end": { "...": 0 } } }
          ]
        }
      ]
    }
  ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `schema` | integer | document schema version (1) |
| `ok` | bool | no errors and no fatal problem |
| `fatal` | string or null | set (and `diagnostics` empty) when the run could not start (exit 2): missing path, broken `oxi.toml` or IDL |
| `notes` | string[] (optional, only when non-empty) | things `oxi` skipped or replaced without it being an error: a symlink or special file named `*.oxs`/`*.idl` that was not read, a project IDL that replaces a built-in `host:` module. Also printed as `note: ...` on stderr. Additive: schema stays 1 |
| `fix` | object (optional, only with `--fix`) | `{applied, capped, reverted, round_limit}`: fixes applied; files with more syntax errors than one check lists (fixed the first ones per round); files where a round was undone because it did not reduce the syntax errors; files where the round limit stopped with fixes pending |
| `files` | string[] | entry files checked (paths as shown to the user) |
| `summary` | object | counts of `errors` and `warnings` |
| `diagnostics[].code` | string | stable registry code; `oxi explain <code>` |
| `diagnostics[].severity` | `"error"`, `"warning"`, `"note"`, `"internal"` | derived from the code class |
| `diagnostics[].file` | string | the file the diagnostic is in (may be an imported module) |
| `range` | object | `start`/`end`, each `{byte, line, col, utf16}`: `byte` = UTF-8 offset from the start of the file, `line` 1-based, `col` 1-based in characters, `utf16` 1-based in UTF-16 code units (LSP / VS Code columns are `utf16 - 1`) |
| `fixes[].title` | string | what the fix does |
| `fixes[].applicability` | string | `machine-applicable`: apply without review (syntax-level repairs: `;`, `===`, parentheses, float suffix, `:` for a switch arm); `maybe-incorrect`: a proposal that may need a human (type-level changes, stub `switch` cases, several alternatives for one diagnostic, the ambiguous `?`, `for...in` to `for...of` because keys become values, and any `;` offered on a line with `throw`/`try`/`catch`); `advice`: text only, `edits` is empty |
| `fixes[].edits[]` | object | `{file, range, new_text}`: replace the bytes of `range` by `new_text` (an insertion has `start == end`); edits of one fix apply together; ranges are in the pre-edit file |

`--fix-only semicolons` (instead of `--fix`; not with `--json`) applies only the machine-applicable insertions of a missing `;` (`OXS-E0004`), with the same convergence guard; with `--stdin` it prints the fixed source on stdout (exit 0) and writes nothing. `OXS-E0004` fixes on a line that starts like a continuation (`(`, `[`, a backtick, an operator) are `maybe-incorrect`, because `a` newline `[1]` already parses as `a[1]`.

Diagnostics are sorted by file then position and de-duplicated across entry files.

## `oxi fmt [paths] [--check] [--stdin]`

Canonical formatter (`oxiscript-syntax`), no options, idempotent. A BOM, the file's line-ending style (CRLF stays CRLF) and the text of `//` / `///` comments are kept; raw control and invisible characters inside string literals are written as escapes. Files with syntax errors are not touched and exit 1 (`oxi check --fix` repairs the safe ones: the missing `;`, ...). Files are replaced atomically; symlinks and special files are skipped. `--check` lists files that would change (`would reformat <path>`) and writes nothing. `--stdin` formats stdin to stdout (exit 1 and nothing on stdout when it has syntax errors).

## `oxi explain <code>`

Prints the registry entry (title, severity, phase, message template, fix) and, for the common codes, a failing example with its fixed form. `OXS-` prefix and case are optional. Unknown code: exit 2.

## `oxi new <name> [--template blink|touch-button|sensor-dashboard|oxiui-page]`

Creates `./<name>/` (must not exist) with `main.oxs`, `oxi.toml`, `idl/*.idl` for the template's placeholder capabilities, `AGENTS.md`, `.vscode/extensions.json`, `.vscode/settings.json`. Every template passes `oxi check` and `oxi fmt --check`.

`oxi.toml`:

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "2026"
needs = ["host:log@1", "host:state@1"]   # host interfaces main receives, in parameter order
```

## `oxi run [file] [--fuel N]`, `oxi build [file] [-o out.wasm]`

Both type-check the entry (default `main.oxs`) exactly like `oxi check` (errors: printed the same way, exit 1; a lowering error exits 1 too), then lower it to Wasm.

* `build` writes the module to `-o FILE` (default `<package name>.wasm` from `oxi.toml`, else `<file stem>.wasm`, in the current directory) and exits 0. The package name must match `[A-Za-z0-9_-]{1,64}` when it is used for the default name (otherwise exit 2, nothing written). The output is written atomically (temp file in the same directory, then rename); a symlink, directory, FIFO or device at the output path is refused (exit 2), an existing regular file is replaced. An unwritable output exits 2.
* `run` runs `main` in `oxiscript-runtime` (wasmi) with the same capabilities and limits as the browser playground: `host:log@1` (levels 0-2 to stdout, 3 and 4 to stderr prefixed `warn:` / `error:`; at most 1000 lines / 64 KiB, the rest is counted and reported) and `host:state@1` (in memory, per run). `--fuel N` sets the instruction budget (default 10000000; `--fuel 0` means the default, like the browser playground). Output is printed when the program stops, not live. A closed stdout/stderr (`oxi run x | head -1`) ends the output quietly with exit 0 (like common CLIs); any other write error (for example a full disk, `/dev/full`) exits 2 with `oxi run: cannot write output: ...`; it never panics. Exit 0 when `main` returns, 4 when it trapped (stderr: `oxi run: stopped: <reason> [OXS-E04xx]`; the log lines written before the trap are still printed), 2 when the program needs a host interface `oxi run` does not provide (use `build`).
* Limitation: the current runtime cannot convert an `f64` to text (`${x}` with a float traps as "unreachable"); convert to an integer first.
