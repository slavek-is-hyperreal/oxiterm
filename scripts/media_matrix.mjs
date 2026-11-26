#!/usr/bin/env node
// Stage 7.6: headless-browser matrix (page x mode) asserting that every media node shows up.
//
//   node scripts/media_matrix.mjs [--cwd DIR (server working dir holding examples/, default repo root)] [--url http://127.0.0.1:8080] [--bin target/release/oxiterm-cli]
//                                 [--shots DIR] [--wait 8000] [--json]
//
// * With --bin the script starts `<bin> serve examples/index.thtml --local-mode --no-auth` itself
//   (cwd = repo root, random ports) and stops it afterwards; with --url it uses a running server
//   (which must have been started with --local-mode for the `local` column).
// * playwright-core is not a repo dependency. Point PLAYWRIGHT_CORE at a node_modules/playwright-core
//   directory (or install it); without it the script prints SKIP and exits 0.
// * Chromium: PLAYWRIGHT_CHROMIUM (executable) or the Playwright default.
//
// Modes: cell = default web client (cell diffs + DOM media overlay), gui = ?gui=1, local = ?local=1.
// Exit code 1 if any check fails.
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdirSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
const opt = (name, dflt) => { const i = args.indexOf('--' + name); return i >= 0 ? args[i + 1] : dflt; };
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const waitMs = Number(opt('wait', '20000')); // upper bound per page, polled
const shots = opt('shots', null);
const asJson = args.includes('--json');
const only = opt('only', null); // e.g. --only media:cell


let chromium;
try {
  const req = createRequire(import.meta.url);
  const spec = process.env.PLAYWRIGHT_CORE || 'playwright-core';
  ({ chromium } = req(spec));
} catch (e) {
  console.log('SKIP: playwright-core not found (set PLAYWRIGHT_CORE=/path/to/node_modules/playwright-core)');
  process.exit(0);
}

// What each page must show. `media` = DOM overlay elements (cell/gui) or in-page elements (local).
// kind: img (decoded <img>), lottie (animated svg child), cells (widget drawn by the cell/GUI renderer
// into the canvas: no DOM element, checked by canvas pixels inside the node rect).
const PAGES = {
  showcase: { page: 'demos/showcase.thtml', media: [], canvasBusy: true },
  media:    { page: 'demos/media.thtml',    media: ['img', 'img', 'lottie', 'widget'] },
  diagram:  { page: 'demos/diagram.thtml',  media: [], diagram: true },
  // Chromium builds without proprietary codecs (Playwright's) cannot decode H.264: only the element is checked.
  video:    { page: 'demos/video.thtml',    media: ['video'] },
};
const MODES = { cell: '', gui: '&gui=1', local: '&local=1' };

let server = null;
let base = opt('url', null);
if (!base) {
  const bin = path.resolve(root, opt('bin', 'target/release/oxiterm-cli'));
  if (!existsSync(bin)) { console.log('SKIP: no --url and binary not found: ' + bin); process.exit(0); }
  const web = 20000 + Math.floor(Math.random() * 20000), ssh = web + 1;
  server = spawn(bin, ['serve', 'examples/index.thtml', '--local-mode', '--no-auth', '--port', String(ssh), '--web-port', String(web)],
    { cwd: path.resolve(opt('cwd', root)), stdio: ['ignore', 'ignore', 'pipe'] });
  let log = '';
  server.stderr.on('data', (d) => { log += d; });
  server.on('exit', () => { server.exited = true; });
  base = `http://127.0.0.1:${web}`;
  for (let i = 0; i < 50; i++) {
    try { const r = await fetch(base + '/'); if (r.ok) break; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  server.log = () => log;
}

const probe = () => {
  const layer = document.getElementById('media-layer');
  const out = { els: [], canvas: null };
  for (const el of layer ? layer.children : []) {
    if (el.style.display === 'none') continue; // pooled for reuse, not shown
    const r = el.getBoundingClientRect();
    const e = { tag: el.tagName.toLowerCase(), cls: el.className || '', w: Math.round(r.width), h: Math.round(r.height), x: Math.round(r.x), y: Math.round(r.y) };
    if (el.tagName === 'IMG') { e.complete = el.complete; e.nat = el.naturalWidth; }
    if (el.classList.contains('lottie-slot')) {
      e.svg = el.querySelectorAll('svg path, svg g').length;
      e.state = el.dataset.lottie;
      e.frame = el.__anim ? el.__anim.currentFrame : null;
    }
    out.els.push(e);
  }
  const c = document.getElementById('terminal-canvas');
  if (c) {
    const r = c.getBoundingClientRect();
    out.canvas = { x: r.x, y: r.y, w: r.width, h: r.height, cw: c.width, ch: c.height };
  }
  return out;
};

// Pixels of exactly `rgb` on the canvas, and their bounding box (device pixels -> CSS pixels).
const findColour = (rgb) => {
  const c = document.getElementById('terminal-canvas');
  if (!c) return { n: -1 };
  const g = c.getContext('2d');
  if (!g) return { n: -2 };
  const r = c.getBoundingClientRect();
  const k = c.width / r.width;
  const d = g.getImageData(0, 0, c.width, c.height).data;
  let n = 0, x0 = 1e9, y0 = 1e9, x1 = -1, y1 = -1;
  for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
    const i = (y * c.width + x) * 4;
    if (d[i] === rgb[0] && d[i + 1] === rgb[1] && d[i + 2] === rgb[2]) { n++; x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
  }
  return { n, cx: r.x + (x0 + x1) / 2 / k, cy: r.y + (y0 + y1) / 2 / k };
};

// Count distinct colours in a canvas region (device pixels); > a handful means "something is drawn".
const colours = (box) => {
  const c = document.getElementById('terminal-canvas');
  if (!c) return -1;
  const g = c.getContext('2d');
  if (!g) return -2;
  const sx = c.width / c.getBoundingClientRect().width;
  const d = g.getImageData(Math.max(0, Math.round((box.x - c.getBoundingClientRect().x) * sx)), Math.max(0, Math.round((box.y - c.getBoundingClientRect().y) * sx)),
    Math.max(1, Math.round(box.w * sx)), Math.max(1, Math.round(box.h * sx))).data;
  const s = new Set();
  for (let i = 0; i < d.length; i += 4) { s.add((d[i] << 16) | (d[i + 1] << 8) | d[i + 2]); if (s.size > 64) break; }
  return s.size;
};

// Distinct colours inside the diagram box interior, anchored on the header's green title (#1DB954) because the
// page is centred: the 60x14 single-border box starts 3 rows (60 px) below the title's centre.
async function diagramInterior(pg, cv) {
  const t = await pg.evaluate(findColour, [29, 185, 84]);
  if (!(t.n > 50)) return { title: t.n, colours: -1 };
  const box = { x: cv.x + 140, y: t.cy + 85, w: 500, h: 200 };
  return { title: t.n, colours: await pg.evaluate(colours, box) };
}

const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined, args: ['--no-sandbox'] });
const results = [];
const fail = (r, msg) => { r.ok = false; r.problems.push(msg); };

for (const [pname, spec] of Object.entries(PAGES)) {
  for (const [mname, q] of Object.entries(MODES)) {
    if (only && only !== `${pname}:${mname}`) continue;
    const r = { page: pname, mode: mname, ok: true, problems: [], info: {} };
    const ctx = await browser.newContext({ viewport: { width: 1200, height: 800 } });
    const pg = await ctx.newPage();
    const errs = [];
    pg.on('pageerror', (e) => errs.push(String(e)));
    // A CSP only breaks what it is not told about: any "Refused to ..." console line fails the row.
    const csp = [];
    pg.on('console', (m) => { if (/Refused to|Content Security Policy/i.test(m.text())) csp.push(m.text().slice(0, 200)); });
    try {
      const bad = [];
      pg.on('response', (res) => { if (res.status() >= 400 && !res.url().endsWith('favicon.ico')) bad.push(`${res.status()} ${res.url().replace(base, '')}`); });
      await pg.goto(`${base}/?page=${spec.page}${q}`, { waitUntil: 'load' });
      // Poll instead of a fixed sleep (a loaded box connects slowly): ready = every expected DOM
      // overlay exists (and, for cell-drawn nodes, the canvas has them), or the deadline passed.
      const wantEls = spec.media.filter((k) => k !== 'widget').length;
      const t0 = Date.now();
      let p = await pg.evaluate(probe);
      while (Date.now() - t0 < waitMs) {
        const riveOk = !spec.media.includes('widget') || (await pg.evaluate(findColour, [35, 35, 35])).n + (await pg.evaluate(findColour, [70, 70, 70])).n + (await pg.evaluate(findColour, [0, 120, 255])).n > 100;
        const diagOk = !spec.diagram || (await diagramInterior(pg, p.canvas)).colours >= 3;
        if (p.els.length >= wantEls && riveOk && diagOk && Date.now() - t0 > 1500) break;
        await pg.waitForTimeout(400);
        p = await pg.evaluate(probe);
      }
      r.info = p;
      const els = p.els;
      // local mode: browsers draw img/svg directly as overlays too (same #media-layer).
      const want = spec.media;
      const imgs = els.filter((e) => e.tag === 'img');
      const vids = els.filter((e) => e.tag === 'video');
      if (want.includes('video') && vids.length < 1) fail(r, 'no <video> overlay');
      for (const v of vids) if (!(v.w > 0 && v.h > 0)) fail(r, 'video overlay has no size');
      const lot = els.filter((e) => e.cls === 'lottie-slot');
      const wantImgs = want.filter((k) => k === 'img').length;
      if (imgs.length < wantImgs) fail(r, `expected ${wantImgs} <img> overlays, got ${imgs.length}`);
      for (const i of imgs) if (!(i.complete && i.nat > 0 && i.w > 0 && i.h > 0)) fail(r, `img overlay not decoded/sized: ${JSON.stringify(i)}`);
      if (want.includes('lottie')) {
        if (lot.length < 1) fail(r, 'no lottie slot');
        else if (!(lot[0].svg > 0)) fail(r, `lottie slot has no drawn svg (state ${lot[0].state})`);
        else {
          // "Animates" = the player's frame counter advances between two samples.
          const f0 = lot[0].frame;
          await pg.waitForTimeout(700);
          const f1 = (await pg.evaluate(probe)).els.find((e) => e.cls === 'lottie-slot')?.frame;
          r.info.lottieFrames = [f0, f1];
          if (!(f0 != null && f1 != null && f0 !== f1)) fail(r, `lottie is not animating (frame ${f0} -> ${f1})`);
        }
      }
      if (want.includes('widget')) {
        // The Rive procedural widget is drawn into the canvas as cells (no DOM element). Its track is
        // (35,35,35) idle, (70,70,70) hovered, (0,120,255) toggled on. The playback state is
        // process-global on the server, so an earlier session may have left any of them: accept any,
        // click, and require "toggled on" to flip.
        const count = async (c) => (await pg.evaluate(findColour, c));
        const idle = await count([35, 35, 35]), hov = await count([70, 70, 70]), on = await count([0, 120, 255]);
        r.info.riveTrackPixels = { idle: idle.n, hover: hov.n, on: on.n };
        const seen = [idle, hov, on].find((x) => x.n > 100);
        if (!seen) fail(r, `rive widget not drawn (${idle.n}/${hov.n}/${on.n} track pixels)`);
        else {
          await pg.mouse.click(seen.cx, seen.cy);
          await pg.waitForTimeout(800);
          const after = await count([0, 120, 255]);
          if ((on.n > 100) === (after.n > 100)) fail(r, `rive widget did not toggle on click (blue pixels ${on.n} -> ${after.n})`);
        }
      }
      if (spec.diagram) {
        // The page is centred, so anchor on the green title (#1DB954 = 29,185,84) of the header box: the diagram
        // box (60x14, single border) starts 3 rows (60 px) below its centre; its interior is what must not be empty
        // (a missing file, a refused path or a box pushed off-screen leaves it one flat colour).
        const d = await diagramInterior(pg, p.canvas);
        r.info.diagram = d;
        if (d.colours < 0) fail(r, 'diagram page header not found on the canvas');
        else if (!(d.colours >= 3)) fail(r, `diagram box interior is empty (${d.colours} colours)`);
      }
      if (spec.canvasBusy) {
        const cv = p.canvas;
        const n = await pg.evaluate(colours, { x: cv.x, y: cv.y, w: cv.w, h: cv.h });
        if (!(n >= 8)) fail(r, `canvas looks empty (${n} colours)`);
      }
      if (csp.length) fail(r, 'CSP violations: ' + csp.join(' | '));
      if (bad.length) r.info.badResponses = bad;
      if (errs.length) r.info.pageErrors = errs;
      if (shots) {
        mkdirSync(shots, { recursive: true });
        await pg.screenshot({ path: path.join(shots, `${pname}-${mname}.png`) });
      }
    } catch (e) {
      fail(r, 'exception: ' + e.message);
    }
    await ctx.close();
    results.push(r);
  }
}
await browser.close();
if (server) server.kill();

if (asJson) console.log(JSON.stringify(results, null, 1));
console.log('\npage      mode   result');
for (const r of results) console.log(`${r.page.padEnd(9)} ${r.mode.padEnd(6)} ${r.ok ? 'OK' : 'FAIL'}${r.problems.length ? '  ' + r.problems.join('; ') : ''}`);
process.exit(results.every((r) => r.ok) ? 0 : 1);
