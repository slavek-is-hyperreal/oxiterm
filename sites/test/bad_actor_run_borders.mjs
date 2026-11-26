// BAD ACTOR (identity): WCAG 1.4.11 on what really renders: control border vs the effective background behind it (needs >= 3:1).
import { serve, launch } from './bad_actor_run_lib.mjs';
const b = await launch();
const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
const pc = (s) => (s.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
const ratio = (a, d) => ((Math.max(lum(a), lum(d)) + .05) / (Math.min(lum(a), lum(d)) + .05));
let port = 19050, bad = 0;
for (const [site, sels] of [['oxiscript', ['#pg-src', '#pg-ex', '#pg-check', '#pg-run', '.btn']], ['oxiui', ['.btn', 'select', 'input', 'textarea']], ['home', ['.btn']]]) {
  const { srv, base } = await serve(site, ++port);
  for (const scheme of ['light', 'dark']) {
    const p = await (await b.newContext({ colorScheme: scheme })).newPage();
    for (const u of site === 'oxiui' ? ['/', '/gallery/'] : ['/']) {
      await p.goto(base + u, { waitUntil: 'networkidle' });
      for (const sel of sels) {
        if (!(await p.locator(sel).count())) continue; const r = await p.locator(sel).first().evaluate((e) => { const cs = getComputedStyle(e); let bg = null; for (let x = e.parentElement; x; x = x.parentElement) { const c = getComputedStyle(x).backgroundColor; if (!/rgba\(.*, 0\)|transparent/.test(c)) { bg = c; break; } } return { border: cs.borderTopColor, w: cs.borderTopWidth, style: cs.borderTopStyle, own: cs.backgroundColor, bg: bg || getComputedStyle(document.body).backgroundColor }; }).catch(() => null);
        if (!r) continue;
        const outer = pc(r.bg); const cr = ratio(pc(r.border), outer);
        const ownFill = !/rgba\(.*, 0\)/.test(r.own) ? ratio(pc(r.own), outer) : null;
        const flag = (r.style !== 'none' && cr < 3 && !(ownFill && ownFill >= 3)) ? 'FAIL' : 'ok';
        if (flag === 'FAIL') bad++;
        console.log(flag.padEnd(5), site + u, scheme, sel.padEnd(10), 'border', r.border, r.w, r.style, 'vs outside bg ->', cr.toFixed(2), ownFill ? 'fill/bg ' + ownFill.toFixed(2) : '');
      }
    }
  }
  srv.close();
}
console.log('border failures:', bad);
await b.close();
