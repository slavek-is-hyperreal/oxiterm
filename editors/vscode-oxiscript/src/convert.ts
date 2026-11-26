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
  if (!Array.isArray(v.files) || v.files.some((f) => typeof f !== "string")) {
    throw new Error("oxi output has no files array");
  }
  const bad = (p: unknown): boolean => {
    const q = p as Partial<Pos> | null;
    return typeof q !== "object" || q === null || typeof q.line !== "number" || typeof q.utf16 !== "number" || typeof q.byte !== "number";
  };
  const badRange = (r: unknown): boolean => {
    const q = r as Partial<Range> | null;
    return typeof q !== "object" || q === null || bad(q.start) || bad(q.end);
  };
  for (const d of v.diagnostics) {
    if (typeof d !== "object" || d === null || typeof d.file !== "string" || typeof d.message !== "string" || badRange(d.range)) {
      throw new Error("oxi output has a malformed diagnostic");
    }
    if (!Array.isArray(d.fixes)) d.fixes = [];
    for (const f of d.fixes) {
      if (typeof f !== "object" || f === null || !Array.isArray(f.edits) || f.edits.some((e) => typeof e !== "object" || e === null || typeof e.file !== "string" || badRange(e.range) || typeof e.new_text !== "string")) {
        throw new Error("oxi output has a malformed fix");
      }
    }
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

/**
 * The edits of "add missing semicolons": the machine-applicable insertions of `;` for `OXS-E0004`
 * (one insertion each, nothing else) in `file`, in position order, one per position.
 */
export function selectSemicolons(diags: Diag[], file: string): Edit[] {
  const seen = new Set<number>();
  const out: Edit[] = [];
  for (const d of diags) {
    if (d.code !== "OXS-E0004") continue;
    for (const f of d.fixes) {
      if (f.applicability !== "machine-applicable" || f.edits.length !== 1) continue;
      const e = f.edits[0];
      if (e.file !== file || e.new_text !== ";" || e.range.start.byte !== e.range.end.byte) continue;
      if (seen.has(e.range.start.byte)) continue;
      seen.add(e.range.start.byte);
      out.push(e);
    }
  }
  return out.sort((a, b) => a.range.start.byte - b.range.start.byte);
}

/**
 * `fixed` must be `orig` with only `;` characters inserted (what `oxi check --fix-only semicolons`
 * returns). Returns the insertion points as zero-based line/character (UTF-16) positions, or undefined
 * when `fixed` is anything else (then the caller must not apply it), or when the text has lone `\r`
 * line breaks whose line numbering is editor-specific.
 */
export function semicolonInsertions(orig: string, fixed: string): Array<{ line: number; character: number }> | undefined {
  if (/\r(?!\n)/.test(orig)) return undefined;
  const out: Array<{ line: number; character: number }> = [];
  let i = 0;
  let line = 0;
  let col = 0;
  for (let j = 0; j < fixed.length; ) {
    if (i < orig.length && orig[i] === fixed[j]) {
      if (orig[i] === "\n") {
        line++;
        col = 0;
      } else {
        col++;
      }
      i++;
      j++;
    } else if (fixed[j] === ";") {
      out.push({ line, character: col });
      j++;
    } else {
      return undefined;
    }
  }
  return i === orig.length ? out : undefined;
}

/**
 * Escape terminal control characters (C0, DEL, C1 including 8-bit CSI/OSC),
 * bidi overrides/isolates (Trojan Source) and invisible format characters into
 * visible ASCII sequences. Matches `oxiscript_abi::sanitize::sanitize_terminal`.
 */
export function sanitizeTerminal(s: string): string {
  let out = "";
  for (const ch of s) {
    const cp = ch.codePointAt(0)!;
    const unsafe =
      cp <= 0x1f ||
      (cp >= 0x7f && cp <= 0x9f) ||
      cp === 0x00ad ||
      cp === 0x061c ||
      cp === 0x180e ||
      (cp >= 0x200b && cp <= 0x200f) ||
      (cp >= 0x2028 && cp <= 0x202e) ||
      (cp >= 0x2060 && cp <= 0x2064) ||
      (cp >= 0x2066 && cp <= 0x206f) ||
      cp === 0xfeff ||
      (cp >= 0xfff9 && cp <= 0xfffb) ||
      (cp >= 0xe0000 && cp <= 0xe007f);
    if (unsafe) {
      out += cp < 0x100 ? `\\x${cp.toString(16).padStart(2, "0")}` : `\\u{${cp.toString(16)}}`;
    } else {
      out += ch;
    }
  }
  return out;
}

/** Message shown in the editor: the diagnostic plus its advice-only fixes. */
export function displayMessage(d: Diag): string {
  const advice = d.fixes.filter((f) => f.edits.length === 0).map((f) => `help: ${sanitizeTerminal(f.title)}`);
  return [sanitizeTerminal(d.message), ...advice].join("\n");
}
