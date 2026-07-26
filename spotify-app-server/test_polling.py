import time
import pytest
from unittest.mock import patch, MagicMock
from clock import FakeClock
from spotify_api import SpotifyApiClient
from poller import PollerManager, AccountState
from playback import parse_snapshot

def test_18_budget_exhaustion():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 200
        mock_resp.json.return_value = {}
        mock_get.return_value = mock_resp

        # Consume 20 budget calls
        for _ in range(20):
            st, _, _ = api.get_player("token_123")
            assert st == 200

        assert mock_get.call_count == 20

        # 21st call must be refused (returns 0, requests.get not called)
        st21, _, _ = api.get_player("token_123")
        assert st21 == 0
        assert mock_get.call_count == 20

def test_19_budget_resets_after_31s():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 200
        mock_resp.json.return_value = {}
        mock_get.return_value = mock_resp

        for _ in range(20):
            api.get_player("token_123")

        assert api.get_player("token_123")[0] == 0

        # Advance clock 31 seconds (31000 ms)
        clock.advance_mono(31000)
        assert api.get_player("token_123")[0] == 200

def test_20_429_retry_after_verbatim():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 429
        mock_resp.headers = {"Retry-After": "4500"}
        mock_get.return_value = mock_resp

        st, _, retry_s = api.get_player("token_123")
        assert st == 429
        assert retry_s == 4500.0
        assert api.is_paused(clock.now_mono_ms()) is True

        # Clock advanced 4499s -> still paused
        clock.advance_mono(4499000)
        assert api.is_paused(clock.now_mono_ms()) is True

        # Clock advanced 2s more -> unpaused
        clock.advance_mono(2000)
        assert api.is_paused(clock.now_mono_ms()) is False

def test_21_429_default_breaker():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 429
        mock_resp.headers = {}
        mock_get.return_value = mock_resp

        st, _, retry_s = api.get_player("token_123")
        assert st == 429
        assert retry_s == 30.0
        assert api.is_paused(clock.now_mono_ms()) is True

def test_22_global_pause_refuses_all_calls():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 429
        mock_resp.headers = {"Retry-After": "60"}
        mock_get.return_value = mock_resp

        api.get_player("user1_tok")

    with patch("requests.get") as mock_get_user2:
        st, _, _ = api.get_player("user2_tok")
        assert st == 0
        assert mock_get_user2.call_count == 0

def test_23_two_sessions_one_user_one_get_two_pushes():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    
    active_sessions = {
        10: ("stoken_u1", time.time()),
        11: ("stoken_u1", time.time())
    }
    
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        poller.poll_once(active_sessions, get_user_fn)

        assert mock_get.call_count == 1
        assert mock_post.call_count == 2

def test_24_poll_twice_in_playing_window_one_get():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    poller.gaps = poller.gaps.__class__(ladder_s=())  # Exhausted ladder

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1

        # Poll again immediately -> inside POLL_PLAYING_S window, no GET
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1
        assert mock_post.call_count == 2

def test_25_tick_once_playing_pushes_only_progress_bar():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    acc = AccountState("u1", "acc1", "stok1")
    acc.model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())
    poller.accounts["u1"] = acc
    poller.watched_sessions[10] = "u1"
    poller.last_sent_progress[10] = "[=-------] 00:10 / 03:00"

    active_sessions = {10: ("stok1", time.time())}

    clock.advance_mono(1000)
    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        poller.tick_once(active_sessions)
        assert mock_post.call_count == 1
        sent_json = mock_post.call_args[1]["json"]
        assert list(sent_json.keys()) == ["progress_bar"]

def test_26_tick_once_paused_deduplicated():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    acc = AccountState("u1", "acc1", "stok1")
    acc.model = parse_snapshot({
        "is_playing": False, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())
    poller.accounts["u1"] = acc
    poller.watched_sessions[10] = "u1"

    active_sessions = {10: ("stok1", time.time())}

    clock.advance_mono(1000)
    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        poller.tick_once(active_sessions)
        assert mock_post.call_count == 0

def test_27_unwatched_page_stops_get():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "features.thtml"}  # Not panel!

        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1

        # Session was marked unwatched -> next poll_once performs 0 GET
        clock.advance_mono(30000)
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1

def test_28_watched_page_continues_get():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    poller.gaps = poller.gaps.__class__(ladder_s=())

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1

        clock.advance_mono(26000)
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 2

def test_29_non_dict_push_body_counts_as_watched():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200
        mock_post.return_value.text = "OK"  # Non-dict string

        poller.poll_once(active_sessions, get_user_fn)
        assert 10 in poller.watched_sessions

def test_30_push_404_removes_session_leaves_tuple_2_elements():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 404

        poller.poll_once(active_sessions, get_user_fn)
        assert 10 not in active_sessions
        assert 10 not in poller.watched_sessions

def test_41_events_command_resets_ladder():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    acc = AccountState("u1", "acc1", "stok1")
    poller.accounts["u1"] = acc

    poller.reset_ladder("u1")
    assert acc.ladder_cursor == 0
    # min_gap_s is 1.0s (1000ms), 0.4s ladder entry is clamped to 1.0s
    assert acc.next_deadline_mono_ms == clock.now_mono_ms() + 1000

def test_42_ladder_advances_on_unchanged_timestamp():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    poller.register_session(10, "u1", "acc_u1", "stoken_u1")
    poller.reset_ladder("u1")
    acc = poller.accounts["u1"]
    assert acc.ladder_cursor == 0

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "timestamp": 1600000000000, "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200

        clock.advance_mono(1000)
        poller.poll_once(active_sessions, get_user_fn)
        assert acc.ladder_cursor == 1

def test_43_cursor_past_ladder_uses_playing_base():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    poller.register_session(10, "u1", "acc_u1", "stoken_u1")
    acc = poller.accounts["u1"]
    acc.ladder_cursor = None  # Exhausted

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "timestamp": 1600000000000, "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions, get_user_fn)
        assert acc.ladder_cursor is None
        assert acc.next_deadline_mono_ms == clock.now_mono_ms() + 25000

def test_44_changed_timestamp_resets_ladder():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    poller.register_session(10, "u1", "acc_u1", "stoken_u1")
    acc = poller.accounts["u1"]
    acc.model = parse_snapshot({
        "timestamp": 1600000000000, "is_playing": True, "progress_ms": 5000,
        "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())
    acc.ladder_cursor = None

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "timestamp": 1600000005000, "is_playing": True, "progress_ms": 10000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions, get_user_fn)
        assert acc.ladder_cursor == 1  # Was reset to 0 during fetch, then advanced to 1

def test_45_unchanged_timestamp_does_not_reset_ladder():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    poller.register_session(10, "u1", "acc_u1", "stoken_u1")
    acc = poller.accounts["u1"]
    acc.model = parse_snapshot({
        "timestamp": 1600000000000, "is_playing": True, "progress_ms": 5000,
        "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())
    acc.ladder_cursor = 3

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "timestamp": 1600000000000, "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "spotify:track:1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200

        clock.advance_mono(5000)
        poller.poll_once(active_sessions, get_user_fn)
        assert acc.ladder_cursor == 4

def test_46_ladder_entry_shorter_than_min_gap_clamped():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    acc = AccountState("u1", "acc1", "stok1")
    poller.accounts["u1"] = acc

    poller.reset_ladder("u1")
    # Ladder 0 is 0.4s, min_gap_s is 1.0s -> deadline is 1.0s ahead
    assert acc.next_deadline_mono_ms == clock.now_mono_ms() + 1000

def test_47_budget_exhausted_at_ladder_0_does_not_advance_cursor():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    api._history = [clock.now_mono_ms()] * 20  # Exhaust budget

    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    poller.register_session(10, "u1", "acc_u1", "stoken_u1")
    poller.reset_ladder("u1")
    acc = poller.accounts["u1"]
    assert acc.ladder_cursor == 0

    active_sessions = {10: ("stoken_u1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc_u1", "session_token": "stoken_u1"}

    with patch("requests.get") as mock_get:
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 0
        assert acc.ladder_cursor == 0
