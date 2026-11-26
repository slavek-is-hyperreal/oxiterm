// BAD ACTOR showcase: drives the OxiUI index of the static site in local mode (drag off-screen, counter, Tab, CSP in all frames, idle CPU).
// PLAYWRIGHT_CORE=... node sites/test/bad_actor_show_ui.mjs  (port 8146; needs sites/dist)
import fs from 'node:fs'; import path from 'node:path'; import http from 'node:http'; import { fileURLToPath } from 'node:url'; import { createRequire } from 'node:module';
const DIST = path.join(path.dirname(fileURLToPath(import.meta.url)), '../dist/' + (process.env.SITE || 'oxiui'));
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_CORE);
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.svg': 'image/svg+xml' };
const csp = fs.readFileSync(path.join(DIST, '_headers'), 'utf8').match(/\/app\/\n\s+Content-Security-Policy: (.*)/)[1];
const srv = http.createServer((req, res) => { const u = decodeURIComponent(new URL(req.url, 'http://x').pathname); let f = path.join(DIST, u); if (u.endsWith('/')) f += 'index.html'; if (!fs.existsSync(f)) { res.writeHead(404); return res.end(); } res.writeHead(200, { 'Content-Type': MIME[path.extname(f)] || 'application/octet-stream', 'Content-Security-Policy': csp }); res.end(fs.readFileSync(f)); });
await new Promise((r) => srv.listen(8146, '127.0.0.1', r));
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });
const W = +(process.env.VW || 1200), H = +(process.env.VH || 1300);
const ctx = await browser.newContext({ viewport: { width: W, height: H } });
await ctx.addInitScript(() => { window.__csp = []; document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(e.violatedDirective + ' ' + e.blockedURI)); });
const page = await ctx.newPage(); const msgs = [];
page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) msgs.push(m.text()); }); page.on('pageerror', (e) => msgs.push('pageerror ' + e.message));
await page.goto(`http://127.0.0.1:8146/app/?local=1&page=${process.env.LANG_ || 'en'}/index.thtml`); await page.waitForTimeout(3000);
const shot = (n) => page.screenshot({ path: `/tmp/oxiba-show/ui_${W}_${n}.png` });
await shot('0');
const steps = JSON.parse(process.env.STEPS || '[]');
for (const s of steps) {
  if (s.click) await page.mouse.click(...s.click);
  if (s.drag) { await page.mouse.move(...s.drag[0]); await page.mouse.down(); for (let i = 1; i <= 10; i++) await page.mouse.move(s.drag[0][0] + (s.drag[1][0] - s.drag[0][0]) * i / 10, s.drag[0][1] + (s.drag[1][1] - s.drag[0][1]) * i / 10); await page.mouse.up(); }
  if (s.wheel) await page.mouse.wheel(0, s.wheel);
  if (s.key) for (let i = 0; i < (s.n || 1); i++) await page.keyboard.press(s.key);
  await page.waitForTimeout(s.wait || 700);
  if (s.shot) await shot(s.shot);
}
console.log('csp', JSON.stringify(await page.evaluate(() => window.__csp)), 'msgs', JSON.stringify(msgs));
if (process.env.CPU) { const c = await ctx.newCDPSession(page); await c.send('Performance.enable'); const g = async () => Object.fromEntries((await c.send('Performance.getMetrics')).metrics.map((m) => [m.name, m.value])); const a = await g(); await page.waitForTimeout(+process.env.CPU); const b = await g(); console.log('cpu%', (((b.TaskDuration - a.TaskDuration) / (b.Timestamp - a.Timestamp)) * 100).toFixed(2), 'scriptMs', ((b.ScriptDuration - a.ScriptDuration) * 1000).toFixed(0)); }
await browser.close(); srv.close();
