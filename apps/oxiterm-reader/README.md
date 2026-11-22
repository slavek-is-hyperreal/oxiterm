# oxiterm-reader

Reader mode over SSH: the web, Hacker News, Wikipedia (PL/EN) and RSS rendered by
OxiTerm as clean THTML pages, with real images. A separate process (an OxiTerm *App
Server*) generates pages into a directory it owns; OxiTerm serves them.

```
 browser / ssh  <->  oxiterm-cli serve reader-site/index.thtml   <->  /events  <->  oxiterm-reader
                                  ^------ reads reader-site/cache/<id>.thtml -----------'
```

## Run

```bash
export OXITERM_APP_TOKEN="$(openssl rand -hex 24)"      # shared secret, REQUIRED (fail closed)

# 1. the reader (writes reader-site/index.thtml and serves /events)
cargo run -p oxiterm-reader -- --site-dir reader-site --listen 127.0.0.1:3100

# 2. OxiTerm, pointed at the reader's start page and /events
OXITERM_APP_SERVER=http://127.0.0.1:3100/events \
  oxiterm-cli serve reader-site/index.thtml --web-port 8080
```

The reader refuses to start without `OXITERM_APP_TOKEN`; `/events` answers `401` to every
request whose Bearer token does not match (constant-time comparison).

## Navigation (`navigate`)

Every successful action answers with a state patch such as

```json
{"status": "", "navigate": "cache/<id>.thtml"}
```

OxiTerm moves the session (SSH and web) to that page. The path is relative to the app base
directory (the directory of the start page, i.e. the reader-site root), only relative
`.thtml` paths are accepted, and the same containment check and navigation throttle as for
`event-htmx` links applies. Mobile viewports get `cache/<id>_mobile.thtml` automatically
(the reader writes both variants). The "Back" link on every page is the action `go_home`
(not rate-limited), answered with `{"navigate": "index.thtml"}`: the engine resolves plain
`.thtml` link targets relative to the *current* page's directory (`cache/`), so a static
`index.thtml` link would not work from there.

On errors the patch contains only `status` (`Error: ...`), shown on the current page (every page binds `status`).

## Actions (`event-htmx`)

| Action | Reads state | Effect |
|---|---|---|
| `fetch_url` | `url` (bound `<input>`) | fetch, extract, render an arbitrary page |
| `hn_top` | | top Hacker News stories (Firebase API) |
| `open_story:<id>` | | one HN story (numeric id) |
| `wiki_search:<pl\|en>` | `wiki_q` | Wikipedia title search (REST) |
| `wiki_random:<pl\|en>` | | random article |
| `rss:<n>` | | n-th feed from the config |
| `open_link:<pageid>.<n>` | | follow link `n` of a generated page |
| `go_home` | | navigate back to the start page |
| `open_original:<pageid>` | | web only: `open_url` patch to the original page |

Unknown actions (including the built-in `set:...` that OxiTerm also forwards) get `204`.
`url` and `wiki_q` arrive only because the start page binds them with `bind-value`.

### Link ids are opaque and page-scoped

Pages never contain a URL taken from content. Links are shown as `[n]` in the text and in a
numbered list at the end; clicking one sends `open_link:<pageid>.<n>`. `<pageid>` is a
128-bit keyed hash (`cache/.salt`), `n` a small integer. The reader keeps the table
(`cache/<pageid>.json`, in memory plus a sidecar file) so an old page opened via "Back"
still works, also after a restart. Ids are validated with a strict grammar before any
filesystem access; forged or stale ids fetch nothing.

## Configuration (`reader.toml`, all optional)

```toml
site_dir = "reader-site"
listen = "127.0.0.1:3100"
allow_any_url = true          # false on public instances!
allowed_hosts = []            # extra hosts when allow_any_url = false (suffix match)
actions_per_minute = 20       # per OxiTerm session
max_concurrent_fetches = 8    # global
max_fetches_per_action = 60   # page + HN items + images of one click
max_html_bytes = 5242880
max_image_bytes = 5242880
max_images_per_article = 20
max_redirects = 5
connect_timeout_secs = 5
total_timeout_secs = 15
cache_ttl_secs = 600
cache_max_age_hours = 24      # pruned at start-up
hn_stories = 25
content_cols = 74

[[feeds]]
name = "Rust Blog"
url = "https://blog.rust-lang.org/feed.xml"
```

Up to 4 feeds are shown on the desktop start page (6 on mobile).

## Security model

The reader fetches URLs chosen by users, so it is an SSRF and DoS target. Layers:

* **URLs**: http/https only, no `user:pass@`, ports 80/443 only, IP literals in any spelling
  (`2130706433`, `0x7f000001`, `127.1`, `[::ffff:7f00:1]`) normalised by the `url` crate and
  checked, `localhost`/`.local`/`.internal`/dotless names refused. Re-validated on every
  redirect hop (redirects are followed manually, max 5).
* **DNS**: a custom `ureq::Resolver` returns only public addresses and ureq connects to exactly
  those, so the vetted address is the connected address (no check-then-resolve rebinding gap).
  Blocked: loopback, RFC 1918, link-local (incl. cloud metadata 169.254.169.254), CGNAT
  100.64/10, 0/8, 192.0.0/24, benchmark/documentation ranges, multicast, 240/4, broadcast;
  IPv6 loopback/unspecified/ULA/link-local/site-local/multicast/documentation/Teredo and
  IPv4-mapped/compatible/NAT64/6to4 forms whose embedded IPv4 is not public.
* **Limits**: connect and total timeouts, a body cap enforced on the decoded stream (gzip
  bombs) plus a Content-Length pre-check, per-action fetch budget, global concurrency cap,
  per-session action rate limit, bounded rate-limiter memory, request body cap on `/events`.
* **No ambient authority**: no cookies, no credentials, no proxy env vars, fixed User-Agent
  `oxiterm-reader/0.1 (+...)`.
* **Content**: all text is control-character/bidi/private-use filtered and entity-escaped; no
  page attribute, class, style, URL or path is ever copied into THTML (alt text is stripped of
  quotes and angle brackets); clickable labels contain no East-Asian-ambiguous symbols;
  unbreakable tokens are shortened; markup nesting beyond 1000 levels is refused, and
  readability is bypassed for pathological documents (it is super-linear).
* **Images**: PNG/JPEG/GIF/WebP only, decode-bomb guard from the header (> 8000 px edge or
  > 40 MP rejected before decoding), decoder allocation limit, re-encoded to PNG (metadata
  stripped, GIF reduced to its first frame, downscaled to <= 800 px), stored under
  `cache/<keyed-hash>.png`. SVG from the web is never fetched (alt text only).
* **Public instances**: set `allow_any_url = false`. Then only Hacker News, Wikipedia, the
  configured feeds and `allowed_hosts` are reachable (HN's "Read the article" will not work
  for other hosts), and run the reader without network access to internal services anyway.

## Known limitations

* Responses are decoded as UTF-8 (lossy); legacy charsets will show replacement characters.
* Tables are skipped on Wikipedia and flattened to one line per row elsewhere.
* No JavaScript execution: client-rendered sites yield little.
* Polish diacritics in link labels trigger `lint_layout.py` info lines (I007, ambiguous-width
  *letters*, harmless); only ambiguous symbols are replaced.
* Image drawing in the browser canvas follows the engine (images are overlays there).

## Tests

`cargo test -p oxiterm-reader` runs without network: unit tests (IP/URL guard, fetcher with a
scripted backend, escaping, link mapping, image math and decode-bomb guard, RSS/Atom, HN,
Wikipedia), a loopback server test for the real ureq backend, and end-to-end tests that parse
every generated page with `THTMLParser`, run `LayoutEngine` + `Renderer`, and assert that every
`event-htmx` and `src` in the output matches an allow-list. Generated pages are copied to
`target/tmp/reader-lint/` so the layout linter can be run on them:

```bash
python3 -I .claude/skills/thtml-tcss-authoring/scripts/lint_layout.py target/tmp/reader-lint
```
