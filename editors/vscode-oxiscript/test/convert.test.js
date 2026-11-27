"use strict";
const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const path = require("path");
const { parseReport, toPosition, selectFixAll, displayMessage, sanitizeTerminal, semicolonInsertions } = require("../out/convert.js");

const SNAP = path.join(__dirname, "..", "..", "..", "oxiscript", "oxiscript-cli", "tests", "snapshots");
const syntax = fs.readFileSync(path.join(SNAP, "check_bad_syntax.json"), "utf8");
const types = fs.readFileSync(path.join(SNAP, "check_bad_types.json"), "utf8");

test("parses the CLI's own JSON snapshots (schema 1)", () => {
  const r = parseReport(syntax);
  assert.strictEqual(r.schema, 1);
  assert.strictEqual(r.ok, false);
  assert.strictEqual(r.diagnostics[0].code, "OXS-E0005");
  const t = parseReport(types);
  assert.strictEqual(t.diagnostics[0].code, "OXS-E0100");
});

test("rejects other schema versions and garbage", () => {
  assert.throws(() => parseReport(JSON.stringify({ schema: 2, diagnostics: [] })), /unsupported/);
  assert.throws(() => parseReport("not json"));
  assert.throws(() => parseReport(JSON.stringify({ schema: 1 })), /diagnostics/);
});

test("positions use UTF-16 columns, zero-based", () => {
  const d = parseReport(syntax).diagnostics[0];
  const p = toPosition(d.range.start);
  assert.strictEqual(p.line, d.range.start.line - 1);
  assert.strictEqual(p.character, d.range.start.utf16 - 1);
  assert.deepStrictEqual(toPosition({ byte: 0, line: 1, col: 1, utf16: 1 }), { line: 0, character: 0 });
});

test("fix-all takes only machine-applicable, non-overlapping fixes of this file", () => {
  const r = parseReport(syntax);
  const file = r.diagnostics[0].file;
  const sets = selectFixAll(r.diagnostics, file);
  assert.strictEqual(sets.length, 1);
  assert.strictEqual(sets[0][0].new_text, "===");
  assert.strictEqual(selectFixAll(r.diagnostics, "other.oxs").length, 0);
  // maybe-incorrect fixes are never applied by fix-all
  assert.strictEqual(selectFixAll(parseReport(types).diagnostics, file).length, 0);
  // overlapping fixes: only the first is taken
  const pos = (b) => ({ byte: b, line: 1, col: b + 1, utf16: b + 1 });
  const mk = (a, b) => ({
    code: "OXS-E0004", severity: "error", message: "m", file: "f", range: { start: pos(a), end: pos(b) },
    fixes: [{ title: "t", applicability: "machine-applicable", edits: [{ file: "f", range: { start: pos(a), end: pos(b) }, new_text: "x" }] }],
  });
  assert.strictEqual(selectFixAll([mk(0, 5), mk(3, 8), mk(20, 22)], "f").length, 2);
});

test("advice fixes are shown in the message", () => {
  const d = { code: "OXS-E0300", severity: "error", message: "m", file: "f", range: { start: {}, end: {} },
    fixes: [{ title: "do this", applicability: "advice", edits: [] }] };
  assert.strictEqual(displayMessage(d), "m\nhelp: do this");
});

test("semicolonInsertions accepts only text that differs by inserted semicolons", () => {
  assert.deepStrictEqual(semicolonInsertions("a\nb c\n", "a;\nb c;\n"), [{ line: 0, character: 1 }, { line: 1, character: 3 }]);
  assert.deepStrictEqual(semicolonInsertions("a\r\nb", "a;\r\nb;"), [{ line: 0, character: 1 }, { line: 1, character: 1 }]);
  assert.deepStrictEqual(semicolonInsertions("x", "x"), []);
  assert.strictEqual(semicolonInsertions("a", "b"), undefined);
  assert.strictEqual(semicolonInsertions("ab", "a"), undefined);
  assert.strictEqual(semicolonInsertions("a\rb", "a;\rb"), undefined); // lone CR: line numbering is the editor's
});

test("sanitizeTerminal and displayMessage escape ESC, OSC, CSI, C1, CR and bidi controls", () => {
  assert.strictEqual(sanitizeTerminal("\x1b[2Jhi"), "\\x1b[2Jhi");
  assert.strictEqual(sanitizeTerminal("\x1b]0;pwn\x07ok"), "\\x1b]0;pwn\\x07ok");
  assert.strictEqual(sanitizeTerminal("\u009b31m\u009d0;t\u009c"), "\\x9b31m\\x9d0;t\\x9c");
  assert.strictEqual(sanitizeTerminal("a\rb\u202ec\u2066d\u2069"), "a\\x0db\\u{202e}c\\u{2066}d\\u{2069}");
  const d = {
    code: "OXS-E0014",
    severity: "error",
    message: "bad \x1b[31m\u202e",
    file: "f",
    range: { start: {}, end: {} },
    fixes: [{ title: "try \x1b]0;x\x07", applicability: "advice", edits: [] }],
  };
  assert.strictEqual(displayMessage(d), "bad \\x1b[31m\\u{202e}\nhelp: try \\x1b]0;x\\x07");
});
