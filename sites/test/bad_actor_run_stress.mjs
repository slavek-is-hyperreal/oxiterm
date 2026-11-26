// BAD ACTOR (run): runaway / resource programs in the real page. For each: wall time until Run is enabled again, main-thread longest task, outcome.
import { serve, launch } from './bad_actor_run_lib.mjs';
const { srv, base } = await serve('oxiscript', 18952);
const b = await launch();
const ctx = await b.newContext({ viewport: { width: 1000, height: 900 } });
const p = await ctx.newPage(); const errs = []; p.on('pageerror', (e) => errs.push(e.message)); p.on('console', (m) => { if (m.type() === 'error') errs.push(m.text().slice(0, 200)); });
await p.addInitScript(() => { window.__lt = 0; try { new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__lt = Math.max(window.__lt, e.duration); }).observe({ entryTypes: ['longtask'] }); } catch (e) {} });
await p.goto(base + '/', { waitUntil: 'networkidle' });
await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
const H = 'import { Log } from "host:log";\nimport { State } from "host:state";\n';
const progs = {
  strdouble: H + 'export function main(log: Log): void { let s = "xxxxxxxx"; for (const i of range(0, 40)) { s = s + s; } log.write(2, `${s.byteLength}`); }',
  arrpush: H + 'export function main(log: Log): void { const a: i32[] = []; for (const i of range(0, 100000000)) { a.push(i); } log.write(2, `${a.length}`); }',
  recursion: H + 'function f(n: i32): i32 { return f(n + 1) + 1; }\nexport function main(log: Log): void { log.write(2, `${f(0)}`); }',
  concatloop: H + 'export function main(log: Log): void { let s = "x"; for (const i of range(0, 100000000)) { s = s + "x"; } log.write(2, `${s.byteLength}`); }',
  whiletrue: H + 'export function main(log: Log): void { let i: i32 = 0; while (true) { i = i +% 1; } }',
  hostloop: H + 'export function main(log: Log): void { for (const i of range(0, 100000000)) { log.write(0, "x"); } }',
  stateloop: H + 'export function main(log: Log, st: State): void { for (const i of range(0, 100000000)) { const r = st.open(`k${i}`); } }',
  bigarr: H + 'export function main(log: Log): void { const a: i64[] = []; for (const i of range(0, 3000000)) { a.push(i as i64); } log.write(2, `${a.length}`); }',
  mapfilter: H + 'export function main(log: Log): void { let a: i32[] = [1,2,3]; for (const i of range(0, 30)) { a = a.concat(a); } log.write(2, `${a.length}`); }',
};
const run = async (name, s, wait = 20000) => {
  await p.evaluate((s) => { const t = document.getElementById('pg-src'); t.value = s; t.dispatchEvent(new Event('input')); window.__lt = 0; document.getElementById('pg-out').textContent='<<cleared>>'; document.getElementById('pg-out-note').textContent=''; }, s);
  const t0 = Date.now(); await p.click('#pg-run');
  const done = await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: wait }).then(() => true, () => false);
  await p.waitForTimeout(250); const ms = Date.now() - t0 - 250; const diag = await p.evaluate(() => [...document.querySelectorAll('#pg-diags li')].slice(0,2).map((l) => l.textContent.slice(0,110)).join(' | ')); const lt = await p.evaluate(() => Math.round(window.__lt));
  const out = (await p.textContent('#pg-out')).slice(0, 80); const note = (await p.textContent('#pg-out-note')).slice(0, 160); const st = (await p.textContent('#pg-status')).slice(0, 120);
  console.log(name.padEnd(12), `done=${done} ms=${ms} longtaskMs=${lt} out=${JSON.stringify(out)} note=${JSON.stringify(note)} status=${JSON.stringify(st)} diag=${JSON.stringify(diag)}`);
};
const only = process.argv[2];
for (const [k, v] of Object.entries(progs)) if (!only || only === k) await run(k, v);
// big sources (check runs on the main thread, 250 ms debounced + on Run)
const big = {
  chain: H + 'export function main(log: Log): void { const x: i32 = ' + '1 + '.repeat(60000) + '1; log.write(2, `${x}`); }',
  nest: H + 'export function main(log: Log): void { const x: i32 = ' + '('.repeat(20000) + '1' + ')'.repeat(20000) + '; log.write(2, `${x}`); }',
  nestarr: H + 'export function main(log: Log): void { const x = ' + '['.repeat(30000) + ']'.repeat(30000) + '; }',
  manyfn: H + Array.from({ length: 4000 }, (_, i) => `function f${i}(a: i32): i32 { return a + ${i}; }`).join('\n') + '\nexport function main(log: Log): void { log.write(2, `${f3999(1)}`); }',
  ifelse: H + 'export function main(log: Log): void { let x: i32 = 0; ' + 'if (x > 0) { '.repeat(5000) + 'x = 1;' + ' }'.repeat(5000) + ' }',
  strlit: H + 'export function main(log: Log): void { log.write(2, "' + 'a'.repeat(250000) + '"); }',
};
for (const [k, v] of Object.entries(big)) if (!only || only === k) { console.log(k, 'bytes', v.length); await run(k, v); }
// after all that: does Check / Run still work on the main wasm?
await run('after-hello', H + 'export function main(log: Log): void { log.write(2, "still alive"); }');
console.log('check status after:', await p.textContent('#pg-status'));
console.log('errors', JSON.stringify(errs.slice(0, 5)));
await b.close(); srv.close();
