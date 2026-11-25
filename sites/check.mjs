#!/usr/bin/env node
// CI-ready checks for the three static sites (run after build.sh):
//  1. i18n parity: every language file has exactly the keys of en.json
//  2. every internal link / asset / anchor in dist/**/*.html resolves; every THTML page referenced
//     (app links, event-htmx navigation) exists in app/pages.json
//  3. light HTML well-formedness (tag nesting, duplicate ids)
//  4. Latin is not published unless SITES_LANGS enables it
// Exit code 1 on any problem. External (http/https) links are not fetched (CI has no network guarantee).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.join(HERE, 'dist');
const SITES = ['home', 'oxiui', 'oxiscript'];
const errors = [];
const err = (m) => errors.push(m);

// 1. i18n parity
const flat = (o, p = '', out = {}) => {
  if (Array.isArray(o)) { out[p + '[]'] = o.length; o.forEach((v, i) => flat(v, `${p}[${i}]`, out)); }
  else if (o && typeof o === 'object') for (const [k, v] of Object.entries(o)) flat(v, p ? `${p}.${k}` : k, out);
  else out[p] = typeof o;
  return out;
};
const en = flat(JSON.parse(fs.readFileSync(path.join(HERE, 'i18n/en.json'), 'utf8')));
for (const l of ['pl', 'la']) {
  const o = flat(JSON.parse(fs.readFileSync(path.join(HERE, `i18n/${l}.json`), 'utf8')));
  for (const k of Object.keys(en)) if (!(k in o)) err(`i18n ${l}: missing ${k}`); else if (o[k] !== en[k]) err(`i18n ${l}: ${k} differs from en (${o[k]} vs ${en[k]})`);
  for (const k of Object.keys(o)) if (!(k in en)) err(`i18n ${l}: extra key ${k}`);
}

const langs = (process.env.SITES_LANGS || 'pl,en').split(',').map((s) => s.trim());
const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));

const VOID = new Set(['meta', 'link', 'br', 'hr', 'img', 'input', 'source', 'wbr']);
function wellFormed(file, html) {
  const stack = [];
  const ids = new Set();
  const re = /<!--[\s\S]*?-->|<(\/?)([a-zA-Z][a-zA-Z0-9-]*)((?:"[^"]*"|'[^']*'|[^>"'])*)>/g;
  let m;
  const body = html.replace(/<script[\s\S]*?<\/script>/g, '<script></script>').replace(/<pre[\s\S]*?<\/pre>/g, '<pre></pre>');
  while ((m = re.exec(body))) {
    if (m[0].startsWith('<!--')) continue;
    const [, close, tag, attrs] = m;
    const t = tag.toLowerCase();
    if (t === 'doctype') continue;
    if (!close) {
      const id = /\sid="([^"]*)"/.exec(attrs);
      if (id) { if (ids.has(id[1])) err(`${file}: duplicate id ${id[1]}`); ids.add(id[1]); }
      if (!VOID.has(t) && !attrs.trim().endsWith('/')) stack.push(t);
    } else {
      const top = stack.pop();
      if (top !== t) { err(`${file}: </${t}> closes <${top}>`); return ids; }
    }
  }
  if (stack.length) err(`${file}: unclosed <${stack[stack.length - 1]}>`);
  return ids;
}

for (const site of SITES) {
  const root = path.join(DIST, site);
  if (!fs.existsSync(root)) { err(`dist/${site} missing: run sites/build.sh`); continue; }
  const files = walk(root);
  const rel = (f) => '/' + path.relative(root, f).split(path.sep).join('/');
  const exists = new Set(files.map(rel));
  const pagesFile = path.join(root, 'app/pages.json');
  const pages = fs.existsSync(pagesFile) ? JSON.parse(fs.readFileSync(pagesFile, 'utf8')).pages : {};
  const idsOf = {};
  const htmls = files.filter((f) => f.endsWith('.html'));
  for (const f of htmls) {
    if (rel(f).startsWith('/app/')) continue;
    idsOf[rel(f)] = wellFormed(`${site}${rel(f)}`, fs.readFileSync(f, 'utf8'));
  }
  const resolve = (from, href) => {
    let u = href.split('#')[0].split('?')[0];
    if (!u) return from;
    if (!u.startsWith('/')) u = path.posix.join(path.posix.dirname(from), u);
    if (u.endsWith('/')) u += 'index.html';
    return u;
  };
  for (const f of htmls) {
    const from = rel(f);
    if (from.startsWith('/app/')) continue;
    const html = fs.readFileSync(f, 'utf8');
    for (const m of html.matchAll(/\s(?:href|src)="([^"]*)"/g)) {
      const h = m[1].replace(/&amp;/g, '&');
      if (/^(https?:|mailto:|data:)/.test(h)) continue;
      const target = resolve(from, h);
      if (!exists.has(target)) { err(`${site}${from}: broken link ${h}`); continue; }
      const hash = h.includes('#') ? h.split('#')[1] : '';
      if (hash && idsOf[target] && !idsOf[target].has(hash)) err(`${site}${from}: missing anchor ${h}`);
      if (h.startsWith('/app/') && h.includes('page=')) {
        const p = decodeURIComponent(new URL(h, 'http://x').searchParams.get('page'));
        if (!(p in pages)) err(`${site}${from}: OxiUI page ${p} not in pages.json`);
      }
    }
    for (const m of html.matchAll(/data-iframe-src="([^"]*)"/g)) {
      const p = decodeURIComponent(new URL(m[1].replace(/&amp;/g, '&'), 'http://x').searchParams.get('page'));
      if (!(p in pages)) err(`${site}${from}: demo page ${p} not in pages.json`);
    }
    if (!langs.includes('la') && /hreflang="la"|lang="la"|href="\/la\//.test(html)) err(`${site}${from}: Latin leaked into a build without la`);
  }
  // THTML navigation targets
  for (const [name, text] of Object.entries(pages)) {
    for (const m of text.matchAll(/event-htmx="([^"]+)"/g)) {
      for (const a of m[1].split(/[;,]/)) {
        if (!a.endsWith('.thtml')) continue;
        const target = path.posix.normalize(path.posix.join(path.posix.dirname(name), a));
        if (!(target in pages)) err(`${site} app page ${name}: navigates to missing ${a}`);
      }
    }
    if (!langs.includes('la') && name.startsWith('la/')) err(`${site}: Latin THTML page ${name} published`);
  }
  if (!langs.includes('la') && files.some((f) => rel(f).startsWith('/la/'))) err(`${site}: /la/ exists in the build`);
  const sm = fs.readFileSync(path.join(root, 'sitemap.xml'), 'utf8');
  if (!langs.includes('la') && /\/la\//.test(sm)) err(`${site}: sitemap lists Latin`);
  console.log(`${site}: ${htmls.length} html files, ${Object.keys(pages).length} OxiUI pages checked`);
}

if (errors.length) {
  console.error(`\n${errors.length} problem(s):`);
  for (const e of errors.slice(0, 80)) console.error('  ' + e);
  process.exit(1);
}
console.log('sites check: ok');
