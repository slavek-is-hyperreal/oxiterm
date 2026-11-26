# OxiScript for VS Code

Support for OxiScript (`.oxs`), the TypeScript-shaped language that compiles to WebAssembly.

* Syntax highlighting (TextMate grammar): keywords, numeric literals with type suffixes (`200u8`, `1.5f32`), strings and template literals, attributes (`@expect`, `@@profile`), `law`, and the whitespace rule of `?` (try `f()?`, conditional ` ? : `, optional chaining `?.`). Reserved and removed words (`any`, `var`, `typeof`, ...) and `==` are highlighted as errors.
* Diagnostics from `oxi check --json` while you type (unsaved buffer, 300 ms debounce) or on save, with the stable `OXS-` codes.
* Quick fixes from the diagnostics' fixes: safe ones (`;`, `===`, parentheses) are preferred; proposals say "check the result". "Fix all" applies every safe fix at once.
* Semicolon assistant: the missing-`;` diagnostic (`OXS-E0004`) sits at the end of the statement, a quick fix "Add missing semicolons in this file" inserts all of them, and on save the extension inserts them for you (`oxiscript.semicolons.fixOnSave`, through `oxi check --fix-only semicolons`; only `;`, never where it would change the meaning: lines starting with `(`, `[`, a backtick or an operator, `throw`/`try`/`catch` lines; other problems are left alone).
* Format on save through `oxi fmt` (no options). Format needs a file without syntax errors, so the `;` fix runs on save too.
* Snippets: `main`, `mainstate`, `law`, `struct`, `class`, `union`, `result`, `state`, `fn`, `forof`.

## Requirements

The `oxi` command on `PATH` (or `oxiscript.oxiPath`). Build it from the repository: `cargo install --path oxiscript/oxiscript-cli --locked`.

## Settings

| Setting | Default | |
|---|---|---|
| `oxiscript.oxiPath` | `oxi` | path to the executable |
| `oxiscript.semicolons.fixOnSave` | `true` | insert missing `;` on save |
| `oxiscript.check.on` | `type` | `type` (unsaved buffer, debounced) or `save` |
| `oxiscript.check.debounceMs` | `300` | delay after the last keystroke |
| `oxiscript.check.timeoutMs` | `10000` | kill `oxi` after this long |

No LSP yet (hover, go to definition, rename arrive with slice 8.S2).

## Develop and test

```text
npm install
npm test            # compiles, then: TextMate grammar (vscode-textmate + vscode-oniguruma),
                    # JSON conversion, snippets (expanded and run through oxi), extension logic
                    # against a fake `vscode` module and the real `oxi` (needs ../../target/debug/oxi
                    # or OXI_BIN=/path/to/oxi; the oxi-based tests skip when it is missing)
npm run package     # oxiscript-0.0.1.vsix  (npx @vscode/vsce package; offline after npm install)
code --install-extension oxiscript-0.0.1.vsix
```

What is not tested: a real VS Code window (rendering of squiggles, the quick-fix lightbulb, format-on-save through the editor, the "oxi not found" notification UI). Manual check list: open a `.oxs` file, type `a == b` (squiggle `OXS-E0005`, lightbulb offers "use `===`"), save (file is formatted), set `oxiscript.oxiPath` to a wrong path (one warning, not one per save).
