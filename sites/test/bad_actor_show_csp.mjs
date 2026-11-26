// BAD ACTOR showcase: CSP/console sweep in ALL frames, laziness and idle CPU of the showcase pages under the real _headers.
// PLAYWRIGHT_CORE=... node sites/test/bad_actor_show_csp.mjs   (ports 8143-8145)
import fs from 'node:fs'; import path from 'node:path'; import http from 'node:http'; import { fileURLToPath } from 'node:url'; import { createRequire } from 'node:module';
const DIST = path.join(path.dirname(fileURLToPath(import.meta.url)), '../dist');
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_CORE);
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.svg': 'image/svg+xml' };
const globToRe = (g) => new RegExp('^' + g.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*') + '$');
function serve(site, port, log) {
  const root = path.join(DIST, site); const rules = []; let cur;
  for (const line of fs.readFileSync(path.join(root, '_headers'), 'utf8').split('\n')) { if (!line.trim() || line.startsWith('#')) continue; if (!/^\s/.test(line)) { cur = { re: globToRe(line.trim()), h: [] }; rules.push(cur); } else { const i = line.indexOf(':'); cur.h.push([line.slice(0, i).trim(), line.slice(i + 1).trim()]); } }
  const srv = http.createServer((req, res) => {
    const u = decodeURIComponent(new URL(req.url, 'http://x').pathname); log.push(u);
    let f = path.join(root, u); if (u.endsWith('/')) f = path.join(f, 'index.html');
    const ok = fs.existsSync(f) && fs.statSync(f).isFile(); if (!ok) f = path.join(root, '404.html');
    const m = {}; for (const r of rules) if (r.re.test(u)) for (const [k, v] of r.h) m[k] = m[k] ? m[k] + ', ' + v : v;
    res.writeHead(ok ? 200 : 404, { 'Content-Type': MIME[path.extname(f)] || 'application/octet-stream', ...m }); res.end(fs.readFileSync(f));
  });
  return new Promise((r) => srv.listen(port, '127.0.0.1', () => r(srv)));
}
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });
let bad = 0; const fail = (m) => { bad++; console.log('FAIL ' + m); };
let port = 8142;
for (const site of ['home', 'oxiui', 'oxiscript']) {
  const log = []; const srv = await serve(site, ++port, log); const base = `http://127.0.0.1:${port}`;
  for (const [w, h] of [[1200, 900], [390, 800]]) for (const lang of ['en', 'pl']) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    await ctx.addInitScript(() => { window.__csp = []; document.addEventListener('securitypolicyviolation', (e) => { const s = `${e.violatedDirective} ${e.blockedURI}`; window.__csp.push(s); try { window.top.__csp.push('(frame) ' + s); } catch {} }); });
    const page = await ctx.newPage(); const msgs = [];
    page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) msgs.push(m.text()); });
    page.on('pageerror', (e) => msgs.push('pageerror ' + e.message));
    page.on('response', (r) => { if (r.status() >= 400) msgs.push('http ' + r.status() + ' ' + r.url()); });
    // HTML index: lazy before start
    log.length = 0;
    await page.goto(`${base}${lang === 'pl' ? '/pl/' : '/'}`, { waitUntil: 'load' }); await page.waitForTimeout(800);
    const pre = log.filter((u) => /pages\.json|\.wasm|lottie|\.riv|\.mmd/.test(u));
    console.log(`${site} ${lang} ${w}: HTML index requests before demo: ${log.length}; heavy: ${JSON.stringify(pre)}`);
    // app direct
    log.length = 0;
    await page.goto(`${base}/app/?local=1&page=${lang}/index.thtml`, { waitUntil: 'load' }); await page.waitForTimeout(5000);
    const heavy = log.filter((u) => /pages\.json|\.wasm|lottie|\.riv|\.mmd|\.svg|\.json/.test(u));
    console.log(`  app ${w}: ${JSON.stringify(heavy)}`);
    const csp = await page.evaluate(() => window.__csp);
    if (csp.length) fail(`${site} ${lang} ${w}: CSP ${JSON.stringify(csp)}`);
    if (msgs.length) fail(`${site} ${lang} ${w}: console ${JSON.stringify(msgs.slice(0, 5))}`);
    await page.screenshot({ path: `/tmp/oxiba-show/shot_${site}_${lang}_${w}.png` });
    await ctx.close();
  }
  srv.close();
}
await browser.close(); console.log(bad ? `FAILURES ${bad}` : 'ok'); process.exit(bad ? 1 : 0);
