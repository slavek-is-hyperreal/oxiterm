// BAD ACTOR (run): drive the 5 s wall-clock timer (never hit in practice: fuel wins) via CDP CPU throttling and a slow/hung runner download.
import { serve, launch } from './bad_actor_run_lib.mjs';
const { srv, base } = await serve('oxiscript', 18953);
const b = await launch();
const H = 'import { Log } from "host:log";\n';
const LOOP = H + 'export function main(log: Log): void { let i: i32 = 0; while (true) { i = i +% 1; } }';
const HELLO = H + 'export function main(log: Log): void { log.write(2, "hello"); }';
async function open(routeFn) {
  const ctx = await b.newContext({ viewport: { width: 1000, height: 900 } });
  const p = await ctx.newPage(); if (routeFn) await routeFn(p);
  await p.goto(base + '/', { waitUntil: 'networkidle' }); await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
  return p;
}
const setSrc = (p, s) => p.evaluate((s) => { const t = document.getElementById('pg-src'); t.value = s; t.dispatchEvent(new Event('input')); document.getElementById('pg-out').textContent = '<<cleared>>'; document.getElementById('pg-out-note').textContent = ''; }, s);
const snap = async (p) => ({ out: (await p.textContent('#pg-out')).slice(0, 60), note: (await p.textContent('#pg-out-note')).slice(0, 120), status: (await p.textContent('#pg-status')).slice(0, 100), runDisabled: await p.evaluate(() => document.getElementById('pg-run').disabled), stopHidden: await p.evaluate(() => document.getElementById('pg-stop').hidden) });
// A. CPU throttle: fuel burn takes > 5 s wall clock
{
  const p = await open(); const cdp = await p.context().newCDPSession(p);
  await p.click('#pg-run'); await p.waitForTimeout(100); // warm worker first
  await setSrc(p, HELLO); await p.click('#pg-run'); await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: 250 });
  await setSrc(p, LOOP); const t0 = Date.now(); await p.click('#pg-run');
  await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: 40000 }).catch(() => {});
  console.log('A throttled loop: ms', Date.now() - t0, JSON.stringify(await snap(p)));
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: 1 });
  await setSrc(p, HELLO); await p.click('#pg-run'); await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: 20000 }).catch(() => {});
  await p.waitForTimeout(250); console.log('A fresh run after timeout:', JSON.stringify(await snap(p)));
}
// A2. timer path with a patched page constant (TIMEOUT_MS 5000 -> 40): proves kill + recovery with a hot worker
{
  const p = await open(async (p) => { await p.route('**/playground.js', async (r) => { const resp = await r.fetch(); const t = (await resp.text()).replace('  }, TIMEOUT_MS);', '  }, (window.__T || TIMEOUT_MS));'); await r.fulfill({ response: resp, body: t }); }); });
  await setSrc(p, HELLO); await p.click('#pg-run'); await p.waitForFunction(() => !document.getElementById('pg-run').disabled); await p.evaluate(() => { window.__T = 40; }); // warm
  await setSrc(p, LOOP); await p.click('#pg-run'); await p.waitForTimeout(800);
  await p.evaluate(() => { window.__T = 5000; }); console.log('A2 timer fired:', JSON.stringify(await snap(p)));
  await setSrc(p, HELLO); await p.click('#pg-run'); await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: 20000 }).catch(() => {}); await p.waitForTimeout(400);
  console.log('A2 run after timeout:', JSON.stringify(await snap(p)));
}
// B. runner wasm download hangs forever: no timer is armed (timer starts at postRun, after 'ready')
{
  const p = await open(async (p) => { await p.route('**/oxiscript_runner_bg.wasm', () => {}); });
  await setSrc(p, HELLO); await p.click('#pg-run'); await p.waitForTimeout(8000);
  console.log('B hung runner after 8 s:', JSON.stringify(await snap(p)));
  await p.click('#pg-stop'); await p.waitForTimeout(250); console.log('B after Stop:', JSON.stringify(await snap(p)));
  await p.unroute('**/oxiscript_runner_bg.wasm'); await setSrc(p, HELLO); await p.click('#pg-run');
  await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: 20000 }).catch(() => {}); await p.waitForTimeout(250);
  console.log('B run after unblock:', JSON.stringify(await snap(p)));
}
// C. runner fetch fails (404/abort)
{
  const p = await open(async (p) => { await p.route('**/oxiscript_runner_bg.wasm', (r) => r.abort()); });
  await setSrc(p, HELLO); await p.click('#pg-run'); await p.waitForTimeout(1500);
  console.log('C aborted runner:', JSON.stringify(await snap(p)));
}
// D. Stop spam / double click / Run during run
{
  const p = await open(); await setSrc(p, LOOP);
  await p.evaluate(() => { const r = document.getElementById('pg-run'); r.click(); r.click(); r.click(); });
  await p.evaluate(() => { document.getElementById('pg-stop').click(); document.getElementById('pg-stop').click(); });
  await p.waitForTimeout(300); console.log('D spam:', JSON.stringify(await snap(p)));
  for (let i = 0; i < 15; i++) { await setSrc(p, LOOP); await p.click('#pg-run'); await p.waitForTimeout(20); await p.click('#pg-stop').catch(() => {}); }
  await setSrc(p, HELLO); await p.click('#pg-run').catch(() => {}); await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: 20000 }).catch(() => {}); await p.waitForTimeout(250);
  console.log('D after 15 run/stop cycles:', JSON.stringify(await snap(p)));
}
await b.close(); srv.close();
