import os
import requests
import logging
from typing import Dict, Optional, Tuple, Any, List
from clock import now_mono_ms

logger = logging.getLogger("spotify_app_server")

def load_budget_config_from_env() -> Tuple[int, float]:
    def get_int(name: str, default: int) -> int:
        try:
            return int(os.getenv(name, str(default)))
        except (ValueError, TypeError):
            return default

    def get_float(name: str, default: float) -> float:
        try:
            return float(os.getenv(name, str(default)))
        except (ValueError, TypeError):
            return default

    return (
        get_int("SPOTIFY_BUDGET_PER_30S", 20),
        get_float("SPOTIFY_BREAKER_DEFAULT_S", 30.0)
    )

class SpotifyApiClient:
    def __init__(self, clock_fn=now_mono_ms):
        self._clock_fn = clock_fn
        self._history: List[int] = []
        self._breaker_until_mono_ms: int = 0

    def _get_budget(self) -> int:
        b, _ = load_budget_config_from_env()
        return b

    def _get_breaker_default(self) -> float:
        _, d = load_budget_config_from_env()
        return d

    def is_paused(self, now_mono_ms: int) -> bool:
        return now_mono_ms < self._breaker_until_mono_ms

    def _clean_history(self, now_mono_ms: int):
        cutoff = now_mono_ms - 30000
        self._history = [t for t in self._history if t > cutoff]

    def get_trailing_call_count(self, now_mono_ms: int) -> int:
        self._clean_history(now_mono_ms)
        return len(self._history)

    def get_me(self, access_token: str) -> Tuple[int, Optional[Dict[str, Any]], Optional[float]]:
        now = self._clock_fn()
        if self.is_paused(now):
            return (0, None, None)

        self._clean_history(now)
        if len(self._history) >= self._get_budget():
            logger.warning(f"Spotify API rate budget ({self._get_budget()}/30s) exhausted. Profile refused.")
            return (0, None, None)

        self._history.append(now)
        headers = {"Authorization": f"Bearer {access_token}"}
        url = "https://api.spotify.com/v1/me"
        try:
            r = requests.get(url, headers=headers, timeout=3)
            if r.status_code == 429:
                retry_hdr = r.headers.get("Retry-After")
                try:
                    retry_s = float(retry_hdr) if retry_hdr else self._get_breaker_default()
                except (ValueError, TypeError):
                    retry_s = self._get_breaker_default()
                self._breaker_until_mono_ms = now + int(retry_s * 1000)
                return (429, None, retry_s)
            elif r.status_code == 200:
                try:
                    return (200, r.json(), None)
                except Exception:
                    return (200, {}, None)
            else:
                return (r.status_code, None, None)
        except Exception as e:
            logger.exception(f"Spotify API get_me error: {e}")
            return (500, None, None)

    def get_player(self, access_token: str) -> Tuple[int, Optional[Dict[str, Any]], Optional[float]]:
        now = self._clock_fn()
        if self.is_paused(now):
            return (0, None, None)
        
        self._clean_history(now)
        if len(self._history) >= self._get_budget():
            logger.warning(f"Spotify API rate budget ({self._get_budget()}/30s) exhausted. Call refused.")
            return (0, None, None)

        self._history.append(now)
        headers = {"Authorization": f"Bearer {access_token}"}
        url = "https://api.spotify.com/v1/me/player?additional_types=episode"
        try:
            r = requests.get(url, headers=headers, timeout=3)
            if r.status_code == 429:
                retry_hdr = r.headers.get("Retry-After")
                try:
                    retry_s = float(retry_hdr) if retry_hdr else self._get_breaker_default()
                except (ValueError, TypeError):
                    retry_s = self._get_breaker_default()
                self._breaker_until_mono_ms = now + int(retry_s * 1000)
                logger.warning(f"Spotify API 429 Rate Limit. Global breaker paused for {retry_s}s")
                return (429, None, retry_s)
            elif r.status_code == 200:
                try:
                    return (200, r.json(), None)
                except Exception as json_err:
                    logger.exception(f"JSON decode error in get_player: {json_err}")
                    return (500, None, None)
            elif r.status_code == 204:
                return (204, None, None)
            else:
                return (r.status_code, None, None)
        except Exception as e:
            logger.exception(f"Spotify API HTTP request error: {e}")
            return (500, None, None)

    def put_command(self, endpoint: str, access_token: str, json_data: Optional[Dict] = None, method: str = "PUT") -> Tuple[int, Optional[float]]:
        now = self._clock_fn()
        if self.is_paused(now):
            return (0, None)

        self._clean_history(now)
        if len(self._history) >= self._get_budget():
            logger.warning(f"Spotify API rate budget ({self._get_budget()}/30s) exhausted. Command refused.")
            return (0, None)

        self._history.append(now)
        headers = {"Authorization": f"Bearer {access_token}"}
        url = f"https://api.spotify.com/v1/me/player/{endpoint}".rstrip('/')
        try:
            if method.upper() == "POST":
                r = requests.post(url, headers=headers, json=json_data, timeout=3)
            else:
                r = requests.put(url, headers=headers, json=json_data, timeout=3)

            if r.status_code == 429:
                retry_hdr = r.headers.get("Retry-After")
                try:
                    retry_s = float(retry_hdr) if retry_hdr else self._get_breaker_default()
                except (ValueError, TypeError):
                    retry_s = self._get_breaker_default()
                self._breaker_until_mono_ms = now + int(retry_s * 1000)
                logger.warning(f"Spotify API 429 Rate Limit on command. Global breaker paused for {retry_s}s")
                return (429, retry_s)
            return (r.status_code, None)
        except Exception as e:
            logger.exception(f"Spotify API Command error: {e}")
            return (500, None)

    def get_playlists(self, access_token: str, limit: int = 5) -> Tuple[int, Optional[Dict[str, Any]]]:
        now = self._clock_fn()
        if self.is_paused(now):
            return (0, None)

        self._clean_history(now)
        if len(self._history) >= self._get_budget():
            logger.warning(f"Spotify API rate budget ({self._get_budget()}/30s) exhausted. Playlists refused.")
            return (0, None)

        self._history.append(now)
        headers = {"Authorization": f"Bearer {access_token}"}
        url = f"https://api.spotify.com/v1/me/playlists?limit={limit}"
        try:
            r = requests.get(url, headers=headers, timeout=3)
            if r.status_code == 200:
                return (200, r.json())
            return (r.status_code, None)
        except Exception as e:
            logger.exception(f"Spotify API Playlists error: {e}")
            return (500, None)

    def search(self, access_token: str, query: str, limit: int = 3) -> Tuple[int, Optional[Dict[str, Any]]]:
        from urllib.parse import quote
        now = self._clock_fn()
        if self.is_paused(now):
            return (0, None)

        self._clean_history(now)
        if len(self._history) >= self._get_budget():
            logger.warning(f"Spotify API rate budget ({self._get_budget()}/30s) exhausted. Search refused.")
            return (0, None)

        self._history.append(now)
        headers = {"Authorization": f"Bearer {access_token}"}
        url = f"https://api.spotify.com/v1/search?q={quote(query)}&type=track&limit={limit}"
        try:
            r = requests.get(url, headers=headers, timeout=3)
            if r.status_code == 200:
                return (200, r.json())
            return (r.status_code, None)
        except Exception as e:
            logger.exception(f"Spotify API Search error: {e}")
            return (500, None)

# Default global instance
spotify_api_client = SpotifyApiClient()
