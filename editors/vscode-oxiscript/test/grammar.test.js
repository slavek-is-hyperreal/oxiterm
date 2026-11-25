"use strict";
// Tokenises OxiScript with the real TextMate engine (vscode-textmate + vscode-oniguruma) and
// asserts the scopes that matter: numeric suffixes, the whitespace-sensitive `?`, `law`, errors.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const path = require("path");
const vsctm = require("vscode-textmate");
const oniguruma = require("vscode-oniguruma");

const ROOT = path.join(__dirname, "..");
const GRAMMAR = path.join(ROOT, "syntaxes", "oxiscript.tmLanguage.json");

let grammarPromise;
function loadGrammar() {
  if (!grammarPromise) {
    grammarPromise = (async () => {
      const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm")).buffer;
      await oniguruma.loadWASM(wasm);
      const registry = new vsctm.Registry({
        onigLib: Promise.resolve({
          createOnigScanner: (s) => new oniguruma.OnigScanner(s),
          createOnigString: (s) => new oniguruma.OnigString(s),
        }),
        loadGrammar: async (scope) =>
          scope === "source.oxiscript"
            ? vsctm.parseRawGrammar(fs.readFileSync(GRAMMAR, "utf8"), GRAMMAR)
            : null,
      });
      return registry.loadGrammar("source.oxiscript");
    })();
  }
  return grammarPromise;
}

/** [{text, scopes[]}] for every token of every line. */
async function tokenize(src) {
  const g = await loadGrammar();
  let stack = vsctm.INITIAL;
  const out = [];
  for (const line of src.split("\n")) {
    const r = g.tokenizeLine(line, stack);
    for (const t of r.tokens) {
      out.push({ text: line.slice(t.startIndex, t.endIndex), scopes: t.scopes });
    }
    stack = r.ruleStack;
  }
  return out;
}

/** Innermost scope of the first token whose text is `text` (after the optional `after` token). */
async function scopeOf(src, text, nth = 0) {
  const toks = (await tokenize(src)).filter((t) => t.text === text || t.text.trim() === text);
  assert.ok(toks[nth], `no token ${JSON.stringify(text)} in ${JSON.stringify(src)}`);
  return toks[nth].scopes[toks[nth].scopes.length - 1];
}

const S = (name) => `${name}.oxiscript`;

test("integer, float and suffix literals", async () => {
  assert.strictEqual(await scopeOf("const a: u8 = 200u8;", "200"), S("constant.numeric.integer"));
  assert.strictEqual(await scopeOf("const a: u8 = 200u8;", "u8", 1), S("constant.numeric.suffix"));
  assert.strictEqual(await scopeOf("const a = 1.5f32;", "1.5"), S("constant.numeric.float"));
  assert.strictEqual(await scopeOf("const a = 1.5f32;", "f32"), S("constant.numeric.suffix"));
  assert.strictEqual(await scopeOf("const a = .5;", ".5"), S("constant.numeric.float"));
  assert.strictEqual(await scopeOf("const a = 1e3f64;", "1e3"), S("constant.numeric.float"));
  assert.strictEqual(await scopeOf("const a = 1_000_000;", "1_000_000"), S("constant.numeric.integer"));
  assert.strictEqual(await scopeOf("const a = 0xffu8;", "0xff"), S("constant.numeric.hex"));
  assert.strictEqual(await scopeOf("const a = 0xffu8;", "u8"), S("constant.numeric.suffix"));
  assert.strictEqual(await scopeOf("const a = 0b1010u8;", "0b1010"), S("constant.numeric.binary"));
  assert.strictEqual(await scopeOf("const a = 0o17;", "0o17"), S("constant.numeric.octal"));
  // 0x1f32 is the hex number 0x1f32: no float suffix inside a hex literal
  const hex = (await tokenize("const a = 0x1f32;")).filter((t) => t.text.startsWith("0x"));
  assert.strictEqual(hex.length, 1);
  assert.strictEqual(hex[0].text, "0x1f32");
  // `1f32` is an error (needs `1.0f32`)
  assert.strictEqual(await scopeOf("const a = 1f32;", "1f32"), S("invalid.illegal.numeric"));
});

test("the whitespace-sensitive ?", async () => {
  assert.strictEqual(await scopeOf("const v = f()?;", "?"), S("keyword.operator.try"));
  assert.strictEqual(await scopeOf("const v = c ? 1 : 2;", "?"), S("keyword.operator.ternary"));
  assert.strictEqual(await scopeOf("const v = a?.b;", "?."), S("keyword.operator.optional-chain"));
  assert.strictEqual(await scopeOf("const v = f()?.b;", "?."), S("keyword.operator.optional-chain"));
  assert.strictEqual(await scopeOf("const v = a ?? b;", "??"), S("keyword.operator.nullish-coalescing"));
  assert.strictEqual(await scopeOf("const v = c ?.5 : 1;", "?"), S("keyword.operator.ternary"));
});

test("operators", async () => {
  assert.strictEqual(await scopeOf("a === b;", "==="), S("keyword.operator.comparison"));
  assert.strictEqual(await scopeOf("a == b;", "=="), S("invalid.illegal.loose-equality"));
  assert.strictEqual(await scopeOf("a != b;", "!="), S("invalid.illegal.loose-equality"));
  assert.strictEqual(await scopeOf("a +% b;", "+%"), S("keyword.operator.arithmetic.wrapping"));
  assert.strictEqual(await scopeOf("a *| b;", "*|"), S("keyword.operator.arithmetic.wrapping"));
  assert.strictEqual(await scopeOf("a += b;", "+="), S("keyword.operator.assignment"));
  assert.strictEqual(await scopeOf("const f = (x: i32) => x;", "=>"), S("storage.type.function.arrow"));
});

test("law is contextual", async () => {
  const toks = await tokenize("law add_commutes(a: i32, b: i32) when a > 0 && b > 0 {");
  const scopesOf = (text) => toks.find((t) => t.text === text).scopes;
  assert.ok(scopesOf("law").includes(S("keyword.declaration.law")));
  assert.ok(scopesOf("add_commutes").includes(S("entity.name.function.law")));
  assert.ok(scopesOf("when").includes(S("keyword.control.when")));
  // as an identifier it is not a keyword
  const id = await tokenize("const law = 1;");
  assert.ok(!id.find((t) => t.text.trim() === "law").scopes.includes(S("keyword.declaration.law")));
  // `when` outside a law header is an ordinary identifier
  const w = await tokenize("const when = 1;");
  assert.ok(!w.find((t) => t.text.trim() === "when").scopes.includes(S("keyword.control.when")));
});

test("attributes, keywords, constants", async () => {
  assert.strictEqual(await scopeOf("@expect(simd)\nfunction f(): void {}", "expect"), S("entity.name.function.attribute"));
  assert.strictEqual(await scopeOf("@@profile(safety)", "profile"), S("entity.name.function.attribute"));
  assert.strictEqual(await scopeOf("export function f(): void {}", "export"), S("keyword.control.import"));
  assert.strictEqual(await scopeOf("export function f(): void {}", "f"), S("entity.name.function"));
  assert.strictEqual(await scopeOf("if (a) { return; }", "if"), S("keyword.control"));
  assert.strictEqual(await scopeOf("const a = true;", "true"), S("constant.language"));
  assert.strictEqual(await scopeOf("const a = null;", "null"), S("constant.language"));
  assert.strictEqual(await scopeOf("this.x = 1;", "this"), S("variable.language.this"));
  assert.strictEqual(await scopeOf("const a = x as u8;", "as"), S("keyword.operator.cast"));
  assert.strictEqual(await scopeOf("switch (s.kind) {}", "switch"), S("keyword.control"));
  assert.strictEqual(await scopeOf("mutating inc(): void {}", "mutating"), S("storage.modifier"));
});

test("reserved and removed words are errors", async () => {
  for (const w of ["var", "any", "undefined", "typeof", "instanceof", "async", "await", "trait", "match", "unsafe"]) {
    assert.strictEqual(await scopeOf(`const x = ${w};`, w), S("invalid.illegal.reserved"), w);
  }
  // not whole-word matches
  assert.notStrictEqual(await scopeOf("const variable = 1;", "variable"), S("invalid.illegal.reserved"));
});

test("types", async () => {
  assert.strictEqual(await scopeOf("let a: u8 = 1;", "u8"), S("support.type.primitive"));
  assert.strictEqual(await scopeOf("let a: f64 = 1.0;", "f64"), S("support.type.primitive"));
  assert.strictEqual(await scopeOf("let a: string = \"\";", "string"), S("support.type.primitive"));
  assert.strictEqual(await scopeOf("let a: Result<i32, string> = Ok(1);", "Result"), S("support.type.builtin"));
  assert.strictEqual(await scopeOf("struct Point { x: f64; }", "Point"), S("entity.name.type"));
  assert.strictEqual(await scopeOf("let p: Point = q;", "Point"), S("entity.name.type"));
  assert.strictEqual(await scopeOf("const c = new Counter(0);", "Counter"), S("entity.name.type"));
  assert.strictEqual(await scopeOf("return Ok(1);", "Ok"), S("entity.name.function.call"));
});

test("strings, escapes, template expressions, comments", async () => {
  assert.ok((await tokenize('const s = "a\\nb";')).some((t) => t.scopes.includes(S("constant.character.escape"))));
  assert.ok((await tokenize("const s = '\\u{1F600}';")).some((t) => t.scopes.includes(S("constant.character.escape"))));
  const t = await tokenize("const s = `n = ${n + 1u8}!`;");
  const inner = t.find((x) => x.text === "1");
  assert.ok(inner.scopes.includes(S("meta.template.expression")), "number inside ${} is code");
  assert.ok(t.find((x) => x.text.includes("n = ")).scopes.includes(S("string.template")));
  assert.strictEqual(await scopeOf("// hi", "// hi"), S("comment.line.double-slash"));
  assert.strictEqual(await scopeOf("/// docs", "/// docs"), S("comment.line.documentation"));
  const block = await tokenize("/* a\nb */ const x = 1;");
  assert.ok(block[1].scopes.includes(S("comment.block")), "multi-line block comment");
  assert.ok(!block.find((x) => x.text === "const").scopes.some((s) => s.startsWith("comment")), "code after the comment");
});

test("every program of the syntax corpus tokenises without an error scope", async () => {
  const dir = path.join(ROOT, "..", "..", "oxiscript", "oxiscript-syntax", "tests", "corpus", "valid");
  const files = fs.readdirSync(dir).filter((f) => f.endsWith(".oxs"));
  assert.ok(files.length >= 40, "corpus not found at " + dir);
  const bad = [];
  for (const f of files) {
    const toks = await tokenize(fs.readFileSync(path.join(dir, f), "utf8"));
    for (const t of toks) {
      if (t.scopes.some((s) => s.startsWith("invalid"))) bad.push(`${f}: ${JSON.stringify(t.text)} ${t.scopes.join(" ")}`);
    }
  }
  assert.deepStrictEqual(bad, []);
});

test("sample file renders the expected scopes (golden)", async () => {
  const src = fs.readFileSync(path.join(__dirname, "sample.oxs"), "utf8");
  const toks = await tokenize(src);
  const golden = toks
    .filter((t) => t.text.trim() !== "")
    .map((t) => `${t.text}\t${t.scopes[t.scopes.length - 1].replace(".oxiscript", "")}`)
    .join("\n");
  const goldenPath = path.join(__dirname, "sample.golden.txt");
  if (process.env.UPDATE_GOLDEN) fs.writeFileSync(goldenPath, golden + "\n");
  assert.strictEqual(golden + "\n", fs.readFileSync(goldenPath, "utf8"));
});
