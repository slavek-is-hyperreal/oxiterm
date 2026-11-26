// BAD ACTOR (run): lazy runner, 320/390 widths, output injection, flood, Stop recovery, delayed runner. Prints a result table.
import { serve, launch } from './bad_actor_run_lib.mjs';
const { srv, log, base } = await serve('oxiscript', 18951);
const b = await launch();
const out = (k, v) => console.log(k.padEnd(34), v);
const ctx = await b.newContext({ viewport: { width: 1000, height: 900 } });
await ctx.addInitScript(() => { window.__csp = []; document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(e.violatedDirective + ' ' + e.blockedURI)); });
const p = await ctx.newPage(); const errs = []; p.on('pageerror', (e) => errs.push(e.message)); p.on('console', (m) => { if (m.type() === 'error') errs.push(m.text()); });
await p.goto(base + '/', { waitUntil: 'networkidle' });
await p.waitForFunction(() => !document.getElementById('pg-run').disabled);
out('runner fetched before Run', log.filter((u) => /runner/.test(u)).join(',') || 'none');
out('modulepreload in html', String(/modulepreload/.test(await p.content())));
out('all fetched before Run', log.join(' '));
const setSrc = async (s) => { await p.evaluate((s) => { const t = document.getElementById('pg-src'); t.value = s; t.dispatchEvent(new Event('input')); }, s); };
const run = async (s, waitMs = 15000) => {
  await setSrc(s); const t0 = Date.now();
  await p.click('#pg-run');
  await p.waitForFunction(() => !document.getElementById('pg-run').disabled, null, { timeout: waitMs }).catch(() => {});
  return { ms: Date.now() - t0, out: await p.textContent('#pg-out'), note: await p.textContent('#pg-out-note'), status: await p.textContent('#pg-status') };
};
let r = await run('import { Log } from "host:log";\nexport function main(log: Log): void { log.write(2, "hi"); }\n');
out('first run', JSON.stringify(r)); out('runner fetched after Run', log.filter((u) => /runner/.test(u)).join(','));
// injection
r = await run('import { Log } from "host:log";\nexport function main(log: Log): void { log.write(2, "<img src=x onerror=window.__pwn=1><script>window.__pwn=2</script>\\u{202e}rtl\\u{1b}[31mred\\u{7}\\u{0}nul"); }\n');
out('inject', JSON.stringify(r.out)); out('__pwn', String(await p.evaluate(() => window.__pwn)));
out('img in #pg-out', String(await p.evaluate(() => document.querySelectorAll('#pg-out img,#pg-out script').length)));
// flood: 1000 lines, 64 KiB single line, many
r = await run('import { Log } from "host:log";\nexport function main(log: Log): void { for (const i of range(0, 5000)) { log.write(2, `line ${i}`); } }\n');
out('flood 5000 lines', `ms=${r.ms} shownLines=${r.out.split('\n').length} note=${r.note}`);
const wide = 'x'.repeat(60000);
r = await run(`import { Log } from "host:log";\nexport function main(log: Log): void { log.write(2, "${wide}"); }\n`);
out('one 60000-char line', `ms=${r.ms} len=${r.out.length} note=${r.note} status=${r.status}`);
out('page scrollWidth@1000', String(await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth].join('/'))));
for (const w of [320, 390]) { await p.setViewportSize({ width: w, height: 800 }); await p.waitForTimeout(200); out('scrollWidth@' + w + ' (long line shown)', String(await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth].join('/')))); }
await p.setViewportSize({ width: 1000, height: 900 });
out('csp violations', JSON.stringify(await p.evaluate(() => window.__csp))); out('page errors', JSON.stringify(errs));
await b.close(); srv.close();
