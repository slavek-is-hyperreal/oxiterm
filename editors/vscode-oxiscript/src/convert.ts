// Pure helpers (no `vscode` import) so they can be tested with plain node.
// The JSON shape is `oxiscript/oxiscript-cli/SCHEMA.md` (schema 1).

export interface Pos {
  byte: number;
  line: number; // 1-based
  col: number; // 1-based, characters
  utf16: number; // 1-based, UTF-16 code units (what VS Code counts)
}
export interface Range {
  start: Pos;
  end: Pos;
}
export interface Edit {
  file: string;
  range: Range;
  new_text: string;
}
export type Applicability = "machine-applicable" | "maybe-incorrect" | "advice";
export interface Fix {
  title: string;
  applicability: Applicability;
  edits: Edit[];
}
export interface Diag {
  code: string;
  severity: "error" | "warning" | "note" | "internal";
  message: string;
  file: string;
  range: Range;
  fixes: Fix[];
}
export interface Report {
  schema: number;
  ok: boolean;
  fatal: string | null;
  files: string[];
  diagnostics: Diag[];
}

export const SUPPORTED_SCHEMA = 1;

export function parseReport(stdout: string): Report {
  const v = JSON.parse(stdout) as Report;
  if (typeof v !== "object" || v === null || v.schema !== SUPPORTED_SCHEMA) {
    throw new Error(
      `unsupported oxi output (schema ${(v as { schema?: unknown } | null)?.schema}); update the extension or oxi`,
    );
  }
  if (!Array.isArray(v.diagnostics)) {
    throw new Error("oxi output has no diagnostics array");
  }
  return v;
}

/** Zero-based position as VS Code wants it. */
export function toPosition(p: Pos): { line: number; character: number } {
  return { line: Math.max(0, p.line - 1), character: Math.max(0, p.utf16 - 1) };
}

/**
 * The machine-applicable fixes that can be applied together: whole fixes are
 * taken in diagnostic order and skipped when any of their edits touches an
 * edit already taken (closed byte intervals, so adjacent edits conflict).
 * Only edits in `file` count.
 */
export function selectFixAll(diags: Diag[], file: string): Edit[][] {
  const taken: Array<[number, number]> = [];
  const out: Edit[][] = [];
  for (const d of diags) {
    for (const f of d.fixes) {
      if (f.applicability !== "machine-applicable" || f.edits.length === 0) continue;
      if (f.edits.some((e) => e.file !== file)) continue;
      const spans = f.edits.map((e) => [e.range.start.byte, e.range.end.byte] as [number, number]);
      const clash = (a: [number, number], b: [number, number]) => a[0] <= b[1] && b[0] <= a[1];
      const inner = spans.some((s, i) => spans.some((t, j) => j > i && clash(s, t)));
      if (inner || spans.some((s) => taken.some((t) => clash(s, t)))) continue;
      taken.push(...spans);
      out.push(f.edits);
    }
  }
  return out;
}

/** Message shown in the editor: the diagnostic plus its advice-only fixes. */
export function displayMessage(d: Diag): string {
  const advice = d.fixes.filter((f) => f.edits.length === 0).map((f) => `help: ${f.title}`);
  return [d.message, ...advice].join("\n");
}
