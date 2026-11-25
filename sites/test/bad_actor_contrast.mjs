// BAD ACTOR 8.W: WCAG contrast from the real CSS cascade (base :root, base light, site :root, site light).
// F9 fix: the old 'line on bg >= 3' pair was retargeted to --ctl (the form-control border token, WCAG 1.4.11).
// --line stays the decorative card/rule colour (exempt from 1.4.11) and is no longer tested as a control border.
import fs from 'node:fs'; import path from 'node:path'; import { fileURLToPath } from 'node:url';
const H = path.dirname(fileURLToPath(import.meta.url));
const vars = (s) => Object.fromEntries([...s.matchAll(/--([a-z-]+):\s*(#[0-9a-fA-F]{6})/g)].map((m) => [m[1], m[2]]));
const lum = (h) => { const c = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)); return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]; };
const cr = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const split = (css) => { const light = [...css.matchAll(/@media \(prefers-color-scheme: light\)\s*\{([\s\S]*?\})\s*\}/g)].map((m) => m[1]).join('\n'); return { dark: vars(css.replace(/@media \(prefers-color-scheme: light\)\s*\{[\s\S]*?\}\s*\}/g, '')), light: vars(light) }; };
// F9: the retargeted pair is only honest if the controls really use --ctl for their borders
{
  const css = fs.readFileSync(path.join(H, '../shared/site.css'), 'utf8');
  for (const sel of ['.btn {', '.pg-bar select {', '.pg-editor {']) {
    const rule = css.split('\n').find((l) => l.startsWith(sel)) || '';
    if (!/border:[^;]*var\(--ctl\)/.test(rule)) { console.log(`FAIL: ${sel} border does not use var(--ctl)`); process.exitCode = 1; }
  }
}
const base = split(fs.readFileSync(path.join(H, '../shared/site.css'), 'utf8').split('/* top bars */')[0]);
for (const site of ['home', 'oxiui', 'oxiscript']) {
  const s = split(fs.readFileSync(path.join(H, `../shared/site.css.d/${site}.css`), 'utf8'));
  for (const mode of ['dark', 'light']) {
    const t = { ...base.dark, ...(mode === 'light' ? base.light : {}), ...s.dark, ...(mode === 'light' ? s.light : {}) };
    const pairs = [['fg', 'bg', 4.5], ['muted', 'bg', 4.5], ['muted', 'card', 4.5], ['muted', 'code-bg', 4.5], ['link', 'bg', 4.5], ['link', 'code-bg', 4.5], ['link', 'card', 4.5], ['acc', 'bg', 4.5], ['acc', 'code-bg', 4.5], ['acc-fg', 'acc', 4.5], ['warn', 'bg', 4.5], ['err', 'bg', 3], ['ok', 'bg', 4.5], ['ctl', 'bg', 3], ['ctl', 'card', 3], ['ctl', 'code-bg', 3], ['acc', 'card', 3]];
    for (const [a, b, min] of pairs) { const r = cr(t[a], t[b]); console.log(`${site.padEnd(9)} ${mode.padEnd(5)} ${(a + ' on ' + b).padEnd(20)} ${r.toFixed(2)} ${r < min ? 'FAIL (<' + min + ')' : ''}`); }
  }
}
