import * as cp from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { Diag, Fix, displayMessage, parseReport, sanitizeTerminal, selectFixAll, selectSemicolons, semicolonInsertions, toPosition } from "./convert";

const LANG = "oxiscript";

interface Item {
  diag: vscode.Diagnostic;
  src: Diag;
}
interface Checked {
  version: number;
  /** The entry file as `oxi` printed it (`files[0]`); diagnostics and edits with this `file` belong to the document. */
  entry: string;
  items: Item[];
}

let output: vscode.OutputChannel;
let collection: vscode.DiagnosticCollection;
const checked = new Map<string, Checked>();
const timers = new Map<string, NodeJS.Timeout>();
const running = new Map<string, cp.ChildProcess>();
let warnedMissing = false;

interface RunResult {
  code: number | null;
  stdout: string;
  stderr: string;
  error?: NodeJS.ErrnoException;
}

function config(): vscode.WorkspaceConfiguration {
  return vscode.workspace.getConfiguration("oxiscript");
}

function cwdFor(doc: vscode.TextDocument): string {
  const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
  return folder ? folder.uri.fsPath : path.dirname(doc.fileName);
}

/** Most `oxi` processes alive at once (opening a workspace of N files must not spawn N). */
const MAX_CONCURRENT = 3;
let active = 0;
const waiting: Array<{ key: string; start: () => void; cancel: () => void }> = [];

function pump(): void {
  while (active < MAX_CONCURRENT && waiting.length > 0) {
    const job = waiting.shift();
    if (!job) break;
    active++;
    job.start();
  }
}

/**
 * Resolve the configured `oxi` to an absolute executable. An absolute setting is used as is; a bare
 * name is looked up in the absolute entries of PATH only (never the working directory, which on
 * Windows `spawn` would otherwise search first); a relative path with a separator is refused.
 */
export function resolveExe(exe: string): string {
  const enoent = (m: string): NodeJS.ErrnoException => Object.assign(new Error(m), { code: "ENOENT" }) as NodeJS.ErrnoException;
  if (path.isAbsolute(exe)) return exe;
  if (/[\\/]/.test(exe)) throw enoent(`oxiscript.oxiPath "${exe}" is a relative path; use an absolute path or a bare command name`);
  const names = process.platform === "win32" ? [`${exe}.exe`, exe] : [exe];
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    if (!dir || !path.isAbsolute(dir)) continue;
    for (const n of names) {
      const full = path.join(dir, n);
      try {
        if (fs.statSync(full).isFile()) {
          fs.accessSync(full, fs.constants.X_OK);
          return full;
        }
      } catch {
        // not here
      }
    }
  }
  throw enoent(`spawn ${exe} ENOENT (not found in PATH)`);
}

function runOxi(args: string[], input: string, cwd: string, key: string): Promise<RunResult> {
  return new Promise((resolve) => {
    // a newer request for the same document replaces a queued one and stops a running one
    const queued = waiting.findIndex((w) => w.key === key);
    if (queued >= 0) {
      waiting[queued].cancel();
      waiting.splice(queued, 1);
    }
    running.get(key)?.kill();
    waiting.push({
      key,
      cancel: () => resolve({ code: null, stdout: "", stderr: "" }),
      start: () => {
        spawnOxi(args, input, cwd, key).then(
          (r) => {
            active--;
            pump();
            resolve(r);
          },
          (e: unknown) => {
            active--;
            pump();
            resolve({ code: null, stdout: "", stderr: "", error: e as NodeJS.ErrnoException });
          },
        );
      },
    });
    pump();
  });
}

function spawnOxi(args: string[], input: string, cwd: string, key: string): Promise<RunResult> {
  return new Promise((resolve) => {
    const timeout = config().get<number>("check.timeoutMs", 10000);
    let child: cp.ChildProcess;
    try {
      const exe = resolveExe(config().get<string>("oxiPath", "oxi") || "oxi");
      child = cp.spawn(exe, args, { cwd, stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, NO_COLOR: "1" } });
    } catch (e) {
      resolve({ code: null, stdout: "", stderr: "", error: e as NodeJS.ErrnoException });
      return;
    }
    running.get(key)?.kill();
    running.set(key, child);
    let stdout = "";
    let stderr = "";
    let settled = false;
    const done = (r: RunResult) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (running.get(key) === child) running.delete(key);
      resolve(r);
    };
    const timer = setTimeout(() => {
      child.kill();
      done({ code: null, stdout, stderr: stderr + `\noxi timed out after ${timeout} ms`, error: undefined });
    }, timeout);
    child.stdout?.on("data", (d: Buffer) => (stdout += d.toString("utf8")));
    child.stderr?.on("data", (d: Buffer) => (stderr += d.toString("utf8")));
    child.on("error", (e: NodeJS.ErrnoException) => done({ code: null, stdout, stderr, error: e }));
    child.on("close", (code) => done({ code, stdout, stderr }));
    child.stdin?.on("error", () => undefined);
    child.stdin?.end(input);
  });
}

function warnMissing(e: NodeJS.ErrnoException): void {
  output.appendLine(`cannot run oxi: ${e.message}`);
  if (warnedMissing) return;
  warnedMissing = true;
  void vscode.window
    .showWarningMessage(
      "OxiScript: the `oxi` command was not found. Install it (cargo install --path oxiscript/oxiscript-cli) or set oxiscript.oxiPath.",
      "Open settings",
    )
    .then((choice) => {
      if (choice) void vscode.commands.executeCommand("workbench.action.openSettings", "oxiscript.oxiPath");
    });
}

async function check(doc: vscode.TextDocument): Promise<void> {
  try {
    await checkInner(doc);
  } catch (e) {
    // never an unhandled rejection; drop results that might be stale
    output.appendLine(`oxi check failed for ${doc.fileName}: ${(e as Error).message}`);
    collection.delete(doc.uri);
    checked.delete(doc.uri.toString());
  }
}

async function checkInner(doc: vscode.TextDocument): Promise<void> {
  if (doc.languageId !== LANG || doc.uri.scheme !== "file") return;
  const key = doc.uri.toString();
  const version = doc.version;
  const cwd = cwdFor(doc);
  const r = await runOxi(["check", "--json", "--stdin", "--stdin-path", doc.fileName], doc.getText(), cwd, key);
  if (r.error) {
    warnMissing(r.error);
    return;
  }
  if (r.code === null) {
    // killed by a newer check, or timed out
    if (r.stderr.trim()) output.appendLine(sanitizeTerminal(r.stderr.trim()));
    return;
  }
  if (doc.version !== version) return; // a newer check is already scheduled
  let report;
  try {
    report = parseReport(r.stdout);
  } catch (e) {
    output.appendLine(sanitizeTerminal(`oxi check failed (exit ${r.code}): ${(e as Error).message} ${r.stderr}`));
    collection.delete(doc.uri);
    checked.delete(key);
    return;
  }
  // `oxi` prints the entry path as given via --stdin-path with `\\` rewritten to `/`, so compare with
  // its own `files[0]` rather than re-deriving the path (drive-letter case, separators).
  const entry = report.files[0] ?? doc.fileName;
  const items: Item[] = [];
  if (report.fatal) {
    const cleanFatal = sanitizeTerminal(report.fatal);
    const d = new vscode.Diagnostic(new vscode.Range(0, 0, 0, 0), cleanFatal, vscode.DiagnosticSeverity.Error);
    d.source = "oxi";
    items.push({ diag: d, src: { code: "fatal", severity: "error", message: cleanFatal, file: doc.fileName, range: { start: { byte: 0, line: 1, col: 1, utf16: 1 }, end: { byte: 0, line: 1, col: 1, utf16: 1 } }, fixes: [] } });
  }
  for (const s of report.diagnostics) {
    if (s.file !== entry) continue;
    const a = toPosition(s.range.start);
    const b = toPosition(s.range.end);
    const sev =
      s.severity === "warning"
        ? vscode.DiagnosticSeverity.Warning
        : s.severity === "note"
          ? vscode.DiagnosticSeverity.Information
          : vscode.DiagnosticSeverity.Error;
    const d = new vscode.Diagnostic(new vscode.Range(a.line, a.character, b.line, b.character), displayMessage(s), sev);
    d.code = s.code;
    d.source = "oxi";
    items.push({ diag: d, src: s });
  }
  checked.set(key, { version, entry, items });
  collection.set(doc.uri, items.map((i) => i.diag));
}

function schedule(doc: vscode.TextDocument, delay: number): void {
  if (doc.languageId !== LANG) return;
  const key = doc.uri.toString();
  const t = timers.get(key);
  if (t) clearTimeout(t);
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      void check(doc);
    }, delay),
  );
}

function editsOf(doc: vscode.TextDocument, entry: string, fix: Fix): vscode.WorkspaceEdit | undefined {
  if (fix.edits.length === 0) return undefined;
  const we = new vscode.WorkspaceEdit();
  for (const e of fix.edits) {
    if (e.file !== entry) return undefined;
    const a = toPosition(e.range.start);
    const b = toPosition(e.range.end);
    we.replace(doc.uri, new vscode.Range(a.line, a.character, b.line, b.character), e.new_text);
  }
  return we;
}

class Fixes implements vscode.CodeActionProvider {
  static readonly kinds = [vscode.CodeActionKind.QuickFix, vscode.CodeActionKind.SourceFixAll.append("oxiscript")];

  provideCodeActions(doc: vscode.TextDocument, range: vscode.Range): vscode.CodeAction[] {
    const st = checked.get(doc.uri.toString());
    // Offsets of a stale check would corrupt the buffer: offer nothing until the next check lands.
    if (!st || st.version !== doc.version) return [];
    const actions: vscode.CodeAction[] = [];
    for (const it of st.items) {
      if (!it.diag.range.intersection(range)) continue;
      const withEdits = it.src.fixes.filter((f) => f.edits.length > 0);
      for (const f of withEdits) {
        const we = editsOf(doc, st.entry, f);
        if (!we) continue;
        const cleanTitle = sanitizeTerminal(f.title);
        const title = f.applicability === "maybe-incorrect" ? `${cleanTitle} (check the result)` : cleanTitle;
        const a = new vscode.CodeAction(title, vscode.CodeActionKind.QuickFix);
        a.edit = we;
        a.diagnostics = [it.diag];
        a.isPreferred = f.applicability === "machine-applicable" && withEdits.length === 1;
        actions.push(a);
      }
    }
    const semis = selectSemicolons(st.items.map((i) => i.src), st.entry);
    if (semis.length > 0) {
      const we = new vscode.WorkspaceEdit();
      for (const e of semis) {
        const a = toPosition(e.range.start);
        we.replace(doc.uri, new vscode.Range(a.line, a.character, a.line, a.character), ";");
      }
      const sa = new vscode.CodeAction(`Add missing semicolons in this file (${semis.length})`, vscode.CodeActionKind.QuickFix);
      sa.edit = we;
      sa.diagnostics = st.items.filter((i) => i.src.code === "OXS-E0004").map((i) => i.diag);
      actions.push(sa);
    }
    const all = selectFixAll(st.items.map((i) => i.src), st.entry);
    // `selectFixAll` compares `file` strings as printed by oxi; the entry file is printed as given via --stdin-path.
    if (all.length > 0) {
      const we = new vscode.WorkspaceEdit();
      for (const edits of all) {
        for (const e of edits) {
          const a = toPosition(e.range.start);
          const b = toPosition(e.range.end);
          we.replace(doc.uri, new vscode.Range(a.line, a.character, b.line, b.character), e.new_text);
        }
      }
      const fa = new vscode.CodeAction(`Fix all OxiScript problems that have a safe fix (${all.length})`, vscode.CodeActionKind.SourceFixAll.append("oxiscript"));
      fa.edit = we;
      actions.push(fa);
    }
    return actions;
  }
}

class Formatter implements vscode.DocumentFormattingEditProvider {
  async provideDocumentFormattingEdits(doc: vscode.TextDocument): Promise<vscode.TextEdit[]> {
    const r = await runOxi(["fmt", "--stdin"], doc.getText(), cwdFor(doc), `fmt:${doc.uri.toString()}`);
    if (r.error) {
      warnMissing(r.error);
      return [];
    }
    if (r.code !== 0) {
      // Syntax errors: the diagnostics already show them; leave the buffer alone.
      output.appendLine(`oxi fmt did not format ${doc.fileName}: ${r.stderr.trim()}`);
      return [];
    }
    if (r.stdout === doc.getText()) return [];
    const last = doc.lineAt(doc.lineCount - 1);
    return [vscode.TextEdit.replace(new vscode.Range(0, 0, doc.lineCount - 1, last.text.length), r.stdout)];
  }
}

/**
 * On save: insert the missing `;` (and nothing else) through `oxi check --fix-only semicolons`.
 * The CLI refuses lines where a `;` would change the meaning and never touches other problems.
 */
async function semicolonEdits(doc: vscode.TextDocument): Promise<vscode.TextEdit[]> {
  if (doc.languageId !== LANG || doc.uri.scheme !== "file") return [];
  if (!config().get<boolean>("semicolons.fixOnSave", true)) return [];
  try {
    const text = doc.getText();
    const r = await runOxi(["check", "--stdin", "--stdin-path", doc.fileName, "--fix-only", "semicolons"], text, cwdFor(doc), `semi:${doc.uri.toString()}`);
    if (r.error) {
      warnMissing(r.error);
      return [];
    }
    if (r.code !== 0 || r.stdout === text || r.stdout.length < text.length) return [];
    const ins = semicolonInsertions(text, r.stdout);
    if (!ins) return [];
    return ins.map((p) => vscode.TextEdit.replace(new vscode.Range(p.line, p.character, p.line, p.character), ";"));
  } catch (e) {
    output.appendLine(`semicolon fix failed for ${doc.fileName}: ${(e as Error).message}`);
    return [];
  }
}

export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel("OxiScript");
  collection = vscode.languages.createDiagnosticCollection("oxiscript");
  const sel: vscode.DocumentSelector = { language: LANG, scheme: "file" };
  context.subscriptions.push(
    output,
    collection,
    vscode.languages.registerCodeActionsProvider(sel, new Fixes(), { providedCodeActionKinds: Fixes.kinds }),
    vscode.languages.registerDocumentFormattingEditProvider(sel, new Formatter()),
    vscode.workspace.onDidOpenTextDocument((d) => schedule(d, 0)),
    vscode.workspace.onDidSaveTextDocument((d) => schedule(d, 0)),
    ...(typeof vscode.workspace.onWillSaveTextDocument === "function"
      ? [vscode.workspace.onWillSaveTextDocument((e) => e.waitUntil(semicolonEdits(e.document)))]
      : []),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (config().get<string>("check.on", "type") === "type") {
        schedule(e.document, config().get<number>("check.debounceMs", 300));
      }
    }),
    vscode.workspace.onDidCloseTextDocument((d) => {
      const key = d.uri.toString();
      collection.delete(d.uri);
      checked.delete(key);
      const t = timers.get(key);
      if (t) clearTimeout(t);
      timers.delete(key);
      running.get(key)?.kill();
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("oxiscript")) {
        warnedMissing = false;
        vscode.workspace.textDocuments.forEach((d) => schedule(d, 0));
      }
    }),
    vscode.commands.registerCommand("oxiscript.checkNow", () => {
      const d = vscode.window.activeTextEditor?.document;
      if (d) void check(d);
    }),
    vscode.commands.registerCommand("oxiscript.showOutput", () => output.show()),
  );
  vscode.workspace.textDocuments.forEach((d) => schedule(d, 0));
}

export function deactivate(): void {
  waiting.splice(0).forEach((w) => w.cancel());
  running.forEach((c) => c.kill());
}
