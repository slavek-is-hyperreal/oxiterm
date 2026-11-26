// BAD ACTOR (run): paste a huge source into the textarea; the gutter/mirror repaint and the check run on the MAIN thread.
import { serve, launch } from './bad_actor_run_lib.mjs';
const { srv, base } = await serve('oxiscript', 18992); const b = await launch();
const p = await (await b.newContext({ viewport: { width: 1000, height: 900 } })).newPage();
await p.addInitScript(() => { window.__lt = []; try { new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__lt.push(Math.round(e.duration)); }).observe({ entryTypes: ['longtask'] }); } catch (e) {} });
await p.goto(base + '/', { waitUntil: 'networkidle' }); await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
// programmatic value= (a real paste is capped by maxlength=200000 CHARS, so 200000 newlines is the worst case a user can paste)
const cases = { '50000 newlines': '\n'.repeat(50_000), '200000 newlines (max the textarea accepts)': '\n'.repeat(200_000), '66000 lines of "a;"': 'a;\n'.repeat(66_000), '8000 lines of normal code (200 KB)': ('const x: i32 = 1;' + ' '.repeat(7) + '\n').repeat(8_000) };
for (const [k, v] of Object.entries(cases)) {
  await p.evaluate(() => { window.__lt = []; });
  const ms = await p.evaluate((v) => { const t = document.getElementById('pg-src'); const s = performance.now(); t.value = v; t.dispatchEvent(new Event('input')); return Math.round(performance.now() - s); }, v);
  await p.waitForTimeout(2500);
  console.log(k.padEnd(46), 'input handler (gutter+mirror) ms:', ms, '| longtasks after (debounced check):', JSON.stringify(await p.evaluate(() => window.__lt)));
}
await b.close(); srv.close();
