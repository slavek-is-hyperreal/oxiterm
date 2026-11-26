// BAD ACTOR (run): the module worker script is served with NO CSP header (only pages get one). Does the worker's connect-src fall away?
import http from 'node:http';
import { serve, launch } from './bad_actor_run_lib.mjs';
let hits = 0; const other = http.createServer((q, r) => { hits++; r.writeHead(200, { 'access-control-allow-origin': '*' }); r.end('x'); }); await new Promise((r) => other.listen(18999, '127.0.0.1', r));
const { srv, base } = await serve('oxiscript', 18991); const b = await launch(); const p = await (await b.newContext()).newPage();
await p.route('**/runner-worker.js', async (r) => { const resp = await r.fetch(); await r.fulfill({ response: resp, body: (await resp.text()) + "\nfetch('http://127.0.0.1:18999/from-worker').then(() => self.postMessage({ type: 'probe', ok: true }), (e) => self.postMessage({ type: 'probe', ok: false, e: String(e) }));\n" }); });
await p.addInitScript(() => { const W = window.Worker; window.Worker = function (u, o) { const w = new W(u, o); w.addEventListener('message', (e) => { if (e.data && e.data.type === 'probe') window.__probe = e.data; }); return w; }; });
await p.goto(base + '/', { waitUntil: 'networkidle' }); await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
const pageFetch = await p.evaluate(() => fetch('http://127.0.0.1:18999/from-page').then(() => 'allowed', (e) => 'blocked: ' + e.message));
await p.click('#pg-run'); await p.waitForTimeout(2500);
console.log('page fetch to other origin:', pageFetch, '| worker probe:', JSON.stringify(await p.evaluate(() => window.__probe)), '| other-origin server hits:', hits);
await b.close(); srv.close(); other.close();
