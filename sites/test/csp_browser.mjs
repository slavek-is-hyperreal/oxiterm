// F6 verification: serve sites/dist/<site> with the REAL `_headers` rules applied (Cloudflare semantics: every matching
// rule contributes, same header values are joined with ", "), open every page and the live demo in headless Chromium and
// fail on any CSP violation, console error or page error. Also drives the playground (F3/F4 behaviour in a browser).
// Needs playwright-core (not a dependency of this repo): PLAYWRIGHT_CORE=/path/to/node_modules/playwright-core
// and a Chromium (PLAYWRIGHT_CHROMIUM or the playwright cache).  Usage: node sites/test/csp_browser.mjs
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.join(HERE, '../dist');
const CORE = process.env.PLAYWRIGHT_CORE || '/my_data/VectorLegis/.ocr-wasm-work/web/node_modules/playwright-core';
let chromium;
try { ({ chromium } = createRequire(import.meta.url)(CORE)); } catch { console.log('SKIP csp_browser: playwright-core not found (set PLAYWRIGHT_CORE)'); process.exit(0); }
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.xml': 'application/xml', '.txt': 'text/plain', '.svg': 'image/svg+xml' };
const globToRe = (g) => new RegExp('^' + g.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*') + '$');

function serve(site, port) {
  const root = path.join(DIST, site);
  const rules = [];
  let cur = null;
  for (const line of fs.readFileSync(path.join(root, '_headers'), 'utf8').split('\n')) {
    if (!line.trim() || line.startsWith('#')) continue;
    if (!/^\s/.test(line)) { cur = { re: globToRe(line.trim()), h: [] }; rules.push(cur); }
    else if (cur) { const i = line.indexOf(':'); cur.h.push([line.slice(0, i).trim(), line.slice(i + 1).trim()]); }
  }
  const srv = http.createServer((req, res) => {
    let u = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    const requested = u; // Cloudflare matches _headers against the requested path, even when it serves 404.html
    let f = path.join(root, u);
    if (!f.startsWith(root)) { res.writeHead(403); return res.end(); }
    if (u.endsWith('/')) f = path.join(f, 'index.html');
    const ok = fs.existsSync(f) && fs.statSync(f).isFile();
    if (!ok) f = path.join(root, '404.html');
    const merged = {};
    for (const r of rules) if (r.re.test(requested)) for (const [k, v] of r.h) merged[k] = merged[k] ? merged[k] + ', ' + v : v;
    res.writeHead(ok ? 200 : 404, { 'Content-Type': MIME[path.extname(f)] || 'application/octet-stream', ...merged });
    res.end(fs.readFileSync(f));
  });
  return new Promise((r) => srv.listen(port, '127.0.0.1', () => r(srv)));
}
const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));

const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM || ['/usr/bin/chromium', '/usr/bin/google-chrome'].find((p) => fs.existsSync(p)), args: ['--no-sandbox'] });
let bad = 0;
let pagesSeen = 0;
const fail = (m) => { bad++; console.log('  FAIL ' + m); };
let port = 18700;
for (const site of ['home', 'oxiui', 'oxiscript']) {
  const srv = await serve(site, ++port);
  const base = `http://127.0.0.1:${port}`;
  const root = path.join(DIST, site);
  const pages = walk(root)
    .filter((f) => f.endsWith('index.html') && !f.includes(path.sep + 'app' + path.sep))
    .map((f) => ('/' + path.relative(root, path.dirname(f)).split(path.sep).join('/') + '/').replace('//', '/'));
  pages.push('/app/?local=1&page=en/index.thtml', '/nope-404/');
  const ctx = await browser.newContext({ viewport: { width: 1000, height: 900 } });
  await ctx.addInitScript(() => {
    window.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(`${e.violatedDirective} ${e.blockedURI} ${e.sourceFile || ''}:${e.lineNumber}`));
  });
  const msgs = [];
  const watch = (page) => {
    page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) msgs.push(`${m.type()}: ${m.text()}`); });
    page.on('response', (r) => { if (r.status() >= 400) msgs.push(`http ${r.status()} ${r.url()}`); });
    page.on('pageerror', (e) => msgs.push('pageerror: ' + e.message));
  };
  for (const u of pages) {
    const page = await ctx.newPage();
    watch(page);
    msgs.length = 0;
    const resp = await page.goto(base + u, { waitUntil: 'load' });
    const csp = resp.headers()['content-security-policy'] || '';
    const is404 = resp.status() === 404;
    if (is404) { if (!(await page.content()).includes('http-equiv="Content-Security-Policy"')) fail(`${site}${u}: 404 page has no meta CSP`); }
    else if (!csp) fail(`${site}${u}: no CSP header`);
    if (new Set(csp.split(', ')).size > 1) fail(`${site}${u}: several policies merged: ${csp.slice(0, 80)}`);
    await page.waitForTimeout(u.startsWith('/app/') ? 4000 : 300);
    const starters = await page.$$('[data-iframe-src]');
    if (starters.length) {
      await starters[0].click();
      await page.waitForTimeout(5000);
      for (const fr of page.frames()) {
        if (fr === page.mainFrame()) continue;
        const v = await fr.evaluate(() => window.__csp).catch(() => null);
        if (v && v.length) fail(`${site}${u} iframe: ${v.join(' | ')}`);
      }
    }
    const v = await page.evaluate(() => window.__csp);
    if (v.length) fail(`${site}${u}: CSP violations: ${v.join(' | ')}`);
    const real = msgs.filter((m) => !(u.includes('nope') && /404/.test(m)));
    if (real.length) fail(`${site}${u}: console: ${real.slice(0, 3).join(' | ')}`);
    pagesSeen++;
    await page.close();
  }
  if (site === 'oxiscript') {
    for (const lang of ['/', '/pl/']) {
      const page = await ctx.newPage();
      watch(page);
      msgs.length = 0;
      await page.goto(base + lang + '#ex=mistakes', { waitUntil: 'load' });
      await page.waitForFunction(() => !document.getElementById('pg-check').disabled, null, { timeout: 15000 });
      await page.waitForSelector('#pg-diags button[data-fix]');
      // F3: typing disables the fix buttons at once
      await page.click('#pg-src');
      await page.keyboard.press('Control+End');
      await page.keyboard.type('\n');
      const dis = await page.$$eval('#pg-diags button', (bs) => bs.every((b) => b.disabled));
      if (!dis) fail(`${lang}: fix buttons not disabled after input`);
      await page.waitForSelector('#pg-diags button[data-fix]:not([disabled])', { timeout: 3000 });
      // F3 (stale click): a fix button that is still enabled but whose text moved must refuse cleanly and change nothing
      await page.$eval('#pg-src', (t) => { t.value = 'const a: i32 = 1;\nconst b: bool = a == 1;\n'; t.dispatchEvent(new Event('input')); });
      await page.waitForSelector('#pg-diags button[data-fix]:not([disabled])', { timeout: 3000 });
      const staleText = 'const z: bool = 2 == 2;\n' + (await page.$eval('#pg-src', (t) => t.value));
      await page.$eval('#pg-src', (t, v) => { t.value = v; }, staleText); // no input event: the buttons stay enabled
      await page.click('#pg-diags button[data-fix]');
      await page.waitForTimeout(300);
      const afterStale = await page.$eval('#pg-src', (t) => t.value);
      const status = await page.$eval('#pg-status', (e) => e.textContent);
      const wantStale = await page.evaluate(() => JSON.parse(document.getElementById('pg-labels').textContent).staleFix);
      console.log(`  ${lang} stale click: text unchanged ${afterStale === staleText}; status shown ${status === wantStale}`);
      if (afterStale !== staleText) fail(`${lang}: stale fix click changed the text`);
      if (status !== wantStale) fail(`${lang}: stale fix message not shown (got ${JSON.stringify(status)})`);
      // F4: Format, then Ctrl+Z restores the text
      await page.$eval('#pg-src', (t) => { t.value = 'export   function f( ):void{return;}\n'; t.dispatchEvent(new Event('input')); });
      await page.click('#pg-src');
      await page.keyboard.press('Control+End');
      await page.keyboard.type(' ');
      const before = await page.$eval('#pg-src', (t) => t.value);
      await page.click('#pg-format');
      await page.waitForTimeout(500);
      const formatted = await page.$eval('#pg-src', (t) => t.value);
      await page.click('#pg-src');
      await page.keyboard.press('Control+z');
      await page.waitForTimeout(100);
      const undone = await page.$eval('#pg-src', (t) => t.value);
      console.log(`  ${lang} format changed text: ${formatted !== before}; ctrl+z restored it: ${undone === before}`);
      if (formatted === before) fail(`${lang}: format did nothing`);
      if (undone !== before) fail(`${lang}: Ctrl+Z did not undo Format`);
      // Apply fix, then Ctrl+Z
      await page.$eval('#pg-src', (t) => { t.value = 'const a: i32 = 1;\nconst b: bool = a == 1;\n'; t.dispatchEvent(new Event('input')); });
      await page.waitForSelector('#pg-diags button[data-fix]:not([disabled])', { timeout: 3000 });
      const pre = await page.$eval('#pg-src', (t) => t.value);
      await page.click('#pg-diags button[data-fix]');
      await page.waitForTimeout(300);
      const fixed = await page.$eval('#pg-src', (t) => t.value);
      await page.click('#pg-src');
      await page.keyboard.press('Control+z');
      await page.waitForTimeout(100);
      const un = await page.$eval('#pg-src', (t) => t.value);
      console.log(`  ${lang} apply fix applied: ${fixed.includes('===')}; ctrl+z undid it: ${un === pre}`);
      if (!fixed.includes('===')) fail(`${lang}: apply fix did not apply`);
      if (un !== pre) fail(`${lang}: Ctrl+Z did not undo Apply fix`);
      // example switch with edits asks first, inline
      await page.selectOption('#pg-ex', 'hello');
      await page.waitForTimeout(200);
      const asked = await page.$eval('#pg-confirm', (e) => !e.hidden && e.querySelectorAll('button').length === 2);
      const still = await page.$eval('#pg-src', (t) => t.value.includes('const a: i32'));
      if (!asked || !still) fail(`${lang}: example switch did not ask / lost edits (asked=${asked})`);
      await page.click('#pg-confirm button:first-of-type');
      await page.waitForTimeout(300);
      const swapped = await page.$eval('#pg-src', (t) => !t.value.includes('const a: i32'));
      if (!swapped) fail(`${lang}: confirm did not switch the example`);
      const v = await page.evaluate(() => window.__csp);
      if (v.length) fail(`${lang}: CSP violations in playground: ${v.join(' | ')}`);
      if (msgs.length) fail(`${lang}: console: ${msgs.slice(0, 3).join(' | ')}`);
      if (process.env.SHOT_DIR) await page.screenshot({ path: path.join(process.env.SHOT_DIR, `pg${lang.replace(/\//g, '_')}.png`) });
      await page.close();
    }
  }
  await ctx.close();
  srv.close();
}
await browser.close();
console.log(`${pagesSeen} pages (+ demos, playground) opened under the real _headers; failures: ${bad}`);
process.exit(bad ? 1 : 0);
