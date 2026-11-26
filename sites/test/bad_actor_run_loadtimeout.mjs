// F6: a runner download that never finishes ends with the load-timeout message (the 20 s constant is patched to 2 s),
// Run is usable again, and the run timer (TIMEOUT_MS) is a different constant.
import { serve, launch } from './bad_actor_run_lib.mjs';
const { srv, base } = await serve('oxiscript', 18955);
const b = await launch(); const p = await (await b.newContext()).newPage();
let hang = true;
await p.route('**/playground.js', async (r) => { const resp = await r.fetch(); const t = await resp.text(); if (!t.includes('LOAD_TIMEOUT_MS = 20000')) { console.log('FAIL: constant not found'); process.exit(1); } await r.fulfill({ response: resp, body: t.replace('LOAD_TIMEOUT_MS = 20000', 'LOAD_TIMEOUT_MS = 2000') }); });
await p.route('**/oxiscript_runner_bg.wasm', (r) => (hang ? undefined : r.continue()));
await p.goto(base + '/', { waitUntil: 'networkidle' }); await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
await p.click('#pg-run');
await p.waitForTimeout(800);
const mid = await p.textContent('#pg-status');
await p.waitForFunction(() => /did not load/.test(document.getElementById('pg-status').textContent), null, { timeout: 8000 });
const st = await p.evaluate(() => ({ status: document.getElementById('pg-status').textContent, note: document.getElementById('pg-out-note').textContent, runDisabled: document.getElementById('pg-run').disabled, stopHidden: document.getElementById('pg-stop').hidden }));
console.log('while loading:', mid, '\nafter timeout:', JSON.stringify(st));
let fail = st.runDisabled || !st.stopHidden || !/Stop/.test(mid);
hang = false; await p.unroute('**/oxiscript_runner_bg.wasm');
await p.click('#pg-run'); await p.waitForFunction(() => /Finished/.test(document.getElementById('pg-status').textContent), null, { timeout: 20000 });
console.log('retry:', await p.textContent('#pg-status'));
await b.close(); srv.close();
if (fail) { console.log('FAIL'); process.exit(1); }
console.log('ok');
