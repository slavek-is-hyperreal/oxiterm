# Deploying the three sites to Cloudflare Pages

Three static sites, three Cloudflare Pages projects. Nothing here needs a server.

| Site | Output directory | Custom domains | Content |
|---|---|---|---|
| Home | `sites/dist/home` | `slavekm.pl` and `www.slavekm.pl` | the owner's home page (placeholders to fill in) |
| OxiUI | `sites/dist/oxiui` | `oxiui.slavekm.pl` | live demo, gallery, docs, roadmap |
| OxiScript | `sites/dist/oxiscript` | `oxiscript.slavekm.pl` | playground, TypeScript page, tour, course landing |

Each directory is self-contained (own copy of the OxiUI wasm bundle, so "view this page in OxiUI" works on every site and no site depends on another at runtime).

## Languages: Polish and English only for now

The first publication is **PL + EN**. Latin (`la`) is written and kept in `sites/i18n/la.json` (draft, awaiting the owner's review) but is **not built by default**: no `/la/` pages, no Latin in the switcher, `sitemap.xml` or `hreflang`, and the in-memoriam dedication and the note about Latin are not rendered. `sites/check.mjs` fails the build if Latin leaks into a default build.

To enable Latin later, after review: build with `SITES_LANGS=pl,en,la sites/build.sh`, fill in the placeholders in `sites/i18n/*.json` (`home.latinNote`, `home.dedication.*`) first.

## Build (on your machine)

Cloudflare's build image has no Rust/wasm toolchain, so build locally and upload the output directories. Requirements: Rust with the `wasm32-unknown-unknown` target, `wasm-pack` (it downloads `wasm-opt` and a matching `wasm-bindgen` on first use), Node 18+.

```
sites/build.sh                 # wasm (release, size-optimised) + all pages + checks + cargo test of the playground
sites/build.sh --pages         # only pages (reuse sites/.build/pkg_*), after editing text or CSS
sites/build.sh --preview       # build, then serve http://localhost:8081 (home), :8082 (oxiui), :8083 (oxiscript)
node sites/check.mjs           # link/i18n/HTML checks alone (also run by build.sh)
```

Preview note: links between the three sites point to the real domains (they are absolute). To keep them local, build with `ORIGIN_HOME=http://localhost:8081 ORIGIN_OXIUI=http://localhost:8082 ORIGIN_OXISCRIPT=http://localhost:8083 node sites/build.mjs`.

## Cloudflare steps (what the owner must click)

Do this once per site. Use **Direct Upload** (no Git integration needed):

1. Cloudflare dashboard, **Workers & Pages**, **Create**, **Pages**, **Upload assets**. Project name `slavekm-home`, `oxiui` or `oxiscript`. Upload the matching `sites/dist/<site>` folder (drag the folder; the `_headers` file inside is picked up automatically).
   Command line alternative, with `npm i -g wrangler` and `wrangler login`:
   `wrangler pages deploy sites/dist/oxiui --project-name oxiui` (same for the others).
   Note: a project created by Direct Upload cannot later be switched to Git integration; create a new project if you want that.
2. **Remove the dead redirect first.** The domain `slavekm.pl` currently has a redirect rule and possibly a conflicting DNS record. In the zone `slavekm.pl`:
   - **Rules, Redirect Rules** (and **Bulk Redirects**, **Page Rules** on older accounts): delete the rule that redirects `slavekm.pl`, `www`, `oxiui` and `oxiscript` to the dead demo.
   - **DNS**: delete existing `A`/`AAAA`/`CNAME` records for `@`, `www`, `oxiui`, `oxiscript` (the old Mikrus host), or let Cloudflare replace them when it offers to.
3. In each Pages project, **Custom domains**, **Set up a custom domain**: `oxiui.slavekm.pl` for the OxiUI project, `oxiscript.slavekm.pl` for OxiScript, and for the home project both `slavekm.pl` and `www.slavekm.pl`. Cloudflare creates the DNS records and the certificate (a few minutes). For the apex, DNS must be on Cloudflare (it is).
4. Optional: in the home project, a redirect `www.slavekm.pl` to `slavekm.pl` (Rules, Redirect Rules, `www` host, 301) so there is one canonical address; the pages already declare `<link rel="canonical">` to the bare domain.
5. Keep the "no trackers" promise true: in the zone leave **Web Analytics** off (it injects a beacon script), keep **Email Address Obfuscation** and **Rocket Loader** off (both inject scripts), and do not enable Cloudflare "Browser Insights" for the Pages projects.

## Headers, MIME and caching (already in each `_headers`)

- `*.wasm` is served as `application/wasm` (needed for streaming compilation).
- `/assets/<hash>/*`, `/app/<hash>/*` and `/play/<hash>/*` are content-hashed and `immutable` for a year; HTML is `max-age=0, must-revalidate`, so a new deploy shows up at once and old wasm is never mixed with new JS.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, a restrictive `Permissions-Policy`.
- No `Content-Security-Policy` is shipped yet. If you add one, wasm needs `script-src 'self' 'wasm-unsafe-eval'` and the demo iframes need `frame-src 'self'`.

## After deploy

- Open each site, start the live demo (OxiUI) and press Check in the playground (OxiScript).
- Run PageSpeed Insights (pagespeed.web.dev) for each URL; Lighthouse was not available on the build machine, so the numbers in HANDOFF-8w.md are page weights only.
- Submit `https://<site>/sitemap.xml` in Search Console if you want indexing sooner.

## What changes later

- 8.L: enable the Run button (`sites/oxiscript/js/playground.js`, search `TODO(8.L)`) and add `compile`/`compile_bytes` to `oxiscript/oxiscript-playground/src/lib.rs`; no page structure changes.
- Latin: see above. Course lessons: add them to `sites/i18n/<lang>.json` (one file per language, no strings elsewhere).
