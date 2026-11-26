# Deploying the three sites to Cloudflare Pages

Three static sites, three Cloudflare Pages projects. Nothing here needs a server.

| Site | Output directory | Custom domains | Content |
|---|---|---|---|
| Home | `sites/dist/home` | `slavekm.pl` and `www.slavekm.pl` | the owner's home page (neutral project overview; the About block is off until the owner writes it) |
| OxiUI | `sites/dist/oxiui` | `oxiui.slavekm.pl` | live demo, gallery, docs, roadmap |
| OxiScript | `sites/dist/oxiscript` | `oxiscript.slavekm.pl` | playground, TypeScript page, tour, course landing |

Each directory is self-contained (own copy of the OxiUI wasm bundle, so "view this page in OxiUI" works on every site and no site depends on another at runtime).

## Languages: Polish and English only for now

The first publication is **PL + EN**. Latin (`la`) is written and kept in `sites/i18n/la.json` (draft, awaiting the owner's review) but is **not built by default**: no `/la/` pages, no Latin in the switcher, `sitemap.xml` or `hreflang`, and the in-memoriam dedication and the note about Latin are not rendered. `sites/check.mjs` fails the build if Latin leaks into a default build.

To enable Latin later, after review: build with `SITES_LANGS=pl,en,la sites/build.sh`, fill in the placeholders in `sites/i18n/*.json` (`home.latinNote`, `home.dedication.*`) first.

## Owner review gates (nothing is published as reviewed until you say so)

- **Polish is a draft.** `sites/i18n/pl.json` has `"meta": { "status": "draft" }`, so every PL page shows the "Tłumaczenie robocze, czeka na przegląd" banner. The Polish copy was written by an agent. After you have read it, change `"status": "draft"` to `"reviewed"` in `sites/i18n/pl.json` and rebuild; the banner disappears. (English is the source language and carries `"source"`; Latin stays `"draft"`.)
- **Home page "About".** The About block on `/` and `/pl/` is behind the flag `home.about` and is **off**: no biography is invented. Write the text in `sites/i18n/en.json` and `pl.json` (`home.pages.index.blocks`, the two blocks with `"flag": "home.about"`, replacing the `[PLACEHOLDER ...]` / `[DO UZUPEŁNIENIA ...]` text), then build with `SITES_FLAGS=home.about sites/build.sh`.
- **Placeholder guard.** `sites/check.mjs` fails the build if `[PLACEHOLDER` or `[DO UZUPE` appears in any output file. `SITES_ALLOW_PLACEHOLDERS=1` switches the guard off (private previews only; never upload that build).
- **Links to `AGENTS.md` and `docs/oxiscript/llms.txt`** on GitHub `main` are checked against the working tree by `sites/check.mjs` like every other repo link (slice 8.D is merged).

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
2. **Remove the dead redirect first.** The domain `slavekm.pl` currently has a redirect rule and possibly a conflicting DNS record. Before adding the custom domains, check ALL of these places for anything that touches `slavekm.pl`, `www`, `oxiui` or `oxiscript` (a leftover one keeps winning over Pages and the site "does not deploy"):
   - zone **Rules, Redirect Rules**: delete the rule that redirects these hosts to the dead demo;
   - **Bulk Redirects**: these live at the **account** level (Account home, Bulk Redirects), not in the zone: open the lists and the Bulk Redirect rules and remove entries for these hosts;
   - **Page Rules** (older accounts; zone, Rules, Page Rules): delete forwarding rules for these hosts;
   - **Workers & Pages, Workers**: any Worker **route** (zone, Workers Routes) or Worker custom domain on these hostnames: remove it;
   - zone **Rules, Origin Rules / Configuration Rules / Transform Rules**: nothing there may rewrite or redirect these hosts.
   Then **DNS**:
   - **Delete only** the `A`, `AAAA` and `CNAME` records for `@`, `www`, `oxiui`, `oxiscript` (the old Mikrus host).
   - **Keep everything else.** In particular keep: **`MX`** (mail), **`TXT`** (SPF `v=spf1 ...`, DKIM selectors such as `*._domainkey`, DMARC `_dmarc`, and any domain-verification records such as Google/Search Console, GitHub, Microsoft), **`CAA`** (an existing CAA that does not allow the CA Cloudflare uses, for example `letsencrypt.org`, `pki.goog`, `digicert.com`, `ssl.com`, blocks the Pages certificate; if issuance fails, add those CAA records or remove the restrictive one deliberately), `NS`, `SRV`, and any record that is not a web record for the four names above.
3. In each Pages project, **Custom domains**, **Set up a custom domain**: `oxiui.slavekm.pl` for the OxiUI project, `oxiscript.slavekm.pl` for OxiScript, and for the home project both `slavekm.pl` and `www.slavekm.pl`. Cloudflare creates the DNS records and the certificate (a few minutes). The **apex** `slavekm.pl` works because the zone's DNS is on Cloudflare: add it in the Pages project exactly like the subdomains (Pages creates a proxied `CNAME` for `@` pointing at `<project>.pages.dev`; Cloudflare's **CNAME flattening** serves it as an apex record). Do not create an `A` record or a redirect for the apex by hand. Wait until the domain shows **Active** before testing.
4. Optional: in the home project, a redirect `www.slavekm.pl` to `slavekm.pl` (Rules, Redirect Rules, `www` host, 301) so there is one canonical address; the pages already declare `<link rel="canonical">` to the bare domain.
5. Keep the "no trackers" promise true: in the zone leave **Web Analytics** off (it injects a beacon script), keep **Email Address Obfuscation** and **Rocket Loader** off (both inject scripts), and do not enable Cloudflare "Browser Insights" for the Pages projects.

## Headers, MIME and caching (already in each `_headers`)

- `*.wasm` is served as `application/wasm` (needed for streaming compilation).
- `/assets/<hash>/*`, `/app/<hash>/*` and `/play/<hash>/*` are content-hashed and `immutable` for a year; HTML is `max-age=0, must-revalidate`, so a new deploy shows up at once and old wasm is never mixed with new JS.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, a `Permissions-Policy` that denies camera, microphone, geolocation, sensors, payment, USB and display capture.
- **Content-Security-Policy**, strict and without `'unsafe-inline'`: own origin only (`default-src 'self'`), no inline script or style, `object-src 'none'`, `base-uri 'none'`, `form-action 'none'`, `frame-ancestors 'self'` (the pages can only be framed by themselves, e.g. the demo iframe). `'wasm-unsafe-eval'` is granted only to `/app/*` (the OxiUI client) and to the OxiScript playground page (`/` and `/pl/` of oxiscript). The copied 7.1 client keeps its script and style in hashed files under `/app/<hash>/`, so it uses the same strict policy plus `blob:` for media and workers.
- **Cloudflare Pages merges the headers of every matching `_headers` rule**; two different CSPs on one path would both be enforced (their intersection). So the CSP is attached to explicit, non-overlapping paths (one rule per HTML page plus `/app/`), never to `/*`. `sites/check.mjs` fails if two different policies apply to a path. If you add a page, rebuild: `_headers` is generated.
- `sites/test/csp_browser.mjs` serves `dist/` with these rules applied and opens every page, the live demo and the playground in headless Chromium (needs `playwright-core`, set `PLAYWRIGHT_CORE`); it fails on any CSP violation or console error.

### HSTS (one switch in Cloudflare, after the sites work over HTTPS)

Not a `_headers` line: set it in the dashboard, zone `slavekm.pl`, **SSL/TLS, Edge Certificates, HTTP Strict Transport Security (HSTS)**. Recommended start: enable, `max-age` 6 months, **Include subdomains: on** only if every subdomain of `slavekm.pl` (including mail or other hosts) is HTTPS-only, **No-Sniff: on**. Leave **Preload off**. Warning: once browsers have seen HSTS they refuse plain HTTP for the whole period, and **preload** (submitting to hstspreload.org) is effectively irreversible for months because browsers ship the list; enabling it for `slavekm.pl` would also force every present and future subdomain to HTTPS. Turn on includeSubDomains/preload only deliberately, never as a default. Also enable **Always Use HTTPS** (SSL/TLS, Edge Certificates) and keep the SSL mode on **Full (strict)**.

## After deploy

- Open each site, start the live demo (OxiUI) and press Check in the playground (OxiScript).
- Run PageSpeed Insights (pagespeed.web.dev) for each URL; Lighthouse was not available on the build machine, so the numbers in docs/stages/8.W/implementation.md are page weights only.
- Submit `https://<site>/sitemap.xml` in Search Console if you want indexing sooner.

## What changes later

- Run is live: the page loads a second wasm bundle (`/play/<hash>/oxiscript_runner*`, built by `sites/build.sh` with the `run` feature of `oxiscript-playground`) inside a Web Worker (`/assets/<hash>/runner-worker.js`, same origin, covered by the page CSP) on the first press of Run. No CSP change was needed.
- Latin: see above. Course lessons: add them to `sites/i18n/<lang>.json` (one file per language, no strings elsewhere).
