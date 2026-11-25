import * as cp from "child_process";
import * as path from "path";
import * as vscode from "vscode";
import { Diag, Fix, displayMessage, parseReport, selectFixAll, toPosition } from "./convert";

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

function runOxi(args: string[], input: string, cwd: string, key: string): Promise<RunResult> {
  return new Promise((resolve) => {
    const exe = config().get<string>("oxiPath", "oxi") || "oxi";
    const timeout = config().get<number>("check.timeoutMs", 10000);
    let child: cp.ChildProcess;
    try {
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
    if (r.stderr.trim()) output.appendLine(r.stderr.trim());
    return;
  }
  if (doc.version !== version) return; // a newer check is already scheduled
  let report;
  try {
    report = parseReport(r.stdout);
  } catch (e) {
    output.appendLine(`oxi check failed (exit ${r.code}): ${(e as Error).message}\n${r.stderr}`);
    return;
  }
  // `oxi` prints the entry path as given via --stdin-path with `\\` rewritten to `/`, so compare with
  // its own `files[0]` rather than re-deriving the path (drive-letter case, separators).
  const entry = report.files[0] ?? doc.fileName;
  const items: Item[] = [];
  if (report.fatal) {
    const d = new vscode.Diagnostic(new vscode.Range(0, 0, 0, 0), report.fatal, vscode.DiagnosticSeverity.Error);
    d.source = "oxi";
    items.push({ diag: d, src: { code: "fatal", severity: "error", message: report.fatal, file: doc.fileName, range: { start: { byte: 0, line: 1, col: 1, utf16: 1 }, end: { byte: 0, line: 1, col: 1, utf16: 1 } }, fixes: [] } });
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
        const title = f.applicability === "maybe-incorrect" ? `${f.title} (check the result)` : f.title;
        const a = new vscode.CodeAction(title, vscode.CodeActionKind.QuickFix);
        a.edit = we;
        a.diagnostics = [it.diag];
        a.isPreferred = f.applicability === "machine-applicable" && withEdits.length === 1;
        actions.push(a);
      }
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
  running.forEach((c) => c.kill());
}
