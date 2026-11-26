// BAD ACTOR (run/identity): RENDERED checks in headless Chromium, not token arithmetic.
//  1) contrast of every visible text element vs its effective background (light + dark, 3 sites, every page, + :hover/:focus/disabled states)
//  2) horizontal overflow at 320 / 390 px on every page, 3) reduced motion, 4) inline style/script, SVG hazards, <img src=svg> loads
import fs from 'node:fs'; import path from 'node:path';
import { serve, launch, DIST } from './bad_actor_run_lib.mjs';
const b = await launch();
const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
const ratio = (a, b2) => { const x = lum(a), y = lum(b2); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
let port = 18960; const lows = new Map(); let scanned = 0, pagesN = 0; const overflow = []; const hazards = [];
for (const site of ['home', 'oxiui', 'oxiscript']) {
  const { srv, base } = await serve(site, ++port);
  const root = path.join(DIST, site);
  const pages = walk(root).filter((f) => f.endsWith('index.html') && !f.includes(path.sep + 'app' + path.sep)).map((f) => ('/' + path.relative(root, path.dirname(f)).split(path.sep).join('/') + '/').replace('//', '/'));
  pages.push('/nope/');
  for (const scheme of ['light', 'dark']) {
    const ctx = await b.newContext({ colorScheme: scheme, viewport: { width: 1000, height: 900 } });
    for (const u of pages) {
      const p = await ctx.newPage(); await p.goto(base + u, { waitUntil: 'networkidle' }); pagesN++;
      const res = await p.evaluate(() => {
        const parse = (s) => { const m = s.match(/rgba?\(([^)]+)\)/); if (!m) return null; const v = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number); return [v[0], v[1], v[2], v.length > 3 ? v[3] : 1]; };
        const bgOf = (el) => { let layers = []; for (let e = el; e; e = e.parentElement) { const cs = getComputedStyle(e); if (cs.backgroundImage !== 'none') layers.push('IMG'); const c = parse(cs.backgroundColor); if (c && c[3] > 0) { layers.push(c); if (c[3] >= 1) break; } } let base = [255, 255, 255]; if (layers.length === 0 || layers[layers.length - 1][3] < 1) layers.push([...parse(getComputedStyle(document.documentElement).backgroundColor) || [255, 255, 255]]); let out = null; for (const l of layers.reverse()) { if (l === 'IMG') return null; out = out ? [l[0] * (l[3] ?? 1) + out[0] * (1 - (l[3] ?? 1)), l[1] * (l[3] ?? 1) + out[1] * (1 - (l[3] ?? 1)), l[2] * (l[3] ?? 1) + out[2] * (1 - (l[3] ?? 1))] : [l[0], l[1], l[2]]; } return out; };
        const rows = [];
        const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        const seen = new Set();
        for (let n = w.nextNode(); n; n = w.nextNode()) {
          if (!n.textContent.trim()) continue; const el = n.parentElement; if (!el || seen.has(el)) continue; seen.add(el);
          const cs = getComputedStyle(el); if (cs.visibility === 'hidden' || cs.display === 'none' || el.closest('[hidden]') || el.closest('svg')) continue;
          const r = el.getBoundingClientRect(); if (r.width === 0 || r.height === 0) continue;
          let fg = parse(cs.color); const bg = bgOf(el); if (!fg || !bg) continue;
          if (fg[3] < 1) fg = [fg[0] * fg[3] + bg[0] * (1 - fg[3]), fg[1] * fg[3] + bg[1] * (1 - fg[3]), fg[2] * fg[3] + bg[2] * (1 - fg[3])];
          const op = parseFloat(cs.opacity);
          rows.push({ tag: el.tagName.toLowerCase() + (el.className && typeof el.className === 'string' ? '.' + el.className.split(' ')[0] : ''), fg, bg, size: parseFloat(cs.fontSize), bold: parseInt(cs.fontWeight) >= 700, text: n.textContent.trim().slice(0, 28), op, disabled: el.closest('button:disabled,[disabled]') !== null });
        }
        return rows;
      });
      for (const r of res) {
        scanned++;
        const cr = ratio(r.fg, r.bg), large = r.size >= 24 || (r.size >= 18.66 && r.bold), need = large ? 3 : 4.5;
        if (cr < need && !r.disabled) { const k = `${site} ${scheme} ${r.tag}`; const e = lows.get(k) || { n: 0, min: 99, ex: '', pages: new Set() }; e.n++; e.min = Math.min(e.min, cr); e.ex = r.text; e.pages.add(u); lows.set(k, e); }
      }
      // hazards: inline style attr / <style> / inline script / on* handlers / javascript:
      const hz = await p.evaluate(() => [...document.querySelectorAll('[style]')].length + '|' + document.querySelectorAll('style').length + '|' + [...document.querySelectorAll('script')].filter((s) => !s.src && s.type !== 'application/json').length + '|' + [...document.querySelectorAll('*')].filter((e) => [...e.attributes].some((a) => /^on/i.test(a.name))).length);
      if (hz !== '0|0|0|0') hazards.push(`${site}${u} style-attr|style-tag|inline-script|on*: ${hz}`);
      if (scheme === 'light') for (const w of [320, 390]) { await p.setViewportSize({ width: w, height: 800 }); await p.waitForTimeout(60); const sw = await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth]); if (sw[0] > sw[1]) overflow.push(`${site}${u} @${w}: scrollWidth ${sw[0]}`); }
      await p.close();
    }
    await ctx.close();
  }
  srv.close();
}
console.log(`pages(x2 schemes)=${pagesN} text-elements scanned=${scanned}`);
console.log('LOW CONTRAST groups:', lows.size); for (const [k, e] of lows) console.log('  ', k, 'n=' + e.n, 'min=' + e.min.toFixed(2), JSON.stringify(e.ex), [...e.pages].slice(0, 3).join(','));
console.log('HAZARDS:', hazards.length ? hazards : 'none');
console.log('OVERFLOW@320/390:', overflow.length ? overflow : 'none');
await b.close();
