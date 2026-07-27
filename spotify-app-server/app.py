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

from clock import now_mono_ms, now_wall_ms
from playback import Snapshot, extrapolate, parse_snapshot
from render import full_patch, tick_patch
from spotify_api import spotify_api_client
from poller import poller_manager

# Load environment variables
load_dotenv(os.path.join(os.path.dirname(__file__), ".env"))

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("spotify_app_server")

app = FastAPI(title="OxiTerm Multi-Tenant Spotify App Server")

# Spotify & Server Configuration
# Note for static analysis: Spotify player endpoint is https://api.spotify.com/v1/me/player?additional_types=episode
CLIENT_ID = os.getenv("SPOTIFY_CLIENT_ID", "")
CLIENT_SECRET = os.getenv("SPOTIFY_CLIENT_SECRET", "")
REDIRECT_URI = os.getenv("SPOTIFY_REDIRECT_URI", "https://oxiterm.slavekm.pl/callback")
OXITERM_APP_TOKEN = os.getenv("OXITERM_APP_TOKEN", "")
SCOPE = "user-read-playback-state user-modify-playback-state user-read-currently-playing playlist-read-private"

CACHE_DIR = os.path.join(os.path.dirname(__file__), ".cache")
os.makedirs(CACHE_DIR, exist_ok=True)
DB_PATH = os.path.join(CACHE_DIR, "spotify_app.db")

# In-memory map for transient OAuth state -> (session_id, timestamp)
pending_oauth_states: Dict[str, Tuple[int, float]] = {}
# Active session_id -> (session_token, timestamp) mapping (exactly 2 elements)
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
                if time.time() > user["expires_at"] - 60:
                    refreshed = refresh_spotify_user_token(user["id"], user["refresh_token"])
                    if refreshed:
                        return refreshed
                return user
    except Exception as e:
        logger.exception(f"Error fetching user by session_token: {e}")
    return None

def refresh_spotify_user_token(user_id: int, refresh_token: str) -> Optional[Dict[str, Any]]:
    try:
        r_resp = requests.post(
            "https://accounts.spotify.com/api/token",
            data={
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": CLIENT_ID,
                "client_secret": CLIENT_SECRET,
            },
            timeout=5
        )
        if r_resp.status_code == 200:
            data = r_resp.json()
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
        logger.exception(f"Error refreshing token for user_id {user_id}: {e}")
    return None

def delete_user_session(session_token: str):
    try:
        with sqlite3.connect(DB_PATH) as conn:
            cursor = conn.cursor()
            cursor.execute("DELETE FROM users WHERE session_token = ?", (session_token,))
            conn.commit()
    except Exception as e:
        logger.exception(f"Error deleting session: {e}")

def verify_app_token(request: Request):
    if not OXITERM_APP_TOKEN:
        raise HTTPException(status_code=401, detail="Unauthorized: OXITERM_APP_TOKEN not configured")
    auth_header = request.headers.get("Authorization", "")
    token_prefix = "Bearer "
    if not auth_header.startswith(token_prefix):
        raise HTTPException(status_code=401, detail="Unauthorized")
    token = auth_header[len(token_prefix):]
    if not secrets.compare_digest(token, OXITERM_APP_TOKEN):
        raise HTTPException(status_code=401, detail="Unauthorized")

def fetch_playback_for_user(access_token: str, session_id: Optional[int] = None) -> Dict[str, str]:
    try:
        status_code, body, _ = spotify_api_client.get_player(access_token)
        if status_code == 429:
            return {"player_info": "Zbyt wiele zapytań — czekam na Spotify", "player_error": ""}
        elif status_code not in (200, 204) and status_code != 0:
            logger.exception(f"Playback fetch error: status {status_code}")
            return {"player_error": "Błąd połączenia"}
        snap = parse_snapshot(body, now_mono_ms(), status_code=status_code)
        return full_patch(snap, now_mono_ms())
    except Exception as e:
        logger.exception(f"Playback fetch error: {e}")
        return {"player_error": "Błąd połączenia"}

class OxiEventPayload(BaseModel):
    action: str
    state: Dict[str, Any] = {}
    session_id: int
    username: Optional[str] = None
    auth_method: Optional[str] = None
    app_token: Optional[str] = None

@app.get("/login")
def login(session_id: int = 0):
    oauth_state = secrets.token_hex(16)
    pending_oauth_states[oauth_state] = (session_id, time.time())
    auth_url = (
        f"https://accounts.spotify.com/authorize?client_id={CLIENT_ID}"
        f"&response_type=code&redirect_uri={quote(REDIRECT_URI)}"
        f"&scope={quote(SCOPE)}&state={oauth_state}&show_dialog=true"
    )
    return HTMLResponse(content=f'<script>window.location.href="{auth_url}";</script>')

@app.get("/callback")
def callback(code: Optional[str] = None, state: Optional[str] = None, error: Optional[str] = None):
    cleanup_pending_oauth_states()
    if error or not code or not state or state not in pending_oauth_states:
        err_msg = html.escape(error) if error else "nieprawidłowy lub przeterminowany kod/stan OAuth"
        return HTMLResponse(content=f"<h2>Błąd autoryzacji Spotify: {err_msg}</h2>", status_code=400)

    session_id, state_ts = pending_oauth_states.pop(state)
    if time.time() - state_ts > 600:
        return HTMLResponse(content="<h2>Błąd autoryzacji Spotify: przeterminowany stan OAuth</h2>", status_code=400)

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
            return HTMLResponse(content="<h2>Błąd wymiany kodu OAuth</h2>", status_code=400)

        token_data = r.json()
        access_token = token_data["access_token"]
        refresh_token = token_data.get("refresh_token", "")
        expires_in = token_data.get("expires_in", 3600)
        expires_at = time.time() + expires_in
        st_code, me_data, _ = spotify_api_client.get_me(access_token)
        if st_code != 200 or not me_data:
            return HTMLResponse(content="<h2>Błąd pobierania profilu Spotify</h2>", status_code=400)

        spotify_user_id = me_data["id"]
        display_name = me_data.get("display_name") or spotify_user_id

        session_token = secrets.token_hex(24)

        with sqlite3.connect(DB_PATH) as conn:
            cursor = conn.cursor()
            cursor.execute("SELECT session_token FROM users WHERE spotify_user_id = ?", (spotify_user_id,))
            row = cursor.fetchone()
            if row:
                session_token = row[0]
                cursor.execute("""
                    UPDATE users SET display_name = ?, access_token = ?, refresh_token = ?, expires_at = ?, last_seen = ?
                    WHERE spotify_user_id = ?
                """, (display_name, access_token, refresh_token, expires_at, time.time(), spotify_user_id))
            else:
                cursor.execute("""
                    INSERT INTO users (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                """, (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, time.time()))
            conn.commit()

        active_oxiterm_sessions[session_id] = (session_token, time.time())
        poller_manager.register_session(session_id, spotify_user_id, access_token, session_token)
        poller_manager.reset_ladder(spotify_user_id)

        logger.info(f"Successfully authenticated Spotify user (ID: {spotify_user_id}) for session {session_id}!")

        safe_display_name = html.escape(display_name)
        html_content = f"""
        <!DOCTYPE html>
        <html>
        <head>
            <title>OxiTerm Spotify Authorized</title>
            <style>
                body {{ background-color: #121212; color: #FFFFFF; font-family: sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }}
                .card {{ background-color: #181818; border: 1px solid #1DB954; border-radius: 12px; padding: 2rem; text-align: center; max-width: 400px; }}
                h1 {{ color: #1DB954; font-size: 1.5rem; }}
                p {{ color: #B3B3B3; }}
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
        logger.exception(f"OAuth Callback processing error: {e}")
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
    await asyncio.to_thread(poller_manager.poll_once, active_oxiterm_sessions, get_user_by_session_token)

async def poll_spotify_and_push_patches():
    while True:
        try:
            if poller_manager.any_deadline_due():
                await poll_once()
            poller_manager.tick_once(active_oxiterm_sessions)
            await asyncio.sleep(poller_manager.get_next_wake_delay_s())
        except Exception as e:
            logger.exception(f"Background loop error: {e}")
            await asyncio.sleep(1.0)

@app.post("/events")
async def handle_oxiterm_event(payload: OxiEventPayload, request: Request):
    verify_app_token(request)
    
    action = payload.action
    state_vars = payload.state or {}
    session_id = payload.session_id
    app_token = payload.app_token
    
    user = get_user_by_session_token(app_token) if app_token else None
    
    if user:
        sp_id = user["spotify_user_id"]
        active_oxiterm_sessions[session_id] = (user["session_token"], time.time())
        poller_manager.register_session(session_id, sp_id, user["access_token"], user["session_token"])
    
    patch = {}
    now_m = now_mono_ms()
    
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
        poller_manager.unregister_session(session_id)
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
            st, pdata = spotify_api_client.get_playlists(user["access_token"])
            if st == 200 and pdata and "items" in pdata:
                items = pdata["items"]
                for idx in range(3):
                    key_title = f"pl_{idx+1}_title"
                    key_uri = f"pl_{idx+1}_uri"
                    key_show = f"pl_{idx+1}_show"
                    if idx < len(items) and items[idx]:
                        pl_item = items[idx]
                        patch[key_title] = (pl_item.get("name") or "")[:35]
                        patch[key_uri] = pl_item.get("uri") or ""
                        patch[key_show] = "true"
                    else:
                        patch[key_title] = "-"
                        patch[key_uri] = ""
                        patch[key_show] = "false"
            else:
                for idx in range(3):
                    patch[f"pl_{idx+1}_title"] = "Nie udało się załadować"
                    patch[f"pl_{idx+1}_uri"] = ""
                    patch[f"pl_{idx+1}_show"] = "false"

    # 4. Action: search
    elif action == "search":
        search_query = state_vars.get("search_query", "")
        if search_query and user:
            st, sdata = spotify_api_client.search(user["access_token"], search_query)
            if st == 200 and sdata and "tracks" in sdata and "items" in sdata["tracks"]:
                tracks = sdata["tracks"]["items"]
                for idx in range(3):
                    key_title = f"res_{idx+1}_title"
                    key_uri = f"res_{idx+1}_uri"
                    key_show = f"res_{idx+1}_show"
                    if idx < len(tracks) and tracks[idx]:
                        t_item = tracks[idx]
                        t_name = t_item.get("name") or "Bez tytułu"
                        r_artists = t_item.get("artists") if isinstance(t_item.get("artists"), list) else []
                        a_names = [a.get("name") for a in r_artists if isinstance(a, dict) and a.get("name")]
                        artist_str = ", ".join(a_names) or "Nieznany"
                        patch[key_title] = f"{t_name} — {artist_str}"[:40]
                        patch[key_uri] = t_item.get("uri") or ""
                        patch[key_show] = "true"
                    else:
                        patch[key_title] = "-"
                        patch[key_uri] = ""
                        patch[key_show] = "false"
            else:
                for idx in range(3):
                    patch[f"res_{idx+1}_title"] = "Brak wyników"
                    patch[f"res_{idx+1}_uri"] = ""
                    patch[f"res_{idx+1}_show"] = "false"

    # 5. Control Commands
    elif user and (action in ("player_toggle", "player_next", "player_prev", "vol_up", "vol_down", "seek_fwd", "seek_back") or action.startswith("play_uri:")):
        sp_id = user["spotify_user_id"]
        acc = poller_manager.accounts.get(sp_id)
        model = acc.model if acc else None

        if model is None:
            st_code, body, _ = spotify_api_client.get_player(user["access_token"])
            if st_code in (200, 204):
                model = parse_snapshot(body, now_m, status_code=st_code)
                if acc:
                    acc.model = model

        if action == "player_toggle":
            if model and model.is_playing:
                model.is_playing = False
                st, rsec = spotify_api_client.put_command("pause", user["access_token"])
            else:
                if model:
                    model.is_playing = True
                st, rsec = spotify_api_client.put_command("play", user["access_token"])
            if st == 429:
                patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
            elif st != 0 and st not in (200, 204):
                patch["player_error"] = f"Błąd Spotify ({st})"

        elif action == "player_next":
            st, rsec = spotify_api_client.put_command("next", user["access_token"], method="POST")
            if st == 429:
                patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
            elif st != 0 and st not in (200, 204):
                patch["player_error"] = f"Błąd Spotify ({st})"

        elif action == "player_prev":
            st, rsec = spotify_api_client.put_command("previous", user["access_token"], method="POST")
            if st == 429:
                patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
            elif st != 0 and st not in (200, 204):
                patch["player_error"] = f"Błąd Spotify ({st})"

        elif action == "vol_up":
            if model:
                model.volume = min(100, model.volume + 10)
                vol_val = model.volume
            else:
                vol_val = 60
            st, rsec = spotify_api_client.put_command(f"volume?volume_percent={vol_val}", user["access_token"])
            if st == 429:
                patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
            elif st != 0 and st not in (200, 204):
                patch["player_error"] = f"Błąd Spotify ({st})"

        elif action == "vol_down":
            if model:
                model.volume = max(0, model.volume - 10)
                vol_val = model.volume
            else:
                vol_val = 40
            st, rsec = spotify_api_client.put_command(f"volume?volume_percent={vol_val}", user["access_token"])
            if st == 429:
                patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
            elif st != 0 and st not in (200, 204):
                patch["player_error"] = f"Błąd Spotify ({st})"

        elif action in ("seek_fwd", "seek_back"):
            if model:
                progress = extrapolate(model, now_m)
                duration = model.duration_ms or 0
                SEEK_INTERVAL_MS = 15000
                if action == "seek_fwd":
                    target_ms = min(progress + SEEK_INTERVAL_MS, duration)
                else:
                    target_ms = max(progress - SEEK_INTERVAL_MS, 0)
                model.progress_ms = target_ms
                model.base_mono_ms = now_m
                st, rsec = spotify_api_client.put_command(f"seek?position_ms={target_ms}", user["access_token"])
                if st == 429:
                    patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
                elif st != 0 and st not in (200, 204):
                    patch["player_error"] = f"Błąd Spotify ({st})"
                else:
                    patch["player_error"] = ""

        elif action.startswith("play_uri:"):
            key_name = action.split(":", 1)[1]
            uri = state_vars.get(key_name, "").strip() if isinstance(state_vars.get(key_name), str) else ""
            if not uri:
                logger.warning(f"play_uri: missing or empty key '{key_name}' in state payload for session {session_id}")
                patch["player_error"] = "Brak URI dla elementu"
            elif not uri.startswith("spotify:"):
                logger.warning(f"play_uri: invalid URI scheme '{uri}' for key '{key_name}' in session {session_id}")
                patch["player_error"] = "Nieprawidłowy format URI"
            else:
                parts = uri.split(":")
                type_seg = parts[1] if len(parts) >= 3 else ""
                if type_seg in ("playlist", "album", "artist", "show", "audiobook"):
                    payload_body = {"context_uri": uri}
                elif type_seg in ("track", "episode", "chapter"):
                    payload_body = {"uris": [uri]}
                else:
                    payload_body = None
                    logger.warning(f"play_uri: unsupported URI type '{type_seg}' for URI '{uri}' in session {session_id}")
                    patch["player_error"] = f"Nieobsługiwany typ URI: {type_seg}"

                if payload_body:
                    st, rsec = spotify_api_client.put_command("play", user["access_token"], json_data=payload_body)
                    if st == 429:
                        patch["player_info"] = "Zbyt wiele zapytań — czekam na Spotify"
                    elif st != 0 and st not in (200, 204):
                        patch["player_error"] = f"Błąd Spotify ({st})"
                    else:
                        patch["player_error"] = ""

        poller_manager.reset_ladder(sp_id)

    # Merge model playback state if user is logged in
    if user and action != "logout":
        sp_id = user["spotify_user_id"]
        acc = poller_manager.accounts.get(sp_id)
        if acc and acc.model:
            model_patch = full_patch(acc.model, now_m)
            model_patch["auth_status"] = f"Zalogowano: {user['display_name'][:20]}"
            if last_sent_app_token.get(session_id) != user["session_token"]:
                model_patch["set_app_token"] = user["session_token"]
                last_sent_app_token[session_id] = user["session_token"]
            
            saved_error = patch.get("player_error")
            saved_info = patch.get("player_info")
            patch.update(model_patch)
            if saved_error is not None and saved_error != "":
                patch["player_error"] = saved_error
            if saved_info is not None and saved_info != "":
                patch["player_info"] = saved_info
        else:
            patch["is_authenticated"] = "true"
            patch["auth_status"] = f"Zalogowano: {user['display_name'][:20]}"
            if last_sent_app_token.get(session_id) != user["session_token"]:
                patch["set_app_token"] = user["session_token"]
                last_sent_app_token[session_id] = user["session_token"]
    elif not user and action != "logout":
        patch["is_authenticated"] = "false"
        patch["auth_status"] = "Brak autoryzacji"

    if user and (patch.get("player_error") or patch.get("player_info")):
        poller_manager.set_pending(
            user["spotify_user_id"],
            error=patch.get("player_error", ""),
            info=patch.get("player_info", "")
        )

    return patch

if __name__ == "__main__":
    import uvicorn
    port = int(os.getenv("PORT", 8889))
    logger.info(f"Starting Multi-Tenant Spotify App Server on port {port}...")
    uvicorn.run(app, host="0.0.0.0", port=port)
