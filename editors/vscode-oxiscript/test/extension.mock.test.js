"use strict";
// Runs the real extension code (out/extension.js) against a minimal fake `vscode` module and the real
// `oxi` binary: diagnostics from an unsaved buffer, quick fixes, fix-all, stale-version guard, formatting.
// A real VS Code window is not needed (and not covered: see README "What is not tested").
const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const path = require("path");
const Module = require("module");

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
      getConfiguration: () => ({ get: (k, d) => (k === "oxiPath" ? state.oxi : k === "check.debounceMs" ? 0 : d) }),
      getWorkspaceFolder: () => undefined,
      onDidOpenTextDocument: reg("open"),
      onDidSaveTextDocument: reg("save"),
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

const file = path.join(require("os").tmpdir(), `oxi-ext-test-${process.pid}.oxs`);

test("diagnostics, quick fixes and fix-all from an unsaved buffer", async (t) => {
  if (!fs.existsSync(OXI)) return t.skip("no oxi binary");
  const { handlers, state } = load(OXI);
  // an emoji before the error: the editor column must be in UTF-16 units
  const src = "export function f(a: i32): bool {\n  return \"\u{1F600}\" === \"x\" || a == 1;\n}\nconst x: i32 = 1\n";
  const d = doc(src, file);
  handlers.open[0](d);
  const diags = await until(() => state.diags.get(d.uri.toString()));
  const codes = diags.map((x) => x.code).sort();
  assert.deepStrictEqual(codes, ["OXS-E0004", "OXS-E0005"]);
  const eq = diags.find((x) => x.code === "OXS-E0005");
  assert.strictEqual(eq.range.start.line, 1);
  const lineText = src.split("\n")[1];
  assert.strictEqual(eq.range.start.character, [...lineText.slice(0, lineText.indexOf("a == 1") + 2)].reduce((n, c) => n + c.length, 0));
  // quick fix for `==`
  const acts = state.actions.provideCodeActions(d, eq.range);
  const fix = acts.find((a) => a.title === "use `===`");
  assert.ok(fix, acts.map((a) => a.title).join("|"));
  assert.strictEqual(fix.isPreferred, true);
  assert.strictEqual(fix.edit.edits[0].text, "===");
  // fix-all gathers both safe fixes
  const all = acts.find((a) => a.kind.value === "source.fixAll.oxiscript");
  assert.ok(all && all.edit.edits.length === 2, "fix-all edits");
  // stale document version: no actions
  assert.deepStrictEqual(state.actions.provideCodeActions(doc(src, file, 2), eq.range), []);
});

test("formatting returns a whole-document edit, and nothing for syntax errors", async (t) => {
  if (!fs.existsSync(OXI)) return t.skip("no oxi binary");
  const { state } = load(OXI);
  const d = doc("let   x=1 ;", file);
  const edits = await state.formatter.provideDocumentFormattingEdits(d);
  assert.strictEqual(edits.length, 1);
  assert.strictEqual(edits[0].newText, "let x = 1;\n");
  assert.deepStrictEqual(await state.formatter.provideDocumentFormattingEdits(doc("let x = ;", file)), []);
  assert.deepStrictEqual(await state.formatter.provideDocumentFormattingEdits(doc("let x = 1;\n", file)), []);
});

test("a missing oxi is reported once, not per save", async () => {
  const { handlers, state } = load("/nonexistent/oxi-binary");
  const d = doc("let x = 1;", file);
  handlers.save[0](d);
  await until(() => state.warnings.length === 1);
  handlers.save[0](d);
  await new Promise((r) => setTimeout(r, 300));
  assert.strictEqual(state.warnings.length, 1);
  assert.match(state.warnings[0], /oxi/);
});
