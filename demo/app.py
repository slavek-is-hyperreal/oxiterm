import os
import time
import secrets
import sqlite3
import logging
import asyncio
from typing import Dict, Any, Optional, Tuple
from urllib.parse import quote

import requests
from fastapi import FastAPI, Request, Response
from fastapi.responses import HTMLResponse
from pydantic import BaseModel
from dotenv import load_dotenv

# Load environment variables
load_dotenv(os.path.join(os.path.dirname(__file__), ".env"))

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("spotify_app_server")

app = FastAPI(title="OxiTerm Multi-Tenant Spotify App Server")

# Spotify Configuration
CLIENT_ID = os.getenv("SPOTIFY_CLIENT_ID", "a2cff4fceae146db8ded92dae9ed9ddd")
CLIENT_SECRET = os.getenv("SPOTIFY_CLIENT_SECRET", "")
REDIRECT_URI = os.getenv("SPOTIFY_REDIRECT_URI", "https://oxiterm.slavekm.pl/callback")
SCOPE = "user-read-playback-state user-modify-playback-state user-read-currently-playing playlist-read-private"

CACHE_DIR = os.path.join(os.path.dirname(__file__), ".cache")
os.makedirs(CACHE_DIR, exist_ok=True)
DB_PATH = os.path.join(CACHE_DIR, "spotify_app.db")

# In-memory map for transient OAuth state -> (session_id, timestamp)
pending_oauth_states: Dict[str, Tuple[int, float]] = {}
# Active session_id -> user_session_token mapping
active_oxiterm_sessions: Dict[int, Tuple[str, float]] = {}

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

init_db()

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

class OxiEventPayload(BaseModel):
    action: str
    state: Dict[str, Any] = {}
    session_id: int
    username: Optional[str] = None
    auth_method: Optional[str] = None

def render_progress_bar(progress_ms: int, duration_ms: int, width: int = 24) -> str:
    if not duration_ms or duration_ms <= 0:
        return "[------------------------] 00:00"
    pct = min(max(progress_ms / duration_ms, 0.0), 1.0)
    filled = int(round(pct * width))
    bar = "=" * max(filled - 1, 0) + (">" if filled > 0 else "")
    bar = bar.ljust(width, "-")
    
    prog_sec = progress_ms // 1000
    dur_sec = duration_ms // 1000
    prog_str = f"{prog_sec // 60:02d}:{prog_sec % 60:02d}"
    dur_str = f"{dur_sec // 60:02d}:{dur_sec % 60:02d}"
    return f"[{bar}] {prog_str} / {dur_str}"

def fetch_playback_for_user(access_token: str) -> Dict[str, str]:
    headers = {"Authorization": f"Bearer {access_token}"}
    try:
        r = requests.get("https://api.spotify.com/v1/me/player", headers=headers, timeout=3)
        if r.status_code == 200:
            pb = r.json()
            if pb and pb.get("item"):
                item = pb["item"]
                track_name = item.get("name", "Brak tytułu")
                artists = ", ".join([a["name"] for a in item.get("artists", [])])
                album_name = item.get("album", {}).get("name", "Album")
                device = pb.get("device", {}).get("name", "Brak urządzenia")
                is_playing = pb.get("is_playing", False)
                progress_ms = pb.get("progress_ms", 0)
                duration_ms = item.get("duration_ms", 1)
                volume = pb.get("device", {}).get("volume_percent", 50)
                
                return {
                    "is_authenticated": "true",
                    "track_name": track_name[:40],
                    "artist_name": artists[:35],
                    "album_name": album_name[:35],
                    "device_name": f"📱 {device}",
                    "is_playing": "true" if is_playing else "false",
                    "play_icon": "❚❚ Pause" if is_playing else "Play",
                    "progress_bar": render_progress_bar(progress_ms, duration_ms),
                    "volume": f"{volume}%"
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
                "progress_bar": "[------------------------] 00:00",
                "volume": "0%"
            }
    except Exception as e:
        logger.error(f"Error fetching playback for token: {e}")
    
    return {
        "is_authenticated": "true",
        "track_name": "Błąd pobierania odtwarzacza",
        "artist_name": "Sprawdź połączenie",
        "album_name": "-",
        "device_name": "-",
        "is_playing": "false",
        "play_icon": "Play",
        "progress_bar": "[------------------------] 00:00",
        "volume": "0%"
    }

@app.get("/callback")
async def spotify_callback(code: Optional[str] = None, state: Optional[str] = None, error: Optional[str] = None):
    if error or not code:
        return HTMLResponse(content=f"<h2>Błąd autoryzacji Spotify: {error}</h2>", status_code=400)
    
    session_id = None
    if state and state in pending_oauth_states:
        session_id, _ = pending_oauth_states.pop(state)

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
            return HTMLResponse(content=f"<h2>Błąd tokena Spotify: {r.text}</h2>", status_code=400)
        
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
        
        # Generate secure random 256-bit session token
        session_token = secrets.token_hex(32)
        
        with sqlite3.connect(DB_PATH) as conn:
            cursor = conn.cursor()
            cursor.execute("""
                INSERT INTO users (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(spotify_user_id) DO UPDATE SET
                    display_name=excluded.display_name,
                    access_token=excluded.access_token,
                    refresh_token=excluded.refresh_token,
                    expires_at=excluded.expires_at,
                    session_token=excluded.session_token,
                    last_seen=excluded.last_seen
            """, (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, time.time()))
            conn.commit()

        if session_id is not None:
            active_oxiterm_sessions[session_id] = (session_token, time.time())
        else:
            # Fallback: bind session_token to all currently active OxiTerm sessions
            for active_sid in list(active_oxiterm_sessions.keys()):
                active_oxiterm_sessions[active_sid] = (session_token, time.time())

        logger.info(f"Successfully authenticated Spotify user '{display_name}' (ID: {spotify_user_id})!")

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
                .btn {{ background: #1DB954; color: #000; text-decoration: none; padding: 0.8rem 1.5rem; border-radius: 50px; font-weight: bold; display: inline-block; margin-top: 1rem; }}
            </style>
        </head>
        <body>
            <div class="card">
                <h1>✅ Zalogowano pomyślnie!</h1>
                <p>Witaj w OxiTerm Spotify Control</p>
                <div class="badge">Zalogowano jako: {display_name}</div>
                <p>Możesz teraz zamknąć tę kartę i powrócić do konsoli OxiTerm.</p>
            </div>
        </body>
        </html>
        """
        return HTMLResponse(content=html_content)
    except Exception as e:
        logger.error(f"OAuth Callback error: {e}")
        return HTMLResponse(content=f"<h2>Błąd autoryzacji OAuth: {e}</h2>", status_code=400)

@app.on_event("startup")
async def start_background_loop():
    asyncio.create_task(poll_spotify_and_push_patches())

async def poll_spotify_and_push_patches():
    while True:
        await asyncio.sleep(1.5)
        now = time.time()
        stale = [sid for sid, (_, last_seen) in list(active_oxiterm_sessions.items()) if now - last_seen > 300]
        for sid in stale:
            active_oxiterm_sessions.pop(sid, None)

        if active_oxiterm_sessions:
            try:
                loop = asyncio.get_event_loop()
                oxiterm_url = os.getenv("OXITERM_URL", "http://host.docker.internal:8087")
                for sid, (stoken, _) in list(active_oxiterm_sessions.items()):
                    user = await loop.run_in_executor(None, lambda: get_user_by_session_token(stoken))
                    if user and user.get("access_token"):
                        patch = await loop.run_in_executor(None, lambda: fetch_playback_for_user(user["access_token"]))
                        patch["auth_status"] = f"Zalogowano: {user['display_name'][:20]}"
                        patch["user_session_token"] = stoken
                        url = f"{oxiterm_url}/sessions/{sid}/patch"
                        try:
                            r = await loop.run_in_executor(None, lambda: requests.post(url, json=patch, timeout=0.8))
                            if r.status_code == 404:
                                active_oxiterm_sessions.pop(sid, None)
                            elif r.status_code != 200:
                                logger.warning(f"Push patch to {url} returned status {r.status_code}")
                        except Exception as push_err:
                            logger.error(f"Push patch to {url} failed: {push_err}")
            except Exception as e:
                logger.error(f"Background polling error: {e}")

@app.post("/events")
async def handle_oxiterm_event(payload: OxiEventPayload):
    action = payload.action
    state_vars = payload.state
    session_id = payload.session_id
    
    stoken = state_vars.get("user_session_token", "")
    if not stoken and session_id in active_oxiterm_sessions:
        stoken, _ = active_oxiterm_sessions[session_id]
    user = get_user_by_session_token(stoken) if stoken else None
    
    if not user and action != "logout":
        try:
            with sqlite3.connect(DB_PATH) as conn:
                conn.row_factory = sqlite3.Row
                cursor = conn.cursor()
                cursor.execute("SELECT * FROM users ORDER BY last_seen DESC LIMIT 1")
                row = cursor.fetchone()
                if row:
                    user = dict(row)
                    stoken = user["session_token"]
                    active_oxiterm_sessions[session_id] = (stoken, time.time())
        except Exception as e:
            logger.error(f"Error checking default user fallback: {e}")
    
    if user:
        active_oxiterm_sessions[session_id] = (user["session_token"], time.time())
    
    patch = {}
    
    # 1. Action: trigger_login
    if action == "trigger_login":
        oauth_state = secrets.token_hex(16)
        pending_oauth_states[oauth_state] = (session_id, time.time())
        auth_url = f"https://accounts.spotify.com/authorize?client_id={CLIENT_ID}&response_type=code&redirect_uri={quote(REDIRECT_URI)}&scope={quote(SCOPE)}&state={oauth_state}&show_dialog=true"
        logger.info(f"Generated Spotify OAuth URL for session {session_id}: {auth_url}")
        patch["auth_msg"] = "Link autoryzacji wygenerowany!"
        patch["auth_url"] = auth_url

    # 2. Action: logout
    elif action == "logout":
        if stoken:
            delete_user_session(stoken)
            active_oxiterm_sessions.pop(session_id, None)
        patch["is_authenticated"] = "false"
        patch["auth_status"] = "Brak autoryzacji"
        patch["user_session_token"] = ""
        patch["track_name"] = "Wymagana autoryzacja"
        patch["artist_name"] = "Zaloguj się do Spotify"
        patch["album_name"] = "-"
        patch["device_name"] = "-"

    # 3. Action: set_tab
    elif action.startswith("set:tab=") or action.startswith("tab:"):
        tab_name = action.split("=", 1)[1] if "=" in action else action.split(":", 1)[1]
        patch["tab"] = tab_name
        if tab_name == "playlists" and user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                r = requests.get("https://api.spotify.com/v1/me/playlists?limit=6", headers=headers, timeout=3)
                if r.status_code == 200:
                    items = r.json().get("items", [])
                    for i in range(1, 7):
                        if i <= len(items):
                            item = items[i-1]
                            patch[f"pl_{i}_name"] = item.get("name", "")[:30]
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
                pb_req = requests.get("https://api.spotify.com/v1/me/player", headers=headers, timeout=3)
                if pb_req.status_code == 200:
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

    # 7. Action: play_uri:...
    elif action.startswith("play_uri:"):
        uri = action.split(":", 1)[1]
        if user and uri:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                payload_data = {"context_uri": uri} if ("playlist" in uri or "album" in uri) else {"uris": [uri]}
                requests.put("https://api.spotify.com/v1/me/player/play", json=payload_data, headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Play URI error: {e}")

    # 8. Action: vol_up / vol_down
    elif action == "vol_up":
        if user:
            try:
                headers = {"Authorization": f"Bearer {user['access_token']}"}
                pb_req = requests.get("https://api.spotify.com/v1/me/player", headers=headers, timeout=3)
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
                pb_req = requests.get("https://api.spotify.com/v1/me/player", headers=headers, timeout=3)
                if pb_req.status_code == 200:
                    pb = pb_req.json()
                    if pb and pb.get("device"):
                        cur_vol = pb["device"].get("volume_percent", 50)
                        new_vol = max(cur_vol - 10, 0)
                        requests.put(f"https://api.spotify.com/v1/me/player/volume?volume_percent={new_vol}", headers=headers, timeout=3)
            except Exception as e:
                logger.error(f"Vol down error: {e}")

    # Merge active playback state if user is logged in
    if user and action != "logout":
        playback_patch = fetch_playback_for_user(user["access_token"])
        playback_patch["auth_status"] = f"Zalogowano: {user['display_name'][:20]}"
        playback_patch["user_session_token"] = user["session_token"]
        patch.update(playback_patch)
    elif not user and action != "logout":
        auth_state = secrets.token_hex(16)
        pending_oauth_states[auth_state] = (session_id, time.time())
        auth_url = f"https://accounts.spotify.com/authorize?client_id={CLIENT_ID}&response_type=code&redirect_uri={quote(REDIRECT_URI)}&scope={quote(SCOPE)}&state={auth_state}&show_dialog=true"
        patch["is_authenticated"] = "false"
        patch["auth_status"] = "Brak autoryzacji"
        patch["auth_url"] = auth_url
        patch["user_session_token"] = ""

    return patch

if __name__ == "__main__":
    import uvicorn
    port = int(os.getenv("PORT", 8889))
    logger.info(f"Starting Multi-Tenant Spotify App Server on port {port}...")
    uvicorn.run(app, host="0.0.0.0", port=port)
