# Spotify Control Center — Demo App

`spotify-app-server/` is a complete example application built on top of the OxiTerm engine. It demonstrates how to connect an external App Server (Python/FastAPI) to OxiTerm to produce a real-world, multi-user TUI with OAuth 2.0 authentication, live state patching, and a panel that works on SSH, web, and mobile simultaneously.

---

## What the Demo Shows

| Feature | Description |
|---|---|
| **OAuth 2.0 Authorization Code Flow** | User initiates login from within OxiTerm; the App Server handles the redirect, token exchange, and securely binds the Spotify token to the session. |
| **Multi-session isolation** | Each OxiTerm session (`session_id`) has its own Spotify login. Sessions do not share tokens; there is no fallback to another user's data. |
| **Push patches from the background** | A background task polls Spotify's Now Playing API and pushes track title, artist, progress, and duration back to the active session via `POST /sessions/{id}/patch`. |
| **Web + SSH + Mobile panels** | The same App Server drives `examples/spotify/panel.thtml` (web/SSH, 80x24) and `examples/spotify/panel_mobile.thtml` (mobile, 48x30). |
| **Animated Progress & Draggable Modal** | Uses OxiTerm 0.6+ CSS transitions (`transition: width 1s linear;`), spring physics (`spring(120, 12, 1)`), absolute positioning, and pointer drag for a floating volume mixer. |
| **Bearer authentication on both channels** | OxiTerm sends `Authorization: Bearer` to the App Server; the App Server checks it with constant-time comparison. |
| **Fail-closed security** | Missing or empty `OXITERM_APP_TOKEN` disables the `/patch` push endpoint entirely (404). |

---

## Architecture

```
User browser / SSH terminal
         │
         ▼
  OxiTerm Server (Rust)
  ┌────────────────────────────────────────┐
  │  examples/spotify/panel.thtml          │
  │  examples/spotify/panel_mobile.thtml   │
  │                                        │
  │  event-htmx → POST /events ──────────►│
  │                              Bearer    │
  │  ◄── state patch (200 JSON) ──────────│
  │                                        │
  │  ◄── background push via              │
  │       POST /sessions/{id}/patch ──────│
  └────────────────────────────────────────┘
         │                      ▲
         ▼                      │
  App Server (Python / FastAPI)
  ┌────────────────────────────────────────┐
  │  app.py                                │
  │  SQLite (spotify_app.db)               │
  │  Background polling thread             │
  └────────────────────────────────────────┘
         │
         ▼
  Spotify Web API
```

---

## Required Environment Variables

Set these in `spotify-app-server/.env` (copy from `.env.example`):

| Variable | Description |
|---|---|
| `SPOTIFY_CLIENT_ID` | Your Spotify application client ID (32-character hex). Create at [developer.spotify.com](https://developer.spotify.com/dashboard). |
| `SPOTIFY_CLIENT_SECRET` | Your Spotify client secret. **Rotate immediately if ever committed to version control.** |
| `SPOTIFY_REDIRECT_URI` | Must exactly match a URI registered in your Spotify app dashboard. Example: `https://your-host/callback`. |
| `OXITERM_APP_TOKEN` | Shared secret between OxiTerm and the App Server. Generate with `openssl rand -hex 32`. |
| `OXITERM_URL` | Base URL of the OxiTerm server (used by the background task to push patches). Example: `http://localhost:8080`. |
| `OXITERM_APP_SERVER` | URL of the App Server's `/events` endpoint. Example: `http://localhost:8889/events`. |

---

## Login Flow

```
1. User clicks "Zaloguj Spotify" in OxiTerm
2. event-htmx="trigger_login" fires
3. OxiTerm → POST /events  (action="trigger_login", session_id=N)
4. App Server generates state + auth_url, returns patch:
      {"auth_url": "https://accounts.spotify.com/authorize?...", "is_authenticated": "false"}
5. OxiTerm renders the auth_url in the panel
   ┌─ Web session ──────────────────────────────────────────┐
   │  action open:https://accounts.spotify.com/...          │
   │  Browser opens Spotify login page automatically        │
   └────────────────────────────────────────────────────────┘
   ┌─ SSH session ──────────────────────────────────────────┐
   │  open: is silently ignored on SSH (see §Limitations)   │
   │  User must manually copy the URL from the auth_url     │
   │  field (bind-state="auth_url") and open it in browser  │
   └────────────────────────────────────────────────────────┘
6. User authorises in Spotify, browser redirects to /callback
7. App Server validates state (single-use, TTL 600 s)
8. App Server exchanges code for access + refresh tokens
9. Tokens stored in SQLite, session_id bound to Spotify user
10. OxiTerm receives patch: {"is_authenticated": "true", "display_name": "..."}
11. Panel switches to player view
```

> [!IMPORTANT]
> The `state` parameter in the OAuth flow is a single-use, time-limited token (TTL 600 seconds). It is consumed via `pop()` on first use. A second `/callback` call with the same state returns `400 Bad Request`.

---

## Running

```bash
# 1. Copy and fill in credentials
cp spotify-app-server/.env.example spotify-app-server/.env
# Edit .env: SPOTIFY_CLIENT_ID, SPOTIFY_CLIENT_SECRET, SPOTIFY_REDIRECT_URI,
#            OXITERM_APP_TOKEN, OXITERM_URL, OXITERM_APP_SERVER

# 2. Start everything (OxiTerm + App Server in Docker)
./spotify-app-server/spotifycontrol.sh
```

The script:
- Configures `core.hooksPath .githooks` (gitleaks pre-commit hook).
- Starts the App Server via Docker Compose.
- Builds and starts the OxiTerm binary.

---

## Running Tests

Tests must run **inside Docker** — never directly on the host. Run from the repository root:

```bash
docker compose -f docker-compose.test.yml up --build test-spotify
```

The test image (`Dockerfile.test`) installs `pytest`, `httpx`, and `pytest-mock` on top of the production dependencies. Each test gets an isolated `tmp_path` SQLite database — no writes to the production `.cache/spotify_app.db`.

---

## Limitations

| Limitation | Detail |
|---|---|
| **SSH: `open:` not supported** | The `open:URL` action only works in web sessions. On SSH, the action is silently ignored. The user must copy the login URL from the `bind-state="auth_url"` field manually. |
| **Session persistence** | `active_oxiterm_sessions` (the in-memory `session_id` → token map) is **not persisted to disk**. Sessions expire after 300 seconds of inactivity. **Restarting the App Server requires all users to log in again.** |
| **Single-process** | The background polling thread runs in the same process as the FastAPI server. Under high load or many sessions, polling may slow down. |

---

## Repository Layout

```
spotify-app-server/
├── app.py                       # FastAPI App Server (OAuth, /events, /callback, polling lifecycle)
├── playback.py                  # Playback snapshot parser & deadline calculator
├── poller.py                    # Multi-session background polling manager
├── render.py                    # Reactive state patch generator (progress, volume, metadata)
├── spotify_api.py               # Spotify Web API client & PKCE flow
├── clock.py                     # Monotonic clock abstraction
├── test_app.py                  # pytest security contracts (tests 08–18)
├── spotifycontrol.sh            # Start script
├── Dockerfile                   # Production image (no test deps)
├── Dockerfile.test              # Test image (includes pytest/httpx)
├── docker-compose.yml           # Production compose
├── requirements.txt             # Production Python deps
├── requirements-test.txt        # Production + test deps
└── .env.example                 # Environment variable template

examples/spotify/
├── panel.thtml                  # OxiTerm UI — desktop/SSH layout (80x24)
└── panel_mobile.thtml           # OxiTerm UI — mobile layout (48x30)
```

---

## Modern UI & State Bindings (OxiTerm 0.6+)

The Spotify integration utilizes modern OxiTerm capabilities:

1. **Animated Progress Bar**:
   - `bind-width="progress_cols"`: Smoothly animated with `transition: width 1s linear;` on desktop (`progress_cols` range 0–62) and `bind-width="progress_cols_mob"` on mobile (range 0–32).
   - Dynamic track time markers via `bind-state="progress_cur"` (e.g. `01:23`) and `bind-state="progress_dur"` (e.g. `03:45`).
   - Backward-compatible `bind-state="progress_bar"` retained for standard text fallbacks.

2. **Floating Draggable Volume Mixer**:
   - Toggled via `event-htmx="toggle:show_vol"` and closed with `event-htmx="set:show_vol=false"`.
   - Utilizes `position: absolute; z-index: 20;` with `draggable="true"` and `drag-handle="true"`.
   - Features spring-physics volume animation: `transition: width 250ms spring(120, 12, 1);` driven by `bind-width="vol_cols"` (range 0–26).

3. **Safe Glyph Navigation**:
   - Arrow controls use ASCII `&lt;&lt;` and `&gt;&gt;` instead of ambiguous-width unicode symbols to prevent cell misalignment and ensure 100% linter compliance (`lint_layout.py --strict`).

---

## Technical Appendix: Episodes, Controls, and API Limitations (Plan 4.3)

### 1. Podcast & Episode Metadata
Playback requests include `additional_types=episode` across all `/v1/me/player` endpoints:
- **Episode Title**: mapped to `track_name` from `item.name`.
- **Show Name**: mapped to `artist_name` from `item.show.name`.
- **Release Date**: mapped to `album_name` from `item.release_date` (or empty string if absent). Note: `show.publisher` is deprecated by Spotify and is not used.

### 2. Audiobooks and Chapter Limitations
- **Catalog URIs**: `play_uri` supports `spotify:audiobook:<id>` (`context_uri`) and `spotify:chapter:<id>` (`uris`).
- **API Limitation**: The Spotify `/v1/me/player` Web API endpoint only supports `currently_playing_type` values `track`, `episode`, `ad`, and `unknown`. Chapters cannot be un-marshalled from playback state. When an unsupported type is active, `player_info` displays `"typ treści nieobsługiwany przez API Spotify"` while maintaining session authentication (`is_authenticated: true`).

### 3. Dynamic Player State Classification (`player_info`)
The `player_info` key carries user-facing notifications for specific playback states:
- **Inactive Device (HTTP 204)**: `"brak aktywnego urządzenia — dotknij telefonu"`
- **Restricted Device (`device.is_restricted`)**: `"urządzenie nie przyjmuje poleceń"`
- **Ad Playback (`currently_playing_type == "ad"`)**: `"reklama"`
- **Unsupported Type**: `"typ treści nieobsługiwany przez API Spotify"`
- **Normal Idle / Active Track**: `""` (empty string)

### 4. Dual `actions` Structure & Device Precedence
- **Disallows Resolution**: Checks `actions.disallows` first (where `true` means blocked), then flat `actions` boolean flags (where `true` means allowed), defaulting to `true` if `actions` is omitted.
- **Device Precedence**: If `device.is_restricted` is `true`, all controls (`can_next`, `can_prev`, `can_seek`, `can_volume`) are forced to `"false"` regardless of `actions`.
- **Volume Support**: If `device.supports_volume` is `false`, `can_volume` is set to `"false"`.

### 5. Rate Limit Backoff & Decision Notes
- **HTTP 429 Rate Limiting**: Managed per-session via the `Retry-After` header without populating `player_error`.
- **Scope Note (`resume_point`)**: `resume_position_ms` is omitted to avoid introducing `user-read-playback-position` scope, which would invalidate active user refresh tokens and force re-authentication.

