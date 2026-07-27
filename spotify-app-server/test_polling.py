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

    poller.register_session(10, "u1", "acc1", "stok1")
    acc = poller.accounts["u1"]
    acc.model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())
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

    poller.register_session(10, "u1", "acc1", "stok1")
    acc = poller.accounts["u1"]
    acc.model = parse_snapshot({
        "is_playing": False, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())

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
        assert poller.sessions[10].is_watching is True

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
        assert 10 not in poller.sessions

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

def test_52_probe_d1_session_returns_to_panel():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    active_sessions = {7: ("stok7", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u7", "access_token": "acc7", "session_token": "stok7"}

    player_resp = {
        "is_playing": True, "progress_ms": 5000,
        "item": {"type": "track", "uri": "spotify:track:7", "name": "Track 7", "duration_ms": 180000}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = player_resp

        # Cycle 1: on panel -> pushes=1, fetch=1
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1
        assert mock_post.call_count == 1
        assert poller.sessions[7].is_watching is True

        # Cycle 2: leaves panel -> pushes=1, fetch=0 (skipped because is_watching is False after this push)
        mock_post.reset_mock()
        mock_get.reset_mock()
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "features.thtml"}
        poller.accounts["u7"].next_deadline_mono_ms = 0
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1  # Was watching before cycle 2
        assert mock_post.call_count == 1
        assert poller.sessions[7].is_watching is False

        # Cycle 3: RETURNS to panel -> fetch=0 (was unwatched), but push=1 returns panel!
        mock_post.reset_mock()
        mock_get.reset_mock()
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}
        poller.accounts["u7"].next_deadline_mono_ms = 0
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 0  # Gated on is_watching
        assert mock_post.call_count == 1  # Push still happens!
        assert poller.sessions[7].is_watching is True  # Recovered!

        # Cycle 4: still on panel -> fetch=1, push=1
        mock_post.reset_mock()
        mock_get.reset_mock()
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}
        poller.accounts["u7"].next_deadline_mono_ms = 0
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_get.call_count == 1
        assert mock_post.call_count == 1

def test_53_non_dict_push_body_preserves_is_watching():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)

    active_sessions = {8: ("stok8", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u8", "access_token": "acc8", "session_token": "stok8"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"is_playing": True, "progress_ms": 1000, "item": {"type": "track", "name": "T"}}
        
        # 1. Unwatch via features.thtml
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "features.thtml"}
        poller.poll_once(active_sessions, get_user_fn)
        assert poller.sessions[8].is_watching is False

        # 2. Push returns MagicMock / unparseable body -> is_watching stays False
        mock_post.return_value.json.return_value = MagicMock()
        poller.accounts["u8"].next_deadline_mono_ms = 0
        poller.poll_once(active_sessions, get_user_fn)
        assert poller.sessions[8].is_watching is False

def test_54_session_never_received_push_is_watched_and_fetched():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    poller.register_session(9, "u9", "acc9", "stok9")
    assert poller.sessions[9].is_watching is True
    assert poller.sessions[9].has_received_push is False

def test_55_push_404_removes_session_from_sessions_and_active():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    active_sessions = {10: ("stok10", time.time())}
    poller.register_session(10, "u10", "acc10", "stok10")

    def get_user_fn(stoken):
        return {"spotify_user_id": "u10", "access_token": "acc10", "session_token": "stok10"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"is_playing": True}
        mock_post.return_value.status_code = 404

        poller.poll_once(active_sessions, get_user_fn)
        assert 10 not in poller.sessions
        assert 10 not in active_sessions
        assert isinstance(active_sessions.get(10, ()), tuple)

def test_56_ten_seconds_simulated_loop_tick_splits():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    acc = AccountState("u1", "acc1", "stok1")
    acc.model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())
    poller.accounts["u1"] = acc
    poller.register_session(1, "u1", "acc1", "stok1")
    poller.sessions[1].has_received_push = True
    poller.last_sent_progress[1] = "0:10 / 3:00"

    active_sessions = {1: ("stok1", time.time())}

    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        for _ in range(9):
            clock.advance_mono(1000)
            poller.tick_once(active_sessions)

        assert mock_post.call_count == 9
        for call in mock_post.call_args_list:
            patch_data = call[1].get("json")
            assert len(patch_data) == 1
            assert "progress_bar" in patch_data

def test_57_budget_log_emitted_once_per_fetch_cycle():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    active_sessions = {1: ("stok1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc1", "session_token": "stok1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post, patch("poller.logger.info") as mock_log_info:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"is_playing": True}
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions, get_user_fn)
        budget_logs = [c for c in mock_log_info.call_args_list if "Spotify API budget" in str(c)]
        assert len(budget_logs) == 1

def test_58_two_accounts_only_fetching_account_sessions_receive_full_patch():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    poller.register_session(1, "u1", "acc1", "stok1")
    poller.register_session(2, "u2", "acc2", "stok2")
    poller.sessions[1].has_received_push = True
    poller.sessions[2].has_received_push = True

    poller.accounts["u1"].next_deadline_mono_ms = clock.now_mono_ms()  # Due
    poller.accounts["u2"].next_deadline_mono_ms = clock.now_mono_ms() + 100000  # Not due

    active_sessions = {1: ("stok1", time.time()), 2: ("stok2", time.time())}
    def get_user_fn(stoken):
        if stoken == "stok1": return {"spotify_user_id": "u1", "access_token": "acc1", "session_token": "stok1"}
        return {"spotify_user_id": "u2", "access_token": "acc2", "session_token": "stok2"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"is_playing": True, "progress_ms": 1000, "item": {"type": "track", "name": "T1"}}
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions, get_user_fn)
        # Full patch pushed ONLY to session 1
        assert mock_post.call_count == 1
        assert mock_post.call_args[0][0].endswith("/sessions/1/patch")

def test_59_breaker_paused_carries_player_info_and_empty_error():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    api._breaker_until_mono_ms = clock.now_mono_ms() + 50000
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    poller.register_session(1, "u1", "acc1", "stok1")

    active_sessions = {1: ("stok1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc1", "session_token": "stok1"}

    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        poller.poll_once(active_sessions, get_user_fn)
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert "Wstrzymano zapytania" in patch_data.get("player_info", "")
        assert patch_data.get("player_error") == ""

def test_60_fetch_500_carries_player_error():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    poller.register_session(1, "u1", "acc1", "stok1")

    active_sessions = {1: ("stok1", time.time())}
    def get_user_fn(stoken):
        return {"spotify_user_id": "u1", "access_token": "acc1", "session_token": "stok1"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 500
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions, get_user_fn)
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("player_error") == "Błąd serwera Spotify (HTTP 500)"

def test_65_set_pending_message_carried_in_poll_once():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    poller.register_session(1, "u1", "acc1", "stok1")
    poller.accounts["u1"].model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())

    poller.set_pending("u1", error="Błąd Spotify (HTTP 403)", info="")
    active_sessions = {1: ("stok1", time.time())}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"is_playing": True, "progress_ms": 10000, "item": {"type": "track", "name": "T1", "duration_ms": 180000}}
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions)
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("player_error") == "Błąd Spotify (HTTP 403)"

def test_66_pending_message_cleared_after_ttl_in_poll_once():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    poller.register_session(1, "u1", "acc1", "stok1")
    poller.accounts["u1"].model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())

    poller.set_pending("u1", error="Błąd Spotify (HTTP 403)", info="")
    active_sessions = {1: ("stok1", time.time())}

    # Advance clock past PENDING_MSG_TTL_S (5.0s -> 6000ms)
    clock.advance_mono(6000)

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"is_playing": True, "progress_ms": 16000, "item": {"type": "track", "name": "T1", "duration_ms": 180000}}
        mock_post.return_value.status_code = 200

        poller.poll_once(active_sessions)
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("player_error") == ""

def test_67_breaker_outranks_pending_message():
    clock = FakeClock(mono_ms=100000)
    api = SpotifyApiClient(clock_fn=clock.now_mono_ms)
    api._breaker_until_mono_ms = clock.now_mono_ms() + 50000
    poller = PollerManager(clock_fn=clock.now_mono_ms, api_client=api)
    poller.register_session(1, "u1", "acc1", "stok1")
    poller.accounts["u1"].model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())

    poller.set_pending("u1", error="Command Error", info="Command Info")
    active_sessions = {1: ("stok1", time.time())}

    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        poller.poll_once(active_sessions)
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert "Wstrzymano zapytania" in patch_data.get("player_info", "")
        assert patch_data.get("player_error") == ""

def test_68_tick_once_pushes_single_clearing_patch_on_expiry():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    poller.register_session(1, "u1", "acc1", "stok1")
    acc = poller.accounts["u1"]
    acc.model = parse_snapshot({
        "is_playing": False, "progress_ms": 10000,
        "item": {"type": "track", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())

    poller.set_pending("u1", error="Old Error", info="")
    active_sessions = {1: ("stok1", time.time())}

    # Advance clock past TTL
    clock.advance_mono(6000)

    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        poller.tick_once(active_sessions)
        assert mock_post.call_count == 1
        sent_json = mock_post.call_args[1].get("json")
        assert sent_json == {"player_error": "", "player_info": ""}

        # Second tick_once at same time pushes nothing
        mock_post.reset_mock()
        poller.tick_once(active_sessions)
        assert mock_post.call_count == 0

def test_69_no_pending_message_tick_once_standard_behavior():
    clock = FakeClock(mono_ms=100000)
    poller = PollerManager(clock_fn=clock.now_mono_ms)
    poller.register_session(1, "u1", "acc1", "stok1")
    acc = poller.accounts["u1"]
    acc.model = parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "name": "T1", "duration_ms": 180000}
    }, clock.now_mono_ms())

    active_sessions = {1: ("stok1", time.time())}
    clock.advance_mono(1000)

    with patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        poller.tick_once(active_sessions)
        assert mock_post.call_count == 1
        sent_json = mock_post.call_args[1].get("json")
        assert list(sent_json.keys()) == ["progress_bar"]

