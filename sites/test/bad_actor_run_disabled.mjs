import { serve, launch } from './bad_actor_run_lib.mjs';
const b = await launch(); const { srv, base } = await serve('oxiscript', 18990);
const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
const pc = (s) => (s.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
for (const scheme of ['light', 'dark']) {
  const ctx = await b.newContext({ colorScheme: scheme }); const p = await ctx.newPage();
  await p.route('**/oxiscript_playground_bg.wasm', (r) => r.abort());
  await p.goto(base + '/', { waitUntil: 'networkidle' }); await p.waitForTimeout(300);
  const r = await p.evaluate(() => { const e = document.getElementById('pg-run'); const cs = getComputedStyle(e); const card = getComputedStyle(e.closest('section,div')).backgroundColor; return { dis: e.disabled, color: cs.color, bg: cs.backgroundColor, op: cs.opacity, bodybg: getComputedStyle(document.body).backgroundColor, status: document.getElementById('pg-status').textContent, border: cs.borderColor }; });
  const f = pc(r.color), g = pc(r.bg.startsWith('rgba(0, 0, 0, 0)') ? r.bodybg : r.bg); const c = (a, d) => ((Math.max(lum(a), lum(d)) + .05) / (Math.min(lum(a), lum(d)) + .05)).toFixed(2);
  console.log(scheme, JSON.stringify(r), 'text/bg=', c(f, g), '(disabled controls are WCAG-exempt)');
  // axolotl coral on light bg: sample the hero svg img pixels
  await ctx.close();
}
await b.close(); srv.close();
