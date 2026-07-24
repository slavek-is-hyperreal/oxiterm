import os
import sys
import logging
from typing import Dict, Any, Optional
from fastapi import FastAPI, Request, Response, status
from pydantic import BaseModel
from dotenv import load_dotenv
import spotipy
from spotipy.oauth2 import SpotifyOAuth

# Load environment variables
load_dotenv(os.path.join(os.path.dirname(__file__), ".env"))

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("spotify_app_server")

app = FastAPI(title="OxiTerm Spotify App Server")

# Spotify Configuration
CLIENT_ID = os.getenv("SPOTIFY_CLIENT_ID", "a2cff4fceae146db8ded92dae9ed9ddd")
CLIENT_SECRET = os.getenv("SPOTIFY_CLIENT_SECRET", "ea0a951bc68b4d5baa0813570b94eca4")
REDIRECT_URI = os.getenv("SPOTIFY_REDIRECT_URI", "https://oxiterm.slavekm.pl/callback")
SCOPE = "user-read-playback-state user-modify-playback-state user-read-currently-playing playlist-read-private"

CACHE_DIR = os.path.join(os.path.dirname(__file__), ".cache")
os.makedirs(CACHE_DIR, exist_ok=True)
CACHE_PATH = os.path.join(CACHE_DIR, "spotify_token.json")

def get_spotify_oauth() -> SpotifyOAuth:
    return SpotifyOAuth(
        client_id=CLIENT_ID,
        client_secret=CLIENT_SECRET,
        redirect_uri=REDIRECT_URI,
        scope=SCOPE,
        cache_path=CACHE_PATH,
        open_browser=False
    )

def get_spotify_client() -> Optional[spotipy.Spotify]:
    auth_manager = get_spotify_oauth()
    token_info = auth_manager.get_cached_token()
    if token_info:
        return spotipy.Spotify(auth_manager=auth_manager)
    return None

class OxiEventPayload(BaseModel):
    action: str
    state: Dict[str, str] = {}
    session_id: int
    username: Optional[str] = None

def render_progress_bar(progress_ms: int, duration_ms: int, length: int = 24) -> str:
    if not duration_ms or duration_ms == 0:
        return "[------------------------] 00:00 / 00:00"
    
    ratio = min(max(progress_ms / duration_ms, 0.0), 1.0)
    filled = int(ratio * length)
    bar = "=" * filled + (">" if filled < length else "=") + " " * (length - filled - 1)
    
    prog_sec = progress_ms // 1000
    dur_sec = duration_ms // 1000
    
    prog_str = f"{prog_sec // 60:02d}:{prog_sec % 60:02d}"
    dur_str = f"{dur_sec // 60:02d}:{dur_sec % 60:02d}"
    
    return f"[{bar[:length]}] {prog_str} / {dur_str}"

def fetch_playback_state_patch() -> Dict[str, str]:
    sp = get_spotify_client()
    auth_manager = get_spotify_oauth()
    auth_url = auth_manager.get_authorize_url()

    if not sp:
        return {
            "auth_status": "Brak autoryzacji",
            "is_authenticated": "false",
            "auth_url": auth_url,
            "auth_msg": "Kliknij lub skopiuj link autoryzacyjny Spotify:",
            "track_name": "Wymagane logowanie Spotify",
            "artist_name": "Otwórz link autoryzacyjny z zakładek",
            "album_name": "Aplikacja: SSHMusicControl",
            "progress_bar": "[------------------------] 00:00 / 00:00",
            "play_icon": "Play",
            "device_name": "Brak połączenia"
        }
    
    try:
        pb = sp.current_playback()
        if pb and pb.get("item"):
            item = pb["item"]
            track_name = item.get("name", "Brak tytułu")
            artists = ", ".join([a["name"] for a in item.get("artists", [])])
            album_name = item.get("album", {}).get("name", "")
            is_playing = pb.get("is_playing", False)
            progress_ms = pb.get("progress_ms", 0)
            duration_ms = item.get("duration_ms", 1)
            device = pb.get("device", {}).get("name", "Spotify Player")
            volume = pb.get("device", {}).get("volume_percent", 50)
            
            return {
                "is_authenticated": "true",
                "auth_status": "Zalogowano",
                "auth_url": auth_url,
                "track_name": track_name[:40],
                "artist_name": artists[:35],
                "album_name": album_name[:35],
                "device_name": f"📱 {device}",
                "is_playing": "true" if is_playing else "false",
                "play_icon": "❚❚ Pause" if is_playing else "Play",
                "progress_bar": render_progress_bar(progress_ms, duration_ms),
                "volume": f"{volume}%"
            }
        else:
            return {
                "is_authenticated": "true",
                "auth_status": "Zalogowano",
                "auth_url": auth_url,
                "track_name": "Brak aktywnego odtwarzania",
                "artist_name": "Włącz Spotify na urządzeniu",
                "album_name": "Wybierz utwór z aplikacji Spotify",
                "progress_bar": "[------------------------] 00:00 / 00:00",
                "play_icon": "Play",
                "device_name": "Czekam na urządzenie Spotify..."
            }
    except Exception as e:
        logger.error(f"Error fetching playback state: {e}")
        return {
            "is_authenticated": "false",
            "auth_url": auth_url,
            "auth_status": f"Błąd API: {str(e)[:30]}"
        }

@app.get("/callback")
async def spotify_callback(code: str, state: Optional[str] = None):
    """Spotify OAuth 2.0 Redirect Callback Endpoint"""
    auth_manager = get_spotify_oauth()
    try:
        token_info = auth_manager.get_access_token(code)
        logger.info(f"Successfully authenticated and saved token to {CACHE_PATH}!")
        return Response(
            content="""
            <!DOCTYPE html>
            <html>
                <head>
                    <title>Spotify Auth Success</title>
                    <meta charset="utf-8">
                </head>
                <body style="font-family: system-ui, sans-serif; text-align: center; padding-top: 60px; background: #121212; color: #1DB954;">
                    <h1>✅ Logowanie do Spotify zakończone sukcesem!</h1>
                    <p style="color: #ffffff; font-size: 1.2rem;">Token autoryzacyjny został pomyślnie zapisany.</p>
                    <p style="color: #b3b3b3; font-size: 1rem;">Możesz zamknąć tę kartę i przejść do swojego terminala OxiTerm.</p>
                </body>
            </html>
            """,
            media_type="text/html"
        )
    except Exception as e:
        logger.error(f"OAuth Callback error: {e}")
        return Response(content=f"Błąd autoryzacji OAuth: {e}", status_code=400)

@app.post("/events")
async def handle_oxiterm_event(payload: OxiEventPayload):
    action = payload.action
    state_vars = payload.state
    session_id = payload.session_id
    
    logger.info(f"Session {session_id} action: '{action}'")
    patch = {}
    sp = get_spotify_client()
    auth_manager = get_spotify_oauth()
    auth_url = auth_manager.get_authorize_url()

    # 1. Action: trigger_login
    if action == "trigger_login":
        logger.info(f"Generated Spotify OAuth URL: {auth_url}")
        patch["auth_msg"] = "Link autoryzacji wygenerowany!"
        patch["auth_url"] = auth_url

    # 2. Action: set_tab
    elif action.startswith("tab:"):
        tab_name = action.split(":", 1)[1]
        patch["tab"] = tab_name
        if tab_name == "playlists" and sp:
            try:
                results = sp.current_user_playlists(limit=6)
                items = results.get("items", [])
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

    # 3. Action: search
    elif action == "search":
        query = state_vars.get("search_query", "").strip()
        if query and sp:
            try:
                res = sp.search(q=query, limit=5, type="track")
                tracks = res.get("tracks", {}).get("items", [])
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

    # 4. Action: player_toggle
    elif action == "player_toggle":
        if sp:
            try:
                pb = sp.current_playback()
                if pb and pb.get("is_playing"):
                    sp.pause_playback()
                else:
                    sp.start_playback()
            except Exception as e:
                logger.error(f"Player toggle error: {e}")

    # 5. Action: player_next
    elif action == "player_next":
        if sp:
            try:
                sp.next_track()
            except Exception as e:
                logger.error(f"Player next error: {e}")

    # 6. Action: player_prev
    elif action == "player_prev":
        if sp:
            try:
                sp.previous_track()
            except Exception as e:
                logger.error(f"Player prev error: {e}")

    # 7. Action: play_uri
    elif action.startswith("play_uri:"):
        uri = action.split(":", 1)[1]
        if sp and uri:
            try:
                if "playlist" in uri or "album" in uri:
                    sp.start_playback(context_uri=uri)
                else:
                    sp.start_playback(uris=[uri])
            except Exception as e:
                logger.error(f"Play URI error: {e}")

    # 8. Action: vol_up / vol_down
    elif action == "vol_up":
        if sp:
            try:
                pb = sp.current_playback()
                if pb and pb.get("device"):
                    cur_vol = pb["device"].get("volume_percent", 50)
                    new_vol = min(cur_vol + 10, 100)
                    sp.volume(new_vol)
            except Exception as e:
                logger.error(f"Vol up error: {e}")

    elif action == "vol_down":
        if sp:
            try:
                pb = sp.current_playback()
                if pb and pb.get("device"):
                    cur_vol = pb["device"].get("volume_percent", 50)
                    new_vol = max(cur_vol - 10, 0)
                    sp.volume(new_vol)
            except Exception as e:
                logger.error(f"Vol down error: {e}")

    # Always merge active playback state
    patch.update(fetch_playback_state_patch())
    
    return patch

if __name__ == "__main__":
    import uvicorn
    port = int(os.getenv("PORT", 8889))
    logger.info(f"Starting Spotify App Server on port {port}...")
    uvicorn.run(app, host="0.0.0.0", port=port)
