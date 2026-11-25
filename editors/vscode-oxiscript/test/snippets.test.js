"use strict";
// Every snippet, expanded with its default placeholders, must pass `oxi check`
// (set OXI_BIN, or build the CLI so that ../../target/debug/oxi exists).
const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const path = require("path");
const cp = require("child_process");

const snippets = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "snippets", "oxiscript.json"), "utf8"));
const OXI = process.env.OXI_BIN || path.join(__dirname, "..", "..", "..", "target", "debug", "oxi");

function expand(lines) {
  let text = lines.join("\n");
  const defaults = {};
  text = text.replace(/\$\{(\d+):([^}]*)\}/g, (_, n, d) => {
    if (!(n in defaults)) defaults[n] = d;
    return defaults[n];
  });
  text = text.replace(/\$\{(\d+)\}/g, (_, n) => defaults[n] ?? "");
  return text.replace(/\$0/g, "").replace(/\$\d+/g, "") + "\n";
}

test("snippets are well formed", () => {
  const prefixes = new Set();
  for (const [name, s] of Object.entries(snippets)) {
    assert.ok(s.prefix && s.body && s.description, name);
    assert.ok(!prefixes.has(s.prefix), `duplicate prefix ${s.prefix}`);
    prefixes.add(s.prefix);
  }
  for (const want of ["main", "law", "struct", "class", "union", "result"]) assert.ok(prefixes.has(want), want);
});

test("every snippet expands to a program that passes oxi check", (t) => {
  if (!fs.existsSync(OXI)) return t.skip(`no oxi binary at ${OXI}`);
  for (const [name, s] of Object.entries(snippets)) {
    const src = expand(s.body).replace(/^[ ]+\n/gm, "");
    const r = cp.spawnSync(OXI, ["check", "--json", "--stdin", "--stdin-path", "snippet.oxs"], { input: src, encoding: "utf8" });
    const j = JSON.parse(r.stdout);
    assert.strictEqual(j.ok, true, `${name}:\n${src}\n${JSON.stringify(j.diagnostics, null, 1)}`);
    const f = cp.spawnSync(OXI, ["fmt", "--stdin"], { input: src, encoding: "utf8" });
    assert.strictEqual(f.stdout, src, `${name} is not in oxi fmt form`);
  }
});
