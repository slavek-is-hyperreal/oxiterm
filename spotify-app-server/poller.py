import os
import time
import requests
import logging
from dataclasses import dataclass
from typing import Dict, Set, Tuple, Optional, Any, List
from clock import now_mono_ms, now_wall_ms
from playback import Snapshot, PollGaps, parse_snapshot, extrapolate, next_deadline, next_second_boundary_mono_ms
from render import full_patch, tick_patch
from spotify_api import spotify_api_client

logger = logging.getLogger("spotify_app_server")

def load_poll_gaps_from_env() -> PollGaps:
    def get_float(name: str, default: float) -> float:
        try:
            return float(os.getenv(name, str(default)))
        except (ValueError, TypeError):
            return default

    def get_int(name: str, default: int) -> int:
        try:
            return int(os.getenv(name, str(default)))
        except (ValueError, TypeError):
            return default

    ladder_raw = os.getenv("POLL_LADDER_S", "0.4,1,2,3,5,8,12,20")
    try:
        ladder_tuple = tuple(float(x.strip()) for x in ladder_raw.split(",") if x.strip())
        if not ladder_tuple:
            ladder_tuple = (0.4, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0)
    except Exception:
        ladder_tuple = (0.4, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0)

    return PollGaps(
        playing_s=get_float("POLL_PLAYING_S", 25.0),
        paused_s=get_float("POLL_PAUSED_S", 60.0),
        idle_s=get_float("POLL_IDLE_S", 90.0),
        end_grace_ms=get_int("POLL_END_GRACE_MS", 800),
        min_gap_s=get_float("POLL_MIN_GAP_S", 1.0),
        ladder_s=ladder_tuple
    )

def load_panel_pages_from_env() -> List[str]:
    raw = os.getenv("SPOTIFY_PANEL_PAGES", "spotify/panel.thtml,spotify/panel_mobile.thtml")
    return [p.strip() for p in raw.split(",") if p.strip()]

@dataclass
class AccountState:
    spotify_user_id: str
    access_token: str
    session_token: str
    model: Optional[Snapshot] = None
    ladder_cursor: Optional[int] = None
    next_deadline_mono_ms: int = 0

class PollerManager:
    def __init__(self, clock_fn=now_mono_ms, wall_clock_fn=now_wall_ms, api_client=spotify_api_client):
        self._clock_fn = clock_fn
        self._wall_clock_fn = wall_clock_fn
        self._api = api_client
        self.gaps = load_poll_gaps_from_env()
        self.panel_pages = load_panel_pages_from_env()
        self.accounts: Dict[str, AccountState] = {}
        self.watched_sessions: Dict[int, str] = {}  # sid -> spotify_user_id
        self.unwatched_sessions: Set[int] = set()
        self.last_sent_progress: Dict[int, str] = {}
        self.last_sent_app_token: Dict[int, str] = {}

    def is_page_watched(self, page_str: Optional[str]) -> bool:
        if not isinstance(page_str, str):
            return True
        for target in self.panel_pages:
            if page_str.endswith(target):
                return True
        return False

    def reset_ladder(self, spotify_user_id: str):
        account = self.accounts.get(spotify_user_id)
        if account:
            account.ladder_cursor = 0
            gap_s = self.gaps.ladder_s[0]
            eff_gap_s = max(gap_s, self.gaps.min_gap_s)
            account.next_deadline_mono_ms = self._clock_fn() + int(eff_gap_s * 1000)

    def register_session(self, sid: int, spotify_user_id: str, access_token: str, session_token: str):
        if spotify_user_id not in self.accounts:
            self.accounts[spotify_user_id] = AccountState(
                spotify_user_id=spotify_user_id,
                access_token=access_token,
                session_token=session_token,
                next_deadline_mono_ms=0
            )
        else:
            acc = self.accounts[spotify_user_id]
            acc.access_token = access_token
            acc.session_token = session_token

        if sid not in self.unwatched_sessions:
            self.watched_sessions[sid] = spotify_user_id

    def unregister_session(self, sid: int):
        self.watched_sessions.pop(sid, None)
        self.unwatched_sessions.discard(sid)
        self.last_sent_progress.pop(sid, None)
        self.last_sent_app_token.pop(sid, None)

    def poll_account(self, spotify_user_id: str, active_oxiterm_sessions: Dict[int, Tuple[str, float]]):
        now = self._clock_fn()
        account = self.accounts.get(spotify_user_id)
        if not account:
            return

        status_code, body, retry_after_s = self._api.get_player(account.access_token)

        if status_code == 0:
            return

        if status_code == 429:
            retry_s = retry_after_s if retry_after_s is not None else 30.0
            account.next_deadline_mono_ms = now + int(retry_s * 1000)
            if account.model:
                account.model.poll_state = "BLOCKED"
            return

        new_snap = parse_snapshot(body, now, status_code=status_code)
        
        old_snap = account.model
        if old_snap is not None and ((old_snap.spotify_timestamp != new_snap.spotify_timestamp) or (old_snap.item_uri != new_snap.item_uri)):
            account.ladder_cursor = 0

        account.model = new_snap
        dl_ms, next_cursor = next_deadline(new_snap, now, account.ladder_cursor, self.gaps)
        account.next_deadline_mono_ms = dl_ms
        account.ladder_cursor = next_cursor

    def push_to_session(self, sid: int, patch: Dict[str, str], active_oxiterm_sessions: Dict[int, Tuple[str, float]]) -> bool:
        oxiterm_url = os.getenv("OXITERM_URL", "http://host.docker.internal:8087")
        app_token_hdr = os.getenv("OXITERM_APP_TOKEN", "")
        headers = {}
        if app_token_hdr:
            headers["Authorization"] = f"Bearer {app_token_hdr}"

        url = f"{oxiterm_url}/sessions/{sid}/patch"
        try:
            r = requests.post(url, json=patch, headers=headers, timeout=0.8)
            if r.status_code == 404:
                active_oxiterm_sessions.pop(sid, None)
                self.unregister_session(sid)
                return False
            elif r.status_code == 200:
                try:
                    resp_json = r.json()
                    if isinstance(resp_json, dict) and "page" in resp_json:
                        page_val = resp_json.get("page")
                        if isinstance(page_val, str):
                            if not self.is_page_watched(page_val):
                                self.unwatched_sessions.add(sid)
                                self.watched_sessions.pop(sid, None)
                            else:
                                self.unwatched_sessions.discard(sid)
                                sp_id = self.watched_sessions.get(sid)
                                if not sp_id and sid in active_oxiterm_sessions:
                                    stoken = active_oxiterm_sessions[sid][0]
                                    acc = next((a for a in self.accounts.values() if a.session_token == stoken), None)
                                    if acc:
                                        self.watched_sessions[sid] = acc.spotify_user_id
                except Exception:
                    pass

                # Update active_oxiterm_sessions wall timestamp (2-element tuple)
                stoken = active_oxiterm_sessions.get(sid, ("", 0.0))[0]
                if not stoken and sid in self.watched_sessions:
                    acc = self.accounts.get(self.watched_sessions[sid])
                    if acc:
                        stoken = acc.session_token
                active_oxiterm_sessions[sid] = (stoken, time.time())
                return True
            else:
                logger.warning(f"Push patch to session {sid} returned status {r.status_code}")
                return True
        except Exception as push_err:
            logger.error(f"Push patch to session {sid} failed: {push_err}")
            return True

    def poll_once(self, active_oxiterm_sessions: Dict[int, Tuple[str, float]], get_user_by_stoken_fn=None):
        now = self._clock_fn()

        # Update watcher state from active_oxiterm_sessions
        for sid, sess_entry in list(active_oxiterm_sessions.items()):
            stoken = sess_entry[0]
            if get_user_by_stoken_fn:
                user = get_user_by_stoken_fn(stoken)
                if user and user.get("spotify_user_id"):
                    sp_id = user["spotify_user_id"]
                    self.register_session(sid, sp_id, user["access_token"], user["session_token"])

        # Determine which accounts need fetching
        accounts_to_fetch = set()
        for sid, sp_id in list(self.watched_sessions.items()):
            if sid not in active_oxiterm_sessions:
                self.unregister_session(sid)
                continue
            acc = self.accounts.get(sp_id)
            if acc and now >= acc.next_deadline_mono_ms:
                accounts_to_fetch.add(sp_id)

        for sp_id in accounts_to_fetch:
            self.poll_account(sp_id, active_oxiterm_sessions)

        # Log budget usage per cycle
        used_calls = self._api.get_trailing_call_count(now)
        logger.info(f"Spotify API budget: {used_calls}/30s calls used in trailing 30s")

        # Push full patch to EVERY watched session of EVERY account holding a model
        for sid, sp_id in list(self.watched_sessions.items()):
            acc = self.accounts.get(sp_id)
            if acc and acc.model:
                f_patch = full_patch(acc.model, now)
                f_patch["auth_status"] = f"Zalogowano: {sp_id[:20]}"
                if self.last_sent_app_token.get(sid) != acc.session_token:
                    f_patch["set_app_token"] = acc.session_token
                    self.last_sent_app_token[sid] = acc.session_token

                if self.push_to_session(sid, f_patch, active_oxiterm_sessions):
                    self.last_sent_progress[sid] = f_patch["progress_bar"]

    def tick_once(self, active_oxiterm_sessions: Dict[int, Tuple[str, float]]):
        now = self._clock_fn()
        for sid, sp_id in list(self.watched_sessions.items()):
            if sid not in active_oxiterm_sessions:
                continue
            acc = self.accounts.get(sp_id)
            if acc and acc.model and acc.model.is_playing:
                t_patch = tick_patch(acc.model, now)
                prog_str = t_patch["progress_bar"]
                if self.last_sent_progress.get(sid) != prog_str:
                    if self.push_to_session(sid, t_patch, active_oxiterm_sessions):
                        self.last_sent_progress[sid] = prog_str

    def get_next_tick_wake_delay_s(self) -> float:
        now = self._clock_fn()
        min_boundary = None
        for acc in self.accounts.values():
            if acc.model and acc.model.is_playing:
                b = next_second_boundary_mono_ms(acc.model, now)
                if b is not None:
                    if min_boundary is None or b < min_boundary:
                        min_boundary = b

        if min_boundary is not None:
            delay_ms = max(10, min_boundary - now)
            return delay_ms / 1000.0
        
        try:
            return float(os.getenv("TICK_FALLBACK_S", "1.0"))
        except Exception:
            return 1.0

# Default global instance
poller_manager = PollerManager()
