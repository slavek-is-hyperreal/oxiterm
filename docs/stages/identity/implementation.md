# Identity (8.W, owner 2026-11-25) applied to the three sites

## Palette (CSS tokens in sites/shared/site.css and site.css.d/*; THTML in sites/shared/brand/palette.json)

| Token | Dark | Light | Role |
|---|---|---|---|
| `--acc` rust orange | `#ef7d3f` (text on it `--acc-fg` `#1a0b03`) | `#a9400f` (`#ffffff`) | action: buttons, nav current, cursor, focus ring |
| `--link` | `#f59a66` | `#a9400f` | links |
| `--pat` bronze patina | `#6cc3a4` | `#1c6b54` | calm/content: eyebrow, h2 markers, card top rule, editor rule, `.oxs` |
| `--coral` (mascot body, ASCII) | `#f6a89f` | `#b0443a` | text-safe coral |
| bg / card / code-bg oxiui | `#0b1513 / #112320 / #08100e` | `#f3f8f5 / #ffffff / #e5efe9` | patina-tinted surfaces |
| bg / card / code-bg oxiscript | `#0c1417 / #131f23 / #080f12` | `#f4f8f8 / #ffffff / #e6eff0` | slate-patina "editor" surfaces |
| bg / card / code-bg home | `#181512 / #221e1a / #120f0d` | `#faf7f2 / #ffffff / #f1ece4` | warm neutral, calm umbrella |

All pairs are checked by `sites/test/bad_actor_contrast.mjs` (thresholds unchanged or raised: acc/link on card now 4.5, pat and coral on bg/card/code-bg 4.5, THTML palette pairs 4.5, and the test now sets a non-zero exit on a failure).

## Brand assets (sites/shared/brand/)

- `oxi-cell-{home,oxiui,oxiscript}.svg`: the "Oxi cell" (square, one rounded corner). Home: ring with a patina cursor, OxiUI: filled with faint grid, OxiScript: filled with a `?` drawn as a path. Pure SVG, no text/fonts, hard-coded colours so the favicon reads on light and dark tab bars.
- `oxi-axolotl.svg` (hero, 240x190), `oxi-axolotl-small.svg` (64x56, phones and footer).
- `oxi-ascii.mjs`: ASCII axolotl, 34 columns, 11 rows. HTML `<pre>` (gill spans rust, body coral) and THTML (three text pieces per row, columns kept with margin-left because the renderer trims blanks at the ends of a text node).
- `palette.json`: THTML colours and per-site surfaces.
- build.mjs copies the SVGs to `/assets/<hash>/brand/` (immutable caching), favicon is `<link rel="icon" type="image/svg+xml">`, header mark and mascots are `<img>` (no inline SVG, CSP and check.mjs untouched).

## Placement

Header: cell mark. Hero (index pages): mascot corner (vector on home and OxiScript, ASCII on OxiUI; phones: small head above the title). 404: vector mascot. Footer: small head. THTML "view in OxiUI": ASCII axolotl on every index page and on the OxiUI demo page. Physarum: skipped as agreed.

## Weight (raw bytes, files other than wasm)

| Site | Before | After | Growth |
|---|---|---|---|
| home | 190716 | 206802 | +16.1 KB |
| oxiui | 415946 | 444315 | +28.4 KB (ASCII repeated in 36 THTML pages) |
| oxiscript | 381294 | 397717 | +16.4 KB |

One HTML page: oxiscript `/` 9399 -> 9570 bytes. Brand SVGs: 4.2 KB total (one extra cached request per site).

## Verification

`sites/build.sh --pages` (sites check ok), all `sites/test/*.mjs` pass including `csp_browser.mjs` (28 pages, 0 failures). Screenshots in the session scratchpad `identity-shots/`.

## Contrast table (sites/test/bad_actor_contrast.mjs output; pass = ratio >= threshold: 4.5 text, 3 control borders and err)

```
home      dark  fg on bg             15.48 
home      dark  muted on bg          7.15 
home      dark  muted on card        6.51 
home      dark  muted on code-bg     7.50 
home      dark  link on bg           8.40 
home      dark  link on code-bg      8.82 
home      dark  link on card         7.64 
home      dark  acc on bg            6.65 
home      dark  acc on code-bg       6.98 
home      dark  acc-fg on acc        7.02 
home      dark  warn on bg           10.84 
home      dark  err on bg            7.97 
home      dark  ok on bg             8.64 
home      dark  ctl on bg            4.23 
home      dark  ctl on card          3.85 
home      dark  ctl on code-bg       4.44 
home      dark  acc on card          6.05 
home      dark  link on card         7.64 
home      dark  pat on bg            8.64 
home      dark  pat on card          7.87 
home      dark  pat on code-bg       9.07 
home      dark  coral on bg          9.54 
home      dark  coral on card        8.69 
home      dark  coral on code-bg     10.02 
home      dark  warn on card         9.86 
home      dark  err on card          7.25 
home      dark  ok on card           7.87 
home      light fg on bg             16.48 
home      light muted on bg          7.27 
home      light muted on card        7.77 
home      light muted on code-bg     6.61 
home      light link on bg           5.73 
home      light link on code-bg      5.21 
home      light link on card         6.13 
home      light acc on bg            5.73 
home      light acc on code-bg       5.21 
home      light acc-fg on acc        6.13 
home      light warn on bg           6.67 
home      light err on bg            6.12 
home      light ok on bg             5.99 
home      light ctl on bg            3.84 
home      light ctl on card          4.10 
home      light ctl on code-bg       3.49 
home      light acc on card          6.13 
home      light link on card         6.13 
home      light pat on bg            5.99 
home      light pat on card          6.40 
home      light pat on code-bg       5.44 
home      light coral on bg          5.26 
home      light coral on card        5.62 
home      light coral on code-bg     4.78 
home      light warn on card         7.13 
home      light err on card          6.54 
home      light ok on card           6.40 
oxiui     dark  fg on bg             15.51 
oxiui     dark  muted on bg          8.11 
oxiui     dark  muted on card        7.14 
oxiui     dark  muted on code-bg     8.41 
oxiui     dark  link on bg           8.57 
oxiui     dark  link on code-bg      8.89 
oxiui     dark  link on card         7.55 
oxiui     dark  acc on bg            6.79 
oxiui     dark  acc on code-bg       7.04 
oxiui     dark  acc-fg on acc        7.02 
oxiui     dark  warn on bg           11.07 
oxiui     dark  err on bg            8.13 
oxiui     dark  ok on bg             8.82 
oxiui     dark  ctl on bg            5.07 
oxiui     dark  ctl on card          4.46 
oxiui     dark  ctl on code-bg       5.26 
oxiui     dark  acc on card          5.97 
oxiui     dark  link on card         7.55 
oxiui     dark  pat on bg            8.82 
oxiui     dark  pat on card          7.77 
oxiui     dark  pat on code-bg       9.15 
oxiui     dark  coral on bg          9.74 
oxiui     dark  coral on card        8.58 
oxiui     dark  coral on code-bg     10.10 
oxiui     dark  warn on card         9.74 
oxiui     dark  err on card          7.16 
oxiui     dark  ok on card           7.77 
oxiui     light fg on bg             15.70 
oxiui     light muted on bg          6.89 
oxiui     light muted on card        7.39 
oxiui     light muted on code-bg     6.29 
oxiui     light link on bg           5.71 
oxiui     light link on code-bg      5.21 
oxiui     light link on card         6.13 
oxiui     light acc on bg            5.71 
oxiui     light acc on code-bg       5.21 
oxiui     light acc-fg on acc        6.13 
oxiui     light warn on bg           6.64 
oxiui     light err on bg            6.09 
oxiui     light ok on bg             5.96 
oxiui     light ctl on bg            3.96 
oxiui     light ctl on card          4.25 
oxiui     light ctl on code-bg       3.61 
oxiui     light acc on card          6.13 
oxiui     light link on card         6.13 
oxiui     light pat on bg            5.96 
oxiui     light pat on card          6.40 
oxiui     light pat on code-bg       5.44 
oxiui     light coral on bg          5.24 
oxiui     light coral on card        5.62 
oxiui     light coral on code-bg     4.78 
oxiui     light warn on card         7.13 
oxiui     light err on card          6.54 
oxiui     light ok on card           6.40 
oxiscript dark  fg on bg             15.54 
oxiscript dark  muted on bg          7.94 
oxiscript dark  muted on card        7.18 
oxiscript dark  muted on code-bg     8.24 
oxiscript dark  link on bg           8.60 
oxiscript dark  link on code-bg      8.92 
oxiscript dark  link on card         7.77 
oxiscript dark  acc on bg            6.80 
oxiscript dark  acc on code-bg       7.06 
oxiscript dark  acc-fg on acc        7.02 
oxiscript dark  warn on bg           11.09 
oxiscript dark  err on bg            8.15 
oxiscript dark  ok on bg             8.85 
oxiscript dark  ctl on bg            4.62 
oxiscript dark  ctl on card          4.18 
oxiscript dark  ctl on code-bg       4.79 
oxiscript dark  acc on card          6.15 
oxiscript dark  link on card         7.77 
oxiscript dark  pat on bg            8.85 
oxiscript dark  pat on card          8.00 
oxiscript dark  pat on code-bg       9.18 
oxiscript dark  coral on bg          9.77 
oxiscript dark  coral on card        8.83 
oxiscript dark  coral on code-bg     10.13 
oxiscript dark  warn on card         10.03 
oxiscript dark  err on card          7.37 
oxiscript dark  ok on card           8.00 
oxiscript light fg on bg             15.97 
oxiscript light muted on bg          6.92 
oxiscript light muted on card        7.40 
oxiscript light muted on code-bg     6.34 
oxiscript light link on bg           5.73 
oxiscript light link on code-bg      5.24 
oxiscript light link on card         6.13 
oxiscript light acc on bg            5.73 
oxiscript light acc on code-bg       5.24 
oxiscript light acc-fg on acc        6.13 
oxiscript light warn on bg           6.66 
oxiscript light err on bg            6.11 
oxiscript light ok on bg             5.98 
oxiscript light ctl on bg            4.08 
oxiscript light ctl on card          4.37 
oxiscript light ctl on code-bg       3.74 
oxiscript light acc on card          6.13 
oxiscript light link on card         6.13 
oxiscript light pat on bg            5.98 
oxiscript light pat on card          6.40 
oxiscript light pat on code-bg       5.48 
oxiscript light coral on bg          5.26 
oxiscript light coral on card        5.62 
oxiscript light coral on code-bg     4.81 
oxiscript light warn on card         7.13 
oxiscript light err on card          6.54 
oxiscript light ok on card           6.40 
home      thtml fg on bg             15.19 
home      thtml muted on bg          7.95 
home      thtml acc on bg            6.65 
home      thtml pat on bg            8.64 
home      thtml coral on bg          9.54 
home      thtml fg on card           13.83 
home      thtml muted on card        7.23 
home      thtml acc on card          6.05 
home      thtml pat on card          7.87 
home      thtml coral on card        8.69 
oxiui     thtml fg on bg             15.51 
oxiui     thtml muted on bg          8.11 
oxiui     thtml acc on bg            6.79 
oxiui     thtml pat on bg            8.82 
oxiui     thtml coral on bg          9.74 
oxiui     thtml fg on card           13.65 
oxiui     thtml muted on card        7.14 
oxiui     thtml acc on card          5.97 
oxiui     thtml pat on card          7.77 
oxiui     thtml coral on card        8.58 
oxiscript thtml fg on bg             15.55 
oxiscript thtml muted on bg          8.13 
oxiscript thtml acc on bg            6.80 
oxiscript thtml pat on bg            8.85 
oxiscript thtml coral on bg          9.77 
oxiscript thtml fg on card           14.05 
oxiscript thtml muted on card        7.35 
oxiscript thtml acc on card          6.15 
oxiscript thtml pat on card          8.00 
oxiscript thtml coral on card        8.83 
```

All rows pass (0 FAIL). The blinking header cursor was dropped (WCAG 2.2.2); the reduced-motion reset now covers pseudo-elements; theme-color has dark and light variants. Wasm bundles were reused from the main checkout (not rebuilt).
