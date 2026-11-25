// BAD ACTOR 8.W: format() must not lose comments / change string contents; must be idempotent.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const PKG = path.join(HERE, '../.build/pkg_play');
const mod = await import(path.join(PKG, 'oxiscript_playground.js'));
mod.initSync({ module: fs.readFileSync(path.join(PKG, 'oxiscript_playground_bg.wasm')) });
const exDir = path.join(HERE, '../oxiscript/examples');
const fmt = (s) => JSON.parse(mod.format(s));
const comments = (s) => [...s.matchAll(/\/\/[^\n]*|\/\*[\s\S]*?\*\*?\//g)].map((m) => m[0].trim());
const strings = (s) => [...s.matchAll(/"(?:[^"\\\n]|\\.)*"/g)].map((m) => m[0]);
let n = 0, lostComment = 0, changedStr = 0, nonIdem = 0, reject = 0, valChanged = 0;
const examples = [];
function mutate(src, seed) {
  // inject comments between tokens at a random whitespace position
  let s = seed; const rnd = () => (s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32;
  const parts = src.split(/(?<=[ ;{}(),\n])/); let out = '';
  for (const p of parts) { out += p; if (rnd() < 0.08) out += rnd() < 0.5 ? ' /* c' + Math.floor(rnd() * 99) + ' */ ' : ' // t' + Math.floor(rnd() * 99) + '\n'; }
  return out;
}
for (const f of fs.readdirSync(exDir).filter((x) => x.endsWith('.oxs'))) {
  const base = fs.readFileSync(path.join(exDir, f), 'utf8');
  for (let i = 0; i < 40; i++) {
    const src = i === 0 ? base : mutate(base, i * 7919 + f.length);
    const c0 = JSON.parse(mod.check(src));
    const r = fmt(src); n++;
    if (!r.ok) { reject++; if (c0.ok && reject < 5) examples.push(['rejected though check ok', f, i, src.slice(0, 0)]); continue; }
    const a = comments(src), b = comments(r.text);
    if (a.join('\n') !== b.join('\n')) { lostComment++; if (examples.length < 8) examples.push(['comments changed', f, i, a.filter((x) => !b.includes(x)).slice(0, 3)]); }
    if (strings(src).join('|') !== strings(r.text).join('|')) changedStr++;
    const r2 = fmt(r.text); if (!r2.ok || r2.text !== r.text) nonIdem++;
    const c1 = JSON.parse(mod.check(r.text)); if (c0.ok !== c1.ok || c0.diagnostics.length !== c1.diagnostics.length) { valChanged++; if (examples.length < 8) examples.push(['check result changed by format', f, i, c0.diagnostics.length, c1.diagnostics.length]); }
  }
}
// hand-picked: comment positions
const picks = ['const x = /* a */ 1 /* b */ + /* c */ 2; // d\n', 'function f(/* p */ a: i32 /* q */): void { /* e */ }\n', 'const s = "a  //  b";  // keep\n', 'const t = `a  ${ 1 }  b`;\n', '/* only */\n', '// only', 'const a = [1, // one\n 2, // two\n];\n', 'switch (x) { // c\n case 1: break; }\n'];
for (const p of picks) { const r = fmt(p); console.log(JSON.stringify(p), '->', r.ok ? JSON.stringify(r.text) : 'REJECT'); if (r.ok && comments(p).join() !== comments(r.text).join()) console.log('  ^^ COMMENTS CHANGED'); }
console.log({ n, reject, lostComment, changedStr, nonIdem, valChanged }); console.log(examples);
