# Spotify Control Center — Demo App

`spotify-app-server/` is a complete example application built on top of the OxiTerm engine. It demonstrates how to connect an external App Server (Python/FastAPI) to OxiTerm to produce a real-world, multi-user TUI with OAuth 2.0 authentication, live state patching, and a panel that works on SSH, web, and mobile simultaneously.

---

## What the Demo Shows

| Feature | Description |
|---|---|
| **OAuth 2.0 Authorization Code Flow** | User initiates login from within OxiTerm; the App Server handles the redirect, token exchange, and securely binds the Spotify token to the session. |
| **Multi-session isolation** | Each OxiTerm session (`session_id`) has its own Spotify login. Sessions do not share tokens; there is no fallback to another user's data. |
| **Push patches from the background** | A background task polls Spotify's Now Playing API and pushes track title, artist, progress, and duration back to the active session via `POST /sessions/{id}/patch`. |
| **Web + SSH + Mobile panels** | The same App Server drives `spotify_panel.thtml` (web/SSH) and `spotify_panel_mobile.thtml` (mobile, `<800 px`). |
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
  │  spotify_panel.thtml                   │
  │  spotify_panel_mobile.thtml            │
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

Tests must run **inside Docker** — never directly on the host:

```bash
cd spotify-app-server/

docker build -f Dockerfile.test -t spotify-app-test .

docker run --rm \
  --env-file .env \
  -e OXITERM_APP_TOKEN=<your-token> \
  spotify-app-test
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
├── app.py                       # FastAPI App Server (OAuth, /events, /callback)
├── test_app.py                  # pytest security contracts (tests 08–18)
├── spotify_panel.thtml          # OxiTerm UI — web/SSH layout
├── spotify_panel_mobile.thtml   # OxiTerm UI — mobile layout (< 800 px)
├── spotifycontrol.sh            # Start script
├── Dockerfile                   # Production image (no test deps)
├── Dockerfile.test              # Test image (includes pytest/httpx)
├── docker-compose.yml           # Production compose
├── requirements.txt             # Production Python deps
├── requirements-test.txt        # Production + test deps
└── .env.example                 # Environment variable template
```

> [!NOTE]
> `examples/` contains isolated single-feature engine demos (SVG, Lottie, Rive widget, input fields). `spotify-app-server/` is a complete end-to-end application demonstrating what can be built on top of the engine.
