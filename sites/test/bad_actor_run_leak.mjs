// BAD ACTOR (run): repeated memory-hungry runs in ONE runner instance: does the wasm linear memory (and time) keep growing?
import { serve, launch } from './bad_actor_run_lib.mjs';
const { srv, base } = await serve('oxiscript', 18955);
const b = await launch(); const p = await (await b.newContext()).newPage();
await p.goto(base + '/', { waitUntil: 'networkidle' });
const url = await p.evaluate(() => document.getElementById('playground').dataset.runner);
const H = 'import { Log } from "host:log";\nimport { State } from "host:state";\n';
const progs = {
  oom: H + 'export function main(log: Log): void { let s = "xxxxxxxx"; for (const i of range(0, 40)) { s = s + s; } }',
  state: H + 'export function main(log: Log, st: State): void { for (const i of range(0, 100000)) { const r = st.open(`k${i}`); } }',
  fuel: H + 'export function main(log: Log): void { let i: i32 = 0; while (true) { i = i +% 1; } }',
  big: H + 'export function main(log: Log): void { const a: i64[] = []; for (const i of range(0, 1000000)) { a.push(i as i64); } }',
};
const res = await p.evaluate(async ([u, progs]) => {
  const m = await import(u); const ex = await m.default(); const out = {};
  for (const [k, s] of Object.entries(progs)) {
    const sizes = []; const t0 = performance.now();
    for (let i = 0; i < 100; i++) { m.run(s, 0); if (i % 25 === 24) sizes.push(Math.round(ex.memory.buffer.byteLength / 1048576)); }
    out[k] = { wasmMiB: sizes, msPerRun: Math.round((performance.now() - t0) / 100) };
  }
  return out;
}, [url, progs]);
console.log(JSON.stringify(res));
await b.close(); srv.close();
