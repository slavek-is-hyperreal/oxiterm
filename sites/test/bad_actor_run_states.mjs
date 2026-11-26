// BAD ACTOR (identity): rendered colours of interactive states (hover, focus-visible, disabled, visited, mascot) on all 3 sites, both schemes.
import { serve, launch } from './bad_actor_run_lib.mjs';
const b = await launch();
const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
const ratio = (a, b2) => { const x = lum(a), y = lum(b2); return ((Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)).toFixed(2); };
const pc = (s) => (s.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
let port = 18970;
for (const [site, sel] of [['oxiscript', '#pg-check'], ['oxiscript', '#pg-run'], ['oxiscript', '.btn'], ['oxiui', '.btn'], ['home', '.btn'], ['home', 'a.nav-link, nav a']]) {
  const { srv, base } = await serve(site, ++port);
  for (const scheme of ['light', 'dark']) {
    const ctx = await b.newContext({ colorScheme: scheme, viewport: { width: 1000, height: 900 } });
    const p = await ctx.newPage(); await p.goto(base + '/', { waitUntil: 'networkidle' }); await p.waitForTimeout(500);
    const el = p.locator(sel).first(); if (!(await el.count())) { console.log(site, scheme, sel, 'not found'); continue; }
    const read = () => el.evaluate((e) => { const cs = getComputedStyle(e); let bgEl = e, bg; for (; bgEl; bgEl = bgEl.parentElement) { const c = getComputedStyle(bgEl).backgroundColor; if (!/rgba\(.*, 0\)|transparent/.test(c)) { bg = c; break; } } return { color: cs.color, bg: cs.backgroundColor, bgEff: bg, outline: cs.outlineColor, outW: cs.outlineWidth, outStyle: cs.outlineStyle, off: cs.outlineOffset, opacity: cs.opacity, disabled: e.disabled === true }; });
    const page = await p.evaluate(() => getComputedStyle(document.body).backgroundColor);
    const base0 = await read(); await el.hover().catch(() => {}); const hov = await read(); await p.mouse.move(0, 0);
    await el.focus(); await p.keyboard.press('Shift+Tab'); await p.keyboard.press('Tab'); const foc = await read();
    const row = (n, s) => `${site} ${scheme} ${sel} ${n}: text ${ratio(pc(s.color), pc(s.bg.startsWith('rgba(0, 0, 0, 0)') ? s.bgEff : s.bg))}  ring ${s.outStyle} ${s.outW} ${ratio(pc(s.outline), pc(page))} vs page; disabled=${s.disabled} opacity=${s.opacity}`;
    console.log(row('rest ', base0)); console.log(row('hover', hov)); console.log(row('focus', foc));
    await ctx.close();
  }
  srv.close();
}
await b.close();
