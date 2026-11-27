// BAD ACTOR round 2026-11-27-1 (target 1, R1-04): `oxi check --json` sanitises
// `diagnostics[].file` but not `files[]`, and the extension keeps only diagnostics whose
// `file` equals `files[0]`. A file whose name contains a character the sanitiser
// escapes (soft hyphen, ZWSP, bidi, ESC) therefore shows no diagnostics at all.
import test from "node:test";
import assert from "node:assert";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
const require = createRequire(import.meta.url);
const Module = require("module");
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OXI = process.env.OXI_BIN || path.join(__dirname, "..", "..", "..", "target", "debug", "oxi");

class Position { constructor(l, c) { this.line = l; this.character = c; } }
class Range {
  constructor(a, b, c, d) {
    if (typeof a === "number") { this.start = new Position(a, b); this.end = new Position(c, d); }
    else { this.start = a; this.end = b; }
  }
  intersection(o) {
    const lt = (p, q) => p.line < q.line || (p.line === q.line && p.character <= q.character);
    return lt(this.start, o.end) && lt(o.start, this.end) ? this : undefined;
  }
}
class Diagnostic { constructor(range, message, severity) { Object.assign(this, { range, message, severity }); } }
class CodeActionKind {
  constructor(v) { this.value = v; }
  append(x) { return new CodeActionKind(this.value + "." + x); }
}
CodeActionKind.QuickFix = new CodeActionKind("quickfix");
CodeActionKind.SourceFixAll = new CodeActionKind("source.fixAll");
class CodeAction { constructor(title, kind) { Object.assign(this, { title, kind }); } }
class WorkspaceEdit {
  constructor() { this.edits = []; }
  replace(uri, range, text) { this.edits.push({ range, text }); }
}
const TextEdit = { replace: (range, text) => ({ range, newText: text }) };

function fakeVscode(handlers, state) {
  const reg = (name) => (fn) => { (handlers[name] ||= []).push(fn); return { dispose() {} }; };
  return {
    Position, Range, Diagnostic, CodeAction, CodeActionKind, WorkspaceEdit, TextEdit,
    DiagnosticSeverity: { Error: 0, Warning: 1, Information: 2, Hint: 3 },
    window: {
      createOutputChannel: () => ({ appendLine() {}, show() {}, dispose() {} }),
      showWarningMessage: async (m) => { state.warnings.push(m); return undefined; },
      activeTextEditor: undefined,
    },
    languages: {
      createDiagnosticCollection: () => ({
        set: (uri, d) => { state.diags.set(uri.toString(), d); },
        delete: (uri) => state.diags.delete(uri.toString()),
        dispose() {},
      }),
      registerCodeActionsProvider: (_s, p) => { state.actions = p; return { dispose() {} }; },
      registerDocumentFormattingEditProvider: (_s, p) => { state.formatter = p; return { dispose() {} }; },
    },
    workspace: {
      textDocuments: [],
      getConfiguration: () => ({ get: (k, d) => (state.cfg && k in state.cfg ? state.cfg[k] : k === "oxiPath" ? state.oxi : k === "check.debounceMs" ? 0 : d) }),
      getWorkspaceFolder: () => undefined,
      onDidOpenTextDocument: reg("open"),
      onDidSaveTextDocument: reg("save"),
      onWillSaveTextDocument: reg("willsave"),
      onDidChangeTextDocument: reg("change"),
      onDidCloseTextDocument: reg("close"),
      onDidChangeConfiguration: reg("config"),
    },
    commands: { registerCommand: () => ({ dispose() {} }), executeCommand: async () => undefined },
  };
}

function load(oxi) {
  const handlers = {};
  const state = { diags: new Map(), warnings: [], oxi };
  const fake = fakeVscode(handlers, state);
  const orig = Module._load;
  Module._load = function (req, ...rest) { return req === "vscode" ? fake : orig.call(this, req, ...rest); };
  try {
    delete require.cache[require.resolve("../out/extension.js")];
    const ext = require("../out/extension.js");
    ext.activate({ subscriptions: [] });
  } finally { Module._load = orig; }
  return { handlers, state };
}

function doc(text, file, version = 1) {
  const lines = text.split("\n");
  return {
    languageId: "oxiscript", fileName: file, version,
    uri: { scheme: "file", fsPath: file, toString: () => "file://" + file },
    getText: () => text,
    lineCount: lines.length,
    lineAt: (i) => ({ text: lines[i] }),
  };
}

async function until(f, ms = 5000) {
  const t0 = Date.now();
  for (;;) {
    const v = f();
    if (v) return v;
    if (Date.now() - t0 > ms) throw new Error("timeout");
    await new Promise((r) => setTimeout(r, 25));
  }
}


test("R1-04: diagnostics of a file with a soft hyphen in its name reach the editor", async (t) => {
  if (!fs.existsSync(OXI)) return t.skip("no oxi binary");
  const dir = fs.mkdtempSync(path.join(process.env.TMPDIR || os.tmpdir(), "oxi-ba-"));
  const file = path.join(dir, "soft\u00adname.oxs");
  const src = "export function f(a: i32, b: i32): bool {\n  return a == b;\n}\n";
  fs.writeFileSync(file, src);
  const { handlers, state } = load(OXI);
  const d = doc(src, file);
  handlers.open[0](d);
  // control: the same source under a plain name produces E0005
  const plain = path.join(dir, "plain.oxs");
  fs.writeFileSync(plain, src);
  const dp = doc(src, plain);
  handlers.open[0](dp);
  const pd = await until(() => state.diags.get(dp.uri.toString()));
  assert.deepStrictEqual(pd.map((x) => x.code), ["OXS-E0005"]);
  const got = await until(() => state.diags.get(d.uri.toString()));
  assert.deepStrictEqual(got.map((x) => x.code), ["OXS-E0005"], "the editor shows no diagnostic for the soft-hyphen file");
});
