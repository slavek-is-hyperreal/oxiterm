// BAD ACTOR 7.6 F-11: which page does the server announce (0x30) for /?page=demos/media.thtml in the real client?
// usage: PLAYWRIGHT_CORE=... node scripts/bad_actor_76_deeplink.mjs http://127.0.0.1:8100 [pagepath]
import { createRequire } from 'node:module';
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_CORE || 'playwright-core');
const base = process.argv[2], pagePath = process.argv[3] || 'demos/media.thtml';
const b = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined, args: ['--no-sandbox'] });
const pg = await b.newPage({ viewport: { width: 1200, height: 800 } });
const wsUrls = [], announces = [];
pg.on('websocket', (ws) => {
  wsUrls.push(ws.url());
  ws.on('framereceived', (f) => { const d = f.payload; if (typeof d !== 'string' && d.length && d[0] === 0x30) announces.push(d.subarray(1).toString()); });
  ws.on('framesent', (f) => { const d = f.payload; if (typeof d !== 'string' && d.length && d[0] === 0x31) announces.push('(client sent 0x31 nav: ' + d.subarray(1).toString() + ')'); });
});
await pg.goto(`${base}/?page=${pagePath}`, { waitUntil: 'load' });
await pg.waitForTimeout(4000);
console.log(JSON.stringify({ wsUrls, announces, finalUrl: pg.url(), mediaChildren: await pg.evaluate(() => document.getElementById('media-layer').children.length) }, null, 1));
await b.close();
