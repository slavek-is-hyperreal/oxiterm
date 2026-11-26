// BAD ACTOR (run): native `run_json` vs the REAL runner bundle (wasm32, in Chromium). Corpus from
//   BAD_ACTOR_CORPUS=<file> cargo test -p oxiscript-playground --test bad_actor_run corpus
// Usage: BAD_ACTOR_CORPUS=<file> node sites/test/bad_actor_run_parity.mjs
import fs from 'node:fs';
import { serve, launch } from './bad_actor_run_lib.mjs';
const cases = JSON.parse(fs.readFileSync(process.env.BAD_ACTOR_CORPUS, 'utf8'));
const { srv, base } = await serve('oxiscript', 18954);
const b = await launch(); const p = await (await b.newContext()).newPage();
await p.goto(base + '/', { waitUntil: 'networkidle' });
const runnerUrl = await p.evaluate(() => document.getElementById('playground').dataset.runner);
await p.evaluate(async (u) => { const m = await import(u); await m.default(); window.__r = m; }, runnerUrl);
let same = 0, diff = 0, traps = {};
for (const c of cases) {
  const got = JSON.parse(await p.evaluate(([s]) => window.__r.run(s, 0), [c.src]));
  const a = JSON.stringify(c.json), g = JSON.stringify(got);
  if (a === g) same++; else { diff++; console.log('DIFF', c.name, '\n  native :', a.slice(0, 400), '\n  browser:', g.slice(0, 400)); }
  const k = c.json.ok ? (c.json.stop ? c.json.stop.reason : 'clean') : 'rejected'; traps[k] = (traps[k] || 0) + 1;
}
console.log(`parity: ${cases.length} programs, identical=${same}, different=${diff}`, JSON.stringify(traps));
await b.close(); srv.close();
