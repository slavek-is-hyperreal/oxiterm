import os
import time
import html
import secrets
import sqlite3
import logging
import asyncio
from typing import Dict, Any, Optional, Tuple
from urllib.parse import quote

import requests
from fastapi import FastAPI, Request, Response, HTTPException
from fastapi.responses import HTMLResponse
from pydantic import BaseModel
from dotenv import load_dotenv

# Load environment variables
load_dotenv(os.path.join(os.path.dirname(__file__), ".env"))

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("spotify_app_server")

app = FastAPI(title="OxiTerm Multi-Tenant Spotify App Server")

# Spotify & Server Configuration
CLIENT_ID = os.getenv("SPOTIFY_CLIENT_ID", "a2cff4fceae146db8ded92dae9ed9ddd")
CLIENT_SECRET = os.getenv("SPOTIFY_CLIENT_SECRET", "")
REDIRECT_URI = os.getenv("SPOTIFY_REDIRECT_URI", "https://oxiterm.slavekm.pl/callback")
OXITERM_APP_TOKEN = os.getenv("OXITERM_APP_TOKEN", "")
SCOPE = "user-read-playback-state user-modify-playback-state user-read-currently-playing playlist-read-private"

CACHE_DIR = os.path.join(os.path.dirname(__file__), ".cache")
os.makedirs(CACHE_DIR, exist_ok=True)
DB_PATH = os.path.join(CACHE_DIR, "spotify_app.db")

# In-memory map for transient OAuth state -> (session_id, timestamp)
pending_oauth_states: Dict[str, Tuple[int, float]] = {}
# Active session_id -> (session_token, timestamp) mapping
active_oxiterm_sessions: Dict[int, Tuple[str, float]] = {}
# Active session_id -> last_sent_app_token mapping
last_sent_app_token: Dict[int, str] = {}

def init_db():
    with sqlite3.connect(DB_PATH) as conn:
        cursor = conn.cursor()
        cursor.execute("""
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                spotify_user_id TEXT UNIQUE NOT NULL,
                display_name TEXT,
                access_token TEXT NOT NULL,
                refresh_token TEXT NOT NULL,
                expires_at REAL NOT NULL,
                session_token TEXT UNIQUE NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                last_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        """)
        conn.commit()
    try:
        os.chmod(DB_PATH, 0o600)
    except Exception:
        pass

init_db()

def cleanup_pending_oauth_states():
    now = time.time()
    stale = [st for st, (_, ts) in list(pending_oauth_states.items()) if now - ts > 600]
    for st in stale:
        pending_oauth_states.pop(st, None)

def get_user_by_session_token(session_token: str) -> Optional[Dict[str, Any]]:
    if not session_token:
        return None
    try:
        with sqlite3.connect(DB_PATH) as conn:
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()
            cursor.execute("SELECT * FROM users WHERE session_token = ?", (session_token,))
            row = cursor.fetchone()
            if row:
                user = dict(row)
                # Check if token needs refresh (expires within 60s)
                if time.time() > user["expires_at"] - 60:
                    refreshed = refresh_spotify_user_token(user["id"], user["refresh_token"])
                    if refreshed:
                        return refreshed
                return user
    except Exception as e:
        logger.error(f"Error fetching user by session_token: {e}")
    return None

def refresh_spotify_user_token(user_id: int, refresh_token: str) -> Optional[Dict[str, Any]]:
    try:
        r = requests.post(
            "https://accounts.spotify.com/api/token",
            data={
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": CLIENT_ID,
                "client_secret": CLIENT_SECRET,
            },
            timeout=5
        )
        if r.status_code == 200:
            data = r.json()
            new_access_token = data.get("access_token")
            new_expires_at = time.time() + data.get("expires_in", 3600)
            new_refresh_token = data.get("refresh_token", refresh_token)
            with sqlite3.connect(DB_PATH) as conn:
                conn.row_factory = sqlite3.Row
                cursor = conn.cursor()
                cursor.execute("""
                    UPDATE users SET access_token = ?, refresh_token = ?, expires_at = ?, last_seen = ?
                    WHERE id = ?
                """, (new_access_token, new_refresh_token, new_expires_at, time.time(), user_id))
                conn.commit()
                cursor.execute("SELECT * FROM users WHERE id = ?", (user_id,))
                return dict(cursor.fetchone())
    except Exception as e:
        logger.error(f"Error refreshing token for user_id {user_id}: {e}")
    return None

def delete_user_session(session_token: str):
    try:
        with sqlite3.connect(DB_PATH) as conn:
            cursor = conn.cursor()
            cursor.execute("DELETE FROM users WHERE session_token = ?", (session_token,))
            conn.commit()
    except Exception as e:
        logger.error(f"Error deleting session: {e}")

def verify_app_token(request: Request):
    if not OXITERM_APP_TOKEN:
        # Fail-closed: If token is unset or empty, /events endpoint is disabled (401)
        raise HTTPException(status_code=401, detail="Unauthorized: OXITERM_APP_TOKEN not configured")
    auth_header = request.headers.get("Authorization", "")
    token_prefix = "Bearer "
    if not auth_header.startswith(token_prefix):
        raise HTTPException(status_code=401, detail="Unauthorized")
    token = auth_header[len(token_prefix):]
    if not secrets.compare_digest(token, OXITERM_APP_TOKEN):
        raise HTTPException(status_code=401, detail="Unauthorized")

class OxiEventPayload(BaseModel):
    action: str
    state: Dict[str, Any] = {}
    session_id: int
    username: Optional[str] = None
    auth_method: Optional[str] = None
    app_token: Optional[str] = None

def render_progress_bar(progress_ms: int, duration_ms: int, width: int = 48) -> str:
    if not duration_ms or duration_ms <= 0:
        return "[" + "-" * width + "] 00:00 / 00:00"
    pct = min(max(progress_ms / duration_ms, 0.0), 1.0)
    filled = int(round(pct * width))
    bar = "=" * max(filled - 1, 0) + (">" if filled > 0 else "")
    bar = bar.ljust(width, "-")
    
    prog_sec = progress_ms // 1000
    dur_sec = duration_ms // 1000
    prog_str = f"{prog_sec // 60:02d}:{prog_sec % 60:02d}"
    dur_str = f"{dur_sec // 60:02d}:{dur_sec % 60:02d}"
    return f"[{bar}] {prog_str} / {dur_str}"

def fetch_playback_for_user(access_token: str, session_id: Optional[int] = None) -> Dict[str, str]:
    headers = {"Authorization": f"Bearer {access_token}"}
    try:
        r = requests.get("https://api.spotify.com/v1/me/player?additional_types=episode", headers=headers, timeout=3)
        if r.status_code == 429:
            retry_hdr = r.headers.get("Retry-After")
            try:
                retry_sec = int(retry_hdr) if retry_hdr else 10
            except (ValueError, TypeError):
                retry_sec = 10
            logger.warning(f"Spotify API 429 Rate Limit encountered. Backoff {retry_sec}s for session {session_id}")
            if session_id and session_id in active_oxiterm_sessions:
                entry = active_oxiterm_sessions[session_id]
                active_oxiterm_sessions[session_id] = (entry[0], entry[1], time.time() + retry_sec)
            return {
                "is_authenticated": "true",
                "player_info": "Zbyt wiele zapytań — czekam na Spotify",
                "player_error": ""
            }

        if r.status_code == 200:
            pb = r.json()
            if not pb:
                return {
                    "is_authenticated": "true",
                    "track_name": "Brak aktywnego odtwarzacza",
                    "artist_name": "Włącz muzykę na telefonie/PC",
                    "album_name": "Spotify Connect",
                    "device_name": "Brak aktywnego urządzenia",
                    "is_playing": "false",
                    "play_icon": "Play",
                    "progress_bar": render_progress_bar(0, 0),
                    "volume": "0%",
                    "can_next": "true",
                    "can_prev": "true",
                    "can_seek": "true",
                    "can_volume": "true",
                    "device_restricted": "false",
                    "player_info": "brak aktywnego urządzenia — dotknij telefonu",
                    "player_error": ""
                }

            item = pb.get("item") if isinstance(pb.get("item"), dict) else None
            item_type = item.get("type") if item else None
            curr_type = pb.get("currently_playing_type")

            # 1. Device resolution (K-10, K-11)
            device_obj = pb.get("device") if isinstance(pb.get("device"), dict) else {}
            device_name = device_obj.get("name") or "Brak urządzenia"
            is_restricted = device_obj.get("is_restricted") is True
            supports_vol = device_obj.get("supports_volume") is not False

            # 2. Actions resolution (G-1: DisallowsObject polarity, true = forbidden)
            actions = pb.get("actions") if isinstance(pb.get("actions"), dict) else {}
            src = actions.get("disallows") if "disallows" in actions and isinstance(actions.get("disallows"), dict) else actions

            if src:
                can_next = "false" if src.get("skipping_next", False) else "true"
                can_prev = "false" if src.get("skipping_prev", False) else "true"
                can_seek = "false" if src.get("seeking", False) else "true"
            else:
                can_next = "true"
                can_prev = "true"
                can_seek = "true"

            if is_restricted:
                can_next = "false"
                can_prev = "false"
                can_seek = "false"
                can_volume = "false"
                device_restricted = "true"
                player_info = "urządzenie nie przyjmuje poleceń"
            else:
                device_restricted = "false"
                can_volume = "true" if supports_vol else "false"
                player_info = ""

            # 3. Item & Player Info classification (K-9, K-12, K-15, K-17, K-18)
            if item and item_type == "episode":
                track_name = item.get("name") or "Brak tytułu"
                show_obj = item.get("show") if isinstance(item.get("show"), dict) else {}
                artists = show_obj.get("name") or "Podcast"
                album_name = item.get("release_date") or show_obj.get("publisher") or ""
            elif item and item_type == "track":
                track_name = item.get("name") or "Brak tytułu"
                raw_artists = item.get("artists") if isinstance(item.get("artists"), list) else []
                artists_list = [str(a.get("name")) for a in raw_artists if isinstance(a, dict) and a.get("name") is not None]
                artists = ", ".join(artists_list) or "Nieznany wykonawca"
                album_obj = item.get("album") if isinstance(item.get("album"), dict) else {}
                album_name = album_obj.get("name") or "Album"
            elif item:
                track_name = "Treść nieobsługiwana"
                artists = "-"
                album_name = "-"
                if not is_restricted:
                    player_info = "typ treści nieobsługiwany przez API Spotify"
            else: # item is null
                if curr_type == "ad":
                    track_name = "Reklama"
                    artists = "-"
                    album_name = "-"
                    if not is_restricted:
                        player_info = "reklama"
                elif curr_type and curr_type not in ("track", "episode"):
                    track_name = "Nieobsługiwany typ"
                    artists = "-"
                    album_name = "-"
                    if not is_restricted:
                        player_info = "typ treści nieobsługiwany przez API Spotify"
                else:
                    track_name = "Brak odtwarzania"
                    artists = "-"
                    album_name = "-"
                    if not is_restricted:
                        player_info = ""

            is_playing = pb.get("is_playing", False)
            progress_ms = pb.get("progress_ms", 0) or 0
            duration_ms = item.get("duration_ms", 1) if item else 1
            volume = device_obj.get("volume_percent") if isinstance(device_obj.get("volume_percent"), int) else 50

            return {
                "is_authenticated": "true",
                "track_name": str(track_name)[:35] if track_name is not None else "",
                "artist_name": str(artists)[:35] if artists is not None else "",
                "album_name": str(album_name)[:35] if album_name is not None else "",
                "device_name": f"📱 {str(device_name)[:35]}" if device_name is not None else "📱 Brak urządzenia",
                "is_playing": "true" if is_playing else "false",
                "play_icon": "❚❚ Pause" if is_playing else "Play",
                "progress_bar": render_progress_bar(progress_ms, duration_ms),
                "volume": f"{volume}%",
                "can_next": can_next,
                "can_prev": can_prev,
                "can_seek": can_seek,
                "can_volume": can_volume,
                "device_restricted": device_restricted,
                "player_info": player_info,
                "player_error": ""
            }
        elif r.status_code == 204:
            return {
                "is_authenticated": "true",
                "track_name": "Brak aktywnego odtwarzacza",
                "artist_name": "Włącz muzykę na telefonie/PC",
                "album_name": "Spotify Connect",
                "device_name": "Brak aktywnego urządzenia",
                "is_playing": "false",
                "play_icon": "Play",
                "progress_bar": render_progress_bar(0, 0),
                "volume": "0%",
                "can_next": "true",
                "can_prev": "true",
                "can_seek": "true",
                "can_volume": "true",
                "device_restricted": "false",
                "player_info": "brak aktywnego urządzenia — dotknij telefonu",
                "player_error": ""
            }
        else:
            logger.error(f"Error fetching playback for token: status {r.status_code}, body: {r.text[:100]}")
            return {
                "is_authenticated": "true",
                "track_name": "Błąd pobierania odtwarzacza",
                "artist_name": "Sprawdź połączenie",
                "album_name": "-",
                "device_name": "-",
                "is_playing": "false",
                "play_icon": "Play",
                "progress_bar": render_progress_bar(0, 0),
                "volume": "0%",
                "can_next": "true",
                "can_prev": "true",
                "can_seek": "true",
                "can_volume": "true",
                "device_restricted": "false",
                "player_info": "",
                "player_error": f"Błąd Spotify ({r.status_code})"
            }
    except Exception as e:
        logger.exception(f"Error fetching playback for token: {e}")

    return {
        "is_authenticated": "true",
        "track_name": "Błąd pobierania odtwarzacza",
        "artist_name": "Sprawdź połączenie",
        "album_name": "-",
        "device_name": "-",
        "is_playing": "false",
        "play_icon": "Play",
        "progress_bar": render_progress_bar(0, 0),
        "volume": "0%",
        "can_next": "true",
        "can_prev": "true",
        "can_seek": "true",
        "can_volume": "true",
        "device_restricted": "false",
        "player_info": "",
        "player_error": "Błąd połączenia"
    }

@app.get("/callback")
async def spotify_callback(code: Optional[str] = None, state: Optional[str] = None, error: Optional[str] = None):
    if error or not code:
        safe_error = html.escape(error) if error else "Brak kodu autoryzacji"
        return HTMLResponse(content=f"<h2>Błąd autoryzacji Spotify: {safe_error}</h2>", status_code=400)
    
    # Strict state verification (CSRF protection)
    if not state or state not in pending_oauth_states:
        return HTMLResponse(content="<h2>Błąd autoryzacji: nieprawidłowy lub przeterminowany token state</h2>", status_code=400)
    
    session_id, state_ts = pending_oauth_states.pop(state)
    if time.time() - state_ts > 600:
        return HTMLResponse(content="<h2>Błąd autoryzacji: przeterminowany token state (powyżej 10 minut)</h2>", status_code=400)

    try:
        r = requests.post(
            "https://accounts.spotify.com/api/token",
            data={
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": REDIRECT_URI,
                "client_id": CLIENT_ID,
                "client_secret": CLIENT_SECRET,
            },
            timeout=5
        )
        if r.status_code != 200:
            return HTMLResponse(content="<h2>Błąd wymiany tokena Spotify</h2>", status_code=400)
        
        token_data = r.json()
        access_token = token_data["access_token"]
        refresh_token = token_data["refresh_token"]
        expires_in = token_data["expires_in"]
        expires_at = time.time() + expires_in
        
        # Fetch user profile /v1/me
        me_req = requests.get("https://api.spotify.com/v1/me", headers={"Authorization": f"Bearer {access_token}"}, timeout=5)
        if me_req.status_code != 200:
            return HTMLResponse(content="<h2>Błąd pobierania profilu Spotify</h2>", status_code=400)
        
        me = me_req.json()
        spotify_user_id = me.get("id", "unknown")
        display_name = me.get("display_name") or spotify_user_id
        safe_display_name = html.escape(display_name)
        
        # Generate secure random 256-bit session token for new user, or reuse existing
        session_token = secrets.token_hex(32)
        
        with sqlite3.connect(DB_PATH) as conn:
            cursor = conn.cursor()
            cursor.execute("SELECT session_token FROM users WHERE spotify_user_id = ?", (spotify_user_id,))
            row = cursor.fetchone()
            if row and row[0]:
                session_token = row[0]

            cursor.execute("""
                INSERT INTO users (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(spotify_user_id) DO UPDATE SET
                    display_name=excluded.display_name,
                    access_token=excluded.access_token,
                    refresh_token=excluded.refresh_token,
                    expires_at=excluded.expires_at,
                    last_seen=excluded.last_seen
            """, (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, time.time()))
            conn.commit()

        # Bind session_token strictly to session_id from state
        active_oxiterm_sessions[session_id] = (session_token, time.time())
        logger.info(f"Successfully authenticated Spotify user (ID: {spotify_user_id}) for session {session_id}!")

        html_content = f"""
        <!DOCTYPE html>
        <html>
        <head>
            <meta charset="utf-8">
            <title>Autoryzacja Spotify Zakończona</title>
            <style>
                body {{ font-family: system-ui, -apple-system, sans-serif; background: #121212; color: #fff; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
                .card {{ background: #181818; border: 1px solid #282828; padding: 2rem 3rem; border-radius: 12px; text-align: center; max-width: 480px; box-shadow: 0 8px 24px rgba(0,0,0,0.5); }}
                h1 {{ color: #1DB954; font-size: 1.8rem; margin-bottom: 0.5rem; }}
                p {{ color: #b3b3b3; line-height: 1.5; }}
                .badge {{ background: #282828; color: #1DB954; padding: 0.4rem 0.8rem; border-radius: 6px; font-weight: bold; display: inline-block; margin: 1rem 0; }}
            </style>
        </head>
        <body>
            <div class="card">
                <h1>✅ Zalogowano pomyślnie!</h1>
                <p>Witaj w OxiTerm Spotify Control</p>
                <div class="badge">Zalogowano jako: {safe_display_name}</div>
                <p>Możesz teraz zamknąć tę kartę i powrócić do konsoli OxiTerm.</p>
            </div>
        </body>
        </html>
        """
        return HTMLResponse(content=html_content)
    except Exception as e:
        logger.error(f"OAuth Callback processing error: {e}")
        return HTMLResponse(content="<h2>Błąd autoryzacji OAuth</h2>", status_code=400)

def generate_auth_url(session_id: int) -> str:
    oauth_state = secrets.token_hex(16)
    pending_oauth_states[oauth_state] = (session_id, time.time())
    return f"https://accounts.spotify.com/authorize?client_id={CLIENT_ID}&response_type=code&redirect_uri={quote(REDIRECT_URI)}&scope={quote(SCOPE)}&state={oauth_state}&show_dialog=true"

@app.on_event("startup")
async def start_background_loop():
    asyncio.create_task(poll_spotify_and_push_patches())

async def poll_once():
    cleanup_pending_oauth_states()
    if active_oxiterm_sessions:
        try:
            loop = asyncio.get_event_loop()
            oxiterm_url = os.getenv("OXITERM_URL", "http://host.docker.internal:8087")
            headers = {}
            if OXITERM_APP_TOKEN:
                headers["Authorization"] = f"Bearer {OXITERM_APP_TOKEN}"
            for sid, sess_entry in list(active_oxiterm_sessions.items()):
                stoken = sess_entry[0]
                backoff_until = sess_entry[2] if len(sess_entry) >= 3 else 0.0
                if time.time() < backoff_until:
                    continue
                user = await loop.run_in_executor(None, lambda st=stoken: get_user_by_session_token(st))
                if user and user.get("access_token"):
                    patch = await loop.run_in_executor(None, lambda tok=user["access_token"], s=sid: fetch_playback_for_user(tok, session_id=s))
                    patch["auth_status"] = f"Zalogowano: {user['display_name'][:20]}"
                    if last_sent_app_token.get(sid) != user["session_token"]:
                        patch["set_app_token"] = user["session_token"]
                        last_sent_app_token[sid] = user["session_token"]
                    url = f"{oxiterm_url}/sessions/{sid}/patch"
                    try:
                        r = await loop.run_in_executor(None, lambda u=url, p=patch, h=headers: requests.post(u, json=p, headers=h, timeout=0.8))
                        if r.status_code == 404:
                            active_oxiterm_sessions.pop(sid, None)
                            last_sent_app_token.pop(sid, None)
                        elif r.status_code == 200:
                            entry = active_oxiterm_sessions.get(sid)
                            if entry and len(entry) >= 3 and entry[2] > time.time():
                                active_oxiterm_sessions[sid] = (stoken, time.time(), entry[2])
                            else:
                                active_oxiterm_sessions[sid] = (stoken, time.time())
                        else:
                            logger.warning(f"Push patch to session {sid} returned status {r.status_code}")
                    except Exception as push_err:
                        logger.error(f"Push patch to session {sid} failed: {push_err}")
        except Exception as e:
            logger.error(f"Background polling error: {e}")

async def poll_spotify_and_push_patches():
    while True:
        await asyncio.sleep(1.5)
        await poll_once()


@app.post("/events")
async def handle_oxiterm_event(payload: OxiEventPayload, request: Request):
    verify_app_token(request)
    
    action = payload.action
    state_vars = payload.state
    session_id = payload.session_id
    app_token = payload.app_token
    
    user = get_user_by_session_token(app_token) if app_token else None
    
    if user:
        entry = active_oxiterm_sessions.get(session_id)
        if entry and len(entry) >= 3 and entry[2] > time.time():
            active_oxiterm_sessions[session_id] = (user["session_token"], time.time(), entry[2])
        else:
            active_oxiterm_sessions[session_id] = (user["session_token"], time.time())
    
    patch = {}
    
    # 1. Action: trigger_login
    if action == "trigger_login":
        auth_url = generate_auth_url(session_id)
        patch["auth_url"] = auth_url

    # 2. Action: trigger_open
    elif action == "trigger_open":
        auth_url = generate_auth_url(session_id)
        patch["open_url"] = auth_url

    # 2b. Action: logout
    elif action == "logout":
        if app_token:
            delete_user_session(app_token)
        active_oxiterm_sessions.pop(session_id, None)
        last_sent_app_token.pop(session_id, None)
        patch["set_app_token"] = ""
        patch["is_authenticated"] = "false"
        patch["auth_status"] = "Brak autoryzacji"
        patch["track_name"] = "Wymagana autoryzacja"
        patch["artist_name"] = "Zaloguj się do Spotify"
        patch["album_name"] = "-"
        patch["device_name"] = "-"

    # 3. Action: set:tab=...
    elif action.startswith("set:tab="):
        tab_name = action.split("=", 1)[1]
        patch["tab"] = tab_name
        if tab_name == "playlists" and user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                r = requests.get("https://api.spotify.com/v1/me/playlists?limit=5", headers=headers, timeout=3)
                if r.status_code == 200:
                    playlists = r.json().get("items", [])
                    patch["playlists_count"] = str(len(playlists))
                    for i in range(1, 6):
                        if i <= len(playlists):
                            item = playlists[i-1]
                            patch[f"pl_{i}_name"] = item.get("name", "")[:25]
                            patch[f"pl_{i}_uri"] = item.get("uri", "")
                            patch[f"pl_{i}_show"] = "true"
                        else:
                            patch[f"pl_{i}_show"] = "false"
            except Exception as e:
                logger.error(f"Error loading playlists: {e}")

    # 4. Action: search
    elif action == "search":
        query = state_vars.get("search_query", "").strip()
        if query and user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                r = requests.get(f"https://api.spotify.com/v1/search?q={quote(query)}&type=track&limit=5", headers=headers, timeout=3)
                if r.status_code == 200:
                    tracks = r.json().get("tracks", {}).get("items", [])
                    patch["search_results_count"] = str(len(tracks))
                    for i in range(1, 6):
                        if i <= len(tracks):
                            t = tracks[i-1]
                            t_name = t.get("name", "")[:28]
                            t_artist = ", ".join([a["name"] for a in t.get("artists", [])])[:22]
                            t_uri = t.get("uri", "")
                            patch[f"res_{i}_title"] = f"{t_name} — {t_artist}"
                            patch[f"res_{i}_uri"] = t_uri
                            patch[f"res_{i}_show"] = "true"
                        else:
                            patch[f"res_{i}_show"] = "false"
            except Exception as e:
                logger.error(f"Search error: {e}")
                patch["search_error"] = str(e)[:40]

    # 5. Action: player_toggle
    elif action == "player_toggle":
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                pb_req = requests.get("https://api.spotify.com/v1/me/player?additional_types=episode", headers=headers, timeout=3)
                if 200 <= pb_req.status_code < 300:
                    pb = pb_req.json()
                    if pb and pb.get("is_playing"):
                        requests.put("https://api.spotify.com/v1/me/player/pause", headers=headers, timeout=3)
                    else:
                        requests.put("https://api.spotify.com/v1/me/player/play", headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Player toggle error: {e}")

    # 6. Action: player_next / player_prev
    elif action == "player_next":
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                requests.post("https://api.spotify.com/v1/me/player/next", headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Player next error: {e}")

    elif action == "player_prev":
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                requests.post("https://api.spotify.com/v1/me/player/previous", headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Player prev error: {e}")

    # 7. Action: play_uri:<key_name>
    elif action.startswith("play_uri:"):
        key_name = action.split(":", 1)[1]
        uri = state_vars.get(key_name, "").strip() if state_vars else ""
        if not uri:
            logger.warning(f"play_uri: missing or empty key '{key_name}' in state payload for session {session_id}")
            patch["player_error"] = "Brak URI dla elementu"
        elif not uri.startswith("spotify:"):
            logger.warning(f"play_uri: invalid URI scheme '{uri}' for key '{key_name}' in session {session_id}")
            patch["player_error"] = "Nieprawidłowy format URI"
        else:
            parts = uri.split(":")
            if len(parts) >= 3 and parts[0] == "spotify":
                type_seg = parts[1]
                if type_seg in ("playlist", "album", "show", "artist", "audiobook"):
                    payload_data = {"context_uri": uri}
                elif type_seg in ("track", "episode", "chapter"):
                    payload_data = {"uris": [uri]}
                else:
                    logger.warning(f"play_uri: unsupported URI type '{type_seg}' for URI '{uri}' in session {session_id}")
                    patch["player_error"] = f"Nieobsługiwany typ URI: {type_seg}"
                    payload_data = None
            else:
                patch["player_error"] = "Nieprawidłowa struktura URI"
                payload_data = None

            if payload_data and user:
                try:
                    headers = {"Authorization": f"Bearer {user['access_token']}"}
                    r = requests.put("https://api.spotify.com/v1/me/player/play", json=payload_data, headers=headers, timeout=3)
                    if 200 <= r.status_code < 300:
                        patch["player_error"] = ""
                    else:
                        logger.error(f"Spotify play API error {r.status_code}: {r.text[:100]}")
                        patch["player_error"] = f"Błąd Spotify ({r.status_code})"
                except Exception as e:
                    logger.error(f"Play URI error: {e}")
                    patch["player_error"] = "Błąd połączenia ze Spotify"

    # 8. Action: vol_up / vol_down
    elif action == "vol_up":
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                pb_req = requests.get("https://api.spotify.com/v1/me/player?additional_types=episode", headers=headers, timeout=3)
                if pb_req.status_code == 200:
                    pb = pb_req.json()
                    if pb and pb.get("device"):
                        cur_vol = pb["device"].get("volume_percent", 50)
                        new_vol = min(cur_vol + 10, 100)
                        requests.put(f"https://api.spotify.com/v1/me/player/volume?volume_percent={new_vol}", headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Vol up error: {e}")

    elif action == "vol_down":
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                pb_req = requests.get("https://api.spotify.com/v1/me/player?additional_types=episode", headers=headers, timeout=3)
                if pb_req.status_code == 200:
                    pb = pb_req.json()
                    if pb and pb.get("device"):
                        cur_vol = pb["device"].get("volume_percent", 50)
                        new_vol = max(cur_vol - 10, 0)
                        requests.put(f"https://api.spotify.com/v1/me/player/volume?volume_percent={new_vol}", headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Vol down error: {e}")

    # 9. Action: seek_fwd / seek_back
    elif action in ("seek_fwd", "seek_back"):
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                pb_req = requests.get("https://api.spotify.com/v1/me/player?additional_types=episode", headers=headers, timeout=3)
                if 200 <= pb_req.status_code < 300:
                    pb = pb_req.json()
                    if pb and pb.get("item"):
                        progress = pb.get("progress_ms", 0) or 0
                        duration = pb["item"].get("duration_ms", 0) or 0
                        SEEK_INTERVAL_MS = 15000
                        if action == "seek_fwd":
                            target_ms = min(progress + SEEK_INTERVAL_MS, duration)
                        else:
                            target_ms = max(progress - SEEK_INTERVAL_MS, 0)
                        s_req = requests.put(f"https://api.spotify.com/v1/me/player/seek?position_ms={target_ms}", headers=headers, timeout=3)
                        if 200 <= s_req.status_code < 300:
                            patch["player_error"] = ""
                        else:
                            patch["player_error"] = f"Błąd Spotify ({s_req.status_code})"
                else:
                    patch["player_error"] = f"Błąd Spotify ({pb_req.status_code})"
            except Exception as e:
                logger.error(f"Seek error: {e}")
                patch["player_error"] = "Błąd połączenia ze Spotify"

    # Merge active playback state if user is logged in
    if user and action != "logout":
        saved_error = patch.get("player_error")
        playback_patch = fetch_playback_for_user(user["access_token"], session_id=session_id)
        playback_patch["auth_status"] = f"Zalogowano: {user['display_name'][:20]}"
        patch.update(playback_patch)
        if saved_error is not None and saved_error != "":
            patch["player_error"] = saved_error
    elif not user and action != "logout":
        patch["is_authenticated"] = "false"
        patch["auth_status"] = "Brak autoryzacji"

    return patch

if __name__ == "__main__":
    import uvicorn
    port = int(os.getenv("PORT", 8889))
    logger.info(f"Starting Multi-Tenant Spotify App Server on port {port}...")
    uvicorn.run(app, host="0.0.0.0", port=port)
