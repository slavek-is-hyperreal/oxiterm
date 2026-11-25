#!/usr/bin/env node
// Static site generator for slavekm.pl, oxiui.slavekm.pl and oxiscript.slavekm.pl (slice 8.W).
// No dependencies. Usage: node sites/build.mjs   (called by sites/build.sh, which builds the wasm first)
//
// Inputs
//   sites/i18n/<lang>.json      all strings, one file per language (en = source of truth, pl, la)
//   sites/shared, sites/<site>  css/js/assets
//   docs/oxi-ui.md              roadmap + wave log tables (parsed, never hand-copied)
//   examples/demos/*.thtml      gallery pages;  sites/oxiscript/examples/*.oxs  playground + tour code
// Environment
//   OXI_LOCAL_PKG  dir with oxiterm_local.js / _bg.wasm   (7.1 local-mode bundle)
//   OXI_PLAY_PKG   dir with oxiscript_playground.js / _bg.wasm
//   ORIGIN_HOME / ORIGIN_OXIUI / ORIGIN_OXISCRIPT  override the public origins (preview uses localhost)
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { demoThtml as demoTemplate } from './demo-thtml.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '..');
const DIST = path.join(HERE, 'dist');
const ALL_LANGS = ['en', 'pl', 'la']; // la = Latin: draft, written but NOT published until the owner enables it
// Published languages. Default pl,en. Enable Latin later with SITES_LANGS=pl,en,la. `en` is always built (site root).
const LANGS = ALL_LANGS.filter((l) => l === 'en' || (process.env.SITES_LANGS || 'pl,en').split(',').map((x) => x.trim()).includes(l));
const LATIN_ON = LANGS.includes('la');
// Content flags (off by default). `SITES_FLAGS=home.about` publishes the About block on the home page once the
// owner has written it (blocks carry `"flag": "home.about"` in sites/i18n/*.json).
const FLAGS = new Set((process.env.SITES_FLAGS || '').split(',').map((x) => x.trim()).filter(Boolean));
const blockOn = (b) => !b.flag || FLAGS.has(b.flag);
const REPO_URL = 'https://github.com/slavek-is-hyperreal/oxiterm';
const SUPPORT_URL = 'https://buycoffee.to/smajcher';

const read = (p) => fs.readFileSync(p, 'utf8');
const mkdir = (p) => fs.mkdirSync(p, { recursive: true });
const write = (p, s) => { mkdir(path.dirname(p)); fs.writeFileSync(p, s); };
const copy = (a, b) => { mkdir(path.dirname(b)); fs.copyFileSync(a, b); };

const ORIGIN = {
  home: process.env.ORIGIN_HOME || 'https://slavekm.pl',
  oxiui: process.env.ORIGIN_OXIUI || 'https://oxiui.slavekm.pl',
  oxiscript: process.env.ORIGIN_OXISCRIPT || 'https://oxiscript.slavekm.pl',
};

const SITES = {
  home: { pages: ['index'] },
  oxiui: { pages: ['index', 'gallery', 'how', 'docs', 'roadmap'] },
  oxiscript: { pages: ['index', 'ts', 'tour', 'course', 'roadmap'] },
};
const GALLERY = ['counter', 'state', 'input', 'todo', 'styles', 'showcase'];
const PALETTE = {
  home: { bg: '#1c1917', acc: '#fbbf24', card: '#292524', line: '#57534e' },
  oxiui: { bg: '#0b1220', acc: '#4ade80', card: '#111c30', line: '#1e3a5f' },
  oxiscript: { bg: '#0f1020', acc: '#a78bfa', card: '#1a1b36', line: '#3b3d73' },
};

// ---------------------------------------------------------------- i18n
const I18N = {};
for (const l of ALL_LANGS) I18N[l] = JSON.parse(read(path.join(HERE, 'i18n', `${l}.json`)));
const missing = new Set();
const dig = (o, key) => key.split('.').reduce((a, k) => (a == null ? undefined : a[k]), o);
function T(lang, key) {
  let v = dig(I18N[lang], key);
  if (v === undefined) {
    missing.add(`${lang}:${key}`);
    v = dig(I18N.en, key);
  }
  if (v === undefined) throw new Error(`i18n: key "${key}" missing in en.json`);
  return v;
}
const isDraft = (lang) => I18N[lang].meta.status === 'draft';

// ---------------------------------------------------------------- helpers
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const pagePath = (lang, id) => (lang === 'en' ? '/' : `/${lang}/`) + (id === 'index' ? '' : `${id}/`);

/** Resolve a link target: `@site` / `@site:page` (lang aware), http(s), or a site-absolute path. */
function resolveLink(site, lang, href) {
  if (href.startsWith('@')) {
    const [s, p = 'index'] = href.slice(1).split(':');
    if (!SITES[s]) throw new Error(`bad link target ${href}`);
    if (!SITES[s].pages.includes(p)) throw new Error(`bad link page ${href}`);
    return (s === site ? '' : ORIGIN[s]) + pagePath(lang, p);
  }
  if (href.startsWith('repo:')) return `${REPO_URL}/blob/main/${href.slice(5)}`;
  if (href.startsWith('repotree:')) return `${REPO_URL}/tree/main/${href.slice(9)}`;
  return safeUrl(href);
}
/** Only http, https, mailto and relative targets may reach an href or an open: action. Control characters and
 *  whitespace are stripped before the scheme is tested (browsers ignore them inside "java\tscript:"). */
function safeUrl(u) {
  const probe = String(u).replace(/[\u0000-\u0020\u007f-\u009f]/g, '').toLowerCase();
  const scheme = /^([a-z][a-z0-9+.-]*):/.exec(probe);
  if (scheme && !['http', 'https', 'mailto'].includes(scheme[1])) throw new Error(`link scheme not allowed: ${JSON.stringify(u)}`);
  if (probe.startsWith('//')) throw new Error(`protocol-relative link not allowed: ${JSON.stringify(u)}`);
  return u;
}
const isExternal = (u) => /^https?:\/\//.test(u);

/** Tiny inline markup: [text](target), `code`, **bold**. Everything else is escaped. */
function inline(site, lang, s) {
  let out = esc(s);
  out = out.replace(/`([^`]+)`/g, '<code>$1</code>');
  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, text, href) => {
    const u = resolveLink(site, lang, href.replace(/&amp;/g, '&'));
    const ext = isExternal(u) && !u.startsWith(ORIGIN[site]);
    return `<a href="${esc(u)}"${ext ? ' rel="noopener"' : ''}>${text}</a>`;
  });
  return out;
}
const plain = (s) => s.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1').replace(/[`*]/g, '');

// ---------------------------------------------------------------- roadmap from docs/oxi-ui.md
function parseTables(md) {
  const lines = md.split('\n');
  const tables = [];
  for (let i = 0; i < lines.length; i++) {
    if (!lines[i].startsWith('|') || !/^\|[\s:|-]+\|?\s*$/.test(lines[i + 1] || '')) continue;
    const cells = (l) => l.trim().replace(/^\||\|$/g, '').split(/(?<!\\)\|/).map((c) => c.replace(/\\\|/g, '|').trim());
    const head = cells(lines[i]);
    const rows = [];
    let j = i + 2;
    while (j < lines.length && lines[j].startsWith('|')) rows.push(cells(lines[j++]));
    tables.push({ head, rows, line: i + 1 });
    i = j;
  }
  return tables;
}
function shorten(s, n) {
  const t = plain(s).replace(/\s+/g, ' ');
  if (t.length <= n) return t;
  const cut = t.slice(0, n);
  const dot = cut.lastIndexOf('. ');
  return (dot > n * 0.5 ? cut.slice(0, dot + 1) : cut.replace(/\s+\S*$/, '') + ' ...');
}
function roadmapData() {
  const md = read(path.join(REPO, 'docs/oxi-ui.md'));
  const tables = parseTables(md);
  const phases = [];
  for (const m of md.matchAll(/^### (Phase \d+): (.+)$/gm)) {
    const title = m[2].replace(/\s*—\s*✅ done/, '').replace(/\s*\((planned|draft)[^)]*\)\s*$/, '').trim();
    const state = /✅ done/.test(m[2]) ? 'done' : /planned|draft/.test(m[2]) ? 'planned' : 'active';
    phases.push({ id: m[1], title: plain(title), state });
  }
  const wave = tables.find((t) => t.head[0] === 'Slice' && t.head[1] === 'Implementation');
  const slices = tables.find((t) => t.head[0] === 'Slice' && t.head[1] === 'Crate / area') || tables.find((t) => t.head[0] === 'Slice' && t.head.length >= 4 && t.head[1] !== 'Implementation');
  const p7 = tables.find((t) => t.head[0] === 'Stage' && t.head[1] === 'Scope' && t.rows.some((r) => r[0] === '7.1'));
  if (!wave || !slices) throw new Error('roadmap: tables not found in docs/oxi-ui.md (format changed?)');
  const stateOf = (id) => {
    const w = wave.rows.find((r) => r[0].split(' ')[0] === id);
    if (!w) return 'planned';
    if (/\b[0-9a-f]{7}\b/.test(w[5] || '') || /merged|20\d\d-\d\d-\d\d/.test(w[5] || '')) return /—/.test(w[5]) ? 'active' : 'done';
    return 'active';
  };
  const slice = (r) => ({ id: r[0].split(' ')[0], name: plain(r[0].replace(/^\S+\s*/, '')), what: shorten(r[2] || '', 230), state: stateOf(r[0].split(' ')[0]) });
  // A phase whose heading says "planned" but whose stages/slices have started is in progress.
  const started = (list) => list.some((x) => x.state !== 'planned');
  const slicesList = slices.rows.map(slice);
  const stages7List = p7 ? p7.rows.map((r) => ({ state: stateOf(r[0]) })) : [];
  for (const ph of phases) {
    if (ph.id === 'Phase 8' && ph.state === 'planned' && started(slicesList)) ph.state = 'active';
    if (ph.id === 'Phase 7' && ph.state !== 'done' && started(stages7List)) ph.state = 'active';
  }
  return {
    phases,
    stages7: p7 ? p7.rows.map((r) => ({ id: r[0], name: shorten(r[1].replace(/\*\*([^*]+)\*\*/, '$1'), 150), state: stateOf(r[0]) })) : [],
    slices: slices.rows.map(slice),
    wave: wave.head.length ? { head: wave.head.map(plain), rows: wave.rows.map((r) => r.map((c) => shorten(c, 170))) } : null,
  };
}
const ROADMAP = roadmapData();
const stateLabel = (lang, s) => T(lang, `common.state.${s}`);

// ---------------------------------------------------------------- code blocks
const readExample = (name) => read(path.join(HERE, 'oxiscript/examples', `${name}.oxs`));
const readTs = (name) => read(path.join(HERE, 'oxiscript/ts-compare', `${name}.ts`));
function demoSource(id) {
  return stripBack(read(path.join(REPO, 'examples/demos', `${id}.thtml`)));
}
/** Gallery pages are served standalone: drop the "< Back" link that points at the examples index. */
function stripBack(src) {
  return src.replace(/[ \t]*<text[^>]*event-htmx="\.\.\/features\.thtml"[^>]*>&lt; Back<\/text>\n?/g, '');
}
const pre = (code, lang) => `<pre class="code" data-lang="${lang}" tabindex="0"><code>${esc(code.replace(/\n+$/, ''))}</code></pre>`;

// ---------------------------------------------------------------- HTML blocks
const LATIN_WIDGETS = new Set(['dedication', 'latinnote']); // hidden while Latin is unpublished
function renderBlock(site, lang, b, ctx) {
  const I = (s) => inline(site, lang, s);
  switch (b.t) {
    case 'h2': { const id = b.id ? ` id="${esc(b.id)}"` : ''; return `<h2${id}>${I(b.x)}</h2>`; }
    case 'h3': return `<h3>${I(b.x)}</h3>`;
    case 'p': return `<p>${I(b.x)}</p>`;
    case 'note': return `<p class="note">${I(b.x)}</p>`;
    case 'ul': return `<ul>${b.items.map((i) => `<li>${I(i)}</li>`).join('')}</ul>`;
    case 'ol': return `<ol>${b.items.map((i) => `<li>${I(i)}</li>`).join('')}</ol>`;
    case 'table':
      return `<div class="tablewrap"><table><thead><tr>${b.head.map((h) => `<th scope="col">${I(h)}</th>`).join('')}</tr></thead><tbody>${b.rows.map((r) => `<tr>${r.map((c) => `<td>${I(c)}</td>`).join('')}</tr>`).join('')}</tbody></table></div>`;
    case 'code': return pre(b.oxs ? readExample(b.oxs) : b.src, b.lang || (b.oxs ? 'oxs' : 'text'));
    case 'compare':
      return `<div class="compare"><figure><figcaption>TypeScript</figcaption>${pre(readTs(b.cmp), 'ts')}</figure><figure><figcaption>OxiScript</figcaption>${pre(readExample(b.cmp), 'oxs')}</figure></div>`;
    case 'links':
      return `<p class="links">${b.items.map((i) => { const u = resolveLink(site, lang, i.href); return `<a class="btn${i.primary ? ' primary' : ''}" href="${esc(u)}"${isExternal(u) ? ' rel="noopener"' : ''}>${esc(i.label)}</a>`; }).join(' ')}</p>`;
    case 'widget': return LATIN_WIDGETS.has(b.id) && !LATIN_ON ? '' : widget(site, lang, b.id, b, ctx);
    default: throw new Error(`unknown block type ${b.t}`);
  }
}

const appUrl = (site, lang, thtml) => `/app/?local=1&amp;page=${encodeURIComponent(thtml)}`.replace(/%2F/g, '/');

function iframeStarter(site, lang, thtml, titleKey) {
  return `<div class="demo" data-demo><div class="demo-poster"><p>${esc(T(lang, 'common.demoHint'))}</p><button type="button" class="btn primary" data-iframe-src="${appUrl(site, lang, thtml)}" data-iframe-title="${esc(T(lang, titleKey))}">${esc(T(lang, 'common.runDemo'))}</button><p class="note">${esc(T(lang, 'common.demoWeight'))}</p></div></div>`;
}

function roadmapHtml(site, lang) {
  const R = ROADMAP;
  const badge = (s) => `<span class="badge s-${s}">${esc(stateLabel(lang, s))}</span>`;
  let h = `<p class="note">${esc(T(lang, 'common.roadmapSource'))}${lang === 'en' ? '' : ' ' + esc(T(lang, 'common.roadmapEnglishOnly'))}</p>`;
  h += `<h2 class="sub">${esc(T(lang, 'common.phases'))}</h2><ul class="phases">${R.phases.map((p) => `<li>${badge(p.state)} <strong>${esc(p.id)}</strong> ${esc(p.title)}</li>`).join('')}</ul>`;
  h += `<h2 class="sub">${esc(T(lang, 'common.stages7'))}</h2><ul class="phases">${R.stages7.map((p) => `<li>${badge(p.state)} <strong>${esc(p.id)}</strong> ${esc(p.name)}</li>`).join('')}</ul>`;
  h += `<h2 class="sub">${esc(T(lang, 'common.slices8'))}</h2><div class="tablewrap"><table><thead><tr><th scope="col">${esc(T(lang, 'common.colSlice'))}</th><th scope="col">${esc(T(lang, 'common.colStatus'))}</th><th scope="col">${esc(T(lang, 'common.colWhat'))}</th></tr></thead><tbody>${R.slices.map((s) => `<tr><td><strong>${esc(s.id)}</strong> ${esc(s.name)}</td><td>${badge(s.state)}</td><td>${esc(s.what)}</td></tr>`).join('')}</tbody></table></div>`;
  h += `<h2 class="sub" id="wave-log">${esc(T(lang, 'common.waveLog'))}</h2><p>${esc(T(lang, 'common.waveLogIntro'))}</p>`;
  h += `<div class="tablewrap"><table><thead><tr>${R.wave.head.map((c) => `<th scope="col">${esc(c)}</th>`).join('')}</tr></thead><tbody>${R.wave.rows.map((r) => `<tr>${r.map((c) => `<td>${esc(c)}</td>`).join('')}</tr>`).join('')}</tbody></table></div>`;
  h += `<p class="note"><a href="${REPO_URL}/blob/main/docs/oxi-ui.md" rel="noopener">docs/oxi-ui.md</a></p>`;
  return h;
}

function widget(site, lang, id, b, ctx) {
  switch (id) {
    case 'demo': return iframeStarter(site, lang, `${lang}/demo.thtml`, 'common.demoTitle');
    case 'gallery':
      return GALLERY.map((g) => `<article class="card gallery-item" id="ex-${g}"><h3>${esc(T(lang, `oxiui.gallery.${g}.title`))}</h3><p>${esc(T(lang, `oxiui.gallery.${g}.desc`))}</p>${iframeStarter(site, lang, `demos/${g}.thtml`, `oxiui.gallery.${g}.title`)}<details><summary>${esc(T(lang, 'common.showSource'))}</summary>${pre(demoSource(g), 'thtml')}</details></article>`).join('');
    case 'roadmap': return roadmapHtml(site, lang);
    case 'projectcards':
      return `<div class="cards">${['oxiui', 'oxiscript'].map((s) => `<article class="card project project-${s}"><h3><a href="${ORIGIN[s]}${pagePath(lang, 'index')}">${esc(T(lang, `common.sites.${s}`))}</a></h3><p>${esc(T(lang, `home.cards.${s}.text`))}</p><p class="links"><a class="btn primary" href="${ORIGIN[s]}${pagePath(lang, 'index')}">${esc(T(lang, `home.cards.${s}.cta`))}</a></p></article>`).join('')}<article class="card project project-next"><h3>${esc(T(lang, 'home.cards.next.title'))}</h3><p>${esc(T(lang, 'home.cards.next.text'))}</p></article></div>`;
    case 'latinnote': return `<p class="note latin-note">${esc(T(lang, 'home.latinNote'))}</p>`;
    case 'dedication':
      return `<aside class="dedication" lang="la" aria-labelledby="ded-h"><h2 id="ded-h" lang="${lang}">${esc(T(lang, 'home.dedication.title'))}</h2><p class="ded-text">${esc(T(lang, 'home.dedication.text'))}</p><p class="ded-name">${esc(T(lang, 'home.dedication.name'))}</p><p class="ded-note" lang="${lang}">${esc(T(lang, 'home.dedication.note'))}</p></aside>`;
    case 'playground': return playgroundHtml(site, lang, ctx);
    case 'course':
      return `<ol class="levels">${T(lang, 'oxiscript.course.levels').map((l) => `<li><strong>${esc(l.name)}</strong> ${esc(l.text)}</li>`).join('')}</ol>`;
    default: throw new Error(`unknown widget ${id}`);
  }
}

function playgroundHtml(site, lang, ctx) {
  const labels = {};
  for (const k of ['check', 'format', 'run', 'runSoon', 'examples', 'apply', 'explain', 'noProblems', 'problems', 'loading', 'failed', 'close', 'line', 'formatFailed', 'formatted', 'sourceLabel', 'warning', 'error', 'fixes', 'staleFix', 'fixFailed', 'tooLargeTitle', 'tooLarge', 'discardAsk', 'discard', 'keepEditing']) labels[k] = T(lang, `oxiscript.playground.${k}`);
  const compilerEnglish = T(lang, 'oxiscript.playground.compilerEnglish'); // empty on the English page
  const examples = fs.readdirSync(path.join(HERE, 'oxiscript/examples')).filter((f) => f.endsWith('.oxs')).sort().map((f) => ({ id: f.replace(/\.oxs$/, ''), src: read(path.join(HERE, 'oxiscript/examples', f)) }));
  const json = (o) => JSON.stringify(o).replace(/</g, '\\u003c');
  return `<section class="pg" id="playground" data-wasm="${ctx.playJs}" aria-labelledby="pg-h">
<h2 id="pg-h">${esc(T(lang, 'oxiscript.playground.title'))}</h2>
<div class="pg-bar">
<label>${esc(labels.examples)} <select id="pg-ex"></select></label>
<button type="button" class="btn" id="pg-check" disabled>${esc(labels.check)}</button>
<button type="button" class="btn" id="pg-format" disabled>${esc(labels.format)}</button>
<button type="button" class="btn" id="pg-run" disabled aria-describedby="pg-run-note">${esc(labels.run)}</button>
<span class="note" id="pg-run-note">${esc(labels.runSoon)}</span>
</div>
<div class="pg-editor">
<pre class="pg-gutter" id="pg-gutter" aria-hidden="true">1</pre>
<div class="pg-pane"><div class="pg-mirror" id="pg-mirror" aria-hidden="true"></div>
<textarea id="pg-src" spellcheck="false" autocapitalize="off" autocomplete="off" wrap="off" maxlength="200000" aria-label="${esc(labels.sourceLabel)}"></textarea></div>
</div>
<p class="pg-confirm" id="pg-confirm" role="alert" hidden></p>
<p class="pg-status" id="pg-status" role="status">${esc(labels.loading)}</p>
${compilerEnglish ? `<p class="note">${esc(compilerEnglish)}</p>` : ''}
<ul class="pg-diags" id="pg-diags"></ul>
<div class="pg-explain" id="pg-explain" hidden></div>
<script type="application/json" id="pg-labels">${json(labels)}</script>
<script type="application/json" id="pg-examples">${json(examples)}</script>
</section>`;
}

// ---------------------------------------------------------------- page chrome
const faviconSvg = (acc, glyph) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="6" fill="#111"/><text x="16" y="23" font-size="20" font-family="monospace" font-weight="700" text-anchor="middle" fill="${acc}">${glyph}</text></svg>`)}`;
const GLYPH = { home: 's', oxiui: '>_', oxiscript: '{}' };

function layout(site, lang, id, page, ctx) {
  const t = (k) => T(lang, k);
  const urlFor = (l) => ORIGIN[site] + pagePath(l, id);
  const nav = SITES[site].pages.length > 1
    ? SITES[site].pages.map((p) => `<a href="${pagePath(lang, p)}"${p === id ? ' aria-current="page"' : ''}>${esc(t(`${site}.nav.${p}`))}</a>`).join('')
    : '';
  const projects = ['home', 'oxiui', 'oxiscript'].map((s) => `<a href="${ORIGIN[s]}${pagePath(lang, 'index')}"${s === site ? ' aria-current="true"' : ''}>${esc(t(`common.sites.${s}`))}</a>`).join('');
  const langs = LANGS.map((l) => `<a href="${pagePath(l, id)}" hreflang="${I18N[l].meta.htmlLang}" lang="${I18N[l].meta.htmlLang}"${l === lang ? ' aria-current="true"' : ''} title="${esc(I18N[l].meta.name)}">${esc(I18N[l].meta.short)}</a>`).join('');
  const hreflangs = LANGS.map((l) => `<link rel="alternate" hreflang="${I18N[l].meta.htmlLang}" href="${urlFor(l)}">`).join('') + `<link rel="alternate" hreflang="x-default" href="${urlFor('en')}">`;
  const draft = isDraft(lang) ? `<p class="draft" role="note">${esc(t('common.draft'))}</p>` : '';
  const hero = page.hero ? `<header class="hero"><p class="eyebrow">${esc(t(`common.sites.${site}`))}</p><h1>${inline(site, lang, page.hero.title)}</h1><p class="lead">${inline(site, lang, page.hero.lead)}</p></header>` : `<h1>${esc(page.h1 || page.title)}</h1>${page.lead ? `<p class="lead">${inline(site, lang, page.lead)}</p>` : ''}`;
  const body = (page.blocks || []).filter(blockOn).map((b) => renderBlock(site, lang, b, ctx)).join('\n');
  const viewLink = `<a class="viewlink" href="/app/?local=1&amp;page=${lang}/${id}.thtml" title="${esc(t('common.viewInOxiuiTitle'))}">${esc(t('common.viewInOxiui'))}</a>`;
  const title = id === 'index' ? `${t(`common.sites.${site}`)}: ${page.title}` : `${page.title} · ${t(`common.sites.${site}`)}`;
  const pal = PALETTE[site];
  return `<!doctype html>
<html lang="${I18N[lang].meta.htmlLang}" data-site="${site}">
<head>
<meta charset="utf-8">
${ctx.is404 ? META_CSP + '\n' : ''}<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)}</title>
<meta name="description" content="${esc(page.desc || t(`${site}.pages.index.desc`))}">
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="${pal.bg}">
<link rel="canonical" href="${urlFor(lang)}">
${hreflangs}
<link rel="icon" href="${faviconSvg(pal.acc, GLYPH[site])}">
<link rel="stylesheet" href="/assets/${ctx.cssHash}/site.css">
<meta property="og:title" content="${esc(title)}"><meta property="og:type" content="website"><meta property="og:url" content="${urlFor(lang)}">
</head>
<body>
<a class="skip" href="#main">${esc(t('common.skip'))}</a>
<header class="top">
<nav class="bar projects" aria-label="${esc(t('common.projects'))}"><div class="wrap"><span class="bar-label">${esc(t('common.projects'))}:</span> ${projects}<span class="spacer"></span><nav class="lang" aria-label="${esc(t('common.language'))}">${langs}</nav></div></nav>
<div class="bar"><div class="wrap"><a class="brand" href="${pagePath(lang, 'index')}">${esc(t(`common.sites.${site}`))}</a><nav class="main" aria-label="${esc(t('common.menu'))}">${nav}</nav></div></div>
</header>
<main id="main" class="wrap">
${draft}
${hero}
${body}
</main>
<footer class="foot"><div class="wrap">
<div class="foot-grid">
<section><h2>${esc(t('common.honestTitle'))}</h2><p>${inline(site, lang, t('common.honest'))}</p></section>
<section><h2>${esc(t('common.support'))}</h2><p>${esc(t('common.supportText'))}</p><p><a class="btn primary" href="${SUPPORT_URL}" rel="noopener">${esc(t('common.supportButton'))}</a></p></section>
<section><h2>${esc(t('common.projects'))}</h2><p class="foot-links">${projects}<a href="${REPO_URL}" rel="noopener">${esc(t('common.repo'))}</a></p><p>${viewLink}</p></section>
</div>
<p class="fine">${esc(t('common.footerFine'))}</p>
</div></footer>
<script src="/assets/${ctx.jsHash}/site.js" defer></script>
${ctx.extraScript || ''}
</body>
</html>
`;
}

// ---------------------------------------------------------------- THTML (view this page in OxiUI)
const tx = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

function thtmlPage(site, lang, id, page, mobile) {
  const W = mobile ? 36 : 100;
  const pal = PALETTE[site];
  const t = (k) => T(lang, k);
  const out = [];
  const textL = (cls, s) => out.push(`  <text class="${cls}">${tx(s)}</text>`);
  const btn = (label, target) => out.push(`  <box class="btn" event-htmx="${tx(target)}"><text style="height: 1;">${tx(label)}</text></box>`);
  const navRow = SITES[site].pages.length > 1
    ? SITES[site].pages.map((p) => `<text class="navi${p === id ? ' cur' : ''}" event-htmx="${p}.thtml"> ${tx(t(`${site}.nav.${p}`))} </text>`).join('')
    : '';
  out.push(`<style>
text { fg: #e2e8f0; }
.brand { fg: ${pal.acc}; height: 1; margin-right: 3; }
.h1 { fg: ${pal.acc}; wrap: word; margin-top: 1; }
.h2 { fg: ${pal.acc}; wrap: word; margin-top: 1; }
.p { fg: #e2e8f0; wrap: word; margin-top: 1; }
.dim { fg: #94a3b8; wrap: word; margin-top: 1; }
.navi { fg: #94a3b8; height: 1; margin-right: 2; }
.cur { fg: ${pal.acc}; }
.card { border-style: rounded; border-color: ${pal.line}; padding-left: 1; padding-right: 1; flex-direction: column; margin-top: 1; }
.btn { border-style: single; border-color: ${pal.acc}; fg: ${pal.acc}; padding-left: 1; padding-right: 1; height: 3; align-items: center; margin-top: 1; }
.code { fg: #86efac; height: 1; }
@media (render: gui) { .page-sans { font-family: sans-serif; } }
.mono { font-family: monospace; }
</style>
<box class="page-sans" style="flex-direction: column; width: ${W}; bg: ${pal.bg}; padding: 1;">`);
  out.push(`  <box style="flex-direction: ${mobile ? 'column' : 'row'};${mobile ? '' : ' height: 1;'}"><text class="brand">${tx(t(`common.sites.${site}`))}  </text>${navRow}</box>`);
  const hero = page.hero;
  textL('h1', hero ? hero.title : page.h1 || page.title);
  if (hero) textL('p', plain(hero.lead));
  else if (page.lead) textL('p', plain(page.lead));
  if (isDraft(lang)) textL('dim', t('common.draft'));
  const codeLines = (src) => {
    out.push(`  <box class="card mono">`);
    for (const l of src.replace(/\n+$/, '').split('\n').slice(0, 24)) {
      const line = l.replace(/\t/g, '  ');
      out.push(`    <text class="code mono">${tx(line.length > W - 8 ? line.slice(0, W - 9) + '~' : line || ' ')}</text>`);
    }
    out.push('  </box>');
  };
  for (const b of (page.blocks || []).filter(blockOn)) {
    switch (b.t) {
      case 'h2': case 'h3': textL('h2', plain(b.x)); break;
      case 'p': case 'note': textL('p', plain(b.x)); break;
      case 'ul': case 'ol': b.items.forEach((i, n) => textL('p', `${b.t === 'ol' ? n + 1 + '.' : '-'} ${plain(i)}`)); break;
      case 'table':
        b.rows.forEach((r) => textL('p', r.map(plain).join(' | ')));
        break;
      case 'code': codeLines(b.oxs ? readExample(b.oxs) : b.src); break;
      case 'compare': codeLines(readTs(b.cmp)); codeLines(readExample(b.cmp)); break;
      case 'links':
        for (const i of b.items) {
          const u = resolveLink(site, lang, i.href);
          btn(i.label, u.startsWith('/') ? `open:${ORIGIN[site]}${u}` : `open:${u}`);
        }
        break;
      case 'widget': if (!(LATIN_WIDGETS.has(b.id) && !LATIN_ON)) thtmlWidget(b.id); break;
    }
  }
  function thtmlWidget(wid) {
    switch (wid) {
      case 'demo': btn(t('common.runDemo'), 'demo.thtml'); break;
      case 'gallery': for (const g of GALLERY) { textL('h2', t(`oxiui.gallery.${g}.title`)); textL('dim', t(`oxiui.gallery.${g}.desc`)); btn(t('common.runDemo'), `../demos/${g}.thtml`); } break;
      case 'roadmap':
        textL('dim', t('common.roadmapSource'));
        textL('h2', t('common.slices8'));
        for (const s of ROADMAP.slices) textL('p', `[${stateLabel(lang, s.state)}] ${s.id} ${s.name}`);
        textL('h2', t('common.waveLog'));
        for (const r of ROADMAP.wave.rows) textL('p', `${r[0]}: ${r[5]}`);
        break;
      case 'projectcards':
        for (const s of ['oxiui', 'oxiscript']) { textL('h2', t(`common.sites.${s}`)); textL('p', t(`home.cards.${s}.text`)); btn(t(`home.cards.${s}.cta`), `open:${ORIGIN[s]}${pagePath(lang, 'index')}`); }
        break;
      case 'latinnote': textL('dim', t('home.latinNote')); break;
      case 'dedication':
        textL('h2', t('home.dedication.title')); textL('p', t('home.dedication.text')); textL('p', t('home.dedication.name')); textL('dim', t('home.dedication.note'));
        break;
      case 'playground': textL('p', t('oxiscript.playground.thtmlNote')); btn(t('common.backToHtml'), `open:${ORIGIN[site]}${pagePath(lang, id)}`); break;
      case 'course': T(lang, 'oxiscript.course.levels').forEach((l) => textL('p', `${l.name}: ${l.text}`)); break;
    }
  }
  btn(t('common.backToHtml'), `open:${ORIGIN[site]}${pagePath(lang, id)}`);
  btn(t('common.supportButton'), `open:${SUPPORT_URL}`);
  textL('dim', t('common.honest').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1'));
  out.push('</box>');
  return out.join('\n') + '\n';
}

// ---------------------------------------------------------------- app (local-mode client) + assets
function patchClient() {
  let h = read(path.join(REPO, 'oxiterm-server/assets/index.html'));
  const must = (from, to) => { if (!h.includes(from)) throw new Error(`app client: pattern not found: ${from}`); h = h.split(from).join(to); };
  must("const wantLocal = pageParams.get('local') === '1';", 'const wantLocal = true; // static site: local mode only, no WebSocket fallback');
  must("'/oxiui/local/pages.json'", "'./pages.json'");
  must("'/oxiui/local/asset/'", "'./asset/'");
  must('/*__OXITERM_WEB_TEST_HOOK__*/', '');
  h = h.replace('<title>OxiTerm Console</title>', '<title>OxiUI</title><link rel="icon" href="data:,">').replace(/OxiTerm Console/g, 'OxiUI');
  // No third-party requests: the Lottie CDN script is only needed for Lottie media, which no site page uses.
  h = h.replace(/\s*<script src="https:\/\/cdnjs[^>]*><\/script>/g, '');
  if (/https?:\/\/(?!localhost)[^"' )]*\.(js|css)/.test(h.replace(/<script type="module">[\s\S]*$/, ''))) throw new Error('app client: external resource remains');
  // Fonts come from the embedded faces in local mode; drop the third-party font request (no trackers).
  h = h.replace(/\s*<link rel="preconnect"[^>]*>/g, '').replace(/\s*<link href="https:\/\/fonts\.googleapis\.com[^>]*>/g, '');
  // F7: the static demo has no App Server, so there is no app token to keep. Every Web Storage access of the copied
  // client goes to an inert object: nothing is read from or written to the visitor's browser.
  h = h.replace(/\b(?:local|session)Storage\b/g, 'noStore'); // also in comments, so the output never names Web Storage
  must('<script type="module">\n', '<script type="module">\n        const noStore = { getItem() { return null; }, setItem() {}, removeItem() {} }; // static site: nothing is stored\n');
  // 0x33 open-URL: http(s) and mailto only (generated `open:` links never need anything else)
  must("window.open(url, '_blank', 'noopener');", "if (/^(https?:|mailto:)/i.test(url)) window.open(url, '_blank', 'noopener');");
  // F6: no inline script or style, so the strict CSP applies to /app/ too. The caller writes both files.
  const styleM = /<style>([\s\S]*?)<\/style>/.exec(h);
  const scriptM = /<script type="module">([\s\S]*?)<\/script>/.exec(h);
  if (!styleM || !scriptM) throw new Error('app client: inline <style>/<script type="module"> not found');
  return { html: h, css: styleM[1], js: scriptM[1], styleTag: styleM[0], scriptTag: scriptM[0] };
}
const hashOf = (buf) => crypto.createHash('sha256').update(buf).digest('hex').slice(0, 10);

const demoThtml = (site, lang, mobile) => demoTemplate({ site, mobile, t: (k) => T(lang, k), pal: PALETTE[site], tx });
function buildPagesJson(site, extraPages) {
  const pages = {};
  const dirs = new Set(['demos', ...LANGS]);
  for (const lang of LANGS) {
    for (const id of SITES[site].pages) {
      const page = T(lang, `${site}.pages.${id}`);
      pages[`${lang}/${id}.thtml`] = thtmlPage(site, lang, id, page, false);
      pages[`${lang}/${id}_mobile.thtml`] = thtmlPage(site, lang, id, page, true);
    }
    if (site === 'oxiui') {
      pages[`${lang}/demo.thtml`] = demoThtml(site, lang, false);
      pages[`${lang}/demo_mobile.thtml`] = demoThtml(site, lang, true);
    }
  }
  Object.assign(pages, extraPages);
  return JSON.stringify({ start: `en/${site === 'oxiui' ? 'demo' : 'index'}.thtml`, actions: false, truncated: false, dirs: [...dirs], pages });
}

function copyBundle(pkgDir, outDir, names) {
  const files = {};
  for (const n of names) {
    if (!fs.existsSync(path.join(pkgDir, n))) throw new Error(`missing ${path.join(pkgDir, n)}`);
    files[n] = fs.readFileSync(path.join(pkgDir, n));
  }
  const wasm = names.find((n) => n.endsWith('.wasm'));
  const hash = hashOf(files[wasm]);
  for (const n of names) write(path.join(outDir, hash, n), files[n]);
  return hash;
}

function buildSite(site) {
  const out = path.join(DIST, site);
  fs.rmSync(out, { recursive: true, force: true });
  mkdir(out);
  const ctx = {};
  // shared assets, hashed names -> immutable caching
  const css = read(path.join(HERE, 'shared/site.css')) + '\n' + read(path.join(HERE, 'shared/site.css.d', `${site}.css`));
  const js = read(path.join(HERE, 'shared/site.js'));
  ctx.cssHash = hashOf(css); ctx.jsHash = hashOf(js);
  write(path.join(out, 'assets', ctx.cssHash, 'site.css'), css);
  write(path.join(out, 'assets', ctx.jsHash, 'site.js'), js);

  // OxiUI local-mode app (every site can show "view this page in OxiUI")
  const localPkg = process.env.OXI_LOCAL_PKG || path.join(HERE, '.build/pkg_local');
  if (localPkg) {
    const hash = copyBundle(localPkg, path.join(out, 'app'), ['oxiterm_local.js', 'oxiterm_local_bg.wasm']);
    const pc = patchClient();
    // the client script lives in /app/<content hash>/ and imports the wasm bundle from its own hashed directory
    if (!pc.js.includes("'./oxiterm_local.js'")) throw new Error('app client: bundle import not found');
    const cjs = pc.js.replace("'./oxiterm_local.js'", `'../${hash}/oxiterm_local.js'`);
    const ch = hashOf(cjs + pc.css);
    write(path.join(out, 'app', ch, 'client.js'), cjs);
    write(path.join(out, 'app', ch, 'client.css'), pc.css);
    const client = pc.html.replace(pc.styleTag, () => `<link rel="stylesheet" href="./${ch}/client.css">`).replace(pc.scriptTag, () => `<script type="module" src="./${ch}/client.js"></script>`);
    write(path.join(out, 'app/index.html'), client);
    const extra = {};
    for (const g of GALLERY) {
      extra[`demos/${g}.thtml`] = demoSource(g);
      const m = path.join(REPO, 'examples/demos', `${g}_mobile.thtml`);
      if (fs.existsSync(m)) extra[`demos/${g}_mobile.thtml`] = stripBack(read(m));
    }
    write(path.join(out, 'app/pages.json'), buildPagesJson(site, extra));
    for (const f of ['NotoSans-OFL.txt', 'NotoSansMono-OFL.txt']) copy(path.join(REPO, 'oxiterm-renderer/assets/fonts', f), path.join(out, 'app/licenses', f));
  }
  if (site === 'oxiscript') {
    const playPkg = process.env.OXI_PLAY_PKG || path.join(HERE, '.build/pkg_play');
    if (!playPkg) throw new Error('OXI_PLAY_PKG not set');
    const hash = copyBundle(playPkg, path.join(out, 'play'), ['oxiscript_playground.js', 'oxiscript_playground_bg.wasm']);
    ctx.playJs = `/play/${hash}/oxiscript_playground.js`;
    const pjs = read(path.join(HERE, 'oxiscript/js/playground.js'));
    const ph = hashOf(pjs);
    write(path.join(out, 'assets', ph, 'playground.js'), pjs);
    ctx.extraScript = `<script type="module" src="/assets/${ph}/playground.js"></script>`;
  }

  // pages
  const urls = [];
  for (const lang of LANGS) {
    for (const id of SITES[site].pages) {
      const page = T(lang, `${site}.pages.${id}`);
      const pctx = { ...ctx };
      if (!(site === 'oxiscript' && id === 'index')) delete pctx.extraScript;
      const dir = path.join(out, pagePath(lang, id));
      write(path.join(dir, 'index.html'), layout(site, lang, id, page, pctx));
      urls.push(ORIGIN[site] + pagePath(lang, id));
    }
  }
  write(path.join(out, '404.html'), layout(site, 'en', 'index', { title: T('en', 'common.notFound.title'), h1: T('en', 'common.notFound.title'), lead: LANGS.map((l) => T(l, 'common.notFound.lead')).join(' / '), blocks: [{ t: 'links', items: [{ label: LANGS.map((l) => T(l, 'common.notFound.home')).join(' / '), href: '@' + site, primary: true }] }] }, { ...ctx, extraScript: '', is404: true }));
  write(path.join(out, 'robots.txt'), `User-agent: *\nAllow: /\nSitemap: ${ORIGIN[site]}/sitemap.xml\n`);
  write(path.join(out, 'sitemap.xml'), `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.map((u) => `<url><loc>${u}</loc></url>`).join('\n')}\n</urlset>\n`);
  write(path.join(out, '_headers'), headers([...LANGS.flatMap((lang) => SITES[site].pages.map((id) => ({ p: pagePath(lang, id), wasm: site === 'oxiscript' && id === 'index' }))), { p: '/404.html', wasm: false }]));
  return out;
}

// Content-Security-Policy (F6). Cloudflare Pages merges the headers of EVERY matching `_headers` rule (two CSPs would
// both be enforced = their intersection), so the policies are attached to explicit, NON-overlapping paths: one rule
// per HTML page and one for /app/*. sites/check.mjs verifies that no path gets two different CSPs.
const CSP_BASE = ["default-src 'self'", "img-src 'self' data:", "font-src 'self'", "connect-src 'self'", "frame-src 'self'", "frame-ancestors 'self'", "object-src 'none'", "base-uri 'none'", "form-action 'none'"];
const cspPage = (wasm, meta = false) => [...CSP_BASE.filter((d) => !(meta && d.startsWith('frame-ancestors'))), `script-src 'self'${wasm ? " 'wasm-unsafe-eval'" : ''}`, "style-src 'self'"].join('; ');
// A real 404 is matched by Cloudflare against the REQUESTED path (which has no per-page rule), so 404.html carries its
// policy in a <meta> too (frame-ancestors is not allowed in a meta policy and is simply left out there).
const META_CSP = `<meta http-equiv="Content-Security-Policy" content="${cspPage(false, true)}">`;
// /app/* (the copied 7.1 client, now with its script and style in files): wasm for the OxiUI core, blob: for the
// media layer / canvas helpers. Still no inline script or style, no remote origins.
const cspApp = () => [...CSP_BASE.map((d) => (d.startsWith('img-src') ? "img-src 'self' data: blob:" : d)), "script-src 'self' 'wasm-unsafe-eval'", "style-src 'self'", "media-src 'self' blob:", "worker-src 'self' blob:"].join('; ');

function headers(htmlPaths) {
  const pageRules = htmlPaths.map(({ p, wasm }) => `${p}\n  Content-Security-Policy: ${cspPage(wasm)}\n`).join('\n');
  return `# Cloudflare Pages headers (see sites/DEPLOY.md). Rules MERGE: do not add a second Content-Security-Policy to a path.
/*
  X-Content-Type-Options: nosniff
  Referrer-Policy: strict-origin-when-cross-origin
  Permissions-Policy: accelerometer=(), camera=(), display-capture=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()

# One CSP per HTML page (strict: no inline script or style, own origin only; 'wasm-unsafe-eval' only on the playground page)
${pageRules}
/app/
  Content-Security-Policy: ${cspApp()}

/app/*
  Content-Security-Policy: ${cspApp()}

/assets/*
  Cache-Control: public, max-age=31536000, immutable

/app/*/*
  Cache-Control: public, max-age=31536000, immutable

/play/*
  Cache-Control: public, max-age=31536000, immutable

/app/*.wasm
  Content-Type: application/wasm

/play/*.wasm
  Content-Type: application/wasm

/app/pages.json
  Cache-Control: public, max-age=300, must-revalidate
`;
}

for (const s of Object.keys(SITES)) {
  const out = buildSite(s);
  console.log(`built ${s} -> ${path.relative(REPO, out)}`);
}
if (missing.size) {
  const by = {};
  for (const m of missing) { const [l] = m.split(':'); (by[l] ||= []).push(m.slice(l.length + 1)); }
  for (const [l, ks] of Object.entries(by)) console.warn(`i18n: ${l} is missing ${ks.length} key(s) (English shown): ${ks.slice(0, 8).join(', ')}${ks.length > 8 ? ', ...' : ''}`);
}
