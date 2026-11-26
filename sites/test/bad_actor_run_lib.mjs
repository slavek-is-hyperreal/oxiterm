// Shared helpers for the bad_actor_run*.mjs probes: serve sites/dist/<site> with the real _headers, launch Chromium.
import fs from 'node:fs'; import path from 'node:path'; import http from 'node:http';
import { fileURLToPath } from 'node:url'; import { createRequire } from 'node:module';
const HERE = path.dirname(fileURLToPath(import.meta.url));
export const DIST = path.join(HERE, '../dist');
const CORE = process.env.PLAYWRIGHT_CORE || '/vectorlegis_ssd_pool/żyćko/olx-mcp/node_modules/playwright-core';
export const { chromium } = createRequire(import.meta.url)(CORE);
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.xml': 'application/xml', '.txt': 'text/plain', '.svg': 'image/svg+xml' };
const globToRe = (g) => new RegExp('^' + g.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*') + '$');
export function serve(site, port, hook) {
  const root = path.join(DIST, site); const rules = []; let cur = null;
  for (const line of fs.readFileSync(path.join(root, '_headers'), 'utf8').split('\n')) {
    if (!line.trim() || line.startsWith('#')) continue;
    if (!/^\s/.test(line)) { cur = { re: globToRe(line.trim()), h: [] }; rules.push(cur); }
    else if (cur) { const i = line.indexOf(':'); cur.h.push([line.slice(0, i).trim(), line.slice(i + 1).trim()]); }
  }
  const log = [];
  const srv = http.createServer((req, res) => {
    let u = decodeURIComponent(new URL(req.url, 'http://x').pathname); log.push(u);
    const requested = u; let f = path.join(root, u);
    if (!f.startsWith(root)) { res.writeHead(403); return res.end(); }
    if (u.endsWith('/')) f = path.join(f, 'index.html');
    const ok = fs.existsSync(f) && fs.statSync(f).isFile();
    if (!ok) f = path.join(root, '404.html');
    const merged = {};
    for (const r of rules) if (r.re.test(requested)) for (const [k, v] of r.h) merged[k] = merged[k] ? merged[k] + ', ' + v : v;
    res.writeHead(ok ? 200 : 404, { 'Content-Type': MIME[path.extname(f)] || 'application/octet-stream', ...merged });
    res.end(fs.readFileSync(f));
  });
  return new Promise((r) => srv.listen(port, '127.0.0.1', () => r({ srv, log, base: `http://127.0.0.1:${port}` })));
}
export const launch = () => chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });
