"use strict";
// BAD ACTOR 8.D attack tests for the VS Code extension. Each test asserts the SAFE behaviour; a failing
// test is a finding (see docs/stages/8.D/bad-actor.md). Tests named `held_up_*` pass.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const Module = require("module");

const PKG = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"));
const OXI = process.env.OXI_BIN || path.join(__dirname, "..", "..", "..", "target", "debug", "oxi");

class Position { constructor(l, c) { this.line = l; this.character = c; } }
class Range {
  constructor(a, b, c, d) {
    if (typeof a === "number") { this.start = new Position(a, b); this.end = new Position(c, d); }
    else { this.start = a; this.end = b; }
  }
  intersection(o) { return this; }
}
class Diagnostic { constructor(range, message, severity) { Object.assign(this, { range, message, severity }); } }
class CodeActionKind { constructor(v) { this.value = v; } append(x) { return new CodeActionKind(this.value + "." + x); } }
CodeActionKind.QuickFix = new CodeActionKind("quickfix");
CodeActionKind.SourceFixAll = new CodeActionKind("source.fixAll");
class CodeAction { constructor(t, k) { Object.assign(this, { title: t, kind: k }); } }
class WorkspaceEdit { constructor() { this.edits = []; } replace(u, r, t) { this.edits.push({ range: r, text: t }); } }

function load(oxi) {
  const handlers = {};
  const state = { diags: new Map(), warnings: [], oxi };
  const reg = (n) => (fn) => { (handlers[n] ||= []).push(fn); return { dispose() {} }; };
  const fake = {
    Position, Range, Diagnostic, CodeAction, CodeActionKind, WorkspaceEdit,
    TextEdit: { replace: (range, text) => ({ range, newText: text }) },
    DiagnosticSeverity: { Error: 0, Warning: 1, Information: 2, Hint: 3 },
    window: {
      createOutputChannel: () => ({ appendLine() {}, show() {}, dispose() {} }),
      showWarningMessage: async (m) => { state.warnings.push(m); },
    },
    languages: {
      createDiagnosticCollection: () => ({
        set: (u, d) => state.diags.set(u.toString(), d), delete: (u) => state.diags.delete(u.toString()), dispose() {},
      }),
      registerCodeActionsProvider: (_s, p) => { state.actions = p; return { dispose() {} }; },
      registerDocumentFormattingEditProvider: (_s, p) => { state.formatter = p; return { dispose() {} }; },
    },
    workspace: {
      textDocuments: [],
      getConfiguration: () => ({ get: (k, d) => (k === "oxiPath" ? state.oxi : k === "check.debounceMs" ? 0 : d) }),
      getWorkspaceFolder: () => undefined,
      onDidOpenTextDocument: reg("open"), onDidSaveTextDocument: reg("save"), onDidChangeTextDocument: reg("change"),
      onDidCloseTextDocument: reg("close"), onDidChangeConfiguration: reg("config"),
    },
    commands: { registerCommand: () => ({ dispose() {} }), executeCommand: async () => undefined },
  };
  const orig = Module._load;
  Module._load = function (req, ...rest) { return req === "vscode" ? fake : orig.call(this, req, ...rest); };
  try {
    delete require.cache[require.resolve("../out/extension.js")];
    require("../out/extension.js").activate({ subscriptions: [] });
  } finally { Module._load = orig; }
  return { handlers, state };
}

function doc(text, file, version = 1) {
  const lines = text.split("\n");
  return {
    languageId: "oxiscript", fileName: file, version,
    uri: { scheme: "file", fsPath: file, toString: () => "file://" + file },
    getText: () => text, lineCount: lines.length, lineAt: (i) => ({ text: lines[i] }),
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "oxi-badactor-"));
function fakeOxi(name, body) {
  const p = path.join(tmp, name);
  fs.writeFileSync(p, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
  return p;
}

test("F-ext-1 package.json must not let a workspace override oxiPath and must declare workspace trust", () => {
  const prop = PKG.contributes.configuration.properties["oxiscript.oxiPath"];
  const cap = PKG.capabilities && PKG.capabilities.untrustedWorkspaces;
  // `machine-overridable` means a cloned repo's .vscode/settings.json can point oxiPath at ./evil.sh;
  // it must be `machine` (or restricted via capabilities.untrustedWorkspaces.restrictedConfigurations).
  const restricted = cap && cap.supported !== true && (cap.supported === false || (cap.restrictedConfigurations || []).includes("oxiscript.oxiPath"));
  assert.ok(prop.scope === "machine" || restricted, `oxiPath scope=${prop.scope}, untrustedWorkspaces=${JSON.stringify(cap)}`);
  assert.ok(cap, "no explicit capabilities.untrustedWorkspaces declaration");
});

test("F-ext-2 a malformed report from oxi must not become an unhandled rejection or leave stale diagnostics", async () => {
  const oxi = fakeOxi("noFiles", `cat >/dev/null; echo '{"schema":1,"diagnostics":[{"code":"X","severity":"error","message":"m","file":"a","range":{"start":{"line":1,"utf16":1,"byte":0},"end":{"line":1,"utf16":1,"byte":0}}}]}'`);
  const { handlers, state } = load(oxi);
  const rejections = [];
  const h = (e) => rejections.push(e);
  process.on("unhandledRejection", h);
  try {
    handlers.open[0](doc("let x = 1;", path.join(tmp, "a.oxs")));
    await sleep(800);
  } finally { process.off("unhandledRejection", h); }
  assert.strictEqual(rejections.length, 0, `unhandled rejection: ${rejections[0] && rejections[0].message}`);
});

test("F-ext-3 opening many documents must not spawn them all at once", async () => {
  const log = path.join(tmp, "conc.log");
  const oxi = fakeOxi("slow", `echo start >> ${log}; cat >/dev/null; sleep 1; echo '{"schema":1,"ok":true,"fatal":null,"files":["a"],"diagnostics":[]}'; echo end >> ${log}`);
  const { handlers } = load(oxi);
  for (let i = 0; i < 60; i++) handlers.open[0](doc("let x = 1;", path.join(tmp, `s${i}.oxs`)));
  await sleep(600);
  const lines = fs.readFileSync(log, "utf8").trim().split("\n");
  let cur = 0, max = 0;
  for (const l of lines) { cur += l === "start" ? 1 : -1; max = Math.max(max, cur); }
  assert.ok(max <= 8, `${max} oxi processes running concurrently`);
});

test("held_up_paths with spaces, quotes and shell metacharacters are passed as plain arguments", async () => {
  const dir = path.join(tmp, `a b'c"d; touch PWNED $(id)`);
  fs.mkdirSync(dir);
  const exe = path.join(dir, "oxi wrapper");
  fs.writeFileSync(exe, `#!/bin/sh\ncat >/dev/null; echo '{"schema":1,"ok":true,"fatal":null,"files":["x"],"diagnostics":[]}'\n`, { mode: 0o755 });
  const { handlers, state } = load(exe);
  const d = doc("let x = 1;", path.join(dir, "f'; touch PWNED2; '.oxs"));
  handlers.open[0](d);
  const t0 = Date.now();
  while (!state.diags.has(d.uri.toString()) && Date.now() - t0 < 4000) await sleep(25);
  assert.ok(state.diags.has(d.uri.toString()), "oxi wrapper with spaces/quotes ran");
  assert.ok(!fs.existsSync(path.join(process.cwd(), "PWNED")) && !fs.existsSync(path.join(dir, "PWNED")) && !fs.existsSync(path.join(dir, "PWNED2")), "no shell injection");
});

test("held_up_diagnostics of an older buffer version are not applied", async () => {
  if (!fs.existsSync(OXI)) return;
  const oxi = fakeOxi("delayed", `sleep 0.4; exec ${OXI} "$@"`);
  const { handlers, state } = load(oxi);
  const d = doc("const x: i32 = 1 == 2;\n", path.join(tmp, "v.oxs"));
  handlers.open[0](d);
  await sleep(150);
  d.version = 2; // user typed meanwhile
  await sleep(1200);
  assert.ok(!state.diags.has(d.uri.toString()), "stale result must be dropped");
});
