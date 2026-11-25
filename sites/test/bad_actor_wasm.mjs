// BAD ACTOR 8.W: fuzz the real release wasm artifact (panic=abort, fixed stack, no threads) in node.
// Usage: node sites/test/bad_actor_wasm.mjs   (needs sites/.build/pkg_play from sites/build.sh)
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const PKG = path.join(HERE, '../.build/pkg_play');
const mod = await import(path.join(PKG, 'oxiscript_playground.js'));
const bytes = fs.readFileSync(path.join(PKG, 'oxiscript_playground_bg.wasm'));
let initCount = 0;
function init() { mod.initSync({ module: bytes }); initCount++; }
init();
const results = []; let traps = 0;
function run(name, input, fns = ['check', 'format']) {
  for (const fn of fns) {
    const t0 = performance.now(); let status = 'ok', size = 0;
    try {
      const out = fn === 'apply_fix' ? mod.apply_fix(input, 0, 0, '', 0, 0, '') : mod[fn](input);
      size = out.length; JSON.parse(out);
    } catch (e) { status = 'TRAP ' + String(e).slice(0, 80); traps++; try { init(); } catch (e2) { status += ' REINIT-FAIL ' + e2; } }
    const ms = performance.now() - t0;
    results.push({ name, fn, bytes: Buffer.byteLength(input), ms: +ms.toFixed(1), size, status });
  }
}
const rep = (s, n) => s.repeat(n);
const consts = (body) => `export function main(): void {\n${body}\n}\n`;
for (const d of [10, 20, 40, 55, 56, 57, 100, 1000, 20000, 200000]) {
  run(`parens d=${d}`, `const x: i32 = ${rep('(', d)}1${rep(')', d)};`);
  run(`calls d=${d}`, `const x: i32 = ${rep('f(', d)}1${rep(')', d)};`);
  run(`array d=${d}`, `const x = ${rep('[', d)}1${rep(']', d)};`);
  run(`arrow d=${d}`, `const x = ${rep('() => ', d)}1;`);
  run(`generic d=${d}`, `const x: ${rep('Array<', d)}i32${rep('>', d)} = 1;`);
  run(`template d=${d}`, `const x = ${rep('`${', d)}1${rep('}`', d)};`);
  run(`blocks d=${d}`, consts(rep('{', d) + rep('}', d)));
  run(`if-nest d=${d}`, consts(rep('if (true) {', d) + rep('}', d)));
  run(`fn-nest d=${d}`, rep('function f() {', d) + rep('}', d));
  run(`objtype d=${d}`, `type T = ${rep('{a:', d)}i32${rep('}', d)};`);
  run(`switch-expr d=${d}`, `const x = ${rep('switch (a) { case 1: ', d)}1${rep('; }', d)};`);
  run(`unary d=${d}`, `const x = ${rep('!', d)}true;`);
  run(`neg d=${d}`, `const x = ${rep('- ', d)}1;`);
}
for (const n of [999, 1000, 1001, 5000, 40000]) {
  run(`plus chain n=${n}`, `const x = 1${rep(' + 1', n)};`);
  run(`member chain n=${n}`, `const x = a${rep('.b()', n)};`);
  run(`ternary chain n=${n}`, `const x = ${rep('a ? 1 : ', n)}2;`);
  run(`else-if n=${n}`, consts('if (a) {}' + rep(' else if (a) {}', n)));
}
// many diagnostics
for (const n of [1000, 10000, 20000]) {
  run(`many errors n=${n}`, rep('const x: i32 = "a";\n', n), ['check', 'format', 'apply_fix']);
  run(`many unknown idents n=${n}`, consts(rep('  foo;\n', n)));
  run(`many unterminated n=${n}`, rep('"\n', n));
  run(`many == n=${n}`, 'const a: i32 = 1;\n' + rep('const b: bool = a == 1;\n', n), ['check', 'apply_fix']);
}
// size edges
run('exactly 256KiB ascii', 'a'.repeat(256 * 1024));
run('256KiB+1', 'a'.repeat(256 * 1024 + 1));
run('200k CJK (maxlength ok, 600KB)', 'あ'.repeat(200000));
run('200k astral', '\u{1D518}'.repeat(200000));
run('256KiB idents', rep('abc ', 65536));
run('256KiB of semicolons', ';'.repeat(256 * 1024));
run('256KiB of comment open', '/*'.repeat(128 * 1024));
run('256KiB of quote', '"'.repeat(256 * 1024));
run('256KiB of backtick', '`'.repeat(256 * 1024));
run('256KiB nul', '\0'.repeat(256 * 1024));
run('long line string', `const s = "${'x'.repeat(250000)}";`);
// random fuzz
let seed = 12345; const rnd = () => (seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32;
const toks = ['export', 'function', 'main', '(', ')', '{', '}', ':', ';', 'const', 'let', 'x', 'i32', 'string', '=', '==', '===', '=>', '"s"', '`a${', '}`', '1', '1.5', '0x', 'if', 'else', 'switch', 'case', 'type', 'interface', 'import', '{ Log }', 'from', '"host:log"', 'log.write(2, "hi")', '<', '>', '[', ']', '?', '|', '&', '...', '.', ',', 'return', 'while', 'for', 'of', 'async', 'await', 'class', 'enum', '//', '/*', '*/', '\n', ' ', '\u{1F600}', 'é', '\u0000', '\\', "'", '@', '#', '$'];
let fz = 0; const t0 = performance.now(); let worst = 0;
for (let i = 0; i < 4000; i++) {
  const n = 1 + Math.floor(rnd() * 80); let s = '';
  for (let j = 0; j < n; j++) s += toks[Math.floor(rnd() * toks.length)] + (rnd() < 0.5 ? ' ' : '');
  const before = results.length;
  run(`rand#${i}`, s, ['check', 'format', 'apply_fix']);
  for (const r of results.slice(before)) { worst = Math.max(worst, r.ms); if (r.status !== 'ok') console.log('RANDOM TRAP', JSON.stringify(s)); }
  results.length = before; fz++;
}
console.log(`random fuzz: ${fz} inputs x3 fns in ${(performance.now() - t0).toFixed(0)} ms, worst call ${worst.toFixed(1)} ms, traps so far ${traps}`);
// apply_fix with out-of-range / non-integer indices. FLIPPED for F11: these used to be coerced to 0 and applied
// fix 0; now every one must be refused (ok:false) and a valid, matching call must still apply.
const FIXSRC = 'const a: i32 = 1;\nconst b: bool = a == 1;\n';
const fdiags = JSON.parse(mod.check(FIXSRC)).diagnostics;
const d0 = fdiags.find((d) => d.fixes.length);
const di = fdiags.indexOf(d0);
const fx = (d, f, src = FIXSRC) => JSON.parse(mod.apply_fix(src, d, f, d0.code, d0.start, d0.end, d0.fixes[0].title));
for (const [d, f] of [[-1, 0], [di, -1], [2 ** 31, 0], [2 ** 53, 0], [1e300, 0], [NaN, 0], [Infinity, 0], [di, NaN], [di, 0.5], [di, 1e9]]) {
  try { const o = fx(d, f); console.log('apply_fix', d, f, JSON.stringify(o).slice(0, 80)); if (o.ok) { console.log('FAIL: accepted a bad index'); traps++; } } catch (e) { console.log('apply_fix', d, f, 'THROW', String(e).slice(0, 80)); traps++; init(); }
}
{
  const good = fx(di, 0); const stale = fx(di, 0, 'const z: bool = 2 == 2;\n' + FIXSRC);
  console.log('apply_fix matching ok=' + good.ok + ', stale kind=' + stale.kind);
  if (!good.ok || stale.ok || stale.kind !== 'stale') { console.log('FAIL: stale-fix guard'); traps++; }
}
// explain with hostile codes
for (const c of ['', '<script>', 'OXS-E0005\0', 'x'.repeat(1e6)]) { try { mod.explain(c); } catch (e) { console.log('explain THROW', String(e).slice(0, 60)); traps++; init(); } }
// lone surrogates (wasm-bindgen replaces them with U+FFFD)
for (const s of ['\ud800', 'a\udc00b', '\ud83d']) { try { mod.check(s); mod.format(s); } catch (e) { console.log('surrogate THROW', e); traps++; init(); } }
const bad = results.filter((r) => r.status !== 'ok' || r.ms > 500);
console.log(JSON.stringify(bad, null, 0).replace(/},/g, '},\n'));
results.sort((a, b) => b.ms - a.ms);
console.log('slowest 12:'); for (const r of results.slice(0, 12)) console.log(' ', r.fn, r.name, r.bytes + 'B', r.ms + 'ms', 'out', r.size);
console.log(`total runs ${results.length}, traps ${traps}, reinit ${initCount - 1}`);
