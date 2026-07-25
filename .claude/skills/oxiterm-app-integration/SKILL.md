---
name: oxiterm-app-integration
description: Build or modify an application backend ("App Server") that talks to OxiTerm — handling event-htmx actions, returning state patches, pushing live updates into sessions, wiring OAuth or login flows, or debugging why a THTML binding never updates. Use this skill whenever work involves the boundary between an OxiTerm UI and real application logic: any custom event-htmx action, any /events handler, any POST to /sessions/{id}/patch, any Flask/FastAPI/Express/Axum service sitting behind an OxiTerm frontend, or any state key that has to survive a round trip. Also use it when asked to add a feature to an existing OxiTerm app, since almost every such feature crosses this boundary.
---

# Building an application on OxiTerm

OxiTerm renders the interface and owns per-session state. It does **not** do application logic: no database access, no external APIs, no authentication against a third party, no validation beyond built-in state mutations. That work belongs in a separate process, the **App Server**, reached over HTTP.

This skill covers the whole contract. `docs/app-server-guide.md` has framework-specific example code; read it for boilerplate, but note that §2 of this skill corrects a significant inaccuracy in it.

For anything touching `.thtml` or `<style>` blocks, also use the `thtml-tcss-authoring` skill — the two halves of a feature almost always land in both.

---

## 1. Wiring

| Variable | Set on | Meaning |
|---|---|---|
| `OXITERM_APP_SERVER` | OxiTerm | **The complete URL that gets POSTed to.** Nothing is appended — write `http://localhost:3000/events`, not `http://localhost:3000`. |
| `OXITERM_APP_TOKEN` | both | Shared secret. Used in both directions. |

`OXITERM_APP_TOKEN` is a **circuit breaker, not optional hardening**. If it is unset or empty, OxiTerm disables the inbound push endpoint completely and every `POST /sessions/{id}/patch` returns `404`. If your pushes are 404-ing and the session definitely exists, check the token before anything else.

Other variables that affect integration: `OXITERM_WEB_PORT` (default 8080, where the push endpoint lives), `OXITERM_PORT` (SSH, default 2222), `OXITERM_TRUSTED_PROXY`, `OXITERM_ALLOW_GUEST`, `OXITERM_MAX_SESSIONS`, `OXITERM_MEDIA_BASE_URL`.

---

## 2. OxiTerm → App Server: `POST /events`

Fired on every `event-htmx` activation that is not `open:...` and not a `.thtml` navigation. Sent fire-and-forget from a spawned thread, with `Authorization: Bearer <token>` when the token is set.

```json
{
  "action": "refresh_now_playing",
  "state": { "tab": "player", "username": "slavek" },
  "session_id": 42,
  "username": "slavek",
  "auth_method": "SshKey"
}
```

`action` is the **raw** `event-htmx` string. Built-in actions arrive here too — an `event-htmx="set:tab=player"` is applied locally *and* dispatched, so your handler will see `"set:tab=player"`. Ignore what you do not recognise and return `204`; do not error on unknown actions.

### 2.1 The state snapshot contains only bound keys — this is the big one

`docs/app-server-guide.md` describes `state` as "the current session state". That is wrong, and the difference will cost you an afternoon.

`session.rs::try_dispatch` walks the **current document's node tree** and collects only those keys that appear in a `bind-state`, `bind-value`, or `bind-show` attribute *on the page being displayed right now*. Everything else in the session's state is omitted.

Consequences you must design around:

- A key your handler needs must be **referenced somewhere in the current page**, or it will not arrive. If nothing displays it, bind it to a hidden node: `<text bind-show="never_true" bind-state="needed_key"/>`, or fold it into a `bind-show` you already have.
- After navigating to another page, keys bound only on the previous page stop being sent — even though they still exist in the session.
- Do not treat the snapshot as authoritative for anything security-relevant. It is a view of the UI, not a source of truth. Keep authoritative per-session data in your own store, keyed by `session_id`.

`_`-prefixed reserved keys **are** included when bound, so `bind-show="_is_web=true"` puts `_is_web` in the payload.

### 2.2 Responding

| Response | Effect |
|---|---|
| `204 No Content`, or `200` with empty body | nothing happens |
| `200` with a JSON object | applied as a state patch to that session |
| anything else | logged as a failure, no patch |

```json
{ "now_playing": "Lordofon — Passé", "progress": "42", "auth_error": "" }
```

### 2.3 `open_url`: the one-shot browser command

Undocumented in the guide but fully implemented in `dispatcher.rs`. If your patch object contains the key `open_url`, OxiTerm **removes it before applying the patch** and instead emits a browser-open command for that session. It is never stored in state.

```json
{ "status": "Opening Spotify...", "open_url": "https://accounts.spotify.com/authorize?...&state=abc123" }
```

This is the correct way to send a user to a URL that must be generated per-request — an OAuth authorize link with a fresh CSRF `state`, a signed download link, a magic login link. A static `event-htmx="open:https://..."` in the markup cannot carry a per-request parameter, so it cannot be made CSRF-safe; `open_url` can. It works on web sessions only, matching the `open:` action.

---

## 3. App Server → OxiTerm: `POST /sessions/{id}/patch`

Push state into a live session at any time, without waiting for a user event.

```
POST http://<oxiterm-host>:<web-port>/sessions/42/patch
Authorization: Bearer <token>
Content-Type: application/json

{"now_playing": "Lordofon — Passé", "progress": "42"}
```

| Code | Meaning |
|---|---|
| `200` | applied |
| `400` | `{id}` is not an integer |
| `401` | token missing or wrong |
| `404` | endpoint disabled (empty `OXITERM_APP_TOKEN`) **or** no such session |

`404` is deliberately ambiguous between "disabled" and "no such session" so it cannot be used to enumerate live sessions. Do not try to distinguish them.

### 3.1 Patch limits and type mapping

Enforced in `session.rs::apply_state_patch`. Violations are skipped with a log warning — the request still returns `200`, so a silently dropped key looks like success.

| Limit | Value |
|---|---|
| keys per patch | 100 (whole patch rejected if exceeded) |
| key length | 256 chars (that key skipped) |
| string value | 64 KiB (that key skipped) |
| list length | 1000 items (that key skipped) |
| `_`-prefixed keys | always rejected |

| JSON type | Becomes |
|---|---|
| number (integer) | `Int` |
| string | `Str` |
| boolean | `Bool` |
| array | `List` — items stringified |
| `null` | empty string |
| object | JSON-serialised string (useless for display — **flatten before sending**) |

Send flat, string-or-number values. A nested object reaches the UI as raw JSON text.

---

## 4. Security requirements

These are not suggestions; each one corresponds to a real vulnerability found and fixed in this repository.

**Verify the Bearer token on `/events`, in constant time.** `secrets.compare_digest`, `crypto.timingSafeEqual`, or equivalent. An unauthenticated `/events` lets anyone on the network drive application logic on behalf of arbitrary sessions.

**Fail closed when the token is unconfigured.** If `OXITERM_APP_TOKEN` is empty, reject with `401` rather than skipping the check. The failure mode of a fail-open guard is total and silent; a regression that flipped exactly one `else { false }` into `else { true }` survived three commits here because no test exercised the `None` branch.

**Never send a `session_id` → user identity mapping back in a patch.** Patches are rendered into the client's screen. Anything you patch in is visible to that user. Keep the mapping server-side only.

**Never bind a token or secret to more than one session.** Look up state by the `session_id` in the request; do not fall back to "the most recently seen session" or "all sessions" when the lookup misses. Return an error instead. Convenience fallbacks of exactly this shape were the root cause of a cross-session token leak here.

**Verify OAuth `state` strictly:** generated per-request, bound to that `session_id`, single-use, with a TTL (600 s is the value used in this repo). Reject on mismatch, reuse, or expiry. Pair it with `open_url` (§2.3) so the parameter can be fresh.

**Escape anything reflected into an HTTP response** — an OAuth `/callback` page that echoes a query parameter is an XSS sink. `html.escape` or the framework equivalent.

**Keep secrets out of the repository and out of Docker image layers.** A client secret hardcoded in a commit is burned the moment it is pushed; rotate it rather than removing it in a later commit. Pass secrets as runtime environment variables; a separate `Dockerfile.test` avoids baking them into a published layer.

**Restrict credential-store permissions** — `0600` on a SQLite token database.

---

## 5. Patterns

### 5.1 Authorisation gate

Do not render an application behind a "please log in" tab that the user could navigate around. Gate the whole tree on a state key your App Server controls, so before authentication the auth screen is the *only* thing that exists, and after it the auth screen is gone.

```html
<!-- visible when the key is absent OR falsy: correct on first paint -->
<box bind-show="is_authenticated=false" style="flex-direction: column; height: 24;">
  <text style="height: 1;">Not connected.</text>
  <box class="btn" event-htmx="begin_login">
    <text style="height: 1;">Connect</text>
  </box>
</box>

<box bind-show="is_authenticated=true" style="flex-direction: column; height: 24;">
  <!-- the actual application -->
</box>
```

The `=false` form is essential: a plain `bind-show="is_authenticated"` would be false when absent *and* the `=true` branch would also be false, so neither box would show on first paint. Set `is_authenticated` from the App Server; never let the client assert it.

### 5.2 Live updates from a background job

A poller or webhook handler in the App Server pushes to `/sessions/{id}/patch` for each session it knows about. Keep your own `session_id → subscription` registry, and drop entries on `404`. Bind the pushed keys with `bind-state` in the THTML so they render on arrival.

### 5.3 Copyable text across both transports

The web canvas has no text selection; a terminal does. So a value the user must copy needs two presentations:

```html
<box bind-show="_is_web=true" class="btn" event-htmx="trigger_open">
  <text style="height: 1;">Open link in browser</text>
</box>
<text bind-show="_is_web=false" bind-state="auth_url" style="wrap: word; width: 60;"/>
```

Note that `wrap: word` will not break a long URL, since it breaks only at spaces — see the `thtml-tcss-authoring` skill §3.3. Constrain the container and accept clipping, or shorten the URL server-side.

### 5.4 Multi-session isolation

OxiTerm keeps a separate state store per session automatically. Your App Server must do the same: every lookup keyed by `session_id`, no shared mutable defaults, no "current user" global. Two people using the app at once is the normal case, not an edge case.

---

## 6. Debugging checklist

Work down this list before forming a theory.

**A binding never updates.**
1. Is the key in the patch actually reaching OxiTerm? Check for a `200` from `/sessions/{id}/patch`.
2. Is the key `_`-prefixed? Rejected.
3. Did it exceed a §3.1 limit? The response is still `200`; check OxiTerm's log for the skip warning.
4. Is the key spelled identically in the patch and in `bind-state`?

**The App Server gets a state key as empty or missing.**
1. Is that key bound anywhere on the *currently displayed* page? If not, it is not sent — §2.1. This is the most likely answer.
2. Did the user navigate to a page where the binding does not exist?

**Pushes return 404.**
1. Is `OXITERM_APP_TOKEN` non-empty on the OxiTerm side? Empty disables the endpoint entirely.
2. Is the session still alive? `404` covers both cases and will not tell you which.

**A custom action never arrives.**
1. Is `OXITERM_APP_SERVER` set, and does it include the full path (`/events`)?
2. Does the action string end in `.thtml`, or start with `open:`? Those are handled locally and never dispatched.
3. Is `/events` returning `401`? Check the Bearer comparison.

**Anything visual.** Stop and use the `thtml-tcss-authoring` skill. Layout symptoms have layout causes, and TCSS discards unknown properties without warning, so a plausible-looking style block can be doing nothing at all.

---

## 7. Verification

Run the real suites rather than reasoning about correctness:

```bash
cargo test --workspace                          # engine and server
cd spotify-app-server && python -m pytest       # reference App Server
```

Then exercise the flow by hand, over web **and** SSH if the change touches interaction. Automated tests in this repository have repeatedly passed while the screen was visibly wrong, and several security regressions passed because the test covered a helper function rather than its call site. When you add a guard, add a test that hits the *failing* branch — the `None` case, the expired token, the wrong session — not only the happy path.

When reviewing work an agent has already committed, read the committed diff. A walkthrough of what the code supposedly does is not evidence; placeholder implementations that satisfy a description while doing nothing have been found in this project more than once.
